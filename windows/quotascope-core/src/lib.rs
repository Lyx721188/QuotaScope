//! QuotaScope core for Windows — a port of the macOS QuotaScope data layer.
//!
//! Everything here is platform-independent: the data model, the provider
//! routes, the refresh loop, the cache, the forecast and the `--json`
//! contract. The Windows shell (`quotascope-win`) draws on top of it.

pub mod accounts;
pub mod additional_spend;
pub mod alerts;
pub mod balance_ring;
pub mod browser_cookies;
pub mod browser_storage;
pub mod cache;
pub mod codex_account;
pub mod codex_rpc;
pub mod diagnostics;
pub mod elsewhere;
pub mod estimate;
pub mod extension;
pub mod gateway;
pub mod history;
pub mod http;
pub mod integration;
pub mod kiro_acp;
pub mod ledger;
pub mod localization;
pub mod model;
pub mod model_details;
pub mod model_prices;
pub mod opencode_console;
pub mod opencode_store;
pub mod prompt_cache;
pub mod providers;
pub mod proxy;
pub mod report;
pub mod scan;
pub mod secrets;
pub mod settings;
pub mod spend;
pub mod spend_warmer;
pub mod statistics_cache;
pub mod statusline;
pub mod store;
pub mod timeutil;
pub mod updates;

/// The application data directory: `%APPDATA%\QuotaScope`.
pub fn data_dir() -> std::path::PathBuf {
    let base = std::env::var("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("USERPROFILE").unwrap_or_default();
            std::path::PathBuf::from(home)
                .join("AppData")
                .join("Roaming")
        });
    let current = base.join("QuotaScope");
    if !current.exists() {
        migrate_legacy_data(&base.join("Pulse"), &current);
    }
    current
}

fn migrate_legacy_data(legacy: &std::path::Path, current: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(legacy) else {
        return;
    };
    if std::fs::create_dir_all(current).is_err() {
        return;
    }
    for entry in entries.flatten() {
        let source = entry.path();
        if !source.is_file() {
            continue;
        }
        let target = current.join(entry.file_name());
        if !target.exists() {
            let _ = std::fs::copy(source, target);
        }
    }
}

/// The user's home directory (`%USERPROFILE%`).
pub fn home_dir() -> std::path::PathBuf {
    std::env::var("USERPROFILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("C:\\Users"))
}
