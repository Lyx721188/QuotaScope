//! A disk budget for rebuildable statistics, independent of account storage.
use serde::Serialize;
use std::io;
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

const FILES: [&str; 4] = [
    "ledger-4-claudeCode.json",
    "ledger-5-claudeCode.json",
    "ledger-4-codex.json",
    "token-spend-2.json",
];
const MIB: u64 = 1024 * 1024;
static DISK: Mutex<()> = Mutex::new(());

pub const LIMITS_MB: [u32; 4] = [0, 16, 64, 256];

pub fn limit_mb(value: u32) -> u32 {
    if LIMITS_MB.contains(&value) {
        value
    } else {
        64
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Inventory {
    pub bytes: u64,
    pub files: Vec<CacheFile>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CacheFile {
    pub name: String,
    pub bytes: u64,
    #[serde(skip)]
    modified: SystemTime,
}

#[derive(Clone, Debug, Default)]
pub struct Cleanup {
    pub removed_files: usize,
    pub freed_bytes: u64,
    pub failed_files: usize,
    pub inventory: Inventory,
}

fn regular_file(path: &Path) -> Option<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return None;
        }
    }
    Some(metadata)
}

fn inspect_at(dir: &Path) -> Inventory {
    let files: Vec<_> = FILES
        .iter()
        .flat_map(|name| [(*name).to_owned(), format!("{name}.tmp")])
        .filter_map(|name| {
            let metadata = regular_file(&dir.join(&name))?;
            Some(CacheFile {
                name,
                bytes: metadata.len(),
                modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            })
        })
        .collect();
    Inventory {
        bytes: files.iter().map(|file| file.bytes).sum(),
        files,
    }
}

pub fn inspect() -> Inventory {
    let _disk = DISK.lock().unwrap_or_else(|e| e.into_inner());
    inspect_at(&crate::data_dir())
}

fn prune_at(dir: &Path, budget: u64) -> Cleanup {
    let mut inventory = inspect_at(dir);
    inventory
        .files
        .sort_by(|a, b| a.modified.cmp(&b.modified).then(a.name.cmp(&b.name)));
    let mut total = inventory.bytes;
    let mut result = Cleanup::default();
    for file in inventory.files {
        if total <= budget && budget != 0 {
            break;
        }
        // Only known, regular files directly in the application directory.
        if regular_file(&dir.join(&file.name)).is_none() {
            continue;
        }
        match std::fs::remove_file(dir.join(&file.name)) {
            Ok(()) => {
                total = total.saturating_sub(file.bytes);
                result.removed_files += 1;
                result.freed_bytes += file.bytes;
            }
            Err(_) => result.failed_files += 1,
        }
    }
    result.inventory = inspect_at(dir);
    result
}

/// Worker-only: serializes budget changes, writes and explicit cleanup.
pub fn enforce() -> Cleanup {
    let budget = crate::settings::with(|s| limit_mb(s.statistics_cache_limit_mb)) as u64 * MIB;
    let _disk = DISK.lock().unwrap_or_else(|e| e.into_inner());
    prune_at(&crate::data_dir(), budget)
}

pub(crate) fn clear_disk() -> Cleanup {
    let _disk = DISK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = crate::data_dir();
    prune_at(&dir, 0)
}

