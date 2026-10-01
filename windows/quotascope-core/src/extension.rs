//! Extensions: a program in a folder that reports one account's usage.
//!
//! Ported from `PulseExtension.swift`. **Out of process, and host-drawn.**
//! QuotaScope starts the program, reads one JSON object from its standard
//! output and draws that with its own ring and card. Nothing is loaded into
//! QuotaScope, nothing the program sends is drawn as it sent it, and
//! QuotaScope hands it no credential: the program owns its own login. The
//! contract is `Docs/extensions.md`; this file is its one implementation.
//!
//! Each extension is an account of `Provider::Extension`, with the
//! manifest's `id` as its slot, so the rail, the cache and `--json` carry it
//! the way they carry an added account.

use crate::model::{AccountKey, CreditAmount, Kind, Provider, ProviderUsage, UsageWindow};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// The only schema this build reads, for the manifest and the report both.
pub const SCHEMA_VERSION: i64 = 1;
pub const MANIFEST_NAME: &str = "quotascope-extension.json";
pub const DEFAULT_TIMEOUT_SECS: f64 = 20.0;
/// A pass waits for every extension it asks, so the ceiling is a ceiling on
/// how late every other ring can be.
pub const TIMEOUT_RANGE: (f64, f64) = (1.0, 60.0);
/// More than any honest report needs. Past it the bytes are dropped and the
/// report fails to parse, rather than QuotaScope holding whatever a runaway
/// program cares to write.
const OUTPUT_CEILING: u64 = 256 * 1024;

/// Where extensions live: QuotaScope's own data folder, so it is somewhere
/// nothing else writes to and a reader who asks can be told exactly where
/// to look.
pub fn folder() -> PathBuf {
    crate::data_dir().join("Extensions")
}

/// One usable extension.
#[derive(Debug, Clone, PartialEq)]
pub struct Extension {
    /// From the manifest. Also the account's slot, so it has to survive
    /// being written into an account id — see `is_valid_id`.
    pub id: String,
    /// What the rail's card and Settings call it. The program's own word
    /// for itself, so it is never translated.
    pub name: String,
    /// The folder the manifest was found in; the program's working directory.
    pub directory: PathBuf,
    /// The program, resolved, and known to be inside `directory`.
    pub executable: PathBuf,
    /// How long a run may take before it is stopped.
    pub timeout_secs: f64,
}

impl Extension {
    pub fn account(&self) -> AccountKey {
        AccountKey {
            provider: Provider::Extension,
            slot: self.id.clone(),
        }
    }
}

/// What a scan found: the extensions that can be used, and why each folder
/// that could not be was turned away.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Scan {
    pub extensions: Vec<Extension>,
    pub problems: Vec<Problem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// The folder's name, which is all the reader needs to find it.
    pub folder: String,
    pub reason: ProblemReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProblemReason {
    NoManifest,
    UnreadableManifest,
    UnsupportedSchema(i64),
    InvalidId,
    MissingName,
    ExecutableOutsideFolder,
    ExecutableMissing,
    DuplicateId,
}

impl ProblemReason {
    pub fn message(&self) -> String {
        match self {
            ProblemReason::NoManifest => {
                crate::localization::t("No quotascope-extension.json in this folder.").to_string()
            }
            ProblemReason::UnreadableManifest => crate::localization::t(
                "quotascope-extension.json isn't valid JSON, or is missing a field.",
            )
            .to_string(),
            ProblemReason::UnsupportedSchema(version) => crate::localization::t_fmt(
                "Written for extension schema {n}, which this version of QuotaScope doesn't read.",
                &[&version.to_string()],
            ),
            ProblemReason::InvalidId => crate::localization::t(
                "Its id must be lowercase letters, digits, dots, dashes or underscores, 64 at most.",
            )
            .to_string(),
            ProblemReason::MissingName => crate::localization::t("Its name is empty.").to_string(),
            ProblemReason::ExecutableOutsideFolder => crate::localization::t(
                "Its program has to be inside its own folder.",
            )
            .to_string(),
            ProblemReason::ExecutableMissing => crate::localization::t(
                "Its program isn't there, or can't be run.",
            )
            .to_string(),
            ProblemReason::DuplicateId => {
                crate::localization::t("Another extension already uses this id.").to_string()
            }
        }
    }
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "schemaVersion")]
    schema_version: i64,
    id: String,
    name: String,
    executable: String,
    #[serde(rename = "timeoutSeconds")]
    timeout_seconds: Option<f64>,
}

