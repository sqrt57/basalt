//! Fixed-size page I/O and free-list allocation over `<prefix>.data.bin`.
//! Format decided in `design/decisions/0015-page-storage-format.md`.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub type PageId = u64;

/// Size of the fixed, page-size-independent read used to learn `page_size`
/// itself before any page-aligned offset can be computed. Not the size of
/// a page — the preamble *content* fits in this many bytes, but it's
/// zero-padded out to fill all of page 1
/// (`design/decisions/0015-page-storage-format.md`).
const PREAMBLE_READ_SIZE: u64 = 64;
const MAGIC: &[u8; 8] = b"BSLTPAGE";
const FORMAT_VERSION: u32 = 1;
/// The preamble's 64 content bytes must fit within page 1 itself, so page
/// size can't go below that floor (see
/// `design/decisions/0015-page-storage-format.md`'s revision note).
const MIN_PAGE_SIZE: u32 = PREAMBLE_READ_SIZE as u32;

// Every page in the file, page 1 included, starts with the same common
// page header: `page_type: u8` (offset 0) + reserved byte (offset 1) +
// `page_lsn: u64` (offset 2..10). Chunk 1 only ever writes zeros into
// `page_lsn` and never reads it back — it's chunk 3's field — so there's
// no named offset constant for it here, just the reserved space that
// falls out of MAGIC_OFFSET / TRUNK_NEXT_OFFSET both starting at 10.
const PAGE_TYPE_FILE_HEADER: u8 = 2;
const PAGE_TYPE_FREE_TRUNK: u8 = 3;

// Page 1 (file header) layout, offsets within the page.
const MAGIC_OFFSET: usize = 10;
const VERSION_OFFSET: usize = 18;
const PAGE_SIZE_OFFSET: usize = 22;
const FREE_TRUNK_HEAD_OFFSET: usize = 26;

// Free-list trunk page layout, offsets within the page.
const TRUNK_NEXT_OFFSET: usize = 10;
const TRUNK_NUM_LEAVES_OFFSET: usize = 18;
const TRUNK_HEADER_SIZE: usize = 20;
const TRUNK_LEAF_SIZE: usize = 8;

#[derive(Debug)]
pub enum PagerError {
    Io(io::Error),
    BadMagic,
    UnsupportedVersion(u32),
    InvalidPageSize(u32),
    PageOutOfRange(PageId),
    BufferLengthMismatch { expected: usize, actual: usize },
}

impl From<io::Error> for PagerError {
    fn from(e: io::Error) -> Self {
        PagerError::Io(e)
    }
}

impl std::fmt::Display for PagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PagerError::Io(e) => write!(f, "I/O error: {e}"),
            PagerError::BadMagic => write!(f, "not a Basalt data file (bad magic bytes)"),
            PagerError::UnsupportedVersion(v) => {
                write!(f, "unsupported data file format version {v}")
            }
            PagerError::InvalidPageSize(s) => write!(
                f,
                "invalid page size {s} (must be a power of two, >= {MIN_PAGE_SIZE})"
            ),
            PagerError::PageOutOfRange(p) => write!(f, "page {p} is out of the allocated range"),
            PagerError::BufferLengthMismatch { expected, actual } => write!(
                f,
                "page buffer length {actual} does not match page size {expected}"
            ),
        }
    }
}

impl std::error::Error for PagerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PagerError::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// Fixed-size page I/O and free-list allocation over a single data file.
///
/// No tree structure, no MVCC, no WAL/crash-safety, no concurrent access —
/// stage-1 chunk 1 scope only (`design/stage1-plan.md`).
pub struct Pager {
    file: File,
    page_size: u32,
    /// Highest currently-valid page id. Page numbering is 1-based
    /// (`design/decisions/0015-page-storage-format.md`), so this is also
    /// the total physical page count, not a one-past-end index.
    page_count: u64,
    /// Page id of the first free-list trunk page, or `0` if there are no
    /// free pages at all — `0` is a standing null, never a physical page.
    free_trunk_head: PageId,
}

