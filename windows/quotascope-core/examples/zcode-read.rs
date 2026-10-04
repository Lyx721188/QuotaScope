//! Offline counters only. The result contains no identities, models, paths,
//! conversation content, credentials, or price/network requests.
use sha2::{Digest, Sha256};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next() else {
        eprintln!("Pass one frozen ZCode usage database path.");
        std::process::exit(2);
    };
    if args.next().is_some() {
        eprintln!("Pass exactly one database path.");
        std::process::exit(2);
    }
    let report = quotascope_core::zcode_spend::read_counts(std::path::Path::new(&path));
    let tally = report.buckets.values().flat_map(|m| m.values()).fold(
        quotascope_core::ledger::TokenTally::default(),
        |sum, tally| sum + *tally,
    );
    let digest = Sha256::digest(serde_json::to_vec(&report.buckets).unwrap());
    println!(
        "{}",
        serde_json::json!({"input":tally.input,"output":tally.output,
            "cacheRead":tally.cache_read,"cacheWrite":tally.cache_write,"total":tally.total(),
            "partial":report.partial,"rowsRead":report.rows_read,"rowsSkipped":report.rows_skipped,
            "bucketDigest":format!("{digest:x}"),"offline":true})
    );
}
