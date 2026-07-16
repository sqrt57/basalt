# ADR 0016: Copy-on-Write B+-Tree — Node Format & Mutation Semantics

Status: Proposed (2026-07-16)

## Context

[stage1-plan.md chunk 2](../stage1-plan.md) needs insert/lookup/delete/
range-scan over the pages [chunk 1](0015-page-storage-format.md) already
provides, with copy-on-write mechanics
([0008](0008-concurrency-durability.md)): a write never mutates a live
page, it allocates new pages along the path to the root instead. None of
that has a concrete node layout yet, and — unlike chunk 1's format,
which the pager itself needed — this is also the first chunk to give
page *contents* structure, so several things have to be decided at once:
what a key/value actually is at this layer, how a node's bytes are laid
out inside one fixed-size page, how splits/deletes propagate copy-on-
write up to the root, and how range-scan works without the classic
leaf-sibling pointers COW makes expensive to maintain.

This tree is generic (byte-string keys/values) — it is *not* the row
store itself. Tuple encoding, table/schema catalog, and the scan/seek
iterator interface the executor eventually drives are chunk 5
([0004](0004-relational-storage-engine.md)); chunk 2 only has to give
chunk 5 a correct keyed container to build on.

## Decision

**Key/value model**: both keys and values are arbitrary-length byte
strings (`&[u8]` in, `Vec<u8>` out). Keys order byte-wise
(lexicographic), not by any type-aware collation — type-aware ordering
is a row-store-layer concern (chunk 5+), not this layer's. `insert`
**upserts**: inserting an already-present key overwrites its value
rather than erroring, since the row store needs update-in-place
semantics and a separate upsert-vs-insert distinction isn't needed yet.

**Node layout**: a slotted page, one node per page, reusing chunk 1's
opaque fixed-size pages with no pager-level changes. Both node types
share the same header shape, slot-directory mechanics, and cell-region
growth; they differ in what sits between the header and the slot
directory, and in what a cell holds.

*Leaf node*:

- 8-byte header at offset 0: `node_type: u8` = 0, 1 reserved byte,
  `num_entries: u16`, `free_start: u16` (end of the slot directory),
  `free_end: u16` (start of the cell data region) — all multi-byte
  fields little-endian, matching [0015](0015-page-storage-format.md)'s
  convention.
- Slot directory immediately follows the header, at offset 8:
  `num_entries` fixed-size 4-byte slots (`offset: u16`, `length: u16`),
  kept sorted by key, each pointing at a cell in the data region.
- Cell data region grows downward from the end of the page as cells are
  appended; `free_end` tracks its current start. The node is full when
  `free_end - free_start` can't fit a new cell plus its 4-byte slot.
- Cell: `key_len: u16`, key bytes, `value_len: u16`, value bytes.

*Internal node*:

- Same 8-byte header shape as a leaf, at offset 0, with `node_type: u8`
  = 1.
- An extra 8-byte `leftmost_child: PageId` field immediately after the
  header, at offset 8..16. An internal node with N separator keys has
  N+1 children; storing N+1 pointers against N slots needs one pointer
  outside the slot array, so the leftmost child is pulled into the
  header and every slot's cell carries the child *to its right*.
- Slot directory follows at offset 16 (after `leftmost_child`): same
  4-byte slot shape and sort-by-key convention as a leaf's.
- Cell data region: same downward-growth convention as a leaf's.
- Cell: `key_len: u16`, key bytes, `child: PageId` (u64) — the subtree
  for keys in `[this key, next slot's key)`, or `[this key, +inf)` for
  the last slot.

Both node types share two further properties:

- No fixed fanout/order parameter — capacity is however many
  variable-length cells fit in one page, the SQLite/InnoDB approach
  rather than a classic fixed-N B-tree.
- **No overflow pages**: a single key+value pair that can't fit in an
  otherwise-empty page's cell region is a reported error
  (`EntryTooLarge`), not silently split across pages. Real payloads
  large enough to hit this are a row-store-layer problem (chunk 5+); not
  designed here (see [backlog.md](../backlog.md)).

