//! The balance ring every prepaid account gets, whatever it reports from.
//!
//! Ported from `BalanceRing.swift` and `DeepSeekBalanceBasis.swift`. A
//! prepaid balance is **not a limit**: no ceiling, no window, no reset — the
//! provider says what is left and stops. So the denominator behind the ring
//! comes from somewhere else, and that somewhere is the reader's choice,
//! per account: something QuotaScope watched (**since top-up**), a figure
//! they typed (**budget**), or nothing at all (**balance only**, where the
//! rail shows the money in place of a percentage).
//!
//! The application point is the store, after any service returns: a live
//! reading with money and no windows gets its one balance window here. A
//! reading that already carries windows is left alone — those providers
//! reported a real allowance, which outranks any arithmetic of ours.

use crate::model::{AccountKey, CreditAmount, Estimate, Kind, UsageWindow};
use std::collections::HashMap;
use std::path::PathBuf;

/// The denominators an API account's ring can measure against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// The highest balance QuotaScope has watched: a balance that goes up
    /// can only be a top-up, so the ring starts again from full.
    SinceTopUp,
    /// A figure the reader typed.
    Budget,
    /// No ring at all; the rail shows the money itself.
    BalanceOnly,
}

impl Basis {
    pub fn token(&self) -> &'static str {
        match self {
            Basis::SinceTopUp => "sinceTopUp",
            Basis::Budget => "budget",
            Basis::BalanceOnly => "balanceOnly",
        }
    }

    pub fn from_token(token: &str) -> Basis {
        match token {
            "budget" => Basis::Budget,
            "balanceOnly" => Basis::BalanceOnly,
            _ => Basis::SinceTopUp,
        }
    }
}

/// The basis a given account's ring measures against, as chosen in
/// Settings. DeepSeek's own settings keys came first and keep answering for
/// its primary account; every other account reads the per-account maps.
pub fn basis_for(account: &AccountKey) -> (Basis, Option<f64>) {
    crate::settings::with(|s| {
        let id = account.id();
        if let Some(token) = s.balance_bases.get(&id) {
            return (
                Basis::from_token(token),
                s.balance_budgets.get(&id).copied(),
            );
        }
        if account.provider == crate::model::Provider::DeepSeek && account.is_primary() {
            return (Basis::from_token(&s.deepseek_basis), s.deepseek_budget);
        }
        (Basis::SinceTopUp, None)
    })
}

/// The one balance window, from the money, the basis and the peak. At most
/// one — there is at most one denominator. **No length and no reset, ever.**
pub fn window(
    balance: &CreditAmount,
    basis: Basis,
    budget: Option<f64>,
    peak: f64,
) -> Option<UsageWindow> {
    let (fraction, estimate) = match basis {
        Basis::BalanceOnly => return None,
        Basis::Budget => {
            let budget = budget.filter(|b| b.is_finite() && *b > 0.0)?;
            (
                ((budget - balance.amount) / budget).clamp(0.0, 1.0),
                Estimate::YourBudget,
            )
        }
        Basis::SinceTopUp => {
            if peak <= 0.0 {
                return None;
            }
            (
                ((peak - balance.amount) / peak).clamp(0.0, 1.0),
                Estimate::SinceTopUp,
            )
        }
    };

    let mut window = UsageWindow::new("balance", Kind::Balance, None, fraction, 30 * 86_400, None);
    window.reports_length = false;
    window.estimate = Some(estimate);
    // A balance is never "spent" — the provider saying so is the only word
    // that counts, and a balance ring carries none. A budget the reader set
    // low can reach 100% with money still in the account, so the fraction
    // tops out just under the mark: a full red ring is the provider's
    // verdict, not our arithmetic's.
    window.is_exhausted = false;
    window.used_fraction = window.used_fraction.min(0.99);
    Some(window)
}

/// Whether this reading is the kind the balance ring applies to: alive,
/// money in hand, and no windows of its own.
pub fn applies(reading: &crate::model::ProviderUsage) -> bool {
    use crate::model::State;
    matches!(reading.state, State::Live)
        && reading.windows.is_empty()
        && reading.credit_remaining.is_some()
        && reading.provider() != crate::model::Provider::DeepSeek
        && reading.provider() != crate::model::Provider::CommandCode
}

