//! Alibaba Cloud's Coding Plan (Model Studio / Bailian): a five-hour, a
//! weekly and a billing-month allowance, each reported as an amount used out
//! of an amount the plan grants.
//!
//! Read with the plan's API key, sent to the console route the Model Studio
//! page itself asks: `POST
//! /data/api.json?action=…queryCodingPlanInstanceInfoV2` on the international
//! console first and, when that one does not know the key or the plan, on the
//! China mainland console. Nothing is chosen in Settings: the key belongs to
//! one site, and whichever site answers for it is the one read. The shape is
//! second-hand — taken from CodexBar's Alibaba provider and its tests, not
//! from a captured reply — and the fixture in the tests says so.
//!
//! **Only figures the console states.** A window needs both its used amount
//! and its total; a missing or non-positive total leaves that window off
//! rather than drawn at zero. A reset time already in the past is dropped
//! rather than moved forward, which would be a guess.
//!
//! Some accounts get `ConsoleNeedLogin` for a key: the route wants a console
//! session instead. That is said as a refused key, the nearest shared reason,
//! and is not retried on the other site, which would refuse it the same way.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// One of the two consoles the plan is sold on. The key is sent to the
/// console's own host and nowhere else.
pub struct Site {
    pub origin: &'static str,
    pub region_id: &'static str,
    pub commodity_code: &'static str,
    pub page: &'static str,
}

impl Site {
    pub const INTERNATIONAL: Site = Site {
        origin: "https://modelstudio.console.alibabacloud.com",
        region_id: "ap-southeast-1",
        commodity_code: "sfm_codingplan_public_intl",
        page: "https://modelstudio.console.alibabacloud.com/ap-southeast-1/?tab=coding-plan#/efm/coding_plan",
    };
    pub const CHINA_MAINLAND: Site = Site {
        origin: "https://bailian.console.aliyun.com",
        region_id: "cn-beijing",
        commodity_code: "sfm_codingplan_public_cn",
        page: "https://bailian.console.aliyun.com/cn-beijing/?tab=model#/efm/coding_plan",
    };
}

/// The console gateway route the Model Studio page itself posts to.
pub fn endpoint(site: &Site) -> String {
    format!(
        "{}/data/api.json\
         ?action=zeldaEasy.broadscope-bailian.codingPlan.queryCodingPlanInstanceInfoV2\
         &product=broadscope-bailian\
         &api=queryCodingPlanInstanceInfoV2\
         &currentRegionId={}",
        site.origin, site.region_id,
    )
}

/// What one site's 200 means: a reading, a reason worth asking the other
/// site about, or a reason that ends it.
#[derive(Debug, PartialEq)]
pub enum Answer {
    Usage {
        windows: Vec<UsageWindow>,
        plan: Option<String>,
    },
    AskOtherSite(Unavailability),
    Final(Unavailability),
}

pub struct AlibabaCodingPlanService {
    http: Arc<HttpClient>,
}

impl AlibabaCodingPlanService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    fn usage(
        &self,
        account: AccountKey,
        windows: Vec<UsageWindow>,
        plan: Option<String>,
    ) -> ProviderUsage {
        let mut usage = ProviderUsage::live_now(account, windows);
        usage.origin = Some(self.origin_token().into());
        usage.plan = plan;
        usage
    }

    fn ask(&self, site: &Site, key: &str, now_ms: i64) -> Answer {
        let body = serde_json::json!({
            "queryCodingPlanInstanceInfoRequest": {"commodityCode": site.commodity_code},
        });
        // The three places the console looks for a key, as the page's own
        // request carries it.
        let response = match self
            .http
            .client_for_login()
            .post(endpoint(site))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("Authorization", format!("Bearer {key}"))
            .header("x-api-key", key)
            .header("X-DashScope-API-Key", key)
            .header("Origin", site.origin)
            .header("Referer", site.page)
            .timeout(Duration::from_secs(15))
            .json(&body)
            .send()
        {
            Ok(response) => response,
            Err(_) => return Answer::Final(Unavailability::Unreachable),
        };
        // The status is read directly: whether a refusal probes the other
        // site or ends the fetch is this provider's own branch.
        let status = crate::http::status_of(&response);
        match status {
            200..=299 => {
                let text = match response.text() {
                    Ok(text) => text,
                    Err(_) => return Answer::Final(Unavailability::UnreadableReply),
                };
                let tree: Value = match serde_json::from_str(&text) {
                    Ok(tree) => tree,
                    Err(_) => return Answer::Final(Unavailability::UnreadableReply),
                };
                answer(&tree, now_ms)
            }
            // Not this site's key, or not this site's route: the other one
            // may know it.
            401 | 403 => Answer::AskOtherSite(Unavailability::ApiKeyRefused),
            404 => Answer::AskOtherSite(Unavailability::ServerError),
            // A redirect is never followed, and the one this route sends is
            // to its sign-in page.
            300..=399 => Answer::Final(Unavailability::ApiKeyRefused),
            429 => Answer::Final(Unavailability::RateLimited),
            _ => Answer::Final(Unavailability::ServerError),
        }
    }
}

