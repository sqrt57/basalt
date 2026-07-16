//! Ties chunk 1 ([`Pager`]), chunk 2 ([`BTree`]), and chunk 3 ([`Wal`])
//! together into a durable, recoverable database handle.
//!
//! Not stage 1's chunk 8 embedded API (builder/options, transaction
//! handles) — a minimal handle sufficient to exercise and test chunk 3's
//! WAL wiring (`design/decisions/0017-wal-format.md`).

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
/// tracking each newly-dirtied page's recLSN for the next checkpoint.
pub struct WalStore<'a> {
    pager: &'a mut Pager,
    wal: &'a mut Wal,
    dpt: &'a mut BTreeMap<PageId, Lsn>,
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

        Ok(new_id)
    }
}

/// An open database: `<prefix>.data.bin` + `<prefix>.log.bin` together,
/// with every committed write recoverable via WAL redo.
pub struct Database {
    pager: Pager,
    wal: Wal,
    dpt: BTreeMap<PageId, Lsn>,
    tree: Option<BTree>,
}

impl Database {
    /// Creates a new, empty database at `path_prefix`.
    pub fn create(path_prefix: &Path, page_size: u32) -> Result<Self, DatabaseError> {
        let pager = Pager::create(path_prefix, page_size)?;
        let wal = Wal::create(path_prefix)?;
        Ok(Database { pager, wal, dpt: BTreeMap::new(), tree: None })
    }

    /// Opens an existing database, replaying the WAL to recover whichever
    /// root the last committed transaction left current (`None` if no
    /// transaction ever committed).
    pub fn open(path_prefix: &Path) -> Result<Self, DatabaseError> {
        let mut pager = Pager::open(path_prefix)?;
        let mut wal = Wal::open(path_prefix)?;
        let root = wal.recover(&mut pager)?;
        let tree = root.map(BTree::from_root);
        Ok(Database { pager, wal, dpt: BTreeMap::new(), tree })
    }

    /// Runs `f` against the tree and a logging store, then commits: logs
    /// and fsyncs a `Commit` record for the resulting root. Writes made by
    /// `f` are only durable once this returns `Ok`.
    pub fn transaction<F>(&mut self, f: F) -> Result<(), DatabaseError>
    where
        F: FnOnce(&mut BTree, &mut WalStore) -> Result<(), BTreeError>,
    {
        if self.tree.is_none() {
            let mut store = WalStore { pager: &mut self.pager, wal: &mut self.wal, dpt: &mut self.dpt };
            self.tree = Some(BTree::create_empty(&mut store)?);
        }
        let tree = self.tree.as_mut().expect("just ensured Some");

        let mut store = WalStore { pager: &mut self.pager, wal: &mut self.wal, dpt: &mut self.dpt };
        f(tree, &mut store)?;

        self.wal.append_commit(tree.root())?;
        Ok(())
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
    /// then drops the now-durable entries. Triggered only explicitly or on
    /// [`Database::close`] — no background trigger yet (see
    /// `design/backlog.md`).
    pub fn checkpoint(&mut self) -> Result<(), DatabaseError> {
        let snapshot: Vec<(PageId, Lsn)> = self.dpt.iter().map(|(&p, &l)| (p, l)).collect();
        self.wal.checkpoint(&mut self.pager, &snapshot)?;
        self.dpt.clear();
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
            let mut store = WalStore { pager: &mut db.pager, wal: &mut db.wal, dpt: &mut db.dpt };
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
}