impl Pager {
    /// Creates a new database at `path_prefix`, i.e. `<path_prefix>.data.bin`.
    /// Fails if that file already exists.
    pub fn create(path_prefix: &Path, page_size: u32) -> Result<Self, PagerError> {
        if !is_valid_page_size(page_size) {
            return Err(PagerError::InvalidPageSize(page_size));
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(data_file_path(path_prefix))?;

        // Page 1: the file header — common page header, magic/version/page
        // size, and free-list trunk head, zero-padded out to a full page so
        // every page after it is aligned to a page_size multiple from file
        // offset 0. Page 1 itself sits at offset 0.
        let mut page1 = vec![0u8; page_size as usize];
        page1[0] = PAGE_TYPE_FILE_HEADER;
        page1[MAGIC_OFFSET..MAGIC_OFFSET + 8].copy_from_slice(MAGIC);
        page1[VERSION_OFFSET..VERSION_OFFSET + 4].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        page1[PAGE_SIZE_OFFSET..PAGE_SIZE_OFFSET + 4].copy_from_slice(&page_size.to_le_bytes());
        page1[FREE_TRUNK_HEAD_OFFSET..FREE_TRUNK_HEAD_OFFSET + 8].copy_from_slice(&0u64.to_le_bytes());
        file.write_all(&page1)?;

        Ok(Pager {
            file,
            page_size,
            page_count: 1,
            free_trunk_head: 0,
        })
    }

    /// Opens an existing database at `path_prefix`. Page size and free-list
    /// state are read back from the file, never supplied by the caller —
    /// both come from the same single fixed-size preamble read.
    pub fn open(path_prefix: &Path) -> Result<Self, PagerError> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(data_file_path(path_prefix))?;

        let mut preamble = [0u8; PREAMBLE_READ_SIZE as usize];
        file.read_exact(&mut preamble)?;
        if preamble[MAGIC_OFFSET..MAGIC_OFFSET + 8] != *MAGIC {
            return Err(PagerError::BadMagic);
        }
        let version = u32::from_le_bytes(preamble[VERSION_OFFSET..VERSION_OFFSET + 4].try_into().unwrap());
        if version != FORMAT_VERSION {
            return Err(PagerError::UnsupportedVersion(version));
        }
        let page_size = u32::from_le_bytes(preamble[PAGE_SIZE_OFFSET..PAGE_SIZE_OFFSET + 4].try_into().unwrap());
        if !is_valid_page_size(page_size) {
            return Err(PagerError::InvalidPageSize(page_size));
        }
        let free_trunk_head = u64::from_le_bytes(
            preamble[FREE_TRUNK_HEAD_OFFSET..FREE_TRUNK_HEAD_OFFSET + 8].try_into().unwrap(),
        );

        let file_len = file.metadata()?.len();
        let page_count = file_len / page_size as u64;

        Ok(Pager {
            file,
            page_size,
            page_count,
            free_trunk_head,
        })
    }

    pub fn page_size(&self) -> u32 {
        self.page_size
    }

    pub fn page_count(&self) -> u64 {
        self.page_count
    }

    /// Ensures the file is large enough to hold `page_id`, extending it
    /// (without touching the free-list) if not. Used only by WAL redo
    /// (`design/decisions/0017-wal-format.md`): a crash may lose the
    /// file-length extension an `allocate_page` call made, so redo
    /// re-extends as needed before writing a page back into place, rather
    /// than assuming `page_count` already covers it.
    pub fn extend_to(&mut self, page_id: PageId) -> Result<(), PagerError> {
        if page_id == 0 {
            return Err(PagerError::PageOutOfRange(0));
        }
        if page_id <= self.page_count {
            return Ok(());
        }
        let new_len = page_id * self.page_size as u64;
        self.file.set_len(new_len)?;
        self.page_count = page_id;
        Ok(())
    }

    /// Flushes the data file to durable storage. Used by WAL checkpointing
    /// (`design/decisions/0017-wal-format.md`) — the pager itself writes
    /// through to the OS on every `write_page` but never fsyncs on its own.
    pub fn sync(&mut self) -> Result<(), PagerError> {
        self.file.sync_all()?;
        Ok(())
    }

