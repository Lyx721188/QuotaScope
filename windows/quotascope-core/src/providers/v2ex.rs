//! V2EX AI Chat quota, read with a Personal Access Token.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{number, HttpClient};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use std::sync::Arc;

const ENDPOINT: &str = "https://edge.v2ex.com/api/v2/chat/quota";

pub struct V2exService {
    http: Arc<HttpClient>,
}

impl V2exService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for V2exService {
    fn provider(&self) -> Provider {
        Provider::V2ex
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::V2ex);
        let Some(key) = pasted_or_none(keys.api_key(Provider::V2ex)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply = match self
            .http
            .fetch_json(crate::http::Method::Get, ENDPOINT, &refs, None)
        {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        if reply.get("success").and_then(|v| v.as_bool()) == Some(false) {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyRefused);
        }
        let Some(quota) = reply.get("result") else {
            return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
        };
        let windows = windows(quota);
        if windows.is_empty() {
            return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
        }
        let mut usage = ProviderUsage::live_now(account, windows);
        usage.origin = Some(self.origin_token().into());
        usage
    }
}

fn fraction(used: Option<f64>, total: Option<f64>) -> Option<f64> {
    let total = total.filter(|v| v.is_finite() && *v > 0.0)?;
    let used = used.filter(|v| v.is_finite())?;
    Some((used / total).clamp(0.0, 1.0))
}

fn spent(remaining: Option<f64>) -> bool {
    remaining
        .filter(|v| v.is_finite())
        .is_some_and(|v| v <= 0.0)
}

pub fn windows(quota: &serde_json::Value) -> Vec<UsageWindow> {
    let active = quota
        .get("active")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut out = Vec::new();
    if let Some(used) = quota.get("used_tokens").and_then(number) {
        if let Some(fraction) = fraction(Some(used), quota.get("total_tokens").and_then(number)) {
            let mut window = UsageWindow::new(
                "window",
                Kind::FiveHour,
                None,
                fraction,
                5 * 3_600,
                active
                    .then(|| quota.get("period_end").and_then(number))
                    .flatten()
                    .map(crate::timeutil::epoch_to_ms),
            );
            window.reports_length = active;
            window.is_exhausted = spent(quota.get("remaining_tokens").and_then(number));
            out.push(window);
        }
    }
    if let Some(extra) = quota.get("extra_usage") {
        let packs = extra.get("pack_count").and_then(number).unwrap_or(0.0);
        if packs > 0.0 {
            if let Some(fraction) = fraction(
                extra.get("used_tokens").and_then(number),
                extra.get("total_tokens").and_then(number),
            ) {
                let mut window = UsageWindow::new(
                    "extra",
                    Kind::Other(30 * 86_400),
                    None,
                    fraction,
                    30 * 86_400,
                    None,
                );
                window.reports_length = false;
                window.label = Some("Extra usage".into());
                window.is_exhausted = spent(extra.get("remaining_tokens").and_then(number));
                out.push(window);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_window_has_no_clock_length() {
        let value = serde_json::json!({"active": false, "total_tokens": 100, "used_tokens": 1, "remaining_tokens": 99});
        let w = &windows(&value)[0];
        assert!(!w.reports_length);
        assert!(w.resets_at.is_none());
    }
}
