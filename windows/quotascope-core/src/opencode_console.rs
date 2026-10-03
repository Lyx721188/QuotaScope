//! OpenCode's console session is separate from its Go API key. Only the
//! quota and billing fields are retained; request headers and locations are not.
use crate::http::{number, HttpClient, Method};
use crate::ledger::{LedgerDay, TokenTally, UsageLedger};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

pub const SECRET: &str = "openCodeGo:console";
pub const HOSTS: &[&str] = &["opencode.ai"];
pub const COOKIES: &[&str] = &["__Host-console_session", "auth"];
const BASE: &str = "https://opencode.ai/console/api";

pub fn cookie(header: &str) -> Option<String> {
    let pairs: Vec<_> = header
        .trim()
        .trim_start_matches("Cookie:")
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .filter(|(name, value)| COOKIES.contains(name) && !value.trim().is_empty())
        .map(|(name, value)| format!("{name}={}", value.trim()))
        .collect();
    pairs
        .iter()
        .any(|p| p.starts_with("__Host-console_session="))
        .then(|| pairs.join("; "))
}

fn session_problem(reason: Unavailability) -> Unavailability {
    if reason == Unavailability::ApiKeyRefused {
        Unavailability::SessionExpired
    } else {
        reason
    }
}

fn get(http: &HttpClient, cookie: &str, org: &str, url: &str) -> Result<Value, Unavailability> {
    http.fetch_json(
        Method::Get,
        url,
        &[
            ("Cookie", cookie),
            ("x-org-id", org),
            ("Accept", "application/json"),
        ],
        None,
    )
    .map_err(session_problem)
}

fn workspace(http: &HttpClient, cookie: &str) -> Result<String, Unavailability> {
    let root = get(http, cookie, "", &format!("{BASE}/orgs"))?;
    let orgs: Vec<_> = root
        .as_array()
        .ok_or(Unavailability::UnreadableReply)?
        .iter()
        .filter_map(|org| org.get("id").and_then(Value::as_str))
        .filter(|id| !id.is_empty())
        .collect();
    let first = orgs.first().ok_or(Unavailability::NoPlan)?;
    if orgs.len() > 1 {
        for org in &orgs {
            match get(http, cookie, org, &format!("{BASE}/go/status")) {
                Ok(status) if status.get("access").is_some_and(Value::is_object) => {
                    return Ok((*org).to_string())
                }
                Err(Unavailability::SessionExpired) => return Err(Unavailability::SessionExpired),
                _ => {}
            }
        }
    }
    Ok((*first).to_string())
}

pub fn usage(http: &HttpClient, header: &str) -> ProviderUsage {
    let account = AccountKey::primary(Provider::OpenCodeGo);
    let result = (|| {
        let cookie = cookie(header).ok_or(Unavailability::SessionMissing)?;
        let org = workspace(http, &cookie)?;
        get(http, &cookie, &org, &format!("{BASE}/go/status"))
    })();
    match result {
        Err(reason) => ProviderUsage::unavailable(account, reason),
        Ok(reply) => {
            let windows = windows(&reply);
            if windows.is_empty() {
                return ProviderUsage::unavailable(
                    account,
                    if reply.get("access").is_some_and(Value::is_object) {
                        Unavailability::NoLimitsReported
                    } else {
                        Unavailability::NoPlan
                    },
                );
            }
            let mut usage = ProviderUsage::live_now(account, windows);
            usage.origin = Some("webSession".into());
            usage.plan =
                (reply.get("product").and_then(Value::as_str) == Some("go")).then(|| "Go".into());
            usage
        }
    }
}

