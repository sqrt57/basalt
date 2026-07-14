# ADR 0008: Concurrency Control & Durability

Status: Proposed (2026-07-10), revised (2026-07-14)

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
model). Sharing the versioning primitive — tagging a keyed value with a
transaction ID and checking visibility the same way regardless of engine
— was judged the path of least duplicated code, since both engines
ultimately store "a keyed value" at the B+-tree level even though the key
shapes differ.

**Durability**: WAL was chosen over alternatives with no serious
contenders discussed, per near-universal precedent (Postgres, SQL
Server, MySQL/InnoDB). Given MVCC is shared, a single shared WAL follows
naturally — a transaction spanning both engines needs one atomic
durability boundary, not coordination between two independent logs.

**Staging**: MVCC is nontrivial to build, and the [build order](../roadmap.md)
prioritizes getting a working embedded relational engine end-to-end
before it exists. Stage 1 (the embedded core) ships with the simplest
concurrency mechanism that still holds up structurally — a single
global lock — and this ADR's MVCC decision becomes a later stage
("other concurrency mechanisms") layered on once client/server
([0001](0001-scope.md)) is underway.

Durability doesn't get the same staging: the WAL is used from stage 1
onward, rather than inventing a simpler ad hoc scheme first. A
single-writer concurrency model doesn't make WAL-based recovery
harder — if anything it's simpler than the usual case, since there's
never more than one in-flight transaction to reconstruct at a crash
point. MVCC remains the committed stage-4 target this ADR decides on
for concurrency; the WAL applies starting at stage 1.

## Decision

**Stage 1 (embedded core)**:

- **Concurrency control**: a single global lock. One writer at a time;
  it blocks all other readers and writers for the duration. No MVCC, no
  concurrent readers.
- **Durability**: a real **write-ahead log**, in its own file separate
  from the data file ([0011](0011-embedded-config.md)'s
  `<prefix>.log.bin`). **STEAL** and **NO-FORCE**: a dirty page may be
  written to disk before its transaction commits, and commit only
  requires the page's WAL record to be fsynced, not the data page
  itself. This needs both **redo** (a committed change may exist only
  in the WAL when a crash hits) and **undo** (an aborted or crashed
  transaction may already have a stolen page on disk) — the same
  logging discipline ARIES uses, though recovery itself is simpler than
  full ARIES: with only one writer ever active, there's at most one
  in-flight transaction to reconstruct at any crash point, so the usual
  analysis phase (rebuilding a multi-transaction winners/losers set)
  is trivial. Requires per-page LSNs (so redo can skip changes already
  applied, idempotently), a checkpoint mechanism (to bound how far back
  recovery replays), and compensation log records for undo (so a repeat
  crash mid-undo never re-undoes the same change twice).
- **Checkpoint mechanism**: fuzzy. Two in-memory tables — a **dirty
  page table** (page → recLSN, the LSN at which it was first dirtied
  since its last flush) and a **transaction table** (open transaction →
  start LSN, chained via each log record's **prevLSN** back to that
  transaction's previous record) — are snapshotted and written to the
  log as the checkpoint record, without pausing new transactions; the
  dirty pages they describe keep flushing in the background rather than
  being forced synchronously. A checkpoint fires on whichever comes
  first: the WAL growing past a size threshold, or a timeout since the
  last checkpoint.

**Stage 4 ("other concurrency mechanisms")** — across both storage
engines ([0004](0004-relational-storage-engine.md),
[0010](0010-hierarchical-storage-engine.md)), replacing stage 1's
concurrency mechanism:

- **Concurrency control**: shared **MVCC** — one versioning/visibility
  mechanism (transaction-ID stamping, snapshot-based visibility checks)
  reused by both engines, even though their physical page layouts differ.
- **Durability**: unchanged from stage 1 — the WAL adopted there is
  already the target mechanism, shared by both engines
  ([0004](0004-relational-storage-engine.md),
  [0010](0010-hierarchical-storage-engine.md)) so a transaction spanning
  both commits atomically through one log. Stage 4 only needs to extend
  it with MVCC's versioning metadata (transaction IDs, visibility info),
  not replace the logging mechanism itself.

## Consequences

- Stage 1's single global lock means zero read/write concurrency —
  every transaction, read or write, waits its turn. That's acceptable
  for an embedded, single-process engine finding its feet, but it's a
  full rework, not an incremental extension, when stage 4 replaces it
  with MVCC.
- Stage 1's WAL, checkpointing, and undo/redo logging are real
  implementation work up front — a meaningfully bigger lift than the
  originally-considered "atomic page writes + fsync, no log" scheme —
  but it means stage 4 doesn't redo the durability story later: only
  concurrency control changes at stage 4, durability doesn't.
- STEAL means a transaction's writeset no longer has to fit entirely in
  the buffer pool (unlike a NO-STEAL design), at the cost of needing
  real undo logging even for ordinary rollback (not just crash
  recovery), since a rolled-back transaction's dirty pages may already
  be on disk.
- The checkpoint's transaction table only records that a transaction
  was open *as of* the checkpoint; recovery still scans forward from
  there to the end of the log to check whether it went on to commit
  before the actual crash. Simpler than full multi-transaction ARIES
  analysis, not a replacement for it.
- Chaining every log record to its transaction's previous record via
  prevLSN isn't needed for undo correctness at stage 1 — single-writer
  concurrency means the log has no interleaving to disambiguate, so
  undo could just scan backward through the whole log unaided. It's
  included anyway so the log format doesn't need reworking once stage 4
  allows overlapping transactions, where prevLSN chaining becomes
  necessary rather than incidental.
- MVCC requires multi-version storage and eventual version reclamation
  (a Postgres-`VACUUM`-like mechanism) in both engines, once stage 4
  lands — accepted as the cost of readers/writers never blocking each
  other.
- The shared visibility mechanism must support multiple exposed isolation
  levels, not just one — [0005](0005-sql-support.md) commits to all five
  usual levels (Read Uncommitted, Read Committed, Repeatable Read,
  Snapshot, Serializable), configurable as a per-database server setting.
  Read Committed and Repeatable Read need their own visibility rules
  distinct from snapshot isolation's, and true Serializable needs
  conflict detection beyond what snapshot visibility alone provides.
- A single shared WAL means transaction atomicity across both engines
  comes for free, but couples their recovery paths together — a
  corruption/bug in one engine's log records affects crash recovery for
  the other engine's transactions too.
