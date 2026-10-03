//! Small local reports constructed from an explicit whitelist of fields.
//! Credentials, account labels, URLs, paths and transcripts are never read.
use serde::Serialize;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize)]
pub struct ProcessMetrics {
    pub private_bytes: Option<u64>,
    pub working_set_bytes: Option<u64>,
    pub handles: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScanStatus {
    #[default]
    Idle,
    Running,
    Stopping,
    Cancelled,
    Failed,
    Complete,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    Failed,
    UpToDate,
    Available,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Runtime {
    pub settings_visible: bool,
    pub scan_status: ScanStatus,
    pub sources_done: usize,
    pub sources_total: usize,
    pub files_read: u64,
    pub update_status: UpdateStatus,
    pub process: ProcessMetrics,
}

#[derive(Serialize)]
struct Preferences {
    enabled_account_count: usize,
    local_statistics_enabled: bool,
    statistics_cache_limit_mb: u32,
    automatic_updates: bool,
    last_update_check_at: Option<i64>,
    update_version_skipped: bool,
    refresh_interval_seconds: i64,
}

#[derive(Serialize)]
struct Report {
    schema_version: u8,
    generated_at: String,
    app_version: &'static str,
    os: &'static str,
    architecture: &'static str,
    preferences: Preferences,
    runtime: Runtime,
    statistics_cache: crate::statistics_cache::Inventory,
}

fn report(
    settings: &crate::settings::AppSettings,
    runtime: Runtime,
    inventory: crate::statistics_cache::Inventory,
) -> Report {
    Report {
        schema_version: 1,
        generated_at: chrono::Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        preferences: Preferences {
            enabled_account_count: settings.enabled_accounts.len(),
            local_statistics_enabled: settings.reads_token_spend,
            statistics_cache_limit_mb: crate::statistics_cache::limit_mb(
                settings.statistics_cache_limit_mb,
            ),
            automatic_updates: settings.checks_for_updates,
            last_update_check_at: settings.last_update_check_at,
            update_version_skipped: settings.skipped_update_version.is_some(),
            refresh_interval_seconds: settings.refresh_interval,
        },
        runtime,
        statistics_cache: inventory,
    }
}

fn regular_or_absent(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Err(e) => e.kind() == io::ErrorKind::NotFound,
        Ok(m) => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    return false;
                }
            }
            m.is_file()
        }
    }
}

fn export_at(dir: &Path, report: &Report) -> io::Result<PathBuf> {
    let folder = dir.join("diagnostics");
    std::fs::create_dir_all(&folder)?;
    let path = folder.join("QuotaScope-diagnostics.json");
    let tmp = folder.join("QuotaScope-diagnostics.json.tmp");
    if !regular_or_absent(&path) || !regular_or_absent(&tmp) {
        return Err(io::Error::other("non-regular diagnostic target"));
    }
    // A fixed report replaces the previous export; repeated clicks stay bounded.
    if tmp.exists() {
        std::fs::remove_file(&tmp)?;
    }
    let data = serde_json::to_vec_pretty(report)?;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp)?;
    use std::io::Write;
    let result = file.write_all(&data);
    drop(file);
    let result = result.and_then(|_| std::fs::rename(&tmp, &path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    Ok(path)
}

/// Worker-only; a fresh export replaces the prior local report.
pub fn export(runtime: Runtime) -> io::Result<PathBuf> {
    static EXPORT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _export = EXPORT.lock().unwrap_or_else(|e| e.into_inner());
    let settings = crate::settings::with(Clone::clone);
    let report = report(&settings, runtime, crate::statistics_cache::inspect());
    export_at(&crate::data_dir(), &report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sensitive_settings_are_not_exported() {
        let mut settings = crate::settings::AppSettings::default();
        settings
            .enabled_accounts
            .insert("private-account-secret".into());
        settings.server_addresses.insert(
            "private-label".into(),
            "https://private-endpoint?key=secret".into(),
        );
        settings
            .extension_names
            .insert("secret-id".into(), "private-extension-name".into());
        settings.display = "private-device-path".into();
        settings.skipped_update_version = Some("private-arbitrary-string".into());
        let value = serde_json::to_value(report(&settings, Runtime::default(), Default::default()))
            .unwrap();
        let text = value.to_string();
        for marker in [
            "private-account-secret",
            "private-label",
            "private-endpoint",
            "private-extension-name",
            "private-device-path",
            "private-arbitrary-string",
            "secret-id",
        ] {
            assert!(!text.contains(marker));
        }
        assert_eq!(value["preferences"]["enabled_account_count"], 1);
        assert_eq!(value["preferences"]["update_version_skipped"], true);
        assert_eq!(value["preferences"].as_object().unwrap().len(), 7);
    }
    #[test]
    fn repeated_exports_replace_one_valid_report_and_preserve_other_files() {
        let dir = std::env::temp_dir().join(format!(
            "qs-diagnostics-{}-{}",
            std::process::id(),
            crate::timeutil::now_ms()
        ));
        std::fs::create_dir(&dir).unwrap();
        let report = report(&Default::default(), Runtime::default(), Default::default());
        let path = export_at(&dir, &report).unwrap();
        std::fs::write(dir.join("diagnostics/other.txt"), b"preserve").unwrap();
        export_at(&dir, &report).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(
            std::fs::read_dir(dir.join("diagnostics")).unwrap().count(),
            2
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(dir.join("diagnostics/other.txt")).unwrap();
        std::fs::remove_dir(dir.join("diagnostics")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
