# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Basalt (code name) is a database engine in the pre-implementation, design
stage. There is no source code, build system, or test suite yet — the
repository currently contains only licensing and design material:

- `LICENSE` — MIT.
- `README.md` — one-paragraph pointer into the design backlog.
- `design/backlog.md` — the open architectural decisions that must be
  settled before implementation starts (scope, implementation platform,
  supported OSes, storage/execution architecture, SQL support level,
  admin/dev tooling, and wire-protocol compatibility such as
  Postgres/MySQL/ODBC/JDBC).
- `design/decisions/` — ADRs recording decisions as backlog items get
  resolved (currently: scope, implementation platform, supported OSes,
  implementation architecture, SQL support, admin/dev client, wire
  protocol).

Store AI-generated design docs under `design/`.

Decided so far: Basalt is a single-node, client/server relational
database engine, built as a solo learning/research project
([0001](design/decisions/0001-scope.md)), implemented in Rust
([0002](design/decisions/0002-implementation-platform.md)), with OS
support staged by priority — Windows day one, Linux next, FreeBSD much
later for ZFS, other BSDs/illumos/macOS "maybe sometime"
([0003](design/decisions/0003-supported-os.md)). Storage/execution
architecture ([0004](design/decisions/0004-implementation-architecture.md))
and SQL support level ([0005](design/decisions/0005-sql-support.md)) are
also decided. Admin and dev tooling are merged into one client shipped
as CLI REPL (first), TUI, GUI (Tauri), and a server-bundled Web console
([0006](design/decisions/0006-admin-dev-client.md)), speaking Basalt's
own native wire protocol first, with Postgres wire-protocol
compatibility layered on later
([0007](design/decisions/0007-wire-protocol.md)). No code exists yet, so
there are still no build/lint/test commands to document.
When implementation begins, this file should be updated with the actual
Cargo commands and the real crate/module architecture once code exists —
do not invent these before they exist.

## Working on this repo right now

Most work at this stage is design discussion, not code. Before writing
any code:

1. Check `design/backlog.md` for whether the relevant decision (scope,
   platform, architecture, SQL dialect, wire protocol, etc.) has actually
   been settled — many items are still open questions with options
   listed, not decisions.
2. Record new design decisions and proposals as new files under
   `design/`, and update `design/backlog.md` as items get resolved.
