//! Fixed SQLite/WAL header identity shared by bounded, read-only usage readers.
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::time::SystemTime;
const MAX_DATABASE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_WAL_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileStamp {
    pub(crate) size: u64,
    pub(crate) modified: SystemTime,
    pub(crate) created: Option<SystemTime>,
    pub(crate) identity: Option<(u64, u64)>,
    pub(crate) signature: [u8; 32],
}

impl FileStamp {
    fn of<const HEADER_BYTES: usize>(path: &Path) -> Option<Self> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return None;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return None;
            }
        }
        let mut file = std::fs::File::open(path).ok()?;
        let meta = file.metadata().ok()?;
        // Only SQLite's fixed header belongs in this cache signature. Never
        // sample arbitrary database pages or WAL frame payloads at the tail.
        let mut header = [0; HEADER_BYTES];
        let length = meta.len().min(HEADER_BYTES as u64) as usize;
        file.read_exact(&mut header[..length]).ok()?;
        let mut hash = Sha256::new();
        hash.update(&header[..length]);
        Some(Self {
            size: meta.len(),
            modified: meta.modified().ok()?,
            created: meta.created().ok(),
            identity: file_identity(&file, &meta),
            signature: hash.finalize().into(),
        })
    }
}

fn file_identity(_file: &std::fs::File, _metadata: &std::fs::Metadata) -> Option<(u64, u64)> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(_file.as_raw_handle()), &mut info) }.ok()?;
        Some((
            u64::from(info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((_metadata.dev(), _metadata.ino()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub(crate) database: FileStamp,
    pub(crate) wal: Option<FileStamp>,
}

impl Stamp {
    pub(crate) fn has_wal(&self) -> bool {
        self.wal.is_some()
    }
    pub(crate) fn has_identity(&self) -> bool {
        self.database.identity.is_some()
    }
    pub(crate) fn of(path: &Path) -> Option<Self> {
        let database = FileStamp::of::<100>(path)?;
        let wal_path = path.with_file_name(format!("{}-wal", path.file_name()?.to_str()?));
        let wal = if wal_path.try_exists().ok()? {
            // A read-only SQLite connection may create an empty WAL sidecar.
            // It carries no frames; treat it as absent so that merely opening
            // a WAL-mode database does not manufacture a data-change gap.
            Some(FileStamp::of::<32>(&wal_path)?).filter(|wal| wal.size != 0)
        } else {
            None
        };
        if database.size > MAX_DATABASE_BYTES || wal.is_some_and(|m| m.size > MAX_WAL_BYTES) {
            return None;
        }
        Some(Self { database, wal })
    }
}

/// The first two 48-byte copies in SQLite's standard WAL index contain the
/// commit counter and last committed frame checksum. The following read marks
/// and locks are volatile reader state and must not invalidate cached counts.
pub(crate) fn wal_index_head(path: &Path) -> Option<[u8; 48]> {
    let path = path.with_file_name(format!("{}-shm", path.file_name()?.to_str()?));
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return None;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return None;
        }
    }
    let mut header = [0_u8; 96];
    std::fs::File::open(path)
        .ok()?
        .read_exact(&mut header)
        .ok()?;
    if header[..48] != header[48..]
        || header[12] != 1
        || u32::from_ne_bytes(header[..4].try_into().ok()?) != 3_007_000
    {
        return None;
    }
    header[..48].try_into().ok()
}
