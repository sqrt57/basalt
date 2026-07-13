# ADR 0009: Query Execution Model

Status: Proposed (2026-07-10)

## Context

Split out from the original implementation-architecture discussion
([0004](0004-relational-storage-engine.md),
[0010](0010-hierarchical-storage-engine.md)) since query execution
strategy is a separate concern from storage engine design, driven by
the two engines exposing a common iterator interface rather than by
either engine's internals.

Three approaches were considered — a tree-walking interpreter (Postgres-
style), a bytecode VM (SQLite-style), and compiled query plans. A
JIT-compiled path via Cranelift (the backend Wasmtime uses) was explored
as a way to get compiled-plan performance without writing a full
compiler backend from scratch, and briefly adopted before being reverted
in favor of the simplest option: a plain tree-walking interpreter.
Embedding a general-purpose scripting runtime (Lua, a JS engine, a
managed VM) for the core executor was considered and rejected —
marshaling typed column values into a dynamic scripting language's value
representation per row/predicate adds overhead that tends to outweigh
any implementation savings, and a managed runtime reintroduces the
GC-latency concern [0002](0002-implementation-platform.md) already
rejected by choosing Rust. Reusing an embedded runtime remains a
plausible fit for procedural extensions/stored procedures (backlog item
5), which is a separate, lower-frequency execution path — not decided
here.

## Decision

Query execution is a plain **tree-walking interpreter** (Volcano/
iterator model) — the query plan is a tree of operator nodes, executed
by recursively pulling rows from the root down to leaf scans. No
bytecode VM, no JIT/compiled query plans.

## Consequences

- Tree-walking execution has real per-row interpretation overhead
  compared to a bytecode VM or compiled plans, but is the simplest to
  build correctly first, matches the logical query plan most directly,
  and keeps the door open to revisiting execution strategy later once
  correctness is established.
- Both storage engines' scan/seek operators need to expose a common
  iterator interface ([0004](0004-relational-storage-engine.md),
  [0010](0010-hierarchical-storage-engine.md)) so the tree-walking
  executor can drive either engine without engine-specific executor
  code.
