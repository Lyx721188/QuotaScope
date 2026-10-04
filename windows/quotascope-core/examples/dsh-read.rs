//! Offline aggregate-only verification. Pass a frozen transcript directory;
//! no prices/network, raw messages, model names or paths appear in the result.
fn main() {
    let paths: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();
    if paths.is_empty() {
        eprintln!("Pass at least one DSH transcript path or directory.");
        std::process::exit(2);
    }
    let ledger =
        quotascope_core::additional_spend::read_with_prices("dsh", &paths, &Default::default());
    let tally = ledger.days.iter().fold(
        quotascope_core::ledger::TokenTally::default(),
        |sum, day| sum + day.tally,
    );
    println!(
        "{}",
        serde_json::json!({"input":tally.input,"output":tally.output,"cacheRead":tally.cache_read,
        "cacheWrite":tally.cache_write,"total":tally.total(),"partial":ledger.has_partial_records,
        "offline":true,"assistantMessagesOnly":true})
    );
}
