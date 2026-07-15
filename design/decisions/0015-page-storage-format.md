# ADR 0015: Page Storage Format — Preamble, Page Size, Free-List

Status: Proposed (2026-07-16)

## Context

[0011](0011-embedded-config.md) decided that a database is two files, a
path prefix apart — `<prefix>.data.bin` (both storage engines' B+-trees
behind one shared page allocator) and `<prefix>.log.bin` (the WAL) — but
left the internal format of `.data.bin` unspecified. [stage1-plan.md
chunk 1](../stage1-plan.md) (page storage foundation: fixed-size page
I/O plus an allocator/free-list, no trees or MVCC yet) can't start
without that format actually being nailed down.

Two questions had to be answered before any code:

- **Where does page size come from?** A single compile-time constant is
  simplest, but a per-database page size is a real, commonly-offered
  knob (SQLite, Postgres both allow choosing page size at creation). If
  page size is stored in the data file itself so it can vary per
  database, that creates a chicken-and-egg problem: reading the stored
  page size requires reading *some* region of the file, but computing
  that region's offset/size would itself require already knowing the
  page size.
- **Where does free-list state live?** Something has to track which
  pages are free vs. allocated, and that state changes on every
  allocate/free — unlike page size, which is fixed for the life of the
  database.

## Decision

- **Page size is a per-database, creation-time parameter**, immutable
  for the life of the database. Not a global compile-time constant.
- **A fixed-size 64-byte preamble** occupies file offset 0, read with a
  plain fixed-size read *before* the page layer computes any
  page-size-dependent offset — this is what breaks the chicken-and-egg
  problem. The preamble holds, at minimum: magic bytes (format
  identification), a format/version tag, and the database's page size.
  64 bytes is deliberately generous relative to those three fields,
  leaving headroom for fields identified later (e.g. a checksum) without
  a format-breaking resize.
- **The preamble's size is fixed and independent of page size and of
  any minimum/maximum page-size policy** — it is not "one page" in any
  sense, just a constant-size region ahead of page 0.
- **Page numbering starts immediately after the preamble**: page 0
  begins at file offset 64; page *n* begins at offset `64 + n *
  page_size`.
- **Free-list state lives in page 0**, not the preamble: a free-list
  head pointer (the page number of the first free page, or a sentinel
  for "none"). Reasoning: the preamble is written once at creation and
  essentially never rewritten (page size and version don't change),
  while the free-list head changes on every allocate/free — keeping it
  out of the preamble keeps that region stable and rarely dirtied.
- **Free-list structure**: a singly-linked list threaded through the
  free pages themselves — each free page holds the page number of the
  next free page, terminating at the sentinel. No separate free-list
  data structure outside the pages it tracks.
- **Reading a page currently on the free list** (i.e. not allocated) is
  defined-but-unspecified: it returns whatever bytes are physically
  there, no guarantee about content, but it is not a reported error.
  Only reads/writes past the allocated range (beyond what the allocator
  has ever handed out) are errors.

## Consequences

- Opening a database is a two-step read: fixed 64-byte preamble read to
  learn page size and validate the file, then a page-sized read of page
  0 (now that its offset is known) to restore free-list head and
  resume allocation. No directory scan, no format sniffing beyond the
  magic bytes.
- Page size, once chosen at creation, cannot be changed in place —
  changing it means creating a new database and copying data across, a
  distinct (not-yet-designed) migration path, not a knob for later.
- Total page count is *not* decided here — whether it's stored
  explicitly in page 0 or derived from file length (`(file_size - 64) /
  page_size`) is left open, since neither choice blocks starting chunk
  1 implementation (see [backlog.md](../backlog.md)).
- Minimum/maximum allowed page size bounds are similarly not decided
  here — the preamble format doesn't require one (unlike the earlier,
  rejected idea of sizing the preamble to the minimum page size), so
  picking bounds is a validation-rule decision that can be made
  independently, later (see [backlog.md](../backlog.md)).
- This format is scoped to chunk 1 only: no crash safety (chunk 3's
  WAL), no concurrent access (chunk 4's MVCC), no tree-structure
  interpretation of page contents (chunk 2+). A page, at this layer, is
  an opaque fixed-size byte buffer plus the one bit of structure the
  free-list linking imposes on *unallocated* pages.
