# ADR 0008: Concurrency Control & Durability

Status: Proposed (2026-07-10), revised (2026-07-14), revised (2026-07-15)

## Context

Split out from the original implementation-architecture discussion
([0004](0004-relational-storage-engine.md),
[0010](0010-hierarchical-storage-engine.md)) since concurrency control
and durability apply across both storage engines rather than being a
property of either one.

**Concurrency control**: of the three standard options (single-writer
serialization à la SQLite, locking à la classic SQL Server, MVCC à la
Postgres), MVCC was chosen and shared across both engines rather than
letting each engine pick its own strategy (e.g. a simpler locking model
for the hierarchical engine, closer to GT.M's historical concurrency
model). Sharing the versioning primitive was judged the path of least
duplicated code, since both engines ultimately store "a keyed value" at
the B+-tree level even though the key shapes differ.

Two structurally different ways to implement MVCC were considered:
**per-row versioning** with a durable transaction-status structure
(Postgres-style: each row tagged with the transaction IDs that created
and superseded it, visibility resolved via status lookups, dead
versions reclaimed by a vacuum-like sweep), and **whole-tree
copy-on-write snapshots** (LMDB/CouchDB-style: a transaction never
mutates a live page, it writes new pages instead, and commit is a
single atomic swap of one root pointer; a reader just walks whichever
root was current when it started). Per-row versioning is what lets
multiple writers commit independently, since visibility is resolved per
row rather than gated by one serialized pointer — at the cost of a
transaction-status structure and vacuum-style reclamation. Copy-on-write
needs neither of those, but a single evolving root pointer inherently
serializes commits, which is only free of cost while there's a single
writer. Copy-on-write was chosen, since stage 1 (see below) is
single-writer regardless.

**Durability**: WAL was chosen over alternatives with no serious
contenders discussed, per near-universal precedent (Postgres, SQL
Server, MySQL/InnoDB). Given MVCC is shared, a single shared WAL follows
naturally — a transaction spanning both engines needs one atomic
durability boundary, not coordination between two independent logs.
Going log-free instead (LMDB-style: no WAL at all, just two alternating
meta-pages holding the current root, recovery picks whichever has the
higher valid transaction ID) was considered and rejected — it's a valid
approach for the same copy-on-write storage model, but a WAL is kept
here for the redo/checkpoint machinery it gives independent of the
storage engine's own page structure.

**Log record format**: for the copy-on-write pages this ADR uses,
records are **binary diffs** between old and new page content (see
Decision), not whole-page images or operation-level descriptions.
Classic **physiological** logging (ARIES/Postgres/InnoDB-style: physical
page addressing plus a logical description of the in-page change) is
the usual middle ground between those two, but its actual purpose —
tolerating a page's physical layout drifting between when a record was
logged and when it's replayed — doesn't apply here: copy-on-write pages
are immutable once written, so there's no drift to tolerate. The
comparable "smaller than whole-page" alternative for this storage model
would be genuinely logical/operation-replay logging, not physiological;
binary diffs were chosen instead since they get most of the same space
saving without needing replay logic that has to track the B+-tree
implementation's evolution.

**Staging**: full MVCC — concurrent writers, write-write conflict
handling, the five-level isolation buildup — is nontrivial, and the
[build order](../roadmap.md) prioritizes a working embedded engine
end-to-end before it exists. But the concurrency-control piece stage 1
actually needs isn't "no MVCC," the way originally planned — it's
**single-writer MVCC**: one write transaction at a time (still fully
serialized, so no write-write conflict handling is needed yet),
implemented as the copy-on-write B+-tree above, giving concurrent
readers a consistent snapshot without ever blocking on the active
writer. Stage 4 ("other concurrency mechanisms") extends this to
concurrent writers, which a single evolving root pointer doesn't support
as-is — see [backlog.md](../backlog.md).

Durability doesn't get its own staging: the WAL is used from stage 1
onward. And because storage is versioned from stage 1, not just from
stage 4, a crash or an aborted transaction never needs undo, at any
stage: its pages are simply unreachable from any committed root and
become garbage, not something to unwind. This is the same trick
Postgres uses to avoid a WAL-based undo phase, arrived at independently
here via copy-on-write rather than per-row versioning.

## Decision

**Stage 1 (embedded core)**:

