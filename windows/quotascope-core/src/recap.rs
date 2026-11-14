//! Monthly facts from the same immutable Token-spend snapshot as its tables.
//! Calendar months, priced floors and unrecorded periods remain explicit.
use crate::ledger::TokenTally;
use crate::spend::{Analysis, Group, Row, Snapshot, Sort};
use chrono::{Datelike, Days, NaiveDate, Timelike};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Period {
    Day,
    Week,
    #[default]
    Month,
    Year,
}
impl Period {
    pub fn default_date(
        self,
        today: NaiveDate,
        hour: u32,
        earliest: Option<NaiveDate>,
    ) -> NaiveDate {
        let current = self.bounds(today).0;
        let early = match self {
            Self::Day => hour < 5,
            Self::Week => today == current,
            Self::Month => today.day() <= 7,
            Self::Year => today.month() == 1 && today.day() <= 7,
        };
        let wanted = if early {
            self.step(current, false)
        } else {
            current
        };
        let floor = earliest
            .map(|first| self.bounds(first).0.min(current))
            .unwrap_or(wanted);
        wanted.max(floor)
    }

    pub fn bounds(self, date: NaiveDate) -> (NaiveDate, NaiveDate) {
        let start = match self {
            Self::Day => date,
            Self::Week => {
                date - Days::new(
                    ((date.weekday().num_days_from_monday() + 7 - first_weekday()) % 7) as u64,
                )
            }
            Self::Month => month_start(date),
            Self::Year => NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap(),
        };
        let end = match self {
            Self::Day => start,
            Self::Week => start + Days::new(6),
            Self::Month => next_month(start) - Days::new(1),
            Self::Year => NaiveDate::from_ymd_opt(start.year(), 12, 31).unwrap(),
        };
        (start, end)
    }
    pub fn step(self, date: NaiveDate, forward: bool) -> NaiveDate {
        let (start, end) = self.bounds(date);
        self.bounds(if forward {
            end + Days::new(1)
        } else {
            start - Days::new(1)
        })
        .0
    }
}
fn first_weekday() -> u32 {
    #[cfg(windows)]
    {
        use windows::Win32::Globalization::*;
        let mut value = [0u16; 4];
        let len = unsafe {
            GetLocaleInfoEx(
                windows::core::PCWSTR::null(),
                LOCALE_IFIRSTDAYOFWEEK,
                Some(&mut value),
            )
        };
        if len > 1 && (48..=54).contains(&value[0]) {
            return (value[0] - 48) as u32;
        }
    }
    0
}

#[derive(Debug, Clone, PartialEq)]
pub struct MonthlyRecap {
    pub period: Period,
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
        Self::build_period(snapshot, Period::Month, month, today, source, model)
    }
    pub fn build_period(
        snapshot: &Snapshot,
        period: Period,
        date: NaiveDate,
        today: NaiveDate,
        source: Option<&str>,
        model: Option<&str>,
    ) -> Self {
        let (month, end) = period.bounds(date);
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
        let (previous, previous_last) = period.bounds(month - Days::new(1));
        let previous_end = previous_last.min(
            previous + Days::new(through.signed_duration_since(month).num_days().max(0) as u64),
        );
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
            period,
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
        let mut weight = 0.0;
        let mut date = self.recorded_from?;
        while date <= self.through {
            weight += 1.0 / (next_month(month_start(date)) - Days::new(1)).day() as f64;
            date = date + Days::new(1);
        }
        let price = monthly_usd * weight;
        Some((price, self.analysis.total.cost / price))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn defaults_follow_early_period_rules_and_recording_floor() {
        let early = NaiveDate::from_ymd_opt(2026, 1, 3).unwrap();
        assert_eq!(
            Period::Day.default_date(early, 4, None),
            early - Days::new(1)
        );
        assert_eq!(Period::Day.default_date(early, 5, None), early);
        assert_eq!(
            Period::Month.default_date(early, 12, None),
            NaiveDate::from_ymd_opt(2025, 12, 1).unwrap()
        );
        assert_eq!(
            Period::Year.default_date(early, 12, None),
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()
        );
        assert_eq!(
            Period::Month.default_date(early, 12, Some(early)),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
        );
        let start = Period::Week.bounds(early).0;
        assert_eq!(
            Period::Week.default_date(start, 12, None),
            start - Days::new(7)
        );
        assert_eq!(
            Period::Week.default_date(start + Days::new(1), 12, None),
            start
        );
    }
    use super::*;
    #[test]
    fn calendar_periods_keep_bounds_across_months_and_years() {
        let mut snapshot = crate::token_activity::tests::snapshot();
        snapshot.sources[0].title = "A&B <tool>".into();
        let today = NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        let day = MonthlyRecap::build_period(&snapshot, Period::Day, today, today, None, None);
        assert_eq!(day.analysis.total.tokens, 5);
        assert_eq!(day.month, day.end);
        let year = MonthlyRecap::build_period(&snapshot, Period::Year, today, today, None, None);
        assert_eq!(year.month, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        assert_eq!(year.end, NaiveDate::from_ymd_opt(2026, 12, 31).unwrap());
        assert_eq!(
            Period::Month.step(NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(), false),
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
        );
    }
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
