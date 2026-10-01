//! Account-wide Codex statistics and one-off rate-limit reset credits.
//! Statistics report tokens, never a price or a subscription bill.

use crate::codex_rpc::{self, RpcError};
use chrono::NaiveDate;
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
pub struct UsageDay {
    pub date: NaiveDate,
    pub tokens: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountUsage {
    pub days: Vec<UsageDay>,
    pub lifetime_tokens: Option<i64>,
    pub peak_daily_tokens: Option<i64>,
    pub current_streak_days: Option<i64>,
    pub longest_streak_days: Option<i64>,
    pub next_credit_title: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountDetails {
    pub usage: Option<AccountUsage>,
    pub reset_credits: ResetCredits,
}

pub fn fetch(include_usage: bool) -> AccountDetails {
    let limits = codex_rpc::request("account/rateLimits/read");
    let reset_credits = match &limits {
        Ok(root) => parse_reset_credits(root),
        Err(RpcError::Missing) => ResetCredits::CodexMissing,
        Err(_) => ResetCredits::Unreported,
    };
    let usage = if include_usage && limits.is_ok() {
        codex_rpc::request("account/usage/read")
            .ok()
            .and_then(|usage| parse_usage(&usage, limits.as_ref().ok()?))
    } else {
        None
    };
    AccountDetails {
        usage,
        reset_credits,
    }
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

pub fn parse_usage(root: &Value, limits: &Value) -> Option<AccountUsage> {
    let summary = root.get("summary").filter(|v| v.is_object());
    let buckets = root.get("dailyUsageBuckets").and_then(Value::as_array);
    if summary.is_none() && buckets.is_none() {
        return None;
    }
    let mut days: Vec<_> = buckets
        .into_iter()
        .flatten()
        .filter_map(|bucket| {
            Some(UsageDay {
                date: NaiveDate::parse_from_str(bucket.get("startDate")?.as_str()?, "%Y-%m-%d")
                    .ok()?,
                tokens: count(bucket.get("tokens"))?,
            })
        })
        .collect();
    days.sort_by_key(|d| d.date);
    let next_credit_title = available(limits)
        .into_iter()
        .filter(|credit| credit.get("title").and_then(Value::as_str).is_some())
        .min_by_key(|credit| expiry(credit.get("expiresAt")).unwrap_or(i64::MAX))
        .and_then(|credit| credit.get("title").and_then(Value::as_str))
        .map(str::to_string);
    Some(AccountUsage {
        days,
        lifetime_tokens: count(summary.and_then(|s| s.get("lifetimeTokens"))),
        peak_daily_tokens: count(summary.and_then(|s| s.get("peakDailyTokens"))),
        current_streak_days: count(summary.and_then(|s| s.get("currentStreakDays"))),
        longest_streak_days: count(summary.and_then(|s| s.get("longestStreakDays"))),
        next_credit_title,
    })
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
    fn malformed_or_unstated_counts_are_unknown_and_missing_summary_is_not_zero() {
        assert_eq!(
            parse_reset_credits(&json!({"rateLimitResetCredits":{"availableCount":1e100}})),
            ResetCredits::Unreported
        );
        assert_eq!(
            parse_reset_credits(&json!({"rateLimitResetCredits":{"credits":"wrong type"}})),
            ResetCredits::Unreported
        );
        assert!(parse_usage(&json!({"unrelated":true}), &json!({})).is_none());
        let usage = parse_usage(
            &json!({"dailyUsageBuckets":[{"startDate":"2026-10-01","tokens":12},
            {"startDate":"wrong","tokens":999}, {"startDate":"2026-09-30","tokens":7}]}),
            &json!({}),
        )
        .unwrap();
        assert_eq!(
            usage.days.iter().map(|d| d.tokens).collect::<Vec<_>>(),
            vec![7, 12]
        );
        assert_eq!(usage.lifetime_tokens, None);
    }
}
