//! Bounded, cancellable analysis of an immutable local-reading snapshot.
//! Results are scoped by snapshot identity and every displayed filter.
use crate::scan::{self, Cancelled, Control};
use crate::spend::{Analysis, Group, Snapshot, Sort, Span};
use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub span: Span,
    pub today: NaiveDate,
    pub source: Option<String>,
    pub model: Option<String>,
    pub group: Group,
    pub sort: Sort,
    pub descending: bool,
}

impl Query {
    fn same_aggregation(&self, other: &Self) -> bool {
        self.span == other.span
            && self.today == other.today
            && self.source == other.source
            && self.model == other.model
            && self.group == other.group
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub snapshot: Arc<Snapshot>,
    pub query: Query,
}

impl Request {
    pub fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.snapshot, &other.snapshot) && self.query == other.query
    }
}

#[derive(Debug)]
pub struct Summary {
    pub request: Request,
    pub analysis: Analysis,
    pub hours: [i64; 24],
    pub models: Vec<String>,
    pub bins: Vec<i64>,
    pub coverage: BTreeMap<String, bool>,
    pub partial_sources: Vec<String>,
    pub reused_aggregation: bool,
}

impl Summary {
    pub fn compute(
        request: Request,
        control: Arc<Control>,
        previous: Option<Arc<Self>>,
    ) -> Result<Self, Cancelled> {
        scan::run(
            control,
            0,
            |_| {},
            || {
                let reusable = previous.as_ref().filter(|old| {
                    Arc::ptr_eq(&old.request.snapshot, &request.snapshot)
                        && old.request.query.same_aggregation(&request.query)
                });
                if let Some(old) = reusable {
                    let mut analysis = old.analysis.clone();
                    analysis.sort(request.query.sort, request.query.descending);
                    return Self {
                        request,
                        analysis,
                        hours: old.hours,
                        models: old.models.clone(),
                        bins: old.bins.clone(),
                        coverage: old.coverage.clone(),
                        partial_sources: old.partial_sources.clone(),
                        reused_aggregation: true,
                    };
                }

                let query = &request.query;
                let snapshot = &request.snapshot;
                let analysis = snapshot.analyze(
                    query.span,
                    query.today,
                    query.source.as_deref(),
                    query.model.as_deref(),
                    query.group,
                    query.sort,
                    query.descending,
                );
                let hours = snapshot.hourly(
                    query.span,
                    query.source.as_deref(),
                    query.model.as_deref(),
                    query.today,
                );
                let mut models = BTreeSet::new();
                let mut coverage = BTreeMap::new();
                let mut partial_sources = Vec::new();
                for source in &snapshot.sources {
                    if !scan::checkpoint() {
                        break;
                    }
                    let mut has_tokens = false;
                    if source.ledger.has_partial_records
                        && query.source.as_ref().is_none_or(|id| *id == source.id)
                    {
                        partial_sources.push(source.title.clone());
                    }
                    for day in &source.ledger.days {
                        if !scan::checkpoint() {
                            break;
                        }
                        has_tokens |= day.tokens > 0;
                        if query.source.as_ref().is_none_or(|id| *id == source.id) {
                            models.extend(day.models.keys().cloned());
                        }
                    }
                    coverage.insert(source.id.clone(), has_tokens);
                }
                let bin_size = analysis.days.len().div_ceil(40).max(1);
                let bins = analysis
                    .days
                    .chunks(bin_size)
                    .map(|days| days.iter().map(|day| day.measures.tokens).sum())
                    .collect();
                Self {
                    request,
                    analysis,
                    hours,
                    models: models.into_iter().collect(),
                    bins,
                    coverage,
                    partial_sources,
                    reused_aggregation: false,
                }
            },
        )
    }
}

/// At most four filter results, all from one held snapshot. Pointer identity
/// cannot be recycled while its Arc is retained, and needs no full-store hash.
#[derive(Default)]
pub struct Cache {
    entries: VecDeque<Arc<Summary>>,
}

