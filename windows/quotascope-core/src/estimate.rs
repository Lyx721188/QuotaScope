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
//!   0.5% to 1.5%, a three-fold spread in the result. How coarse a provider's
//!   percentages are is itself something only the provider knows, so it comes
//!   in as [`Rule::minimum_used`]: Antigravity reports its fraction to seven
//!   decimals and can be divided at a tenth of what the others allow.
//! - A window scoped to one model group is spent by that group alone, but the
//!   logs' spending for the period is everything together — dividing one by
//!   the other priced Claude Code's 2%-used Fable window at ten thousand
//!   dollars, because almost all the money in it had gone through Opus. Such a
//!   window is priced only where the provider can say which of the logged
//!   models spend it ([`Rule::place`]), and only from those models' money.
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

/// Where one of the ledger's models sits relative to one window's scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Its spending counts against this window.
    InScope,
    /// It counts against a different window of the same account, so leaving it
    /// out of this one's spending is correct rather than lossy.
    OutOfScope,
    /// Nobody can say. The estimate is withheld: a window priced from part of
    /// its own spending reads too low, and that is the one error this figure
    /// cannot be labelled its way out of.
    Unknown,
}

/// What only the provider knows about the figures it reports, as far as the
/// estimator is concerned.
#[derive(Debug, Clone, Copy)]
pub struct Rule {
    /// The smallest share of a window worth dividing by, given how coarsely
    /// this provider reports it.
    pub minimum_used: f64,
    /// For a window scoped to a model group: whether a given model's spending
    /// counts against a window of the given scope. `None` says the provider
    /// cannot tell, and its scoped windows are then never priced.
    pub place: Option<fn(scope: &str, model: &str) -> Placement>,
}

impl Default for Rule {
    /// Whole percentages, and no way to separate one model group's spending
    /// from another's: the shape Claude Code and Codex report, and the safe
    /// assumption for anything new.
    fn default() -> Self {
        Rule {
            minimum_used: MINIMUM_USED,
            place: None,
        }
    }
}

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

