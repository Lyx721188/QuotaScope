//! Bifrost, a self-hosted AI gateway: the budgets its governance puts on one
//! virtual key — dollars spent out of dollars allowed, per period.
//!
//! Read with the virtual key the user pastes and the gateway address they
//! enter, from the gateway's own governance route:
//! `GET <address>/api/governance/virtual-keys/quota`, key in `x-bf-vk`. The
//! key goes to that address and nowhere else. The shape is second-hand —
//! taken from CodexBar's Bifrost plugin, not from a captured reply — and the
//! fixtures in the tests say so.
//!
//! **Budgets only.** The key's rate limits are left off: a token limit and a
//! request limit over the same period would both be named "1-hour limit",
//! and nothing here can tell the reader which is which without a unit label
//! QuotaScope does not have. The per-model spend breakdown is spend with no
//! limit and has nowhere to go either.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::gateway;
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde::Deserialize;
use std::sync::Arc;

const PATH: &str = "/api/governance/virtual-keys/quota";

/// The named lengths a period can state: five hours is its own kind, and a
/// day reads as a day even when the gateway wrote `24h`.
const FIVE_HOURS: i64 = 5 * 3_600;
const DAY: i64 = 86_400;
const WEEK: i64 = 7 * DAY;

pub struct BifrostService {
    http: Arc<HttpClient>,
}

