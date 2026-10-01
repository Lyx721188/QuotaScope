//! ZenMux, a model gateway sold by subscription: a rolling five-hour and a
//! rolling seven-day quota, each reported as a share used by the service
//! itself, and a pay-as-you-go balance in US dollars beside them.
//!
//! Read with a **Management** API key the user pastes — ZenMux's ordinary
//! inference keys are refused by these endpoints — from
//! `GET https://zenmux.ai/api/v1/management/subscription/detail`, and the
//! balance from `…/payg/balance`. The shapes are second-hand — taken from
//! CodexBar's ZenMux provider and its tests, not from a captured reply — and
//! the tests here say so.
//!
//! The balance is best-effort: the quotas are the reading, and a balance that
//! cannot be had leaves them standing rather than failing the refresh.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{bool_field, number_field, object_field, string_field, HttpClient, Method};
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, Unavailability, UsageWindow,
};
use std::sync::Arc;

const SUBSCRIPTION_ENDPOINT: &str = "https://zenmux.ai/api/v1/management/subscription/detail";
const BALANCE_ENDPOINT: &str = "https://zenmux.ai/api/v1/management/payg/balance";

pub struct ZenMuxService {
    http: Arc<HttpClient>,
}

impl ZenMuxService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for ZenMuxService {
    fn provider(&self) -> Provider {
        Provider::ZenMux
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::ZenMux);
        let Some(key) = pasted_or_none(keys.api_key(Provider::ZenMux)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let subscription =
            match self
                .http
                .fetch_json(Method::Get, SUBSCRIPTION_ENDPOINT, &refs, None)
            {
                Ok(value) => value,
                Err(reason) => return ProviderUsage::unavailable(account, reason),
            };
        // Asked only once the quotas are in hand; whatever becomes of it, the
        // quotas are what is shown.
        let balance = self
            .http
            .fetch_json(Method::Get, BALANCE_ENDPOINT, &refs, None)
            .ok();
        let mut usage = reading(&subscription, balance.as_ref(), account);
        if matches!(usage.state, crate::model::State::Live) {
            usage.origin = Some(self.origin_token().into());
        }
        usage
    }
}

/// Both replies are wrapped in `{success: …, data: …}`; a wrapper that does
/// not say success is not a reading.
fn unwrapped(reply: &serde_json::Value) -> Option<&serde_json::Value> {
    match bool_field(reply, "success") {
        Some(true) => object_field(reply, "data"),
        _ => None,
    }
}

pub fn reading(
    subscription: &serde_json::Value,
    balance: Option<&serde_json::Value>,
    account: AccountKey,
) -> ProviderUsage {
    let Some(detail) = unwrapped(subscription) else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };

    // Both quotas are named for their length, so the length is stated.
    let named = [
        ("quota5_hour", "zenmux.five_hour", Kind::FiveHour, 5 * 3_600),
        ("quota7_day", "zenmux.seven_day", Kind::Weekly, 7 * 86_400),
    ];
    let mut windows = Vec::new();
    for (key, id, kind, seconds) in named {
        let Some(quota) = object_field(detail, key) else {
            continue;
        };
        let Some(fraction) = number_field(quota, "usage_percentage")
            .filter(|fraction| fraction.is_finite() && *fraction >= 0.0)
        else {
            continue;
        };
        let resets_at =
            string_field(quota, "resets_at").and_then(crate::timeutil::parse_iso8601_ms);
        let mut window = UsageWindow::new(id, kind, None, fraction, seconds, resets_at);
        window.is_exhausted = fraction >= 1.0;
        windows.push(window);
    }

    let payg = balance.and_then(payg_balance);
    if windows.is_empty() && payg.is_none() {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }
    let mut usage = ProviderUsage::live_now(account, windows);
    usage.plan = detail
        .get("plan")
        .and_then(|plan| string_field(plan, "tier"))
        .and_then(tier_name);
    usage.credit_balance = payg
        .as_ref()
        .map(|credit| money_text(credit.amount, &credit.currency));
    usage.credit_remaining = payg;
    usage
}

/// The pay-as-you-go balance, in the currency ZenMux names. Kept negative
/// when it is: an overdue account is not an empty one.
pub fn payg_balance(reply: &serde_json::Value) -> Option<CreditAmount> {
    let data = unwrapped(reply)?;
    let amount = number_field(data, "total_credits").filter(|amount| amount.is_finite())?;
    let currency = string_field(data, "currency")?.trim().to_uppercase();
    if currency.chars().count() != 3 {
        return None;
    }
    Some(CreditAmount { amount, currency })
}

