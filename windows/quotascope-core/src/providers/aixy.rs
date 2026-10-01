//! Aixy's budgets for one API key, read with that key from the hosted
//! gateway's `GET https://api.aixy-gateway.com/v1/usage`.
//!
//! A budget applies to the key, its user, team, project or organization, and
//! states an amount used and a limit in US dollars. **Only budgets whose
//! balance the gateway knows are drawn**: one marked unavailable has a limit
//! and no figure against it, and is left off rather than drawn at zero. A
//! hard budget's use is what was spent plus what is reserved for requests in
//! flight, because that is what the gateway enforces against; a monitor-only
//! budget's is what was spent.
//!
//! Budgets overlap and are never summed. Where several share a period, the
//! one that binds is drawn: an enforced budget before a monitored one, then
//! the one nearest its limit. A budget's scope — key, team, project — is not
//! a model name, so it is never put on the row. Left out on purpose: the
//! last seven days' attributed spend, which Aixy itself says may be estimated
//! or partial and which has no limit beside it. The reply's shape is
//! second-hand (taken from CodexBar's Aixy provider and the usage contract
//! it validates, not from a captured reply).

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, State, Unavailability, UsageWindow};
use std::sync::Arc;

const ENDPOINT: &str = "https://api.aixy-gateway.com/v1/usage";

/// The periods Aixy names, in the order they sort, with the length each is
/// and whether it stated it. A month is a calendar month and a lifetime
/// budget never turns over, so both lengths are only sort keys. Upstream's
/// `.daily` has no Windows Kind case; the day as `other` is the stand-in the
/// crate already uses.
fn period(interval: &str) -> Option<(Kind, i64, bool, &'static str)> {
    match interval {
        "daily" => Some((Kind::Other(86_400), 86_400, true, "daily")),
        "weekly" => Some((Kind::Weekly, 7 * 86_400, true, "weekly")),
        "monthly" => Some((Kind::Monthly, 30 * 86_400, false, "monthly")),
        "lifetime" => Some((Kind::Spend, 365 * 86_400, false, "lifetime")),
        _ => None,
    }
}

/// One budget that is this key's, with a known balance and a limit.
struct Candidate {
    window: UsageWindow,
    hard: bool,
    period: &'static str,
}

/// Whether `item` binds the key more than `current`: an enforced budget
/// before a monitored one, then the one nearer its limit.
fn binds_tighter(current: &Candidate, item: &Candidate) -> bool {
    if current.hard != item.hard {
        !current.hard
    } else {
        current.window.used_fraction < item.window.used_fraction
    }
}

pub struct AixyService {
    http: Arc<HttpClient>,
}

