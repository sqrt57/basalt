# ADR 0005: SQL Support

Status: Proposed (2026-07-10), revised (2026-07-14)

## Context

Per [backlog.md item 5](../backlog.md#5-sql-support), this item is
tightly coupled to item 7 (Postgres wire protocol) — if Basalt wants to
reuse Postgres client tooling eventually, the dialect should track
Postgres semantics closely enough that common queries "just work." That
coupling is why Postgres was chosen as the dialect target even though the
initial implementation will only cover a simple subset: the target
informs the shape of the subset now, not just a later rewrite.

Procedural extensions follow the same "distant goal, simple start"
pattern as the SQL level itself — a PL/pgSQL-like language is the
eventual target, but isn't needed until the core SQL subset and execution
engine ([0009](0009-query-execution.md)) are solid. Reusing
an existing embedded language runtime (e.g. WASM via Wasmtime) was
raised as a possible implementation path for procedural extensions during
the item 4 discussion and remains open for when this is picked up.

Isolation levels: this reverses the earlier snapshot-isolation-only
decision. [0008](0008-concurrency-durability.md) already committed
to shared MVCC as the concurrency mechanism across both storage engines;
that choice doesn't force a single exposed isolation level, and
Postgres itself proves the pattern of offering several levels (Read
Committed, Repeatable Read, Serializable) over one MVCC substrate.
Restricting Basalt to snapshot isolation only would have been a
narrower guarantee surface than what users reasonably expect to be able
to choose. Unlike Postgres's per-transaction/per-session
`SET TRANSACTION ISOLATION LEVEL`, Basalt fixes the isolation level as a
per-database server setting — one level applies to all transactions
against a given database, not chosen per transaction. This is a
deliberate simplification, not a dialect-compatibility concern; it
narrows what the transaction layer needs to track (no per-transaction
level state) at the cost of per-transaction flexibility Postgres offers.

## Decision

- **Level**: start with a **simple SQL subset** — basic DDL/DML
  (`CREATE TABLE`, `SELECT`/`INSERT`/`UPDATE`/`DELETE`, straightforward
  `WHERE`/`JOIN`), growing over time. No exotic or Postgres-only features
  in the initial subset.
- **Dialect compatibility target**: **Postgres-compatible**, as a distant
  goal — even the initial subset should track Postgres syntax/semantics
  rather than inventing divergent syntax, since retrofitting Postgres
  compatibility onto an already-diverged dialect is harder than growing
  toward it from day one.
- **Procedural extensions**: distant goal is a **PL/pgSQL-like language**,
  consistent with the Postgres target. Starts minimal/none and grows
  alongside the SQL subset itself, on the same phased trajectory.
- **Isolation levels**: **configurable as a per-database server
  setting** (not per-transaction/per-session), targeting the usual
  five — **Read Uncommitted, Read Committed, Repeatable Read, Snapshot,
  Serializable** — rather than exposing only one, though like the SQL
  subset, support can be built up incrementally from the simplest level
  rather than landing all five at once. This entire axis is moot for
  stage 1 ([0008](0008-concurrency-durability.md)): a single global lock
  makes every transaction trivially serial, with no isolation-level
  choice to expose. The five-level buildup begins once stage 4 brings
  MVCC online.

## Consequences

- The SQL subset and the isolation levels supported are independent
  growth axes raised by this ADR — each can be built starting from its
  simplest option and expanded incrementally, rather than needing to
  land complete at once.
- Early query surface will be deliberately narrow; features get added
  incrementally rather than attempting broad ANSI SQL coverage upfront.
- Syntax/semantics decisions for the initial subset should default to
  "what would Postgres do here" to avoid dialect drift that would need
  to be unwound later for item 7.
- No procedural SQL (stored procedures/functions) in the initial scope —
  revisit once the core executor and SQL subset are stable.
- Isolation levels can likewise be added one at a time rather than all
  five (Read Uncommitted, Read Committed, Repeatable Read, Snapshot,
  Serializable) landing together. **Snapshot** is the natural starting
  point once stage 4 brings MVCC online
  ([0008](0008-concurrency-durability.md)) — stage 1 predates MVCC
  entirely and has no isolation-level surface at all — since Snapshot
  falls directly out of the shared MVCC substrate's native visibility
  rules, with Read Committed, Repeatable Read, Serializable, and Read
  Uncommitted layered on afterward — more surface area for the
  transaction layer to eventually cover than a single fixed level, but
  not all required up front. Since
  it's a per-database setting rather than per-transaction, there's no
  need for transaction-scoped isolation-level state or
  `SET TRANSACTION ISOLATION LEVEL`-style syntax, but changing a
  database's level is an administrative operation, not something a
  client can do mid-session.
- True Serializable is a stronger guarantee than snapshot isolation
  alone provides; how it's actually implemented on top of the shared
  MVCC substrate (e.g. Postgres-style SSI/predicate-conflict detection
  vs. locking) is not resolved by this ADR — see
  [backlog.md](../backlog.md).
- Read Uncommitted is a weaker guarantee than plain MVCC snapshot reads
  naturally give; how dirty reads are actually surfaced under a
  versioned storage model is also not resolved here — see
  [backlog.md](../backlog.md).
