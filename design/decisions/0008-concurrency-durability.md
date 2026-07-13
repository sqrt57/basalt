# ADR 0008: Concurrency Control & Durability

Status: Proposed (2026-07-10)

## Context

Split out from the original implementation-architecture discussion
([0004](0004-storage-engines.md)) since concurrency control and
durability apply across both storage engines rather than being a
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

## Decision

Across both storage engines ([0004](0004-storage-engines.md)):

- **Concurrency control**: shared **MVCC** — one versioning/visibility
  mechanism (transaction-ID stamping, snapshot-based visibility checks)
  reused by both engines, even though their physical page layouts differ.
- **Durability**: a single shared **write-ahead log (WAL)** — one log
  stream, one crash-recovery pass, one durability boundary, regardless of
  whether a transaction touches the relational engine, the hierarchical
  engine, or both. No cross-engine two-phase commit between independent
  logs.

## Consequences

- MVCC requires multi-version storage and eventual version reclamation
  (a Postgres-`VACUUM`-like mechanism) in both engines — accepted as the
  cost of readers/writers never blocking each other.
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
