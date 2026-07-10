# Basalt — Initial Design Backlog

Basalt is a new database engine (code name). This document lists the open
architectural decisions that need to be made before implementation starts.
Each item frames the question, the options on the table, and relevant
precedent from existing systems. Nothing here is decided yet — this is the
starting point for discussion.

## 1. Scope — DECIDED

See [decisions/0001-scope.md](decisions/0001-scope.md). Basalt is a
single-node, client/server relational database engine, built as a solo
learning/research project. Distributed/clustered operation, embedded/
in-process linking, and heavy enterprise tooling are explicitly out of
scope for now. Original framing kept below for reference.

What kind of database is Basalt?

- **In-process / embedded, no server** — like SQLite: linked into the host
  process, single file or in-memory, no network protocol required.
- **Embedded with optional server mode** — like a hybrid (e.g. DuckDB,
  H2, HSQLDB): usable as a library, but can also run standalone and speak
  a wire protocol.
- **Client/server, single-node** — like early Postgres/MySQL: a standalone
  server process, one machine, no built-in clustering.
- **Enterprise-grade, distributed/clustered** — like MS SQL Server,
  IRIS, or Postgres with extensions: HA, replication, sharding,
  multi-node transactions, enterprise tooling (backup, monitoring, RBAC).

This choice drives almost everything else (platform, architecture,
interfaces). Needs to be settled first.

~~**Open question:** is the goal a learning/research project, a niche
special-purpose engine (e.g. optimized for a particular data model), or a
general-purpose competitor to existing engines?~~ Answered: learning/
research project — see decision above.

## 2. Implementation Platform — DECIDED

See [decisions/0002-implementation-platform.md](decisions/0002-implementation-platform.md).
Basalt's core will be built in Rust — chosen for native-level control
without a GC, and because learning systems programming (not just DB
internals) is part of the project's goal. Original framing kept below
for reference.

Language/runtime for the core engine.

- **C / C++** — max control over memory layout and I/O, matches SQLite/
  Postgres/MySQL precedent, no GC pauses, but slower dev velocity and
  memory-safety burden.
- **Rust** — C-level performance and control with memory safety, growing
  ecosystem for DB engines (sled, TiKV, SurrealDB, LimboDB/Turso).
