# ADR 0002: Implementation Platform

Status: Proposed (2026-07-10)

## Context

Per [0001-scope.md](0001-scope.md), Basalt is a solo learning/research
project — the choice of platform should optimize for what the developer
gets out of building it, not for team velocity or enterprise support.

Two inputs settled this:

- **Learning focus includes systems programming**, not just database
  internals — manual-ish memory/IO control is itself part of the goal,
  which rules out reaching for a managed runtime (C#/.NET, Java, Go) to
  "get out of the way."
- **Language choice**: Rust specifically, over C/C++, to get that
  low-level control with memory safety rather than manual memory-safety
  burden. This trades some of the historical fidelity of "build it the
  way Postgres/SQLite are built" (C) for a substantially safer path to
  the same class of control — reasonable for a project where the person
  writing the storage engine is also its only reviewer.

## Decision

Basalt's core engine will be built in **Rust**.

## Consequences

- No garbage collector to reason about in the storage/execution core —
  latency and memory-layout decisions are explicit and in Basalt's
  control, which is part of the point.
- Rust's ecosystem for DB-adjacent building blocks (e.g. tokio for async
  I/O/networking, bytes/memmap crates, existing DB projects like sled/
  TiKV/LimboDB for reference) is usable as reference material and
  potentially as dependencies, without being forced into an existing
  engine's architecture.
- Expect real time cost paying down the borrow-checker/ownership learning
  curve if Rust is new — that cost is accepted as part of the learning
  goal, not a risk to mitigate away.
- No decision yet on splitting "core in Rust, tooling in something else"
  (backlog item 2's "Mixed" option) — default assumption is Rust
  end-to-end (server, and likely CLI tooling) unless a specific piece
  (e.g. a GUI client) later makes a strong case for another language.
