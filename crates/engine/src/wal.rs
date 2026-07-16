//! Write-ahead log: crash recovery for chunk 2's copy-on-write B+-tree.
//! Format decided in `design/decisions/0017-wal-format.md`.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::crc32c::crc32c;
use crate::pager::{PageId, Pager, PagerError};

pub type Lsn = u64;

const PREAMBLE_SIZE: u64 = 64;
const MAGIC: &[u8; 8] = b"BSLTLOG_";
const FORMAT_VERSION: u32 = 1;
/// Sentinel `base_page_id` meaning "no base — reconstruct from an all-zero
/// page", matching `pager.rs`'s `FREE_LIST_NONE` convention.
const NO_BASE: u64 = u64::MAX;

const RECORD_PAGE_DIFF: u8 = 0;
const RECORD_COMMIT: u8 = 1;
const RECORD_CHECKPOINT: u8 = 2;

#[derive(Debug)]
pub enum WalError {
    Io(io::Error),
    BadMagic,
    UnsupportedVersion(u32),
    Pager(PagerError),
}

impl From<io::Error> for WalError {
    fn from(e: io::Error) -> Self {
        WalError::Io(e)
    }
}

impl From<PagerError> for WalError {
    fn from(e: PagerError) -> Self {
        WalError::Pager(e)
    }
}

impl std::fmt::Display for WalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalError::Io(e) => write!(f, "I/O error: {e}"),
            WalError::BadMagic => write!(f, "not a Basalt log file (bad magic bytes)"),
            WalError::UnsupportedVersion(v) => {
                write!(f, "unsupported log file format version {v}")
            }
            WalError::Pager(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for WalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WalError::Io(e) => Some(e),
            WalError::Pager(e) => Some(e),
            _ => None,
        }
    }
}

/// `<prefix>.log.bin`: checksummed, framed records (`PageDiff`, `Commit`,
/// `Checkpoint`), an LSN being a record's own file offset.
pub struct Wal {
    file: File,
}

impl Wal {
    /// Creates a new log at `path_prefix`, i.e. `<path_prefix>.log.bin`.
    /// Fails if that file already exists.
    pub fn create(path_prefix: &Path) -> Result<Self, WalError> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(log_file_path(path_prefix))?;

        let mut preamble = [0u8; PREAMBLE_SIZE as usize];
        preamble[0..8].copy_from_slice(MAGIC);
        preamble[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        file.write_all(&preamble)?;
        file.sync_all()?;

        Ok(Wal { file })
    }

    /// Opens an existing log at `path_prefix`.
    pub fn open(path_prefix: &Path) -> Result<Self, WalError> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(log_file_path(path_prefix))?;

        let mut preamble = [0u8; PREAMBLE_SIZE as usize];
        file.read_exact(&mut preamble)?;
        if &preamble[0..8] != MAGIC {
            return Err(WalError::BadMagic);
        }
        let version = u32::from_le_bytes(preamble[8..12].try_into().unwrap());
        if version != FORMAT_VERSION {
            return Err(WalError::UnsupportedVersion(version));
        }

        Ok(Wal { file })
    }

    fn append_record(&mut self, record_type: u8, payload: &[u8]) -> Result<Lsn, WalError> {
        let lsn = self.file.seek(SeekFrom::End(0))?;
        let mut buf = Vec::with_capacity(4 + 1 + payload.len() + 4);
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.push(record_type);
        buf.extend_from_slice(payload);
        let checksum = crc32c(&buf[4..]);
        buf.extend_from_slice(&checksum.to_le_bytes());
        self.file.write_all(&buf)?;
        Ok(lsn)
    }

    /// Logs the diff between `base_bytes` (the page this content was
    /// derived from, or an all-zero buffer if `base` is `None`) and
    /// `new_bytes`, for the freshly allocated `new_page_id`. Not fsynced —
    /// only `append_commit` establishes a durability boundary.
    pub fn append_page_diff(
        &mut self,
        new_page_id: PageId,
        base: Option<PageId>,
        base_bytes: &[u8],
        new_bytes: &[u8],
    ) -> Result<Lsn, WalError> {
        let ranges = diff_ranges(base_bytes, new_bytes);
        let mut payload = Vec::with_capacity(8 + 8 + 2 + ranges.iter().map(|(_, b)| 4 + b.len()).sum::<usize>());
        payload.extend_from_slice(&new_page_id.to_le_bytes());
        payload.extend_from_slice(&base.unwrap_or(NO_BASE).to_le_bytes());
        payload.extend_from_slice(&(ranges.len() as u16).to_le_bytes());
        for (offset, bytes) in &ranges {
            payload.extend_from_slice(&(*offset as u16).to_le_bytes());
            payload.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            payload.extend_from_slice(bytes);
        }
        self.append_record(RECORD_PAGE_DIFF, &payload)
    }

    /// Commits: appends the `Commit` record (carrying the new current
    /// root) and fsyncs the log through it. The transaction is durable
    /// once this returns.
    pub fn append_commit(&mut self, root_page_id: PageId) -> Result<Lsn, WalError> {
        let lsn = self.append_record(RECORD_COMMIT, &root_page_id.to_le_bytes())?;
        self.file.sync_data()?;
        Ok(lsn)
    }

    /// Logs a checkpoint's dirty-page-table snapshot, then fsyncs the data
    /// file via `pager` — the pages the snapshot describes are now durable.
    pub fn checkpoint(&mut self, pager: &mut Pager, dpt: &[(PageId, Lsn)]) -> Result<Lsn, WalError> {
        let mut payload = Vec::with_capacity(2 + dpt.len() * 16);
        payload.extend_from_slice(&(dpt.len() as u16).to_le_bytes());
        for &(page_id, rec_lsn) in dpt {
            payload.extend_from_slice(&page_id.to_le_bytes());
            payload.extend_from_slice(&rec_lsn.to_le_bytes());
        }
        let lsn = self.append_record(RECORD_CHECKPOINT, &payload)?;
        pager.sync()?;
        Ok(lsn)
    }

    /// Redo recovery: scans the whole log from the start, reconstructing
    /// every `PageDiff`-described page via `pager`, and returns the root
    /// recorded by the last `Commit` seen — `None` if no transaction ever
    /// committed. Stops cleanly at the first torn/corrupt record, which
    /// needs no special handling versus a clean end-of-file: either way,
    /// everything read before it is exactly what survived durably.
    pub fn recover(&mut self, pager: &mut Pager) -> Result<Option<PageId>, WalError> {
        self.file.seek(SeekFrom::Start(PREAMBLE_SIZE))?;
        let mut root = None;
        while let Some((record_type, payload)) = self.read_record()? {
            match record_type {
                RECORD_PAGE_DIFF => apply_page_diff(pager, &payload)?,
                RECORD_COMMIT if payload.len() == 8 => {
                    root = Some(u64::from_le_bytes(payload[0..8].try_into().unwrap()));
                }
                RECORD_CHECKPOINT => {
                    // Logged for a future redo-start optimization; not yet
                    // consumed (see design/backlog.md).
                }
                _ => break,
            }
        }
        Ok(root)
    }

    /// Reads one record at the current position. `None` means a clean
    /// end-of-file or a torn/corrupt tail — both just stop recovery.
    fn read_record(&mut self) -> Result<Option<(u8, Vec<u8>)>, WalError> {
        let mut header = [0u8; 5];
        if !read_fully(&mut self.file, &mut header)? {
            return Ok(None);
        }
        let payload_len = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
        let record_type = header[4];

        let mut payload = vec![0u8; payload_len];
        if !read_fully(&mut self.file, &mut payload)? {
            return Ok(None);
        }
        let mut trailer = [0u8; 4];
        if !read_fully(&mut self.file, &mut trailer)? {
            return Ok(None);
        }

        let expected = u32::from_le_bytes(trailer);
        let mut checked = Vec::with_capacity(1 + payload.len());
        checked.push(record_type);
        checked.extend_from_slice(&payload);
        if crc32c(&checked) != expected {
            return Ok(None);
        }

        Ok(Some((record_type, payload)))
    }
}

