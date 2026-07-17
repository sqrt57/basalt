# ADR 0017: WAL Format — Log Records, LSN, and Redo Recovery

Status: Proposed (2026-07-16), revised (2026-07-17)

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

**Revision (2026-07-17)**: [0015](0015-page-storage-format.md) added a
`page_lsn` field to the common page header every page starts with, so
that a page's own bytes record which write last produced them. Chunk
3's write path is the thing that actually has an LSN to stamp there —
every newly-encoded page gets `page_lsn` set to its own `PageDiff`
record's LSN before it's written. That interacts with this ADR's diff
computation: `page_lsn` differs between *any* two distinct pages (a COW
base and its successor never share the same stamp), so a naive
prefix-trim comparing base-vs-new byte-for-byte would never get past
offset 2 before hitting a mismatch — collapsing the diff to nearly the
whole page regardless of how small the real structural change was, the
opposite of what prefix-trim is for. Revised below: the `page_lsn`
bytes are logged as their own small, unconditional range, and
prefix/suffix trimming runs only over the content *after* the common
header, where it still works as intended.

## Decision

**Log file & preamble**: `<prefix>.log.bin` starts with a fixed 64-byte
preamble, mirroring [0015](0015-page-storage-format.md)'s convention.
No page size field — diff records are self-describing and don't need
it. All multi-byte fields little-endian.

| Offset | Length | Description |
|---|---|---|
| 0 | 8 | magic bytes `b"BSLTLOG_"` |
| 8 | 4 | `format_version: u32` |
| 12 | 52 | reserved (zeroed) |

Records begin immediately after, at offset 64.

**LSN = log byte offset**: an LSN is the file offset (`u64`) of a
record's own header, not a separate counter. `recLSN` in the
dirty-page table reuses this same value. This needs no counter state
to persist or recover — the position to resume appending at *is* the
next free LSN.

**Record framing** (offsets relative to the record's own start):

| Offset | Length | Description |
|---|---|---|
| 0 | 4 | `payload_len: u32` (LE) |
| 4 | 1 | `record_type: u8` |
| 5 | `payload_len` | payload bytes |
| `5 + payload_len` | 4 | `checksum: u32` (LE) — CRC-32C (Castagnoli) over the `record_type` byte followed by the payload bytes |

Castagnoli over the classic IEEE polynomial: better error detection at
the record lengths a WAL actually sees, and hardware-accelerated on
both platforms this project targets (x86_64 SSE4.2 `crc32`, ARMv8
CRC32C) — the same choice Postgres, RocksDB/LevelDB, btrfs, ext4, and
iSCSI made for the same reason.

Recovery reads records sequentially from offset 64. If the header can't
be fully read, or the payload+trailer can't be fully read, or the
checksum doesn't match, that record and everything after it is a
**torn tail** — not an error, just the point where redo stops. This is
sound because a transaction is only acknowledged as committed once its
commit record's bytes, and everything before them, are fsynced; a torn
tail can therefore only contain work that was never acknowledged.

**Record types**:

- `PageDiff = 0` — payload:

  | Offset | Length | Description |
  |---|---|---|
  | 0 | 8 | `new_page_id: u64` |
  | 8 | 8 | `base_page_id: u64` — sentinel `0`, matching [0015](0015-page-storage-format.md)'s page-id-`0`-is-null convention (page id 0 is never a physical page at all, so it can never collide with a real base page), meaning "no base — reconstruct from an all-zero page" |
  | 16 | 2 | `num_ranges: u16` |
  | 18 | varies | `num_ranges` range entries (see below) |

  Each range entry:

  | Offset | Length | Description |
  |---|---|---|
  | 0 | 2 | `offset: u16` |
  | 2 | 2 | `len: u16` |
  | 4 | `len` | `bytes: [u8; len]` |

  Redo: start from the base page's current on-disk content (or a zeroed
  `page_size`-byte buffer if the sentinel), overwrite each range in
  listed order, write the result to `new_page_id` — extending the data
  file first if `new_page_id` is beyond its currently-allocated range
  (see Consequences).
- `Commit = 1` — payload:

  | Offset | Length | Description |
  |---|---|---|
  | 0 | 8 | `root_page_id: u64` |

  Marks every `PageDiff` since the previous `Commit`/`Checkpoint` as
  belonging to a completed transaction and gives recovery the new
  current root. A transaction is durable once its `Commit` record —
  and everything before it — is fsynced; the write path fsyncs the log
  up to and including this record before returning success to the
  caller.
