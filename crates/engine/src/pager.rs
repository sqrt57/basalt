//! Fixed-size page I/O and free-list allocation over `<prefix>.data.bin`.
//! Format decided in `design/decisions/0015-page-storage-format.md`.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub type PageId = u64;

const PREAMBLE_SIZE: u64 = 64;
const MAGIC: &[u8; 8] = b"BSLTPAGE";
const FORMAT_VERSION: u32 = 1;
const FREE_LIST_NONE: u64 = u64::MAX;
/// A free page's next-pointer is a `u64`, so a page must be at least this big.
const MIN_PAGE_SIZE: u32 = 8;

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
    page_count: u64,
    free_list_head: u64,
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

        let mut preamble = [0u8; PREAMBLE_SIZE as usize];
        preamble[0..8].copy_from_slice(MAGIC);
        preamble[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        preamble[12..16].copy_from_slice(&page_size.to_le_bytes());
        file.write_all(&preamble)?;

        // Page 0: allocator state. First 8 bytes are the free-list head
        // (empty), the rest of the page is reserved/unused for now.
        let mut page0 = vec![0u8; page_size as usize];
        page0[0..8].copy_from_slice(&FREE_LIST_NONE.to_le_bytes());
        file.write_all(&page0)?;

        Ok(Pager {
            file,
            page_size,
            page_count: 1,
            free_list_head: FREE_LIST_NONE,
        })
    }

    /// Opens an existing database at `path_prefix`. Page size and free-list
    /// state are read back from the file, never supplied by the caller.
    pub fn open(path_prefix: &Path) -> Result<Self, PagerError> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(data_file_path(path_prefix))?;

        let mut preamble = [0u8; PREAMBLE_SIZE as usize];
        file.read_exact(&mut preamble)?;
        if &preamble[0..8] != MAGIC {
            return Err(PagerError::BadMagic);
        }
        let version = u32::from_le_bytes(preamble[8..12].try_into().unwrap());
        if version != FORMAT_VERSION {
            return Err(PagerError::UnsupportedVersion(version));
        }
        let page_size = u32::from_le_bytes(preamble[12..16].try_into().unwrap());
        if !is_valid_page_size(page_size) {
            return Err(PagerError::InvalidPageSize(page_size));
        }

        let file_len = file.metadata()?.len();
        let page_count = (file_len - PREAMBLE_SIZE) / page_size as u64;

        let mut pager = Pager {
            file,
            page_size,
            page_count,
            free_list_head: FREE_LIST_NONE,
        };
        pager.free_list_head = pager.read_free_list_head()?;
        Ok(pager)
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
        if page_id < self.page_count {
            return Ok(());
        }
        let new_count = page_id + 1;
        let new_len = PREAMBLE_SIZE + new_count * self.page_size as u64;
        self.file.set_len(new_len)?;
        self.page_count = new_count;
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
    pub fn allocate_page(&mut self) -> Result<PageId, PagerError> {
        if self.free_list_head != FREE_LIST_NONE {
            let page_id = self.free_list_head;
            let next = self.read_next_pointer(page_id)?;
            self.set_free_list_head(next)?;
            return Ok(page_id);
        }

        let page_id = self.page_count;
        let new_len = PREAMBLE_SIZE + (page_id + 1) * self.page_size as u64;
        self.file.set_len(new_len)?;
        self.page_count += 1;
        Ok(page_id)
    }

    /// Pushes `page_id` onto the free-list. Does not detect double-frees or
    /// freeing a page still referenced elsewhere — that's the caller's
    /// responsibility, same as any free-list allocator.
    pub fn free_page(&mut self, page_id: PageId) -> Result<(), PagerError> {
        self.check_range(page_id)?;
        let next = self.free_list_head;
        self.seek_to(page_id)?;
        self.file.write_all(&next.to_le_bytes())?;
        self.set_free_list_head(page_id)?;
        Ok(())
    }

    /// Reads exactly one page into `buf`, which must be `page_size()` bytes.
    /// A page currently on the free-list reads back as defined-but-
    /// unspecified content, not an error.
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

    fn read_free_list_head(&mut self) -> Result<u64, PagerError> {
        self.read_next_pointer(0)
    }

    fn set_free_list_head(&mut self, head: u64) -> Result<(), PagerError> {
        self.free_list_head = head;
        self.seek_to(0)?;
        self.file.write_all(&head.to_le_bytes())?;
        Ok(())
    }

    /// Reads the first 8 bytes of `page_id` as a `u64` free-list pointer.
    /// Used both for page 0's free-list head and for a free page's next
    /// pointer, which share the same "first 8 bytes" convention.
    fn read_next_pointer(&mut self, page_id: PageId) -> Result<u64, PagerError> {
        self.seek_to(page_id)?;
        let mut buf = [0u8; 8];
        self.file.read_exact(&mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    fn seek_to(&mut self, page_id: PageId) -> Result<(), PagerError> {
        let offset = PREAMBLE_SIZE + page_id * self.page_size as u64;
        self.file.seek(SeekFrom::Start(offset))?;
        Ok(())
    }

    fn check_range(&self, page_id: PageId) -> Result<(), PagerError> {
        if page_id >= self.page_count {
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
            assert_eq!(pager.page_count(), 1);
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
        assert_eq!(pager.page_count(), 3); // page 0 (allocator) + a + b
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
}
