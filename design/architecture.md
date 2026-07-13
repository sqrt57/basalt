# Basalt — Proposed Architecture

Snapshot of Basalt's design as of 2026-07-11, synthesized from every ADR
in `design/decisions/`. All ADRs are currently **proposed**, not yet
decided. This file is for orientation — the ADRs remain the
authoritative record of *why* each decision was made; if this ever
drifts out of sync with an ADR, the ADR wins. Open questions not yet
resolved are tracked in `design/backlog.md`, not here.

## What Basalt Is

Single-node, client/server relational database engine, built as a solo
learning/research project. Client/server is the primary and initial
target; distributed/clustered operation and heavy enterprise tooling are
explicitly out of scope for now. An in-process/embedded mode is a
possible later stage — optional and not committed, added only if it
ends up free (see [backlog.md](backlog.md)). ([0001](decisions/0001-scope.md))

## Implementation Platform

Rust — chosen for native-level control without a GC, and because
learning systems programming (not just DB internals) is part of the
project's goal. ([0002](decisions/0002-implementation-platform.md))

## Supported Operating Systems

Server support is staged by priority:

1. **Windows** — day one.
2. **Linux** — next.
3. **FreeBSD** — much later, for ZFS.
4. **Maybe/sometime, uncommitted**: other BSDs (DragonflyBSD, OpenBSD,
   NetBSD), illumos (OmniOS/SmartOS), macOS.

Client tooling splits by interface form: CLI REPL and TUI aim to track
the server's platform list exactly; GUI is required only on
Windows + Linux, with other platforms best-effort.
([0003](decisions/0003-supported-os.md))

## Storage Engines

Two purpose-built storage engines, both B+-tree-backed, implementable
independently of each other and sharing only the concurrency/durability
substrate below:

- A **row store** for relational tables.
  ([0004](decisions/0004-relational-storage-engine.md))
- A **GT.M/globals-style sparse associative array** for hierarchical
  tables. ([0010](decisions/0010-hierarchical-storage-engine.md))

## Concurrency Control & Durability

Both storage engines share one **MVCC** concurrency mechanism and a
single **WAL** — one durability boundary and one crash-recovery path
regardless of which engine(s) a transaction touches.
([0008](decisions/0008-concurrency-durability.md))

## Query Execution

Starts as a plain tree-walking interpreter (Volcano/iterator model); a
bytecode VM (SQLite-style) is a plausible next stage once that's solid.
JIT/compiled query plans remain rejected outright — already explored via
Cranelift and reverted. ([0009](decisions/0009-query-execution.md))

## SQL Support

Starts with a simple SQL subset (basic DDL/DML: `CREATE TABLE`,
`SELECT`/`INSERT`/`UPDATE`/`DELETE`, straightforward `WHERE`/`JOIN`),
targeting Postgres-dialect compatibility as the distant goal. A
PL/pgSQL-like procedural language is a later addition. Isolation level
is a per-database server setting (not per-transaction), targeting the
usual five (Read Uncommitted, Read Committed, Repeatable Read, Snapshot,
Serializable) built up incrementally starting from Snapshot — native to
the shared MVCC substrate — rather than landing all five at once.
([0005](decisions/0005-sql-support.md))

## Client Tooling

One unified admin/dev client — not separate tools — covering both admin
operations (connection/server management, user/role management,
backup/restore, monitoring) and dev operations (query editor/REPL,
schema browsing, result display, explain-plan visualization). Shipped
as four interface forms sharing one Rust core library crate:

1. **CLI REPL/executor**, sqlcmd-style — built first, using
   **reedline** for line editing.
2. **TUI** — ratatui + crossterm.
3. **GUI** — Tauri (Rust backend, web frontend).
4. **Web** — browser-based console bundled directly into the server
   (HTTP-served, no separate process), built alongside/after GUI since
   it's expected to reuse GUI's frontend.

([0006](decisions/0006-admin-dev-client.md))

## Wire Protocol

Basalt speaks its **own native wire protocol** — this is the committed
target for client connectivity, and what the CLI REPL speaks. Postgres
wire-protocol compatibility is an optional possibility, not a committed
direction; it isn't assumed to happen, and standardized ODBC/JDBC/
ADO.NET provider access isn't assumed to arrive "for free" through it
either. Native client libraries are committed for Rust, C, and .NET
initially, Rust first (other languages later, unscheduled).
([0007](decisions/0007-wire-protocol.md))

See [roadmap.md](roadmap.md) for build order/sequencing derived from
these decisions.

## Open Questions

See `design/backlog.md` for implementation-level questions the ADRs
above left unresolved (CLI REPL crate choice, TUI/GUI build order,
native protocol wire format, and similar).