- `Checkpoint = 2` — payload:

  | Offset | Length | Description |
  |---|---|---|
  | 0 | 2 | `num_entries: u16` |
  | 2 | varies | `num_entries` dirty-page-table entries (see below) |

  Each DPT entry:

  | Offset | Length | Description |
  |---|---|---|
  | 0 | 8 | `page_id: u64` |
  | 8 | 8 | `rec_lsn: u64` |

  A snapshot of the dirty-page table (pages `write_page` has touched
  since they were last made durable, each tagged with the LSN of the
  `PageDiff` that first dirtied it). `rec_lsn` is a distinct, in-memory
  bookkeeping value from [0015](0015-page-storage-format.md)'s on-disk
  `page_lsn`: `rec_lsn` is the *earliest* not-yet-durable LSN for a page
  (used to know how far back redo would need to scan if a crash
  happened right now), while `page_lsn` is whatever LSN *most recently*
  produced the page's current on-disk bytes — the same page can have
  been written multiple times between checkpoints, in which case
  `rec_lsn` stays pinned at the first of those writes while `page_lsn`
  keeps advancing. Taking a checkpoint: snapshot the
  DPT, write it as this record, fsync the data file, then drop from the
  in-memory DPT every entry whose page is now covered by that fsync.
  Chunk 3 triggers a checkpoint only explicitly (a caller-invoked
  method) and always on clean close — no background size/timeout
  trigger, since stage 1 has no background-task mechanism to run one on
  yet (see Consequences).

**Diff computation**: the format supports multiple changed ranges per
page, and chunk 3's first algorithm always uses exactly one or two of
them:

- **Range 1, unconditional**: the common page header's 8-byte
  `page_lsn` field (offset 2..10 — see
  [0015](0015-page-storage-format.md)), always logged, since it's
  stamped fresh on every write and therefore always differs from the
  base page's. Logging it as its own tiny range keeps it from
  poisoning the prefix-trim comparison below.
- **Range 2, conditional**: trim the common prefix and common suffix
  between base and new page content *starting from offset 10* (i.e.
  excluding the whole common header, not just `page_lsn`, from the
  comparison — `page_type` and reserved byte are expected to match
  between a COW base and its successor, but excluding all 10 header
  bytes uniformly is simpler than special-casing just the 8 that
  actually differ), and log whatever differs in between as a second
  range. Omitted (`num_ranges = 1`, just the `page_lsn` range) if
  content past offset 10 is byte-identical — the normal case is this
  range *is* present, since every write changes real content, not just
  the LSN stamp.

This degrades toward a whole-page-sized second range for structural
operations (splits) where old and new content share little, matching
what [0008](0008-concurrency-durability.md) already accepts.
`offset`/`len` as `u16` mirrors [0016](0016-btree-node-format.md)'s
existing slot-offset convention, which already implicitly caps in-page
addressing at 65536 bytes — this introduces no new page-size ceiling.
The multi-range format is deliberately more general than this first
algorithm needs, so a smarter one (e.g. treating the header, slot
directory, and cell region as independent ranges) can replace it later
without a format change.

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
identical bytes each time. That still holds with `page_lsn` in the mix:
a record's `page_lsn` range always overwrites with the same fixed value
it was logged with, so replaying it twice writes that same value twice
— idempotent, just like every other range. Redo does not currently
*read* `page_lsn` to decide anything (e.g. to skip a page already known
durable) — it unconditionally reconstructs and rewrites every page a
valid `PageDiff` names, same as before this revision; using `page_lsn`
to skip redundant work is a possible future optimization, not
implemented here (see [backlog.md](../backlog.md)).

**Write path**: chunk 3 stays single-writer ([0008](0008-concurrency-durability.md)).
For each new page chunk 2's tree encodes: the LSN this write will use is
fixed as soon as it's known — the current end-of-log offset, before
that record's bytes are appended (LSN = log byte offset, decided
above) — and that value is stamped into the new page's `page_lsn` field
*before* the diff against the base page is computed, so the diff
(Range 1 above) correctly captures the `page_lsn` change along with
whatever else differs. The `PageDiff` record is then appended (not yet
fsynced) for each `write_page` call chunk 2's tree makes, then on commit
appends the `Commit` record and fsyncs the log up through it before
returning. Data file writes happen inline as before, unsynced —
NO-FORCE — durable only once the following checkpoint's fsync catches
up to them.

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
- [0015](0015-page-storage-format.md)'s data-file preamble/page-1
  format is untouched by this ADR: chunk 3 never persists the root
  anywhere in the data file, only in the log, via `Commit` records.
  Durable root *history* (multiple generations, as opposed to one
  recoverable current root) stays chunk 4's job.
- `num_ranges` is effectively never `0` anymore now that every
  `PageDiff` unconditionally includes the `page_lsn` range — the
  smallest possible diff is `num_ranges = 1` (just those 8 bytes, if
  nothing else differs) rather than `num_ranges = 0` (identical pages).
  A small, fixed per-write overhead (one extra range header, 4 bytes,
  plus the 8-byte `page_lsn` payload) in exchange for every page
  carrying real provenance; not a concern at chunk-3's correctness-first
  stage.
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
- CRC-32C per record is what makes a torn tail after a crash
  distinguishable from a valid record at all — without it, a
  partially-written last record could be misread as well-formed,
  silently corrupting the recovered root or a page's content instead of
  being cleanly dropped. This only needs to answer one binary question
  per record ("did this survive the crash intact") on records that are
  at most page-sized — no need for a wider checksum (CRC-64, xxHash64,
  a cryptographic hash), which would trade cost for protecting against
  threats (long-term bit-rot across large blocks, adversarial
  tampering) that don't apply to a local embedded WAL file.
