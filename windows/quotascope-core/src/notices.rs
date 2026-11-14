//! Installed-version messages and the one optional GitHub banner.
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

pub const REPOSITORY: &str = "https://github.com/Lyx721188/QuotaScope";
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Record {
    pub last_version: Option<String>,
    pub pending_version: Option<String>,
    pub dismissed_version: Option<String>,
    pub active_since: Option<i64>,
    pub active_days: u32,
    pub last_active_day: String,
    pub star_shown: bool,
}
impl Record {
    pub fn launch(&mut self, version: &str, had_run: bool, now: i64) {
        if self.last_version.as_deref() != Some(version) {
            self.pending_version = (self.last_version.is_some() || had_run).then(|| version.into());
            self.last_version = Some(version.into());
        }
        self.note_active(now);
    }
    pub fn note_active(&mut self, now: i64) {
        let Some(at) = chrono::DateTime::from_timestamp_millis(now) else {
            return;
        };
        self.active_since.get_or_insert(now);
        let day = at.with_timezone(&chrono::Local).date_naive().to_string();
        if self.last_active_day != day {
            self.last_active_day = day;
            self.active_days = self.active_days.saturating_add(1);
        }
    }
    pub fn star_due(&self, now: i64) -> bool {
        !self.star_shown
            && self.active_days >= 7
            && self
                .active_since
                .is_some_and(|since| now.saturating_sub(since) >= 14 * 86_400_000)
    }
    fn message_due(&self, version: &str) -> bool {
        self.pending_version.as_deref() == Some(version)
            && self.dismissed_version.as_deref() != Some(version)
    }
}
#[derive(Default)]
struct State {
    started: bool,
    text: Option<String>,
}
fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}
pub fn initialize() {
    let now = crate::timeutil::now_ms();
    crate::settings::mutate(|s| s.notices.launch(env!("CARGO_PKG_VERSION"), s.has_run, now));
}
pub fn note_active() {
    let now = crate::timeutil::now_ms();
    let day = chrono::Local::now().date_naive().to_string();
    if crate::settings::with(|s| s.notices.last_active_day != day) {
        crate::settings::mutate(|s| s.notices.note_active(now));
    }
}
pub fn claim_star() -> bool {
    if !crate::settings::with(|s| s.notices.star_due(crate::timeutil::now_ms())) {
        return false;
    }
    crate::settings::mutate(|s| {
        if !s.notices.star_due(crate::timeutil::now_ms()) {
            return false;
        }
        s.notices.star_shown = true;
        true
    })
}
pub fn dismiss_message() {
    crate::settings::mutate(|s| {
        s.notices.dismissed_version = Some(env!("CARGO_PKG_VERSION").into())
    });
    state().lock().unwrap().text = None;
}
pub fn message() -> Option<String> {
    if !crate::settings::with(|s| s.notices.message_due(env!("CARGO_PKG_VERSION"))) {
        return None;
    }
    state().lock().unwrap().text.clone()
}
/// One bounded public request per launch, using the application's proxy.
pub fn fetch_if_due() {
    if !crate::settings::with(|s| s.notices.message_due(env!("CARGO_PKG_VERSION"))) {
        return;
    }
    let mut state = state().lock().unwrap();
    if state.started {
        return;
    }
    state.started = true;
    drop(state);
    std::thread::spawn(|| {
        use std::io::Read;
        let language = match crate::localization::current() {
            crate::localization::Language::Chinese => "zh-Hans",
            crate::localization::Language::TraditionalChinese => "zh-Hant",
            crate::localization::Language::Japanese => "ja",
            crate::localization::Language::Korean => "ko",
            crate::localization::Language::Russian => "ru",
            _ => "en",
        };
        let fallback = reqwest::blocking::Client::new();
        let client = crate::proxy::client(&fallback);
        let url = format!(
            "https://raw.githubusercontent.com/Lyx721188/QuotaScope/main/Messages/{}/{language}.md",
            env!("CARGO_PKG_VERSION")
        );
        let Ok(response) = client
            .get(url)
            .timeout(std::time::Duration::from_secs(12))
            .header("User-Agent", "QuotaScope/Windows")
            .send()
        else {
            return;
        };
        if !response.status().is_success() {
            return;
        }
        let mut bytes = Vec::new();
        if response.take(65537).read_to_end(&mut bytes).is_err() || bytes.len() > 65536 {
            return;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            return;
        };
        let text = text.trim();
        if !text.is_empty() {
            self::state().lock().unwrap().text = Some(text.into());
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn updates_keep_pending_notes_until_dismissed_and_fresh_installs_do_not() {
        let mut record = Record::default();
        record.launch("1.0.0", false, 1000);
        assert!(!record.message_due("1.0.0"));
        record.launch("1.1.0", true, 2000);
        assert!(record.message_due("1.1.0"));
        record.launch("1.1.0", true, 3000);
        assert!(record.message_due("1.1.0"));
        record.dismissed_version = Some("1.1.0".into());
        assert!(!record.message_due("1.1.0"));
        record.launch("1.2.0", true, 4000);
        assert!(record.message_due("1.2.0"));
    }
    #[test]
    fn star_requires_both_elapsed_time_and_active_days_and_is_shown_once() {
        let mut record = Record::default();
        record.note_active(1000);
        assert!(!record.star_due(15 * 86400000));
        for day in 1..7 {
            record.note_active(day * 86400000 + 1000);
        }
        assert!(record.star_due(15 * 86400000));
        record.star_shown = true;
        assert!(!record.star_due(16 * 86400000));
    }
}
