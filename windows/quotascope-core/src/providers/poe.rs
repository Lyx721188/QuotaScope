//! Poe's point balance, read with a key from `GET
//! https://api.poe.com/usage/current_balance`.
//!
//! Poe reports no allowance and no period beside the balance, so there is no
//! ring here — only the credit line. Points are Poe's own unit, not money,
//! and are never compared against a currency. The reply's shape is
//! second-hand (taken from CodexBar's Poe plugin and its docs, not from a
//! captured reply), which is one reason the reader stays strict.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, Provider, ProviderUsage, State, Unavailability};
use std::sync::Arc;

const ENDPOINT: &str = "https://api.poe.com/usage/current_balance";

pub struct PoeService {
    http: Arc<HttpClient>,
}

impl PoeService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for PoeService {
    fn provider(&self) -> Provider {
        Provider::Poe
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Poe);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Poe)) else {
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
        let mut usage = reading(&reply, account);
        if matches!(usage.state, State::Live) {
            usage.origin = Some(self.origin_token().into());
        }
        usage
    }
}

/// Poe writes the balance as a number, or as a numeric string; anything else
/// — including a balance that is missing — is a reply that cannot be read.
/// It is never read as zero: zero would draw a spent account the service
/// never reported.
pub fn reading(reply: &serde_json::Value, account: AccountKey) -> ProviderUsage {
    let Some(points) = crate::http::number_field(reply, "current_point_balance")
        .filter(|points| points.is_finite())
    else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    let mut usage = ProviderUsage::live_now(account, Vec::new());
    usage.credit_balance = Some(balance_text(points));
    usage
}

/// The balance in whole points, the way upstream's zero-fraction formatter
/// shows it — Poe's own unit, so no currency sign.
pub fn balance_text(points: f64) -> String {
    crate::localization::t_fmt("{n} points", &[(&(points.round() as i64).to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    fn reading_for(value: serde_json::Value) -> ProviderUsage {
        reading(&value, AccountKey::primary(Provider::Poe))
    }

    #[test]
    fn numeric_balance_reads_live_without_a_ring() {
        let usage = reading_for(serde_json::json!({"current_point_balance": 1234.0}));
        assert!(matches!(usage.state, State::Live));
        assert!(usage.windows.is_empty());
        let balance = usage.credit_balance.expect("balance text");
        assert!(balance.contains("1234"), "unexpected text: {balance}");
    }

    #[test]
    fn string_balance_reads_and_shows_whole_points() {
        // The string shape trims, and the display rounds to whole points.
        let usage = reading_for(serde_json::json!({"current_point_balance": " 1234.6 "}));
        let balance = usage.credit_balance.expect("balance text");
        assert!(balance.contains("1235"), "unexpected text: {balance}");
    }

    #[test]
    fn missing_or_null_balance_is_unreadable_not_zero() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"current_point_balance": null}),
        ] {
            let usage = reading_for(value);
            assert!(matches!(
                usage.state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }

    #[test]
    fn non_finite_balance_is_unreadable() {
        // A string the number reader accepts but no formatter could show.
        let usage = reading_for(serde_json::json!({"current_point_balance": "inf"}));
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::UnreadableReply)
        ));
    }

    #[test]
    fn non_object_reply_is_unreadable() {
        let usage = reading_for(serde_json::json!([1, 2, 3]));
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::UnreadableReply)
        ));
    }
}
