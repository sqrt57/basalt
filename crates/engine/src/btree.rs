//! Copy-on-write B+-tree over pages from [`crate::pager`].
//! Node format and mutation semantics decided in
//! `design/decisions/0016-btree-node-format.md`.

use std::ops::Bound;

use crate::pager::{PageId, Pager, PagerError};

const LEAF_HEADER_SIZE: usize = 8;
const INTERNAL_HEADER_SIZE: usize = 16;
const SLOT_SIZE: usize = 4;
const NODE_TYPE_LEAF: u8 = 0;
const NODE_TYPE_INTERNAL: u8 = 1;

type Entries = Vec<(Vec<u8>, Vec<u8>)>;

#[derive(Debug)]
pub enum BTreeError {
    Pager(PagerError),
    /// A single key/value pair too large to fit in an otherwise-empty page.
    EntryTooLarge { key_len: usize, value_len: usize },
    /// A node's on-disk bytes don't parse as a valid leaf/internal node, an
    /// internal invariant was violated, or an underlying WAL write failed.
    Corrupt(String),
}

impl From<PagerError> for BTreeError {
    fn from(e: PagerError) -> Self {
        BTreeError::Pager(e)
    }
}

impl std::fmt::Display for BTreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BTreeError::Pager(e) => write!(f, "{e}"),
            BTreeError::EntryTooLarge { key_len, value_len } => write!(
                f,
                "key ({key_len} bytes) + value ({value_len} bytes) too large for one page"
            ),
            BTreeError::Corrupt(msg) => write!(f, "corrupt b-tree node: {msg}"),
        }
    }
}

impl std::error::Error for BTreeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BTreeError::Pager(e) => Some(e),
            _ => None,
        }
    }
}

/// What a B+-tree mutation writes new node pages through. `Pager` alone
/// (chunk 2 scope: in-memory tree, no WAL) implements this directly;
/// `crate::engine::WalStore` (chunk 3) also logs each write as it happens.
pub trait NodeStore {
    fn page_size(&self) -> usize;
    fn read_page(&mut self, id: PageId, buf: &mut [u8]) -> Result<(), BTreeError>;
    /// Allocates a fresh page and writes `buf` to it, returning its id.
    /// `base` is the page this content was derived from (`None` if it's
    /// brand new, not a rewrite of an existing node) — plain `Pager` use
    /// ignores it; a logging store uses it to compute a WAL diff.
    fn write_new_node(&mut self, base: Option<PageId>, buf: &[u8]) -> Result<PageId, BTreeError>;
}

impl NodeStore for Pager {
    fn page_size(&self) -> usize {
        Pager::page_size(self) as usize
    }

    fn read_page(&mut self, id: PageId, buf: &mut [u8]) -> Result<(), BTreeError> {
        Pager::read_page(self, id, buf)?;
        Ok(())
    }

    fn write_new_node(&mut self, _base: Option<PageId>, buf: &[u8]) -> Result<PageId, BTreeError> {
        let id = Pager::allocate_page(self)?;
        Pager::write_page(self, id, buf)?;
        Ok(id)
    }
}

#[derive(Debug, Clone)]
struct LeafEntry {
    key: Vec<u8>,
    value: Vec<u8>,
}

#[derive(Debug, Clone)]
struct InternalEntry {
    key: Vec<u8>,
    /// Subtree for keys in `[key, next entry's key)`, or `[key, +inf)` for
    /// the last entry.
    child: PageId,
}

#[derive(Debug, Clone)]
enum Node {
    Leaf(Vec<LeafEntry>),
    Internal {
        leftmost: PageId,
        entries: Vec<InternalEntry>,
    },
}

/// A handle onto a copy-on-write B+-tree rooted at a page.
///
/// The root is tracked only in memory (`design/stage1-plan.md` chunk 2):
/// nothing persists it to the data file. A crash, or simply dropping this
/// handle, loses the tree unless the caller remembers `root()` itself —
/// the pages themselves are still genuinely durable via the pager. Chunk 3
/// (`crate::engine`) makes the root recoverable via the WAL instead of
/// changing this.
pub struct BTree {
    root: PageId,
}

