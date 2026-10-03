//! Synthetic immutable-snapshot probe, with no disk or account reads.
use chrono::NaiveDate;
use quotascope_core::ledger::{self, TokenTally};
use quotascope_core::scan::Control;
use quotascope_core::spend::{Group, Snapshot, Sort, Source, Span};
use quotascope_core::spend_analysis::{Cache, Query, Request, Summary};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let sources = 16;
    let days = 180;
    let models = 24;
    let today = NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
    let mut snapshot = Snapshot::default();
    for source in 0..sources {
        let buckets = (0..days)
            .map(|day| {
                let date = today - chrono::Duration::days(day);
                let entries = (0..models)
                    .map(|model| {
                        (
                            format!("fixture-{model}"),
                            TokenTally {
                                input: 100 + source,
                                output: 50,
                                cache_read: model,
                                ..Default::default()
                            },
                        )
                    })
                    .collect();
                (format!("{date} 09:00"), entries)
            })
            .collect();
        snapshot.sources.push(Source {
            id: format!("fixture-source-{source}"),
            title: format!("Fixture {source}"),
            location: String::new(),
            present: true,
            ledger: ledger::priced(&buckets, &BTreeMap::new(), None),
        });
    }
    let request = Request {
        snapshot: Arc::new(snapshot),
        query: Query {
            today,
            span: Span::All,
            source: None,
            model: None,
            group: Group::Models,
            sort: Sort::Tokens,
            descending: true,
        },
    };
    let started = Instant::now();
    let summary =
        Arc::new(Summary::compute(request.clone(), Arc::new(Control::default()), None).unwrap());
    let initial_us = started.elapsed().as_micros();
    let expected = summary.analysis.clone();
    let hours = summary.hours;
    let mut cache = Cache::default();
    cache.insert(summary.clone());
    let iterations = 30;
    for path in ["reaggregate", "exact-cache", "sort-reuse"] {
        let started = Instant::now();
        for round in 0..iterations {
            match path {
                "reaggregate" => {
                    let analysis = request.snapshot.analyze(
                        Span::All,
                        today,
                        None,
                        None,
                        Group::Models,
                        Sort::Tokens,
                        true,
                    );
                    let hourly = request.snapshot.hourly(Span::All, None, None, today);
                    assert_eq!(analysis, expected);
                    assert_eq!(hourly, hours);
                    black_box(analysis);
                }
                "exact-cache" => {
                    let cached = cache.exact(&request).unwrap();
                    assert!(Arc::ptr_eq(&cached, &summary));
                    black_box(cached);
                }
                _ => {
                    let mut sorted = request.clone();
                    sorted.query.descending = round % 2 == 0;
                    sorted.query.sort = Sort::Name;
                    let reused = Summary::compute(
                        sorted.clone(),
                        Arc::new(Control::default()),
                        cache.aggregation(&sorted),
                    )
                    .unwrap();
                    assert!(reused.reused_aggregation);
                    assert_eq!(reused.analysis.total, expected.total);
                    assert_eq!(reused.hours, hours);
                    black_box(reused);
                }
            }
        }
        println!(
            "{}",
            serde_json::json!({
                "path":path, "iterations":iterations,
                "sources":sources, "days":days, "models":models,
                "initialSummaryMicroseconds":initial_us,
                "tokens":expected.total.tokens, "debug":cfg!(debug_assertions),
                "microseconds":started.elapsed().as_micros()
            })
        );
    }
}
