# Basalt — Open Design Questions

Basalt is a database engine (code name) in the design stage. This
document tracks **open** design/implementation questions — the original
top-level architectural backlog (scope, implementation platform,
supported OSes, storage/execution architecture, SQL support level,
admin/dev client, wire protocol) is now fully decided. See
[architecture.md](architecture.md) for the current-state summary and
`decisions/` for the full ADRs.

What remains here are sub-questions that surfaced while resolving those
ADRs but weren't settled at the time, plus possibilities that were
raised and set aside without a commitment either way.

## Open Implementation Questions

- **TUI vs. GUI build order** — CLI ships first
  ([0006](decisions/0006-admin-dev-client.md)), but the relative order
  of TUI and GUI after that is undecided.
- **Web console frontend stack** — expected to reuse the Tauri GUI's
  frontend ([0006](decisions/0006-admin-dev-client.md)), but the actual
  framework/stack choice hasn't been made.
- **Native wire protocol design** — text vs. binary framing,
  request/response shape, auth, streaming results, etc. Flagged as
  unresolved in [0007](decisions/0007-wire-protocol.md).
- **Supporting client crates** — `clap` (CLI flag parsing) and a
  table-rendering crate (`comfy-table` vs. `tabled`) are likely
  defaults, not firmly confirmed
  ([0006](decisions/0006-admin-dev-client.md)).
- **Serializable isolation implementation strategy** — the shared MVCC
  substrate ([0008](decisions/0008-concurrency-durability.md))
  gives snapshot-style visibility naturally; true Serializable needs
  additional conflict detection on top (e.g. Postgres-style SSI/
  predicate locking vs. an alternative). Not decided
  ([0005](decisions/0005-sql-support.md)).
- **Read Uncommitted implementation strategy** — how dirty reads are
  actually surfaced under a versioned (MVCC) storage model, where reads
  normally see a consistent snapshot rather than in-flight writes. Not
  decided ([0005](decisions/0005-sql-support.md)).
- **In-process/embedded linking mode** — optional if it comes for free:
  gated on whether the storage/execution engine core
  ([0004](decisions/0004-storage-engines.md)) ends up as an
  independently linkable crate separate from the network/wire-protocol
  layer ([0007](decisions/0007-wire-protocol.md)). Not committed either
  way; revisit once the engine/workspace crate layout exists.
  ([0001](decisions/0001-scope.md))

## Deferred, Not Yet Scheduled

Raised as possibilities but with no committed timeline — not ruled out,
just not planned:

- **Postgres wire-protocol compatibility** — an optional possibility,
  not a committed direction ([0007](decisions/0007-wire-protocol.md):
  native protocol is the actual target; Postgres compatibility may be
  revisited later but isn't assumed).
- MySQL wire protocol.
- Standardized ODBC / JDBC / ADO.NET *provider* layers (e.g. a
  `DbConnection`-based ADO.NET provider, not just a plain native .NET
  client library) — no longer assumed to arrive "for free" via Postgres
  compatibility, since that's now optional rather than committed
  ([0007](decisions/0007-wire-protocol.md)). Distinct from the native
  Rust/C/.NET client libraries already committed to in
  [0007](decisions/0007-wire-protocol.md).
- Native client libraries for languages beyond the initial Rust/C/.NET
  set (Python, Node, Go, Java, etc.) — unscheduled
  ([0007](decisions/0007-wire-protocol.md)).
- A general REST/HTTP + JSON data API (distinct from the narrow HTTP
  endpoint that serves the bundled web console — see
  [0006](decisions/0006-admin-dev-client.md) /
  [0007](decisions/0007-wire-protocol.md)).
- A standard replication/CDC hook (logical-decoding-style).
- OS support beyond the committed FreeBSD tier — DragonflyBSD, OpenBSD,
  NetBSD, illumos, macOS ("maybe sometime," see
  [0003](decisions/0003-supported-os.md)).