/// Letters, digits and `.-_`, lowercase, starting with a letter or digit.
///
/// **Not a style rule.** The id becomes an account id — `extension#<id>` —
/// and account ids are split on `#`, so that character in here would cut
/// the account in two when it is read back.
pub fn is_valid_id(id: &str) -> bool {
    let len = id.chars().count();
    if !(1..=64).contains(&len) {
        return false;
    }
    let mut chars = id.chars();
    let first = chars.next().unwrap();
    let leading = first.is_ascii_lowercase() || first.is_ascii_digit();
    leading
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'))
}

/// Every folder directly inside `folder`, in name order. Hidden entries and
/// plain files are skipped: a `desktop.ini` is not a broken extension.
///
/// **Reading a manifest never runs anything.** A folder dropped into the
/// extensions directory is listed in Settings and does nothing else until it
/// is switched on there — the same rule as every built-in provider, which is
/// not fetched while it is off.
pub fn scan() -> Scan {
    scan_in(&folder())
}

pub fn scan_in(folder: &std::path::Path) -> Scan {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Scan::default();
    };

    let mut folders: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.path())
        .collect();
    folders.sort_by_key(|path| {
        path.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });

    let mut scan = Scan::default();
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    for directory in folders {
        let folder_name = directory
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        match load(&directory) {
            Ok(found) => {
                // The first folder in name order keeps it. Two programs
                // answering as one account would draw whichever ran last.
                if taken.contains(&found.id) {
                    scan.problems.push(Problem {
                        folder: folder_name,
                        reason: ProblemReason::DuplicateId,
                    });
                } else {
                    taken.insert(found.id.clone());
                    scan.extensions.push(found);
                }
            }
            Err(reason) => scan.problems.push(Problem {
                folder: folder_name,
                reason,
            }),
        }
    }
    scan
}

fn load(directory: &std::path::Path) -> Result<Extension, ProblemReason> {
    let manifest_path = directory.join(MANIFEST_NAME);
    let Ok(text) = std::fs::read_to_string(&manifest_path) else {
        return Err(ProblemReason::NoManifest);
    };
    let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
        return Err(ProblemReason::UnreadableManifest);
    };

    if manifest.schema_version != SCHEMA_VERSION {
        return Err(ProblemReason::UnsupportedSchema(manifest.schema_version));
    }
    if !is_valid_id(&manifest.id) {
        return Err(ProblemReason::InvalidId);
    }

    let name = manifest.name.trim();
    if name.is_empty() {
        return Err(ProblemReason::MissingName);
    }

    // Resolved before it is compared, so neither `..` nor a link can make a
    // program outside the folder look like one inside it.
    let root = canonical(directory)?;
    let executable = canonical(&root.join(&manifest.executable))?;
    if manifest.executable.starts_with('/') || manifest.executable.starts_with('\\') {
        return Err(ProblemReason::ExecutableOutsideFolder);
    }
    if !executable.starts_with(&root) || executable == root {
        return Err(ProblemReason::ExecutableOutsideFolder);
    }
    if !executable.is_file() {
        return Err(ProblemReason::ExecutableMissing);
    }

    let timeout = manifest
        .timeout_seconds
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(TIMEOUT_RANGE.0, TIMEOUT_RANGE.1);

    Ok(Extension {
        id: manifest.id,
        name: name.chars().take(60).collect(),
        directory: root,
        executable,
        timeout_secs: timeout,
    })
}

/// Canonicalised, or the path as it is when the resolution fails — a missing
/// program is reported as missing, not as unreadable.
fn canonical(path: &std::path::Path) -> Result<PathBuf, ProblemReason> {
    match path.canonicalize() {
        Ok(resolved) => Ok(resolved),
        Err(_) => Ok(path.to_path_buf()),
    }
}

