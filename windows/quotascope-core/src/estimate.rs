//! What a rate-limit window is worth in money, ported from
//! `BudgetEstimate.swift`.
//!
//! The providers publish a percentage and nothing else — they will tell you a
//! weekly limit is 3% gone, never what the other 97% is worth. But the two
//! halves of that answer are both here: they report the percentage, and the
//! local logs say what was actually spent in the same stretch of time. Divide
//! one by the other and the window has a price.
//!
//! This is the one number in QuotaScope that is **inferred rather than
//! reported**, so it is labelled as an estimate wherever it appears, and it is
//! withheld rather than guessed when the inputs can't support it:
//!
//! - Too little used, and the percentage's own rounding swamps the answer —
//!   at 1% used, a provider reporting whole numbers could mean anywhere from
//!   0.5% to 1.5%, a three-fold spread in the result.
//! - Logs that start after the window did miss part of the spending, which
//!   would put the window's worth too low.
//! - Work done on another machine is invisible here, and pulls the same way.
//!   Nothing can detect that, which is the honest limit of this figure.

use crate::ledger::UsageLedger;
use crate::model::UsageWindow;

/// Below this the percentage is too coarse to divide by. The providers report
/// whole numbers, so at 2% used the true figure is somewhere between 1.5% and
/// 2.5% and the answer carries about a quarter either way — wide, but it is
/// labelled an estimate and a quarter either way still tells you whether a
/// window is worth ten dollars or a thousand. Below 2% it stops meaning
/// anything at all.
pub const MINIMUM_USED: f64 = 0.02;

/// And below this there isn't enough money in play to be worth reporting.
pub const MINIMUM_SPEND: f64 = 0.20;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BudgetEstimate {
    /// What the whole window is worth.
    pub full: f64,
    /// What is left of it.
    pub remaining: f64,
    /// What has already gone, by this machine's reckoning.
    pub spent: f64,
}

impl BudgetEstimate {
    /// "≈$220": whole dollars from a hundred up, because the figure is an
    /// estimate and cents would claim a precision it does not have. Shared by
    /// the card and any other surface, so they agree.
    pub fn approximate(&self) -> String {
        approximate(self.full)
    }
}

pub fn approximate(amount: f64) -> String {
    if amount >= 100.0 {
        format!("≈${:.0}", amount)
    } else {
        format!("≈${:.2}", amount)
    }
}

pub fn estimate(window: &UsageWindow, ledger: &UsageLedger, now_ms: i64) -> Option<BudgetEstimate> {
    if window.window_seconds <= 0 {
        return None;
    }
    let resets = window.resets_at?;
    // Account-wide windows only. A limit scoped to one model is spent by that
    // model alone, but the logs' spending for the period is everything
    // together — dividing one by the other priced Claude Code's 2%-used
    // Fable window at ten thousand dollars, because almost all of the money
    // in it had gone through Opus.
    if window.scope.is_some() {
        return None;
    }
    if window.used_fraction < MINIMUM_USED {
        return None;
    }

    let opened_ms = resets - window.window_seconds * 1000;
    if opened_ms >= now_ms {
        return None;
    }

    // Logs that begin after the window did would only show part of the
    // spending, and the shortfall lands straight in the answer.
    let first_logged = ledger.slots.first()?.start_ms;
    if first_logged > opened_ms {
        return None;
    }

    let spent = ledger.spend_since(opened_ms).1;
    if spent < MINIMUM_SPEND {
        return None;
    }

    let full = spent / window.used_fraction;
    Some(BudgetEstimate {
        full,
        remaining: (full - spent).max(0.0),
        spent,
    })
}

