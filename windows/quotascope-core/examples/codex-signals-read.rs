//! Read-only, cancellable smoke check. Prints aggregate facts, never rollout
//! text, session paths, credentials or individual model names.
use quotascope_core::{codex_signal_reader::Reader, scan::Control};
use std::sync::Arc;
fn main() {
    let home = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(quotascope_core::home_dir);
    let control = Arc::new(Control::default());
    let stop = control.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(30));
        stop.cancel();
    });
    let mut reader = Reader::default();
    for days in [Some(30), Some(90)] {
        let began = std::time::Instant::now();
        let result = quotascope_core::scan::run(
            control.clone(),
            1,
            |_| {},
            || reader.read(&home, days, chrono::Local::now().date_naive()),
        );
        let Ok(report) = result else {
            println!(
                "{}",
                serde_json::json!({"cancelled":true,"complete":false,"deadlineSeconds":30})
            );
            return;
        };
        let (responses, eligible, hits) = report.models.values().fold(
            (0u64, 0u64, 0u64),
            |(responses, eligible, hits), counts| {
                (
                    responses + counts.responses,
                    eligible + counts.reached,
                    hits + counts.hits,
                )
            },
        );
        println!(
            "{}",
            serde_json::json!({
                "days":days,"milliseconds":began.elapsed().as_millis(),"sessions":report.sessions,
                "comparableSettingsSessions":report.judged_sessions,"partial":report.partial,
                "models":report.models.len(),"positiveReasoningResponses":responses,
                "eligibleResponses":eligible,"latticeHits":hits,"recordedDifferences":report.changes_count,
                "filesParsed":report.files_parsed,"filesReused":report.files_reused,
                "failedFiles":report.failed_files,"cacheFiles":reader.retained_files(),
                "cacheAccountedBytes":reader.estimated_bytes(),"algorithm":quotascope_core::codex_signals::ALGORITHM
            })
        );
    }
}