- **Managed language (C#/.NET, Java, Go)** — faster to build and iterate,
  good tooling, GC behavior is a real concern for a latency-sensitive
  storage engine; Go is used by CockroachDB/TiDB companion tools, Java by
  H2/Derby/Neo4j.
- **Mixed** — safety-critical storage core in one language, tooling/
  clients in another.

~~**Open question:** does the team's existing expertise (the org works
heavily in C#/.NET per other repos) push toward a managed-language core
with careful attention to GC/pinning, or is a native core worth the
investment?~~ Answered: Rust — see decision above.

## 3. Supported Operating Systems

- Windows only (matches org's primary platform, per Acumatica context)
- Windows + Linux (typical server matrix)
- Windows + Linux + macOS (adds dev-machine convenience)
- Server OSes only vs. also supporting desktop/dev-machine use for the
  embedded scenario

Platform choice interacts with storage layer decisions (file I/O, memory
mapping, direct I/O availability, filesystem guarantees for durability).

## 4. Implementation Architecture

Reference points to study and position against:

| System | Model | Notable architecture traits |
|---|---|---|
| PostgreSQL | Client/server, relational | Process-per-connection, MVCC via tuple versions, WAL, extensible types/index AMs |
| SQLite | Embedded, relational | Single file, B-tree pager, serialized writers, no server |
| MS SQL Server | Client/server, relational, enterprise | Cost-based optimizer, row+column store, clustered indexes, rich HA/DR |
| InterSystems IRIS | Multi-model (object/relational/global), embedded engine + server | Sparse array "globals" as the universal storage primitive, SQL and object views layered on top |
| GT.M / YottaDB | Embedded KV engine ("globals"), hierarchical sparse arrays | M (MUMPS) language, extreme write throughput, replication, used as the engine under IRIS-like systems |

Decisions to make:

- **Storage model**: row store, column store, or a sparse
  hierarchical/associative array (à la GT.M/YottaDB "globals", which IRIS
  builds SQL on top of) as the universal primitive.
- **Concurrency control**: MVCC (Postgres-style), locking (SQL Server
  classic), or single-writer serialization (SQLite-style).
- **Durability**: WAL/redo log design, checkpointing strategy.
- **Query execution**: interpreted tree-walking vs. bytecode VM
  (SQLite uses a custom VM) vs. compiled query plans.
- **Extensibility**: pluggable storage engines (MySQL-style) vs. single
  fixed engine (Postgres/SQL Server-style, though Postgres allows FDWs/
  custom index AMs).

**Note:** GT.M/YottaDB and IRIS represent a genuinely different
architectural family (schema-less sparse arrays with SQL bolted on) vs.
the Postgres/SQLite/SQL Server family (native relational storage). Worth
deciding early whether Basalt's core primitive is relational tuples or a
more primitive associative structure that relational is built on top of.

## 5. SQL Support

- **Level**: no SQL (pure KV/document API), SQL-92 subset, broad
  ANSI SQL with common extensions, or full Postgres-dialect compatibility.
- **Dialect compatibility target**: none, Postgres-compatible dialect,
  or a novel dialect.
- **Procedural extensions**: none, simple stored procedures, or a full
  PL/pgSQL-like language.
- Transactions/isolation levels to support (read committed, snapshot,
  serializable).

This is tightly coupled to item 7 (Postgres wire protocol) — if Basalt
wants to reuse Postgres client tooling, the SQL dialect should track
Postgres semantics closely enough that common queries "just work."

## 6. Administration Client (GUI/TUI)

- Do we build one at all initially, or rely on third-party tools via a
  compatible wire protocol (see item 8)?
- If built: GUI (cross-platform, e.g. Avalonia/Electron/web-based) vs.
  TUI (terminal, e.g. a `psql`-like REPL) vs. both, starting with TUI.
- Scope: connection/server management, user/role management, backup/
  restore, monitoring, query console.

## 7. Development Client (GUI/TUI)

- Query editor / REPL, schema browser, result grid, explain-plan
  visualization.
- Overlap with item 6 — many DBs ship one tool that covers both admin
  and dev use cases (e.g. Azure Data Studio, DBeaver, pgAdmin) rather than
  two separate tools. Decide whether Basalt follows suit or keeps them
  split.
- Build vs. rely on generic third-party SQL clients (DBeaver, DataGrip)
  via a standard wire protocol — likely the pragmatic starting point.

## 8. Postgres Wire Protocol Compatibility

Goal: implement enough of the Postgres frontend/backend protocol (and
enough SQL semantics) that existing Postgres clients and GUIs work
against Basalt with no modification.

- **Protocol layer**: implement the Postgres wire protocol (startup,
  simple + extended query, COPY, SSL negotiation) — this is the
  well-documented, versioned part and is the highest-leverage single
  piece of compatibility work, since it unlocks `psql`, pgAdmin, DBeaver,
  most ORMs and drivers (npgsql, psycopg, JDBC) for free.
- **Semantic layer**: how much of Postgres's actual SQL dialect, system
  catalogs (`pg_catalog`), and error codes need to be emulated for tools
  to work well, not just connect. GUI tools often query catalog tables
  directly for schema browsing — that's usually the hard part, not the
  wire protocol itself.
- Precedent worth studying: CockroachDB, YugabyteDB, and Amazon Redshift
  all implement "Postgres wire compatible, own storage/execution engine"
  — a proven pattern for exactly this goal.

**Other valuable interfaces to consider** (beyond Postgres wire protocol):

- **ODBC / JDBC** drivers — needed for BI tools and legacy enterprise
  clients regardless of wire protocol choice.
- **MySQL wire protocol** — second-most-common "speak this and get free
  tooling" target; broadens client compatibility if Basalt's dialect can
  flex toward MySQL semantics too.
- **REST/HTTP + JSON API** — for lightweight/web/serverless clients,
  following PostgREST or Supabase's pattern of layering REST over a
  Postgres-speaking core.
- **A native/lightweight binary protocol** — for the embedded scenario,
  bypassing wire-protocol overhead entirely when used in-process.
- **Standard replication/CDC hook** — a logical-replication-style output
  (à la Postgres logical decoding) so external tools can stream changes;
  increasingly expected by integrators.

## Suggested Sequencing

1. Settle scope (item 1) and platform (item 2) — everything else depends
   on these.
2. Prototype the storage/architecture core (item 4) against a minimal
   subset of SQL (item 5) before building any client.
3. Get a bare-bones Postgres wire protocol handshake + simple query
   working early (item 8) — it's the cheapest way to get `psql` and a
   GUI client talking to Basalt for testing, ahead of building bespoke
   tooling (items 6–7).
4. Layer admin/dev tooling on top once the wire protocol and core SQL
   surface are stable enough to be worth building a UI against.