impl ProviderService for AlibabaCodingPlanService {
    fn provider(&self) -> Provider {
        Provider::AlibabaCodingPlan
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::AlibabaCodingPlan);
        let Some(key) = pasted_or_none(keys.api_key(Provider::AlibabaCodingPlan)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let now_ms = crate::timeutil::now_ms();

        let first = self.ask(&Site::INTERNATIONAL, &key, now_ms);
        let first_reason = match first {
            Answer::Usage { windows, plan } => return self.usage(account, windows, plan),
            Answer::Final(reason) => return ProviderUsage::unavailable(account, reason),
            Answer::AskOtherSite(reason) => reason,
        };

        match self.ask(&Site::CHINA_MAINLAND, &key, now_ms) {
            Answer::Usage { windows, plan } => self.usage(account, windows, plan),
            // A site that refused the key says less than one that took it
            // and found nothing, so the refusal is only reported when both
            // refused.
            Answer::Final(reason) | Answer::AskOtherSite(reason) => {
                let reason = if reason == Unavailability::ApiKeyRefused {
                    first_reason
                } else {
                    reason
                };
                ProviderUsage::unavailable(account, reason)
            }
        }
    }
}

/// The three allowances the console names, each with the keys it has used
/// for them. A billing month is not a stated length, so its thirty days are
/// a sort key only.
struct Allowance {
    id: &'static str,
    kind: Kind,
    seconds: i64,
    reports_length: bool,
    used: &'static [&'static str],
    total: &'static [&'static str],
    reset: &'static [&'static str],
}

const ALLOWANCES: [Allowance; 3] = [
    Allowance {
        id: "fiveHour",
        kind: Kind::FiveHour,
        seconds: 5 * 3_600,
        reports_length: true,
        used: &["per5HourUsedQuota", "perFiveHourUsedQuota"],
        total: &["per5HourTotalQuota", "perFiveHourTotalQuota"],
        reset: &[
            "per5HourQuotaNextRefreshTime",
            "perFiveHourQuotaNextRefreshTime",
        ],
    },
    Allowance {
        id: "weekly",
        kind: Kind::Weekly,
        seconds: 7 * 86_400,
        reports_length: true,
        used: &["perWeekUsedQuota"],
        total: &["perWeekTotalQuota"],
        reset: &["perWeekQuotaNextRefreshTime"],
    },
    Allowance {
        id: "monthly",
        kind: Kind::Monthly,
        seconds: 30 * 86_400,
        reports_length: false,
        used: &["perBillMonthUsedQuota", "perMonthUsedQuota"],
        total: &["perBillMonthTotalQuota", "perMonthTotalQuota"],
        reset: &[
            "perBillMonthQuotaNextRefreshTime",
            "perMonthQuotaNextRefreshTime",
        ],
    },
];