/// The card's estimate line for one window — "Estimated value ≈$220 ·
/// ≈$57 used" — or nothing where the estimator withholds it. The line says
/// "estimated" wherever it appears, because this is the one number in
/// QuotaScope that is inferred rather than reported.
pub fn window_text(window: &UsageWindow, ledger: &UsageLedger, now_ms: i64) -> Option<String> {
    let e = estimate(window, ledger, now_ms)?;
    Some(crate::localization::t_fmt(
        "Estimated value {value} · {spent} used",
        &[&approximate(e.full), &approximate(e.spent)],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::Slot;
    use std::collections::BTreeMap;

    fn window(
        used: f64,
        window_seconds: i64,
        resets_in_ms: i64,
        scope: Option<&str>,
    ) -> UsageWindow {
        UsageWindow {
            scope: scope.map(str::to_string),
            window_seconds,
            resets_at: Some(resets_in_ms),
            used_fraction: used,
            ..test_window()
        }
    }

    fn test_window() -> UsageWindow {
        UsageWindow::new(
            "test",
            crate::model::Kind::Other(3600),
            None,
            0.0,
            3600,
            None,
        )
    }

    fn slot(start_ms: i64, cost: f64) -> Slot {
        Slot {
            start_ms,
            tokens: 0,
            cost,
            unpriced_tokens: 0,
            models: BTreeMap::new(),
        }
    }

    fn ledger_with(slots: Vec<Slot>) -> UsageLedger {
        UsageLedger {
            days: vec![],
            earliest: None,
            unpriced_models: vec![],
            model_names: BTreeMap::new(),
            slots,
        }
    }

    #[test]
    fn a_window_with_enough_history_has_a_price() {
        // Window opened 2026-10-01T09:00Z, resets at noon: a 3h window.
        let resets = 1_792_915_200_000i64; // arbitrary epoch ms
        let opened = resets - 3 * 3600 * 1000;
        let w = window(0.25, 3 * 3600, resets, None);
        // $100 spent since the window opened → full = $400. A slot before the
        // window opened is this ledger's older history and stays out.
        let ledger = ledger_with(vec![slot(opened, 40.0), slot(opened + 60_000, 60.0)]);
        let now = resets - 1000;
        let e = estimate(&w, &ledger, now).expect("estimated");
        assert!((e.full - 400.0).abs() < 1e-9);
        assert!((e.spent - 100.0).abs() < 1e-9);
        assert!((e.remaining - 300.0).abs() < 1e-9);
        assert_eq!(e.approximate(), "≈$400");
    }

    #[test]
    fn too_little_used_is_withheld() {
        let resets = 1_792_915_200_000i64;
        let opened = resets - 3 * 3600 * 1000;
        let w = window(0.01, 3 * 3600, resets, None);
        let ledger = ledger_with(vec![slot(opened + 60_000, 60.0)]);
        assert_eq!(estimate(&w, &ledger, resets - 1000), None);
    }

    #[test]
    fn too_little_spent_is_withheld() {
        let resets = 1_792_915_200_000i64;
        let opened = resets - 3 * 3600 * 1000;
        let w = window(0.25, 3 * 3600, resets, None);
        let ledger = ledger_with(vec![slot(opened + 60_000, 0.10)]);
        assert_eq!(estimate(&w, &ledger, resets - 1000), None);
    }

    #[test]
    fn logs_that_begin_after_the_window_are_withheld() {
        let resets = 1_792_915_200_000i64;
        let opened = resets - 3 * 3600 * 1000;
        let w = window(0.25, 3 * 3600, resets, None);
        let ledger = ledger_with(vec![slot(opened + 3600 * 1000, 60.0)]);
        assert_eq!(estimate(&w, &ledger, resets - 1000), None);
    }

    #[test]
    fn scoped_windows_are_never_priced() {
        let resets = 1_792_915_200_000i64;
        let opened = resets - 3 * 3600 * 1000;
        let w = window(0.25, 3 * 3600, resets, Some("Opus"));
        let ledger = ledger_with(vec![slot(opened - 3600 * 1000, 60.0)]);
        assert_eq!(estimate(&w, &ledger, resets - 1000), None);
    }

    #[test]
    fn a_window_that_has_not_opened_is_not_priced() {
        let resets = 1_792_915_200_000i64;
        let w = window(0.25, 3 * 3600, resets, None);
        let ledger = ledger_with(vec![slot(0, 60.0)]);
        // `now` before the window opened.
        assert_eq!(estimate(&w, &ledger, resets - 3 * 3600 * 1000 - 1), None);
    }

    #[test]
    fn approximate_formats_whole_dollars_from_a_hundred_up() {
        assert_eq!(approximate(219.6), "≈$220");
        assert_eq!(approximate(99.99), "≈$99.99");
        assert_eq!(approximate(12.0), "≈$12.00");
        assert_eq!(approximate(10_000.0), "≈$10000");
    }
}