    /// Reuses a free-listed page if one exists, otherwise extends the file
    /// by one page. Newly allocated (never-before-written) page content is
    /// unspecified until the caller writes it.
    ///
    /// Free-list structure is trunk pages + contentless leaves
    /// (`design/decisions/0015-page-storage-format.md`): if the head trunk
    /// has spare leaf ids, pop and return the last one; if the head trunk is
    /// itself exhausted, advance the head to its `next_trunk` and hand out
    /// the exhausted trunk's own page id.
    pub fn allocate_page(&mut self) -> Result<PageId, PagerError> {
        if self.free_trunk_head == 0 {
            let page_id = self.page_count + 1;
            let new_len = page_id * self.page_size as u64;
            self.file.set_len(new_len)?;
            self.page_count = page_id;
            return Ok(page_id);
        }

        let trunk_id = self.free_trunk_head;
        let mut buf = vec![0u8; self.page_size as usize];
        self.read_page(trunk_id, &mut buf)?;
        let next_trunk = u64::from_le_bytes(buf[TRUNK_NEXT_OFFSET..TRUNK_NEXT_OFFSET + 8].try_into().unwrap());
        let num_leaves =
            u16::from_le_bytes(buf[TRUNK_NUM_LEAVES_OFFSET..TRUNK_NUM_LEAVES_OFFSET + 2].try_into().unwrap());

        if num_leaves > 0 {
            let leaf_off = TRUNK_HEADER_SIZE + (num_leaves as usize - 1) * TRUNK_LEAF_SIZE;
            let page_id = u64::from_le_bytes(buf[leaf_off..leaf_off + 8].try_into().unwrap());
            let new_num_leaves = num_leaves - 1;
            buf[TRUNK_NUM_LEAVES_OFFSET..TRUNK_NUM_LEAVES_OFFSET + 2]
                .copy_from_slice(&new_num_leaves.to_le_bytes());
            self.write_page(trunk_id, &buf)?;
            Ok(page_id)
        } else {
            self.set_free_trunk_head(next_trunk)?;
            Ok(trunk_id)
        }
    }

    /// Pushes `page_id` onto the free-list. Does not detect double-frees or
    /// freeing a page still referenced elsewhere — that's the caller's
    /// responsibility, same as any free-list allocator.
    ///
    /// If the head trunk has spare capacity, `page_id` is appended to its
    /// leaf list. Otherwise (no trunk yet, or the head trunk is full),
    /// `page_id` itself becomes the new head trunk.
    pub fn free_page(&mut self, page_id: PageId) -> Result<(), PagerError> {
        self.check_range(page_id)?;
        let capacity = self.trunk_capacity();

        if self.free_trunk_head != 0 {
            let head = self.free_trunk_head;
            let mut buf = vec![0u8; self.page_size as usize];
            self.read_page(head, &mut buf)?;
            let num_leaves =
                u16::from_le_bytes(buf[TRUNK_NUM_LEAVES_OFFSET..TRUNK_NUM_LEAVES_OFFSET + 2].try_into().unwrap())
                    as usize;
            if num_leaves < capacity {
                let leaf_off = TRUNK_HEADER_SIZE + num_leaves * TRUNK_LEAF_SIZE;
                buf[leaf_off..leaf_off + 8].copy_from_slice(&page_id.to_le_bytes());
                buf[TRUNK_NUM_LEAVES_OFFSET..TRUNK_NUM_LEAVES_OFFSET + 2]
                    .copy_from_slice(&((num_leaves + 1) as u16).to_le_bytes());
                self.write_page(head, &buf)?;
                return Ok(());
            }
        }

        let mut buf = vec![0u8; self.page_size as usize];
        buf[0] = PAGE_TYPE_FREE_TRUNK;
        buf[TRUNK_NEXT_OFFSET..TRUNK_NEXT_OFFSET + 8].copy_from_slice(&self.free_trunk_head.to_le_bytes());
        buf[TRUNK_NUM_LEAVES_OFFSET..TRUNK_NUM_LEAVES_OFFSET + 2].copy_from_slice(&0u16.to_le_bytes());
        self.write_page(page_id, &buf)?;
        self.set_free_trunk_head(page_id)?;
        Ok(())
    }

