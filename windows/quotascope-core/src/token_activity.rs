//! Calendar activity from an existing snapshot, independent of its span picker.
use crate::spend::Snapshot;
use chrono::{Datelike, Days, NaiveDate};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityDay {
    pub date: NaiveDate,
    /// None before the first record; zero is a quiet observed-calendar day.
    pub tokens: Option<i64>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct TokenActivity {
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub first_record: Option<NaiveDate>,
    pub days: Vec<ActivityDay>,
    pub total: i64,
    pub active_days: usize,
    pub partial_sources: Vec<String>,
    pub cuts: [i64; 3],
}
impl TokenActivity {
    pub fn build(
        snapshot: &Snapshot,
        today: NaiveDate,
        source: Option<&str>,
        model: Option<&str>,
    ) -> Self {
        let last_week = today - Days::new(today.weekday().num_days_from_monday() as u64);
        let year_ago = today
            .with_year(today.year() - 1)
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(today.year() - 1, 2, 28).unwrap());
        let start = (year_ago + Days::new(1)).max(last_week - Days::new(52 * 7));
        let mut out = Self {
            start,
            end: today,
            first_record: None,
            days: vec![],
            total: 0,
            active_days: 0,
            partial_sources: vec![],
            cuts: [0; 3],
        };
        let mut counts = BTreeMap::<NaiveDate, i64>::new();
        let mut partial = BTreeSet::new();
        for item in snapshot
            .sources
            .iter()
            .filter(|s| source.is_none_or(|id| id == s.id))
        {
            if !crate::scan::checkpoint() {
                return out;
            }
            if item.ledger.has_partial_records {
                partial.insert(item.title.clone());
            }
            let first = if let Some(model) = model {
                item.ledger
                    .days
                    .iter()
                    .filter(|d| d.date <= today && d.models.contains_key(model))
                    .map(|d| d.date)
                    .min()
            } else {
                item.ledger.earliest.filter(|d| *d <= today)
            };
            if let Some(first) = first {
                out.first_record = Some(out.first_record.map_or(first, |old| old.min(first)));
            }
            for day in item
                .ledger
                .days
                .iter()
                .filter(|d| d.date >= start && d.date <= today)
            {
                if !crate::scan::checkpoint() {
                    return out;
                }
                let n = model
                    .map(|m| day.models.get(m).copied().unwrap_or(0))
                    .unwrap_or(day.tokens)
                    .max(0);
                let sum = counts.entry(day.date).or_default();
                if let Some(next) = sum.checked_add(n) {
                    *sum = next;
                } else {
                    *sum = i64::MAX;
                    partial.insert(item.title.clone());
                }
            }
        }
        if let Some(first) = out.first_record {
            let mut date = start;
            while date <= today {
                let tokens = (date >= first).then(|| counts.get(&date).copied().unwrap_or(0));
                if let Some(n) = tokens {
                    out.total = out.total.saturating_add(n);
                    out.active_days += usize::from(n > 0);
                }
                out.days.push(ActivityDay { date, tokens });
                date = date + Days::new(1);
            }
            let mut busy: Vec<_> = out
                .days
                .iter()
                .filter_map(|d| d.tokens.filter(|n| *n > 0))
                .collect();
            busy.sort_unstable();
            if !busy.is_empty() {
                out.cuts = [
                    busy[busy.len() / 4],
                    busy[busy.len() / 2],
                    busy[(busy.len() * 3) / 4],
                ];
            }
        }
        out.partial_sources = partial.into_iter().collect();
        out
    }
    pub fn step(&self, tokens: i64) -> usize {
        if tokens <= 0 {
            0
        } else {
            1 + self.cuts.iter().filter(|cut| tokens > **cut).count()
        }
    }
    pub fn week_start(&self) -> NaiveDate {
        self.start - Days::new(self.start.weekday().num_days_from_monday() as u64)
    }
    pub fn week_totals(&self) -> Vec<Option<i64>> {
        let first = self.week_start();
        let columns = (self.end.signed_duration_since(first).num_days() / 7 + 1) as usize;
        let mut out = vec![None; columns];
        for day in &self.days {
            if let Some(tokens) = day.tokens {
                let i = (day.date.signed_duration_since(first).num_days() / 7) as usize;
                out[i] = Some(out[i].unwrap_or(0_i64).saturating_add(tokens));
            }
        }
        out
    }
    pub fn cumulative(&self) -> Vec<(NaiveDate, i64)> {
        let mut total = 0_i64;
        self.days
            .iter()
            .filter_map(|day| {
                day.tokens.map(|n| {
                    total = total.saturating_add(n);
                    (day.date, total)
                })
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ledger::{priced, TokenTally};
    use crate::spend::Source;
    pub(crate) fn snapshot() -> Snapshot {
        let buckets = BTreeMap::from([
            (
                "2026-09-29 09:00".into(),
                BTreeMap::from([(
                    "m".into(),
                    TokenTally {
                        input: 10,
                        ..Default::default()
                    },
                )]),
            ),
            (
                "2026-10-01 10:00".into(),
                BTreeMap::from([(
                    "m".into(),
                    TokenTally {
                        input: 30,
                        ..Default::default()
                    },
                )]),
            ),
            (
                "2026-10-03 23:00".into(),
                BTreeMap::from([(
                    "other".into(),
                    TokenTally {
                        output: 5,
                        ..Default::default()
                    },
                )]),
            ),
            (
                "2026-11-01 09:00".into(),
                BTreeMap::from([(
                    "m".into(),
                    TokenTally {
                        input: 999,
                        ..Default::default()
                    },
                )]),
            ),
        ]);
        Snapshot {
            sources: vec![Source {
                id: "test".into(),
                title: "Test".into(),
                location: String::new(),
                present: true,
                ledger: priced(&buckets, &BTreeMap::new(), None),
            }],
        }
    }
    #[test]
    fn calendar_has_no_false_zero_before_records_and_no_future_counts() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        let a = TokenActivity::build(&snapshot(), today, None, None);
        assert_eq!(a.total, 45);
        assert_eq!(a.active_days, 3);
        assert_eq!(
            a.days
                .iter()
                .find(|d| d.date.to_string() == "2026-09-28")
                .unwrap()
                .tokens,
            None
        );
        assert_eq!(
            a.days
                .iter()
                .find(|d| d.date.to_string() == "2026-09-30")
                .unwrap()
                .tokens,
            Some(0)
        );
        assert_eq!(a.cumulative().last().unwrap().1, 45);
        assert_eq!(a.week_totals().iter().flatten().sum::<i64>(), 45);
        assert!(a.week_totals().len() <= 53);
        assert_eq!(
            TokenActivity::build(&snapshot(), today, None, Some("m")).total,
            40
        );
        assert!(
            TokenActivity::build(&snapshot(), today, Some("missing"), None)
                .days
                .is_empty()
        );
    }
    #[test]
    fn leap_year_and_all_weekday_alignments_fit_fifty_three_columns() {
        for y in [2024, 2025, 2026] {
            for day in 1..=31 {
                let today = NaiveDate::from_ymd_opt(y, 3, day).unwrap();
                let mut snapshot = snapshot();
                snapshot.sources[0].ledger.earliest =
                    Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap());
                let a = TokenActivity::build(&snapshot, today, None, None);
                assert!(a.week_totals().len() <= 53);
                assert_eq!(a.days.last().unwrap().date, today);
            }
        }
    }
}