pub fn windows(reply: &Value) -> Vec<UsageWindow> {
    let access = &reply["access"];
    let meters = &access["meters"];
    [
        ("fiveHour", "rolling", Kind::FiveHour, 18000),
        ("week", "weekly", Kind::Weekly, 604800),
        ("month", "monthly", Kind::Monthly, 2592000),
    ]
    .into_iter()
    .filter_map(|(name, id, kind, nominal)| {
        let meter = &meters[name];
        let limit = number(&meter["limitMicroCents"]).filter(|n| n.is_finite() && *n > 0.0)?;
        let used = number(&meter["usedMicroCents"]).filter(|n| n.is_finite() && *n >= 0.0)?;
        let reset = meter["resetsAt"]
            .as_str()
            .and_then(crate::timeutil::parse_iso8601_ms);
        let start = if name == "month" && access["endsAt"] == meter["resetsAt"] {
            &access["startsAt"]
        } else {
            &meter["startsAt"]
        };
        let length = start
            .as_str()
            .and_then(crate::timeutil::parse_iso8601_ms)
            .zip(reset)
            .and_then(|(s, r)| r.checked_sub(s))
            .map(|ms| ms / 1000)
            .filter(|s| *s > 0);
        let mut window = UsageWindow::new(
            id,
            kind,
            None,
            (used / limit).clamp(0.0, 1.0),
            length.unwrap_or(nominal),
            reset,
        );
        window.reports_length = length.is_some();
        window.is_exhausted = used >= limit;
        Some(window)
    })
    .collect()
}

pub fn history() -> crate::history::HistoryRead {
    let Some(header) = crate::secrets::key_for(SECRET).and_then(|c| cookie(&c)) else {
        return crate::history::HistoryRead::NotConfigured;
    };
    let http = HttpClient::new();
    let result = workspace(&http, &header).and_then(|org| {
        request_history(crate::timeutil::now_ms(), |since, until, cursor| {
            let mut url = url::Url::parse(&format!("{BASE}/request-logs"))
                .map_err(|_| Unavailability::UnreadableReply)?;
            url.query_pairs_mut()
                .append_pair("since", &since.to_string())
                .append_pair("category", "inference")
                .append_pair("limit", "100");
            if let Some(until) = until {
                url.query_pairs_mut()
                    .append_pair("until", &until.to_string());
            }
            if let Some(cursor) = cursor {
                url.query_pairs_mut().append_pair("cursor", cursor);
            }
            get(&http, &header, &org, url.as_str())
        })
    });
    match result {
        Ok(ledger) => crate::history::HistoryRead::Answered {
            ledger,
            account_wide: true,
            actual_costs: true,
            currency: Some("USD".into()),
        },
        Err(reason) => crate::history::HistoryRead::Failed(reason),
    }
}

fn request_history(
    now: i64,
    mut fetch: impl FnMut(i64, Option<i64>, Option<&str>) -> Result<Value, Unavailability>,
) -> Result<UsageLedger, Unavailability> {
    let since = now.saturating_sub(31 * 86400000);
    let mut until = None;
    let mut cursor: Option<String> = None;
    let mut seen = HashSet::new();
    let mut items = BTreeMap::new();
    let mut partial = false;
    for page in 0..200 {
        if !crate::scan::checkpoint() {
            partial = true;
            break;
        }
        let root = match fetch(since, until, cursor.as_deref()) {
            Ok(v) => v,
            Err(reason) if page > 0 && reason != Unavailability::SessionExpired => {
                partial = true;
                break;
            }
            Err(reason) => return Err(reason),
        };
        let rows = root["items"]
            .as_array()
            .ok_or(Unavailability::UnreadableReply)?;
        for row in rows {
            if let Some(id) = row["id"].as_str().filter(|s| !s.is_empty()) {
                // Decode into the small billing record immediately: never retain
                // the request's headers, geography, or service API key identity.
                if let Some(item) = bill(row, since, now) {
                    items.insert(id.to_string(), item);
                }
            }
        }
        until = until.or_else(|| root["until"].as_i64());
        let Some(next) = root["nextCursor"].as_str().filter(|s| !s.is_empty()) else {
            break;
        };
        if rows.is_empty() || !seen.insert(next.to_string()) || page == 199 {
            partial = true;
            break;
        }
        cursor = Some(next.to_string());
    }
    let mut ledger = UsageLedger::empty();
    let mut days: BTreeMap<_, LedgerDay> = BTreeMap::new();
    for (date, model, tally, cost) in items.into_values() {
        let day = days.entry(date).or_insert_with(|| LedgerDay {
            date,
            tokens: 0,
            cost: 0.0,
            unpriced_tokens: 0,
            models: BTreeMap::new(),
            tally: TokenTally::default(),
            model_tallies: BTreeMap::new(),
            model_costs: BTreeMap::new(),
        });
        let tokens = tally.total();
        day.tokens = day.tokens.saturating_add(tokens);
        day.tally += tally;
        *day.models.entry(model.clone()).or_default() += tokens;
        *day.model_tallies.entry(model).or_default() += tally;
        if let Some(cost) = cost {
            day.cost += cost;
        } else {
            day.unpriced_tokens = day.unpriced_tokens.saturating_add(tokens);
        }
    }
    ledger.earliest = days.keys().next().copied();
    ledger.days = days.into_values().collect();
    ledger.has_partial_records = partial;
    Ok(ledger)
}