/// The figures that mark an object as the quota holder, whatever the field
/// names around them.
const QUOTA_KEYS: [&str; 6] = [
    "per5HourUsedQuota",
    "per5HourTotalQuota",
    "perWeekUsedQuota",
    "perWeekTotalQuota",
    "perBillMonthUsedQuota",
    "perBillMonthTotalQuota",
];

/// The reading, for anything that only wants the result: a 200's tree in,
/// a live reading or the reason there is none, out.
pub fn reading(tree: &Value, account: AccountKey, now_ms: i64) -> ProviderUsage {
    match answer(tree, now_ms) {
        Answer::Usage { windows, plan } => {
            let mut usage = ProviderUsage::live_now(account, windows);
            usage.plan = plan;
            usage
        }
        Answer::AskOtherSite(reason) | Answer::Final(reason) => {
            ProviderUsage::unavailable(account, reason)
        }
    }
}

pub fn answer(tree: &Value, now_ms: i64) -> Answer {
    // The tree is expanded first: the console sometimes carries the real
    // reply as a JSON string inside a field.
    let tree = expanded(tree.clone());
    if let Some(refusal) = refusal(&tree) {
        return refusal;
    }
    if !tree.is_object() {
        return Answer::Final(Unavailability::UnreadableReply);
    }

    let instances: Option<Vec<&Value>> = first_value(
        &["codingPlanInstanceInfos", "coding_plan_instance_infos"],
        &tree,
        true,
    )
    .and_then(|found| found.as_array())
    .map(|items| items.iter().filter(|item| item.is_object()).collect());
    let empty: Vec<&Value> = Vec::new();
    let list: &[&Value] = instances.as_deref().unwrap_or(&empty);

    // The instance that is running, or the first when none says either way.
    let chosen = active_instance(list, now_ms);
    let chosen_is_active = chosen
        .map(|instance| activity(instance, now_ms) > 0)
        .unwrap_or(false);

    // Several plans on one account: the active one's figures, and never an
    // expired one's standing in for them.
    let quota = match chosen {
        Some(instance) => match quota_info(instance) {
            Some(own) => Some(own),
            None => {
                if list.len() > 1 && chosen_is_active {
                    None
                } else {
                    quota_info(&tree)
                }
            }
        },
        None => quota_info(&tree),
    };

    let windows = quota
        .map(|quota| allowance_windows(quota, now_ms))
        .unwrap_or_default();
    if windows.is_empty() {
        // A list of plans that is empty, or holds only lapsed ones, is the
        // console saying there is no plan; anything else is figures missing.
        let reason = if instances
            .as_ref()
            .is_some_and(|items| items.iter().all(|item| activity(item, now_ms) < 0))
        {
            Unavailability::NoPlan
        } else {
            Unavailability::NoLimitsReported
        };
        return Answer::AskOtherSite(reason);
    }

    let plan = chosen
        .and_then(instance_plan_name)
        .or_else(|| plan_name(&tree));
    Answer::Usage { windows, plan }
}

/// A failure the console reports inside a 200.
pub fn refusal(tree: &Value) -> Option<Answer> {
    let message = first_string(&["statusMessage", "status_msg", "message", "msg"], tree)
        .unwrap_or_default()
        .to_lowercase();

    if let Some(status) = first_int(&["statusCode", "status_code", "code"], tree) {
        if status != 0 && status != 200 {
            if status == 401
                || status == 403
                || message.contains("api key")
                || message.contains("unauthorized")
            {
                return Some(Answer::AskOtherSite(Unavailability::ApiKeyRefused));
            }
            return Some(Answer::Final(Unavailability::ServerError));
        }
    }

    // `ConsoleNeedLogin`: this route wants a signed-in console, not a key.
    let code = first_string(&["code", "status", "statusCode"], tree)
        .unwrap_or_default()
        .to_lowercase();
    if code.contains("login") || message.contains("login") || message.contains("log in") {
        return Some(Answer::Final(Unavailability::ApiKeyRefused));
    }
    None
}

