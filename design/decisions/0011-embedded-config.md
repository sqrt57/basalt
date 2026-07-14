# ADR 0011: Embedded Configuration — DB File Layout

Status: Proposed (2026-07-14)

## Context

[0001](0001-scope.md) makes the in-process/embedded library the first
stage, so the embedded API needs a minimal config surface before any
other embedded work can start: given nothing yet, how does a caller tell
the engine where a database's files live?

Three physical file layout models were considered, using other engines
as reference points:

- **Single opaque file** (SQLite-style): one file holds every table's
  B+-tree behind a shared page allocator; a `-wal` sidecar file is
  derived implicitly from the same filename.
- **Directory of files** (Postgres-style): a data directory whose
  contents are managed by convention — one file per relation, a `pg_wal/`
  subdirectory — inferred by scanning the directory rather than named
  explicitly.
- **Explicit multi-file** (SQL Server-style): the caller names and
  places each file individually — primary/secondary data files, one or
  more log files — with no fixed directory or naming convention tying
  them together.

Basalt's two storage engines ([0004](0004-relational-storage-engine.md)
relational, [0010](0010-hierarchical-storage-engine.md) hierarchical)
already share a single WAL ([0008](0008-concurrency-durability.md)), so
a fully explicit per-engine file layout (SQL Server-style) would be
inventing independence at the file level that doesn't exist anywhere
else in the design. A pure single-file model (SQLite-style) fits the
"closer to SQLite/DuckDB" framing from [0001](0001-scope.md) but its
implicit `-wal` sidecar naming isn't self-explanatory. A directory
convention (Postgres-style) is more machinery than a two-file layout
needs.

## Decision

The minimal embedded config is a single **path prefix** (e.g. a
`Path`/`PathBuf` passed into the engine's builder/options) identifying a
database. The engine derives exactly two files from it by fixed suffix
— no directory scanning, no per-engine file, no other config required to
locate a database:

- `<prefix>.data.bin` — the single data file, holding both storage
  engines' B+-trees behind one shared page allocator.
- `<prefix>.log.bin` — the single WAL file
  ([0008](0008-concurrency-durability.md)).

A database named `abc` at path prefix `abc` is therefore exactly the
pair `abc.data.bin` + `abc.log.bin`.

## Consequences

- Both storage engines' physical pages live in one file, a tighter
  physical coupling than "implementable independently of each other"
  ([0004](0004-relational-storage-engine.md),
  [0010](0010-hierarchical-storage-engine.md)) suggested — but no new
  failure-isolation loss beyond what the already-shared WAL
  ([0008](0008-concurrency-durability.md)) implied. The relational and
  hierarchical engines' page allocators will need to coordinate
  free-space/allocation bookkeeping within that one file; an
  implementation detail for whichever engine lands first.
- Two files, one fixed naming convention: a user or tool can find both
  files given only the prefix, without scanning a directory — more
  explicit than SQLite's implicit sidecar, without SQL Server's
  per-file placement flexibility (both files must share one prefix
  path; they can't be put on separate drives independently).
- Deliberately minimal: doesn't address other embedded config (isolation
  level, memory/cache limits, sync/fsync policy, read-only mode) or how
  server mode ([0001](0001-scope.md) second stage) resolves a prefix
  from a config file/flags/env vars — left open, see
  [backlog.md](../backlog.md).
