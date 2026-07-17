# ADR 0018: MVCC Snapshots: Generations, Reader Table, Reclamation Wiring

Status: Proposed (2026-07-16)

## Context

[0008](0008-concurrency-durability.md) decided single-writer MVCC via a
copy-on-write B+-tree, and [0013](0013-page-reclamation.md) decided
*how* superseded pages get reclaimed (a low-water mark over an
in-memory reader table, riding the checkpoint pass). Neither pins down
the concrete shapes stage-1 chunk 4 needs to actually build: what a
"snapshot generation" is, what the reader table and the pending-reclaim
list actually look like in memory, how a reader transaction is exposed
as an API without fighting Rust's borrow checker, and how superseded
pages get identified in the first place from chunk 2's COW writes.

**Generation numbering.** Needs to be totally ordered and cheap to
compare (the low-water mark is a min over open readers' generations).
A plain in-memory `u64` counter, incremented once per successful commit,
does this with no persistence: [0013](0013-page-reclamation.md) already
decided the reader table itself isn't crash-durable (readers don't
survive a process crash, so losing their pins is moot), and the same
reasoning extends to the counter that labels generations — a fresh
process can restart numbering from 0 with no correctness impact, since
nothing from a prior process's reader table can still be pinning an old
generation.

**Reader-table shape.** [0013](0013-page-reclamation.md) describes "each
open read transaction registers its snapshot generation... the minimum
across open entries is the low-water mark" — that only requires a
multiset of generations, not a table keyed by reader identity. A
`BTreeMap<generation, open_count>` gives the minimum as its first key in
O(1) and needs no reader-id allocation.

**Superseded-page tracking.** Chunk 2's `NodeStore::write_new_node`
already takes `base: Option<PageId>` — the page a new page was derived
from — but chunk 2/3 only use it to compute the WAL diff, never record
it anywhere. Chunk 4 needs exactly that list: every `Some(base)` seen
during a transaction is a page that becomes unreachable from the root
the transaction produces. It's provisional until commit, though — an
aborted transaction's new pages are simply discarded (per
[0008](0008-concurrency-durability.md), no undo needed), and the *old*
pages they would have superseded were never actually superseded, since
the root never changed. So the list of `base`s seen has to be collected
per-transaction and only folded into the durable pending-reclaim set
(tagged with the new generation) on a successful commit, discarded
outright on abort.

**Reader-transaction API shape.** `Pager::read_page` takes `&mut self`,
and stage 1 exposes the database as one `&mut Database` handle with no
internal synchronization — there's no thread-safe concurrent execution
model here yet (see Consequences). A reader handle therefore can't hold
a live borrow of the pager for its whole lifetime without blocking every
other `Database` call for as long as it's open, which would defeat the
point of MVCC readers entirely. The alternative already precedented by
LMDB's C API: a read transaction is a lightweight token (the snapshot's
generation and root), and actual reads are `Database` methods that take
`&mut self` plus a `&Snapshot` each time, re-borrowing the pager per
call rather than for the reader's whole lifetime. This lets a writer
commit in between two reads made through the same open snapshot, exactly
as the interleaved, single-handle, no-threads embedded model requires.

**Registration lifetime.** A `Snapshot` needs to deregister from the
reader table when the caller is done with it. Auto-deregistration on
`Drop` would need the reader table to be shared (`Rc<RefCell<_>>`)
between `Database` and every outstanding `Snapshot`, purely to support a
convenience the rest of this codebase doesn't otherwise need —
`Database::close` is already explicit/consuming, not `Drop`-based.
Matching that, `Snapshot` is a plain, cheap-to-copy struct with no
special `Drop` behavior; the caller calls `Database::end_read` explicitly
when done. A caller that forgets to do so leaks that generation's pin
forever — a stage-1-accepted variant of the starvation tradeoff
[0013](0013-page-reclamation.md) already accepts for a long-running
reader, not a new category of risk.

## Decision

- **Generation counter**: `Database` holds `current_generation: u64`,
  starting at 0 on both `create` and `open` (recovery doesn't attempt to
  reconstruct a prior process's generation numbering — nothing depends
  on it surviving a restart). Incremented by exactly 1 per successful
  `transaction` commit, regardless of how many tree operations that
  transaction performed.