/// Why a run produced nothing. Distinct from the report's own statuses:
/// these are about the program, not about the account.
enum RunOutcome {
    Report(String),
    CouldNotStart,
    TimedOut,
    Exited,
}

/// Runs an extension and turns what it printed into a reading.
pub fn fetch(extension: &Extension) -> ProviderUsage {
    let account = extension.account();
    // Checked again here, not only when the folder was scanned: the scan
    // was earlier, and the program may have gone since.
    if !extension.executable.is_file() {
        return ProviderUsage::unavailable(account, crate::model::Unavailability::ExtensionMissing);
    }

    match run(extension) {
        RunOutcome::Report(text) => reading(&text, &account),
        RunOutcome::CouldNotStart => {
            ProviderUsage::unavailable(account, crate::model::Unavailability::ExtensionMissing)
        }
        RunOutcome::TimedOut => {
            ProviderUsage::unavailable(account, crate::model::Unavailability::ExtensionTimedOut)
        }
        RunOutcome::Exited => {
            ProviderUsage::unavailable(account, crate::model::Unavailability::ExtensionFailed)
        }
    }
}

fn run(extension: &Extension) -> RunOutcome {
    use std::io::Read;
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    let child = Command::new(&extension.executable)
        .current_dir(&extension.directory)
        .env_clear()
        .envs(environment_for(extension))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn();

    let mut child = match child {
        Ok(child) => child,
        // A script whose interpreter is missing lands here, as does a file
        // Windows will not run.
        Err(_) => return RunOutcome::CouldNotStart,
    };

    // Read on a side thread so a program that writes a great deal cannot
    // fill the pipe and deadlock against our wait; `take` is the ceiling,
    // and one byte past it makes the parse fail.
    let (tx, rx) = mpsc::channel();
    let mut stdout = child.stdout.take().expect("piped stdout");
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = (&mut stdout)
            .take(OUTPUT_CEILING + 1)
            .read_to_end(&mut buffer);
        let _ = tx.send(buffer);
    });

    let deadline = Duration::from_secs_f64(extension.timeout_secs);
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = rx.recv().unwrap_or_default();
                if !status.success() {
                    return RunOutcome::Exited;
                }
                match String::from_utf8(output) {
                    Ok(text) => return RunOutcome::Report(text),
                    Err(_) => return RunOutcome::Exited,
                }
            }
            Ok(None) => {
                if started.elapsed() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return RunOutcome::TimedOut;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(_) => return RunOutcome::Exited,
        }
    }
}

/// **Only what a program needs to run, and the proxy.** Not QuotaScope's own
/// environment: whatever a shell or a terminal happened to leave in it —
/// tokens exported for some other tool included — is nobody's business but
/// the tool it was meant for. The PATH covers the system directories first,
/// then what the user's own session resolves, which is where a script's
/// `node` or `python` is going to be found.
fn environment_for(extension: &Extension) -> HashMap<String, String> {
    let mut env = HashMap::new();
    for key in [
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "TEMP",
        "TMP",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "COMPUTERNAME",
        "APPDATA",
        "LOCALAPPDATA",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "PATH",
    ] {
        if let Ok(value) = std::env::var(key) {
            env.insert(key.to_string(), value);
        }
    }
    for key in ["http_proxy", "https_proxy", "all_proxy", "no_proxy"] {
        for candidate in [key, &key.to_uppercase()] {
            if let Ok(value) = std::env::var(candidate) {
                env.insert(candidate.to_string(), value);
            }
        }
    }
    env.insert("QUOTASCOPE_EXTENSION_ID".into(), extension.id.clone());
    env.insert(
        "QUOTASCOPE_EXTENSION_SCHEMA".into(),
        SCHEMA_VERSION.to_string(),
    );
    env
}

#[derive(Deserialize)]
struct Report {
    #[serde(rename = "schemaVersion")]
    schema_version: i64,
    status: Option<String>,
    plan: Option<String>,
    limits: Option<Vec<Limit>>,
    balance: Option<Balance>,
}

