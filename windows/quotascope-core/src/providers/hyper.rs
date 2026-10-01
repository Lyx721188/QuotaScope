//! Charm's Hyper: the account's Hypercredit balance, and nothing else — the
//! service reports no allowance and no period, so there is no ring.
//!
//! Read with a key the user pastes, from Hyper's credits route:
//! `GET https://hyper.charm.land/v1/credits`. The shape is second-hand —
//! taken from CodexBar's Hyper plugin, not from a captured reply — and the
//! test here says so. Hypercredits are Charm's own unit rather than money, so
//! the balance is shown and never compared against a currency.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{number_field, HttpClient, Method};
use crate::model::{AccountKey, Provider, ProviderUsage, Unavailability};
use std::sync::Arc;

const ENDPOINT: &str = "https://hyper.charm.land/v1/credits";

pub struct HyperService {
    http: Arc<HttpClient>,
}

impl HyperService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for HyperService {
    fn provider(&self) -> Provider {
        Provider::Hyper
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Hyper);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Hyper)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply = match self.http.fetch_json(Method::Get, ENDPOINT, &refs, None) {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let mut usage = reading(&reply, account);
        if matches!(usage.state, crate::model::State::Live) {
            usage.origin = Some(self.origin_token().into());
        }
        usage
    }
}

pub fn reading(reply: &serde_json::Value, account: AccountKey) -> ProviderUsage {
    // A balance that is absent or unreadable is never read as zero: zero
    // would draw a spent account.
    let Some(balance) =
        number_field(reply, "balance").filter(|balance| balance.is_finite() && *balance >= 0.0)
    else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    let mut usage = ProviderUsage::live_now(account, Vec::new());
    usage.credit_balance = Some(hc_text(balance));
    usage
}

/// "1,234.5 HC". HC is Charm's own abbreviation for Hypercredits, the unit's
/// name, and reads the same in every language.
pub fn hc_text(balance: f64) -> String {
    // Two decimal places at most, thousands grouped — the shape Swift's
    // decimal formatter gave on macOS.
    let rounded = (balance * 100.0).round() / 100.0;
    let whole = rounded.trunc();
    let cents = ((rounded - whole) * 100.0).round() as i64;
    let digits = (whole as i128).to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if cents > 0 {
        grouped.push_str(format!(".{cents:02}").trim_end_matches('0'));
    }
    format!("{grouped} HC")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    fn reading_for(value: serde_json::Value) -> ProviderUsage {
        reading(&value, AccountKey::primary(Provider::Hyper))
    }

    #[test]
    fn the_balance_shows_without_a_ring() {
        let usage = reading_for(serde_json::json!({"balance": 1234.5}));
        assert!(matches!(usage.state, State::Live));
        assert!(usage.windows.is_empty());
        assert_eq!(usage.credit_balance.as_deref(), Some("1,234.5 HC"));
        assert!(
            usage.credit_remaining.is_none(),
            "hypercredits are not money"
        );
    }

    #[test]
    fn a_whole_number_prints_without_decimals() {
        assert_eq!(hc_text(12.0), "12 HC");
        assert_eq!(hc_text(1_234_567.0), "1,234,567 HC");
        assert_eq!(hc_text(0.256), "0.26 HC");
    }

    #[test]
    fn a_missing_or_negative_balance_is_unreadable_not_zero() {
        for reply in [
            serde_json::json!({}),
            serde_json::json!({"balance": null}),
            serde_json::json!({"balance": -1.0}),
            serde_json::json!({"balance": "soon"}),
        ] {
            assert!(matches!(
                reading_for(reply).state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }

    #[test]
    fn a_number_in_a_string_is_still_a_balance() {
        let usage = reading_for(serde_json::json!({"balance": "12.5"}));
        assert_eq!(usage.credit_balance.as_deref(), Some("12.5 HC"));
    }
}
