//! LLM API Key Proxy, a self-hosted gateway: the quota groups it reports for
//! the upstream accounts behind it, each as a percentage left and a reset.
//!
//! Read with the key the user pastes and the proxy address they enter, from
//! the proxy's own statistics route: `GET <address>/v1/quota-stats`, bearer
//! key. The key goes to that address and nowhere else. The shape is
//! second-hand — taken from CodexBar's LLM Proxy plugin and its tests, not
//! from a captured reply — and the fixtures in the tests say so.
//!
//! **Each group is its own row.** CodexBar folds every group into one
//! figure — the lowest remainder anywhere — which is a number no upstream
//! account reported. Here each group keeps its own, scoped by the upstream's
//! name and the group's. None states its length, so each is an allowance
//! with a reset and no claimed period.
//!
//! Request counts, token counts and the approximate cost are spend with no
//! limit behind them, which QuotaScope has nowhere to show yet, and are left
//! off.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::gateway;
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct LlmProxyService {
    http: Arc<HttpClient>,
}

impl LlmProxyService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for LlmProxyService {
    fn provider(&self) -> Provider {
        Provider::LlmProxy
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::LlmProxy);
        let Some(key) = pasted_or_none(keys.api_key(Provider::LlmProxy)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let Some(address) = keys
            .address(Provider::LlmProxy)
            .filter(|s| !s.trim().is_empty())
        else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressMissing);
        };
        // The service root or its `/v1` base both lead to `/v1/quota-stats`.
        let Some(endpoint) = gateway::url_from(&address, "/v1/quota-stats", &["/v1"]) else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressRefused);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".to_string()),
        ];
        let header_refs: Vec<(&str, &str)> =
            headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply =
            match self
                .http
                .fetch_json(crate::http::Method::Get, &endpoint, &header_refs, None)
            {
                Ok(value) => value,
                Err(reason) => return ProviderUsage::unavailable(account, reason),
            };
        match windows(&reply) {
            Ok(list) => {
                let mut usage = ProviderUsage::live_now(account, list);
                usage.origin = Some(self.origin_token().into());
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// One quota group: a percentage left and, when the proxy says, a reset.
#[derive(Deserialize)]
struct Group {
    #[serde(default, rename = "remaining_percent")]
    remaining_percent: Option<f64>,
    #[serde(default, rename = "reset_time")]
    reset_time: Option<String>,
}

/// `quota_groups` comes keyed by the group's name or as a plain list. A
/// keyed reply keeps its names; sorted, so the rows hold still between
/// fetches.
enum Groups {
    Named(Vec<(String, Group)>),
    List(Vec<Group>),
}

impl<'de> Deserialize<'de> for Groups {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Object(map) => {
                let mut named: Vec<(String, Group)> = map
                    .into_iter()
                    .map(|(name, group)| {
                        Group::deserialize(group)
                            .map(|group| (name, group))
                            .map_err(serde::de::Error::custom)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                named.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(Groups::Named(named))
            }
            serde_json::Value::Array(items) => {
                let list = items
                    .into_iter()
                    .map(|item| Group::deserialize(item).map_err(serde::de::Error::custom))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Groups::List(list))
            }
            // Anything else is a malformed group list.
            _ => Err(serde::de::Error::custom("malformed quota_groups")),
        }
    }
}

#[derive(Deserialize)]
struct Upstream {
    #[serde(default, deserialize_with = "groups_or_none")]
    quota_groups: Option<Groups>,
}

/// A malformed `quota_groups` is left out rather than failing the whole
/// reply, as CodexBar does.
fn groups_or_none<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Groups>, D::Error> {
    match Option::<Groups>::deserialize(deserializer) {
        Ok(groups) => Ok(groups),
        Err(_) => Ok(None),
    }
}

#[derive(Deserialize)]
struct Reply {
    providers: BTreeMap<String, Upstream>,
}

/// One row per quota group, scoped by the upstream's name and the group's.
///
/// An error is the reading's verdict: a reply without a `providers` object
/// cannot be read at all, and nothing left after the malformed groups are
/// dropped is no limits.
pub fn windows(reply: &serde_json::Value) -> Result<Vec<UsageWindow>, Unavailability> {
    let reply: Reply =
        serde_json::from_value(reply.clone()).map_err(|_| Unavailability::UnreadableReply)?;

    let mut windows = Vec::new();
    for (upstream, stats) in &reply.providers {
        let Some(groups) = &stats.quota_groups else {
            continue;
        };
        let entries: Vec<(Option<&str>, &Group)> = match groups {
            Groups::Named(named) => named
                .iter()
                .map(|(name, group)| (Some(name.as_str()), group))
                .collect(),
            Groups::List(list) => list.iter().map(|group| (None, group)).collect(),
        };
        for (index, (name, group)) in entries.iter().enumerate() {
            let Some(left) = group
                .remaining_percent
                .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
            else {
                continue;
            };
            // Upstream and group names are the proxy's own identifiers —
            // "gemini_cli", "claude-sonnet" — so they stay untranslated.
            let group_name = name.filter(|n| *n != "default");
            let scope = [Some(upstream.as_str()), group_name]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            let mut window = UsageWindow::new(
                &format!(
                    "llmproxy.{}.{}",
                    upstream,
                    name.unwrap_or(index.to_string().as_str())
                ),
                // Upstream kinds this `.credits` — an allowance with a reset
                // it states and no length it claims. The Windows kind set has
                // no credits case; Spend is the one kind that never claims a
                // length either.
                Kind::Spend,
                Some(scope),
                (100.0 - left) / 100.0,
                // A sort key only: no group states its period.
                86_400,
                group
                    .reset_time
                    .as_deref()
                    .and_then(crate::timeutil::parse_iso8601_ms),
            );
            window.reports_length = false;
            window.is_exhausted = left <= 0.0;
            windows.push(window);
        }
    }

    if windows.is_empty() {
        return Err(Unavailability::NoLimitsReported);
    }
    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape pinned by upstream's `llm-proxy-quota-stats` fixture —
    /// second-hand, written from CodexBar's LLM Proxy plugin, not captured
    /// from a live proxy.
    fn fixture() -> serde_json::Value {
        serde_json::json!({
            "providers": {
                "openai": {
                    "credential_count": 3, "active_count": 2, "exhausted_count": 1,
                    "total_requests": 120,
                    "tokens": {"input_cached": 1000, "input_uncached": 2000, "output": 3000},
                    "approx_cost": 12.5,
                    "quota_groups": {"default": {"remaining_percent": 42, "reset_time": "2026-05-18T12:00:00Z"}}
                },
                "anthropic": {
                    "credential_count": 1, "active_count": 1, "exhausted_count": 0,
                    "total_requests": 40,
                    "tokens": {"input_cached": 0, "input_uncached": 500, "output": 500},
                    "approx_cost": 3.0,
                    "quota_groups": [{"remaining_percent": 80}]
                },
                "gemini_cli": {
                    "quota_groups": {"gemini-2.5-pro": {"remaining_percent": 0, "reset_time": "2026-05-18T08:00:00.123Z"}}
                }
            },
            "summary": {"total_requests": 160, "total_tokens": 7000, "approx_cost": 15.5}
        })
    }

    #[test]
    fn each_quota_group_is_its_own_row_scoped_by_upstream_and_group() {
        let list = windows(&fixture()).unwrap();
        assert_eq!(
            list.iter().map(|w| w.scope.clone()).collect::<Vec<_>>(),
            vec![
                Some("anthropic".to_string()),
                Some("gemini_cli · gemini-2.5-pro".to_string()),
                Some("openai".to_string())
            ]
        );
        assert_eq!(
            list.iter().map(|w| w.used_fraction).collect::<Vec<_>>(),
            vec![0.2, 1.0, 0.58]
        );
        // The "default" group's own name is not a scope, but it stays in the id.
        assert_eq!(list[2].id, "llmproxy.openai.default");
        assert_eq!(list[1].id, "llmproxy.gemini_cli.gemini-2.5-pro");
        assert!(list[1].is_exhausted);
        assert_eq!(
            list[1].resets_at,
            crate::timeutil::parse_iso8601_ms("2026-05-18T08:00:00.123Z")
        );
        assert_eq!(
            list[2].resets_at,
            crate::timeutil::parse_iso8601_ms("2026-05-18T12:00:00Z")
        );
        // No group states its period.
        assert!(list.iter().all(|w| !w.reports_length));
    }

    #[test]
    fn requests_tokens_and_cost_are_left_off() {
        let list = windows(&fixture()).unwrap();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn a_malformed_group_list_is_left_out_without_failing_the_rest() {
        let reply = serde_json::json!({
            "providers": {
                "a": {"quota_groups": "nope"},
                "b": {"quota_groups": [{"remaining_percent": 90}]}
            }
        });
        let list = windows(&reply).unwrap();
        assert_eq!(
            list.iter().map(|w| w.scope.clone()).collect::<Vec<_>>(),
            vec![Some("b".to_string())]
        );
    }

    #[test]
    fn figures_outside_the_percentage_are_left_off_and_nothing_left_is_no_limits() {
        let reply = serde_json::json!({
            "providers": {
                "a": {"quota_groups": [{"remaining_percent": -1}, {"remaining_percent": 150}, {}]},
                "b": {}
            }
        });
        assert_eq!(windows(&reply), Err(Unavailability::NoLimitsReported));
    }

    #[test]
    fn a_reply_with_no_providers_cant_be_read() {
        for reply in [
            serde_json::Value::Null,
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!({"providers": "nope"}),
        ] {
            assert_eq!(windows(&reply), Err(Unavailability::UnreadableReply));
        }
    }

    #[test]
    fn the_service_root_and_its_v1_base_lead_to_the_same_route() {
        let root = gateway::url_from("https://proxy.example.com", "/v1/quota-stats", &["/v1"]);
        let versioned =
            gateway::url_from("https://proxy.example.com/v1", "/v1/quota-stats", &["/v1"]);
        assert_eq!(
            root.as_deref(),
            Some("https://proxy.example.com/v1/quota-stats")
        );
        assert_eq!(versioned, root);
    }
}