    /// Reads exactly one page into `buf`, which must be `page_size()` bytes.
    /// A page currently listed as a free-list leaf reads back as defined-
    /// but-unspecified content, not an error; a trunk page's content is
    /// well-defined (see `free_page`/`allocate_page`).
    pub fn read_page(&mut self, page_id: PageId, buf: &mut [u8]) -> Result<(), PagerError> {
        self.check_range(page_id)?;
        self.check_buffer_len(buf.len())?;
        self.seek_to(page_id)?;
        self.file.read_exact(buf)?;
        Ok(())
    }

    /// Writes exactly one page from `buf`, which must be `page_size()` bytes.
    pub fn write_page(&mut self, page_id: PageId, buf: &[u8]) -> Result<(), PagerError> {
        self.check_range(page_id)?;
        self.check_buffer_len(buf.len())?;
        self.seek_to(page_id)?;
        self.file.write_all(buf)?;
        Ok(())
    }

    fn trunk_capacity(&self) -> usize {
        (self.page_size as usize - TRUNK_HEADER_SIZE) / TRUNK_LEAF_SIZE
    }

    /// Updates the free-list trunk head, both in memory and in page 1's
    /// preamble (a partial write of just that field, not a full-page
    /// rewrite — page 1's own `page_lsn` stays untouched, per
    /// `design/decisions/0015-page-storage-format.md`).
    fn set_free_trunk_head(&mut self, head: PageId) -> Result<(), PagerError> {
        self.free_trunk_head = head;
        self.file.seek(SeekFrom::Start(FREE_TRUNK_HEAD_OFFSET as u64))?;
        self.file.write_all(&head.to_le_bytes())?;
        Ok(())
    }

    fn seek_to(&mut self, page_id: PageId) -> Result<(), PagerError> {
        if page_id == 0 {
            return Err(PagerError::PageOutOfRange(0));
        }
        let offset = (page_id - 1) * self.page_size as u64;
        self.file.seek(SeekFrom::Start(offset))?;
        Ok(())
    }

    fn check_range(&self, page_id: PageId) -> Result<(), PagerError> {
        if page_id == 0 || page_id > self.page_count {
            return Err(PagerError::PageOutOfRange(page_id));
        }
        Ok(())
    }

    fn check_buffer_len(&self, len: usize) -> Result<(), PagerError> {
        if len != self.page_size as usize {
            return Err(PagerError::BufferLengthMismatch {
                expected: self.page_size as usize,
                actual: len,
            });
        }
        Ok(())
    }
}

fn is_valid_page_size(page_size: u32) -> bool {
    page_size >= MIN_PAGE_SIZE && page_size.is_power_of_two()
}

