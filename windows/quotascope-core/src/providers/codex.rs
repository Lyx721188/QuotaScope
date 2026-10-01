//! Codex's usage, from `chatgpt.com/backend-api/wham/usage` — the call the
//! CLI's own client makes, with the OAuth credentials Codex stored in
//! `~/.codex/auth.json`.
//!
//! Missing/refused CLI OAuth falls back to Codex's own resident app-server.

use super::{KeyRing, ProviderService};
use crate::http::{number, object_field, string_field, HttpClient};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, State, Unavailability, UsageWindow};
use std::sync::Arc;

const ENDPOINT: &str = "https://chatgpt.com/backend-api/wham/usage";

pub struct CodexService {
    http: Arc<HttpClient>,
}

impl CodexService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        CodexService { http }
    }
}

impl ProviderService for CodexService {
    fn provider(&self) -> Provider {
        Provider::Codex
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, _keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Codex);
        let Some(credentials) = load_credentials() else {
            return fetch_app_server(account);
        };

        let mut headers: Vec<(String, String)> = vec![
            ("Authorization".into(), format!("Bearer {}", credentials.0)),
            ("Accept".into(), "application/json".into()),
        ];
        // Sent only when there is one. An empty header is not the same as no
        // header: it names no account, and the service is free to answer for
        // whichever it likes.
        if !credentials.1.is_empty() {
            headers.push(("ChatGPT-Account-Id".into(), credentials.1.clone()));
        }
        let header_refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let root =
            match self
                .http
                .fetch_json(crate::http::Method::Get, ENDPOINT, &header_refs, None)
            {
                Ok(v) => v,
                Err(Unavailability::ApiKeyRefused) => return fetch_app_server(account),
                Err(reason) => return ProviderUsage::unavailable(account, reason),
            };

        let mut usage = ProviderUsage::live_now(account, parse_usage_response(&root));
        usage.origin = Some(self.origin_token().to_string());
        if usage.windows.is_empty() {
            usage.state = State::Unavailable(Unavailability::NoLimitsReported);
        }
        usage
    }
}

fn fetch_app_server(account: AccountKey) -> ProviderUsage {
    match crate::codex_rpc::request("account/rateLimits/read") {
        Ok(root) => parse_app_server_response(&root),
        Err(error) => ProviderUsage::unavailable(
            account,
            match error {
                crate::codex_rpc::RpcError::Missing => Unavailability::CodexNotInstalled,
                crate::codex_rpc::RpcError::Refused => Unavailability::SignInRequired,
                _ => Unavailability::CodexServerFailed,
            },
        ),
    }
}

pub fn parse_app_server_response(root: &serde_json::Value) -> ProviderUsage {
    let fallback;
    let groups = if let Some(groups) = root
        .get("rateLimitsByLimitId")
        .and_then(serde_json::Value::as_object)
    {
        groups
    } else {
        fallback = root
            .get("rateLimits")
            .filter(|v| v.is_object())
            .map(|v| {
                [("codex".to_string(), v.clone())]
                    .into_iter()
                    .collect::<serde_json::Map<String, serde_json::Value>>()
            })
            .unwrap_or_default();
        &fallback
    };
    let mut ordered: Vec<_> = groups.iter().collect();
    ordered.sort_by_key(|(key, group)| {
        (
            group
                .get("limitName")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            key.as_str(),
        )
    });
    let ordinary_refused = root
        .get("ordinaryUsageAllowed")
        .and_then(serde_json::Value::as_bool)
        == Some(false);
    let mut windows = Vec::new();
    let mut plan = None;
    let mut credits = None;
    for (key, group) in ordered {
        let scope = string_field(group, "limitName").map(str::to_string);
        let group_windows = ["primary", "secondary"]
            .into_iter()
            .filter_map(|slot| {
                let node = group.get(slot)?;
                let percent = node
                    .get("usedPercent")
                    .and_then(number)
                    .filter(|p| *p >= 0.0)?;
                let seconds = node
                    .get("windowDurationMins")
                    .and_then(number)
                    .filter(|m| *m > 0.0 && *m <= i64::MAX as f64 / 60.0)
                    .map(|m| (m * 60.0) as i64);
                let resets = node
                    .get("resetsAt")
                    .and_then(number)
                    .map(crate::timeutil::epoch_to_ms);
                Some(UsageWindow::new(
                    &format!("{key}.{slot}"),
                    seconds.map(kind_from_seconds).unwrap_or(Kind::Other(0)),
                    scope.clone(),
                    percent / 100.0,
                    seconds.unwrap_or(0),
                    resets,
                ))
            })
            .collect();
        let spent = ordinary_refused
            || group
                .get("spendControlReached")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            || group
                .get("rateLimitReachedType")
                .is_some_and(|v| !v.is_null());
        windows.extend(marking_spent(group_windows, spent));
        plan = plan.or_else(|| string_field(group, "planType").map(str::to_string));
        if credits.is_none() {
            if let Some(node) = group.get("credits") {
                if node.get("unlimited").and_then(serde_json::Value::as_bool) != Some(true) {
                    credits = string_field(node, "balance").map(str::to_string);
                }
            }
        }
    }
    let mut usage = ProviderUsage::live_now(AccountKey::primary(Provider::Codex), windows);
    usage.plan = plan;
    usage.credit_balance = credits;
    usage.origin = Some("appServer".into());
    if usage.windows.is_empty() && usage.credit_balance.is_none() {
        usage.state = State::Unavailable(Unavailability::NoLimitsReported);
    }
    usage
}

