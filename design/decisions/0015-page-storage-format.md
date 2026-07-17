# ADR 0015: Page Storage Format — Preamble, Page Size, Free-List

Status: Proposed (2026-07-16), revised (2026-07-16), revised (2026-07-17), revised (2026-07-17), revised (2026-07-17)

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

**Revision (2026-07-17, cont'd)**: page 1 was still structurally
different from every other page in the file at the pager layer — it had
no type tag, while [chunk 2](0016-btree-node-format.md)'s leaf/internal
nodes each start with a `node_type: u8` identifying what kind of page
they are. That's an asymmetry with no real reason behind it: page 1 is
just as much "a page" as any allocatable one, it just happens to hold
file-header content instead of tree-node content. Revised below: every
page in the file — including page 1 — starts with the same 2-byte
common header (a type tag plus a reserved byte), and page 1's type tag
identifies it as the file header, the one reserved instance of that
type. This costs nothing new: chunk 2's node header already reserved
exactly these first 2 bytes for `node_type`/reserved, so this ADR is
just naming that shape once, at the pager layer, instead of leaving
chunk 2 to invent it independently and page 1 to go without.

**Revision (2026-07-17, free-list trunk pages)**: the free-list was a
singly-linked list threaded through every free page — each one held an
8-byte "next" pointer as its entire content. That means *every*
`allocate_page`/`free_page` call touches exactly one free page to
update one pointer, and — looking ahead to [chunk 3](0017-wal-format.md),
which logs every `write_page` — freeing or allocating a long run of
pages one at a time would WAL-log one `PageDiff` per page, purely to
maintain the chain, before any real work happens. It also gave every
free page *some* defined structure (the next-pointer), when the whole
point of a free page is that its content is garbage until reused.
Revised below to SQLite's freelist-trunk-page design: a small number of
**trunk pages**, each batching a whole array of free page ids, chained
to each other; the free pages a trunk lists are pure, contentless
leaves. Freeing/allocating N pages now touches O(N / leaves-per-trunk)
pages instead of O(N), and a free page's content goes back to being
completely unconstrained — no next-pointer required.

**Revision (2026-07-17, page LSN)**: [chunk 3](0017-wal-format.md)'s WAL
gives every write a durable, totally-ordered LSN, but nothing on disk
records which LSN produced a given page's *current* content — recovery
can only replay the whole log blindly, with no way to tell (without
external bookkeeping) whether a given page already reflects a record it
is about to redo. The standard fix, used by every ARIES-descended
write-ahead-logging design (Postgres's `pd_lsn`, among others): stamp
each page with the LSN of the write that last produced it, so redo (or
any future tool) can compare a page's on-disk LSN against a record's
LSN. Since this applies to every page type uniformly — not just tree
nodes — it belongs in the common page header (this ADR), decided once,
rather than each page type inventing its own version later or the
field being bolted on after the format has already shipped (which would
shift every offset in [0016](0016-btree-node-format.md) a second time).
Chunk 1 itself has no notion of an LSN — it just reserves the bytes and
treats them as opaque content it neither reads nor writes meaning into,
the same way it already treats the preamble's other reserved bytes;
[chunk 3](0017-wal-format.md) is what actually stamps a real value in
there, as part of its write path.

## Decision

- **Page size is a per-database, creation-time parameter**, immutable
  for the life of the database. Not a global compile-time constant.
- **Common page header**: every page in the file — including page 1 —
  begins with the same 10 bytes:

  | Offset | Length | Description |
  |---|---|---|
  | 0 | 1 | `page_type: u8` |
  | 1 | 1 | reserved |
  | 2 | 8 | `page_lsn: u64` — LSN of the write that produced this page's current content, or `0` if never written under WAL |

  `page_type` is a single namespace shared across this ADR and higher
  layers: `2` = file header (this ADR, page 1 only — see below); `3` =
  free-list trunk (this ADR — see below); `0` (leaf) and `1` (internal)
  are [0016](0016-btree-node-format.md)'s B+-tree node types. Every page
  is thus self-describing at a glance, page 1 included, rather than
  page 1 being an untyped special case while every other allocated page
  carries a type tag. `page_lsn` is purely reserved space at this ADR's
  layer — the pager writes zeros and never reads it back; only
  [0017](0017-wal-format.md)'s write path gives it a real value, once a
  WAL exists to produce LSNs from.
- **A 64-byte preamble** holds, at minimum: the common page header,
  magic bytes (format identification), a format/version tag, the
  database's page size, and the free-list trunk head pointer (page id
  of the first trunk page, or `0` if there are no free pages at all —
  see below). It's read with a plain fixed-size (64-byte) read *before*
  the page layer computes any page-size-dependent offset — this is what
  breaks the chicken-and-egg problem, and now also means opening a
  database learns page size *and* free-list state from that same single
  read, no second page-sized read needed. All multi-byte fields
  little-endian.

  | Offset | Length | Description |
  |---|---|---|
  | 0 | 1 | `page_type: u8` = 2 (file header) |
  | 1 | 1 | reserved |
  | 2 | 8 | `page_lsn: u64` — common header field, effectively unused on page 1 (see below) |
  | 10 | 8 | magic bytes `b"BSLTPAGE"` |
  | 18 | 4 | `format_version: u32` |
  | 22 | 4 | `page_size: u32` |
  | 26 | 8 | `free_trunk_head: PageId` (u64) — page id of the first free-list trunk page, or `0` if there are no free pages |
  | 34 | 30 | reserved (zeroed) |

  64 bytes is deliberately generous relative to those fields, leaving
  headroom (the reserved 30 bytes above) for fields identified later
  (e.g. a checksum) without a format-breaking resize. Magic bytes no
  longer sit at absolute file offset 0 (the common header does instead)
  — they're still the first thing checked, at a fixed offset within the
  same single bootstrap read, so this costs nothing functionally; see
  Consequences. Page 1's own writes (creation, and `free_trunk_head`
  updates from `allocate_page`/`free_page`) aren't produced by
  [0017](0017-wal-format.md)'s WAL-logged write path, so its `page_lsn`
  stays `0` under the current design — the same pre-existing gap
  [backlog.md](../backlog.md) already tracks for free-list crash safety,
  not a new one.
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
  out-of-band sentinel: the free-list trunk head/`next_trunk` pointer's
  "no next trunk" terminator is `0` (not `u64::MAX`), and
  [0017](0017-wal-format.md)'s `PageDiff.base_page_id` "no base" case
  reuses the same convention.
