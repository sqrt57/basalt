# ADR 0005: SQL Support

Status: Proposed (2026-07-10), revised (2026-07-14), revised (2026-07-15)

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

[0008](0008-concurrency-durability.md)'s later revision replaced stage
1's originally planned plain global lock with a copy-on-write
tree-snapshot design, still for stage 1. That changes the isolation
picture this ADR originally assumed: a stage-1 reader already gets a
fixed, fully consistent view of the whole database for its entire
transaction, which is real Snapshot Isolation, not the "moot, trivially
serial" state a plain lock would give. Separately, stage 1's
single-writer restriction rules out write skew — the anomaly that
normally separates Snapshot from Serializable, since it needs two
writers in flight at once — which raises the possibility that stage 1
could deliver Serializable for free too. That stronger claim isn't
adopted here: it's noted as worth implementing opportunistically if it
turns out to need no real extra work, rather than committed to as a
guarantee.

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
  rather than landing all five at once. Stage 1 already clears the
  **Snapshot** bar from day one, not starting at stage 4 as originally
  framed: its copy-on-write tree-snapshot reads
  ([0008](0008-concurrency-durability.md)) give every reader a fixed,
  fully consistent view of the whole database for its entire
  transaction, with no separate mechanism needed. **Serializable** may
  be reachable at stage 1 too, essentially for free, since stage 1's
  single-writer restriction rules out write skew — but this isn't
  committed to as a guarantee here: implement it opportunistically if it
  needs no real extra work, otherwise let it arrive properly once the
  isolation-level buildup reaches Serializable on its own (relevant
  again once stage 4 brings concurrent writers and reopens the
  write-skew question — see [backlog.md](../backlog.md)).

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
  Serializable) landing together. **Snapshot** is the starting point,
  already met from stage 1 onward
  ([0008](0008-concurrency-durability.md)) since it falls directly out
  of the copy-on-write tree-snapshot reads stage 1 already has, not
  something that waits for stage 4. Read Committed, Repeatable Read,
  Serializable, and Read Uncommitted are layered on afterward — more
  surface area for the transaction layer to eventually cover than a
  single fixed level, but not all required up front. Since it's a
  per-database setting rather than per-transaction, there's no need for
  transaction-scoped isolation-level state or
  `SET TRANSACTION ISOLATION LEVEL`-style syntax, but changing a
  database's level is an administrative operation, not something a
  client can do mid-session.
- True Serializable is a stronger guarantee than Snapshot Isolation
  alone provides in general, though stage 1's single-writer restriction
  may already rule out the gap between them (write skew needs two
  writers in flight at once). Whether that gap is actually closed for
  free at stage 1, and how Serializable is implemented once stage 4's
  concurrent writers reopen it (e.g. Postgres-style SSI/predicate-
  conflict detection vs. locking), is not resolved by this ADR — see
  [backlog.md](../backlog.md).
- Read Uncommitted is a weaker guarantee than the Snapshot-level
  consistency stage 1's tree-snapshot reads already give by default;
  how dirty reads would actually be surfaced under this versioned
  storage model, when weaker visibility is explicitly requested, is not
  resolved here — see [backlog.md](../backlog.md).