enum InsertOutcome {
    Fit(PageId),
    Split {
        left: PageId,
        sep_key: Vec<u8>,
        right: PageId,
    },
}

impl BTree {
    /// Allocates a fresh page holding an empty leaf node and returns a tree
    /// rooted there.
    pub fn create_empty<S: NodeStore>(store: &mut S) -> Result<Self, BTreeError> {
        let page_size = store.page_size();
        let buf = try_encode_leaf(&[], page_size).ok_or_else(|| {
            BTreeError::Corrupt("page size too small to hold even an empty leaf node".into())
        })?;
        let id = store.write_new_node(None, &buf)?;
        Ok(BTree { root: id })
    }

    /// Reconstructs a tree handle from a previously observed root page id —
    /// e.g. one captured before a mutation, to inspect the pre-mutation
    /// snapshot COW is supposed to have left untouched.
    pub fn from_root(root: PageId) -> Self {
        BTree { root }
    }

    pub fn root(&self) -> PageId {
        self.root
    }

    /// Inserts `key` → `value`, or overwrites the value if `key` is already
    /// present (upsert). Never mutates a live page: writes new pages along
    /// the path to the root and leaves old ones untouched.
    pub fn insert<S: NodeStore>(&mut self, store: &mut S, key: &[u8], value: &[u8]) -> Result<(), BTreeError> {
        let page_size = store.page_size();
        let solo = vec![LeafEntry {
            key: key.to_vec(),
            value: value.to_vec(),
        }];
        if try_encode_leaf(&solo, page_size).is_none() {
            return Err(BTreeError::EntryTooLarge {
                key_len: key.len(),
                value_len: value.len(),
            });
        }

        match insert_into(store, self.root, key, value)? {
            InsertOutcome::Fit(new_root) => self.root = new_root,
            InsertOutcome::Split { left, sep_key, right } => {
                let entries = vec![InternalEntry { key: sep_key, child: right }];
                let buf = try_encode_internal(left, &entries, page_size)
                    .ok_or_else(|| BTreeError::Corrupt("new root overflows its page".into()))?;
                let id = store.write_new_node(None, &buf)?;
                self.root = id;
            }
        }
        Ok(())
    }

    /// Returns the value for `key`, or `None` if absent.
    pub fn lookup<S: NodeStore>(&self, store: &mut S, key: &[u8]) -> Result<Option<Vec<u8>>, BTreeError> {
        lookup_in(store, self.root, key)
    }

