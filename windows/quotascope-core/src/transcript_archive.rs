//! Non-rebuildable transcript counts. Statistics-cache cleanup must not touch
//! these files. Only measured counts are kept, never transcript text.
use crate::ledger::{canonical_slot_key, TokenTally};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub(crate) type Buckets = BTreeMap<String, BTreeMap<String, TokenTally>>;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// The most files one archive holds. With the per-file kind cap this keeps
/// every stored figure and every aggregate exact in an `i64`; the bounds test
/// below pins that relationship.
pub(crate) const MAX_FILES: usize = 50_000;
static DISK: Mutex<()> = Mutex::new(());

/// One file's last observation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Kept {
    pub days: Buckets,
    /// This read saw only part of the file. Such an observation never replaces
    /// nor blends with one already stored.
    pub partial: bool,
}

impl Kept {
    pub fn add_into(&self, buckets: &mut Buckets) {
        add_days(buckets, &self.days);
    }

    /// Counts a reader of this version could have measured: canonical
    /// quarter-hour slots and kind totals no single file can report above.
    fn measured(&self) -> bool {
        self.days.iter().all(|(slot, models)| {
            canonical_slot_key(slot) && models.values().all(TokenTally::storable)
        })
    }
}

/// What happened to an observation offered to the archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stored {
    /// The observation is now the file's stored one.
    Updated,
    /// The stored observation already matched it.
    Unchanged,
    /// The archive is full and this is a file it has never held. Its counts
    /// are shown by the current read but cannot be kept.
    Rejected,
    /// A partial read did not displace the stored observation.
    PartialKept,
    /// Not a measured observation of this root; nothing was stored.
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Archive {
    format: u8,
    root: String,
    pub files: BTreeMap<String, Kept>,
}

impl Archive {
    pub fn empty(root: &Path) -> Self {
        Self {
            format: 1,
            root: root_key(root),
            files: BTreeMap::new(),
        }
    }

    /// Missing means new. Damaged, oversized, future-format or another root
    /// means read-only failure: never replace irreplaceable history with empty.
    /// A file that parses but states impossible counts, non-canonical slots or
    /// paths outside this root is treated the same way and left untouched.
    pub fn load(path: &Path, root: &Path) -> io::Result<Self> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::empty(root)),
            Err(error) => return Err(error),
        };
        if !regular(path) || file.metadata()?.len() > MAX_BYTES {
            return Err(io::Error::other("archive file is not readable"));
        }
        let mut data = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut data)?;
        if data.len() as u64 > MAX_BYTES {
            return Err(io::Error::other("archive too large"));
        }
        let archive: Self = serde_json::from_slice(&data)?;
        if archive.format != 1
            || archive.root != root_key(root)
            || archive.files.len() > MAX_FILES
            || !archive.readable()
        {
            return Err(io::Error::other("unsupported archive"));
        }
        Ok(archive)
    }

    fn readable(&self) -> bool {
        let root = Path::new(&self.root);
        self.files
            .iter()
            .all(|(name, kept)| within_root(name, root) && kept.measured())
    }

    /// One file, one observation. A complete reread replaces the previous one,
    /// including an explicit zero or a corrected count. A partial read never
    /// blends with or replaces a stored observation: a blend would state a
    /// figure no single read produced. It only fills a path the archive does
    /// not know, and it marks the stored observation as standing in for a file
    /// that has grown since, so the read stays flagged partial.
    pub fn observe(&mut self, key: String, kept: Kept) -> Stored {
        if !within_root(&key, Path::new(&self.root)) || !kept.measured() {
            return Stored::Invalid;
        }
        match self.files.get_mut(&key) {
            Some(old) => {
                if *old == kept {
                    return Stored::Unchanged;
                }
                if kept.partial {
                    old.partial = true;
                    return Stored::PartialKept;
                }
                *old = kept;
                Stored::Updated
            }
            None => {
                if self.files.len() >= MAX_FILES {
                    return Stored::Rejected;
                }
                self.files.insert(key, kept);
                Stored::Updated
            }
        }
    }

    pub fn buckets(&self) -> Buckets {
        let mut buckets: Buckets = BTreeMap::new();
        for file in self.files.values() {
            if !crate::scan::checkpoint() {
                break;
            }
            add_days(&mut buckets, &file.days);
        }
        buckets
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let _guard = DISK.lock().unwrap_or_else(|e| e.into_inner());
        if !crate::scan::checkpoint() {
            return Err(io::Error::other("cancelled archive write"));
        }
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(io::Error::other("archive too large"));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if path.symlink_metadata().is_ok() && !regular(path) {
            return Err(io::Error::other("non-regular archive target"));
        }
        let tmp = path.with_extension("json.tmp");
        if tmp.symlink_metadata().is_ok() {
            if !regular(&tmp) {
                return Err(io::Error::other("non-regular archive temporary"));
            }
            std::fs::remove_file(&tmp)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        if !crate::scan::checkpoint() {
            let _ = std::fs::remove_file(&tmp);
            return Err(io::Error::other("cancelled archive write"));
        }
        std::fs::rename(tmp, path)
    }
}

