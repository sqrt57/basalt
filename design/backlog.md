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
- **Order after roadmap stage 4** — the hierarchical storage engine
  ([0010](decisions/0010-hierarchical-storage-engine.md)) and TUI/GUI/
  Web ([0006](decisions/0006-admin-dev-client.md)) are both unstaged
  past [roadmap.md](roadmap.md) stage 4; their relative order isn't
  decided.
- **Stage-4 concurrent-writer mechanism** — stage 1's single evolving
  root pointer ([0008](decisions/0008-concurrency-durability.md))
  doesn't support concurrent writers as-is; reconciling independent
  copy-on-write changes into one next root (or moving away from a
  single root pointer) isn't decided. If root history stops being
  linear, stage 1's low-water-mark page reclamation
  ([0013](decisions/0013-page-reclamation.md)) stops working too and
  would need to move to per-page refcounting or another scheme — also
  not decided.
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
- **Serializable isolation implementation strategy** — stage 1's
  single-writer restriction may make Serializable reachable for free
  (no concurrent writers means no write skew, the anomaly that
  otherwise separates Snapshot from Serializable); whether that's
  actually true, and worth implementing opportunistically at stage 1,
  isn't confirmed. Once stage 4 brings concurrent writers online
  ([roadmap.md](roadmap.md),
  [0008](decisions/0008-concurrency-durability.md)), that free ride
  ends and true Serializable needs real conflict detection on top of
  the shared MVCC substrate (e.g. Postgres-style SSI/predicate locking
  vs. an alternative). Not decided ([0005](decisions/0005-sql-support.md)).
- **Read Uncommitted implementation strategy** — relevant from stage 1
  onward, since MVCC (and the Snapshot-level consistency it gives by
  default) already exists there: how dirty reads would actually be
  surfaced under this versioned storage model when weaker visibility is
  explicitly requested. Not decided
  ([0005](decisions/0005-sql-support.md)).

- **Embedded config surface beyond DB file location** — isolation
  level, memory/cache limits, sync/fsync policy, read-only mode, and
  anything else the embedded builder/options should expose. Only file
  location (path prefix) is decided so far
  ([0011](decisions/0011-embedded-config.md)).
- **Server's own config file location** — default path next to the
  executable, a CLI flag pointing at it, or something else. Not decided
  ([0012](decisions/0012-server-config.md)).
- **CLI flag / env var overrides for server config** — whether values
  from the TOML file ([0012](decisions/0012-server-config.md)) can be
  overridden by CLI flags or env vars, and their precedence if so. Not
  decided.

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
