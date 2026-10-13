//! Monthly facts from the same immutable Token-spend snapshot as its tables.
//! Calendar months, priced floors and unrecorded periods remain explicit.
use crate::ledger::TokenTally;
use crate::spend::{Analysis, Group, Row, Snapshot, Sort};
use chrono::{Datelike, Days, NaiveDate, Timelike};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq)]
pub struct MonthlyRecap {
    pub month: NaiveDate,
    pub end: NaiveDate,
    pub through: NaiveDate,
    pub recorded_from: Option<NaiveDate>,
    pub running: bool,
    pub analysis: Analysis,
    pub models: Vec<Row>,
    pub active_days: usize,
    pub observed_days: usize,
    pub longest_streak: usize,
    pub hours: [i64; 24],
    pub timed_tokens: i64,
    pub excluded_time_sources: Vec<String>,
    pub partial_sources: Vec<String>,
    pub cache_hit_rate: Option<f64>,
    pub excluded_cache_sources: Vec<String>,
    pub daily_change: Option<f64>,
}
pub fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap()
}
pub fn next_month(date: NaiveDate) -> NaiveDate {
    if date.month() == 12 {
        NaiveDate::from_ymd_opt(date.year() + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(date.year(), date.month() + 1, 1).unwrap()
    }
}
pub fn months(snapshot: &Snapshot, today: NaiveDate) -> Vec<NaiveDate> {
    let mut out = BTreeSet::from([month_start(today)]);
    let mut date = month_start(today);
    for _ in 0..24 {
        out.insert(date);
        date = month_start(date - Days::new(1));
    }
    for source in &snapshot.sources {
        for day in &source.ledger.days {
            if day.date <= today && day.tokens > 0 {
                out.insert(month_start(day.date));
            }
        }
    }
    out.into_iter().rev().collect()
}
impl MonthlyRecap {
    pub fn build(
        snapshot: &Snapshot,
        month: NaiveDate,
        today: NaiveDate,
        source: Option<&str>,
        model: Option<&str>,
    ) -> Self {
        let month = month_start(month);
        let end = next_month(month) - Days::new(1);
        let through = end.min(today);
        let activity = crate::token_activity::TokenActivity::build(snapshot, today, source, model);
        let recorded_from = activity
            .first_record
            .filter(|first| *first <= through)
            .map(|first| first.max(month));
        let range = month..=through;
        let mut analysis = snapshot.analyze_range(
            range.clone(),
            source,
            model,
            Group::Agents,
            Sort::Tokens,
            true,
        );
        analysis.rows.retain(|row| row.measures.tokens > 0);
        let mut models = snapshot
            .analyze_range(
                range.clone(),
                source,
                model,
                Group::Models,
                Sort::Tokens,
                true,
            )
            .rows;
        models.retain(|row| row.measures.tokens > 0);
        let active_days = analysis
            .days
            .iter()
            .filter(|d| d.measures.tokens > 0)
            .count();
        let observed_days = recorded_from
            .map(|first| (through.signed_duration_since(first).num_days() + 1).max(0) as usize)
            .unwrap_or(0);
        let mut longest_streak = 0;
        let mut streak = 0;
        if let Some(mut date) = recorded_from {
            while date <= through {
                if analysis
                    .days
                    .iter()
                    .any(|d| d.date == date && d.measures.tokens > 0)
                {
                    streak += 1;
                } else {
                    streak = 0;
                }
                longest_streak = longest_streak.max(streak);
                date = date + Days::new(1);
            }
        }
        let mut hours = [0_i64; 24];
        let mut timed_tokens = 0_i64;
        let mut cache = TokenTally::default();
        let mut excluded_time_sources = Vec::new();
        let mut excluded_cache_sources = Vec::new();
        let mut partial_sources = Vec::new();
        for item in snapshot
            .sources
            .iter()
            .filter(|s| source.is_none_or(|id| id == s.id))
        {
            if !crate::scan::checkpoint() {
                break;
            }
            let one = snapshot.analyze_range(
                range.clone(),
                Some(&item.id),
                model,
                Group::Days,
                Sort::Name,
                false,
            );
            if one.total.tokens <= 0 {
                continue;
            }
            if item.ledger.has_partial_records {
                partial_sources.push(item.title.clone());
            }
            if !item.ledger.has_partial_records && one.total.unclassified == 0 {
                cache += one.total.tally;
            } else {
                excluded_cache_sources.push(item.title.clone());
            }
            let mut own_hours = [0_i64; 24];
            let mut sum = 0_i64;
            for slot in &item.ledger.slots {
                if !crate::scan::checkpoint() {
                    break;
                }
                let Some(at) = chrono::DateTime::from_timestamp_millis(slot.start_ms)
                    .map(|at| at.with_timezone(&chrono::Local))
                else {
                    continue;
                };
                if !range.contains(&at.date_naive()) {
                    continue;
                }
                let n = model
                    .map(|m| {
                        slot.models.get(m).map(TokenTally::total).unwrap_or(0)
                            + slot.unclassified_models.get(m).copied().unwrap_or(0)
                    })
                    .unwrap_or(slot.tokens)
                    .max(0);
                own_hours[at.hour() as usize] = own_hours[at.hour() as usize].saturating_add(n);
                sum = sum.saturating_add(n);
            }
            if item.ledger.has_partial_records || sum != one.total.tokens {
                excluded_time_sources.push(item.title.clone());
            } else {
                for (hour, n) in own_hours.into_iter().enumerate() {
                    hours[hour] = hours[hour].saturating_add(n);
                }
                timed_tokens = timed_tokens.saturating_add(sum);
            }
        }
        let input = cache
            .input
            .saturating_add(cache.cache_read)
            .saturating_add(cache.cache_write);
        let cache_hit_rate = (input > 0).then(|| cache.cache_read as f64 / input as f64);
        let previous = month_start(month - Days::new(1));
        let previous_end = (month - Days::new(1))
            .min(previous + Days::new(through.day().saturating_sub(1) as u64));
        let prev = snapshot.analyze_range(
            previous..=previous_end,
            source,
            model,
            Group::Days,
            Sort::Name,
            false,
        );
        let prev_days = activity
            .first_record
            .filter(|first| *first <= previous_end)
            .map(|first| {
                (previous_end
                    .signed_duration_since(first.max(previous))
                    .num_days()
                    + 1)
                .max(0) as usize
            })
            .unwrap_or(0);
        let daily_change =
            (prev.total.tokens > 0 && prev_days > 0 && observed_days > 0).then(|| {
                (analysis.total.tokens as f64 / observed_days as f64)
                    / (prev.total.tokens as f64 / prev_days as f64)
                    - 1.0
            });
        Self {
            month,
            end,
            through,
            recorded_from,
            running: today < end,
            analysis,
            models,
            active_days,
            observed_days,
            longest_streak,
            hours,
            timed_tokens,
            excluded_time_sources,
            partial_sources,
            cache_hit_rate,
            excluded_cache_sources,
            daily_change,
        }
    }
    pub fn busiest_four_hours(&self) -> Option<(usize, f64)> {
        if self.timed_tokens <= 0 {
            return None;
        }
        let (start, tokens) = (0..24)
            .map(|start| {
                (
                    start,
                    (0..4)
                        .map(|i| self.hours[(start + i) % 24])
                        .fold(0_i64, i64::saturating_add),
                )
            })
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))?;
        Some((start, tokens as f64 / self.timed_tokens as f64))
    }
    pub fn day_tokens(&self, date: NaiveDate) -> Option<i64> {
        if date < self.recorded_from? || date > self.through || date < self.month || date > self.end
        {
            return None;
        }
        Some(
            self.analysis
                .days
                .iter()
                .find(|d| d.date == date)
                .map(|d| d.measures.tokens)
                .unwrap_or(0),
        )
    }
    /// API-equivalent value / prorated reader-supplied monthly price. This is
    /// an estimate based on public rates, never an actual subscription bill.
    pub fn payback(&self, monthly_usd: f64) -> Option<(f64, f64)> {
        if !monthly_usd.is_finite() || monthly_usd <= 0.0 || self.observed_days == 0 {
            return None;
        }
        let price = monthly_usd * self.observed_days as f64 / self.end.day() as f64;
        Some((price, self.analysis.total.cost / price))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn monthly_calendar_is_not_a_rolling_thirty_day_span() {
        let snapshot = crate::token_activity::tests::snapshot();
        let today = NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        let recap = MonthlyRecap::build(&snapshot, today, today, None, None);
        assert_eq!(recap.analysis.total.tokens, 35);
        assert_eq!(recap.analysis.total.unpriced, 35);
        assert_eq!(recap.active_days, 2);
        assert_eq!(recap.observed_days, 3);
        assert_eq!(recap.longest_streak, 1);
        assert_eq!(
            recap.day_tokens(NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()),
            Some(0)
        );
        assert_eq!(
            recap.day_tokens(NaiveDate::from_ymd_opt(2026, 10, 4).unwrap()),
            None
        );
        assert!(recap.running);
        assert_eq!(recap.models.len(), 2);
        assert!(recap.payback(20.0).is_some());
        assert_eq!(recap.payback(0.0), None);
        assert_eq!(recap.payback(f64::NAN), None);
    }
    #[test]
    fn periods_before_recording_have_no_active_day_denominator_or_change() {
        let snapshot = crate::token_activity::tests::snapshot();
        let today = NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        let recap = MonthlyRecap::build(
            &snapshot,
            NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
            today,
            None,
            None,
        );
        assert_eq!(recap.observed_days, 0);
        assert_eq!(recap.daily_change, None);
        assert_eq!(recap.day_tokens(recap.month), None);
        assert_eq!(recap.payback(20.0), None);
        let sep = MonthlyRecap::build(
            &snapshot,
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            today,
            None,
            None,
        );
        assert_eq!(sep.observed_days, 2);
        assert_eq!(
            sep.day_tokens(NaiveDate::from_ymd_opt(2026, 9, 28).unwrap()),
            None
        );
        assert!(!sep.running);
    }
    #[test]
    fn one_coarse_source_does_not_suppress_other_tools_rhythm_or_cache_rate() {
        let mut snapshot = crate::token_activity::tests::snapshot();
        let mut coarse = snapshot.sources[0].clone();
        coarse.id = "coarse".into();
        coarse.title = "Coarse".into();
        coarse.ledger.slots.clear();
        coarse.ledger.has_partial_records = true;
        snapshot.sources.push(coarse);
        let today = NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        let recap = MonthlyRecap::build(&snapshot, today, today, None, None);
        assert_eq!(recap.analysis.total.tokens, 70);
        assert_eq!(recap.timed_tokens, 35);
        assert_eq!(recap.excluded_time_sources, vec!["Coarse"]);
        assert_eq!(recap.excluded_cache_sources, vec!["Coarse"]);
        assert_eq!(recap.cache_hit_rate, Some(0.0));
        assert_eq!(recap.busiest_four_hours().unwrap().0, 7);
        assert!((recap.busiest_four_hours().unwrap().1 - 30.0 / 35.0).abs() < 1e-10);
    }
}