fn add_days(buckets: &mut Buckets, days: &Buckets) {
    for (slot, models) in days {
        for (model, tally) in models {
            let total = buckets
                .entry(slot.clone())
                .or_default()
                .entry(model.clone())
                .or_default();
            total.input = total.input.saturating_add(tally.input);
            total.output = total.output.saturating_add(tally.output);
            total.cache_read = total.cache_read.saturating_add(tally.cache_read);
            total.cache_write = total.cache_write.saturating_add(tally.cache_write);
        }
    }
}

fn regular(path: &Path) -> bool {
    let Ok(meta) = path.symlink_metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}
fn root_key(root: &Path) -> String {
    let text = root.to_string_lossy().replace('/', "\\");
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}
/// A stored key must name a file under the archive's own root. `..` is never a
/// file name, so a key containing it is not a measurement of this root.
fn within_root(name: &str, root: &Path) -> bool {
    use std::path::Component;
    let normalized = root_key(Path::new(name));
    Path::new(name)
        .components()
        .all(|part| !matches!(part, Component::ParentDir | Component::CurDir))
        && Path::new(&normalized).starts_with(root)
}
pub(crate) fn path(directory: &Path, provider: crate::model::Provider, root: &Path) -> PathBuf {
    let hash = root_key(root).bytes().fold(0xcbf29ce484222325_u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    directory.join(format!(
        "archive-ledger-{}-{hash:016x}.json",
        provider.raw()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(days: &[(&str, &str, i64)], partial: bool) -> Kept {
        let mut buckets: Buckets = BTreeMap::new();
        for (slot, model, input) in days {
            buckets.entry((*slot).to_string()).or_default().insert(
                (*model).to_string(),
                TokenTally {
                    input: *input,
                    ..Default::default()
                },
            );
        }
        Kept {
            days: buckets,
            partial,
        }
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "qs-archive-{}-{}-{}",
                std::process::id(),
                crate::timeutil::now_ms(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn complete_corrections_and_zero_replace_while_partials_never_blend() {
        let root = Path::new("synthetic-home");
        let mut archive = Archive::empty(root);
        let key = root.join("session.jsonl").to_string_lossy().to_string();
        archive.observe(
            key.clone(),
            sample(&[("2026-10-01 09:00", "model", 20)], false),
        );
        archive.observe(
            key.clone(),
            sample(&[("2026-10-01 09:00", "model", 5)], true),
        );
        assert_eq!(archive.buckets()["2026-10-01 09:00"]["model"].input, 20);
        assert!(archive.files[&key].partial);
        archive.observe(
            key.clone(),
            sample(&[("2026-10-01 09:00", "model", 3)], false),
        );
        assert_eq!(archive.buckets()["2026-10-01 09:00"]["model"].input, 3);
        assert!(!archive.files[&key].partial);
        archive.observe(
            key.clone(),
            sample(&[("2026-10-01 09:00", "model", 0)], false),
        );
        assert_eq!(archive.buckets()["2026-10-01 09:00"]["model"].input, 0);
        assert_ne!(
            path(root, crate::model::Provider::Codex, root),
            path(root, crate::model::Provider::Codex, Path::new("other-home"))
        );
    }

    #[test]
    fn a_partial_read_of_a_known_file_keeps_the_stored_reads_figures() {
        let root = Path::new("synthetic-home");
        let mut archive = Archive::empty(root);
        let key = root.join("session.jsonl").to_string_lossy().to_string();
        let stored = sample(&[("2026-10-01 09:00", "model", 100)], false);
        assert_eq!(
            archive.observe(key.clone(), stored.clone()),
            Stored::Updated
        );
        // A second read saw the same file half-written: its figure differs and
        // must not combine with the stored one field by field.
        let half = Kept {
            days: BTreeMap::from([(
                "2026-10-01 09:00".to_string(),
                BTreeMap::from([(
                    "model".to_string(),
                    TokenTally {
                        input: 4,
                        output: 50,
                        ..Default::default()
                    },
                )]),
            )]),
            partial: true,
        };
        assert_eq!(archive.observe(key.clone(), half), Stored::PartialKept);
        let kept = &archive.files[&key];
        assert_eq!(
            (
                kept.days["2026-10-01 09:00"]["model"].input,
                kept.days["2026-10-01 09:00"]["model"].output
            ),
            (100, 0)
        );
        assert!(kept.partial);
        // The same partial observation changes nothing on the next scan.
        let before = archive.files.clone();
        assert_eq!(
            archive.observe(
                key.clone(),
                Kept {
                    days: archive.files[&key].days.clone(),
                    partial: true
                }
            ),
            Stored::Unchanged
        );
        assert_eq!(archive.files, before);
    }

    #[test]
    fn load_refuses_impossible_counts_slots_and_foreign_paths_untouched() {
        let fixture = Fixture::new();
        let root = fixture.0.join("home");
        std::fs::create_dir(&root).unwrap();
        let path = fixture.0.join("archive.json");
        let tally = serde_json::json!({"input": 1, "cache_write": 0, "cache_read": 0, "output": 0});
        let key = root.join("a.jsonl").to_string_lossy().to_string();
        let base = serde_json::json!({
            "format": 1,
            "root": root_key(&root),
            "files": { key.clone(): {"days": {"2026-10-01 09:00": {"model": tally}}, "partial": false} }
        });
        assert!(Archive::load(&path, &root).is_ok() || !path.exists());
        let variants = [
            serde_json::json!({"days": {"2026-10-01 09:00": {"model": {"input": -1, "output": 0}}}, "partial": false}),
            serde_json::json!({"days": {"2026-10-01 09:00": {"model": {"input": crate::ledger::MAX_STORED_COUNTER + 1}}}, "partial": false}),
            serde_json::json!({"days": {"2026-10-1 09:00": {"model": tally}}, "partial": false}),
            serde_json::json!({"days": {"not a slot": {"model": tally}}, "partial": false}),
        ];
        for days in variants {
            let mut value = base.clone();
            value["files"][&key] = days;
            let bytes = serde_json::to_vec(&value).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            assert!(Archive::load(&path, &root).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        for foreign in [
            "C:\\elsewhere\\a.jsonl".to_string(),
            root.join("..")
                .join("escaped.jsonl")
                .to_string_lossy()
                .to_string(),
            String::new(),
        ] {
            let mut value = base.clone();
            let kept = value["files"][&key].clone();
            value["files"] = serde_json::json!({ foreign: kept });
            let bytes = serde_json::to_vec(&value).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            assert!(Archive::load(&path, &root).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        let bytes = serde_json::to_vec(&base).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(Archive::load(&path, &root).unwrap().files.len(), 1);
    }

    #[test]
    fn archive_bounds_keep_every_aggregate_exact() {
        let most = MAX_FILES as i128 * crate::ledger::MAX_STORED_COUNTER as i128 * 4;
        assert!(most < i64::MAX as i128);
    }
}