/// (access token, account id)
fn load_credentials() -> Option<(String, String)> {
    let text = std::fs::read_to_string(crate::model::home_path(".codex/auth.json")).ok()?;
    let root: serde_json::Value = serde_json::from_str(&text).ok()?;
    let tokens = root.get("tokens")?;
    let access = string_field(tokens, "access_token")?;
    let account_id = string_field(tokens, "account_id").unwrap_or("");
    Some((access.to_string(), account_id.to_string()))
}

pub fn parse_usage_response(root: &serde_json::Value) -> Vec<UsageWindow> {
    let mut windows: Vec<UsageWindow> = Vec::new();

    let spend_reached = object_field(root, "spend_control")
        .and_then(|s| s.get("reached"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if let Some(limit) = object_field(root, "rate_limit") {
        let group_spent = is_group_spent(limit)
            || root
                .get("rate_limit_reached_type")
                .map(|v| !v.is_null())
                .unwrap_or(false)
            || spend_reached;
        windows.extend(marking_spent(
            http_windows(limit, "codex", None),
            group_spent,
        ));
    }

    for extra in crate::http::array_field(root, "additional_rate_limits") {
        let Some(limit) = object_field(extra, "rate_limit") else {
            continue;
        };
        let label = string_field(extra, "limit_name").map(|s| s.to_string());
        let key = string_field(extra, "metered_feature")
            .or(string_field(extra, "limit_name"))
            .unwrap_or("extra");
        windows.extend(marking_spent(
            http_windows(limit, key, label),
            is_group_spent(limit),
        ));
    }

    windows
}

/// `primary_window` and `secondary_window` are not tied to particular
/// durations, and which windows exist depends on the plan — so a window's
/// kind comes from its duration, never from which slot it arrived in.
fn http_windows(
    limit: &serde_json::Value,
    id_prefix: &str,
    scope: Option<String>,
) -> Vec<UsageWindow> {
    ["primary_window", "secondary_window"]
        .into_iter()
        .filter_map(|slot| {
            let node = object_field(limit, slot)?;
            let percent = node.get("used_percent").and_then(number)?;
            let seconds = node
                .get("limit_window_seconds")
                .and_then(number)
                .map(|s| s as i64);
            let resets = node
                .get("reset_at")
                .and_then(number)
                .map(crate::timeutil::epoch_to_ms);

            let mut window = UsageWindow::new(
                &format!("{id_prefix}.{slot}"),
                seconds.map(kind_from_seconds).unwrap_or(Kind::Other(0)),
                scope.clone(),
                percent / 100.0,
                seconds.unwrap_or(0),
                resets,
            );
            window.is_exhausted = false;
            Some(window)
        })
        .collect()
}

fn is_group_spent(limit: &serde_json::Value) -> bool {
    if limit.get("limit_reached").and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    if limit.get("allowed").and_then(|v| v.as_bool()) == Some(false) {
        return true;
    }
    false
}

/// Codex reports "limit reached" for a whole group, but a group can hold
/// both a 5-hour and a weekly window and only one of them is the reason.
/// Flagging the fullest one keeps the claim as precise as the data allows.
fn marking_spent(windows: Vec<UsageWindow>, spent: bool) -> Vec<UsageWindow> {
    if !spent {
        return windows;
    }
    let Some(fullest) = windows
        .iter()
        .max_by(|a, b| {
            a.used_fraction
                .partial_cmp(&b.used_fraction)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|w| w.id.clone())
    else {
        return windows;
    };
    windows
        .into_iter()
        .map(|mut w| {
            if w.id == fullest {
                w.is_exhausted = true;
            }
            w
        })
        .collect()
}

pub fn kind_from_seconds(seconds: i64) -> Kind {
    match seconds {
        18_000 => Kind::FiveHour,
        604_800 => Kind::Weekly,
        _ => Kind::Other(seconds),
    }
}

/// The API answers with an internal tier name — `prolite`, `plus` — which is
/// not the name on the plan anywhere the user has seen it. Anything
/// unrecognised is passed through as-is rather than blanked.
pub fn plan_name(raw: &str) -> String {
    match raw.to_lowercase().as_str() {
        "free" => "Free".into(),
        "go" => "Go".into(),
        "plus" => "Plus".into(),
        "pro" => "Pro".into(),
        // Reported for the 5× Pro tier.
        "prolite" => "Pro 5x".into(),
        "team" => "Team".into(),
        "business" => "Business".into(),
        "enterprise" => "Enterprise".into(),
        "edu" => "Edu".into(),
        other => other.to_string(),
    }
}