- **Reader table**: `BTreeMap<u64, u32>`, stored on `Database`. Low-water
  mark = `reader_table.keys().next()`, or `current_generation` if the
  table is empty (nothing pins anything older than the latest commit).

  | Field | Type | Description |
  |---|---|---|
  | key | `u64` | a snapshot generation |
  | value | `u32` | count of open snapshots currently pinned at that generation |
- **`Snapshot`**: `Copy`, returned by `Database::begin_read(&mut self)
  -> Snapshot`, which captures `(current_generation,
  tree.as_ref().map(BTree::root))` and increments its count in the
  reader table.

  | Field | Type | Description |
  |---|---|---|
  | `generation` | `u64` | the generation this snapshot was taken at |
  | `root` | `Option<PageId>` | tree root as of that generation, `None` for an empty database |

  `Database::end_read(&mut self, snap: Snapshot)` decrements that count,
  removing the entry at 0. Reads against a snapshot go through
  `Database` methods taking `&mut self, &Snapshot` (`lookup_at`,
  `scan_all_at`, `range_at`), reconstructing a `BTree::from_root` per
  call rather than holding one open.
- **Per-transaction superseded-page list**: `Database::transaction`
  collects every `base` passed to `write_new_node` during the closure
  (a `Vec<PageId>`, threaded through the same store wrapper that already
  carries the dirty-page table). On successful commit, `current_generation`
  is incremented and every collected page is appended to
  `pending_reclaim: Vec<(PageId, u64)>`, tagged with the *new*
  generation:

  | Field | Type | Description |
  |---|---|---|
  | `.0` | `PageId` | a page superseded by this transaction's commit |
  | `.1` | `u64` | the generation it was superseded at (the new `current_generation`) |

  On abort (closure returns `Err`), the collected list is simply
  dropped — those old pages remain reachable from the unchanged current
  root, so they were never actually superseded.
- **Reclaim trigger**: `Database::checkpoint` (already the reclaim
  trigger per [0013](0013-page-reclamation.md)) computes the low-water
  mark, then partitions `pending_reclaim`: every entry with
  `superseded_at <= low_water_mark` is freed via `Pager::free_page` and
  dropped from the list; the rest stay pending for a future pass.
- **Concurrency execution model**: unchanged from the rest of stage 1 —
  one `&mut Database` handle, no internal locking, no thread-safety
  claim. "Concurrent readers" here means logically-interleaved snapshot
  handles across sequential calls on that one handle (a write can commit
  between two reads made through the same open `Snapshot`), not readers
  and a writer executing on separate OS threads simultaneously. Whether
  stage 1 ever needs the latter, and what it would take (`Sync` pager,
  actual locking), isn't decided here (see Consequences).

## Consequences

- No new on-disk format: generations, the reader table, and the
  pending-reclaim list are all in-memory-only, consistent with
  [0013](0013-page-reclamation.md) already deciding the reader table
  isn't crash-durable. `free_page` itself still isn't WAL-logged or
  fsynced — a pre-existing gap from chunk 1 that chunk 4 is the first to
  actually exercise on the normal commit path; deferred to
  [backlog.md](../backlog.md) rather than fixed here.
- A caller that never calls `end_read` after `begin_read` leaks that
  pin permanently, same failure shape as the already-accepted
  long-running-reader starvation case — not a new risk class, just a
  second way to trigger the same one.
- The snapshot API (`Snapshot` token + `_at` methods re-borrowing
  `&mut Database` per call) works cleanly for the single-handle,
  no-threads model stage 1 already has everywhere else, but doesn't by
  itself enable readers and a writer to run in parallel on separate
  threads — that would need the pager and reader table to be `Sync` and
  independently locked, which is a bigger change not attempted here.
- Because commit-vs-abort determines whether collected `base` pages
  actually became reclaimable, `Database::transaction`'s existing
  closure shape (commit only on `Ok`, nothing durable on `Err`) already
  gives the right hook for free — no new transaction-outcome plumbing
  needed beyond threading the collected list through it.
