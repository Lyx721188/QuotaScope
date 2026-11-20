//! One-off rate-limit reset credits from the Codex account endpoint.

use crate::codex_rpc::{self, RpcError};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum ResetCredits {
    Available {
        count: u32,
        next_expiry_ms: Option<i64>,
    },
    Unreported,
    CodexMissing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountDetails {
    pub reset_credits: ResetCredits,
}

pub fn fetch() -> AccountDetails {
    let limits = codex_rpc::request("account/rateLimits/read");
    let reset_credits = match &limits {
        Ok(root) => parse_reset_credits(root),
        Err(RpcError::Missing) => ResetCredits::CodexMissing,
        Err(_) => ResetCredits::Unreported,
    };
    AccountDetails { reset_credits }
}

fn count(value: Option<&Value>) -> Option<i64> {
    value?
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0 && *n < i64::MAX as f64)
        .map(|n| n as i64)
}

fn expiry(value: Option<&Value>) -> Option<i64> {
    let seconds = value?
        .as_f64()
        .filter(|n| n.is_finite() && *n > 0.0 && *n < i64::MAX as f64 / 1000.0)?;
    let ms = (seconds * 1000.0) as i64;
    chrono::DateTime::from_timestamp_millis(ms).map(|_| ms)
}

fn available(root: &Value) -> Vec<&Value> {
    root.pointer("/rateLimitResetCredits/credits")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|v| v.get("status").and_then(Value::as_str) == Some("available"))
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_reset_credits(root: &Value) -> ResetCredits {
    let Some(block) = root.get("rateLimitResetCredits").filter(|v| v.is_object()) else {
        return ResetCredits::Unreported;
    };
    let available = available(root);
    let stated = count(block.get("availableCount"))
        .filter(|n| *n < 1_000_000)
        .map(|n| n as u32);
    let count = stated.or_else(|| {
        block
            .get("credits")
            .and_then(Value::as_array)
            .map(|_| available.len() as u32)
    });
    let Some(count) = count else {
        return ResetCredits::Unreported;
    };
    let next_expiry_ms = available
        .iter()
        .filter_map(|credit| expiry(credit.get("expiresAt")))
        .min();
    ResetCredits::Available {
        count,
        next_expiry_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stated_counts_win_and_only_available_credits_supply_expiry() {
        let root = json!({"rateLimitResetCredits":{"availableCount":4,"credits":[
            {"status":"used","expiresAt":1}, {"status":"available","expiresAt":1800000000},
            {"status":"available","expiresAt":1900000000}]}});
        assert_eq!(
            parse_reset_credits(&root),
            ResetCredits::Available {
                count: 4,
                next_expiry_ms: Some(1800000000000)
            }
        );
        assert_eq!(parse_reset_credits(&json!({})), ResetCredits::Unreported);
        assert_eq!(
            parse_reset_credits(&json!({"rateLimitResetCredits":{"credits":[]}})),
            ResetCredits::Available {
                count: 0,
                next_expiry_ms: None
            }
        );
    }

    #[test]
    fn malformed_or_unstated_counts_are_unknown() {
        assert_eq!(
            parse_reset_credits(&json!({"rateLimitResetCredits":{"availableCount":1e100}})),
            ResetCredits::Unreported
        );
        assert_eq!(
            parse_reset_credits(&json!({"rateLimitResetCredits":{"credits":"wrong type"}})),
            ResetCredits::Unreported
        );
    }
}
