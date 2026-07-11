# ADR 0004: Implementation Architecture

Status: Decided (2026-07-10)

## Decision

Basalt has **two separate, purpose-built storage engines** rather than a
single universal storage primitive or an arbitrary-plugin architecture:

- **Relational tables**: a row store — tuples stored together, tuple-per-
  page layout — matching the Postgres/MySQL/SQL Server family.
- **Hierarchical tables**: a sparse hierarchical/associative array, in the
  style of GT.M/YottaDB "globals," with its own multidimensional-subscript
  API distinct from the relational one — rather than SQL being the only
  entry point to it (unlike IRIS, which builds SQL as the sole layer on
  top of globals).

Both engines are physically backed by **B+-trees**.

Across both engines:

- **Concurrency control**: shared **MVCC** — one versioning/visibility
  mechanism (transaction-ID stamping, snapshot-based visibility checks)
  reused by both engines, even though their physical page layouts differ.
- **Durability**: a single shared **write-ahead log (WAL)** — one log
  stream, one crash-recovery pass, one durability boundary, regardless of
  whether a transaction touches the relational engine, the hierarchical
  engine, or both. No cross-engine two-phase commit between independent
  logs.
- **Query execution**: a simple **tree-walking interpreter** (Volcano/
  iterator model) — the query plan is a tree of operator nodes, executed
  by recursively pulling rows from the root down to leaf scans. No
  bytecode VM, no JIT/compiled query plans.

## Context

Reference points considered (see
[backlog.md item 4](../backlog.md#4-implementation-architecture)):
PostgreSQL, SQLite, MS SQL Server, InterSystems IRIS, GT.M/YottaDB.

**Storage model**: the backlog framed a choice between row store, column
store, and a single universal sparse-array primitive (the IRIS/GT.M
pattern, where relational tables are encoded as subscripts over one
underlying tree). A universal-primitive design was explored in detail —
encoding rows as `(table_id, primary_key, column_id) → value` entries,
relying on B+-tree leaf ordering/linking to keep a row's columns
physically adjacent — and rejected in favor of keeping relational and
hierarchical data as genuinely separate engines with separate APIs. This
avoids forcing SQL to be the only way to reach hierarchical data, and
avoids re-deriving relational schema/type/constraint enforcement on top
of a storage layer that has no concept of "column." The cost is two
storage engines to build and maintain instead of one, and no automatic
data-model interop between them at the storage layer.

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

**Query execution**: three approaches were considered — a tree-walking
interpreter (Postgres-style), a bytecode VM (SQLite-style), and compiled
query plans. A JIT-compiled path via Cranelift (the backend Wasmtime
uses) was explored as a way to get compiled-plan performance without
writing a full compiler backend from scratch, and briefly adopted before
being reverted in favor of the simplest option: a plain tree-walking
interpreter. Embedding a general-purpose scripting runtime (Lua, a JS
engine, a managed VM) for the core executor was considered and rejected —
marshaling typed column values into a dynamic scripting language's value
representation per row/predicate adds overhead that tends to outweigh
any implementation savings, and a managed runtime reintroduces the
GC-latency concern [0002](0002-implementation-platform.md) already
rejected by choosing Rust. Reusing an embedded runtime remains a
plausible fit for procedural extensions/stored procedures (backlog item
5), which is a separate, lower-frequency execution path — not decided
here.

## Consequences

- Two storage engines to design, build, and maintain — more surface area
  than a single universal primitive, but each engine's internals stay
  simpler and closer to well-understood precedent (row store ≈ Postgres/
  MySQL; globals ≈ GT.M/YottaDB).
- No built-in interop between relational and hierarchical data at the
  storage layer; any bridging (e.g. exposing hierarchical data through
  SQL) would be a higher-level feature layered on top, not a storage-
  layer given.
- MVCC requires multi-version storage and eventual version reclamation
  (a Postgres-`VACUUM`-like mechanism) in both engines — accepted as the
  cost of readers/writers never blocking each other.
- A single shared WAL means transaction atomicity across both engines
  comes for free, but couples their recovery paths together — a
  corruption/bug in one engine's log records affects crash recovery for
  the other engine's transactions too.
- Tree-walking execution has real per-row interpretation overhead
  compared to a bytecode VM or compiled plans, but is the simplest to
  build correctly first, matches the logical query plan most directly,
  and keeps the door open to revisiting execution strategy later once
  correctness is established.
- Both engines' scan/seek operators need to expose a common iterator
  interface so the tree-walking executor can drive either engine without
  engine-specific executor code.