/// Optional fields, so a balance missing one is dropped rather than taking
/// the limits beside it down with it.
#[derive(Deserialize)]
struct Balance {
    amount: Option<f64>,
    currency: Option<String>,
}

#[derive(Deserialize)]
struct Limit {
    id: Option<String>,
    label: String,
    #[serde(rename = "usedPercent")]
    used_percent: Option<f64>,
    used: Option<f64>,
    limit: Option<f64>,
    #[serde(rename = "resetsAt")]
    resets_at: Option<String>,
    #[serde(rename = "windowSeconds")]
    window_seconds: Option<i64>,
}

/// What an extension prints: one JSON object. See `Docs/extensions.md`.
///
/// **QuotaScope still invents nothing.** A limit is drawn from a percentage
/// the program states, or from a used amount and a limit it states; one with
/// neither is left off rather than drawn at zero, and a report with neither a
/// limit nor a balance left says so.
///
/// **A balance is money and nothing more.** A relay that sells prepaid
/// credit reports what is left and no allowance, so the report carries the
/// amount and its currency, and the ring it gets is the balance ring every
/// API account gets.
pub fn reading(text: &str, account: &AccountKey) -> ProviderUsage {
    let report = match serde_json::from_str::<Report>(text) {
        Ok(report) if report.schema_version == SCHEMA_VERSION => report,
        _ => {
            return ProviderUsage::unavailable(
                account.clone(),
                crate::model::Unavailability::UnreadableReply,
            )
        }
    };

    // Named failures only, each one mapped to wording that names no
    // provider. Anything else is a reply QuotaScope cannot read.
    match report.status.as_deref().unwrap_or("ok") {
        "ok" => {}
        "signedOut" => {
            return ProviderUsage::unavailable(
                account.clone(),
                crate::model::Unavailability::ExtensionSignedOut,
            )
        }
        "unreachable" => {
            return ProviderUsage::unavailable(
                account.clone(),
                crate::model::Unavailability::Unreachable,
            )
        }
        "rateLimited" => {
            return ProviderUsage::unavailable(
                account.clone(),
                crate::model::Unavailability::RateLimited,
            )
        }
        "serverError" => {
            return ProviderUsage::unavailable(
                account.clone(),
                crate::model::Unavailability::ServerError,
            )
        }
        "noLimits" => {
            return ProviderUsage::unavailable(
                account.clone(),
                crate::model::Unavailability::NoLimitsReported,
            )
        }
        _ => {
            return ProviderUsage::unavailable(
                account.clone(),
                crate::model::Unavailability::UnreadableReply,
            )
        }
    }

    let mut seen = std::collections::HashSet::new();
    let mut windows = Vec::new();
    for (index, limit) in (report.limits.unwrap_or_default()).into_iter().enumerate() {
        let Some(fraction) = used_fraction(&limit) else {
            continue;
        };
        let label = limit.label.trim();
        if label.is_empty() {
            continue;
        }
        // Pinning a ring to a limit is by id, so it has to stay put across
        // runs — the program's own if it gave one, else its place.
        let id = format!(
            "extension.{}",
            limit
                .id
                .map(|id| id.chars().take(64).collect::<String>())
                .unwrap_or_else(|| index.to_string())
        );
        if !seen.insert(id.clone()) {
            continue;
        }
        let seconds = limit.window_seconds.filter(|s| *s > 0);
        let mut window = UsageWindow::new(
            &id,
            Kind::Other(seconds.unwrap_or(0)),
            None,
            fraction,
            seconds.unwrap_or(0),
            limit
                .resets_at
                .and_then(|text| crate::timeutil::parse_iso8601_ms(&text)),
        );
        // A length only when the program stated one. Without it the window
        // clock has nothing to divide by and draws nothing.
        window.reports_length = seconds.is_some();
        window.is_exhausted = fraction >= 1.0;
        window.label = Some(label.chars().take(60).collect());
        windows.push(window);
    }

    let money = report.balance.as_ref().and_then(credit);
    if windows.is_empty() && money.is_none() {
        return ProviderUsage::unavailable(
            account.clone(),
            crate::model::Unavailability::NoLimitsReported,
        );
    }

    let mut usage = ProviderUsage::live_now(account.clone(), windows);
    usage.plan = report
        .plan
        .map(|p| p.trim().chars().take(60).collect::<String>())
        .filter(|p| !p.is_empty());
    usage.credit_balance = money.as_ref().map(balance_text);
    usage.credit_remaining = money;
    usage.origin = Some("extensionProgram".into());
    usage
}