- **Free-list structure: trunk pages + contentless leaves.** The
  free-list is a chain of **trunk pages**, each one a batch of free page
  ids; the free pages themselves (the ids a trunk lists) carry no
  structure at all. A trunk page carries the common page header (its
  `page_type` is `3`, in the same namespace as file header (`2`) and
  B+-tree leaf/internal (`0`/`1`) — see [0016](0016-btree-node-format.md)):

  | Offset | Length | Description |
  |---|---|---|
  | 0 | 1 | `page_type: u8` = 3 (free-list trunk) |
  | 1 | 1 | reserved |
  | 2 | 8 | `page_lsn: u64` — common header field; see below (same "not WAL-logged yet" caveat as page 1's) |
  | 10 | 8 | `next_trunk: PageId` (u64) — the next trunk page, or `0` if this is the last trunk |
  | 18 | 2 | `num_leaves: u16` — count of free page ids listed below |
  | 20 | `num_leaves * 8` | free page ids (`PageId`/u64 each) — pages available for `allocate_page` to hand out |
  | `20 + num_leaves * 8` | (rest of page) | unused |

  A trunk's capacity — how many leaf ids it can list — is
  `(page_size - 20) / 8`, e.g. 5 at the minimum page size of 64, several
  hundred at a typical 4 KiB page size.

  **`allocate_page`**: if `free_trunk_head == 0`, no free pages exist;
  extend the file by one page as before. Otherwise read the trunk at
  `free_trunk_head`:
  - if `num_leaves > 0`, pop the last leaf id from its list (decrement
    `num_leaves`, rewrite the trunk), and return that id — the trunk
    page itself is untouched otherwise, staying the head.
  - if `num_leaves == 0` (this trunk is exhausted), set
    `free_trunk_head` to this trunk's `next_trunk`, and return *this
    trunk's own page id* as the allocated page — an empty trunk has
    nothing left to offer but itself, so it gets recycled rather than
    permanently stranded.

  **`free_page(page_id)`**: read the trunk at `free_trunk_head` (if any).
  If one exists and has spare capacity (`num_leaves <
  (page_size - 20) / 8`), append `page_id` to its leaf list, increment
  `num_leaves`, and rewrite the trunk. Otherwise (no trunk yet, or the
  head trunk is full) `page_id` itself *becomes the new head trunk*:
  write a trunk header into it (`next_trunk` = the old `free_trunk_head`,
  `num_leaves` = 0), and set `free_trunk_head` to `page_id`. This one
  rule handles both "first page ever freed" (old head was `0`) and
  "current trunk is full" (old head becomes this new trunk's `next`)
  identically.

  A page that's listed as a leaf in some trunk has no defined content —
  it's free for `allocate_page` to hand out, unspecified until then. A
  trunk page itself is never handed out by `allocate_page` while it's
  still the current head with leaves or a further chain — it holds live
  free-list metadata, not free space, until its own turn to be recycled
  (above).
- **Reading a page currently listed as a free leaf** is
  defined-but-unspecified: it returns whatever bytes are physically
  there, no guarantee about content, but it is not a reported error. A
  trunk page's content, by contrast, *is* specified (the table above) —
  only leaf-listed pages are opaque. Reads/writes past the allocated
  range (beyond what the allocator has ever handed out) are errors
  regardless.

## Consequences

- Opening a database is a single fixed 64-byte read (the first 64 bytes
  of the file, i.e. of page 1 — page 1 always starts at offset 0): it
  validates the file (magic/version), learns page size, and restores
  the free-list trunk head to resume allocation, all from the same
  read. (Revised 2026-07-17 — previously a two-step read, the fixed
  preamble read followed by a page-sized read of a separate allocator
  page; merging the free-list head into the preamble drops that second
  read.) No directory scan, no format sniffing beyond the magic bytes.
  That bootstrap read is the *only* special-cased I/O page 1 gets: once
  page size is known (immediately after, every time), page 1's own
  offset (`0`) is computable the same way any other page's is, so
  ordinary `read_page`/`write_page` calls against it work like any
  other page — updating the free-list trunk head on every allocate/free
  is a normal page write, not a distinct code path.
- Magic bytes now sit at offset 2 rather than absolute file offset 0,
  since the common page header (`page_type` + reserved) comes first.
  This has no functional cost — both are read as part of the same fixed
  64-byte bootstrap read, and the magic check runs before anything else
  is trusted either way — but it does mean a raw hex dump or `file`-like
  tool can no longer identify the format from byte 0 alone; a
  format-detection tool would need to know to look at offset 2. Accepted
  as the price of every page, including page 1, being uniformly
  type-tagged.
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
- The preamble's 64 content bytes must fit *within* page 1, so the
  minimum page size floor is 64 (the trunk page format's own floor is
  much lower — 20 header bytes plus room for at least one leaf id needs
  only 28 — so the preamble stays the binding constraint). This isn't a
  new minimum/maximum page-size *policy* decision (still open, see
  [backlog.md](../backlog.md)), just the floor this revision's own
  layout requires to stay internally consistent.
- Free page content goes back to being *fully* unconstrained — no
  next-pointer or any other structure required, unlike the original
  singly-linked design. Only trunk pages (a small, bounded fraction of
  total free pages, one per `leaves-per-trunk` freed pages) carry
  defined structure and the common page header; this is a strict
  reduction in how much of the free-list's total page count needs to be
  self-describing, not an increase.
- This format is scoped to chunk 1 only: no crash safety (chunk 3's
  WAL), no concurrent access (chunk 4's MVCC), no tree-structure
  interpretation of page contents (chunk 2+). A page, at this layer, is
  an opaque fixed-size byte buffer plus the one bit of structure the
  common page header and, for trunk pages, the free-list's own format
  impose.
- The common header grows from 2 to 10 bytes (adding `page_lsn`), which
  ripples into every page-type-specific header defined so far: the file
  header's content-bytes shift by 8 (still well within the 64-byte
  preamble), the trunk header shifts by 8 (capacity per trunk drops from
  6 to 5 leaf ids at the minimum page size — negligible at any realistic
  page size), and [0016](0016-btree-node-format.md)'s leaf/internal node
  headers shift by 8 (slightly less cell-region capacity per page).
  [0017](0017-wal-format.md) additionally has to account for `page_lsn`
  always differing between a base page and its COW successor, which
  interacts with that ADR's prefix-trim diff computation — addressed
  there, not here.