fn bill(
    row: &Value,
    since: i64,
    now: i64,
) -> Option<(chrono::NaiveDate, String, TokenTally, Option<f64>)> {
    if row["product"].as_str()? != "go" {
        return None;
    }
    let at = row["startedAt"]
        .as_i64()
        .filter(|t| *t >= since && *t <= now)?;
    let date = chrono::DateTime::from_timestamp_millis(at)?
        .with_timezone(&chrono::Local)
        .date_naive();
    let n = |key: &str| row[key].as_i64().unwrap_or(0).clamp(0, 1_000_000_000_000);
    let tally = TokenTally {
        input: n("inputTokens"),
        output: n("outputTokens"),
        cache_read: n("cacheReadTokens"),
        cache_write: n("cacheWriteTokens"),
    };
    let cost = number(&row["cost"]).filter(|c| c.is_finite() && *c >= 0.0);
    Some((
        date,
        row["model"].as_str().unwrap_or("unknown").to_string(),
        tally,
        cost,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn console_session_drops_unrelated_cookies_and_measures_only_stated_windows() {
        assert_eq!(
            cookie("auth=a; _stripe=b; __Host-console_session=c"),
            Some("auth=a; __Host-console_session=c".into())
        );
        assert!(cookie("auth=a").is_none());
        let windows = windows(
            &json!({"access":{"meters":{"fiveHour":{"limitMicroCents":"100", "usedMicroCents":"150", "startsAt":"2026-10-01T00:00:00Z", "resetsAt":"2026-10-01T05:00:00Z"},"month":{"limitMicroCents":"100","usedMicroCents":"25"}}}}),
        );
        assert!(windows[0].reports_length && windows[0].is_exhausted);
        assert!(!windows[1].reports_length);
        assert_eq!(windows[1].used_fraction, 0.25);
    }
    #[test]
    fn cursor_duplicates_replace_and_actual_bill_ignores_api_prices() {
        let now = 1790850000000;
        let mut pages = vec![json!({"items":[{"id":"r1","product":"go","startedAt":now,"model":"unpriced","inputTokens":10,"cost":0.3}],"nextCursor":"opaque"}), json!({"items":[{"id":"r1","product":"go","startedAt":now,"model":"unpriced","inputTokens":12,"cost":0.4},{"id":"r2","product":"zen","startedAt":now,"inputTokens":9000,"cost":50.0}]})].into_iter();
        let ledger = request_history(now, |_, _, _| Ok(pages.next().unwrap())).unwrap();
        assert_eq!(ledger.days[0].tokens, 12);
        assert_eq!(ledger.days[0].cost, 0.4);
        assert_eq!(ledger.days[0].unpriced_tokens, 0);
        assert!(!ledger.has_partial_records);
    }
    #[test]
    fn repeated_cursor_is_partial_and_expired_auth_never_returns_old_bills() {
        let now = 1790850000000;
        let page = json!({"items":[{"id":"r1","product":"go","startedAt":now,"inputTokens":7}],"nextCursor":"same"});
        assert!(
            request_history(now, |_, _, _| Ok(page.clone()))
                .unwrap()
                .has_partial_records
        );
        let mut call = 0;
        assert!(matches!(
            request_history(now, |_, _, _| {
                call += 1;
                if call == 1 {
                    Ok(page.clone())
                } else {
                    Err(Unavailability::SessionExpired)
                }
            }),
            Err(Unavailability::SessionExpired)
        ));
    }
}
