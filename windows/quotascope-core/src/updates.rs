//! Optional Windows release discovery. No downloading or installation.

use serde::Deserialize;
use std::io::Read;
use std::time::Duration;

const API: &str = "https://api.github.com/repos/Lyx721188/QuotaScope/releases?per_page=100";
pub const RELEASES_PAGE: &str = "https://github.com/Lyx721188/QuotaScope/releases";
const MAX_REPLY: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Daily,
    About,
    Manual,
}

/// About needs an answer in this process, rather than only a persisted attempt
/// time. Recent successes and failures both cool down; manual retry is explicit.
pub fn should_check(
    trigger: Trigger,
    enabled: bool,
    last_attempt: Option<i64>,
    now: i64,
    result_age: Option<Duration>,
) -> bool {
    match trigger {
        Trigger::Manual => true,
        Trigger::About => enabled && result_age.is_none_or(|age| age >= Duration::from_secs(300)),
        Trigger::Daily => {
            enabled
                && last_attempt
                    .is_none_or(|at| now < at || now.saturating_sub(at) >= 24 * 60 * 60 * 1000)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    Available(Release),
    UpToDate,
    Failed,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
}

fn version(text: &str) -> Option<[u64; 3]> {
    let parts: Vec<_> = text.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut out = [0; 3];
    for (slot, part) in out.iter_mut().zip(parts) {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    Some(out)
}

pub fn check(current: &str) -> CheckResult {
    check_inner(current).unwrap_or(CheckResult::Failed)
}

fn check_inner(current: &str) -> Option<CheckResult> {
    let client = crate::http::HttpClient::new().client_for_login();
    let response = client
        .get(API)
        .timeout(Duration::from_secs(20))
        .header(
            "User-Agent",
            concat!("QuotaScope/", env!("CARGO_PKG_VERSION"), " (Windows)"),
        )
        .header("Accept", "application/vnd.github+json")
        .send()
        .ok()?
        .error_for_status()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut bytes = Vec::new();
    response.take(MAX_REPLY + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_REPLY {
        return None;
    }
    parse(&bytes, current)
}

fn parse(bytes: &[u8], current: &str) -> Option<CheckResult> {
    let current = version(current)?;
    let releases: Vec<ApiRelease> = serde_json::from_slice(bytes).ok()?;
    let latest = releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter(|r| {
            r.assets
                .iter()
                .any(|a| a.state == "uploaded" && a.name == "quotascope-windows-x64.zip")
        })
        .filter_map(|r| {
            let number = r.tag_name.strip_prefix("windows-v")?;
            Some((version(number)?, number.to_owned(), r.tag_name))
        })
        .max_by_key(|r| r.0)?;
    Some(if latest.0 > current {
        CheckResult::Available(Release {
            version: latest.1,
            // Construct from a validated version instead of trusting remote URLs.
            url: format!("{RELEASES_PAGE}/tag/{}", latest.2),
        })
    } else {
        CheckResult::UpToDate
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    #[ignore = "contacts the public GitHub releases API; run explicitly"]
    fn live_windows_release_can_be_discovered() {
        let result = check("0.0.0");
        assert!(matches!(&result, CheckResult::Available(_)), "{result:?}");
        println!("Public Windows release discovery: {result:?}");
    }

    fn release(tag: &str) -> serde_json::Value {
        json!({"tag_name":tag,"draft":false,"prerelease":false,
            "assets":[{"name":"quotascope-windows-x64.zip","state":"uploaded"}]})
    }

    #[test]
    fn selects_numeric_windows_version_and_ignores_other_channels() {
        let mut draft = release("windows-v9.0.0");
        draft["draft"] = json!(true);
        let mut preview = release("windows-v8.0.0");
        preview["prerelease"] = json!(true);
        let mut incomplete = release("windows-v7.0.0");
        incomplete["assets"] = json!([]);
        let bytes = serde_json::to_vec(&json!([
            release("v99.0.0"),
            draft,
            preview,
            incomplete,
            release("windows-v1.9.0"),
            release("windows-v1.10.0"),
            release("windows-v2.0.0-beta"),
            release("windows-v1.11.0/evil")
        ]))
        .unwrap();
        assert_eq!(
            parse(&bytes, "1.2.3"),
            Some(CheckResult::Available(Release {
                version: "1.10.0".into(),
                url: format!("{RELEASES_PAGE}/tag/windows-v1.10.0"),
            }))
        );
        assert_eq!(parse(&bytes, "1.10.0"), Some(CheckResult::UpToDate));
        assert_eq!(parse(&bytes, "2.0.0"), Some(CheckResult::UpToDate));
    }

    #[test]
    fn missing_or_invalid_release_data_is_not_up_to_date() {
        for bytes in [b"[]".as_slice(), b"{}", b"invalid"] {
            assert_eq!(parse(bytes, "1.2.3"), None);
        }
        assert_eq!(version("1.2"), None);
        assert_eq!(version("1.2.3.4"), None);
    }

    #[test]
    fn old_settings_default_to_manual_checks_and_round_trip_choices() {
        let mut settings: crate::settings::AppSettings = serde_json::from_str("{}").unwrap();
        assert!(!settings.checks_for_updates);
        settings.checks_for_updates = true;
        settings.skipped_update_version = Some("1.10.0".into());
        let copy: crate::settings::AppSettings =
            serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
        assert_eq!(copy, settings);
    }

    #[test]
    fn about_and_daily_checks_respect_opt_in_while_manual_retry_is_explicit() {
        for trigger in [Trigger::About, Trigger::Daily] {
            assert!(!should_check(trigger, false, None, 1000, None));
        }
        assert!(should_check(
            Trigger::Manual,
            false,
            Some(1000),
            1000,
            Some(Duration::ZERO)
        ));
    }

    #[test]
    fn about_probes_a_new_process_and_reuses_both_recent_answers_and_failures() {
        assert!(should_check(Trigger::About, true, Some(1000), 1000, None));
        assert!(!should_check(
            Trigger::About,
            true,
            Some(1000),
            1000,
            Some(Duration::from_secs(299))
        ));
        assert!(should_check(
            Trigger::About,
            true,
            Some(1000),
            1000,
            Some(Duration::from_secs(300))
        ));
    }

    #[test]
    fn daily_schedule_retains_its_persisted_day_boundary_and_clock_rollback_rule() {
        let day = 24 * 60 * 60 * 1000;
        assert!(!should_check(
            Trigger::Daily,
            true,
            Some(1000),
            1000 + day - 1,
            None
        ));
        assert!(should_check(
            Trigger::Daily,
            true,
            Some(1000),
            1000 + day,
            None
        ));
        assert!(should_check(Trigger::Daily, true, Some(1000), 999, None));
        assert!(should_check(Trigger::Daily, true, None, 1000, None));
    }
}
