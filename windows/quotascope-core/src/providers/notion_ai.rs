//! Notion AI's usage allowance on a Business or Enterprise workspace: a
//! rolling window (six hours when this was written) and the billing period,
//! each reported as credits used out of a limit.
//!
//! Read with the browser session Notion's web app uses — the `token_v2`
//! cookie — from the two internal endpoints its own Settings → Notion AI →
//! Usage page calls, both on `app.notion.com`: `POST /api/v3/getSpaces` for
//! the workspaces this account can see, then
//! `POST /api/v3/getCreditRateLimitStatus` for the chosen one. Neither is a
//! public API. The shapes are second-hand — taken from CodexBar's Notion
//! provider and its fixtures, not from a captured reply.
//!
//! **Which workspace.** There is no picker for one, so the first workspace
//! on a Business or Enterprise plan is taken, or the first there is. A plan
//! without an allowance answers `not_applicable`, which is "no plan with
//! usage limits" — an answer, not an outage.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::{HttpClient, HttpFailure, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde::Deserialize;
use std::sync::Arc;

/// The session comes from `notion.com`; `token_v2` is the session, and
/// without it every call is a 401.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["notion.com"],
    cookies: &["token_v2"],
};

const BASE: &str = "https://app.notion.com";

pub struct NotionAiService {
    http: Arc<HttpClient>,
}

impl NotionAiService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    fn post(
        &self,
        method: &str,
        body: serde_json::Value,
        cookie: &str,
    ) -> Result<serde_json::Value, Unavailability> {
        let headers = [
            ("Cookie", cookie.to_string()),
            ("Accept", "application/json".to_string()),
            ("Origin", BASE.to_string()),
            ("Referer", format!("{BASE}/")),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        session_json(
            &self.http,
            Method::Post,
            &format!("{BASE}/api/v3/{method}"),
            &refs,
            Some(&body),
        )
    }
}

impl ProviderService for NotionAiService {
    fn provider(&self) -> Provider {
        Provider::NotionAi
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::NotionAi);
        let Some(cookie) = pasted_or_none(keys.api_key(Provider::NotionAi)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };

