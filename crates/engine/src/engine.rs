//! Ties chunk 1 ([`Pager`]), chunk 2 ([`BTree`]), chunk 3 ([`Wal`]), and
//! chunk 4 (MVCC snapshots + reclamation) together into a durable,
//! recoverable, snapshot-isolated database handle.
//!
//! Not stage 1's chunk 8 embedded API (builder/options, transaction
//! handles) — a minimal handle sufficient to exercise and test chunk 3's
//! WAL wiring (`design/decisions/0017-wal-format.md`) and chunk 4's MVCC
//! snapshot/reclamation wiring (`design/decisions/0018-mvcc-snapshot-format.md`).

use std::collections::BTreeMap;
use std::path::Path;

use crate::btree::{BTree, BTreeError, NodeStore};
use crate::pager::{PageId, Pager, PagerError};
use crate::wal::{Lsn, Wal, WalError};

#[derive(Debug)]
pub enum DatabaseError {
    Pager(PagerError),
    Wal(WalError),
    BTree(BTreeError),
}

impl From<PagerError> for DatabaseError {
    fn from(e: PagerError) -> Self {
        DatabaseError::Pager(e)
    }
}

impl From<WalError> for DatabaseError {
    fn from(e: WalError) -> Self {
        DatabaseError::Wal(e)
    }
}

impl From<BTreeError> for DatabaseError {
    fn from(e: BTreeError) -> Self {
        DatabaseError::BTree(e)
    }
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseError::Pager(e) => write!(f, "{e}"),
            DatabaseError::Wal(e) => write!(f, "{e}"),
            DatabaseError::BTree(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DatabaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DatabaseError::Pager(e) => Some(e),
            DatabaseError::Wal(e) => Some(e),
            DatabaseError::BTree(e) => Some(e),
        }
    }
}

/// Writes new B+-tree node pages through both the pager and the WAL,
/// tracking each newly-dirtied page's recLSN for the next checkpoint and
/// collecting every superseded `base` page for chunk 4's reclamation
/// (`design/decisions/0018-mvcc-snapshot-format.md`) — provisional until
/// the transaction actually commits.
pub struct WalStore<'a> {
    pager: &'a mut Pager,
    wal: &'a mut Wal,
    dpt: &'a mut BTreeMap<PageId, Lsn>,
    superseded: &'a mut Vec<PageId>,
}

impl NodeStore for WalStore<'_> {
    fn page_size(&self) -> usize {
        self.pager.page_size() as usize
    }

    fn read_page(&mut self, id: PageId, buf: &mut [u8]) -> Result<(), BTreeError> {
        self.pager.read_page(id, buf)?;
        Ok(())
    }

    fn write_new_node(&mut self, base: Option<PageId>, buf: &[u8]) -> Result<PageId, BTreeError> {
        let page_size = self.pager.page_size() as usize;
        let mut base_bytes = vec![0u8; page_size];
        if let Some(base_id) = base {
            self.pager.read_page(base_id, &mut base_bytes)?;
        }

        let new_id = self.pager.allocate_page()?;
        self.pager.write_page(new_id, buf)?;

        let lsn = self
            .wal
            .append_page_diff(new_id, base, &base_bytes, buf)
            .map_err(|e| BTreeError::Corrupt(e.to_string()))?;
        self.dpt.entry(new_id).or_insert(lsn);

        if let Some(base_id) = base {
            self.superseded.push(base_id);
        }

        Ok(new_id)
    }
}

/// A point-in-time read handle: the root and generation current when
/// [`Database::begin_read`] was called. Cheap to copy, holds no borrow of
/// `Database` — reads go through `Database`'s `_at` methods instead of
/// through this handle directly (`design/decisions/0018-mvcc-snapshot-format.md`).
/// Must be released via [`Database::end_read`] or its pinned generation
/// (and everything superseded at or after it) is never reclaimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    generation: u64,
    root: Option<PageId>,
}

