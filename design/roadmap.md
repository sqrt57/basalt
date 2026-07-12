# Build Order

Derived from the ADRs in `decisions/`, roughly in sequence. This is a
plan, not a decision — it can reorder as work actually starts or
priorities shift, independent of any ADR changing. See
[architecture.md](architecture.md) for the proposed state this is
sequencing, and [backlog.md](backlog.md) for items intentionally left
off this roadmap (deferred, not scheduled).

1. Storage/execution core ([0004](decisions/0004-implementation-architecture.md))
   against the minimal SQL subset ([0005](decisions/0005-sql-support.md)).
2. Native wire protocol ([0007](decisions/0007-wire-protocol.md)) plus
   the CLI REPL ([0006](decisions/0006-admin-dev-client.md)) — the first
   client.
3. TUI and GUI, then the Web console alongside/after GUI.