impl Cache {
    pub fn exact(&mut self, request: &Request) -> Option<Arc<Summary>> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.request.matches(request))?;
        let entry = self.entries.remove(index)?;
        self.entries.push_front(entry.clone());
        Some(entry)
    }

    pub fn aggregation(&self, request: &Request) -> Option<Arc<Summary>> {
        self.entries
            .iter()
            .find(|entry| {
                Arc::ptr_eq(&entry.request.snapshot, &request.snapshot)
                    && entry.request.query.same_aggregation(&request.query)
            })
            .cloned()
    }

    pub fn models(&self, snapshot: &Arc<Snapshot>, source: Option<&str>) -> Option<&[String]> {
        self.entries
            .iter()
            .find(|entry| {
                Arc::ptr_eq(&entry.request.snapshot, snapshot)
                    && entry.request.query.source.as_deref() == source
            })
            .map(|entry| entry.models.as_slice())
    }

    pub fn insert(&mut self, summary: Arc<Summary>) {
        if self
            .entries
            .front()
            .is_some_and(|entry| !Arc::ptr_eq(&entry.request.snapshot, &summary.request.snapshot))
        {
            self.clear();
        }
        self.entries
            .retain(|entry| !entry.request.matches(&summary.request));
        self.entries.push_front(summary);
        self.entries.truncate(4);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::TokenTally;
    use crate::spend::Source;

    fn request() -> Request {
        let sources = [("a", 10), ("b", 25)]
            .into_iter()
            .map(|(id, tokens)| {
                let buckets = BTreeMap::from([(
                    "2026-10-01 09:00".into(),
                    BTreeMap::from([(
                        "unknown".into(),
                        TokenTally {
                            input: tokens,
                            ..Default::default()
                        },
                    )]),
                )]);
                Source {
                    id: id.into(),
                    title: id.into(),
                    location: String::new(),
                    present: true,
                    ledger: crate::ledger::priced(&buckets, &BTreeMap::new(), None),
                }
            })
            .collect();
        Request {
            snapshot: Arc::new(Snapshot { sources }),
            query: Query {
                span: Span::Week,
                today: NaiveDate::from_ymd_opt(2026, 10, 4).unwrap(),
                source: None,
                model: None,
                group: Group::Agents,
                sort: Sort::Tokens,
                descending: true,
            },
        }
    }
    fn compute(request: Request, previous: Option<Arc<Summary>>) -> Arc<Summary> {
        Arc::new(Summary::compute(request, Arc::new(Control::default()), previous).unwrap())
    }

    #[test]
    fn sorting_reuses_counts_and_filter_results_stay_isolated() {
        let initial = request();
        let first = compute(initial.clone(), None);
        assert_eq!(first.analysis.total.tokens, 35);
        assert_eq!(first.analysis.rows[0].id, "b");
        let mut cache = Cache::default();
        cache.insert(first.clone());
        assert!(Arc::ptr_eq(&cache.exact(&initial).unwrap(), &first));

        let mut reordered = initial.clone();
        reordered.query.descending = false;
        let sorted = compute(reordered.clone(), cache.aggregation(&reordered));
        assert!(sorted.reused_aggregation);
        assert_eq!(sorted.analysis.total, first.analysis.total);
        assert_eq!(sorted.analysis.rows[0].id, "a");
        assert_eq!(sorted.hours, first.hours);
        assert_eq!(sorted.analysis.page(0, 1)[0].measures.tokens, 10);

        let mut selected = initial.clone();
        selected.query.source = Some("a".into());
        assert!(cache.aggregation(&selected).is_none());
        let isolated = compute(selected.clone(), Some(first));
        assert!(!isolated.reused_aggregation);
        assert_eq!(isolated.analysis.total.tokens, 10);
        cache.insert(isolated);
        assert_eq!(cache.exact(&initial).unwrap().analysis.total.tokens, 35);
        assert_eq!(cache.exact(&selected).unwrap().analysis.total.tokens, 10);
    }

    #[test]
    fn unreadable_sources_remain_visible_in_cached_and_filtered_summaries() {
        let mut initial = request();
        Arc::make_mut(&mut initial.snapshot).sources[1]
            .ledger
            .has_partial_records = true;
        let first = compute(initial.clone(), None);
        assert_eq!(first.partial_sources, ["b"]);
        let mut reordered = initial.clone();
        reordered.query.descending = false;
        let cached = compute(reordered, Some(first));
        assert!(cached.reused_aggregation);
        assert_eq!(cached.partial_sources, ["b"]);
        let mut clean = initial.clone();
        clean.query.source = Some("a".into());
        assert!(compute(clean, None).partial_sources.is_empty());
        initial.query.source = Some("b".into());
        Arc::make_mut(&mut initial.snapshot).sources[1]
            .ledger
            .days
            .clear();
        assert_eq!(compute(initial, None).partial_sources, ["b"]);
    }

    #[test]
    fn snapshot_date_model_span_and_group_all_invalidate_reuse() {
        let initial = request();
        let summary = compute(initial.clone(), None);
        let mut cache = Cache::default();
        cache.insert(summary);
        let mut changed = initial.clone();
        changed.snapshot = Arc::new((*initial.snapshot).clone());
        assert!(cache.aggregation(&changed).is_none());
        changed = initial.clone();
        changed.query.today = changed.query.today.succ_opt().unwrap();
        assert!(cache.aggregation(&changed).is_none());
        changed = initial.clone();
        changed.query.model = Some("other".into());
        assert!(cache.aggregation(&changed).is_none());
        changed = initial.clone();
        changed.query.span = Span::Today;
        assert!(cache.aggregation(&changed).is_none());
        changed = initial.clone();
        changed.query.group = Group::Models;
        assert!(cache.aggregation(&changed).is_none());
    }

    #[test]
    fn empty_results_are_cached_and_old_snapshots_are_released() {
        let mut initial = request();
        initial.query.model = Some("missing".into());
        let mut cache = Cache::default();
        let empty = compute(initial.clone(), None);
        assert_eq!(empty.analysis.total.tokens, 0);
        cache.insert(empty);
        assert!(cache.exact(&initial).is_some());
        for day in 5..12 {
            let mut next = initial.clone();
            next.query.today = NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
            cache.insert(compute(next, None));
        }
        assert_eq!(cache.entries.len(), 4);
        let weak = Arc::downgrade(&initial.snapshot);
        drop(initial);
        cache.insert(compute(request(), None));
        assert!(weak.upgrade().is_none());
        cache.clear();
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn cancellation_cannot_be_returned_as_a_complete_empty_summary() {
        let control = Arc::new(Control::default());
        control.cancel();
        assert!(Summary::compute(request(), control, None).is_err());
    }
}