/// An open database: `<prefix>.data.bin` + `<prefix>.log.bin` together,
/// with every committed write recoverable via WAL redo, and MVCC snapshot
/// reads over the generations that write transactions produce
/// (`design/decisions/0018-mvcc-snapshot-format.md`).
pub struct Database {
    pager: Pager,
    wal: Wal,
    dpt: BTreeMap<PageId, Lsn>,
    tree: Option<BTree>,
    /// Generation the current root belongs to; bumped by 1 per successful
    /// commit. Not persisted — a fresh process restarts numbering at 0,
    /// which is fine since no prior process's readers survive a restart.
    current_generation: u64,
    /// Open snapshots' generations, each keyed to how many `Snapshot`s are
    /// currently pinned there. The low-water mark is its minimum key.
    reader_table: BTreeMap<u64, u32>,
    /// Pages superseded by a committed transaction, tagged with the
    /// generation they were superseded at, awaiting reclamation once no
    /// reader's generation is older than that.
    pending_reclaim: Vec<(PageId, u64)>,
}

impl Database {
    /// Creates a new, empty database at `path_prefix`.
    pub fn create(path_prefix: &Path, page_size: u32) -> Result<Self, DatabaseError> {
        let pager = Pager::create(path_prefix, page_size)?;
        let wal = Wal::create(path_prefix)?;
        Ok(Database {
            pager,
            wal,
            dpt: BTreeMap::new(),
            tree: None,
            current_generation: 0,
            reader_table: BTreeMap::new(),
            pending_reclaim: Vec::new(),
        })
    }

    /// Opens an existing database, replaying the WAL to recover whichever
    /// root the last committed transaction left current (`None` if no
    /// transaction ever committed).
    pub fn open(path_prefix: &Path) -> Result<Self, DatabaseError> {
        let mut pager = Pager::open(path_prefix)?;
        let mut wal = Wal::open(path_prefix)?;
        let root = wal.recover(&mut pager)?;
        let tree = root.map(BTree::from_root);
        Ok(Database {
            pager,
            wal,
            dpt: BTreeMap::new(),
            tree,
            current_generation: 0,
            reader_table: BTreeMap::new(),
            pending_reclaim: Vec::new(),
        })
    }

    /// Runs `f` against the tree and a logging store, then commits: logs
    /// and fsyncs a `Commit` record for the resulting root, advances the
    /// current generation, and queues every page `f` superseded for later
    /// reclamation. If `f` returns `Err`, nothing commits, the generation
    /// doesn't advance, and any pages `f` wrote become simply orphaned —
    /// never queued for reclamation, same as an unrecovered crash
    /// (`design/decisions/0018-mvcc-snapshot-format.md`).
    pub fn transaction<F>(&mut self, f: F) -> Result<(), DatabaseError>
    where
        F: FnOnce(&mut BTree, &mut WalStore) -> Result<(), BTreeError>,
    {
        // Captured so an aborted `f` can be undone below: `BTree::insert`/
        // `delete` mutate `self.tree`'s root in place as they go, so a
        // closure that does some successful writes and *then* returns
        // `Err` would otherwise leave those writes visible in `self.tree`
        // despite never reaching a WAL `Commit` record.
        let pre_root = self.tree.as_ref().map(BTree::root);
        let mut superseded = Vec::new();

        if self.tree.is_none() {
            let mut store = WalStore {
                pager: &mut self.pager,
                wal: &mut self.wal,
                dpt: &mut self.dpt,
                superseded: &mut superseded,
            };
            self.tree = Some(BTree::create_empty(&mut store)?);
        }
        let tree = self.tree.as_mut().expect("just ensured Some");

        let mut store = WalStore {
            pager: &mut self.pager,
            wal: &mut self.wal,
            dpt: &mut self.dpt,
            superseded: &mut superseded,
        };
        if let Err(e) = f(tree, &mut store) {
            self.tree = pre_root.map(BTree::from_root);
            return Err(e.into());
        }

        let tree = self.tree.as_ref().expect("still Some");
        self.wal.append_commit(tree.root())?;
        self.current_generation += 1;
        self.pending_reclaim
            .extend(superseded.into_iter().map(|p| (p, self.current_generation)));
        Ok(())
    }

    /// Begins a read-only snapshot pinned to the current generation and
    /// root. Read via `lookup_at`/`scan_all_at`/`range_at`, and release
    /// with [`Database::end_read`] when done.
    pub fn begin_read(&mut self) -> Snapshot {
        let generation = self.current_generation;
        *self.reader_table.entry(generation).or_insert(0) += 1;
        Snapshot { generation, root: self.tree.as_ref().map(BTree::root) }
    }