pub fn estimate(
    window: &UsageWindow,
    ledger: &UsageLedger,
    now_ms: i64,
    rule: Rule,
) -> Option<BudgetEstimate> {
    if window.window_seconds <= 0 {
        return None;
    }
    let resets = window.resets_at?;
    if window.used_fraction < rule.minimum_used {
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

    let spent = match (&window.scope, rule.place) {
        (None, _) => ledger.spend_since(opened_ms).1,
        (Some(scope), Some(place)) => scoped_spend(ledger, opened_ms, scope, place)?,
        // A limit scoped to one model is spent by that model alone, but the
        // logs' spending for the period is everything together — dividing one
        // by the other priced Claude Code's 2%-used Fable window at ten
        // thousand dollars, because almost all of the money in it had gone
        // through Opus.
        (Some(_), None) => return None,
    };
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

/// What the models of one group spent since the window opened.
///
/// Every model in the stretch has to be placed, and `None` is the answer where
/// one cannot be: the unplaced spending belongs to the window or it does not,
/// and either guess puts a figure on the card that this machine's own records
/// do not support.
fn scoped_spend(
    ledger: &UsageLedger,
    opened_ms: i64,
    scope: &str,
    place: fn(&str, &str) -> Placement,
) -> Option<f64> {
    let mut spent = 0.0;
    for slot in &ledger.slots {
        if slot.start_ms < opened_ms {
            continue;
        }
        // Placed by what was logged, not by what reached a price: a model the
        // table has no rate for is absent from `costs` and still spent the
        // window, so it has to be recognised as one of the group's own — or as
        // something nobody can say.
        for model in slot.models.keys() {
            match place(scope, model) {
                Placement::InScope => spent += slot.costs.get(model).copied().unwrap_or(0.0),
                Placement::OutOfScope => {}
                Placement::Unknown => return None,
            }
        }
    }
    Some(spent)
}

/// The card's estimate line for one window — "Estimated value ≈$220 ·
/// ≈$57 used" — or nothing where the estimator withholds it. The line says
/// "estimated" wherever it appears, because this is the one number in
/// QuotaScope that is inferred rather than reported.
pub fn window_text(
    window: &UsageWindow,
    ledger: &UsageLedger,
    now_ms: i64,
    rule: Rule,
) -> Option<String> {
    let e = estimate(window, ledger, now_ms, rule)?;
    Some(crate::localization::t_fmt(
        "Estimated value {value} · {spent} used",
        &[&approximate(e.full), &approximate(e.spent)],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{Slot, TokenTally};
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

    /// A quarter-hour of work by one unnamed model — all the account-wide
    /// tests care about is the money in it.
    fn slot(start_ms: i64, cost: f64) -> Slot {
        slot_by(start_ms, &[("some-model", cost)])
    }

    /// A quarter-hour of work, split by the models that did it.
    fn slot_by(start_ms: i64, models: &[(&str, f64)]) -> Slot {
        let mut tallies = BTreeMap::new();
        let mut costs = BTreeMap::new();
        let mut total = 0.0;
        for (model, cost) in models {
            tallies.insert((*model).to_string(), TokenTally::default());
            costs.insert((*model).to_string(), *cost);
            total += cost;
        }
        Slot {
            start_ms,
            tokens: 0,
            cost: total,
            unpriced_tokens: 0,
            models: tallies,
            costs,
        }
    }

    fn ledger_with(slots: Vec<Slot>) -> UsageLedger {
        UsageLedger {
            days: vec![],
            earliest: None,
            unpriced_models: vec![],
            model_names: BTreeMap::new(),
            slots,
            has_partial_records: false,
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
        let e = estimate(&w, &ledger, now, Rule::default()).expect("estimated");
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
        assert_eq!(estimate(&w, &ledger, resets - 1000, Rule::default()), None);
    }

    #[test]
    fn too_little_spent_is_withheld() {
        let resets = 1_792_915_200_000i64;
        let opened = resets - 3 * 3600 * 1000;
        let w = window(0.25, 3 * 3600, resets, None);
        let ledger = ledger_with(vec![slot(opened + 60_000, 0.10)]);
        assert_eq!(estimate(&w, &ledger, resets - 1000, Rule::default()), None);
    }

    #[test]
    fn logs_that_begin_after_the_window_are_withheld() {
        let resets = 1_792_915_200_000i64;
        let opened = resets - 3 * 3600 * 1000;
        let w = window(0.25, 3 * 3600, resets, None);
        let ledger = ledger_with(vec![slot(opened + 3600 * 1000, 60.0)]);
        assert_eq!(estimate(&w, &ledger, resets - 1000, Rule::default()), None);
    }

    #[test]
    fn scoped_windows_are_never_priced_without_a_way_to_place_models() {
        let resets = 1_792_915_200_000i64;
        let opened = resets - 3 * 3600 * 1000;
        let w = window(0.25, 3 * 3600, resets, Some("Opus"));
        let ledger = ledger_with(vec![slot(opened - 3600 * 1000, 60.0)]);
        assert_eq!(estimate(&w, &ledger, resets - 1000, Rule::default()), None);
    }

    #[test]
    fn a_scoped_window_is_priced_from_its_own_group_alone() {
        let resets = 1_792_915_200_000i64;
        let opened = resets - 3 * 3600 * 1000;
        // The quarter-hour held $100 of work, but split across two allowances:
        // $30 of Gemini and $70 of Claude. Each window is worth what its own
        // share divided by its own percentage — not what the whole hour was.
        let ledger = ledger_with(vec![slot_by(
            opened,
            &[("gemini-3.8-flash", 30.0), ("claude-opus-4-6", 70.0)],
        )]);
        let now = resets - 1000;

        let gemini = window(0.25, 3 * 3600, resets, Some("Gemini"));
        let e = estimate(&gemini, &ledger, now, GROUPED).expect("estimated");
        assert!((e.spent - 30.0).abs() < 1e-9);
        assert!((e.full - 120.0).abs() < 1e-9);

        let other = window(0.25, 3 * 3600, resets, Some("Claude and GPT"));
        let e = estimate(&other, &ledger, now, GROUPED).expect("estimated");
        assert!((e.spent - 70.0).abs() < 1e-9);
        assert!((e.full - 280.0).abs() < 1e-9);
    }

    #[test]
    fn a_scoped_window_is_withheld_where_a_model_cannot_be_placed() {
        let resets = 1_792_915_200_000i64;
        let opened = resets - 3 * 3600 * 1000;
        // A model nobody can assign: it spent one window or the other, and
        // guessing would put one of the two figures on the card wrong.
        let ledger = ledger_with(vec![slot_by(
            opened,
            &[("gemini-3.8-flash", 30.0), ("unheard-of-9", 70.0)],
        )]);
        let gemini = window(0.25, 3 * 3600, resets, Some("Gemini"));
        assert_eq!(estimate(&gemini, &ledger, resets - 1000, GROUPED), None);
        // The account-wide path is untouched by it — that model's money is in
        // the total either way.
        let whole = window(0.25, 3 * 3600, resets, None);
        assert!(estimate(&whole, &ledger, resets - 1000, Rule::default()).is_some());
    }

    #[test]
    fn a_provider_that_reports_exact_fractions_can_be_divided_lower() {
        let resets = 1_792_915_200_000i64;
        let opened = resets - 7 * 86_400 * 1000;
        let ledger = ledger_with(vec![slot(opened, 60.0)]);
        let w = window(0.005, 7 * 86_400, resets, None);
        // Half a percent used: the whole-percent floor throws this away.
        assert_eq!(estimate(&w, &ledger, resets - 1000, Rule::default()), None);
        let exact = Rule {
            minimum_used: 0.002,
            ..Rule::default()
        };
        let e = estimate(&w, &ledger, resets - 1000, exact).expect("estimated");
        assert!((e.full - 12_000.0).abs() < 1e-6);
    }

    #[test]
    fn a_window_that_has_not_opened_is_not_priced() {
        let resets = 1_792_915_200_000i64;
        let w = window(0.25, 3 * 3600, resets, None);
        let ledger = ledger_with(vec![slot(0, 60.0)]);
        // `now` before the window opened.
        assert_eq!(
            estimate(&w, &ledger, resets - 3 * 3600 * 1000 - 1, Rule::default()),
            None
        );
    }

    /// A provider with two allowances and a way to say which model spends
    /// which — the shape Antigravity reports, with its names.
    const GROUPED: Rule = Rule {
        minimum_used: MINIMUM_USED,
        place: Some(place_by_name),
    };

    fn place_by_name(scope: &str, model: &str) -> Placement {
        let group = if model.starts_with("gemini") {
            Some("Gemini")
        } else if model.starts_with("claude") {
            Some("Claude and GPT")
        } else {
            None
        };
        match group {
            Some(group) if group == scope => Placement::InScope,
            Some(_) => Placement::OutOfScope,
            None => Placement::Unknown,
        }
    }

    #[test]
    fn approximate_formats_whole_dollars_from_a_hundred_up() {
        assert_eq!(approximate(219.6), "≈$220");
        assert_eq!(approximate(99.99), "≈$99.99");
        assert_eq!(approximate(12.0), "≈$12.00");
        assert_eq!(approximate(10_000.0), "≈$10000");
    }
}
