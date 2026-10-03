//! Synthetic lookup probe. No local transcripts, credentials or network reads.
use quotascope_core::model_prices::{self, ModelPrice};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Instant;

fn price(input: f64) -> ModelPrice {
    ModelPrice {
        input,
        output: input * 2.0,
        cache_read: Some(input / 10.0),
        cache_write: None,
        name: None,
    }
}

fn main() {
    let mut table: BTreeMap<_, _> = (0..1000)
        .map(|n| (format!("fixture-model-{n}"), price(1.0)))
        .collect();
    table.insert("gemini-3.8-flash".into(), price(0.75));
    let iterations = 10_000;
    for (kind, model, expected) in [
        ("alias", "Gemini 3.8 Flash (High)", Some(0.75)),
        ("unpublished", "unpublished-model", None),
        ("exact", "gemini-3.8-flash", Some(0.75)),
    ] {
        for path in ["single", "indexed"] {
            // Construction and first resolution are included in the indexed
            // timing; fixture construction is outside both paths' timer.
            let started = Instant::now();
            let mut lookup = model_prices::ModelPriceLookup::new(&table);
            let mut hits = 0;
            let mut sum = 0.0;
            for _ in 0..iterations {
                let result = if path == "indexed" {
                    lookup.price(black_box(model), None)
                } else {
                    model_prices::price_for(black_box(model), black_box(&table), None)
                };
                assert_eq!(result.as_ref().map(|p| p.input), expected);
                if let Some(result) = result {
                    hits += 1;
                    sum += black_box(result.input);
                }
            }
            assert_eq!(hits, if expected.is_some() { iterations } else { 0 });
            println!(
                "{}",
                serde_json::json!({
                    "operation":kind, "path":path, "iterations":iterations,
                    "tableEntries":table.len(), "hits":hits, "rateSum":sum,
                    "debug":cfg!(debug_assertions),
                    "microseconds":started.elapsed().as_micros()
                })
            );
        }
    }
}
