# ADR 0012: Server Configuration

Status: Proposed (2026-07-14)

## Context

[0001](0001-scope.md) makes client/server a second stage wrapping the
same embedded engine core. Unlike embedded mode — one path prefix per
library instance ([0011](0011-embedded-config.md)) — a server process
can reasonably host more than one database at once, the way a single
Postgres/MySQL instance does. That means server config needs to
represent a *list* of databases, each identified by a name clients use
to select it, distinct from where its files live.

A list of named entries doesn't fit cleanly as repeated bare CLI flags
past one or two databases, so a config file is needed rather than flags
alone. For format, there's no reason to deviate from the Rust
ecosystem's default: TOML, via `serde` + the `toml` crate — the same
format Cargo itself uses ([0002](0002-implementation-platform.md)).

Each database entry needs a path prefix ([0011](0011-embedded-config.md)).
Resolving a relative path against the process's current working
directory is fragile for a server — services (systemd, Windows Service
Manager) may launch it with an unpredictable or arbitrary CWD — so
relative paths are resolved against the server executable's own
directory instead. Absolute paths bypass this and are used as-is.

## Decision

Server config is a single **TOML file** with two top-level pieces:

- **Listen address/port** — the network endpoint the wire protocol
  ([0007](0007-wire-protocol.md)) binds to.
- **A list of databases**, each entry with:
  - `name` — the identifier clients use to select this database;
    independent of its file location.
  - `path` — a path prefix ([0011](0011-embedded-config.md)), yielding
    `<path>.data.bin` and `<path>.log.bin`. Either absolute, or relative
    to the server executable's own directory (not the process's current
    working directory).

## Consequences

- A server process can host multiple databases from one config file —
  a capability embedded mode doesn't have (one prefix per instance,
  [0011](0011-embedded-config.md)); this is where client/server
  genuinely diverges from embedded, not just wraps it.
- How the server locates its *own* config file at startup (default path
  next to the executable, a CLI flag pointing at it, etc.) isn't decided
  here — open, see [backlog.md](../backlog.md).
- Whether CLI flags or environment variables can override values from
  the file, and their precedence if so, isn't decided here — the TOML
  file is the sole source of truth for now. Open, see
  [backlog.md](../backlog.md).
- Resolving "the executable's own directory" needs the server to locate
  its own binary at startup (e.g. `std::env::current_exe()`) and decide
  how to handle it being reached via a symlink — an implementation
  detail, but one that affects what "relative to the executable" ends up
  meaning in practice.
- No new dependency category: `serde` + `toml` is already the
  path-of-least-resistance choice given Rust as the implementation
  platform ([0002](0002-implementation-platform.md)).
