# Build Order

Derived from the ADRs in `decisions/`, roughly in sequence. This is a
plan, not a decision — it can reorder as work actually starts or
priorities shift, independent of any ADR changing. See
[architecture.md](architecture.md) for the proposed state this is
sequencing, and [backlog.md](backlog.md) for items intentionally left
off this roadmap (deferred, not scheduled).

1. Storage engines ([0004](decisions/0004-storage-engines.md)),
   concurrency/durability
   ([0008](decisions/0008-concurrency-durability.md)), and query
   execution ([0009](decisions/0009-query-execution.md)) against the
   minimal SQL subset ([0005](decisions/0005-sql-support.md)).
2. Native wire protocol ([0007](decisions/0007-wire-protocol.md)) plus
   the CLI REPL ([0006](decisions/0006-admin-dev-client.md)) — the first
   client.
3. TUI and GUI, then the Web console alongside/after GUI.
