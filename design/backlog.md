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

## Deferred, Not Yet Scheduled

Raised as possibilities but with no committed timeline — not ruled out,
just not planned:

- **Postgres wire-protocol compatibility** — still the decided eventual
  direction ([0007](decisions/0007-wire-protocol.md): native protocol
  first, Postgres compatibility later), but pulled out of the active
  build order ([roadmap.md](roadmap.md)) — no committed timeline for
  starting it.
- MySQL wire protocol.
- ODBC / JDBC drivers.
- A general REST/HTTP + JSON data API (distinct from the narrow HTTP
  endpoint that serves the bundled web console — see
  [0006](decisions/0006-admin-dev-client.md) /
  [0007](decisions/0007-wire-protocol.md)).
- A standard replication/CDC hook (logical-decoding-style).
- OS support beyond the committed FreeBSD tier — DragonflyBSD, OpenBSD,
  NetBSD, illumos, macOS ("maybe sometime," see
  [0003](decisions/0003-supported-os.md)).