fn allowance_windows(quota: &serde_json::Map<String, Value>, now_ms: i64) -> Vec<UsageWindow> {
    ALLOWANCES
        .iter()
        .filter_map(|allowance| {
            let used = first_of(allowance.used, quota)
                .and_then(console_number)
                .filter(|used| *used >= 0.0)?;
            let total = first_of(allowance.total, quota)
                .and_then(console_number)
                .filter(|total| *total > 0.0)?;
            // A reset already gone by is dropped, not pushed forward —
            // moving it would be a guess.
            let resets_at = first_of(allowance.reset, quota)
                .and_then(console_date)
                .filter(|at| *at > now_ms);
            let mut window = UsageWindow::new(
                &format!("alibabaCodingPlan.{}", allowance.id),
                allowance.kind,
                None,
                used / total,
                allowance.seconds,
                resets_at,
            );
            window.reports_length = allowance.reports_length;
            window.is_exhausted = used >= total;
            Some(window)
        })
        .collect()
}

/// The first of the named keys present, checked directly on the quota
/// object — the allowance keys are never nested deeper.
fn first_of<'a>(keys: &[&str], quota: &'a serde_json::Map<String, Value>) -> Option<&'a Value> {
    keys.iter().find_map(|key| quota.get(*key))
}

/// The quota object beside an instance, or — searching the whole tree — the
/// one object that carries the allowance figures. The search does not
/// descend into arrays, where another plan's quota could be waiting.
fn quota_info(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    if let Some(named) = first_value(
        &["codingPlanQuotaInfo", "coding_plan_quota_info"],
        value,
        false,
    )
    .and_then(|found| found.as_object())
    {
        return Some(named);
    }
    first_object(value, false, &mut |object| {
        QUOTA_KEYS.iter().any(|key| object.contains_key(*key))
    })
}

/// The instance that is running, or the first when none says either way.
fn active_instance<'a>(instances: &[&'a Value], now_ms: i64) -> Option<&'a Value> {
    let mut best: Option<&'a Value> = None;
    let mut best_activity = i64::MIN;
    for instance in instances {
        let score = activity(instance, now_ms);
        if score > best_activity {
            best_activity = score;
            best = Some(instance);
        }
    }
    match best {
        Some(best) if best_activity > 0 => Some(best),
        // Nothing claims to be running: the first plan stands rather than
        // none, so a lapsed plan's name still reads.
        Some(_) => instances.first().copied(),
        None => None,
    }
}

/// How sure the instance is about being active: stated outright, implied by
/// an end date still ahead, or said to have lapsed.
fn activity(instance: &Value, now_ms: i64) -> i64 {
    let Some(map) = instance.as_object() else {
        return 0;
    };
    let status = map
        .get("status")
        .filter(|value| !value.is_null())
        .or_else(|| map.get("instanceStatus"))
        .and_then(console_string)
        .map(str::to_uppercase);
    if let Some(status) = status {
        if status == "VALID" || status == "ACTIVE" {
            return 3;
        }
        if [
            "EXPIRED",
            "INVALID",
            "INACTIVE",
            "DISABLED",
            "TERMINATED",
            "STOPPED",
        ]
        .contains(&status.as_str())
        {
            return -1;
        }
    }
    if let Some(active) = map
        .get("isActive")
        .filter(|value| !value.is_null())
        .or_else(|| map.get("active"))
        .and_then(|value| value.as_bool())
    {
        return if active { 3 } else { -1 };
    }
    let end = ["endTime", "periodEndTime", "expireTime", "expirationTime"]
        .iter()
        .find_map(|key| map.get(*key).and_then(console_date));
    match end {
        Some(at) if at > now_ms => 1,
        _ => 0,
    }
}

fn instance_plan_name(instance: &Value) -> Option<String> {
    let map = instance.as_object()?;
    [
        "planName",
        "plan_name",
        "instanceName",
        "instance_name",
        "packageName",
        "package_name",
    ]
    .iter()
    .find_map(|key| map.get(*key).and_then(console_string))
    .map(str::to_string)
}

fn plan_name(tree: &Value) -> Option<String> {
    first_string(
        &["planName", "plan_name", "packageName", "package_name"],
        tree,
    )
    .map(str::to_string)
}