- **Concurrency control**: single-writer MVCC via a copy-on-write
  B+-tree. One write transaction at a time — still fully serialized, no
  write-write conflict handling needed. A write never mutates a live
  page; it allocates new pages along the path to the root, and commit
  is a single atomic write recording the new root as current. Readers
  hold whichever root was current when they started and only walk pages
  reachable from it — never blocked by the active writer, with no
  per-row visibility bookkeeping and no transaction-status structure.
- **Durability**: a real **write-ahead log**, in its own file separate
  from the data file ([0011](0011-embedded-config.md)'s
  `<prefix>.log.bin`). **STEAL** and **NO-FORCE**: a new (uncommitted)
  page may reach disk before its transaction commits, and commit only
  requires the page's WAL record to be fsynced, not the page itself.
  Needs only **redo** — no undo, because an aborted or crashed
  transaction's pages are simply unreferenced by any committed root,
  never requiring unwinding. Log records are **binary diffs**: since a
  write always produces a wholly new, immutable page built from an
  existing (also immutable) old page, the log stores the byte-level
  diff between old and new page content, rather than either the whole
  new page (physical) or a semantic description of the write
  (logical/operation-level). This captures most of a semantic log's
  space saving for routine inserts/updates, without needing
  operation-replay logic that has to stay correct as the B+-tree
  implementation evolves; it degrades toward whole-page size for
  structural operations (splits/merges), where old and new page content
  share little.
- **Checkpoint mechanism**: fuzzy. A **dirty page table** (page →
  recLSN, the LSN it was first dirtied at since its last flush) is
  snapshotted and written to the log as the checkpoint record, without
  pausing new transactions; the pages it describes keep flushing in the
  background rather than being forced synchronously. Redo starts from
  the minimum recLSN recorded in the latest checkpoint. No transaction
  table is needed — recovery doesn't track which transaction was open
  for undo purposes; it only redoes forward and finds the latest commit
  record to know which root is actually current. No prevLSN chaining is
  needed either, for the same reason: there's no undo to chain records
  for.

**Stage 4 ("other concurrency mechanisms")** — across both storage
engines ([0004](0004-relational-storage-engine.md),
[0010](0010-hierarchical-storage-engine.md)):

- **Concurrency control**: extend single-writer MVCC to concurrent
  writers. The single evolving root pointer stage 1 picked doesn't
  support that directly — two writers' independent copy-on-write
  changes need reconciling into one next root, or the technique needs
  to change. Not decided by this ADR (see [backlog.md](../backlog.md)).
- **Durability**: unchanged from stage 1 — no undo is needed at stage 4
  either, for the same reason it isn't needed at stage 1 (MVCC
  visibility, not physical unwinding, is what hides an aborted
  transaction's writes). Stage 4 doesn't reopen the durability design.

## Consequences

- Single-writer MVCC gives concurrent readers from stage 1 onward — a
  real improvement over the plain global lock originally planned — but
  writers stay fully serialized, so stage 1 has no write-write conflict
  question to solve, and no concurrent-writer throughput either.
- Copy-on-write means a single-key write costs O(tree height) new
  pages, not one — real write amplification per transaction, traded
  against needing no transaction-status structure, no status lookups,
  and no vacuum-style dead-tuple sweep the way per-row versioning
  (Postgres's approach) would need.
- No undo, at any stage: STEAL is safe without it because an aborted or
  crashed transaction's pages are simply unreferenced by any committed
  root — nothing to unwind, no compensation log records, no prevLSN
  chain.
- Old, unreferenced pages need reclaiming once no reader still holds a
  root that reaches them — a reference-counting or epoch-based scheme,
  not yet designed (see [backlog.md](../backlog.md)).
- Binary diff logging saves real space for routine inserts/updates but
  degrades toward whole-page size for structural operations
  (splits/merges), where old and new page content share little.
- Stage 4's move to concurrent writers is a genuine open problem, not
  just "turn on more concurrency" — reconciling independent
  copy-on-write changes into one next root (or moving away from a
  single root pointer) needs its own decision (see
  [backlog.md](../backlog.md)).
- Tree-snapshot readers already get a fully consistent, unchanging view
  of the whole database for the duration of their transaction, from
  stage 1 onward — in tension with [0005](0005-sql-support.md)'s
  framing that isolation is moot until MVCC lands at stage 4, since this
  may already amount to Snapshot isolation (or better) before then. Not
  resolved here — flagged in [backlog.md](../backlog.md).
- A single shared WAL means transaction atomicity across both engines
  comes for free, but couples their recovery paths together — a
  corruption/bug in one engine's log records affects crash recovery for
  the other engine's transactions too.
