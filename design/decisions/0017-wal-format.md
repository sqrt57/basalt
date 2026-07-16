# ADR 0017: WAL Format — Log Records, LSN, and Redo Recovery

Status: Proposed (2026-07-16)

## Context

[stage1-plan.md chunk 3](../stage1-plan.md) (WAL + redo recovery: wiring
chunk 2's copy-on-write B+-tree writes through
`<prefix>.log.bin`) can't start without a concrete log format, the same
way chunks 1 and 2 needed [0015](0015-page-storage-format.md) and
[0016](0016-btree-node-format.md) before any code. [0008](0008-concurrency-durability.md)
decided the *policy* — STEAL/NO-FORCE, redo-only (no undo, no
transaction table, no prevLSN chaining), binary-diff log records, a
fuzzy checkpoint built from a dirty-page table (page → recLSN) — but
none of the byte-level format: what an LSN actually is, how one record
is told apart from the next, how a **torn write** at the log's tail
after a crash is detected (nothing in the design so far handles this —
[0015](0015-page-storage-format.md) only reserves an unused checksum
field for the *data* file), what a "binary diff" between two
differently-numbered copy-on-write pages actually encodes, or how the
current root — which [0016](0016-btree-node-format.md) explicitly left
as in-memory-only on the tree handle — becomes recoverable after a
restart.

Also relevant: chunk 1's `Pager` ([0015](0015-page-storage-format.md))
writes every page straight through to the OS file on `write_page`, with
no in-memory page cache and no fsync call anywhere yet. STEAL/NO-FORCE
and a dirty-page table presuppose *some* notion of "written but not yet
durable" — here that's satisfied by the gap between an unsynced
`write_page` and the next fsync, not by a deferred/buffered cache. This
ADR treats that gap as the thing the dirty-page table tracks, rather
than introducing a page cache chunk 3 doesn't otherwise need.

