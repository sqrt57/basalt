# ADR 0007: Wire Protocol — Native First, Postgres Compatibility Later

Status: Decided (2026-07-11)

## Decision

Basalt implements its **own native wire protocol** as the primary,
first-built protocol — this is what the CLI REPL
([0006](0006-admin-dev-client.md)) speaks. **Postgres wire-protocol
compatibility** (backlog item 7, formerly item 8 before items 6/7 were
merged) is **deferred to a later phase**,
added afterward as a separate interop layer for third-party tooling
(`psql`, pgAdmin, DBeaver, ORMs/drivers such as npgsql/psycopg/JDBC) —
it is not the initial path.

## Context

The original backlog framed Postgres wire-protocol compatibility as the
single highest-leverage piece of interop work, since it unlocks an
entire existing client ecosystem for free. That leverage argument still
holds, but this decision reverses the *sequencing*: Basalt's own core
semantics and its CLI client come first, on a protocol designed for
Basalt specifically, rather than coupling early development to emulating
another system's wire format before Basalt's own semantics are settled.

Precedent for the destination (Postgres-wire-compatible, own
storage/execution engine underneath) is unchanged from the original
backlog framing — CockroachDB, YugabyteDB, and Amazon Redshift all prove
the pattern out. This decision only changes what gets built first.

## Consequences

- The native protocol's own design (text vs. binary framing, request/
  response shape, auth, streaming results, etc.) is a follow-on open
  question, not resolved by this ADR.
- Third-party Postgres-speaking tools (psql, pgAdmin, DBeaver, ORMs)
  cannot connect to Basalt until the Postgres-compatibility phase is
  reached; until then, the CLI REPL is the only client.
- MySQL wire protocol, ODBC/JDBC, and replication/CDC hooks (all
  mentioned as possibilities in the original backlog item) remain
  unscheduled and are not affected by this decision either way. HTTP is
  a partial exception: the server will speak HTTP to serve the bundled
  web console ([0006](0006-admin-dev-client.md)), but that's a narrow,
  UI-serving endpoint — a general REST/JSON data API is a separate,
  still-unscheduled question.