impl BifrostService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for BifrostService {
    fn provider(&self) -> Provider {
        Provider::Bifrost
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Bifrost);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Bifrost)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let Some(address) = keys
            .address(Provider::Bifrost)
            .filter(|s| !s.trim().is_empty())
        else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressMissing);
        };
        // People paste the OpenAI-compatible base, which ends in `/v1`.
        let Some(endpoint) = gateway::url_from(&address, PATH, &["/v1"]) else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressRefused);
        };
        let headers = [("x-bf-vk", key.as_str()), ("Accept", "application/json")];
        let reply = match self
            .http
            .fetch_json(crate::http::Method::Get, &endpoint, &headers, None)
        {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        match windows(&reply, crate::timeutil::now_ms()) {
            Ok(list) => {
                let mut usage = ProviderUsage::live_now(account, list);
                usage.origin = Some(self.origin_token().into());
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// One budget as the gateway keeps it on a key, an upstream provider or a
/// model. Decoded strictly: a figure that arrived in the wrong shape fails
/// the whole reply rather than reading as zero, which would draw a spent
/// limit out of nothing.
#[derive(Deserialize)]
struct Budget {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    max_limit: Option<f64>,
    #[serde(default)]
    current_usage: Option<f64>,
    #[serde(default)]
    reset_duration: Option<String>,
    #[serde(default)]
    last_reset: Option<String>,
    #[serde(default)]
    override_amount: Option<f64>,
    #[serde(default)]
    override_mode: Option<String>,
    #[serde(default)]
    override_cycles_remaining: Option<f64>,
}

/// A budget scoped to one upstream provider or one model.
#[derive(Deserialize)]
struct Scoped {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    budgets: Option<Vec<Budget>>,
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    is_active: Option<bool>,
    #[serde(default)]
    budgets: Option<Vec<Budget>>,
    #[serde(default)]
    provider_configs: Option<Vec<Scoped>>,
    #[serde(default)]
    model_configs: Option<Vec<Scoped>>,
}

/// The budgets the gateway puts on the key: its own, then those it has per
/// upstream provider and per model, each scoped by the name the gateway
/// gives it. Shortest period first, so the ring reads the tightest limit.
///
/// An error is the reading's verdict: an unreadable reply, a refused key
/// (an inactive key with nothing on it), or no limits left to report.
pub fn windows(reply: &serde_json::Value, now_ms: i64) -> Result<Vec<UsageWindow>, Unavailability> {
    // A JSON array would otherwise slip through the derived decoder's
    // sequence form and read as an empty reply; a reply that isn't an
    // object can't be read.
    if !reply.is_object() {
        return Err(Unavailability::UnreadableReply);
    }
    let reply: Reply =
        serde_json::from_value(reply.clone()).map_err(|_| Unavailability::UnreadableReply)?;

    let mut scoped: Vec<(Option<String>, String, &Budget)> = Vec::new();
    for budget in reply.budgets.iter().flatten() {
        scoped.push((None, String::new(), budget));
    }
    for (index, config) in reply.provider_configs.iter().flatten().enumerate() {
        let name = clean(config.provider.as_deref());
        for budget in config.budgets.iter().flatten() {
            scoped.push((name.clone(), format!("provider{index}."), budget));
        }
    }
    for (index, config) in reply.model_configs.iter().flatten().enumerate() {
        let name =
            clean(config.model_name.as_deref()).or_else(|| clean(config.provider.as_deref()));
        for budget in config.budgets.iter().flatten() {
            scoped.push((name.clone(), format!("model{index}."), budget));
        }
    }

    let mut list: Vec<UsageWindow> = scoped
        .iter()
        .filter_map(|(scope, id_prefix, budget)| {
            window(budget, scope.as_deref(), id_prefix, now_ms)
        })
        .collect();
    list.sort_by_key(|w| w.window_seconds);

    if list.is_empty() {
        // An inactive key with nothing on it is a key the gateway won't
        // honour, not a key with no limits.
        return Err(if reply.is_active == Some(false) {
            Unavailability::ApiKeyRefused
        } else {
            Unavailability::NoLimitsReported
        });
    }
    Ok(list)
}

fn window(
    budget: &Budget,
    scope: Option<&str>,
    id_prefix: &str,
    now_ms: i64,
) -> Option<UsageWindow> {
    let id = clean(budget.id.as_deref())?;
    let used = budget
        .current_usage
        .filter(|v| v.is_finite() && *v >= 0.0)?;
    let base = budget.max_limit.filter(|v| v.is_finite())?;

    // A temporary raise the gateway reports on top of the budget counts
    // while it is in force: for good, or for cycles it says remain.
    let mut limit = base;
    if let Some(extra) = budget.override_amount.filter(|v| v.is_finite() && *v > 0.0) {
        let cycles = budget.override_cycles_remaining.unwrap_or(0.0);
        if budget.override_mode.as_deref() == Some("forever")
            || (budget.override_mode.as_deref() == Some("cycles") && cycles > 0.0)
        {
            limit += extra;
        }
    }
    if limit <= 0.0 {
        return None;
    }

    let period = Period::parse(budget.reset_duration.as_deref());
    let resets_at = if period.is_fixed {
        // The reset that follows the one the gateway reports, if it is still
        // ahead. Only for a fixed-length period: one counted in days or
        // longer may be aligned to the calendar, and the reply does not say.
        next_reset(
            budget
                .last_reset
                .as_deref()
                .and_then(crate::timeutil::parse_iso8601_ms),
            period.seconds,
            now_ms,
        )
    } else {
        None
    };
    let mut window = UsageWindow::new(
        &format!("bifrost.{id_prefix}{id}"),
        period.kind,
        scope.map(str::to_string),
        used / limit,
        period.seconds,
        resets_at,
    );
    window.reports_length = period.reports_length;
    window.is_exhausted = used >= limit;
    Some(window)
}

/// The reset that follows the one the gateway reports, if it is still ahead.
/// Already past: the gateway hasn't moved it on, and neither do we.
pub fn next_reset(last: Option<i64>, period_seconds: i64, now_ms: i64) -> Option<i64> {
    let last = last?;
    if period_seconds <= 0 {
        return None;
    }
    let next = last + period_seconds * 1000;
    (next > now_ms).then_some(next)
}

/// A budget's `reset_duration`, as Bifrost writes it: a Go duration such as
/// `1h` or `30m`, or a count of days, weeks, months, quarters or years
/// (`1d`, `1w`, `1M`, `1Q`, `1Y`).
pub struct Period {
    pub kind: Kind,
    pub seconds: i64,
    pub reports_length: bool,
    /// Counted in hours or less, so its next reset follows from the last.
    pub is_fixed: bool,
}

impl Period {
    /// A budget with no length this can state: a spend limit, sorted by
    /// `seconds`.
    fn unstated(sort_key: i64) -> Period {
        Period {
            kind: Kind::Spend,
            seconds: sort_key,
            reports_length: false,
            is_fixed: false,
        }
    }

    pub fn parse(raw: Option<&str>) -> Period {
        let text = raw.unwrap_or("").trim();
        if let Some(seconds) = fixed_seconds(text) {
            // Named by its length only when that is a whole number of hours:
            // "30m" read as a "1-hour limit" would be a claim. Upstream's
            // `.daily` case has no Windows `Kind`; the day as `other` is the
            // stand-in the rest of the port already uses.
            if seconds % 3_600 != 0 {
                return Period {
                    kind: Kind::Spend,
                    seconds,
                    reports_length: false,
                    is_fixed: true,
                };
            }
            let kind = match seconds {
                FIVE_HOURS => Kind::FiveHour,
                DAY => Kind::Other(DAY),
                other => Kind::Other(other),
            };
            return Period {
                kind,
                seconds,
                reports_length: true,
                is_fixed: true,
            };
        }

        let mut chars = text.chars();
        let Some(unit) = chars.next_back() else {
            return Self::unstated(30 * 86_400);
        };
        let count: i64 = match chars.as_str().parse() {
            Ok(count) if count > 0 && count < 1_000 => count,
            _ => return Self::unstated(30 * 86_400),
        };
        match (unit, count) {
            ('d', 1) => Period {
                kind: Kind::Other(DAY),
                seconds: DAY,
                reports_length: true,
                is_fixed: false,
            },
            ('d', 7) | ('w', 1) => Period {
                kind: Kind::Weekly,
                seconds: WEEK,
                reports_length: true,
                is_fixed: false,
            },
            ('d', _) => Period {
                kind: Kind::Other(count * DAY),
                seconds: count * DAY,
                reports_length: true,
                is_fixed: false,
            },
            ('w', _) => Period {
                kind: Kind::Other(count * WEEK),
                seconds: count * WEEK,
                reports_length: true,
                is_fixed: false,
            },
            // A month is not a fixed length: a name and a sort key only.
            ('M', 1) => Period {
                kind: Kind::Monthly,
                seconds: 30 * 86_400,
                reports_length: false,
                is_fixed: false,
            },
            // Several months, a quarter or a year: a length nothing here can
            // name without claiming a number of days.
            ('M', _) => Self::unstated(count * 30 * 86_400),
            ('Q', _) => Self::unstated(count * 90 * 86_400),
            ('Y', _) => Self::unstated(count * 365 * 86_400),
            _ => Self::unstated(30 * 86_400),
        }
    }
}

/// `1h`, `90m`, `1h30m`: whole seconds, or None if it isn't one. The `ms`
/// guard is what keeps a Go millisecond duration out of the minutes slot.
fn fixed_seconds(text: &str) -> Option<i64> {
    const UNITS: [(char, f64); 3] = [('h', 3_600.0), ('m', 60.0), ('s', 1.0)];
    let mut rest = text;
    let mut total = 0.0;
    let mut matched = false;
    while !rest.is_empty() {
        let digits_len: usize = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .map(|c| c.len_utf8())
            .sum();
        if digits_len == 0 {
            return None;
        }
        let value: f64 = rest[..digits_len].parse().ok()?;
        rest = &rest[digits_len..];
        let (_, multiplier) = UNITS
            .into_iter()
            .find(|(unit, _)| rest.starts_with(*unit) && !rest.starts_with("ms"))?;
        rest = &rest[1..];
        total += value * multiplier;
        matched = true;
    }
    if !matched || !total.is_finite() || !(1.0..(i32::MAX as f64)).contains(&total) {
        return None;
    }
    Some(total.round() as i64)
}

/// A name the gateway gives something, blank ones as none.
fn clean(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> i64 {
        crate::timeutil::parse_iso8601_ms("2026-06-10T12:30:00Z").unwrap()
    }

    /// The shape pinned by upstream's `bifrost-quota` fixture — second-hand,
    /// written from CodexBar's Bifrost plugin, not captured from a live
    /// gateway.
    fn fixture() -> serde_json::Value {
        serde_json::json!({
            "virtual_key_name": "team-key",
            "is_active": true,
            "budgets": [
                {"id": "b-monthly", "max_limit": 100, "current_usage": 25, "reset_duration": "1M", "last_reset": "2026-06-01T00:00:00Z", "source_name": "Team"},
                {"id": "b-hourly", "max_limit": 5, "current_usage": 1, "reset_duration": "1h", "last_reset": "2026-06-10T12:00:00Z", "override_amount": 5, "override_mode": "cycles", "override_cycles_remaining": 2},
                {"id": "b-empty", "max_limit": 0, "current_usage": 3, "reset_duration": "1d"}
            ],
            "model_configs": [
                {"provider": "openai", "model_name": "gpt-4o", "budgets": [
                    {"id": "m-weekly", "max_limit": 20, "current_usage": 10, "reset_duration": "1w"}
                ]}
            ],
            "rate_limits": [{"token_max_limit": 100000, "token_current_usage": 500, "token_reset_duration": "1h"}]
        })
    }

    #[test]
    fn budgets_are_read_as_spent_out_of_allowed_shortest_first() {
        let list = windows(&fixture(), now()).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(
            list.iter().map(|w| w.kind.clone()).collect::<Vec<_>>(),
            vec![Kind::Other(3_600), Kind::Weekly, Kind::Monthly]
        );
        // 1 of 5 + a 5 raise still in force; 10 of 20; 25 of 100.
        assert_eq!(
            list.iter().map(|w| w.used_fraction).collect::<Vec<_>>(),
            vec![0.1, 0.5, 0.25]
        );
        assert_eq!(
            list.iter().map(|w| w.scope.clone()).collect::<Vec<_>>(),
            vec![None, Some("gpt-4o".to_string()), None]
        );
        // The rate limits and the zero budget are left off.
        assert!(!list.iter().any(|w| w.id.contains("b-empty")));
        // A month is a name and a sort key, not a length.
        assert!(!list[2].reports_length);
    }

    #[test]
    fn a_fixed_periods_next_reset_follows_the_last_a_calendar_ones_is_not_guessed() {
        let list = windows(&fixture(), now()).unwrap();
        assert_eq!(
            list[0].resets_at,
            crate::timeutil::parse_iso8601_ms("2026-06-10T13:00:00Z")
        );
        assert_eq!(list[1].resets_at, None);
        assert_eq!(list[2].resets_at, None);
        // Already past: the gateway hasn't moved it on, and neither do we.
        let past = crate::timeutil::parse_iso8601_ms("2026-06-10T10:00:00Z");
        assert_eq!(next_reset(past, 3_600, now()), None);
    }

    #[test]
    fn durations_read_as_bifrost_writes_them() {
        let cases: &[(&str, Kind, i64, bool, bool)] = &[
            // (raw, kind, seconds, reports_length, is_fixed)
            ("5h", Kind::FiveHour, 5 * 3_600, true, true),
            ("24h", Kind::Other(86_400), 86_400, true, true),
            ("1d", Kind::Other(86_400), 86_400, true, false),
            ("7d", Kind::Weekly, 7 * 86_400, true, false),
            ("2w", Kind::Other(14 * 86_400), 14 * 86_400, true, false),
            ("1h30m", Kind::Spend, 5_400, false, true),
            ("30m", Kind::Spend, 1_800, false, true),
            ("1M", Kind::Monthly, 30 * 86_400, false, false),
            ("1Q", Kind::Spend, 90 * 86_400, false, false),
            ("nonsense", Kind::Spend, 30 * 86_400, false, false),
        ];
        for (raw, kind, seconds, reports, fixed) in cases {
            let period = Period::parse(Some(raw));
            assert_eq!(period.kind, *kind, "{raw}");
            assert_eq!(period.seconds, *seconds, "{raw}");
            assert_eq!(period.reports_length, *reports, "{raw}");
            assert_eq!(period.is_fixed, *fixed, "{raw}");
        }
    }

    #[test]
    fn an_inactive_key_with_nothing_on_it_is_refused() {
        let reply = serde_json::json!({"is_active": false});
        assert_eq!(windows(&reply, now()), Err(Unavailability::ApiKeyRefused));
    }

    #[test]
    fn a_reply_that_isnt_an_object_cant_be_read() {
        for reply in [
            serde_json::Value::Null,
            serde_json::json!([]),
            serde_json::json!({"budgets": {}}),
        ] {
            assert_eq!(windows(&reply, now()), Err(Unavailability::UnreadableReply));
        }
    }

    #[test]
    fn negative_or_missing_figures_are_left_off_and_nothing_left_is_no_limits() {
        let reply = serde_json::json!({
            "budgets": [
                {"id": "a", "max_limit": 10, "current_usage": -1},
                {"id": "b", "current_usage": 1},
                {"max_limit": 5, "current_usage": 1}
            ]
        });
        assert_eq!(
            windows(&reply, now()),
            Err(Unavailability::NoLimitsReported)
        );
    }

    #[test]
    fn the_pasted_v1_base_leads_to_the_governance_route() {
        // People paste the OpenAI-compatible base; it must not be doubled.
        assert_eq!(
            gateway::url_from("https://gw.example.com/v1/", PATH, &["/v1"]).as_deref(),
            Some("https://gw.example.com/api/governance/virtual-keys/quota")
        );
        // A public host over plain http may not carry the key at all.
        assert!(gateway::url_from("http://gw.example.com", PATH, &["/v1"]).is_none());
    }
}