// MARK: - The console's JSON, unwrapped

/// The console replies nest, and sometimes carry the real reply as a JSON
/// string inside a field. Unwrapped here so a reading sees one tree.
pub fn expanded(value: Value) -> Value {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                match serde_json::from_str::<Value>(trimmed) {
                    Ok(inner) => expanded(inner),
                    Err(_) => Value::String(text),
                }
            } else {
                Value::String(text)
            }
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, expanded(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(expanded).collect()),
        other => other,
    }
}

/// The first object, the outer one before what it holds, that `matches`.
fn first_object<'a, F>(
    value: &'a Value,
    into_arrays: bool,
    matches: &mut F,
) -> Option<&'a serde_json::Map<String, Value>>
where
    F: FnMut(&'a serde_json::Map<String, Value>) -> bool,
{
    if let Value::Object(map) = value {
        if matches(map) {
            return Some(map);
        }
        for nested in map.values() {
            if let Some(found) = first_object(nested, into_arrays, matches) {
                return Some(found);
            }
        }
    } else if into_arrays {
        if let Value::Array(items) = value {
            for nested in items {
                if let Some(found) = first_object(nested, into_arrays, matches) {
                    return Some(found);
                }
            }
        }
    }
    None
}

fn first_value<'a>(keys: &[&str], value: &'a Value, into_arrays: bool) -> Option<&'a Value> {
    let mut hit: Option<&'a Value> = None;
    first_object(value, into_arrays, &mut |object| {
        for key in keys {
            if let Some(found) = object.get(*key) {
                hit = Some(found);
                return true;
            }
        }
        false
    });
    hit
}

fn first_int(keys: &[&str], value: &Value) -> Option<i64> {
    let mut hit: Option<i64> = None;
    first_object(value, true, &mut |object| {
        for key in keys {
            if let Some(number) = object.get(*key).and_then(console_number) {
                if let Some(exact) = exact_int(number) {
                    hit = Some(exact);
                    return true;
                }
            }
        }
        false
    });
    hit
}

fn first_string<'a>(keys: &[&str], value: &'a Value) -> Option<&'a str> {
    let mut hit: Option<&'a str> = None;
    first_object(value, true, &mut |object| {
        for key in keys {
            if let Some(text) = object.get(*key).and_then(console_string) {
                hit = Some(text);
                return true;
            }
        }
        false
    });
    hit
}

fn exact_int(number: f64) -> Option<i64> {
    if number.is_finite()
        && number.fract() == 0.0
        && (i64::MIN as f64..=i64::MAX as f64).contains(&number)
    {
        Some(number as i64)
    } else {
        None
    }
}

/// A number, or a string that is one. Never a boolean, which a lenient
/// reader would otherwise take as 0 or 1.
pub fn console_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64().filter(|n| n.is_finite()),
        Value::String(text) => text.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        _ => None,
    }
}

pub fn console_string(value: &Value) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// Epoch seconds or milliseconds, ISO 8601, or the console's own
/// `yyyy-MM-dd HH:mm[:ss]`.
pub fn console_date(value: &Value) -> Option<i64> {
    if let Some(stamp) = console_number(value).filter(|stamp| *stamp > 0.0) {
        // Upstream's seconds/millisecond threshold is 1e12, stricter than
        // the shared helper's, so this keeps its own arithmetic.
        return Some(if stamp >= 1_000_000_000_000.0 {
            stamp as i64
        } else {
            (stamp * 1000.0) as i64
        });
    }
    let text = console_string(value)?;
    if let Some(ms) = crate::timeutil::parse_iso8601_ms(text) {
        return Some(ms);
    }
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(text, format) {
            return local_ms(naive);
        }
    }
    let date = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
    local_ms(date.and_hms_opt(0, 0, 0)?)
}