Chunk 2 already decided the tree never frees a superseded page
(reclamation is chunk 4's job), so chunk 3 never exercises
`Pager::free_page` — only `allocate_page` and `write_page`. That keeps
this ADR's scope to logging page *content* writes; free-list mutations
aren't part of chunk 3's exercised paths and need no special handling
here.

## Decision

**Log file & preamble**: `<prefix>.log.bin` starts with a fixed 64-byte
preamble, mirroring [0015](0015-page-storage-format.md)'s convention:
magic bytes `b"BSLTLOG_"` (8 bytes), `format_version: u32` (little-
endian), the rest reserved/zeroed. No page size field — diff records are
self-describing and don't need it. Records begin immediately after, at
offset 64.

**LSN = log byte offset**: an LSN is the file offset (`u64`) of a
record's own header, not a separate counter. `recLSN` in the
dirty-page table reuses this same value. This needs no counter state
to persist or recover — the position to resume appending at *is* the
next free LSN.

**Record framing**:

- header: `payload_len: u32` (LE), `record_type: u8`
- payload: exactly `payload_len` bytes
- trailer: `checksum: u32` (LE) — CRC-32 (IEEE) over the `record_type`
  byte followed by the payload bytes

Recovery reads records sequentially from offset 64. If the header can't
be fully read, or the payload+trailer can't be fully read, or the
checksum doesn't match, that record and everything after it is a
**torn tail** — not an error, just the point where redo stops. This is
sound because a transaction is only acknowledged as committed once its
commit record's bytes, and everything before them, are fsynced; a torn
tail can therefore only contain work that was never acknowledged.

**Record types**:

- `PageDiff = 0` — payload: `new_page_id: u64`, `base_page_id: u64`
  (sentinel `u64::MAX`, matching `pager.rs`'s existing
  `FREE_LIST_NONE` convention, meaning "no base — reconstruct from an
  all-zero page"), `num_ranges: u16`, then `num_ranges` entries of
  (`offset: u16`, `len: u16`, `bytes: [u8; len]`). Redo: start from the
  base page's current on-disk content (or a zeroed `page_size`-byte
  buffer if the sentinel), overwrite each range in listed order, write
  the result to `new_page_id` — extending the data file first if
  `new_page_id` is beyond its currently-allocated range (see
  Consequences).
- `Commit = 1` — payload: `root_page_id: u64`. Marks every `PageDiff`
  since the previous `Commit`/`Checkpoint` as belonging to a completed
  transaction and gives recovery the new current root. A transaction is
  durable once its `Commit` record — and everything before it — is
  fsynced; the write path fsyncs the log up to and including this
  record before returning success to the caller.
- `Checkpoint = 2` — payload: `num_entries: u16`, then `num_entries`
  entries of (`page_id: u64`, `rec_lsn: u64`): a snapshot of the
  dirty-page table (pages `write_page` has touched since they were last
  made durable, each tagged with the LSN of the `PageDiff` that first
  dirtied it). Taking a checkpoint: snapshot the DPT, write it as this
  record, fsync the data file, then drop from the in-memory DPT every
  entry whose page is now covered by that fsync. Chunk 3 triggers a
  checkpoint only explicitly (a caller-invoked method) and always on
  clean close — no background size/timeout trigger, since stage 1 has
  no background-task mechanism to run one on yet (see Consequences).

**Diff computation**: the format supports multiple changed ranges per
page, but chunk 3's first algorithm computes at most one — trim the
common prefix and common suffix between base and new page content, and
log whatever differs in between as a single range (`num_ranges` is 0 if
the pages are identical, otherwise 1). This degrades toward a
whole-page-sized range for structural operations (splits) where old and
new content share little, matching what [0008](0008-concurrency-durability.md)
already accepts. `offset`/`len` as `u16` mirrors
[0016](0016-btree-node-format.md)'s existing slot-offset convention,
which already implicitly caps in-page addressing at 65536 bytes — this
introduces no new page-size ceiling. The multi-range format is
deliberately more general than this first algorithm needs, so a
smarter one (e.g. treating the header, slot directory, and cell region
as independent ranges) can replace it later without a format change.

**Redo recovery algorithm**: open the log, validate the preamble, then
scan records sequentially from offset 64 to end-of-file, applying the
framing rules above (stopping cleanly at the first torn/unreadable
record, which is indistinguishable from — and requires no special
handling versus — a clean end-of-file). For each valid `PageDiff`,
reconstruct and write the page. For each valid `Commit`, remember its
`root_page_id` as the current candidate. `Checkpoint` records are
logged but not yet consumed by recovery (see Consequences). After the
scan, the current root is whatever the last `Commit` record's
`root_page_id` was, or "empty database" if none was ever seen. Redo is
naturally idempotent: reconstructing a page always starts from the
(immutable) base page's content, never from the page being redone
itself, so replaying the same record twice — or re-running recovery
from the start after an interrupted first attempt — reproduces
identical bytes each time.

**Write path**: chunk 3 stays single-writer ([0008](0008-concurrency-durability.md)).
A write transaction appends a `PageDiff` record (not yet fsynced) for
each `write_page` call chunk 2's tree makes, then on commit appends the
`Commit` record and fsyncs the log up through it before returning. Data
file writes happen inline as before, unsynced — NO-FORCE — durable only
once the following checkpoint's fsync catches up to them.

## Consequences

- `Checkpoint` records exist and are testable (a checkpoint call
  produces a valid record and fsyncs the data file), but redo doesn't
  yet use the DPT to pick a later scan-start offset — full-log-scan
  recovery is always correct, just not the fast path a large log would
  eventually want. Making that work needs a durable pointer to the
  latest checkpoint's own LSN (so recovery doesn't have to scan the
  whole log just to find it) — not designed here, left open (see
  [backlog.md](../backlog.md)).
- No automatic/background checkpoint triggering (size- or
  timeout-based) in chunk 3, only explicit-call and on-close — revisit
  once a background-task mechanism exists elsewhere in the engine.
  Left open (see [backlog.md](../backlog.md)).
- [0015](0015-page-storage-format.md)'s data-file preamble/page-0
  format is untouched by this ADR: chunk 3 never persists the root
  anywhere in the data file, only in the log, via `Commit` records.
  Durable root *history* (multiple generations, as opposed to one
  recoverable current root) stays chunk 4's job.
- Redo applying a `PageDiff` to a `new_page_id` beyond the data file's
  currently-allocated range must extend the file first rather than
  assume the original `allocate_page` call's extension survived a
  crash — file-metadata durability (length) isn't otherwise guaranteed
  synced before a crash on every filesystem.
- Diff compression is intentionally not maximized: the prefix/suffix
  trim will over-log routine inserts that touch disjoint regions of a
  node (header fields, mid-directory slot insertion, tail-appended
  cell) as one large range spanning all of them. Acceptable for
  correctness-first chunk 3 scope; revisit if log size becomes a
  concern (see [backlog.md](../backlog.md)).
- CRC-32 per record is what makes a torn tail after a crash
  distinguishable from a valid record at all — without it, a
  partially-written last record could be misread as well-formed,
  silently corrupting the recovered root or a page's content instead of
  being cleanly dropped.
