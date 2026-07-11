# ADR 0007: Wire Protocol — Native Is the Target, Postgres Compatibility Optional

Status: Decided (2026-07-11)

## Decision

Basalt implements its **own native wire protocol** as the primary
protocol — this is what the CLI REPL ([0006](0006-admin-dev-client.md))
speaks, and it is the committed target for client connectivity, not just
a first phase. **Postgres wire-protocol compatibility** (backlog item 7,
formerly item 8 before items 6/7 were merged) is an **optional,
uncommitted possibility** — it may be built later as a separate interop
layer for third-party tooling (`psql`, pgAdmin, DBeaver, ORMs/drivers
such as npgsql/psycopg/JDBC) if there's a leverage case for it at the
time, but Basalt's design does not assume it will happen.

**Native client libraries**: initial language coverage is **Rust, .NET,
and C**, with **Rust as top priority**. Other languages are later,
unscheduled work.

## Context

The original backlog framed Postgres wire-protocol compatibility as the
single highest-leverage piece of interop work, since it unlocks an
entire existing client ecosystem for free. That leverage argument still
holds in principle, but a closer look at what "compatible" actually
requires changed the calculus: the wire format itself (framing, auth,
simple/extended query flow) is the easy part, well-documented and proven
tractable by many from-scratch implementations. The hard, open-ended
part is faking enough of Postgres's semantics and catalog
(`pg_catalog`/`information_schema`, type OIDs, built-in functions) that
real client tools and ORMs work without special-casing — an ongoing tax
that would otherwise shape Basalt's own type system and catalog design
around emulating another engine before Basalt's own semantics are
settled.

Given that cost, Postgres compatibility is downgraded from a committed
future phase to an optional one: worth revisiting later if the leverage
case still holds, but not a target Basalt's core design should carry a
standing obligation toward. The native protocol is the actual, permanent
target for client connectivity.

With Postgres compatibility no longer a guaranteed path to app-level
driver access, native client libraries need their own priority order.
Rust comes first since it's Basalt's own implementation language
([0002](0002-implementation-platform.md)) — a Rust client can share
code with the server (wire types, encoding) and serves Basalt's own
CLI/TUI/GUI/Web client crates ([0006](0006-admin-dev-client.md)), which
themselves need a client library to talk to a Basalt server. C comes
next as a low-level FFI-friendly target that other languages' bindings
can wrap, the way `libpq` anchors much of the Postgres client ecosystem.
.NET is included in the initial set alongside Rust and C on the strength
of the enterprise/Windows-adjacent audience implied by Windows being the
first supported server OS ([0003](0003-supported-os.md)). Other
languages (Python, Node, Go, Java, etc.) are left for later,
unscheduled work.

## Consequences

- The native protocol's own design (text vs. binary framing, request/
  response shape, auth, streaming results, etc.) is a follow-on open
  question, not resolved by this ADR.
- Third-party Postgres-speaking tools (psql, pgAdmin, DBeaver, ORMs)
  have no committed path to connect to Basalt; the native protocol, via
  the CLI REPL and native client libraries, is the intended way
  applications talk to Basalt.
- Rust, C, and .NET client libraries are the committed initial set, Rust
  first — Python, Node, Go, Java, and others are unscheduled. A
  standardized ODBC/JDBC/ADO.NET *provider* (as opposed to a plain
  native .NET/C client library) is a separate, still-open question — see
  [backlog.md](../backlog.md).
- ODBC/JDBC/ADO.NET driver access is **not** guaranteed to arrive "for
  free" via a future Postgres-compatibility layer, since that layer is
  now optional rather than committed. If app-level driver access is
  needed, it remains an open question independent of this ADR.
- MySQL wire protocol and replication/CDC hooks (also mentioned as
  possibilities in the original backlog item) remain unscheduled and are
  not affected by this decision either way. HTTP is a partial exception:
  the server will speak HTTP to serve the bundled web console
  ([0006](0006-admin-dev-client.md)), but that's a narrow, UI-serving
  endpoint — a general REST/JSON data API is a separate,
  still-unscheduled question.