**Copy-on-write mutation**: every insert/delete builds an updated node
**in memory** first (decoded from the old page, if any), then encodes it
into a **newly allocated page** — the old page is left untouched and not
freed (reclamation is [0013](0013-page-reclamation.md)'s job, staged for
chunk 4; this chunk has no reader table to know when it's safe). The
new child page id is threaded up through every ancestor on the path,
each of which is itself copied to a new page for the same reason,
terminating in a new root. The tree handle holding `root: PageId` is
**in-memory only** for this chunk — nothing writes it to page 0 or
anywhere else durable; that's chunk 3/4's concern (WAL, then MVCC root
history). A process restart loses track of the root unless the caller
remembered the `PageId` itself, even though the pages it points to are
genuinely durable (chunk 1's pager already persists page bytes across
close/reopen).

- **Insert/split**: descend to the target leaf, insert/overwrite the
  entry in the decoded copy. If the re-encoded node fits one page,
  write it to a new page id and propagate that id up (each ancestor
  just swaps the one child pointer that changed, and is itself
  rewritten to a new page). If it doesn't fit, split the entries into
  two new pages (roughly even, first pass at a partition that makes
  both halves fit — not necessarily an exact midpoint), and propagate
  `(separator_key, right_page_id)` up to be inserted into the parent as
  a new slot, recursing the same overflow check at that level. If the
  root itself splits, a new root page is allocated one level taller,
  `leftmost_child` = old root's left half, one slot = the promoted
  separator and the right half.
- **Delete**: descend to the target leaf; if the key isn't present,
  nothing is written and the tree is unchanged (`delete` reports
  whether the key existed, same as a map). If present, remove the entry
  from the decoded leaf and write it to a new page (even if now
  underfull), propagating the new child pointer up the same way insert
  does. **No rebalancing or merging of underfull nodes in this chunk**
  — internal nodes never lose entries here, they only ever repoint one
  child. This is a deliberate scope cut, not an oversight: it keeps
  delete's COW propagation symmetric with insert's (always exactly one
  pointer swap per ancestor level, never a variable-width change), and
  nothing in chunk 2's acceptance criteria needs space reclamation.
  Left open for later (see [backlog.md](../backlog.md)).
- **Range-scan**: no leaf-sibling pointers — maintaining them under COW
  would mean touching a leaf's neighbor on every split/delete, cascading
  writes further than the path to the root. Instead, range-scan descends
  once, keeping the path of internal nodes and a "next child index" per
  level (a cursor stack), and advances by walking back up the stack to
  the next unvisited child and back down its leftmost path when the
  current leaf is exhausted — the standard technique for
  sibling-pointer-free COW trees (LMDB takes the same approach). Pruned
  by comparing each child's key range against the scan bounds, so a
  bounded range-scan doesn't have to visit the whole tree.

## Consequences

- Node contents have real structure for the first time; chunk 1's
  "opaque fixed-size buffer" framing stops applying to allocated pages
  once a tree is layered on top; free-listed pages are still opaque.
- Every mutation costs `O(tree height)` newly allocated pages, same
  write-amplification tradeoff [0008](0008-concurrency-durability.md)
  already accepted for the COW approach generally — this ADR just makes
  it concrete at the node level (one new page per level touched, not
  per key).
- Deletes leaking underfull nodes (no merge) means a tree with heavy
  delete traffic gets sparser over time with no chunk-2-level fix;
  revisited once reclamation exists (chunk 4) or is flagged as its own
  problem (see [backlog.md](../backlog.md)).
- Every mutation also leaks its old page chain (no reclamation yet) —
  expected and harmless for chunk 2's in-memory-tree-correctness scope,
  but means the data file only grows across a long test run; not a
  regression chunk 1's allocator needs to account for.
- No overflow pages means there's a hard per-entry size ceiling
  (roughly one page's cell capacity, minus its own header/slot
  overhead) that the row store (chunk 5+) will have to either respect
  or design around (e.g. large-value overflow chains) — not solved
  here.
- Range-scan pruning depends on the tree actually being ordered
  correctly by the insert/split logic above; a bug there would silently
  under-return rather than error, which is why acceptance criteria
  (see [stage1-plan.md](../stage1-plan.md)) need dedicated multi-split
  range coverage, not just single-key lookup coverage.
