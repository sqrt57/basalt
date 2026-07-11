# ADR 0005: SQL Support

Status: Decided (2026-07-10)

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
- **Isolation levels**: start with **snapshot isolation** only.

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
engine ([0004](0004-implementation-architecture.md)) are solid. Reusing
an existing embedded language runtime (e.g. WASM via Wasmtime) was
raised as a possible implementation path for procedural extensions during
the item 4 discussion and remains open for when this is picked up.

Isolation levels: [0004](0004-implementation-architecture.md) already
committed to shared MVCC as the concurrency mechanism across both
storage engines. Of the levels MVCC naturally supports, snapshot
isolation was chosen over read committed as the starting (and initially
only) level — stronger guarantees, and a closer match to what MVCC gives
you by default, over Postgres's actual default of read committed.

## Consequences

- Early query surface will be deliberately narrow; features get added
  incrementally rather than attempting broad ANSI SQL coverage upfront.
- Syntax/semantics decisions for the initial subset should default to
  "what would Postgres do here" to avoid dialect drift that would need
  to be unwound later for item 7.
- No procedural SQL (stored procedures/functions) in the initial scope —
  revisit once the core executor and SQL subset are stable.
- Only snapshot isolation needs to be exposed and tested initially — read
  committed, repeatable read, and serializable are deferred, not ruled
  out.

