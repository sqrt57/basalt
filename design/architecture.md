# Basalt — Proposed Architecture

Snapshot of Basalt's design as of 2026-07-14, synthesized from every ADR
in `design/decisions/`. All ADRs are currently **proposed**, not yet
decided. This file is for orientation — the ADRs remain the
authoritative record of *why* each decision was made; if this ever
drifts out of sync with an ADR, the ADR wins. Open questions not yet
resolved are tracked in `design/backlog.md`, not here.

## What Basalt Is

Single-node relational database engine, built as a solo learning/
research project. Ships first as an in-process/embedded library — no
server process required; client/server mode, wrapping the same engine
core, is a committed second stage, not the first-stage goal.
Distributed/clustered operation and heavy enterprise tooling are
explicitly out of scope for now. ([0001](decisions/0001-scope.md))

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

The embedded core (stage 1) already runs on **MVCC**, just in its
simplest form ([roadmap.md](roadmap.md)): a copy-on-write B+-tree with
one root pointer, one write transaction at a time (fully serialized, no
write-write conflict handling needed), but concurrent readers via
snapshot visibility, never blocked by the active writer. A real
**write-ahead log** backs it from the start, in its own file separate
from the data file ([0011](decisions/0011-embedded-config.md)) —
STEAL/NO-FORCE, redo-only (no undo: an aborted or crashed transaction's
pages are simply unreferenced by any committed root), binary-diff log
records, fuzzy checkpointing. Old pages superseded by copy-on-write
writes are reclaimed via a low-water mark over an in-memory reader
table (the oldest active reader's snapshot generation), riding the
checkpoint pass's cadence rather than a separate mechanism — viable
because stage 1's root history is strictly linear; a long-running
reader can starve reclamation indefinitely, accepted as a stage-1
tradeoff. ([0013](decisions/0013-page-reclamation.md)) Both storage
engines later extend this to **concurrent writers** — a genuine open
problem, not just "more concurrency," since a single evolving root
pointer doesn't support it as-is, and reclamation would need to change
too once root history stops being linear — once that later stage
(roadmap stage 4) lands; durability is unchanged at that point.
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
Serializable). The embedded core (roadmap stage 1) already meets
**Snapshot** isolation from day one, not starting later — its
copy-on-write tree-snapshot reads ([0008](decisions/0008-concurrency-durability.md))
give every reader a fixed, fully consistent view of the whole database
for its transaction. Stage 1's single-writer restriction may even reach
**Serializable** for free (no concurrent writers means no write skew),
though that's not committed to as a guarantee; the remaining levels are
layered on as the buildup continues.
([0005](decisions/0005-sql-support.md))

## Client Tooling

One unified admin/dev client — not separate tools — covering both admin
operations (connection/server management, user/role management,
backup/restore, monitoring) and dev operations (query editor/REPL,
schema browsing, result display, explain-plan visualization). Shipped
as four interface forms sharing one Rust core library crate:

1. **CLI REPL/executor**, sqlcmd-style — built first, using
   **reedline** for line editing. Its first working form (roadmap stage
   2) runs in-process against the embedded engine core, no network;
   wire-protocol support follows once client/server (roadmap stage 3)
   exists.
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

## Embedded Configuration

A database is identified by a single path prefix; the engine derives
exactly two files from it — `<prefix>.data.bin` (both storage engines'
data, one shared page allocator) and `<prefix>.log.bin` (the shared
WAL). No directory convention, no per-engine file.
([0011](decisions/0011-embedded-config.md))

`<prefix>.data.bin` itself starts with a fixed 64-byte preamble (magic
bytes, format/version, page size), read before any page-sized I/O is
possible; page size is chosen per database at creation and immutable
after. Page 0 immediately follows the preamble and holds the free-list
head; free pages are threaded into a singly-linked list among
themselves. ([0015](decisions/0015-page-storage-format.md))

Allocated pages hold a copy-on-write B+-tree: a slotted-page node
format (byte-string keys/values, no fixed fanout, no overflow pages),
where a write decodes the affected node, re-encodes it into a newly
allocated page, and threads the new child pointer up through freshly
copied ancestors to a new root — old pages are left untouched. No
leaf-sibling pointers; range-scan instead walks a cursor stack up and
back down the tree. The tree's root is tracked only by the in-process
handle for now, not persisted anywhere durable.
([0016](decisions/0016-btree-node-format.md))

`<prefix>.log.bin` gives that root a durable recovery path: a 64-byte
preamble like the data file's, then a sequence of checksummed records
(LSN = a record's own file offset) — `PageDiff` (a base page plus
changed byte ranges, redoing one newly-allocated page), `Commit` (the
new root, durable once its record is fsynced), and `Checkpoint` (a
dirty-page-table snapshot, fsyncing the data file behind it). Redo
scans the whole log forward and stops cleanly at the first torn/corrupt
record; the current root is whichever `Commit` was seen last.
([0017](decisions/0017-wal-format.md))

A reader transaction is a lightweight `Snapshot` token (its generation
and root, `Copy`), not a live borrow — reads go through `Database`
methods taking `&mut self` plus the token, re-acquiring the pager each
call rather than holding it for the snapshot's whole lifetime, so a
writer can commit in between two reads made through the same open
snapshot. Generations are an in-memory `u64` counter, incremented once
per commit and never persisted; a `BTreeMap<generation, open_count>`
reader table gives the low-water mark as its minimum key. Pages a
transaction's writes superseded are collected provisionally and only
folded into the durable pending-reclaim list, tagged with the new
generation, on that transaction's successful commit — discarded outright
on abort, since an aborted transaction never actually superseded
anything. The checkpoint pass reclaims (frees) whatever in that list
falls at or below the current low-water mark.
([0018](decisions/0018-mvcc-snapshot-format.md))

## Server Configuration

A TOML config file lists the databases a server process hosts — each a
`name` (what clients connect to) paired with a `path` prefix
([0011](decisions/0011-embedded-config.md)), absolute or relative to the
server executable's own directory — plus the listen address/port.
([0012](decisions/0012-server-config.md))

## Crate Layout

A Cargo workspace: `engine` (the embedded core — storage engines,
MVCC/WAL, SQL, execution; no networking) is the only crate stage 1 needs.
`basalt-proto` (wire protocol types) and `basalt-server`/`basalt-client`
(server binary and native Rust client library) are reserved slots that
fill in at stage 3. `basalt-client-core` holds the logic shared by all
four unified-client interface forms and is the seam that lets them run
in-process (stage 2, against `engine` directly) or networked
(stage 3+, against `basalt-client`) without duplicating client logic;
`basalt-cli`/`basalt-tui`/`basalt-gui` are thin binaries wiring that core
to their respective toolkits. The web frontend itself lives in its own
crate, `basalt-web-ui`, consumed both by `basalt-gui`'s webview and by
`basalt-server`'s bundled web console.
([0014](decisions/0014-crate-layout.md))

See [roadmap.md](roadmap.md) for build order/sequencing derived from
these decisions.

## Open Questions

See `design/backlog.md` for implementation-level questions the ADRs
above left unresolved (CLI REPL crate choice, TUI/GUI build order,
native protocol wire format, and similar).
