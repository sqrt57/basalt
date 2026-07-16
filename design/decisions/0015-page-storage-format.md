# ADR 0015: Page Storage Format — Preamble, Page Size, Free-List

Status: Proposed (2026-07-16), revised (2026-07-16)

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

**Revision (2026-07-16)**: the original decision below put the 64-byte
preamble at file offset 0 and started page numbering immediately after
it (page *n* at offset `64 + n * page_size`) — deliberately *not*
page-aligned, on the reasoning that the preamble is a fixed-size region
independent of page size, not "one page" in any sense. That breaks a
constraint every other part of this design assumes without stating it:
every page-sized I/O in [chunks 2-4](../stage1-plan.md) reads/writes
exactly one `page_size`-sized region, and the pager itself only ever
extends the file by whole pages (`allocate_page`, `extend_to`) — the
whole model implicitly assumes the file is a sequence of page-size-
aligned blocks from offset 0. A 64-byte preamble followed immediately by
page-size blocks means every page after it starts at `64 + n *
page_size`, which is page-size-aligned only when `page_size` happens to
divide 64 evenly — false for the common case (e.g. any page size that
isn't itself a divisor of 64, and even page sizes that are require the
coincidence lining up). Revised below: the preamble occupies a full,
page-size-aligned page by itself (page 0), so every subsequent page
starts at a clean `n * page_size` with no offset term at all.

## Decision

- **Page size is a per-database, creation-time parameter**, immutable
  for the life of the database. Not a global compile-time constant.
- **A 64-byte preamble** holds, at minimum: magic bytes (format
  identification), a format/version tag, and the database's page size.
  It's read with a plain fixed-size (64-byte) read *before* the page
  layer computes any page-size-dependent offset — this is what breaks
  the chicken-and-egg problem. 64 bytes is deliberately generous
  relative to those three fields, leaving headroom for fields
  identified later (e.g. a checksum) without a format-breaking resize.
- **The preamble occupies the whole of page 0**, zero-padded from byte
  64 out to `page_size` — not a bare 64-byte region followed immediately
  by page-sized blocks. This keeps every page in the file, including
  page 0, aligned to a `page_size` multiple from file offset 0: page *n*
  begins at offset `n * page_size`, full stop, no separate preamble
  offset term anywhere in the pager. (Revised from the original version
  of this decision — see the Context revision note above.)
- **Free-list state lives in page 1**, not the preamble: a free-list
  head pointer (the page number of the first free page, or a sentinel
  for "none"). Reasoning: the preamble is written once at creation and
  essentially never rewritten (page size and version don't change),
  while the free-list head changes on every allocate/free — keeping it
  out of the preamble keeps that region stable and rarely dirtied.
  Pages 0 and 1 are both reserved: `allocate_page` never hands either
  out, since the allocator's page-count baseline starts at 2.
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

- Opening a database is a two-step read: fixed 64-byte preamble read
  (the first 64 bytes of page 0) to learn page size and validate the
  file, then a page-sized read of page 1 (now that its offset is known,
  a clean `1 * page_size`) to restore the free-list head and resume
  allocation. No directory scan, no format sniffing beyond the magic
  bytes.
- Page size, once chosen at creation, cannot be changed in place —
  changing it means creating a new database and copying data across, a
  distinct (not-yet-designed) migration path, not a knob for later.
- Total page count is *not* decided here beyond "derived from file
  length" being the simpler option now that it's a clean division —
  `file_size / page_size`, no subtraction term — rather than stored
  explicitly in page 1; whether to switch to explicit storage anyway is
  left open, since neither choice blocks chunk 1 (see
  [backlog.md](../backlog.md)).
- Minimum/maximum allowed page size bounds are similarly not decided
  here — the preamble format doesn't require one (unlike the earlier,
  rejected idea of sizing the preamble to the minimum page size), so
  picking bounds is a validation-rule decision that can be made
  independently, later (see [backlog.md](../backlog.md)).
- Reserving two pages (0 and 1) instead of one before allocatable data
  starts is a two-page fixed cost per database, paid once — negligible
  next to any real page size.
- This format is scoped to chunk 1 only: no crash safety (chunk 3's
  WAL), no concurrent access (chunk 4's MVCC), no tree-structure
  interpretation of page contents (chunk 2+). A page, at this layer, is
  an opaque fixed-size byte buffer plus the one bit of structure the
  free-list linking imposes on *unallocated* pages.