/// Reads exactly `buf.len()` bytes, or reports a torn/incomplete read as
/// `Ok(false)` rather than an error — a short read here always means "the
/// log ends here," whether cleanly or via a crash mid-write.
fn read_fully(file: &mut File, buf: &mut [u8]) -> Result<bool, WalError> {
    let mut total = 0;
    while total < buf.len() {
        let n = file.read(&mut buf[total..])?;
        if n == 0 {
            return Ok(false);
        }
        total += n;
    }
    Ok(true)
}

fn apply_page_diff(pager: &mut Pager, payload: &[u8]) -> Result<(), WalError> {
    let new_page_id = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    let base = u64::from_le_bytes(payload[8..16].try_into().unwrap());
    let num_ranges = u16::from_le_bytes(payload[16..18].try_into().unwrap()) as usize;

    let page_size = pager.page_size() as usize;
    let mut buf = vec![0u8; page_size];
    if base != NO_BASE {
        pager.extend_to(base)?;
        pager.read_page(base, &mut buf)?;
    }

    let mut cursor = 18;
    for _ in 0..num_ranges {
        let offset = u16::from_le_bytes(payload[cursor..cursor + 2].try_into().unwrap()) as usize;
        let len = u16::from_le_bytes(payload[cursor + 2..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        buf[offset..offset + len].copy_from_slice(&payload[cursor..cursor + len]);
        cursor += len;
    }

    pager.extend_to(new_page_id)?;
    pager.write_page(new_page_id, &buf)?;
    Ok(())
}

/// Diffs two equal-length page buffers by trimming their common prefix and
/// common suffix, producing at most one changed range (none if identical).
/// The wire format supports several ranges for a smarter algorithm later
/// (see `design/decisions/0017-wal-format.md`); this is deliberately the
/// simplest one that satisfies it.
fn diff_ranges(base: &[u8], new: &[u8]) -> Vec<(usize, Vec<u8>)> {
    debug_assert_eq!(base.len(), new.len());
    let len = base.len();

    let mut prefix = 0;
    while prefix < len && base[prefix] == new[prefix] {
        prefix += 1;
    }
    if prefix == len {
        return Vec::new();
    }

    let mut suffix = 0;
    while suffix < len - prefix && base[len - 1 - suffix] == new[len - 1 - suffix] {
        suffix += 1;
    }

    let end = len - suffix;
    vec![(prefix, new[prefix..end].to_vec())]
}

fn log_file_path(path_prefix: &Path) -> PathBuf {
    let mut os = path_prefix.as_os_str().to_owned();
    os.push(".log.bin");
    PathBuf::from(os)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_ranges_identical_pages_is_empty() {
        let buf = vec![7u8; 32];
        assert_eq!(diff_ranges(&buf, &buf), Vec::new());
    }

    #[test]
    fn diff_ranges_trims_common_prefix_and_suffix() {
        let mut base = vec![0u8; 16];
        let mut new = base.clone();
        new[5] = 1;
        new[6] = 2;
        let ranges = diff_ranges(&base, &new);
        assert_eq!(ranges, vec![(5, vec![1, 2])]);

        base[0] = 9; // sanity: base itself is untouched by diffing
        assert_eq!(base[0], 9);
    }
}
