//! xAI's prepaid credit, read with a **management** key from the documented
//! Management API. Not Grok — that is the consumer subscription, read by its
//! own provider — and nothing is shared between the two.
//!
//! `GET https://management-api.x.ai/v1/billing/teams/{team}/prepaid/balance`.
//! The ledger is inverted and in cents as a string: a $10 top-up reads
//! `"-1000"`, so what is left is the negated figure. It is the **posted**
//! ledger, which xAI updates when a billing cycle closes, so mid-cycle it can
//! read higher than the console's live remainder. The shape is second-hand —
//! from CodexBar's xAI provider and its docs — and the fixture in the tests
//! says so.
//!
//! **Two values, one field.** The balance is per team and the key does not
//! name one, so the field takes `TeamID:ManagementKey`, the way Volcengine's
//! takes its key pair. Left out: the thirty-day spend history, which there is
//! no place for.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, CreditAmount, Provider, ProviderUsage, State, Unavailability};
use std::sync::Arc;

const ENDPOINT: &str = "https://management-api.x.ai/v1/billing/teams";

pub struct XaiApiService {
    http: Arc<HttpClient>,
}

impl XaiApiService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for XaiApiService {
    fn provider(&self) -> Provider {
        Provider::XaiApi
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::XaiApi);
        let Some(text) = pasted_or_none(keys.api_key(Provider::XaiApi)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        // Not in the documented shape, so nothing is sent: it is not a key
        // xAI turned away, but it is the key field that needs fixing.
        let Some((team, key)) = credential(&text) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyRefused);
        };
        let Some(url) = endpoint(&team) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyRefused);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply = match self
            .http
            .fetch_json(crate::http::Method::Get, &url, &refs, None)
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

/// The team and the key out of `TeamID:ManagementKey`, split at the first
/// colon. A team id that could step out of the path is not one.
pub fn credential(text: &str) -> Option<(String, String)> {
    let (team, key) = text.split_once(':')?;
    let team = team.trim();
    let key = key.trim();
    let usable =
        !team.is_empty() && !key.is_empty() && !team.contains('/') && team != "." && team != "..";
    usable.then(|| (team.to_string(), key.to_string()))
}

/// The balance route, with the team id placed as a path segment so it is
/// percent-encoded the way upstream's URL builder would — a team id that
/// survived `credential` still cannot step out of the path.
pub fn endpoint(team: &str) -> Option<String> {
    let mut url = url::Url::parse(ENDPOINT).ok()?;
    {
        let mut segments = url.path_segments_mut().ok()?;
        segments.push(team).push("prepaid").push("balance");
    }
    Some(url.into())
}

/// The ledger's own shape: an optionally signed decimal with an optional
/// fraction, and nothing else — `^-?\d+(\.\d+)?$`.
pub fn is_decimal(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    match digits.split_once('.') {
        Some((whole, fraction)) => {
            !whole.is_empty()
                && !fraction.is_empty()
                && whole.bytes().all(|b| b.is_ascii_digit())
                && fraction.bytes().all(|b| b.is_ascii_digit())
        }
        None => !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
    }
}

pub fn reading(reply: &serde_json::Value, account: AccountKey) -> ProviderUsage {
    let Some(raw) = reply
        .get("total")
        .and_then(|total| total.get("val"))
        .and_then(|val| val.as_str())
        .map(str::trim)
    else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    if !is_decimal(raw) {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    }
    // Cents, signed because the ledger is: a top-up posts negative.
    let Ok(cents) = raw.parse::<f64>() else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    let dollars = -cents / 100.0;
    // No ring: what xAI reports here is money, and the balance is shown as
    // one.
    let mut usage = ProviderUsage::live_now(account, Vec::new());
    usage.credit_balance = Some(money_text(dollars));
    usage.credit_remaining = Some(CreditAmount {
        amount: dollars,
        currency: "USD".into(),
    });
    usage
}

/// The balance as money — symbol first, cents kept. A negative balance is
/// money owed, and the sign leads the figure the way a currency formatter
/// writes it.
fn money_text(amount: f64) -> String {
    if amount < 0.0 {
        format!("-${:.2}", -amount)
    } else {
        format!("${amount:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    fn reading_for(value: serde_json::Value) -> ProviderUsage {
        reading(&value, AccountKey::primary(Provider::XaiApi))
    }

    #[test]
    fn credential_splits_at_the_first_colon_and_trims() {
        assert_eq!(
            credential("team123:mk_abc"),
            Some(("team123".into(), "mk_abc".into()))
        );
        assert_eq!(
            credential(" team : key "),
            Some(("team".into(), "key".into()))
        );
        // The team is a path segment and the key a secret; only the first
        // colon separates them.
        assert_eq!(credential("t:a:b"), Some(("t".into(), "a:b".into())));
    }

    #[test]
    fn credential_refuses_shapes_that_could_step_out_of_the_path() {
        for text in ["noteam", "team:", ":key", ":", "a/b:key", ".:key", "..:key"] {
            assert_eq!(credential(text), None, "{text}");
        }
    }

    #[test]
    fn endpoint_places_the_team_as_one_path_segment() {
        assert_eq!(
            endpoint("team123").as_deref(),
            Some("https://management-api.x.ai/v1/billing/teams/team123/prepaid/balance")
        );
        // A team id with reserved characters cannot break out of the path.
        assert_eq!(
            endpoint("a b").as_deref(),
            Some("https://management-api.x.ai/v1/billing/teams/a%20b/prepaid/balance")
        );
    }

    #[test]
    fn decimal_is_the_ledgers_own_shape() {
        for text in ["0", "-1000", "10.5", "-1050.5"] {
            assert!(is_decimal(text), "{text}");
        }
        for text in ["", ".5", "5.", "abc", "1e3", "+5", "--5", "10.5.6", "- "] {
            assert!(!is_decimal(text), "{text}");
        }
    }

    #[test]
    fn a_negative_ledger_reads_as_credit() {
        let usage = reading_for(json!({"total": {"val": "-1000"}}));
        assert!(matches!(usage.state, State::Live));
        assert!(usage.windows.is_empty());
        assert_eq!(usage.credit_balance.as_deref(), Some("$10.00"));
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 10.0,
                currency: "USD".into(),
            })
        );
    }

    #[test]
    fn fractions_read_as_cents_too() {
        let usage = reading_for(json!({"total": {"val": "-1050.5"}}));
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 10.505,
                currency: "USD".into(),
            })
        );
    }

    #[test]
    fn a_spent_ledger_reads_as_money_owed() {
        let usage = reading_for(json!({"total": {"val": "500"}}));
        assert_eq!(usage.credit_balance.as_deref(), Some("-$5.00"));
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: -5.0,
                currency: "USD".into(),
            })
        );
    }

    #[test]
    fn anything_out_of_the_ledgers_shape_is_unreadable() {
        for reply in [
            json!({}),
            json!({"total": {}}),
            // The figure arrives as a string; a bare number is not the
            // documented shape.
            json!({"total": {"val": -1000}}),
            json!({"total": {"val": "abc"}}),
            json!({"total": {"val": ".5"}}),
            json!({"total": {"val": "1e3"}}),
        ] {
            let usage = reading_for(reply);
            assert!(matches!(
                usage.state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_before_the_shape_check() {
        let usage = reading_for(json!({"total": {"val": " -1000 "}}));
        assert!(matches!(usage.state, State::Live));
        assert_eq!(usage.credit_balance.as_deref(), Some("$10.00"));
    }
}
