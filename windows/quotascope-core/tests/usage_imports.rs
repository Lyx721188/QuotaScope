//! I/O stress lives in a separate test process from the cookie and CLI fixtures.
use quotascope_core::{
    additional_spend,
    ledger::{TokenTally, UsageLedger},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf};
const MAX_IMPORT_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "qs-import-limits-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn read(source: &str, paths: &[PathBuf]) -> UsageLedger {
    additional_spend::read_with_prices(source, paths, &BTreeMap::new())
}
fn total(ledger: &UsageLedger) -> i64 {
    ledger.days.iter().map(|day| day.tokens).sum()
}
fn import_row() -> Value {
    json!({"schema":"quotascope.usage.v1","source":"zed","id":"request",
        "timestamp":1790850000000_i64,"model":"fixture",
        "usage":{"inputTokens":10,"outputTokens":7,"cacheReadTokens":5,
            "cacheWriteTokens":3,"reasoningTokens":4,"totalTokens":25}})
}
fn transcript() -> Vec<u8> {
    [json!({"type":"request/header","data":{"header":{"config":{"model":"native-fixture"}}}}),
     json!({"type":"assistant/message","seq":1,"time":1790850000000_i64,
        "data":{"message":{"id":"native"},"usage":{"inputTokens":10,"outputTokens":8,"cacheReadTokens":2}}})]
        .into_iter().map(|row|row.to_string()+"\n").collect::<String>().into_bytes()
}
#[test]
fn every_import_source_has_depth_and_file_count_limits() {
    let fixture = Fixture::new();
    for source in ["zed", "zcode"] {
        let imports = fixture.0.join("UsageImports").join(source);
        let deep = (0..10).fold(imports.clone(), |path, _| path.join("nested"));
        std::fs::create_dir_all(&deep).unwrap();
        let mut row = import_row();
        row["source"] = json!(source);
        std::fs::write(deep.join("usage.jsonl"), row.to_string() + "\n").unwrap();
        let skipped = read(source, std::slice::from_ref(&imports));
        assert!(skipped.has_partial_records && skipped.days.is_empty());
        for index in 0..1025 {
            std::fs::write(
                imports.join(format!("{index:04}.jsonl")),
                row.to_string() + "\n",
            )
            .unwrap();
        }
        let capped = read(source, &[imports]);
        assert!(capped.has_partial_records);
        assert_eq!(total(&capped), 25);
    }
}

#[test]
fn import_file_and_line_bounds_preserve_only_readable_prefixes() {
    let fixture = Fixture::new();
    let imports = fixture.0.join("UsageImports/zed");
    std::fs::create_dir_all(&imports).unwrap();
    let path = imports.join("usage.json");
    let mut row = import_row();
    row["padding"] = json!("x".repeat(MAX_IMPORT_FILE_BYTES as usize));
    std::fs::write(&path, row.to_string()).unwrap();
    let skipped = read("zed", std::slice::from_ref(&imports));
    assert!(skipped.has_partial_records && skipped.days.is_empty());
    std::fs::remove_file(path).unwrap();
    let prefix = import_row().to_string() + "\n";
    row["padding"] = json!("x".repeat(MAX_LINE_BYTES));
    row["id"] = json!("too-large");
    std::fs::write(
        imports.join("usage.jsonl"),
        prefix + &row.to_string() + "\n",
    )
    .unwrap();
    let bounded = read("zed", &[imports]);
    assert!(bounded.has_partial_records);
    assert_eq!(total(&bounded), 25);
}

#[test]
fn streamed_import_arrays_keep_valid_prefix_and_reject_a_damaged_tail() {
    let fixture = Fixture::new();
    let imports = fixture.0.join("UsageImports/zed");
    std::fs::create_dir_all(&imports).unwrap();
    let one = import_row();
    let mut two = import_row();
    two["id"] = json!("second");
    std::fs::write(imports.join("usage.json"), format!("[{one},{two},{{")).unwrap();
    let prefix = read("zed", std::slice::from_ref(&imports));
    assert!(prefix.has_partial_records);
    assert_eq!(total(&prefix), 50);
    std::fs::write(imports.join("usage.json"), format!("[{one},{two}]")).unwrap();
    let repaired = read("zed", &[imports]);
    assert!(!repaired.has_partial_records);
    assert_eq!(total(&repaired), 50);
}

#[test]
fn import_identity_budget_is_shared_across_files_and_keeps_native_usage() {
    use std::io::Write;
    let fixture = Fixture::new();
    let native = fixture.0.join("native");
    std::fs::create_dir_all(&native).unwrap();
    std::fs::write(native.join("session.jsonl"), transcript()).unwrap();
    let imports = fixture.0.join("UsageImports/dsh");
    std::fs::create_dir_all(&imports).unwrap();
    for part in 0..2 {
        let mut output = std::io::BufWriter::new(
            std::fs::File::create(imports.join(format!("{part}.json"))).unwrap(),
        );
        output.write_all(b"[").unwrap();
        for index in 0..10000 {
            if index != 0 {
                output.write_all(b",").unwrap();
            }
            let mut row = import_row();
            row["source"] = json!("dsh");
            row["id"] = json!(format!("{part}-{index}-{}", "x".repeat(300)));
            serde_json::to_writer(&mut output, &row).unwrap();
        }
        output.write_all(b"]").unwrap();
        output.flush().unwrap();
    }
    let capped = read("dsh", &[imports, native]);
    assert!(capped.has_partial_records);
    assert!(total(&capped) > 20 && total(&capped) < 20 + 20000 * 25);
    assert_eq!(
        capped
            .days
            .iter()
            .filter_map(|day| day.model_tallies.get("native-fixture"))
            .map(TokenTally::total)
            .sum::<i64>(),
        20
    );
}