        let spaces_reply = match self.post("getSpaces", serde_json::json!({}), &cookie) {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let Some(spaces) = workspaces(&spaces_reply) else {
            return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
        };
        let Some(workspace) = choose(&spaces) else {
            // A workspace list with no plan that carries an allowance is the
            // site saying there is nothing to report. Upstream says
            // `.noPlan`; the shared vocabulary lands it on "no limits".
            return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
        };

        let status_reply = match self.post(
            "getCreditRateLimitStatus",
            serde_json::json!({ "spaceId": workspace.id }),
            &cookie,
        ) {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        match reading(&status_reply, crate::timeutil::now_ms()) {
            Ok(windows) => {
                let mut usage = ProviderUsage::live_now(account, windows);
                usage.origin = Some(self.origin_token().into());
                usage.plan = workspace.tier();
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// One call with the imported session as its credential. `fetch_json` folds
/// 401, 403 and the unfollowed redirect into `ApiKeyRefused`, which is the
/// pasted key's refusal — for a session those statuses are the session
/// expiring, so the folded reason is read here and remapped. A request that
/// sends no key can arrive at that folding no other way. 404 lands in
/// `NotFound`; upstream's classifier calls it a server error like any other.
fn session_json(
    http: &HttpClient,
    method: Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, Unavailability> {
    match http.fetch_json_detailed(method, url, headers, body) {
        Ok(value) => Ok(value),
        Err(HttpFailure::NotFound) => Err(Unavailability::ServerError),
        Err(HttpFailure::Unavailable(Unavailability::ApiKeyRefused)) => {
            Err(Unavailability::SessionExpired)
        }
        Err(HttpFailure::Unavailable(reason)) => Err(reason),
    }
}

/// A workspace `getSpaces` names: its id, and the plan as Notion spells it —
/// "business", "enterprise", "free".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    pub subscription_tier: Option<String>,
}

impl Workspace {
    /// Whether the plan carries an AI allowance at all.
    pub fn has_allowance(&self) -> bool {
        self.subscription_tier
            .as_deref()
            .map(|tier| tier.to_ascii_lowercase())
            .is_some_and(|tier| tier == "business" || tier == "enterprise")
    }

    /// The plan's name for the card, capitalised as Notion's own page does.
    pub fn tier(&self) -> Option<String> {
        let raw = self
            .subscription_tier
            .as_deref()
            .map(str::trim)
            .filter(|tier| !tier.is_empty())?;
        let mut chars = raw.chars();
        match chars.next() {
            Some(first) => Some(first.to_uppercase().collect::<String>() + chars.as_str()),
            None => Some(String::new()),
        }
    }
}

/// The workspaces of the one account the reply names, in key order. Only a
/// reply that names exactly one user is read: taking whichever key came
/// first could report another account's allowance. `None` when it cannot be
/// read; empty when it names the user and no workspace.
pub fn workspaces(reply: &serde_json::Value) -> Option<Vec<Workspace>> {
    let root = reply.as_object()?;
    let identified: Vec<&String> = root.keys().filter(|key| names_self(root, key)).collect();
    let user_id = if identified.len() == 1 {
        identified[0].clone()
    } else if identified.is_empty() && root.len() == 1 {
        // Older replies leave the id out of the record; one key is still one
        // user.
        root.keys().next()?.clone()
    } else {
        return None;
    };
    let container = root.get(&user_id)?.as_object()?;

    let empty = serde_json::Map::new();
    let spaces = container.get("space").and_then(|space| space.as_object());
    let spaces = spaces.unwrap_or(&empty);
    let mut keys: Vec<&String> = spaces.keys().collect();
    keys.sort();

    let mut found = Vec::new();
    for key in keys {
        let Some(fields) = spaces.get(key).and_then(record) else {
            continue;
        };
        found.push(Workspace {
            id: fields
                .get("id")
                .and_then(|id| id.as_str())
                .unwrap_or(key)
                .to_string(),
            subscription_tier: fields
                .get("subscription_tier")
                .and_then(|tier| tier.as_str())
                .map(str::to_string),
        });
    }
    Some(found)
}

/// Whether the user record under `key` names that same key — the mark that
/// this reply is about one known account.
fn names_self(root: &serde_json::Map<String, serde_json::Value>, key: &str) -> bool {
    let Some(users) = root
        .get(key)
        .and_then(|value| value.get("notion_user"))
        .and_then(|value| value.as_object())
    else {
        return false;
    };
    let Some(record) = users.get(key).and_then(record) else {
        return false;
    };
    record.get("id").and_then(|id| id.as_str()) == Some(key)
}

/// Records come as `{"value": {…}}` or, newer, `{"value": {"value": {…}}}`.
fn record(raw: &serde_json::Value) -> Option<&serde_json::Map<String, serde_json::Value>> {
    let outer = raw.as_object()?;
    let Some(value) = outer.get("value").and_then(|value| value.as_object()) else {
        return Some(outer);
    };
    Some(
        value
            .get("value")
            .and_then(|inner| inner.as_object())
            .unwrap_or(value),
    )
}

/// The first workspace on a Business or Enterprise plan, or the first there
/// is.
pub fn choose(workspaces: &[Workspace]) -> Option<&Workspace> {
    workspaces
        .iter()
        .find(|workspace| workspace.has_allowance())
        .or_else(|| workspaces.first())
}

#[derive(Deserialize)]
struct Reply {
    status: Option<String>,
    window: Option<Rolling>,
    #[serde(rename = "resetsInSeconds")]
    resets_in_seconds: Option<f64>,
    #[serde(rename = "billingPeriodWindow")]
    billing_period_window: Option<Billing>,
}

#[derive(Deserialize)]
struct Rolling {
    window: Option<String>,
    used: Option<f64>,
    limit: Option<f64>,
}

#[derive(Deserialize)]
struct Billing {
    used: Option<f64>,
    limit: Option<f64>,
    #[serde(rename = "periodEndMs")]
    period_end_ms: Option<f64>,
}

/// The rolling and billing windows, soonest first.
pub fn reading(reply: &serde_json::Value, now_ms: i64) -> Result<Vec<UsageWindow>, Unavailability> {
    let parsed: Reply =
        serde_json::from_value(reply.clone()).map_err(|_| Unavailability::UnreadableReply)?;
    if parsed
        .status
        .as_deref()
        .map(|status| status.to_ascii_lowercase())
        .as_deref()
        == Some("not_applicable")
    {
        // A plan without an allowance answers `not_applicable` — an answer,
        // not an outage. Upstream says `.noPlan`; the shared vocabulary has
        // no such case, so it lands on "no limits reported".
        return Err(Unavailability::NoLimitsReported);
    }
    // Every field is optional, so an unrelated body decodes as all nil.
    if parsed.window.is_none() && parsed.billing_period_window.is_none() {
        return Err(Unavailability::UnreadableReply);
    }

    let mut windows = Vec::new();
    if let Some(rolling) = &parsed.window {
        if let Some(used) = fraction(rolling.used, rolling.limit) {
            let reset = parsed
                .resets_in_seconds
                .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
                .map(|seconds| now_ms + (seconds * 1000.0) as i64);
            let (kind, seconds, stated) = length(rolling.window.as_deref());
            let mut window = UsageWindow::new("notion.rolling", kind, None, used, seconds, reset);
            window.reports_length = stated;
            window.is_exhausted = used >= 1.0;
            windows.push(window);
        }
    }
    if let Some(billing) = &parsed.billing_period_window {
        if let Some(used) = fraction(billing.used, billing.limit) {
            // A billing period is only as long as the month it falls in,
            // and Notion states only its end.
            let reset = billing
                .period_end_ms
                .filter(|ms| ms.is_finite() && *ms > 0.0)
                .map(|ms| ms as i64);
            let mut window = UsageWindow::new(
                "notion.billing",
                Kind::Monthly,
                None,
                used,
                30 * 86_400,
                reset,
            );
            window.reports_length = false;
            window.is_exhausted = used >= 1.0;
            windows.push(window);
        }
    }

    if windows.is_empty() {
        return Err(Unavailability::NoLimitsReported);
    }
    windows.sort_by_key(|window| window.window_seconds);
    Ok(windows)
}

/// Credits used over the stated limit. A missing or non-positive limit is
/// nothing to measure against, not 0%. Over the limit is kept as reported.
fn fraction(used: Option<f64>, limit: Option<f64>) -> Option<f64> {
    let used = used.filter(|used| used.is_finite() && *used >= 0.0)?;
    let limit = limit.filter(|limit| limit.is_finite() && *limit > 0.0)?;
    Some(used / limit)
}

/// Notion states the rolling window as a token — `6h`. A token that can't
/// be read leaves the allowance with no length claimed; upstream kinds that
/// fallback `.credits`, and Spend is the Windows stand-in for it.
pub fn length(token: Option<&str>) -> (Kind, i64, bool) {
    let fallback = (Kind::Spend, 86_400, false);
    let Some(raw) = token
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(|raw| raw.to_ascii_lowercase())
    else {
        return fallback;
    };
    let Some(unit) = raw.chars().last() else {
        return fallback;
    };
    let Ok(value) = raw[..raw.len() - unit.len_utf8()].parse::<i64>() else {
        return fallback;
    };
    if value <= 0 {
        return fallback;
    }
    let unit_seconds = match unit {
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        'w' => 7 * 86_400,
        _ => return fallback,
    };
    let seconds = value * unit_seconds;
    const FIVE_HOURS: i64 = 5 * 3_600;
    const WEEK: i64 = 7 * 86_400;
    match seconds {
        FIVE_HOURS => (Kind::FiveHour, seconds, true),
        // Upstream kinds a day `.daily`; the day rides as `other`, as
        // elsewhere in the port.
        86_400 => (Kind::Other(86_400), seconds, true),
        WEEK => (Kind::Weekly, seconds, true),
        other => (Kind::Other(other), seconds, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `getSpaces` for one user with two workspaces, in both record shapes.
    fn spaces_reply() -> serde_json::Value {
        serde_json::json!({
            "user-1": {
                "notion_user": {
                    "user-1": {"value": {"id": "user-1", "name": "Reader"}}
                },
                "space": {
                    "space-b": {"value": {"id": "space-b-id", "subscription_tier": "free"}},
                    "space-a": {"value": {"value": {"id": "space-a-id", "subscription_tier": "business"}}}
                }
            }
        })
    }

    #[test]
    fn the_reply_names_one_user_and_both_record_shapes_read() {
        let spaces = workspaces(&spaces_reply()).unwrap();
        // Keys sort, so "space-a" comes first whatever the reply's order.
        assert_eq!(spaces.len(), 2);
        assert_eq!(spaces[0].id, "space-a-id");
        assert_eq!(spaces[0].subscription_tier.as_deref(), Some("business"));
        assert_eq!(spaces[1].id, "space-b-id");
        assert_eq!(spaces[1].subscription_tier.as_deref(), Some("free"));
    }

    #[test]
    fn the_first_workspace_with_an_allowance_is_chosen() {
        let spaces = workspaces(&spaces_reply()).unwrap();
        assert_eq!(choose(&spaces).unwrap().id, "space-a-id");

        // Without a plan that carries one, the first there is.
        let free = vec![spaces[1].clone()];
        assert_eq!(choose(&free).unwrap().id, "space-b-id");
        assert_eq!(choose(&[]), None);
    }

    #[test]
    fn a_reply_about_two_users_is_not_read() {
        let mut reply = spaces_reply();
        reply["user-2"] = serde_json::json!({
            "notion_user": {"user-2": {"value": {"id": "user-2"}}},
            "space": {}
        });
        assert_eq!(workspaces(&reply), None);
    }

    #[test]
    fn an_older_reply_with_one_key_still_names_one_user() {
        let reply = serde_json::json!({
            "user-1": {"space": {"space-1": {"value": {"id": "space-1-id", "subscription_tier": "enterprise"}}}}
        });
        let spaces = workspaces(&reply).unwrap();
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].id, "space-1-id");
        assert!(spaces[0].has_allowance());
        // A workspace with no tier at all draws nothing and no plan.
        assert_eq!(spaces[0].tier().as_deref(), Some("Enterprise"));

        let bare = vec![Workspace {
            id: "s".into(),
            subscription_tier: None,
        }];
        assert!(!bare[0].has_allowance());
        assert_eq!(bare[0].tier(), None);
    }

    #[test]
    fn the_rolling_window_token_becomes_a_stated_length() {
        let cases: &[(&str, Kind, i64)] = &[
            ("6h", Kind::Other(6 * 3_600), 6 * 3_600),
            ("5h", Kind::FiveHour, 5 * 3_600),
            ("1d", Kind::Other(86_400), 86_400),
            ("1w", Kind::Weekly, 7 * 86_400),
            ("30m", Kind::Other(1_800), 1_800),
            ("6H", Kind::Other(6 * 3_600), 6 * 3_600),
        ];
        for (token, expected_kind, expected_seconds) in cases {
            let (kind, seconds, stated) = length(Some(token));
            assert_eq!(kind, *expected_kind, "{token}");
            assert_eq!(seconds, *expected_seconds, "{token}");
            assert!(stated, "{token}");
        }
        // A token that can't be read leaves no length claimed, and the day
        // it falls back to is a sort key.
        for token in [Some("soon"), Some("0h"), Some("1h30m"), Some(""), None] {
            let (kind, seconds, stated) = length(token);
            assert_eq!(kind, Kind::Spend, "{token:?}");
            assert_eq!(seconds, 86_400, "{token:?}");
            assert!(!stated, "{token:?}");
        }
    }

    #[test]
    fn rolling_and_billing_windows_sorted_by_length() {
        let reply = serde_json::json!({
            "window": {"window": "6h", "used": 30, "limit": 100},
            "resetsInSeconds": 3600,
            "billingPeriodWindow": {"used": 10, "limit": 100, "periodEndMs": 1_760_000_000_000i64}
        });
        let windows = reading(&reply, 0).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].id, "notion.rolling");
        assert_eq!(windows[0].kind, Kind::Other(6 * 3_600));
        assert_eq!(windows[0].used_fraction, 0.3);
        assert!(windows[0].reports_length);
        assert_eq!(windows[0].resets_at, Some(3_600_000));
        assert_eq!(windows[1].id, "notion.billing");
        assert_eq!(windows[1].kind, Kind::Monthly);
        assert_eq!(windows[1].used_fraction, 0.1);
        assert!(!windows[1].reports_length);
        assert_eq!(windows[1].resets_at, Some(1_760_000_000_000));
    }

    #[test]
    fn not_applicable_is_an_answer_not_an_outage() {
        assert_eq!(
            reading(&serde_json::json!({"status": "not_applicable"}), 0),
            Err(Unavailability::NoLimitsReported)
        );
    }

    #[test]
    fn an_unrelated_body_is_unreadable_not_empty() {
        for reply in [
            serde_json::json!({"foo": 1}),
            serde_json::json!([]),
            serde_json::json!({"window": "6h"}),
        ] {
            assert_eq!(
                reading(&reply, 0).map(|_| ()),
                Err(Unavailability::UnreadableReply)
            );
        }
    }

    #[test]
    fn over_the_limit_is_kept_and_a_negative_reset_is_ignored() {
        let reply = serde_json::json!({
            "window": {"window": "1h", "used": 120, "limit": 100},
            "resetsInSeconds": -5
        });
        let windows = reading(&reply, 0).unwrap();
        assert_eq!(windows[0].used_fraction, 1.2);
        assert!(windows[0].is_exhausted);
        assert_eq!(windows[0].resets_at, None);
    }

    #[test]
    fn no_stated_limit_is_nothing_to_measure_against() {
        for reply in [
            serde_json::json!({"window": {"window": "1h", "used": 10}}),
            serde_json::json!({"window": {"window": "1h", "used": 10, "limit": 0}}),
            serde_json::json!({"window": {"window": "1h", "used": -1, "limit": 100}}),
        ] {
            assert_eq!(
                reading(&reply, 0).map(|_| ()),
                Err(Unavailability::NoLimitsReported)
            );
        }
    }

    #[test]
    fn a_rolling_window_without_a_token_still_draws_with_no_length() {
        // The figures stand; only the length is left unclaimed.
        let reply = serde_json::json!({"window": {"used": 10, "limit": 100}});
        let windows = reading(&reply, 0).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].kind, Kind::Spend);
        assert!(!windows[0].reports_length);
        assert_eq!(windows[0].window_seconds, 86_400);
    }
}