fn write_at(dir: &Path, name: &str, data: &[u8], budget: u64) -> io::Result<()> {
    if !FILES.contains(&name) {
        return Err(io::Error::other("unknown statistics cache"));
    }
    let path = dir.join(name);
    if data.len() as u64 > budget || budget == 0 {
        if regular_file(&path).is_some() {
            std::fs::remove_file(&path)?;
        }
        let _ = prune_at(dir, budget);
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    // A pre-existing non-regular target must never be followed or replaced.
    if path.symlink_metadata().is_ok() && regular_file(&path).is_none() {
        return Err(io::Error::other("non-regular cache target"));
    }
    let temporary = dir.join(format!("{name}.tmp"));
    // The process-wide disk lock makes an old regular temporary file an
    // interrupted write. Recover it even when the total is below budget.
    if regular_file(&temporary).is_some() {
        std::fs::remove_file(&temporary)?;
    }
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    use std::io::Write;
    let saved = output.write_all(data);
    drop(output);
    let saved = saved.and_then(|_| std::fs::rename(&temporary, &path));
    if saved.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    saved?;
    let result = prune_at(dir, budget);
    if result.failed_files > 0 && result.inventory.bytes > budget {
        return Err(io::Error::other("statistics cache budget exceeded"));
    }
    Ok(())
}

pub(crate) fn write(name: &str, data: &[u8]) -> io::Result<()> {
    let budget = crate::settings::with(|s| limit_mb(s.statistics_cache_limit_mb)) as u64 * MIB;
    let _disk = DISK.lock().unwrap_or_else(|e| e.into_inner());
    if !crate::scan::checkpoint() {
        return Err(io::Error::other("scan cancelled"));
    }
    write_at(&crate::data_dir(), name, data, budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "qs-cache-{}-{}-{}",
                std::process::id(),
                crate::timeutil::now_ms(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn put(&self, name: &str, bytes: &[u8]) {
            std::fs::write(self.0.join(name), bytes).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn budget_evicts_oldest_and_preserves_unrelated_files() {
        let dir = Fixture::new();
        dir.put(FILES[0], b"old");
        dir.put(FILES[1], b"newer");
        use std::fs::FileTimes;
        std::fs::File::options()
            .write(true)
            .open(dir.0.join(FILES[0]))
            .unwrap()
            .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        for name in [
            "settings.json",
            "keys.bin",
            "last-readings.json",
            "model-prices-4.json",
            "ledger-4-unknown.json",
        ] {
            dir.put(name, b"retain");
        }
        let cleaned = prune_at(&dir.0, 5);
        assert_eq!(
            (
                cleaned.removed_files,
                cleaned.freed_bytes,
                cleaned.inventory.bytes
            ),
            (1, 3, 5)
        );
        assert_eq!(
            std::fs::read(dir.0.join("settings.json")).unwrap(),
            b"retain"
        );
        assert_eq!(inspect_at(&dir.0).files.len(), 1);
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 6);
    }
    #[test]
    fn oversized_and_disabled_writes_preserve_live_results_without_disk_growth() {
        let dir = Fixture::new();
        dir.put(FILES[1], b"old");
        write_at(&dir.0, FILES[1], b"too large", 4).unwrap();
        assert_eq!(inspect_at(&dir.0).bytes, 0);
        write_at(&dir.0, FILES[1], b"tiny", 0).unwrap();
        assert!(!dir.0.join(FILES[1]).exists());
        assert!(write_at(&dir.0, "../settings.json", b"no", 100).is_err());
    }
    #[test]
    fn atomic_write_replaces_cache_and_enforces_combined_budget() {
        let dir = Fixture::new();
        write_at(&dir.0, FILES[0], b"old", 5).unwrap();
        write_at(&dir.0, FILES[1], b"newer", 5).unwrap();
        assert_eq!(inspect_at(&dir.0).bytes, 5);
        write_at(&dir.0, FILES[1], b"new", 5).unwrap();
        assert_eq!(std::fs::read(dir.0.join(FILES[1])).unwrap(), b"new");
        assert!(!dir.0.join(format!("{}.tmp", FILES[1])).exists());
    }
    #[test]
    fn directories_and_busy_temporary_targets_are_preserved() {
        let dir = Fixture::new();
        std::fs::create_dir(dir.0.join(FILES[0])).unwrap();
        std::fs::create_dir(dir.0.join(format!("{}.tmp", FILES[1]))).unwrap();
        assert!(write_at(&dir.0, FILES[0], b"bad", 100).is_err());
        assert!(write_at(&dir.0, FILES[1], b"bad", 100).is_err());
        assert_eq!(prune_at(&dir.0, 0).removed_files, 0);
        assert!(dir.0.join(FILES[0]).is_dir());
    }
    #[test]
    fn cleanup_removes_zero_length_and_interrupted_writes() {
        let dir = Fixture::new();
        dir.put(FILES[1], b"");
        dir.put(&format!("{}.tmp", FILES[0]), b"unfinished");
        assert_eq!(prune_at(&dir.0, 0).removed_files, 2);
        assert!(inspect_at(&dir.0).files.is_empty());
    }
    #[test]
    fn a_stale_temporary_file_does_not_disable_future_cache_writes() {
        let dir = Fixture::new();
        dir.put(&format!("{}.tmp", FILES[1]), b"interrupted");
        write_at(&dir.0, FILES[1], b"complete", 100).unwrap();
        assert_eq!(std::fs::read(dir.0.join(FILES[1])).unwrap(), b"complete");
        assert_eq!(inspect_at(&dir.0).files.len(), 1);
    }
    #[test]
    fn legacy_settings_default_to_a_bounded_cache() {
        let settings: crate::settings::AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.statistics_cache_limit_mb, 64);
        assert_eq!(limit_mb(123), 64);
        assert_eq!(limit_mb(0), 0);
    }
    #[cfg(windows)]
    #[test]
    fn a_locked_cache_is_reported_and_can_be_cleaned_after_release() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = Fixture::new();
        dir.put(FILES[1], b"locked");
        let held = std::fs::File::options()
            .read(true)
            .share_mode(3)
            .open(dir.0.join(FILES[1]))
            .unwrap();
        let blocked = prune_at(&dir.0, 0);
        assert_eq!(blocked.failed_files, 1);
        assert_eq!(blocked.removed_files, 0);
        assert_eq!(blocked.inventory.bytes, 6);
        drop(held);
        assert_eq!(prune_at(&dir.0, 0).removed_files, 1);
    }
}
