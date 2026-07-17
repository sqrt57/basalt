# ADR 0015: Page Storage Format — Preamble, Page Size, Free-List

Status: Proposed (2026-07-16), revised (2026-07-16), revised (2026-07-17)

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

**Revision (2026-07-17)**: the original decision below reserved *two*
special pages — page 0 (preamble) and page 1 (free-list head) — before
allocatable pages started at 2, with page id 0 just an ordinary
(if reserved) page number, no different in kind from page 1. Revised
below on two fronts. First, the free-list head merges into the
preamble: both fit the existing 64-byte fixed region with room to
spare, so there's no reason to keep them on separate pages — collapsing
two special-cased pages into one. Second, numbering shifts to be
**1-based rather than 0-based**: page id `0` is retired from ever
addressing a physical page at all — it has no file offset, and the
pager never seeks to it — becoming a pure null value with no other
meaning to conflict with. The merged preamble+free-list page becomes
**page 1** (the first physical page, at file offset 0), and allocatable
pages now start at 2. This follows SQLite's convention (page 1 is the
header/schema page; 0 is reserved purely as an invalid/null page
reference in on-disk pointers) rather than overloading one page id as
both "the special physical page" *and* "the null sentinel" at once —
conflating those meant page id 0 would address real bytes in one
context (open the file, read page 0) while meaning "no page" in another
(a free-list terminator). Splitting them removes the overlap: `0` now
only ever means "no page," physical or special, and every page-id-typed
field elsewhere in the format (the free-list "no next page" terminator,
[0017](0017-wal-format.md)'s `PageDiff.base_page_id` "no base" case) can
use it directly instead of a separate out-of-band sentinel value
(`u64::MAX`).

## Decision

- **Page size is a per-database, creation-time parameter**, immutable
  for the life of the database. Not a global compile-time constant.
- **A 64-byte preamble** holds, at minimum: magic bytes (format
  identification), a format/version tag, the database's page size, and
  the free-list head pointer (page id of the first free page, or `0`
  for "none" — see below). It's read with a plain fixed-size (64-byte)
  read *before* the page layer computes any page-size-dependent offset
  — this is what breaks the chicken-and-egg problem, and now also
  means opening a database learns page size *and* free-list state from
  that same single read, no second page-sized read needed. All
  multi-byte fields little-endian.

  | Offset | Length | Description |
  |---|---|---|
  | 0 | 8 | magic bytes `b"BSLTPAGE"` |
  | 8 | 4 | `format_version: u32` |
  | 12 | 4 | `page_size: u32` |
  | 16 | 8 | `free_list_head: u64` — page id of the first free page, or `0` for none |
  | 24 | 40 | reserved (zeroed) |

  64 bytes is deliberately generous relative to those four fields,
  leaving headroom (the reserved 40 bytes above) for fields identified
  later (e.g. a checksum) without a format-breaking resize.
- **Page numbering is 1-based: page *n* begins at file offset
  `(n - 1) * page_size`, for `n >= 1`.** Page id `0` has no offset at
  all — the pager never computes one for it and never seeks to it; it
  is not a physical page, just a value. (Revised 2026-07-17 from the
  original 0-based `n * page_size` — see the Context revision note
  above.)
- **The preamble occupies the whole of page 1**, zero-padded from byte
  64 out to `page_size` — not a bare 64-byte region followed immediately
  by page-sized blocks. Page 1 sits at file offset 0 (the first physical
  page in the file), keeping every page-size-aligned to the formula
  above with no separate preamble offset term anywhere in the pager.
  (Revised 2026-07-17 — previously this was page 0, under the 0-based
  scheme; see the Context revision note above.)
- **Free-list state lives in page 1's preamble**, not a separate page.
  (Revised 2026-07-17 — originally a distinct page held it, reasoned as
  keeping the rarely-dirtied preamble region separate from the every-
  allocate/free-dirtied free-list head; that reasoning is dropped in
  favor of a single reserved page — see the Context revision note. The
  preamble is still only 64 content bytes out of a full page, so adding
  one more 8-byte field to it costs nothing relative to the two-page
  layout's fixed cost.) **Page 1 is the only reserved physical page**:
  `allocate_page` never hands it out, since the allocator's page-count
  baseline starts at 2 — page numbering for allocatable pages begins at
  2.
- **Page id `0` is a standing null value — not a physical page at
  all**, reserved purely as "no page." It's not merely unallocatable the
  way page 1 is (page 1 holds real bytes read/written like any other
  page); it corresponds to no file offset and the pager rejects any
  attempt to read/write/seek to it. Any page-id-typed field elsewhere in
  the format can use `0` for "no page" instead of a dedicated
  out-of-band sentinel: the free-list head/next-pointer's "no next page"
  terminator is `0` (not `u64::MAX`), and
  [0017](0017-wal-format.md)'s `PageDiff.base_page_id` "no base" case
  reuses the same convention.
- **Free-list structure**: a singly-linked list threaded through the
  free pages themselves — each free page holds the page number of the
  next free page, terminating at `0`. No separate free-list data
  structure outside the pages it tracks. Per free page:

  | Offset | Length | Description |
  |---|---|---|
  | 0 | 8 | `next: u64` — page id of the next free page, or `0` if this is the last |
- **Reading a page currently on the free list** (i.e. not allocated) is
  defined-but-unspecified: it returns whatever bytes are physically
  there, no guarantee about content, but it is not a reported error.
  Only reads/writes past the allocated range (beyond what the allocator
  has ever handed out) are errors.

## Consequences

- Opening a database is a single fixed 64-byte read (the first 64 bytes
  of the file, i.e. of page 1 — page 1 always starts at offset 0): it
  validates the file (magic/version), learns page size, and restores
  the free-list head to resume allocation, all from the same read.
  (Revised 2026-07-17 — previously a two-step read, the fixed preamble
  read followed by a page-sized read of a separate allocator page;
  merging the free-list head into the preamble drops that second read.)
  No directory scan, no format sniffing beyond the magic bytes. That
  bootstrap read is the *only* special-cased I/O page 1 gets: once
  page size is known (immediately after, every time), page 1's own
  offset (`0`) is computable the same way any other page's is, so
  ordinary `read_page`/`write_page` calls against it work like any
  other page — updating the free-list head on every allocate/free is a
  normal page write, not a distinct code path.
- Page size, once chosen at creation, cannot be changed in place —
  changing it means creating a new database and copying data across, a
  distinct (not-yet-designed) migration path, not a knob for later.
- Total page count is *not* decided here beyond "derived from file
  length" being the simpler option now that it's a clean division —
  `file_size / page_size`, no subtraction term — rather than stored
  explicitly in page 1; whether to switch to explicit storage anyway is
  left open, since neither choice blocks chunk 1 (see
  [backlog.md](../backlog.md)). Under 1-based numbering this count is
  the highest valid page id (pages `1..=page_count`), not a one-past-end
  index as it would be under 0-based numbering — `check_range` must
  reject both `0` and anything `> page_count`, not just `>= page_count`.
- Minimum/maximum allowed page size bounds are similarly not decided
  here — the preamble format doesn't require one (unlike the earlier,
  rejected idea of sizing the preamble to the minimum page size), so
  picking bounds is a validation-rule decision that can be made
  independently, later (see [backlog.md](../backlog.md)).
- Reserving one physical page (page 1) before allocatable data starts at
  page 2 — down from two pages in the original layout — is a one-page
  fixed cost per database, paid once, negligible next to any real page
  size either way. Page id `0` costs nothing extra since it was never a
  physical page to begin with. (Revised 2026-07-17 — see the Context
  revision note.)
- The preamble's 64 content bytes must now fit *within* page 1 rather
  than occupying their own independent region, so the minimum page size
  floor rises from 8 (driven only by the free-list next-pointer's 8
  bytes) to 64 — otherwise the "deliberately generous headroom" framing
  above is false for small page sizes. This isn't a new minimum/maximum
  page-size *policy* decision (still open, see
  [backlog.md](../backlog.md)), just the floor this revision's own
  layout requires to stay internally consistent.
- This format is scoped to chunk 1 only: no crash safety (chunk 3's
  WAL), no concurrent access (chunk 4's MVCC), no tree-structure
  interpretation of page contents (chunk 2+). A page, at this layer, is
  an opaque fixed-size byte buffer plus the one bit of structure the
  free-list linking imposes on *unallocated* pages.
