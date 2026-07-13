# Build Order

Derived from the ADRs in `decisions/`, roughly in sequence. This is a
plan, not a decision — it can reorder as work actually starts or
priorities shift, independent of any ADR changing. See
[architecture.md](architecture.md) for the proposed state this is
sequencing, and [backlog.md](backlog.md) for items intentionally left
off this roadmap (deferred, not scheduled).

1. Storage engines — relational
   ([0004](decisions/0004-relational-storage-engine.md)) and
   hierarchical ([0010](decisions/0010-hierarchical-storage-engine.md)),
   implementable independently of each other — plus
   concurrency/durability ([0008](decisions/0008-concurrency-durability.md))
   and query execution ([0009](decisions/0009-query-execution.md))
   against the minimal SQL subset ([0005](decisions/0005-sql-support.md)).
   Built from the start as an independently linkable crate, separate
   from any network/wire-protocol layer ([0001](decisions/0001-scope.md)).
2. In-process/embedded build mode — expose that engine core directly as
   a library, no server process required. First usable form of Basalt
   ([0001](decisions/0001-scope.md)).
3. Native wire protocol ([0007](decisions/0007-wire-protocol.md)) plus
   the CLI REPL ([0006](decisions/0006-admin-dev-client.md)) — a server
   process wraps the same engine core and becomes the first
   client/server-facing form.
4. TUI and GUI, then the Web console alongside/after GUI.
