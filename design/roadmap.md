# Build Order

Derived from the ADRs in `decisions/`, roughly in sequence. This is a
plan, not a decision — it can reorder as work actually starts or
priorities shift, independent of any ADR changing. See
[architecture.md](architecture.md) for the proposed state this is
sequencing, and [backlog.md](backlog.md) for items intentionally left
off this roadmap (deferred, not scheduled).

1. Minimal embedded core — simplest form of everything needed for a
   first working engine:
   - Relational storage only ([0004](decisions/0004-relational-storage-engine.md));
     the hierarchical engine ([0010](decisions/0010-hierarchical-storage-engine.md))
     is deferred, unstaged (see below).
   - Simplest concurrency: a single global lock, one writer at a time,
     blocking everyone else — no MVCC yet
     ([0008](decisions/0008-concurrency-durability.md)).
   - Simplest durability: crash-safe writes without a WAL (atomic page
     writes + fsync on commit, no redo log)
     ([0008](decisions/0008-concurrency-durability.md)).
   - Simplest SQL subset, no isolation-level surface yet — the global
     lock makes every transaction trivially serial
     ([0005](decisions/0005-sql-support.md)).
   - Shipped in-process/embedded, no server process required
     ([0001](decisions/0001-scope.md)), as an independently linkable
     Rust library ([0002](decisions/0002-implementation-platform.md)).
2. CUI (CLI) utility — in-process only, linked directly against the
   embedded core from stage 1. No wire protocol yet
   ([0006](decisions/0006-admin-dev-client.md)).
3. Client/server — native wire protocol
   ([0007](decisions/0007-wire-protocol.md)) plus a server process
   wrapping the same engine core; the CLI from stage 2 gains a network
   mode rather than becoming a separate tool
   ([0001](decisions/0001-scope.md), [0006](decisions/0006-admin-dev-client.md)).
4. Other concurrency mechanisms — MVCC replaces stage 1's global lock,
   WAL-grade durability replaces stage 1's sync-write durability, and
   the SQL isolation-level buildup begins from Snapshot
   ([0008](decisions/0008-concurrency-durability.md),
   [0005](decisions/0005-sql-support.md)).

Unstaged — after stage 4, relative order not yet decided:

- Hierarchical storage engine
  ([0010](decisions/0010-hierarchical-storage-engine.md)).
- TUI, then GUI, then the Web console
  ([0006](decisions/0006-admin-dev-client.md)).
