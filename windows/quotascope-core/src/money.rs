//! Estimate-only currency conversion, adapted from Pulse 338c796.
//! Provider balances and actual bills keep their reported currency.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
pub const CURRENCIES: [&str; 7] = ["USD", "CNY", "HKD", "TWD", "JPY", "KRW", "RUB"];
pub const ATTRIBUTION: &str = "https://www.exchangerate-api.com";
const SOURCE: &str = "https://open.er-api.com/v6/latest/USD";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rates {
    pub updated_at: i64,
    pub next_update_at: i64,
    pub fetched_at: i64,
    pub rates: BTreeMap<String, f64>,
}
#[derive(Default)]
struct State {
    kept: Option<Rates>,
    attempted: Option<i64>,
    running: bool,
}
static CHANGED: AtomicBool = AtomicBool::new(false);
fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        let kept = std::fs::File::open(crate::data_dir().join("exchange-rates.json"))
            .ok()
            .and_then(|file| serde_json::from_reader::<_, Rates>(file.take(65536)).ok())
            .filter(|r| {
                r.rates.iter().all(|(code, n)| {
                    CURRENCIES.contains(&code.as_str()) && n.is_finite() && *n > 0.0
                })
            });
        Mutex::new(State {
            kept,
            ..Default::default()
        })
    })
}
pub fn parse(root: &serde_json::Value, now: i64) -> Option<Rates> {
    if root["result"] != "success" || root["base_code"] != "USD" {
        return None;
    }
    let stamp = |key: &str| {
        root[key]
            .as_i64()
            .filter(|n| *n > 0 && *n <= 253402300799)
            .and_then(|n| n.checked_mul(1000))
    };
    let updated_at = stamp("time_last_update_unix")?;
    let next_update_at = stamp("time_next_update_unix")?;
    if next_update_at < updated_at {
        return None;
    }
    let rates = CURRENCIES
        .iter()
        .filter(|c| **c != "USD")
        .filter_map(|c| {
            Some((
                c.to_string(),
                root["rates"][*c]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n > 0.0)?,
            ))
        })
        .collect::<BTreeMap<_, _>>();
    if rates.is_empty() {
        return None;
    }
    Some(Rates {
        updated_at,
        next_update_at,
        fetched_at: now,
        rates,
    })
}
pub fn manual_rate(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    text.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n > 0.0)
}
pub fn changed() {
    CHANGED.store(true, Ordering::Release);
}
pub fn take_changed() -> bool {
    CHANGED.swap(false, Ordering::AcqRel)
}
pub fn kept_rates() -> Option<Rates> {
    state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .kept
        .clone()
}
pub fn converted(
    usd: f64,
    code: &str,
    manual: Option<f64>,
    rates: Option<&Rates>,
) -> (f64, String) {
    let rate = if code == "USD" {
        Some(1.0)
    } else {
        manual.filter(|n| n.is_finite() && *n > 0.0).or_else(|| {
            rates?
                .rates
                .get(code)
                .copied()
                .filter(|n| n.is_finite() && *n > 0.0)
        })
    };
    if let Some(rate) = rate {
        let value = usd * rate;
        if value.is_finite() {
            return (value, code.into());
        }
    }
    (usd, "USD".into())
}
pub fn format(amount: f64, code: &str, whole_from: f64) -> String {
    if !amount.is_finite() {
        return "—".into();
    }
    let symbol = match code {
        "USD" => "$",
        "CNY" => "¥",
        "HKD" => "HK$",
        "TWD" => "NT$",
        "JPY" => "¥",
        "KRW" => "₩",
        "RUB" => "₽",
        _ => code,
    };
    let cents = !matches!(code, "JPY" | "KRW");
    let unit = if cents { 0.01 } else { 1.0 };
    let places = if cents && amount.abs() < whole_from {
        2
    } else {
        0
    };
    if amount > 0.0 && amount < unit {
        return format!(
            "< {symbol}{unit:.places$}",
            places = if cents { 2 } else { 0 }
        );
    }
    format!("{symbol}{amount:.places$}")
}
pub fn estimate(usd: f64) -> String {
    estimate_with_threshold(usd, 1000.0)
}
pub fn estimate_with_threshold(usd: f64, whole_from: f64) -> String {
    let (code, manual) = crate::settings::with(|s| {
        (
            s.display_currency.clone(),
            s.manual_exchange_rates.get(&s.display_currency).copied(),
        )
    });
    let state = state().lock().unwrap_or_else(|e| e.into_inner());
    let (amount, code) = converted(usd, &code, manual, state.kept.as_ref());
    format(amount, &code, whole_from)
}
/// Called from the app poll; all network and disk work is off the UI thread.
pub fn ensure_rates() {
    let (code, manual) = crate::settings::with(|s| {
        (
            s.display_currency.clone(),
            s.manual_exchange_rates.get(&s.display_currency).copied(),
        )
    });
    if code == "USD" || manual.is_some_and(|n| n.is_finite() && n > 0.0) {
        return;
    }
    let now = crate::timeutil::now_ms();
    {
        let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
        if state.running
            || state
                .attempted
                .is_some_and(|at| now.saturating_sub(at) < 20 * 60 * 1000)
            || state.kept.as_ref().is_some_and(|r| {
                now < r.next_update_at || now.saturating_sub(r.fetched_at) < 3600 * 1000
            })
        {
            return;
        }
        state.running = true;
        state.attempted = Some(now);
    }
    std::thread::spawn(move || {
        let fresh = crate::http::HttpClient::new()
            .fetch_json(crate::http::Method::Get, SOURCE, &[], None)
            .ok()
            .and_then(|root| parse(&root, now));
        if let Some(fresh) = &fresh {
            let path = crate::data_dir().join("exchange-rates.json");
            if let Ok(bytes) = serde_json::to_vec(fresh) {
                let _ = std::fs::create_dir_all(crate::data_dir());
                let temporary = path.with_extension("json.tmp");
                if std::fs::write(&temporary, bytes).is_ok() {
                    let _ = std::fs::rename(temporary, path);
                }
            }
        }
        let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
        state.running = false;
        if let Some(fresh) = fresh {
            state.kept = Some(fresh);
            changed();
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversion_uses_typed_then_daily_rates_and_falls_back_to_dollars() {
        let rates=parse(&serde_json::json!({"result":"success","base_code":"USD","time_last_update_unix":1794441600i64,"time_next_update_unix":1794528000i64,"rates":{"CNY":7.0}}),1).unwrap();
        assert_eq!(
            converted(2.0, "CNY", Some(6.5), Some(&rates)),
            (13.0, "CNY".into())
        );
        assert_eq!(
            converted(2.0, "CNY", None, Some(&rates)),
            (14.0, "CNY".into())
        );
        assert_eq!(
            converted(2.0, "JPY", None, Some(&rates)),
            (2.0, "USD".into())
        );
    }
    #[test]
    fn manual_rates_are_whole_positive_numbers_not_stripped_text() {
        for s in ["-7", "+7", "7 yuan", "0", "NaN", "inf", "7.2.1"] {
            assert_eq!(manual_rate(s), None);
        }
        assert_eq!(manual_rate(" 6.71 "), Some(6.71));
    }
    #[test]
    fn tiny_positive_estimates_are_not_free_and_yen_has_no_cents() {
        assert_eq!(format(0.001, "USD", 1000.0), "< $0.01");
        assert_eq!(format(0.1, "JPY", 1000.0), "< ¥1");
        assert_eq!(format(12.6, "JPY", 1000.0), "¥13");
        assert_eq!(format(12.6, "HKD", 1000.0), "HK$12.60");
        assert_eq!(format(0.0, "USD", 1000.0), "$0.00");
    }
}
