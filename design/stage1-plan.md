# Stage 1 Implementation Plan

Stage 1 is [roadmap.md](roadmap.md)'s "minimal embedded core" — a working
embedded engine end-to-end: relational storage only, single-writer MVCC,
WAL-backed durability, a minimal SQL subset, tree-walking execution,
shipped as a linkable Rust library. This is a plan, not a decision — it
can reorder as implementation reveals gaps, independent of any ADR
changing. See [architecture.md](architecture.md) for the proposed
design each chunk below implements, and the referenced ADRs for the
reasoning behind it.

## Chunks

Roughly bottom-up; each chunk depends on the ones before it.

1. **Page storage foundation** — fixed-size page I/O against
   `<prefix>.data.bin`, a page allocator/free-list
   ([0011](decisions/0011-embedded-config.md),
   [0015](decisions/0015-page-storage-format.md)). No trees, no MVCC
   yet.

   Acceptance criteria:
   - Creating a new database at a path prefix produces
     `<prefix>.data.bin` with a valid 64-byte preamble (magic, version,
     page size) and an empty page-0 allocator state.
   - Opening an existing file reads the preamble first (fixed-size read,
     independent of page size), validates magic/version, then locates
     page 0 to restore the free-list head. Corrupt/unrecognized magic is
     a reported error.
   - Page size is a creation-time parameter, immutable thereafter;
     opening a file uses whatever page size is stored, never a
     caller-supplied override.
   - Close/reopen round-trips: allocate/write/free done before a clean
     close are visible in the same shape after reopen.
   - `allocate_page()` reuses a free-listed page number if one exists,
     otherwise extends the file by one page; `free_page(n)` pushes `n`
     onto the free-list. Allocate N → free all N → allocate N again does
     not grow the file further.
   - `read_page(n)` / `write_page(n, buf)` operate on exactly one
     page-sized buffer each; no partial-page I/O in the API. Data
     written is returned byte-for-byte by a later read, same session and
     after a clean reopen.
   - Reading/writing beyond the allocated range is a reported error.
     Reading a page currently on the free-list is defined-but-
     unspecified content, not an error.
   - Explicit non-goals for this chunk: no crash safety across a crash
     (chunk 3), no concurrent access (chunk 4), no tree-structure
     interpretation of page contents (chunk 2+).
2. **Copy-on-write B+-tree** — insert/lookup/delete/range-scan over
   pages, in-memory only (no WAL, no root persisted across restarts
   yet). Get the COW mechanics correct in isolation
   ([0008](decisions/0008-concurrency-durability.md)).
3. **WAL + redo recovery** — `<prefix>.log.bin`, binary-diff records,
   fuzzy checkpoint with dirty-page table, wiring writes from chunk 2
   through the log ([0008](decisions/0008-concurrency-durability.md),
   [0011](decisions/0011-embedded-config.md)).
4. **MVCC snapshots + reclamation** — root history, in-memory reader
   table, transaction begin/commit/abort, snapshot-isolated readers,
   low-water-mark reclamation riding the checkpoint pass
   ([0008](decisions/0008-concurrency-durability.md),
   [0013](decisions/0013-page-reclamation.md)).
5. **Row store** — tuple encoding, table/schema catalog, on top of the
   tree from chunks 2-4; exposes the scan/seek iterator interface
   ([0004](decisions/0004-relational-storage-engine.md)).
6. **SQL front end** — parser for the stage-1 subset, producing a
   minimal AST/logical plan
   ([0005](decisions/0005-sql-support.md)).
7. **Query execution** — Volcano-style operators (scan/filter/project/
   join/insert/update/delete) driving the row store's iterators
   ([0009](decisions/0009-query-execution.md)).
8. **Embedded API** — the public library surface: builder/options
   taking a path prefix, open/close, transaction handles
   ([0011](decisions/0011-embedded-config.md)).
9. **Integration** — end-to-end tests: DDL/DML round-trips,
   crash-recovery (kill mid-write, confirm redo brings it back
   consistent).

Chunks 1-4 are the hard, novel part (COW MVCC + WAL is most of the
design work); chunks 5-8 are comparatively mechanical once that
substrate is solid.

## Resolved Since This Plan Was Written

- **Crate layout** — decided in
  [0014](decisions/0014-crate-layout.md): stage 1 lives entirely in the
  `basalt-engine` crate.
- **Starting point** — chunks 1-2 (page I/O + in-memory COW B+-tree, no
  WAL yet) first, in the order the chunk list above already gives, rather
  than front-loading WAL+recovery. The binary-diff log format
  ([0008](decisions/0008-concurrency-durability.md)) is computed from
  old/new *page content* after the tree produces it, so the WAL layer
  needs stable page bytes to diff, not tree internals — building WAL
  first gains little design insight and adds the cost of debugging
  crash-recovery and COW-tree correctness at the same time, instead of
  getting the tree solid via direct, in-memory tests before durability
  sits underneath it.