/// The console's date-only and no-zone stamps read the way the device reads
/// them — in the local zone, which is how Swift's date formatter defaults.
fn local_ms(naive: chrono::NaiveDateTime) -> Option<i64> {
    use chrono::{Local, TimeZone};
    Local
        .from_local_datetime(&naive)
        .single()
        .or_else(|| Local.from_local_datetime(&naive).earliest())
        .map(|time| time.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    const NOW: i64 = 1_790_812_800_000; // 2026-10-01T00:00:00Z

    fn tree(quota: serde_json::Value) -> Value {
        json!({
            "success": true,
            "codingPlanQuotaInfo": quota,
        })
    }

    #[test]
    fn the_three_allowances_read_from_the_named_keys() {
        let answer = answer(
            &tree(json!({
                "per5HourUsedQuota": 10, "per5HourTotalQuota": 100,
                "per5HourQuotaNextRefreshTime": NOW / 1000 + 3600,
                "perWeekUsedQuota": 30, "perWeekTotalQuota": 200,
                "perWeekQuotaNextRefreshTime": NOW / 1000 + 86_400,
                "perBillMonthUsedQuota": 5, "perBillMonthTotalQuota": 1000,
                "perBillMonthQuotaNextRefreshTime": NOW / 1000 + 86_400,
            })),
            NOW,
        );
        let Answer::Usage { windows, plan } = answer else {
            panic!("expected a usage, got {answer:?}");
        };
        assert_eq!(plan, None);
        let five_hour = &windows[0];
        assert_eq!(five_hour.id, "alibabaCodingPlan.fiveHour");
        assert_eq!(five_hour.kind, Kind::FiveHour);
        assert_eq!(five_hour.window_seconds, 5 * 3_600);
        assert!(five_hour.reports_length);
        assert_eq!(five_hour.resets_at, Some(NOW + 3_600_000));
        let weekly = &windows[1];
        assert_eq!(weekly.id, "alibabaCodingPlan.weekly");
        assert_eq!(weekly.kind, Kind::Weekly);
        // A billing month is not a stated length.
        let monthly = &windows[2];
        assert_eq!(monthly.id, "alibabaCodingPlan.monthly");
        assert_eq!(monthly.kind, Kind::Monthly);
        assert!(!monthly.reports_length);
    }

    #[test]
    fn figures_arrive_as_strings_and_booleans_are_not_numbers() {
        let answer = answer(
            &tree(json!({
                "per5HourUsedQuota": "10", "per5HourTotalQuota": "100",
                "perWeekUsedQuota": true, "perWeekTotalQuota": 200,
            })),
            NOW,
        );
        let Answer::Usage { windows, .. } = answer else {
            panic!("expected a usage");
        };
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "alibabaCodingPlan.fiveHour");
    }

    #[test]
    fn a_window_without_both_figures_is_left_off_not_drawn_at_zero() {
        // One allowance has a used and no total, the next a total and no
        // used: neither is drawn, and with nothing left the console is
        // asked on the other site.
        let answer = answer(
            &tree(json!({
                "per5HourUsedQuota": 10,
                "perWeekTotalQuota": 200,
            })),
            NOW,
        );
        assert_eq!(
            answer,
            Answer::AskOtherSite(Unavailability::NoLimitsReported)
        );
    }

    #[test]
    fn a_reset_already_gone_by_is_dropped() {
        let answer = answer(
            &tree(json!({
                "per5HourUsedQuota": 10, "per5HourTotalQuota": 100,
                "per5HourQuotaNextRefreshTime": NOW / 1000 - 60,
            })),
            NOW,
        );
        let Answer::Usage { windows, .. } = answer else {
            panic!("expected a usage");
        };
        assert_eq!(windows[0].resets_at, None);
    }

    #[test]
    fn a_spent_allowance_is_exhausted() {
        let answer = answer(
            &tree(json!({"per5HourUsedQuota": 100, "per5HourTotalQuota": 100})),
            NOW,
        );
        let Answer::Usage { windows, .. } = answer else {
            panic!("expected a usage");
        };
        assert!(windows[0].is_exhausted);
    }

    #[test]
    fn refusals_inside_a_200() {
        // A status the console answers for itself, other than success.
        assert_eq!(
            refusal(&json!({"statusCode": 401})),
            Some(Answer::AskOtherSite(Unavailability::ApiKeyRefused))
        );
        assert_eq!(
            refusal(&json!({"statusCode": 500})),
            Some(Answer::Final(Unavailability::ServerError))
        );
        assert_eq!(
            refusal(&json!({"statusCode": 400, "statusMessage": "Invalid API key provided"})),
            Some(Answer::AskOtherSite(Unavailability::ApiKeyRefused))
        );
        // The "api key" wording only matters beside a failed status; without
        // one the message is not read as a refusal.
        assert_eq!(
            refusal(&json!({"statusMessage": "Invalid ApiKey provided"})),
            None
        );
        // `ConsoleNeedLogin`: a console session, not a key, is wanted — and
        // the other site would refuse it the same way.
        assert_eq!(
            refusal(&json!({"code": "ConsoleNeedLogin"})),
            Some(Answer::Final(Unavailability::ApiKeyRefused))
        );
        // Success-shaped frames are not refusals.
        assert_eq!(refusal(&json!({"statusCode": 200})), None);
        assert_eq!(refusal(&json!({"statusCode": 0})), None);
        assert_eq!(refusal(&json!({"code": "OK", "message": "success"})), None);
    }

    #[test]
    fn the_active_instance_lends_its_own_quota() {
        let answer = answer(
            &json!({
                "codingPlanInstanceInfos": [
                    {"status": "EXPIRED", "planName": "Old Plan",
                     "codingPlanQuotaInfo": {"per5HourUsedQuota": 99, "per5HourTotalQuota": 100}},
                    {"status": "VALID", "planName": "New Plan",
                     "codingPlanQuotaInfo": {"per5HourUsedQuota": 10, "per5HourTotalQuota": 100}},
                ],
            }),
            NOW,
        );
        let Answer::Usage { windows, plan } = answer else {
            panic!("expected a usage, got {answer:?}");
        };
        assert_eq!(plan.as_deref(), Some("New Plan"));
        assert!((windows[0].used_fraction - 0.1).abs() < 1e-9);
    }

    #[test]
    fn only_lapsed_or_empty_plan_lists_say_there_is_no_plan() {
        for reply in [
            json!({"codingPlanInstanceInfos": [{"status": "EXPIRED"}, {"status": "STOPPED"}]}),
            json!({"codingPlanInstanceInfos": []}),
        ] {
            assert_eq!(
                answer(&reply, NOW),
                Answer::AskOtherSite(Unavailability::NoPlan)
            );
        }
    }

    #[test]
    fn a_lapsed_list_with_figures_beside_it_still_reads() {
        // The chosen (first) instance is lapsed and carries no quota of its
        // own, so the tree's quota stands rather than nothing.
        let answer = answer(
            &json!({
                "codingPlanInstanceInfos": [{"status": "EXPIRED"}],
                "codingPlanQuotaInfo": {"per5HourUsedQuota": 1, "per5HourTotalQuota": 2},
            }),
            NOW,
        );
        let Answer::Usage { windows, .. } = answer else {
            panic!("expected a usage, got {answer:?}");
        };
        assert_eq!(windows.len(), 1);
    }

    #[test]
    fn one_instance_falls_back_to_the_trees_quota() {
        let answer = answer(
            &json!({
                "codingPlanInstanceInfos": [{"status": "VALID", "planName": "New Plan"}],
                "codingPlanQuotaInfo": {"per5HourUsedQuota": 1, "per5HourTotalQuota": 2},
            }),
            NOW,
        );
        let Answer::Usage { windows, plan } = answer else {
            panic!("expected a usage, got {answer:?}");
        };
        assert_eq!(plan.as_deref(), Some("New Plan"));
        assert_eq!(windows.len(), 1);
    }

    #[test]
    fn nothing_at_all_asks_the_other_site() {
        assert_eq!(
            answer(&json!({"success": true}), NOW),
            Answer::AskOtherSite(Unavailability::NoLimitsReported)
        );
    }

    #[test]
    fn the_reading_maps_answers_onto_one_provider_usage() {
        let usage = reading(
            &tree(json!({"per5HourUsedQuota": 1, "per5HourTotalQuota": 2})),
            AccountKey::primary(Provider::AlibabaCodingPlan),
            NOW,
        );
        assert!(matches!(usage.state, crate::model::State::Live));
        assert_eq!(usage.windows.len(), 1);

        let usage = reading(
            &json!({"statusCode": 500}),
            AccountKey::primary(Provider::AlibabaCodingPlan),
            NOW,
        );
        assert!(matches!(
            usage.state,
            crate::model::State::Unavailable(Unavailability::ServerError)
        ));
    }

    #[test]
    fn nested_json_strings_are_unwrapped() {
        let inner = json!({
            "statusCode": 0,
            "codingPlanQuotaInfo": {"per5HourUsedQuota": 1, "per5HourTotalQuota": 2},
        });
        // The console sometimes delivers the reply as a string in a field.
        let wrapped = json!({"data": inner.to_string()});
        let Answer::Usage { windows, .. } = answer(&wrapped, NOW) else {
            panic!("expected the wrapped reply to read");
        };
        assert_eq!(windows.len(), 1);
        // A string that is not JSON stays a string.
        let kept = expanded(Value::String("just text".into()));
        assert_eq!(kept, Value::String("just text".into()));
    }

    #[test]
    fn a_non_object_reply_is_unreadable() {
        assert_eq!(
            answer(&json!([1, 2, 3]), NOW),
            Answer::Final(Unavailability::UnreadableReply)
        );
    }

    #[test]
    fn dates_read_in_every_shape_the_console_uses() {
        assert_eq!(console_date(&json!(NOW / 1000)), Some(NOW));
        assert_eq!(console_date(&json!(NOW)), Some(NOW));
        assert_eq!(console_date(&json!("2026-10-01T00:00:00Z")), Some(NOW));
        // The console's own no-zone stamps, read in the local zone.
        let local = console_date(&json!("2026-10-01 00:00:00")).unwrap();
        let naive = chrono::Local
            .from_local_datetime(
                &chrono::NaiveDateTime::parse_from_str("2026-10-01 00:00:00", "%Y-%m-%d %H:%M:%S")
                    .unwrap(),
            )
            .single()
            .unwrap();
        assert_eq!(local, naive.timestamp_millis());
        assert_eq!(console_date(&json!(0)), None);
        assert_eq!(console_date(&json!("not a date")), None);
    }

    #[test]
    fn search_finds_the_first_object_carrying_the_keys() {
        let value = json!({"a": {"codingPlanQuotaInfo": {"per5HourUsedQuota": 1}}});
        assert_eq!(
            first_value(&["codingPlanQuotaInfo"], &value, true),
            Some(&json!({"per5HourUsedQuota": 1}))
        );
        // Without arrays in the search, an object hidden inside one is out
        // of reach.
        let hidden = json!({"items": [{"codingPlanQuotaInfo": {"per5HourUsedQuota": 2}}]});
        assert!(first_value(&["codingPlanQuotaInfo"], &hidden, true).is_some());
        assert!(first_value(&["codingPlanQuotaInfo"], &hidden, false).is_none());
        assert_eq!(first_int(&["code"], &json!({"code": 401})), Some(401));
        assert_eq!(first_int(&["code"], &json!({"code": 401.5})), None);
        assert_eq!(
            first_string(&["msg"], &json!({"msg": " go away "})),
            Some("go away")
        );
    }

    #[test]
    fn the_endpoints_name_each_console() {
        let international = endpoint(&Site::INTERNATIONAL);
        assert!(international
            .starts_with("https://modelstudio.console.alibabacloud.com/data/api.json?"));
        assert!(international.contains("currentRegionId=ap-southeast-1"));
        let mainland = endpoint(&Site::CHINA_MAINLAND);
        assert!(mainland.starts_with("https://bailian.console.aliyun.com/data/api.json?"));
        assert!(mainland.contains("currentRegionId=cn-beijing"));
    }
}
