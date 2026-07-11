# ADR 0006: Admin/Dev Client

Status: Decided (2026-07-11)

## Decision

Backlog items 6 (administration client) and 7 (development client) are
merged: Basalt builds **one unified client tool**, not separate admin
and dev tools. It covers both:

- **Admin operations**: connection/server management, user/role
  management, backup/restore, monitoring.
- **Dev operations**: query editor/REPL, schema browsing, result
  display, explain-plan visualization.

The unified client ships in four interface forms sharing one core:

1. **CLI REPL/executor**, sqlcmd-style — interactive prompt plus
   scriptable non-interactive execution (run a `.sql` file, pipe
   output). **Built first.**
2. **TUI** (terminal UI).
3. **GUI** (cross-platform).
4. **Web** — browser-based, **bundled into the server** itself (the
   server serves the web UI over HTTP directly, no separate process to
   run), following the CockroachDB/RethinkDB console pattern rather than
   a standalone web app like pgAdmin/Supabase Studio.

Build order: CLI first; then TUI/GUI in an order still open; web is
built **alongside or after GUI**, since it's expected to reuse the
Tauri GUI's web frontend code rather than being written from scratch.

OS support differs by interface form — see
[0003](0003-supported-os.md): CLI REPL and TUI strive to match whatever
platform list the server supports; GUI is required only on Windows +
Linux, with other platforms supported best-effort. Web has no separate
OS question — since it's bundled into the server, it's available
wherever the server runs, and the browser rendering it is platform-
independent.

### Tooling

All four interfaces are built in Rust ([0002](0002-implementation-platform.md)),
sharing one core library crate (connection handling, native-protocol
client, result-set model) underneath:

- **GUI: Tauri** (Rust backend, web frontend).
- **Web**: server-bundled HTTP endpoint expected to reuse the Tauri
  GUI's web frontend, served directly by the Basalt server rather than
  a separate process. Frontend framework/stack not yet chosen — that's
  a Tauri-GUI implementation detail to settle when GUI work starts.
- **TUI: ratatui**, with **crossterm** as the terminal backend.
- **CLI REPL**: line-editing crate **not yet decided** — open between
  **rustyline** (simpler, battle-tested; used by `datafusion-cli`, a
  close Rust-SQL-CLI precedent) and **reedline** (newer, more capable —
  syntax highlighting, multiline editing, richer completion; built for
  and used by `nushell`). Revisit when CLI work starts.
- Likely supporting crates, not yet firmly confirmed: **clap** for the
  scriptable/non-interactive execution mode's flag parsing, and a table
  crate (**comfy-table** or **tabled**) for rendering query results.

## Context

Precedent for combining admin and dev tooling into one client rather
than splitting them: Azure Data Studio, DBeaver, and pgAdmin all cover
both use cases in a single tool, rather than shipping two separate
products. Basalt follows that pattern instead of the split originally
posed in the backlog.

The CLI-first build order follows the backlog's suggested sequencing
(get a client talking to the server early, ahead of GUI/TUI work) and
the sqlcmd (MS SQL Server) precedent for what that CLI should feel
like: a REPL for interactive use, but also scriptable for automation.

This decision is entangled with [0007](0007-wire-protocol.md): the CLI
REPL is the first client to speak Basalt's own native wire protocol.

The web interface follows the "database ships its own web console"
precedent (CockroachDB, RethinkDB) rather than the "separate web app
talking to the API" precedent (pgAdmin, Supabase Studio) — chosen
because bundling means zero extra deployment/process for a solo-project
server, and because the frontend code is expected to already exist from
the Tauri GUI.

## Consequences

- Early client-side development effort concentrates on the CLI REPL and
  the native protocol it speaks. The GUI toolkit choice is already
  settled (Tauri, see Tooling above), but actual GUI work is still
  deferred until CLI/TUI work is further along.
- Because all four interface forms share one core, the admin/dev
  operation set (connection mgmt, users/roles, backup/restore,
  monitoring, query/REPL, schema browsing) should be designed as a
  protocol/API surface the core exposes, not duplicated per interface.
- The server now needs to speak HTTP (to serve the bundled web UI) in
  addition to the native protocol ([0007](0007-wire-protocol.md)) — an
  extra listener/port, and its own authentication story, separate from
  the native-protocol auth path.
- Web build timing follows GUI rather than being independently
  scheduled, so it inherits GUI's position in the still-open TUI/GUI
  ordering.