/// "pro" reads as "Pro".
fn tier_name(tier: &str) -> Option<String> {
    let tier = tier.trim();
    if tier.is_empty() {
        return None;
    }
    let mut chars = tier.chars();
    let first = chars.next()?.to_uppercase().collect::<String>();
    Some(first + &chars.as_str().to_lowercase())
}

/// The balance as money — symbol first, cents kept.
fn money_text(amount: f64, currency: &str) -> String {
    let symbol = match currency {
        "USD" => "$",
        "CNY" => "¥",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" => "¥",
        other => return format!("{other} {amount:.2}"),
    };
    format!("{symbol}{amount:.2}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    fn subscription() -> serde_json::Value {
        serde_json::json!({
            "success": true,
            "data": {
                "plan": {"tier": "pro"},
                "quota5_hour": {"usage_percentage": 0.0715, "resets_at": "2026-10-01T18:00:00Z"},
                "quota7_day": {"usage_percentage": 0.02},
            },
        })
    }

    fn reading_with(balance: Option<serde_json::Value>) -> ProviderUsage {
        reading(
            &subscription(),
            balance.as_ref(),
            AccountKey::primary(Provider::ZenMux),
        )
    }

    #[test]
    fn the_two_quotas_read_with_their_lengths() {
        let usage = reading_with(None);
        assert!(matches!(usage.state, State::Live));
        assert_eq!(usage.plan.as_deref(), Some("Pro"));
        assert_eq!(usage.windows.len(), 2);

        let five_hour = &usage.windows[0];
        assert_eq!(five_hour.id, "zenmux.five_hour");
        assert_eq!(five_hour.kind, Kind::FiveHour);
        assert_eq!(five_hour.used_fraction, 0.0715);
        assert_eq!(five_hour.window_seconds, 5 * 3_600);
        assert!(five_hour.reports_length);
        assert_eq!(
            five_hour.resets_at,
            crate::timeutil::parse_iso8601_ms("2026-10-01T18:00:00Z")
        );

        let seven_day = &usage.windows[1];
        assert_eq!(seven_day.kind, Kind::Weekly);
        assert_eq!(seven_day.used_fraction, 0.02);
        assert!(seven_day.resets_at.is_none());
    }

    #[test]
    fn the_balance_is_best_effort_and_kept_in_its_currency() {
        let usage = reading_with(Some(serde_json::json!({
            "success": true,
            "data": {"currency": "usd", "total_credits": 12.5},
        })));
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 12.5,
                currency: "USD".into()
            })
        );
        assert_eq!(usage.credit_balance.as_deref(), Some("$12.50"));
    }

    #[test]
    fn a_failed_or_odd_balance_leaves_the_quotas_standing() {
        for balance in [
            serde_json::json!({"success": false, "data": {}}),
            serde_json::json!({"success": true, "data": {"currency": "DOLLAR", "total_credits": 1.0}}),
            serde_json::json!({"success": true, "data": {"currency": "USD"}}),
        ] {
            let usage = reading_with(Some(balance));
            assert!(matches!(usage.state, State::Live));
            assert!(usage.credit_remaining.is_none());
            assert_eq!(usage.windows.len(), 2);
        }
    }

    #[test]
    fn an_overdue_balance_is_kept_negative() {
        let usage = reading_with(Some(serde_json::json!({
            "success": true,
            "data": {"currency": "USD", "total_credits": -3.0},
        })));
        assert_eq!(
            usage.credit_remaining.map(|credit| credit.amount),
            Some(-3.0)
        );
    }

    #[test]
    fn an_envelope_that_does_not_say_success_is_unreadable() {
        for reply in [
            serde_json::json!({}),
            serde_json::json!({"success": false, "data": {}}),
            serde_json::json!({"success": true}),
        ] {
            assert!(matches!(
                reading(&reply, None, AccountKey::primary(Provider::ZenMux)).state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }

    #[test]
    fn a_negative_or_missing_share_drops_its_window() {
        let reply = serde_json::json!({
            "success": true,
            "data": {"quota5_hour": {"usage_percentage": -0.1}},
        });
        assert_eq!(
            reading(&reply, None, AccountKey::primary(Provider::ZenMux)),
            ProviderUsage::unavailable(
                AccountKey::primary(Provider::ZenMux),
                Unavailability::NoLimitsReported
            )
        );
    }

    #[test]
    fn a_full_quota_is_marked_spent() {
        let reply = serde_json::json!({
            "success": true,
            "data": {"quota5_hour": {"usage_percentage": 1.0}},
        });
        let usage = reading(&reply, None, AccountKey::primary(Provider::ZenMux));
        assert!(usage.windows[0].is_exhausted);
    }
}