    /// Releases a snapshot taken via [`Database::begin_read`], unpinning
    /// its generation from the reader table.
    pub fn end_read(&mut self, snapshot: Snapshot) {
        if let std::collections::btree_map::Entry::Occupied(mut entry) =
            self.reader_table.entry(snapshot.generation)
        {
            let count = entry.get_mut();
            *count -= 1;
            if *count == 0 {
                entry.remove();
            }
        }
    }

    /// Looks up `key` as of `snapshot`, independent of any later commits.
    pub fn lookup_at(&mut self, snapshot: &Snapshot, key: &[u8]) -> Result<Option<Vec<u8>>, DatabaseError> {
        match snapshot.root {
            None => Ok(None),
            Some(root) => Ok(BTree::from_root(root).lookup(&mut self.pager, key)?),
        }
    }

    /// All entries in ascending key order, as of `snapshot`.
    #[allow(clippy::type_complexity)]
    pub fn scan_all_at(&mut self, snapshot: &Snapshot) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DatabaseError> {
        match snapshot.root {
            None => Ok(Vec::new()),
            Some(root) => Ok(BTree::from_root(root).scan_all(&mut self.pager)?),
        }
    }

    /// Entries with a key in `[start, end)`, as of `snapshot`.
    #[allow(clippy::type_complexity)]
    pub fn range_at(
        &mut self,
        snapshot: &Snapshot,
        start: std::ops::Bound<&[u8]>,
        end: std::ops::Bound<&[u8]>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DatabaseError> {
        match snapshot.root {
            None => Ok(Vec::new()),
            Some(root) => Ok(BTree::from_root(root).range(&mut self.pager, start, end)?),
        }
    }

    pub fn insert(&mut self, key: &[u8], value: &[u8]) -> Result<(), DatabaseError> {
        self.transaction(|tree, store| tree.insert(store, key, value))
    }

    pub fn delete(&mut self, key: &[u8]) -> Result<bool, DatabaseError> {
        let mut existed = false;
        self.transaction(|tree, store| {
            existed = tree.delete(store, key)?;
            Ok(())
        })?;
        Ok(existed)
    }

    pub fn lookup(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, DatabaseError> {
        match &self.tree {
            None => Ok(None),
            Some(tree) => Ok(tree.lookup(&mut self.pager, key)?),
        }
    }

    #[allow(clippy::type_complexity)]
    pub fn scan_all(&mut self) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DatabaseError> {
        match &self.tree {
            None => Ok(Vec::new()),
            Some(tree) => Ok(tree.scan_all(&mut self.pager)?),
        }
    }

    /// Snapshots the dirty-page table to the log, fsyncs the data file,
    /// then drops the now-durable entries. Also reclaims (frees) every
    /// pending-reclaim page superseded at or before the current low-water
    /// mark — the oldest open snapshot's generation, or the current
    /// generation if no snapshot is open
    /// (`design/decisions/0018-mvcc-snapshot-format.md`). Triggered only
    /// explicitly or on [`Database::close`] — no background trigger yet
    /// (see `design/backlog.md`).
    pub fn checkpoint(&mut self) -> Result<(), DatabaseError> {
        let dpt_snapshot: Vec<(PageId, Lsn)> = self.dpt.iter().map(|(&p, &l)| (p, l)).collect();
        self.wal.checkpoint(&mut self.pager, &dpt_snapshot)?;
        self.dpt.clear();

        let low_water_mark = self
            .reader_table
            .keys()
            .next()
            .copied()
            .unwrap_or(self.current_generation);
        let mut still_pending = Vec::with_capacity(self.pending_reclaim.len());
        for (page, superseded_at) in self.pending_reclaim.drain(..) {
            if superseded_at <= low_water_mark {
                self.pager.free_page(page)?;
            } else {
                still_pending.push((page, superseded_at));
            }
        }
        self.pending_reclaim = still_pending;
        Ok(())
    }

    /// Checkpoints, then consumes the handle.
    pub fn close(mut self) -> Result<(), DatabaseError> {
        self.checkpoint()
    }

    pub fn root(&self) -> Option<PageId> {
        self.tree.as_ref().map(BTree::root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::{Seek, SeekFrom, Write};
    use tempfile::tempdir;

    fn key_for(i: usize) -> Vec<u8> {
        format!("key{i:05}").into_bytes()
    }
    fn value_for(i: usize) -> Vec<u8> {
        format!("value{i:05}").into_bytes()
    }

    fn log_path(prefix: &Path) -> std::path::PathBuf {
        let mut os = prefix.as_os_str().to_owned();
        os.push(".log.bin");
        std::path::PathBuf::from(os)
    }

    #[test]
    fn commit_then_reopen_recovers_root_and_data() {
        let dir = tempdir().unwrap();
        let prefix = dir.path().join("db");
        {
            let mut db = Database::create(&prefix, 256).unwrap();
            db.transaction(|tree, store| {
                for i in 0..20 {
                    tree.insert(store, &key_for(i), &value_for(i))?;
                }
                Ok(())
            })
            .unwrap();
            db.close().unwrap();
        }

        let mut db = Database::open(&prefix).unwrap();
        for i in 0..20 {
            assert_eq!(db.lookup(&key_for(i)).unwrap(), Some(value_for(i)));
        }
        let expected: Vec<_> = (0..20).map(|i| (key_for(i), value_for(i))).collect();
        assert_eq!(db.scan_all().unwrap(), expected);
    }

    #[test]
    fn multiple_commits_reopen_recovers_latest_root_with_all_data() {
        let dir = tempdir().unwrap();
        let prefix = dir.path().join("db");
        {
            let mut db = Database::create(&prefix, 256).unwrap();
            for i in 0..10 {
                db.insert(&key_for(i), &value_for(i)).unwrap();
            }
            db.close().unwrap();
        }

        let mut db = Database::open(&prefix).unwrap();
        let expected: Vec<_> = (0..10).map(|i| (key_for(i), value_for(i))).collect();
        assert_eq!(db.scan_all().unwrap(), expected);
    }

    #[test]
    fn uncommitted_writes_lost_on_recovery() {
        let dir = tempdir().unwrap();
        let prefix = dir.path().join("db");
        {
            let mut db = Database::create(&prefix, 256).unwrap();
            db.insert(b"committed", b"yes").unwrap();

            // Bypass Database::transaction to write without ever committing
            // — simulates a crash mid-transaction.
            let mut superseded = Vec::new();
            let mut store = WalStore {
                pager: &mut db.pager,
                wal: &mut db.wal,
                dpt: &mut db.dpt,
                superseded: &mut superseded,
            };
            let tree = db.tree.as_mut().unwrap();
            tree.insert(&mut store, b"uncommitted", b"no").unwrap();
            // `db` is dropped here without a Commit record for this write
            // and without a checkpoint/close.
        }

        let mut db = Database::open(&prefix).unwrap();
        assert_eq!(db.lookup(b"committed").unwrap(), Some(b"yes".to_vec()));
        assert_eq!(db.lookup(b"uncommitted").unwrap(), None);
    }

    #[test]
    fn torn_tail_is_ignored_by_recovery() {
        let dir = tempdir().unwrap();
        let prefix = dir.path().join("db");
        {
            let mut db = Database::create(&prefix, 256).unwrap();
            db.insert(b"a", b"1").unwrap();
            db.close().unwrap();
        }

        // Append a record header claiming a payload far longer than what
        // actually follows — a torn write mid-record.
        {
            let mut f = OpenOptions::new().write(true).open(log_path(&prefix)).unwrap();
            f.seek(SeekFrom::End(0)).unwrap();
            f.write_all(&100u32.to_le_bytes()).unwrap(); // payload_len = 100
            f.write_all(&[0u8]).unwrap(); // record_type = PageDiff
            f.write_all(&[1, 2, 3, 4, 5]).unwrap(); // only 5 of 100 payload bytes
        }

        let mut db = Database::open(&prefix).unwrap();
        assert_eq!(db.lookup(b"a").unwrap(), Some(b"1".to_vec()));
    }

    #[test]
    fn corrupted_checksum_is_ignored_by_recovery() {
        let dir = tempdir().unwrap();
        let prefix = dir.path().join("db");
        let valid_len;
        {
            let mut db = Database::create(&prefix, 256).unwrap();
            db.insert(b"a", b"1").unwrap();
            db.close().unwrap();
            valid_len = std::fs::metadata(log_path(&prefix)).unwrap().len();
        }

        // Append a well-framed record, then flip a payload byte in place —
        // breaks the checksum without breaking the framing.
        {
            let mut f = OpenOptions::new().read(true).write(true).open(log_path(&prefix)).unwrap();
            f.seek(SeekFrom::End(0)).unwrap();
            let payload = [9u8; 8];
            f.write_all(&(payload.len() as u32).to_le_bytes()).unwrap();
            f.write_all(&[1u8]).unwrap(); // record_type = Commit-shaped, doesn't matter
            f.write_all(&payload).unwrap();
            f.write_all(&0u32.to_le_bytes()).unwrap(); // checksum, deliberately wrong
        }

        let mut db = Database::open(&prefix).unwrap();
        assert_eq!(db.lookup(b"a").unwrap(), Some(b"1".to_vec()));
        assert!(std::fs::metadata(log_path(&prefix)).unwrap().len() > valid_len);
    }

    #[test]
    fn checkpoint_then_more_commits_then_crash_recovers_everything() {
        let dir = tempdir().unwrap();
        let prefix = dir.path().join("db");
        {
            let mut db = Database::create(&prefix, 256).unwrap();
            db.insert(b"a", b"1").unwrap();
            db.checkpoint().unwrap();
            db.insert(b"b", b"2").unwrap();
            // Dropped without an explicit close/checkpoint — simulates a
            // crash right after the second commit's fsync.
        }

        let mut db = Database::open(&prefix).unwrap();
        assert_eq!(db.lookup(b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(db.lookup(b"b").unwrap(), Some(b"2".to_vec()));
    }

    #[test]
    fn recovery_is_idempotent_across_reopens() {
        let dir = tempdir().unwrap();
        let prefix = dir.path().join("db");
        {
            let mut db = Database::create(&prefix, 256).unwrap();
            db.insert(b"a", b"1").unwrap();
            db.close().unwrap();
        }

        let first: Vec<_>;
        {
            let mut db = Database::open(&prefix).unwrap();
            first = db.scan_all().unwrap();
            db.close().unwrap();
        }

        let mut db = Database::open(&prefix).unwrap();
        let second = db.scan_all().unwrap();
        assert_eq!(first, second);
        assert_eq!(first, vec![(b"a".to_vec(), b"1".to_vec())]);
    }

    #[test]
    fn snapshot_taken_before_commit_is_unaffected_by_it() {
        let dir = tempdir().unwrap();
        let mut db = Database::create(&dir.path().join("db"), 256).unwrap();
        db.insert(b"a", b"1").unwrap();

        let snap = db.begin_read();
        db.insert(b"b", b"2").unwrap();
        db.delete(b"a").unwrap();

        assert_eq!(db.lookup_at(&snap, b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(db.lookup_at(&snap, b"b").unwrap(), None);
        assert_eq!(db.scan_all_at(&snap).unwrap(), vec![(b"a".to_vec(), b"1".to_vec())]);

        // The live view, meanwhile, reflects the later writes.
        assert_eq!(db.lookup(b"a").unwrap(), None);
        assert_eq!(db.lookup(b"b").unwrap(), Some(b"2".to_vec()));
        db.end_read(snap);
    }

    #[test]
    fn overlapping_snapshots_each_see_their_own_point_in_time() {
        let dir = tempdir().unwrap();
        let mut db = Database::create(&dir.path().join("db"), 256).unwrap();

        db.insert(&key_for(0), &value_for(0)).unwrap();
        let snap0 = db.begin_read();

        db.insert(&key_for(1), &value_for(1)).unwrap();
        let snap1 = db.begin_read();

        db.insert(&key_for(2), &value_for(2)).unwrap();

        let expected0 = vec![(key_for(0), value_for(0))];
        let expected1 = vec![(key_for(0), value_for(0)), (key_for(1), value_for(1))];

        // Read in reverse order of when the snapshots were taken: a later
        // snapshot is never missing data an earlier one has, and an
        // earlier one never sees a later snapshot's writes.
        assert_eq!(db.scan_all_at(&snap1).unwrap(), expected1);
        assert_eq!(db.scan_all_at(&snap0).unwrap(), expected0);
        assert_eq!(db.scan_all_at(&snap0).unwrap(), expected0);
        assert_eq!(db.scan_all_at(&snap1).unwrap(), expected1);

        db.end_read(snap0);
        db.end_read(snap1);
    }

    #[test]
    fn aborted_transaction_leaves_generation_and_root_unchanged() {
        let dir = tempdir().unwrap();
        let mut db = Database::create(&dir.path().join("db"), 256).unwrap();
        db.insert(b"a", b"1").unwrap();

        let root_before = db.root();
        let generation_before = db.current_generation;
        let pending_before = db.pending_reclaim.len();

        let result = db.transaction(|tree, store| {
            // A real write, followed by a forced failure — simulates a
            // transaction that did some work before aborting.
            tree.insert(store, b"should-not-stick", b"x")?;
            Err(BTreeError::Corrupt("forced abort".into()))
        });
        assert!(result.is_err());

        assert_eq!(db.root(), root_before);
        assert_eq!(db.current_generation, generation_before);
        assert_eq!(db.pending_reclaim.len(), pending_before);
        assert_eq!(db.lookup(b"should-not-stick").unwrap(), None);
        assert_eq!(db.lookup(b"a").unwrap(), Some(b"1".to_vec()));
    }

    #[test]
    fn checkpoint_reclaims_superseded_pages_when_no_snapshot_is_open() {
        let dir = tempdir().unwrap();
        let mut db = Database::create(&dir.path().join("db"), 256).unwrap();
        db.insert(b"a", b"1").unwrap();

        // Upserting the same key supersedes the leaf page holding it.
        db.insert(b"a", b"2").unwrap();
        assert!(!db.pending_reclaim.is_empty());

        db.checkpoint().unwrap();
        assert!(db.pending_reclaim.is_empty());
        let page_count_after_reclaim = db.pager.page_count();

        // The freed page gets reused rather than growing the file further.
        db.insert(b"c", b"3").unwrap();
        assert_eq!(db.pager.page_count(), page_count_after_reclaim);
    }

    #[test]
    fn checkpoint_does_not_reclaim_pages_a_live_snapshot_still_needs() {
        let dir = tempdir().unwrap();
        let mut db = Database::create(&dir.path().join("db"), 256).unwrap();
        db.insert(b"a", b"1").unwrap();

        let snap = db.begin_read();
        db.insert(b"a", b"2").unwrap();
        db.checkpoint().unwrap();

        // The snapshot's data must still be intact after the checkpoint.
        assert_eq!(db.lookup_at(&snap, b"a").unwrap(), Some(b"1".to_vec()));
        assert!(!db.pending_reclaim.is_empty());

        db.insert(b"b", b"3").unwrap();
        db.checkpoint().unwrap();
        assert_eq!(db.lookup_at(&snap, b"a").unwrap(), Some(b"1".to_vec()));
        assert!(!db.pending_reclaim.is_empty());

        db.end_read(snap);
        db.checkpoint().unwrap();
        assert!(db.pending_reclaim.is_empty());
    }

    #[test]
    fn reclamation_respects_the_older_of_two_overlapping_snapshots() {
        let dir = tempdir().unwrap();
        let mut db = Database::create(&dir.path().join("db"), 256).unwrap();
        db.insert(b"a", b"1").unwrap();

        let older = db.begin_read();
        db.insert(b"a", b"2").unwrap();
        let newer = db.begin_read();
        db.insert(b"a", b"3").unwrap();

        db.checkpoint().unwrap();
        // Nothing superseded before `older`'s generation exists here, but
        // pages superseded after it must survive while it's still open.
        assert_eq!(db.lookup_at(&older, b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(db.lookup_at(&newer, b"a").unwrap(), Some(b"2".to_vec()));

        db.end_read(newer);
        db.checkpoint().unwrap();
        // `older` is still open, so its generation's data must survive.
        assert_eq!(db.lookup_at(&older, b"a").unwrap(), Some(b"1".to_vec()));

        db.end_read(older);
        db.checkpoint().unwrap();
        assert!(db.pending_reclaim.is_empty());
    }
}
