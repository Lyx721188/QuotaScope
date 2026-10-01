//! Hugging Face ZeroGPU daily quota.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{number, HttpClient};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use std::sync::Arc;

const ENDPOINT: &str = "https://huggingface.co/api/spaces/zero-gpu/quota";

pub struct HuggingFaceService {
    http: Arc<HttpClient>,
}
impl HuggingFaceService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for HuggingFaceService {
    fn provider(&self) -> Provider {
        Provider::HuggingFace
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }
    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::HuggingFace);
        let key = pasted_or_none(keys.api_key(Provider::HuggingFace)).or_else(saved_token);
        let Some(key) = key else {
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
        let Some(base) = reply
            .get("base")
            .and_then(number)
            .filter(|v| v.is_finite() && *v > 0.0)
        else {
            return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
        };
        let Some(current) = reply
            .get("current")
            .and_then(number)
            .filter(|v| v.is_finite() && *v >= 0.0)
        else {
            return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
        };
        let mut window = UsageWindow::new(
            "huggingface.zeroGPU",
            Kind::Other(86_400),
            Some("ZeroGPU".into()),
            ((base - current) / base).max(0.0),
            86_400,
            parse_date(reply.get("resetsAt")),
        );
        window.reports_length = false;
        window.is_exhausted = current <= 0.0;
        let mut usage = ProviderUsage::live_now(account, vec![window]);
        usage.origin = Some(self.origin_token().into());
        usage
    }
}

fn saved_token() -> Option<String> {
    let path = crate::home_dir().join(".cache/huggingface/token");
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().next()?.trim();
    let trimmed = line
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| line.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(line)
        .trim();
    (!trimmed.is_empty()).then_some(trimmed.to_string())
}

fn parse_date(value: Option<&serde_json::Value>) -> Option<i64> {
    match value? {
        serde_json::Value::String(text) => crate::timeutil::parse_iso8601_ms(text),
        other => number(other)
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .map(crate::timeutil::epoch_to_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_gpu_is_sort_key_only() {
        let value = serde_json::json!({"base": 100, "current": 25, "resetsAt": 1700000000});
        let _ = value;
        assert_eq!(Kind::Other(86_400).token(), "other:86400");
    }
}