/// Adds the balance window to a reading that qualifies. The peak advances
/// on every reading, whichever basis is in force: switching to "since
/// top-up" later should find a peak already there rather than start over
/// from whatever the balance happens to be that afternoon.
pub fn apply(reading: &mut crate::model::ProviderUsage, peaks: &mut Peaks) {
    if !applies(reading) {
        return;
    }
    let Some(balance) = reading.credit_remaining.clone() else {
        return;
    };
    let (basis, budget) = basis_for(&reading.account);
    let peak = peaks.advance(
        &reading.account.id(),
        &balance.currency,
        balance.amount,
        crate::timeutil::now_ms(),
    );
    if let Some(window) = window(&balance, basis, budget, peak) {
        reading.windows = vec![window];
    }
    peaks.save();
}

fn peaks_path() -> PathBuf {
    crate::data_dir().join("balance-baseline.json")
}

/// The highest balance QuotaScope has watched, per account and currency.
/// The one measurement the "since top-up" denominator is allowed to rest
/// on. DeepSeek keeps its own file and its own copy of this rule, inherited
/// from before the ring was general.
#[derive(Default, Clone)]
pub struct Peaks {
    marks: HashMap<String, Mark>,
}

#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct Mark {
    peak: f64,
    #[serde(default)]
    set_at: i64,
}

impl Peaks {
    pub fn load() -> Peaks {
        std::fs::read_to_string(peaks_path())
            .ok()
            .and_then(|text| serde_json::from_str::<HashMap<String, Mark>>(&text).ok())
            .map(|marks| Peaks { marks })
            .unwrap_or_default()
    }

    /// Records today's figure and returns the peak to measure against.
    pub fn advance(&mut self, account_id: &str, currency: &str, total: f64, now: i64) -> f64 {
        let key = format!("{account_id}|{currency}");
        let mark = self.marks.entry(key).or_default();
        if total > mark.peak {
            mark.peak = total;
            mark.set_at = now;
        } else if mark.set_at == 0 {
            mark.set_at = now;
        }
        mark.peak
    }

    pub fn save(&self) {
        let dir = crate::data_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(text) = serde_json::to_string(&self.marks) {
            let _ = std::fs::write(peaks_path(), text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn money(amount: f64) -> CreditAmount {
        CreditAmount {
            amount,
            currency: "USD".into(),
        }
    }

    #[test]
    fn since_top_up_measures_against_the_peak() {
        let window = window(&money(4.0), Basis::SinceTopUp, None, 10.0).expect("window");
        assert!((window.used_fraction - 0.6).abs() < 1e-9);
        assert_eq!(window.estimate, Some(Estimate::SinceTopUp));
        assert!(!window.reports_length);
        assert_eq!(window.resets_at, None);
        assert!(!window.is_exhausted);
    }

    #[test]
    fn a_budget_measures_against_the_figure_typed() {
        let w = window(&money(2.0), Basis::Budget, Some(8.0), 0.0).expect("window");
        assert!((w.used_fraction - 0.75).abs() < 1e-9);
        assert_eq!(w.estimate, Some(Estimate::YourBudget));
        // The peak is irrelevant to a budget.
        let again = window(&money(2.0), Basis::Budget, Some(8.0), 100.0).expect("window");
        assert!((again.used_fraction - 0.75).abs() < 1e-9);
    }

    #[test]
    fn balance_only_draws_no_window_at_all() {
        assert!(window(&money(4.0), Basis::BalanceOnly, None, 10.0).is_none());
    }

    #[test]
    fn no_peak_draws_nothing_yet() {
        // The first day on since-top-up reads low rather than invented: with
        // no peak watched there is no denominator, so no window either.
        assert!(window(&money(4.0), Basis::SinceTopUp, None, 0.0).is_none());
        assert!(window(&money(4.0), Basis::Budget, None, 0.0).is_none());
    }

    #[test]
    fn a_balance_never_reads_as_spent() {
        // An empty purse tops out just under the mark, not at a full ring.
        let window = window(&money(0.0), Basis::SinceTopUp, None, 10.0).expect("window");
        assert!(window.used_fraction < 1.0);
        assert!(!window.is_exhausted);
    }

    #[test]
    fn peaks_climb_and_never_fall() {
        let mut peaks = Peaks::default();
        assert_eq!(peaks.advance("a", "USD", 5.0, 100), 5.0);
        assert_eq!(peaks.advance("a", "USD", 3.0, 200), 5.0);
        assert_eq!(peaks.advance("a", "USD", 9.0, 300), 9.0);
        // Another account, another currency: separate marks.
        assert_eq!(peaks.advance("b", "USD", 1.0, 400), 1.0);
        assert_eq!(peaks.advance("a", "CNY", 40.0, 500), 40.0);
    }
}