fn data_file_path(path_prefix: &Path) -> PathBuf {
    let mut os = path_prefix.as_os_str().to_owned();
    os.push(".data.bin");
    PathBuf::from(os)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn prefix(dir: &Path, name: &str) -> PathBuf {
        dir.join(name)
    }

    #[test]
    fn create_then_open_round_trips_page_size() {
        let dir = tempdir().unwrap();
        let p = prefix(dir.path(), "db");
        {
            let pager = Pager::create(&p, 256).unwrap();
            assert_eq!(pager.page_size(), 256);
            assert_eq!(pager.page_count(), 1); // page 1: file header, the only reserved page
        }
        let pager = Pager::open(&p).unwrap();
        assert_eq!(pager.page_size(), 256);
        assert_eq!(pager.page_count(), 1);
    }

    #[test]
    fn create_fails_if_file_already_exists() {
        let dir = tempdir().unwrap();
        let p = prefix(dir.path(), "db");
        Pager::create(&p, 64).unwrap();
        assert!(Pager::create(&p, 64).is_err());
    }

    #[test]
    fn open_rejects_bad_magic() {
        let dir = tempdir().unwrap();
        let p = prefix(dir.path(), "db");
        std::fs::write(data_file_path(&p), vec![0u8; 128]).unwrap();
        assert!(matches!(Pager::open(&p), Err(PagerError::BadMagic)));
    }

    #[test]
    fn create_rejects_invalid_page_size() {
        let dir = tempdir().unwrap();
        assert!(matches!(
            Pager::create(&prefix(dir.path(), "zero"), 0),
            Err(PagerError::InvalidPageSize(0))
        ));
        assert!(matches!(
            Pager::create(&prefix(dir.path(), "small"), 4),
            Err(PagerError::InvalidPageSize(4))
        ));
        assert!(matches!(
            Pager::create(&prefix(dir.path(), "npot"), 100),
            Err(PagerError::InvalidPageSize(100))
        ));
    }

    #[test]
    fn write_then_read_round_trips_bytes() {
        let dir = tempdir().unwrap();
        let p = prefix(dir.path(), "db");
        let mut pager = Pager::create(&p, 64).unwrap();

        let id = pager.allocate_page().unwrap();
        let written: Vec<u8> = (0..64).collect();
        pager.write_page(id, &written).unwrap();

        let mut read_back = vec![0u8; 64];
        pager.read_page(id, &mut read_back).unwrap();
        assert_eq!(read_back, written);
    }

    #[test]
    fn write_then_read_round_trips_across_reopen() {
        let dir = tempdir().unwrap();
        let p = prefix(dir.path(), "db");
        let written: Vec<u8> = (0..64).map(|b| b ^ 0xAA).collect();
        let id;
        {
            let mut pager = Pager::create(&p, 64).unwrap();
            id = pager.allocate_page().unwrap();
            pager.write_page(id, &written).unwrap();
        }
        let mut pager = Pager::open(&p).unwrap();
        let mut read_back = vec![0u8; 64];
        pager.read_page(id, &mut read_back).unwrap();
        assert_eq!(read_back, written);
    }

    #[test]
    fn allocate_extends_file_when_free_list_empty() {
        let dir = tempdir().unwrap();
        let mut pager = Pager::create(&prefix(dir.path(), "db"), 64).unwrap();
        let a = pager.allocate_page().unwrap();
        let b = pager.allocate_page().unwrap();
        assert_ne!(a, b);
        assert_eq!(pager.page_count(), 3); // page 1 (file header) + a + b
    }

    #[test]
    fn free_then_allocate_reuses_page_without_growing_file() {
        let dir = tempdir().unwrap();
        let mut pager = Pager::create(&prefix(dir.path(), "db"), 64).unwrap();

        let mut allocated = Vec::new();
        for _ in 0..5 {
            allocated.push(pager.allocate_page().unwrap());
        }
        let count_after_alloc = pager.page_count();

        for &id in &allocated {
            pager.free_page(id).unwrap();
        }

        let mut reallocated = Vec::new();
        for _ in 0..5 {
            reallocated.push(pager.allocate_page().unwrap());
        }

        assert_eq!(pager.page_count(), count_after_alloc);
        let mut allocated_sorted = allocated.clone();
        let mut reallocated_sorted = reallocated.clone();
        allocated_sorted.sort();
        reallocated_sorted.sort();
        assert_eq!(allocated_sorted, reallocated_sorted);
    }

    #[test]
    fn freeing_more_pages_than_one_trunk_holds_chains_trunks() {
        // At page_size 64, trunk capacity is (64 - 20) / 8 = 5, so freeing
        // more than that forces a second trunk to be created and chained.
        let dir = tempdir().unwrap();
        let mut pager = Pager::create(&prefix(dir.path(), "db"), 64).unwrap();

        let mut allocated = Vec::new();
        for _ in 0..12 {
            allocated.push(pager.allocate_page().unwrap());
        }
        for &id in &allocated {
            pager.free_page(id).unwrap();
        }

        let mut reallocated = Vec::new();
        for _ in 0..12 {
            reallocated.push(pager.allocate_page().unwrap());
        }

        // Every freed page was handed back out, including trunk pages
        // themselves once exhausted and recycled — no growth needed.
        let mut allocated_sorted = allocated.clone();
        let mut reallocated_sorted = reallocated.clone();
        allocated_sorted.sort();
        reallocated_sorted.sort();
        assert_eq!(allocated_sorted, reallocated_sorted);

        // The free-list is now fully drained.
        assert_eq!(pager.free_trunk_head, 0);
    }

    #[test]
    fn read_write_out_of_range_is_an_error() {
        let dir = tempdir().unwrap();
        let mut pager = Pager::create(&prefix(dir.path(), "db"), 64).unwrap();
        let buf = vec![0u8; 64];
        assert!(matches!(
            pager.read_page(99, &mut buf.clone()),
            Err(PagerError::PageOutOfRange(99))
        ));
        assert!(matches!(
            pager.write_page(99, &buf),
            Err(PagerError::PageOutOfRange(99))
        ));
    }

    #[test]
    fn page_id_zero_is_always_out_of_range() {
        let dir = tempdir().unwrap();
        let mut pager = Pager::create(&prefix(dir.path(), "db"), 64).unwrap();
        let buf = vec![0u8; 64];
        assert!(matches!(
            pager.read_page(0, &mut buf.clone()),
            Err(PagerError::PageOutOfRange(0))
        ));
        assert!(matches!(pager.write_page(0, &buf), Err(PagerError::PageOutOfRange(0))));
        assert!(matches!(pager.free_page(0), Err(PagerError::PageOutOfRange(0))));
        assert!(matches!(pager.extend_to(0), Err(PagerError::PageOutOfRange(0))));
    }

    #[test]
    fn wrong_buffer_length_is_an_error() {
        let dir = tempdir().unwrap();
        let mut pager = Pager::create(&prefix(dir.path(), "db"), 64).unwrap();
        let id = pager.allocate_page().unwrap();
        let short = vec![0u8; 10];
        assert!(matches!(
            pager.write_page(id, &short),
            Err(PagerError::BufferLengthMismatch {
                expected: 64,
                actual: 10
            })
        ));
    }

    #[test]
    fn reading_a_free_listed_page_does_not_error() {
        let dir = tempdir().unwrap();
        let mut pager = Pager::create(&prefix(dir.path(), "db"), 64).unwrap();
        let id = pager.allocate_page().unwrap();
        pager.free_page(id).unwrap();

        let mut buf = vec![0u8; 64];
        // Defined-but-unspecified content: must not error, value not asserted.
        assert!(pager.read_page(id, &mut buf).is_ok());
    }

    #[test]
    fn pages_are_1_based_and_aligned_to_page_size_multiples_from_file_start() {
        let dir = tempdir().unwrap();
        let p = prefix(dir.path(), "db");
        let page_size: u32 = 128;
        let written: Vec<u8> = (0..page_size as u8).collect();
        let id;
        {
            let mut pager = Pager::create(&p, page_size).unwrap();
            id = pager.allocate_page().unwrap();
            assert_eq!(id, 2); // page 1 is reserved; first allocatable page is 2
            pager.write_page(id, &written).unwrap();
        }

        // Read the raw file directly at `(id - 1) * page_size` and confirm
        // it's exactly the page written.
        let mut file = std::fs::File::open(data_file_path(&p)).unwrap();
        file.seek(SeekFrom::Start((id - 1) * page_size as u64)).unwrap();
        let mut raw = vec![0u8; page_size as usize];
        file.read_exact(&mut raw).unwrap();
        assert_eq!(raw, written);

        // Page 1 sits at file offset 0; its magic bytes sit at offset 10
        // (after the common page header), not at absolute file offset 0.
        let mut file = std::fs::File::open(data_file_path(&p)).unwrap();
        file.seek(SeekFrom::Start(MAGIC_OFFSET as u64)).unwrap();
        let mut magic = [0u8; 8];
        file.read_exact(&mut magic).unwrap();
        assert_eq!(&magic, MAGIC);

        // File length itself is an exact multiple of page_size.
        assert_eq!(std::fs::metadata(data_file_path(&p)).unwrap().len() % page_size as u64, 0);
    }
}