    /// Removes `key` if present. Returns whether it was present; deleting
    /// an absent key is a no-op, not an error. Never rebalances/merges
    /// underfull nodes (see `design/decisions/0016-btree-node-format.md`).
    pub fn delete<S: NodeStore>(&mut self, store: &mut S, key: &[u8]) -> Result<bool, BTreeError> {
        match delete_from(store, self.root, key)? {
            Some(new_root) => {
                self.root = new_root;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Returns every `(key, value)` pair with a key in `[start, end)`
    /// (per the given bounds), in ascending key order.
    pub fn range<S: NodeStore>(
        &self,
        store: &mut S,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Result<Entries, BTreeError> {
        let mut out = Vec::new();
        collect_range(store, self.root, start, end, &mut out)?;
        Ok(out)
    }

    /// All entries in ascending key order.
    pub fn scan_all<S: NodeStore>(&self, store: &mut S) -> Result<Entries, BTreeError> {
        self.range(store, Bound::Unbounded, Bound::Unbounded)
    }
}

fn insert_into<S: NodeStore>(
    store: &mut S,
    node_id: PageId,
    key: &[u8],
    value: &[u8],
) -> Result<InsertOutcome, BTreeError> {
    match decode_node(store, node_id)? {
        Node::Leaf(mut entries) => {
            match entries.binary_search_by(|e| e.key.as_slice().cmp(key)) {
                Ok(i) => entries[i].value = value.to_vec(),
                Err(i) => entries.insert(
                    i,
                    LeafEntry {
                        key: key.to_vec(),
                        value: value.to_vec(),
                    },
                ),
            }
            write_leaf_or_split(store, node_id, entries)
        }
        Node::Internal { mut leftmost, mut entries } => {
            let idx = child_index(&entries, key);
            let child_id = if idx == 0 { leftmost } else { entries[idx - 1].child };
            match insert_into(store, child_id, key, value)? {
                InsertOutcome::Fit(new_child) => {
                    if idx == 0 {
                        leftmost = new_child;
                    } else {
                        entries[idx - 1].child = new_child;
                    }
                }
                InsertOutcome::Split { left, sep_key, right } => {
                    if idx == 0 {
                        leftmost = left;
                    } else {
                        entries[idx - 1].child = left;
                    }
                    entries.insert(idx, InternalEntry { key: sep_key, child: right });
                }
            }
            write_internal_or_split(store, node_id, leftmost, entries)
        }
    }
}

fn lookup_in<S: NodeStore>(store: &mut S, node_id: PageId, key: &[u8]) -> Result<Option<Vec<u8>>, BTreeError> {
    match decode_node(store, node_id)? {
        Node::Leaf(entries) => Ok(entries
            .binary_search_by(|e| e.key.as_slice().cmp(key))
            .ok()
            .map(|i| entries[i].value.clone())),
        Node::Internal { leftmost, entries } => {
            let idx = child_index(&entries, key);
            let child_id = if idx == 0 { leftmost } else { entries[idx - 1].child };
            lookup_in(store, child_id, key)
        }
    }
}

fn delete_from<S: NodeStore>(store: &mut S, node_id: PageId, key: &[u8]) -> Result<Option<PageId>, BTreeError> {
    let page_size = store.page_size();
    match decode_node(store, node_id)? {
        Node::Leaf(mut entries) => match entries.binary_search_by(|e| e.key.as_slice().cmp(key)) {
            Err(_) => Ok(None),
            Ok(i) => {
                entries.remove(i);
                let buf = try_encode_leaf(&entries, page_size)
                    .expect("removing an entry cannot make a leaf node overflow");
                let id = store.write_new_node(Some(node_id), &buf)?;
                Ok(Some(id))
            }
        },
        Node::Internal { mut leftmost, mut entries } => {
            let idx = child_index(&entries, key);
            let child_id = if idx == 0 { leftmost } else { entries[idx - 1].child };
            match delete_from(store, child_id, key)? {
                None => Ok(None),
                Some(new_child) => {
                    if idx == 0 {
                        leftmost = new_child;
                    } else {
                        entries[idx - 1].child = new_child;
                    }
                    let buf = try_encode_internal(leftmost, &entries, page_size)
                        .expect("repointing one child cannot make an internal node overflow");
                    let id = store.write_new_node(Some(node_id), &buf)?;
                    Ok(Some(id))
                }
            }
        }
    }
}

/// Index of the child that should hold `key`: 0 means `leftmost`, `i`
/// (i >= 1) means `entries[i - 1].child`.
fn child_index(entries: &[InternalEntry], key: &[u8]) -> usize {
    entries.partition_point(|e| e.key.as_slice() <= key)
}

fn write_leaf_or_split<S: NodeStore>(
    store: &mut S,
    base: PageId,
    entries: Vec<LeafEntry>,
) -> Result<InsertOutcome, BTreeError> {
    let page_size = store.page_size();
    if let Some(buf) = try_encode_leaf(&entries, page_size) {
        let id = store.write_new_node(Some(base), &buf)?;
        return Ok(InsertOutcome::Fit(id));
    }

    let (left, right) = split_leaf_entries(entries, page_size)?;
    let sep_key = right[0].key.clone();
    let left_buf = try_encode_leaf(&left, page_size)
        .ok_or_else(|| BTreeError::Corrupt("left half of leaf split still overflows".into()))?;
    let right_buf = try_encode_leaf(&right, page_size)
        .ok_or_else(|| BTreeError::Corrupt("right half of leaf split still overflows".into()))?;
    let left_id = store.write_new_node(Some(base), &left_buf)?;
    let right_id = store.write_new_node(Some(base), &right_buf)?;
    Ok(InsertOutcome::Split { left: left_id, sep_key, right: right_id })
}

fn write_internal_or_split<S: NodeStore>(
    store: &mut S,
    base: PageId,
    leftmost: PageId,
    entries: Vec<InternalEntry>,
) -> Result<InsertOutcome, BTreeError> {
    let page_size = store.page_size();
    if let Some(buf) = try_encode_internal(leftmost, &entries, page_size) {
        let id = store.write_new_node(Some(base), &buf)?;
        return Ok(InsertOutcome::Fit(id));
    }

    let (l_leftmost, l_entries, sep_key, r_leftmost, r_entries) =
        split_internal_entries(leftmost, entries, page_size)?;
    let left_buf = try_encode_internal(l_leftmost, &l_entries, page_size)
        .ok_or_else(|| BTreeError::Corrupt("left half of internal split still overflows".into()))?;
    let right_buf = try_encode_internal(r_leftmost, &r_entries, page_size)
        .ok_or_else(|| BTreeError::Corrupt("right half of internal split still overflows".into()))?;
    let left_id = store.write_new_node(Some(base), &left_buf)?;
    let right_id = store.write_new_node(Some(base), &right_buf)?;
    Ok(InsertOutcome::Split { left: left_id, sep_key, right: right_id })
}

fn split_leaf_entries(
    mut entries: Vec<LeafEntry>,
    page_size: usize,
) -> Result<(Vec<LeafEntry>, Vec<LeafEntry>), BTreeError> {
    let sizes: Vec<usize> = entries.iter().map(|e| 2 + e.key.len() + 2 + e.value.len()).collect();
    let split_at = choose_split_point(&sizes, LEAF_HEADER_SIZE, page_size)
        .ok_or_else(|| BTreeError::Corrupt("leaf node cannot be split to fit the page size".into()))?;
    let right = entries.split_off(split_at);
    Ok((entries, right))
}

#[allow(clippy::type_complexity)]
fn split_internal_entries(
    leftmost: PageId,
    mut entries: Vec<InternalEntry>,
    page_size: usize,
) -> Result<(PageId, Vec<InternalEntry>, Vec<u8>, PageId, Vec<InternalEntry>), BTreeError> {
    let n = entries.len();
    let sizes: Vec<usize> = entries.iter().map(|e| 2 + e.key.len() + 8).collect();
    let fits = |lo: usize, hi: usize| -> bool {
        let cnt = hi.saturating_sub(lo);
        let sum: usize = sizes[lo..hi].iter().sum();
        INTERNAL_HEADER_SIZE + cnt * SLOT_SIZE + sum <= page_size
    };

    let mut candidates: Vec<usize> = (0..n).collect();
    candidates.sort_by_key(|&k| (k as isize - n as isize / 2).unsigned_abs());
    for k in candidates {
        if fits(0, k) && fits(k + 1, n) {
            let right_leftmost = entries[k].child;
            let promoted_key = entries[k].key.clone();
            let right_entries = entries.split_off(k + 1);
            let mut left_entries = entries;
            left_entries.truncate(k);
            return Ok((leftmost, left_entries, promoted_key, right_leftmost, right_entries));
        }
    }
    Err(BTreeError::Corrupt(
        "internal node cannot be split to fit the page size".into(),
    ))
}

/// Picks an index in `1..sizes.len()` splitting `sizes` into two halves that
/// each fit `page_size` once `overhead` (node header) and per-entry slots
/// are accounted for, biased toward an even split.
fn choose_split_point(sizes: &[usize], overhead: usize, page_size: usize) -> Option<usize> {
    let n = sizes.len();
    if n < 2 {
        return None;
    }
    let mut prefix = vec![0usize; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + sizes[i];
    }
    let total = prefix[n];
    let target = total / 2;

    let fits = |lo: usize, hi: usize| -> bool {
        let cnt = hi - lo;
        overhead + cnt * SLOT_SIZE + (prefix[hi] - prefix[lo]) <= page_size
    };

    let mut candidates: Vec<usize> = (1..n).collect();
    candidates.sort_by_key(|&i| (prefix[i] as isize - target as isize).unsigned_abs());
    candidates.into_iter().find(|&i| fits(0, i) && fits(i, n))
}

fn key_before_start(key: &[u8], start: Bound<&[u8]>) -> bool {
    match start {
        Bound::Unbounded => false,
        Bound::Included(s) => key < s,
        Bound::Excluded(s) => key <= s,
    }
}

fn key_after_end(key: &[u8], end: Bound<&[u8]>) -> bool {
    match end {
        Bound::Unbounded => false,
        Bound::Included(e) => key > e,
        Bound::Excluded(e) => key >= e,
    }
}

/// True if every key in `[-inf, upper_exclusive)` is before `start`.
fn range_before_start(upper_exclusive: &[u8], start: Bound<&[u8]>) -> bool {
    match start {
        Bound::Unbounded => false,
        Bound::Included(s) | Bound::Excluded(s) => upper_exclusive <= s,
    }
}

/// True if every key in `[lower_inclusive, +inf)` is after `end`.
fn range_after_end(lower_inclusive: &[u8], end: Bound<&[u8]>) -> bool {
    match end {
        Bound::Unbounded => false,
        Bound::Included(e) => lower_inclusive > e,
        Bound::Excluded(e) => lower_inclusive >= e,
    }
}

fn collect_range<S: NodeStore>(
    store: &mut S,
    node_id: PageId,
    start: Bound<&[u8]>,
    end: Bound<&[u8]>,
    out: &mut Entries,
) -> Result<(), BTreeError> {
    match decode_node(store, node_id)? {
        Node::Leaf(entries) => {
            for e in entries {
                if key_before_start(&e.key, start) {
                    continue;
                }
                if key_after_end(&e.key, end) {
                    break;
                }
                out.push((e.key, e.value));
            }
            Ok(())
        }
        Node::Internal { leftmost, entries } => {
            let n = entries.len();
            for i in 0..=n {
                let child = if i == 0 { leftmost } else { entries[i - 1].child };
                if i < n && range_before_start(&entries[i].key, start) {
                    continue;
                }
                if i > 0 && range_after_end(&entries[i - 1].key, end) {
                    break;
                }
                collect_range(store, child, start, end, out)?;
            }
            Ok(())
        }
    }
}

fn decode_node<S: NodeStore>(store: &mut S, id: PageId) -> Result<Node, BTreeError> {
    let page_size = store.page_size();
    let mut buf = vec![0u8; page_size];
    store.read_page(id, &mut buf)?;
    decode_bytes(&buf)
}

fn decode_bytes(buf: &[u8]) -> Result<Node, BTreeError> {
    let node_type = buf[0];
    let num_entries = u16::from_le_bytes(buf[2..4].try_into().unwrap()) as usize;

    match node_type {
        NODE_TYPE_LEAF => {
            let mut entries = Vec::with_capacity(num_entries);
            for i in 0..num_entries {
                let slot_off = LEAF_HEADER_SIZE + i * SLOT_SIZE;
                let (cell_off, cell_len) = read_slot(buf, slot_off);
                let cell = &buf[cell_off..cell_off + cell_len];
                let key_len = u16::from_le_bytes(cell[0..2].try_into().unwrap()) as usize;
                let key = cell[2..2 + key_len].to_vec();
                let val_off = 2 + key_len;
                let value_len = u16::from_le_bytes(cell[val_off..val_off + 2].try_into().unwrap()) as usize;
                let value = cell[val_off + 2..val_off + 2 + value_len].to_vec();
                entries.push(LeafEntry { key, value });
            }
            Ok(Node::Leaf(entries))
        }
        NODE_TYPE_INTERNAL => {
            let leftmost = u64::from_le_bytes(buf[8..16].try_into().unwrap());
            let mut entries = Vec::with_capacity(num_entries);
            for i in 0..num_entries {
                let slot_off = INTERNAL_HEADER_SIZE + i * SLOT_SIZE;
                let (cell_off, cell_len) = read_slot(buf, slot_off);
                let cell = &buf[cell_off..cell_off + cell_len];
                let key_len = u16::from_le_bytes(cell[0..2].try_into().unwrap()) as usize;
                let key = cell[2..2 + key_len].to_vec();
                let child = u64::from_le_bytes(cell[2 + key_len..2 + key_len + 8].try_into().unwrap());
                entries.push(InternalEntry { key, child });
            }
            Ok(Node::Internal { leftmost, entries })
        }
        other => Err(BTreeError::Corrupt(format!("unknown node type tag {other}"))),
    }
}

fn read_slot(buf: &[u8], slot_off: usize) -> (usize, usize) {
    let off = u16::from_le_bytes(buf[slot_off..slot_off + 2].try_into().unwrap()) as usize;
    let len = u16::from_le_bytes(buf[slot_off + 2..slot_off + 4].try_into().unwrap()) as usize;
    (off, len)
}

fn try_encode_leaf(entries: &[LeafEntry], page_size: usize) -> Option<Vec<u8>> {
    if page_size > u16::MAX as usize {
        return None;
    }
    let n = entries.len();
    if n > u16::MAX as usize {
        return None;
    }
    let mut cells = Vec::with_capacity(n);
    for e in entries {
        if e.key.len() > u16::MAX as usize || e.value.len() > u16::MAX as usize {
            return None;
        }
        let mut c = Vec::with_capacity(2 + e.key.len() + 2 + e.value.len());
        c.extend_from_slice(&(e.key.len() as u16).to_le_bytes());
        c.extend_from_slice(&e.key);
        c.extend_from_slice(&(e.value.len() as u16).to_le_bytes());
        c.extend_from_slice(&e.value);
        cells.push(c);
    }
    let cells_total: usize = cells.iter().map(Vec::len).sum();
    let required = LEAF_HEADER_SIZE + n * SLOT_SIZE + cells_total;
    if required > page_size {
        return None;
    }

    let free_start = LEAF_HEADER_SIZE + n * SLOT_SIZE;
    let free_end = page_size - cells_total;
    let mut buf = vec![0u8; page_size];
    buf[0] = NODE_TYPE_LEAF;
    buf[2..4].copy_from_slice(&(n as u16).to_le_bytes());
    buf[4..6].copy_from_slice(&(free_start as u16).to_le_bytes());
    buf[6..8].copy_from_slice(&(free_end as u16).to_le_bytes());

    let mut cursor = free_end;
    for (i, cell) in cells.iter().enumerate() {
        let slot_off = LEAF_HEADER_SIZE + i * SLOT_SIZE;
        buf[slot_off..slot_off + 2].copy_from_slice(&(cursor as u16).to_le_bytes());
        buf[slot_off + 2..slot_off + 4].copy_from_slice(&(cell.len() as u16).to_le_bytes());
        buf[cursor..cursor + cell.len()].copy_from_slice(cell);
        cursor += cell.len();
    }
    Some(buf)
}

fn try_encode_internal(leftmost: PageId, entries: &[InternalEntry], page_size: usize) -> Option<Vec<u8>> {
    if page_size > u16::MAX as usize {
        return None;
    }
    let n = entries.len();
    if n > u16::MAX as usize {
        return None;
    }
    let mut cells = Vec::with_capacity(n);
    for e in entries {
        if e.key.len() > u16::MAX as usize {
            return None;
        }
        let mut c = Vec::with_capacity(2 + e.key.len() + 8);
        c.extend_from_slice(&(e.key.len() as u16).to_le_bytes());
        c.extend_from_slice(&e.key);
        c.extend_from_slice(&e.child.to_le_bytes());
        cells.push(c);
    }
    let cells_total: usize = cells.iter().map(Vec::len).sum();
    let required = INTERNAL_HEADER_SIZE + n * SLOT_SIZE + cells_total;
    if required > page_size {
        return None;
    }

    let free_start = INTERNAL_HEADER_SIZE + n * SLOT_SIZE;
    let free_end = page_size - cells_total;
    let mut buf = vec![0u8; page_size];
    buf[0] = NODE_TYPE_INTERNAL;
    buf[2..4].copy_from_slice(&(n as u16).to_le_bytes());
    buf[4..6].copy_from_slice(&(free_start as u16).to_le_bytes());
    buf[6..8].copy_from_slice(&(free_end as u16).to_le_bytes());
    buf[8..16].copy_from_slice(&leftmost.to_le_bytes());

    let mut cursor = free_end;
    for (i, cell) in cells.iter().enumerate() {
        let slot_off = INTERNAL_HEADER_SIZE + i * SLOT_SIZE;
        buf[slot_off..slot_off + 2].copy_from_slice(&(cursor as u16).to_le_bytes());
        buf[slot_off + 2..slot_off + 4].copy_from_slice(&(cell.len() as u16).to_le_bytes());
        buf[cursor..cursor + cell.len()].copy_from_slice(cell);
        cursor += cell.len();
    }
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn new_pager(page_size: u32) -> (tempfile::TempDir, Pager) {
        let dir = tempdir().unwrap();
        let prefix = dir.path().join("db");
        let pager = Pager::create(&prefix, page_size).unwrap();
        (dir, pager)
    }

    #[test]
    fn empty_tree_lookup_and_scan() {
        let (_dir, mut pager) = new_pager(256);
        let tree = BTree::create_empty(&mut pager).unwrap();
        assert_eq!(tree.lookup(&mut pager, b"anything").unwrap(), None);
        assert_eq!(tree.scan_all(&mut pager).unwrap(), Vec::new());
    }

    #[test]
    fn insert_then_lookup_round_trips() {
        let (_dir, mut pager) = new_pager(256);
        let mut tree = BTree::create_empty(&mut pager).unwrap();
        tree.insert(&mut pager, b"foo", b"bar").unwrap();
        assert_eq!(tree.lookup(&mut pager, b"foo").unwrap(), Some(b"bar".to_vec()));
        assert_eq!(tree.lookup(&mut pager, b"missing").unwrap(), None);
    }

    #[test]
    fn insert_upserts_existing_key() {
        let (_dir, mut pager) = new_pager(256);
        let mut tree = BTree::create_empty(&mut pager).unwrap();
        tree.insert(&mut pager, b"k", b"v1").unwrap();
        tree.insert(&mut pager, b"k", b"v2").unwrap();
        assert_eq!(tree.lookup(&mut pager, b"k").unwrap(), Some(b"v2".to_vec()));
        assert_eq!(tree.scan_all(&mut pager).unwrap(), vec![(b"k".to_vec(), b"v2".to_vec())]);
    }

    #[test]
    fn delete_present_key_removes_it() {
        let (_dir, mut pager) = new_pager(256);
        let mut tree = BTree::create_empty(&mut pager).unwrap();
        tree.insert(&mut pager, b"k", b"v").unwrap();
        assert!(tree.delete(&mut pager, b"k").unwrap());
        assert_eq!(tree.lookup(&mut pager, b"k").unwrap(), None);
    }

    #[test]
    fn delete_absent_key_is_noop() {
        let (_dir, mut pager) = new_pager(256);
        let mut tree = BTree::create_empty(&mut pager).unwrap();
        tree.insert(&mut pager, b"k", b"v").unwrap();
        assert!(!tree.delete(&mut pager, b"missing").unwrap());
        assert_eq!(tree.lookup(&mut pager, b"k").unwrap(), Some(b"v".to_vec()));
    }

    #[test]
    fn entry_too_large_is_a_reported_error() {
        let (_dir, mut pager) = new_pager(64);
        let mut tree = BTree::create_empty(&mut pager).unwrap();
        let huge_value = vec![0u8; 1000];
        let err = tree.insert(&mut pager, b"k", &huge_value).unwrap_err();
        assert!(matches!(err, BTreeError::EntryTooLarge { .. }));
    }

    fn key_for(i: usize) -> Vec<u8> {
        format!("key{i:05}").into_bytes()
    }
    fn value_for(i: usize) -> Vec<u8> {
        format!("value{i:05}").into_bytes()
    }

    #[test]
    fn many_sequential_inserts_survive_splits() {
        let (_dir, mut pager) = new_pager(128);
        let mut tree = BTree::create_empty(&mut pager).unwrap();
        let n = 200;
        for i in 0..n {
            tree.insert(&mut pager, &key_for(i), &value_for(i)).unwrap();
        }
        for i in 0..n {
            assert_eq!(tree.lookup(&mut pager, &key_for(i)).unwrap(), Some(value_for(i)));
        }
        let scanned = tree.scan_all(&mut pager).unwrap();
        let expected: Vec<_> = (0..n).map(|i| (key_for(i), value_for(i))).collect();
        assert_eq!(scanned, expected);
    }

    #[test]
    fn many_shuffled_inserts_and_deletes_match_reference_map() {
        let (_dir, mut pager) = new_pager(128);
        let mut tree = BTree::create_empty(&mut pager).unwrap();
        let mut reference: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();

        let n = 150usize;
        // Deterministic pseudo-random permutation (n and 61 are coprime).
        let order: Vec<usize> = (0..n).map(|i| (i * 61) % n).collect();

        for &i in &order {
            tree.insert(&mut pager, &key_for(i), &value_for(i)).unwrap();
            reference.insert(key_for(i), value_for(i));
        }

        for &i in order.iter().step_by(3) {
            let existed = tree.delete(&mut pager, &key_for(i)).unwrap();
            let ref_existed = reference.remove(&key_for(i)).is_some();
            assert_eq!(existed, ref_existed);
        }

        for i in 0..n {
            assert_eq!(
                tree.lookup(&mut pager, &key_for(i)).unwrap(),
                reference.get(&key_for(i)).cloned()
            );
        }

        let scanned = tree.scan_all(&mut pager).unwrap();
        let expected: Vec<_> = reference.into_iter().collect();
        assert_eq!(scanned, expected);
    }

    #[test]
    fn range_scan_respects_bounds() {
        let (_dir, mut pager) = new_pager(128);
        let mut tree = BTree::create_empty(&mut pager).unwrap();
        let n = 100;
        for i in 0..n {
            tree.insert(&mut pager, &key_for(i), &value_for(i)).unwrap();
        }

        let start = key_for(20);
        let end = key_for(30);
        let got = tree
            .range(&mut pager, Bound::Included(start.as_slice()), Bound::Excluded(end.as_slice()))
            .unwrap();
        let expected: Vec<_> = (20..30).map(|i| (key_for(i), value_for(i))).collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn cow_old_root_unaffected_by_later_mutations() {
        let (_dir, mut pager) = new_pager(128);
        let mut tree = BTree::create_empty(&mut pager).unwrap();
        for i in 0..50 {
            tree.insert(&mut pager, &key_for(i), &value_for(i)).unwrap();
        }
        let old_root = tree.root();
        let snapshot_before: Vec<_> = (0..50).map(|i| (key_for(i), value_for(i))).collect();

        for i in 50..100 {
            tree.insert(&mut pager, &key_for(i), &value_for(i)).unwrap();
        }
        for i in (0..50).step_by(2) {
            tree.delete(&mut pager, &key_for(i)).unwrap();
        }

        let old_tree = BTree::from_root(old_root);
        let scanned_old = old_tree.scan_all(&mut pager).unwrap();
        assert_eq!(scanned_old, snapshot_before);

        // The live tree, meanwhile, reflects the later mutations.
        assert_eq!(tree.lookup(&mut pager, &key_for(0)).unwrap(), None);
        assert_eq!(tree.lookup(&mut pager, &key_for(99)).unwrap(), Some(value_for(99)));
    }

    #[test]
    fn cow_survives_reopen_via_remembered_root() {
        let dir = tempdir().unwrap();
        let prefix = dir.path().join("db");
        let root;
        {
            let mut pager = Pager::create(&prefix, 128).unwrap();
            let mut tree = BTree::create_empty(&mut pager).unwrap();
            for i in 0..50 {
                tree.insert(&mut pager, &key_for(i), &value_for(i)).unwrap();
            }
            root = tree.root();
        }
        let mut pager = Pager::open(&prefix).unwrap();
        let tree = BTree::from_root(root);
        for i in 0..50 {
            assert_eq!(tree.lookup(&mut pager, &key_for(i)).unwrap(), Some(value_for(i)));
        }
    }
}
