//! Selection/lifetime state for one serial Codex signal worker. The UI only
//! sees the completed summary; it never locks the reader or walks rollouts.
use quotascope_core::{
    codex_signal_reader::{Reader, Report},
    scan::Control,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Query {
    pub home: PathBuf,
    pub days: Option<u32>,
    pub today: chrono::NaiveDate,
}

pub(crate) struct Summary {
    pub report: Report,
    pub models: Vec<(String, quotascope_core::codex_signals::Counts)>,
    pub hidden_models: usize,
}
impl Summary {
    pub fn from_report(mut report: Report) -> Self {
        let mut models: Vec<_> = std::mem::take(&mut report.models).into_iter().collect();
        models.sort_by(|a, b| b.1.reached.cmp(&a.1.reached).then_with(|| a.0.cmp(&b.0)));
        let hidden_models = models.len().saturating_sub(64);
        models.truncate(64);
        Self {
            report,
            models,
            hidden_models,
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub query: Option<Query>,
    pub result: Option<Arc<Summary>>,
    pub reader: Option<Arc<Mutex<Reader>>>,
    pub generation: u64,
    pub worker_running: bool,
    pub control: Option<Arc<Control>>,
    pub dirty: bool,
    pub failed: bool,
    pub cancelled: bool,
}
impl State {
    pub fn select(&mut self, query: Query, force: bool) {
        if self.query.as_ref() == Some(&query) && !force {
            return;
        }
        if let Some(control) = &self.control {
            control.cancel();
        }
        self.generation = self.generation.wrapping_add(1);
        if self.query.as_ref() != Some(&query) {
            self.result = None;
        }
        self.query = Some(query);
        self.failed = false;
        self.cancelled = false;
        self.dirty = true;
        if force {
            self.reader = None;
        }
        self.reader
            .get_or_insert_with(|| Arc::new(Mutex::new(Reader::default())));
    }
    pub fn release(&mut self) {
        self.cancel();
        self.query = None;
        self.result = None;
        self.reader = None;
        self.failed = false;
        self.cancelled = false;
        // Keep worker_running until that worker exits. Reopening can queue
        // a fresh reader but cannot spawn a concurrent scan.
    }
    pub fn cancel(&mut self) {
        if let Some(control) = &self.control {
            control.cancel();
        }
        self.generation = self.generation.wrapping_add(1);
        self.cancelled = true;
        self.dirty = false;
    }
    pub fn accepts(&self, query: &Query, generation: u64) -> bool {
        self.query.as_ref() == Some(query) && self.generation == generation && !self.cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn query() -> Query {
        Query {
            home: PathBuf::from("fixture"),
            days: Some(30),
            today: chrono::NaiveDate::from_ymd_opt(2026, 10, 4).unwrap(),
        }
    }
    #[test]
    fn periods_keep_the_reader_refresh_replaces_it_and_stale_results_are_rejected() {
        let mut state = State::default();
        let old_query = query();
        state.select(old_query.clone(), false);
        let reader = state.reader.clone().unwrap();
        let generation = state.generation;
        let control = Arc::new(Control::default());
        state.control = Some(control.clone());
        let mut quarter = query();
        quarter.days = Some(90);
        state.select(quarter.clone(), false);
        assert!(control.is_cancelled() && Arc::ptr_eq(&reader, state.reader.as_ref().unwrap()));
        assert!(!state.accepts(&old_query, generation));
        let generation = state.generation;
        state.select(quarter.clone(), true);
        assert!(!Arc::ptr_eq(&reader, state.reader.as_ref().unwrap()));
        assert!(!state.accepts(&quarter, generation));
        assert!(state.accepts(&quarter, state.generation));
    }
    #[test]
    fn close_and_cancel_keep_one_worker_and_preserve_only_completed_snapshots() {
        let mut state = State::default();
        state.select(query(), false);
        state.worker_running = true;
        state.result = Some(Arc::new(Summary::from_report(Report::default())));
        state.select(query(), true);
        state.cancel();
        assert!(state.result.is_some() && !state.dirty && state.worker_running);
        let generation = state.generation;
        state.release();
        assert!(state.reader.is_none() && state.result.is_none() && state.worker_running);
        state.select(query(), false);
        assert!(!state.accepts(&query(), generation));
    }
}