impl AixyService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for AixyService {
    fn provider(&self) -> Provider {
        Provider::Aixy
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Aixy);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Aixy)) else {
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

pub fn reading(reply: &serde_json::Value, account: AccountKey) -> ProviderUsage {
    // The gateway states its contract in the envelope; the wrong object, a
    // currency other than the dollars the limits are written in, or a reply
    // without the key and its budgets is one this reader does not
    // understand — not an account without budgets.
    let key = reply.get("key");
    let shape_ok = crate::http::string_field(reply, "object") == Some("key.usage")
        && crate::http::string_field(reply, "currency") == Some("USD")
        && key
            .and_then(|key| crate::http::string_field(key, "id"))
            .is_some()
        && reply
            .get("budgets")
            .is_some_and(|budgets| budgets.is_array());
    if !shape_ok {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    }
    let key_id = key
        .and_then(|key| crate::http::string_field(key, "id"))
        .unwrap_or_default();
    let project_id = key.and_then(|key| crate::http::string_field(key, "project_id"));
    let budgets = crate::http::array_field(reply, "budgets");

    let known: Vec<Candidate> = budgets
        .iter()
        .filter_map(|budget| {
            let id = crate::http::string_field(budget, "id")?;
            let interval = crate::http::string_field(budget, "interval")?;
            let (kind, seconds, reports_length, period) = period(interval)?;
            let targets = crate::http::array_field(budget, "applies_to");
            // Every budget that is **this key's** — a team's or project's
            // budget that merely names this key beside others is not.
            if targets.is_empty()
                || !targets.iter().all(|target| {
                    crate::http::string_field(target, "api_key_id") == Some(key_id)
                        && crate::http::string_field(target, "project_id") == project_id
                })
            {
                return None;
            }
            let limit = crate::http::number_field(budget, "limit_usd")
                .filter(|limit| limit.is_finite() && *limit > 0.0)?;

            let hard = crate::http::string_field(budget, "enforcement") == Some("hard");
            if !hard && crate::http::string_field(budget, "enforcement") != Some("monitor") {
                return None;
            }
            let used = if hard {
                // A budget marked unavailable has a limit and no figure
                // against it; drawing it at zero would flatter the account.
                let availability = budget.get("availability")?;
                if crate::http::string_field(availability, "status") != Some("available") {
                    return None;
                }
                crate::http::number_field(availability, "spent_usd")?
                    + crate::http::number_field(availability, "reserved_usd")?
            } else if crate::http::string_field(budget, "spend_status") != Some("available") {
                return None;
            } else {
                crate::http::number_field(budget, "spend_usd")?
            };
            if !used.is_finite() || used < 0.0 {
                return None;
            }

            let fraction = used / limit;
            let mut window = UsageWindow::new(
                &format!("aixy.{id}"),
                kind,
                None,
                fraction,
                seconds,
                // A lifetime budget has no reset, whatever the reply says.
                if interval == "lifetime" {
                    None
                } else {
                    crate::http::string_field(budget, "resets_at")
                        .and_then(crate::timeutil::parse_iso8601_ms)
                },
            );
            window.reports_length = reports_length;
            window.is_exhausted = fraction >= 1.0;
            Some(Candidate {
                window,
                hard,
                period,
            })
        })
        .collect();

    // One per period: the one that binds. Budgets overlap and are never
    // summed.
    let mut binding: Vec<UsageWindow> = Vec::new();
    for name in ["daily", "weekly", "monthly", "lifetime"] {
        let best = known
            .iter()
            .filter(|candidate| candidate.period == name)
            .fold(None::<&Candidate>, |current, item| match current {
                None => Some(item),
                Some(current) => Some(if binds_tighter(current, item) {
                    item
                } else {
                    current
                }),
            });
        if let Some(best) = best {
            binding.push(best.window.clone());
        }
    }
    binding.sort_by_key(|window| window.window_seconds);

    if binding.is_empty() {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }
    ProviderUsage::live_now(account, binding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    fn reading_for(value: serde_json::Value) -> ProviderUsage {
        reading(&value, AccountKey::primary(Provider::Aixy))
    }

    fn envelope(budgets: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "object": "key.usage",
            "currency": "USD",
            "key": {"id": "k1", "project_id": "p1"},
            "budgets": budgets,
        })
    }

    fn live_windows(usage: &ProviderUsage) -> &[UsageWindow] {
        match &usage.state {
            State::Live => &usage.windows,
            other => panic!("expected a live reading, got {other:?}"),
        }
    }

    #[test]
    fn hard_budget_reads_spent_plus_reserved() {
        let usage = reading_for(envelope(serde_json::json!([{
            "id": "b1",
            "interval": "monthly",
            "enforcement": "hard",
            "limit_usd": 10,
            "applies_to": [{"api_key_id": "k1", "project_id": "p1"}],
            "availability": {"status": "available", "spent_usd": "2.5", "reserved_usd": 1.5},
        }])));
        let windows = live_windows(&usage);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "aixy.b1");
        assert_eq!(windows[0].kind, Kind::Monthly);
        assert!((windows[0].used_fraction - 0.4).abs() < 1e-9);
        // A calendar month: a sort key, not a stated length.
        assert!(!windows[0].reports_length);
    }

    #[test]
    fn monitor_budget_reads_spend_only() {
        let usage = reading_for(envelope(serde_json::json!([{
            "id": "b1",
            "interval": "weekly",
            "enforcement": "monitor",
            "limit_usd": 6,
            "applies_to": [{"api_key_id": "k1", "project_id": "p1"}],
            "spend_status": "available",
            "spend_usd": 3,
        }])));
        let windows = live_windows(&usage);
        assert_eq!(windows[0].kind, Kind::Weekly);
        assert!((windows[0].used_fraction - 0.5).abs() < 1e-9);
    }

    #[test]
    fn lifetime_has_no_reset_and_reports_no_length() {
        let usage = reading_for(envelope(serde_json::json!([{
            "id": "b1",
            "interval": "lifetime",
            "enforcement": "hard",
            "limit_usd": 100,
            "applies_to": [{"api_key_id": "k1", "project_id": "p1"}],
            "availability": {"status": "available", "spent_usd": 5, "reserved_usd": 0},
            "resets_at": "2026-01-01T00:00:00Z",
        }])));
        let windows = live_windows(&usage);
        assert_eq!(windows[0].kind, Kind::Spend);
        assert!(!windows[0].reports_length);
        assert!(windows[0].resets_at.is_none());
    }

    #[test]
    fn daily_window_reports_its_length_and_reset() {
        let usage = reading_for(envelope(serde_json::json!([{
            "id": "b1",
            "interval": "daily",
            "enforcement": "monitor",
            "limit_usd": 4,
            "applies_to": [{"api_key_id": "k1", "project_id": "p1"}],
            "spend_status": "available",
            "spend_usd": 1,
            "resets_at": "2026-10-02T00:00:00Z",
        }])));
        let windows = live_windows(&usage);
        assert!(windows[0].reports_length);
        assert_eq!(
            windows[0].resets_at,
            crate::timeutil::parse_iso8601_ms("2026-10-02T00:00:00Z")
        );
    }

    #[test]
    fn budget_for_another_key_is_not_this_keys() {
        let usage = reading_for(envelope(serde_json::json!([{
            "id": "b1",
            "interval": "daily",
            "enforcement": "hard",
            "limit_usd": 10,
            "applies_to": [{"api_key_id": "k2", "project_id": "p1"}],
            "availability": {"status": "available", "spent_usd": 1, "reserved_usd": 0},
        }])));
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::NoLimitsReported)
        ));
    }

    #[test]
    fn unavailable_balance_is_left_off_not_drawn_at_zero() {
        let usage = reading_for(envelope(serde_json::json!([{
            "id": "b1",
            "interval": "daily",
            "enforcement": "hard",
            "limit_usd": 10,
            "applies_to": [{"api_key_id": "k1", "project_id": "p1"}],
            "availability": {"status": "unavailable", "spent_usd": 1, "reserved_usd": 0},
        }])));
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::NoLimitsReported)
        ));
    }

    #[test]
    fn enforced_budget_binds_before_a_monitored_one() {
        let usage = reading_for(envelope(serde_json::json!([
            {
                "id": "watch",
                "interval": "daily",
                "enforcement": "monitor",
                "limit_usd": 10,
                "applies_to": [{"api_key_id": "k1", "project_id": "p1"}],
                "spend_status": "available",
                "spend_usd": 9,
            },
            {
                "id": "cap",
                "interval": "daily",
                "enforcement": "hard",
                "limit_usd": 10,
                "applies_to": [{"api_key_id": "k1", "project_id": "p1"}],
                "availability": {"status": "available", "spent_usd": 2, "reserved_usd": 0},
            },
        ])));
        let windows = live_windows(&usage);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "aixy.cap");
    }

    #[test]
    fn between_equals_the_one_nearer_its_limit_binds() {
        let usage = reading_for(envelope(serde_json::json!([
            {
                "id": "low",
                "interval": "daily",
                "enforcement": "monitor",
                "limit_usd": 10,
                "applies_to": [{"api_key_id": "k1", "project_id": "p1"}],
                "spend_status": "available",
                "spend_usd": 3,
            },
            {
                "id": "high",
                "interval": "daily",
                "enforcement": "monitor",
                "limit_usd": 10,
                "applies_to": [{"api_key_id": "k1", "project_id": "p1"}],
                "spend_status": "available",
                "spend_usd": 7,
            },
        ])));
        let windows = live_windows(&usage);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "aixy.high");
    }

    #[test]
    fn distinct_periods_each_draw_one_row() {
        let usage = reading_for(envelope(serde_json::json!([
            {
                "id": "day",
                "interval": "daily",
                "enforcement": "monitor",
                "limit_usd": 10,
                "applies_to": [{"api_key_id": "k1", "project_id": "p1"}],
                "spend_status": "available",
                "spend_usd": 1,
            },
            {
                "id": "life",
                "interval": "lifetime",
                "enforcement": "hard",
                "limit_usd": 100,
                "applies_to": [{"api_key_id": "k1", "project_id": "p1"}],
                "availability": {"status": "available", "spent_usd": 50, "reserved_usd": 0},
            },
        ])));
        let windows = live_windows(&usage);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].id, "aixy.day");
        assert_eq!(windows[1].id, "aixy.life");
    }

    #[test]
    fn wrong_envelope_is_unreadable_not_empty() {
        let mut wrong_currency = envelope(serde_json::json!([]));
        wrong_currency["currency"] = serde_json::json!("EUR");
        for reply in [
            wrong_currency,
            serde_json::json!({"object": "other.thing", "currency": "USD"}),
            serde_json::json!({"object": "key.usage", "currency": "USD"}),
        ] {
            let usage = reading_for(reply);
            assert!(matches!(
                usage.state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }
}