/// The amount as stated, in a currency named by its ISO code. Below zero is
/// kept — some services run an account negative and keep serving it — but a
/// figure that is not a number, or a currency that is not a code, is no
/// balance at all.
fn credit(balance: &Balance) -> Option<CreditAmount> {
    let amount = balance.amount.filter(|a| a.is_finite())?;
    let currency = balance.currency.as_deref()?.trim().to_uppercase();
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    Some(CreditAmount { amount, currency })
}

/// The full figure, for the card — the rail's ring gets its own shape. Two
/// places, always, so ¥12.5 does not read as 12.5 of nothing.
fn balance_text(money: &CreditAmount) -> String {
    let symbol = match money.currency.as_str() {
        "CNY" | "RMB" => "¥",
        "USD" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" => "¥",
        other => {
            return format!("{:.2} {}", money.amount, other);
        }
    };
    format!("{symbol}{:.2}", money.amount)
}

/// The percentage as stated, or the used amount over the stated limit.
/// Nothing negative, nothing that is not a number, and no division by a
/// limit of zero.
fn used_fraction(limit: &Limit) -> Option<f64> {
    if let Some(percent) = limit.used_percent {
        return if percent.is_finite() && percent >= 0.0 {
            Some(percent / 100.0)
        } else {
            None
        };
    }
    let used = limit.used?;
    let total = limit.limit?;
    if !used.is_finite() || !total.is_finite() || used < 0.0 || total <= 0.0 {
        return None;
    }
    Some(used / total)
}

/// The catalog the store keeps: what a scan found, by id.
#[derive(Default)]
pub struct Catalog {
    pub by_id: HashMap<String, Extension>,
}

impl Catalog {
    pub fn rescan(&mut self) -> Scan {
        let scan = scan();
        self.by_id = scan
            .extensions
            .iter()
            .map(|e| (e.id.clone(), e.clone()))
            .collect();
        scan
    }

    pub fn get(&self, id: &str) -> Option<&Extension> {
        self.by_id.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Provider, State, Unavailability};

    fn account() -> AccountKey {
        AccountKey {
            provider: Provider::Extension,
            slot: "relay".into(),
        }
    }

