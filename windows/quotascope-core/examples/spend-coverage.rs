//! What this machine's own agent records are worth under the models.dev table,
//! and which spellings the table cannot reach.
//!
//! `cargo run -p quotascope-core --example spend-coverage`
//!
//! It reads the same ledgers the detailed cards read, so it answers the only
//! question that matters when a figure looks too low: is a model unpriced
//! because models.dev publishes no rate for that spelling, or because the
//! lookup never got there?
use quotascope_core::ledger;
use quotascope_core::model::Provider;
use quotascope_core::model_prices;
use std::collections::BTreeMap;

fn main() {
    let table = model_prices::prices();
    println!(
        "models.dev table: {} priced ids (this either reads the cache or downloads)",
        table.len()
    );
    if table.is_empty() {
        eprintln!("no price table: nothing can be priced, so the totals below are all unpriced");
    }

    for provider in [Provider::ClaudeCode, Provider::Codex, Provider::Antigravity] {
        let ledger = ledger::ledger(provider);
        let mut tokens = 0_i64;
        let mut unpriced = 0_i64;
        let mut cost = 0.0_f64;
        let mut per_model: BTreeMap<String, (i64, f64)> = BTreeMap::new();
        for day in &ledger.days {
            tokens += day.tokens;
            unpriced += day.unpriced_tokens;
            cost += day.cost;
            for (model, value) in &day.model_costs {
                let entry = per_model.entry(model.clone()).or_default();
                entry.1 += value.total();
            }
            for (model, count) in &day.models {
                per_model.entry(model.clone()).or_default().0 += *count;
            }
        }
        println!(
            "\n{} — {} tokens, API value ${:.4}, {} tokens unpriced",
            provider.display_name(),
            tokens,
            cost,
            unpriced
        );
        let mut rows: Vec<_> = per_model.into_iter().collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.1 .0));
        for (model, (count, value)) in rows {
            let priced = model_prices::price_for(&model, &table, None).is_some();
            println!(
                "  {:>8} {:>12} tokens  ${:>9.4}  {model}",
                if priced { "priced" } else { "UNPRICED" },
                count,
                value
            );
        }
        if !ledger.unpriced_models.is_empty() {
            println!(
                "  spellings with no published rate: {:?}",
                ledger.unpriced_models
            );
        }
    }
}