    #[test]
    fn ids_keep_the_rules_the_account_id_needs() {
        assert!(is_valid_id("relay"));
        assert!(is_valid_id("a1.2-3_4"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("#leading"));
        assert!(!is_valid_id("-leading"));
        assert!(!is_valid_id("has space"));
        assert!(!is_valid_id("Uppercase"));
        assert!(!is_valid_id(&"a".repeat(65)));
    }

    #[test]
    fn a_report_with_a_percentage_becomes_a_window() {
        let usage = reading(
            r#"{"schemaVersion":1,"status":"ok","limits":[{"id":"monthly","label":"Monthly plan","usedPercent":42.5,"resetsAt":"2026-10-05T00:00:00Z","windowSeconds":2592000}]}"#,
            &account(),
        );
        assert!(matches!(usage.state, State::Live));
        assert_eq!(usage.windows.len(), 1);
        let w = &usage.windows[0];
        assert_eq!(w.id, "extension.monthly");
        assert_eq!(w.label.as_deref(), Some("Monthly plan"));
        assert!((w.used_fraction - 0.425).abs() < 1e-9);
        assert!(w.reports_length);
        assert!(!w.is_exhausted);
        assert_eq!(usage.origin.as_deref(), Some("extensionProgram"));
    }

    #[test]
    fn a_report_with_used_and_limit_divides_them() {
        let usage = reading(
            r#"{"schemaVersion":1,"limits":[{"label":"Week","used":30,"limit":120}]}"#,
            &account(),
        );
        assert!((usage.windows[0].used_fraction - 0.25).abs() < 1e-9);
        // No stated length: the window clock has nothing to divide by.
        assert!(!usage.windows[0].reports_length);
    }

    #[test]
    fn a_full_limit_is_spent_and_an_unstated_one_is_left_off() {
        let usage = reading(
            r#"{"schemaVersion":1,"limits":[{"label":"Full","usedPercent":100},{"label":"No figures"}]}"#,
            &account(),
        );
        assert_eq!(usage.windows.len(), 1);
        assert!(usage.windows[0].is_exhausted);
    }

    #[test]
    fn a_balance_alone_is_money_and_no_ring() {
        let usage = reading(
            r#"{"schemaVersion":1,"balance":{"amount":16.33,"currency":"USD"}}"#,
            &account(),
        );
        assert!(usage.windows.is_empty());
        assert_eq!(usage.credit_balance.as_deref(), Some("$16.33"));
        assert_eq!(usage.credit_remaining.unwrap().amount, 16.33);
    }

    #[test]
    fn a_bad_currency_is_no_balance() {
        let usage = reading(
            r#"{"schemaVersion":1,"balance":{"amount":5,"currency":"dollars"}}"#,
            &account(),
        );
        assert!(usage.credit_balance.is_none());
    }

    #[test]
    fn statuses_map_to_their_own_words() {
        for (status, expected) in [
            ("signedOut", Unavailability::ExtensionSignedOut),
            ("unreachable", Unavailability::Unreachable),
            ("rateLimited", Unavailability::RateLimited),
            ("serverError", Unavailability::ServerError),
            ("noLimits", Unavailability::NoLimitsReported),
            ("gibberish", Unavailability::UnreadableReply),
        ] {
            let text = format!(r#"{{"schemaVersion":1,"status":"{status}"}}"#);
            let usage = reading(&text, &account());
            match usage.state {
                State::Unavailable(reason) => assert_eq!(reason, expected, "{status}"),
                _ => panic!("{status} should be unavailable"),
            }
        }
    }

    #[test]
    fn a_report_from_another_schema_is_unreadable() {
        let usage = reading(r#"{"schemaVersion":2,"limits":[]}"#, &account());
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::UnreadableReply)
        ));
    }

    #[test]
    fn duplicate_limit_ids_keep_the_first() {
        let usage = reading(
            r#"{"schemaVersion":1,"limits":[{"id":"m","label":"First","usedPercent":10},{"id":"m","label":"Second","usedPercent":20}]}"#,
            &account(),
        );
        assert_eq!(usage.windows.len(), 1);
        assert!((usage.windows[0].used_fraction - 0.1).abs() < 1e-9);
    }

    #[test]
    fn a_scan_turns_away_folders_for_named_reasons() {
        let dir = std::env::temp_dir().join(format!("qs-ext-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let valid = dir.join("valid");
        std::fs::create_dir_all(&valid).unwrap();
        std::fs::write(
            valid.join(MANIFEST_NAME),
            r#"{"schemaVersion":1,"id":"probe","name":"Probe","executable":"probe.exe"}"#,
        )
        .unwrap();
        // The program must exist for the folder to be usable.
        std::fs::write(valid.join("probe.exe"), b"").unwrap();

        let scan = scan_in(&dir);
        assert_eq!(scan.extensions.len(), 1);
        assert_eq!(scan.extensions[0].id, "probe");
        assert_eq!(scan.extensions[0].name, "Probe");
        assert_eq!(scan.problems.len(), 1);
        assert_eq!(scan.problems[0].folder, "empty");
        assert_eq!(scan.problems[0].reason, ProblemReason::NoManifest);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_program_outside_its_folder_is_turned_away() {
        let dir = std::env::temp_dir().join(format!("qs-ext-esc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = dir.join("esc");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join(MANIFEST_NAME),
            r#"{"schemaVersion":1,"id":"esc","name":"Esc","executable":"../outside.exe"}"#,
        )
        .unwrap();
        let scan = scan_in(&dir);
        assert_eq!(scan.extensions.len(), 0);
        assert_eq!(
            scan.problems[0].reason,
            ProblemReason::ExecutableOutsideFolder
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
