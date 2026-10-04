//! The settings window, on **Windows Reactor** — Microsoft's official
//! declarative WinUI 3 library for Rust, shipped in `windows-rs` (May 2026).
//!
//! The first Windows build drew its own controls with Direct2D; this one
//! uses the real WinUI 3 controls — `NavigationView`, `ToggleSwitch`,
//! `ComboBox`, `PasswordBox` — so the look and the accessibility tree are
//! WinUI's own.
//!
//! The window lives on the main thread with one Reactor host, opened on
//! demand and hidden when closed. The app pushes per-provider status lines
//! into a shared snapshot; a background poller wakes the component when it
//! changes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows_reactor::SlotsControl;
use windows_reactor::*;

use quotascope_core::model::Provider;
use quotascope_core::spend_analysis::{
    Cache as AnalysisCache, Query as AnalysisQuery, Request as AnalysisRequest,
    Summary as SpendSummary,
};

/// Messages the settings window sends the app. The app owns the panel, the
/// store and the tray; the window only describes what the user asked for.
pub enum SettingsAction {
    /// A setting changed; the app re-reads everything it drives.
    Changed,
    RefreshProvider(String),
    RefreshAccount(quotascope_core::model::AccountKey),
    SaveKey,
    SignInCopilot,
    SignOutCopilot,
    OpenUrl(String),
    OpenFolder(std::path::PathBuf),
    CheckUpdates,
    SkipUpdate(String),
}

/// What the app pushes to the window while it is open. The poller watches
/// the generation counter and wakes the component when it moves.
#[derive(Default)]
struct SettingsSnapshot {
    generation: u64,
    status: HashMap<String, String>,
    readings: HashMap<String, quotascope_core::model::ProviderUsage>,
    dashboard_requested: bool,
    spend: Option<Arc<quotascope_core::spend::Snapshot>>,
    spend_generation: u64,
    spend_loading: bool,
    spend_worker_running: bool,
    spend_failed: bool,
    spend_cancelled: bool,
    spend_control: Option<Arc<quotascope_core::scan::Control>>,
    spend_progress: Option<quotascope_core::scan::Progress>,
    spend_analysis: SpendAnalysisState,
    update: UpdateStatus,
    maintenance_running: bool,
    cache: Option<quotascope_core::statistics_cache::Inventory>,
    maintenance_result: Option<MaintenanceResult>,
    diagnostic_path: Option<std::path::PathBuf>,
    account_details: HashMap<Provider, Arc<AccountDetails>>,
    account_detail_controls: HashMap<Provider, Arc<quotascope_core::scan::Control>>,
    account_detail_generation: u64,
    codex_signals: crate::codex_signal_state::State,
    console_operations: HashMap<String, ConsoleOperation>,
    service_status: HashMap<quotascope_core::service_status::Page, ServiceStatusState>,
}

#[derive(Default)]
struct ServiceStatusState {
    generation: u64,
    running: bool,
    attempted: Option<std::time::Instant>,
    reading: Option<quotascope_core::service_status::Reading>,
    failed: bool,
}

impl ServiceStatusState {
    fn release(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.attempted = None;
        self.reading = None;
        self.failed = false;
    }
    fn complete(
        &mut self,
        generation: u64,
        visible: bool,
        result: Result<
            quotascope_core::service_status::Reading,
            quotascope_core::service_status::ReadError,
        >,
    ) {
        self.running = false;
        if self.generation == generation && visible {
            self.failed = result.is_err();
            if let Ok(reading) = result {
                self.reading = Some(reading);
            }
        }
    }
}

#[derive(Default)]
struct ConsoleOperation {
    generation: u64,
    running: bool,
    message: Option<String>,
}

struct AccountDetails {
    models: Vec<quotascope_core::model_details::Model>,
    cache: quotascope_core::prompt_cache::CacheReading,
    partial: bool,
    timings_partial: bool,
}

#[derive(Default)]
struct SpendAnalysisState {
    request: Option<AnalysisRequest>,
    result: Option<Arc<SpendSummary>>,
    cache: AnalysisCache,
    generation: u64,
    worker_running: bool,
    control: Option<Arc<quotascope_core::scan::Control>>,
    failed: bool,
    cancelled: bool,
}

impl SpendAnalysisState {
    fn select(&mut self, request: AnalysisRequest) {
        if self
            .request
            .as_ref()
            .is_some_and(|old| old.matches(&request))
        {
            return;
        }
        if let Some(control) = &self.control {
            control.cancel();
        }
        self.generation = self.generation.wrapping_add(1);
        self.result = self.cache.exact(&request);
        self.request = Some(request);
        self.failed = false;
        self.cancelled = false;
    }

    fn release(&mut self) {
        if let Some(control) = &self.control {
            control.cancel();
        }
        self.generation = self.generation.wrapping_add(1);
        self.request = None;
        self.result = None;
        self.cache.clear();
        self.failed = false;
        self.cancelled = false;
        // A reopened page waits for the cancelled worker to leave its loop.
    }

    fn cancel(&mut self) {
        if let Some(control) = &self.control {
            control.cancel();
        }
        self.generation = self.generation.wrapping_add(1);
        self.cancelled = true;
    }

    fn accepts(&self, request: &AnalysisRequest, generation: u64) -> bool {
        self.generation == generation
            && !self.cancelled
            && self
                .request
                .as_ref()
                .is_some_and(|current| current.matches(request))
    }
}

enum SpendAnalysisStatus {
    Ready(Arc<SpendSummary>),
    Loading,
    Failed,
    Cancelled,
}

impl SettingsSnapshot {
    fn set_spend(&mut self, snapshot: Arc<quotascope_core::spend::Snapshot>) {
        if self
            .spend
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, &snapshot))
        {
            self.spend_analysis.release();
        }
        self.spend = Some(snapshot);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum MaintenanceOperation {
    Inspect,
    Clear,
    Enforce,
    Export,
}

#[derive(Clone)]
enum MaintenanceResult {
    Inspected,
    Cleaned(quotascope_core::statistics_cache::Cleanup),
    Exported,
    Failed,
}

#[derive(Default, Clone)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    Done(quotascope_core::updates::CheckResult),
}

pub(crate) struct Shared {
    snapshot: Mutex<SettingsSnapshot>,
    actions: Sender<SettingsAction>,
    /// Whether Settings is visible. The WinUI host survives hiding it.
    alive: AtomicBool,
    show_requested: AtomicBool,
    shutdown_requested: AtomicBool,
    window_ready: AtomicBool,
}

impl Shared {
    fn request_service_status(
        self: &Arc<Self>,
        page: quotascope_core::service_status::Page,
        force: bool,
    ) {
        if !self.alive.load(Ordering::SeqCst) {
            return;
        }
        let generation = {
            let mut state = self.snapshot.lock().unwrap();
            let status = state.service_status.entry(page).or_default();
            if status.running
                || (!force
                    && status.attempted.is_some_and(|at| {
                        at.elapsed() < quotascope_core::service_status::CHECK_INTERVAL
                    }))
            {
                return;
            }
            status.running = true;
            status.attempted = Some(std::time::Instant::now());
            let generation = status.generation;
            state.generation = state.generation.wrapping_add(1);
            generation
        };
        let shared = self.clone();
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(|| quotascope_core::service_status::read(page, force))
                    .unwrap_or(Err(quotascope_core::service_status::ReadError::Unreadable));
            let mut state = shared.snapshot.lock().unwrap();
            let status = state.service_status.entry(page).or_default();
            status.complete(generation, shared.alive.load(Ordering::SeqCst), result);
            state.generation = state.generation.wrapping_add(1);
        });
    }

    fn release_service_status(&self, page: Option<quotascope_core::service_status::Page>) {
        let mut state = self.snapshot.lock().unwrap();
        for (key, status) in &mut state.service_status {
            if page.is_none_or(|page| page == *key) {
                status.release();
            }
        }
        state.generation = state.generation.wrapping_add(1);
    }

    fn change_deepseek_console(
        self: &Arc<Self>,
        account: quotascope_core::model::AccountKey,
        import: bool,
    ) {
        if account.provider != Provider::DeepSeek {
            return;
        }
        let generation = {
            let mut state = self.snapshot.lock().unwrap();
            let operation = state.console_operations.entry(account.id()).or_default();
            if operation.running {
                return;
            }
            operation.generation = operation.generation.wrapping_add(1);
            operation.running = true;
            operation.message = None;
            let generation = operation.generation;
            state.generation = state.generation.wrapping_add(1);
            generation
        };
        let shared = self.clone();
        std::thread::spawn(move || {
            let found = if import {
                std::panic::catch_unwind(quotascope_core::deepseek_session::from_browser)
                    .ok()
                    .flatten()
            } else {
                None
            };
            let mut state = shared.snapshot.lock().unwrap();
            let Some(operation) = state.console_operations.get_mut(&account.id()) else {
                return;
            };
            if operation.generation != generation {
                return;
            }
            // Removing an additional account invalidates this operation under
            // the same lock before clearing its encrypted credential slots.
            let (saved, message) = if import {
                match found {
                    Some((token, browser)) => {
                        let saved = quotascope_core::deepseek_session::set_token(&account, &token);
                        (
                            saved,
                            if saved {
                                quotascope_core::localization::t_fmt(
                                    "Imported the session from {browser}.",
                                    &[&browser],
                                )
                            } else {
                                quotascope_core::localization::t(
                                    "Couldn't save the console session.",
                                )
                                .into()
                            },
                        )
                    }
                    None => (
                        false,
                        quotascope_core::localization::t(
                            "No matching session found in your browsers.",
                        )
                        .into(),
                    ),
                }
            } else {
                let saved = quotascope_core::deepseek_session::set_token(&account, "");
                (
                    saved,
                    quotascope_core::localization::t(if saved {
                        "Console session cleared."
                    } else {
                        "Couldn't save the console session."
                    })
                    .into(),
                )
            };
            if saved {
                quotascope_core::deepseek_history::forget(&account);
            }
            operation.running = false;
            operation.message = Some(message);
            state.generation = state.generation.wrapping_add(1);
            drop(state);
            if saved {
                shared.send(SettingsAction::SaveKey);
                if quotascope_core::settings::with(|s| s.enabled_accounts.contains(&account.id())) {
                    shared.send(SettingsAction::RefreshAccount(account));
                }
            }
        });
    }

    fn invalidate_console_operation(&self, id: &str) {
        self.snapshot.lock().unwrap().console_operations.remove(id);
    }

    pub(crate) fn request_dashboard(&self) {
        let mut state = self.snapshot.lock().unwrap();
        state.dashboard_requested = true;
        state.generation += 1;
    }
    fn release_account_details(&self) {
        self.release_service_status(None);
        let mut state = self.snapshot.lock().unwrap();
        for control in state.account_detail_controls.values() {
            control.cancel();
        }
        state.account_details.clear();
        state.codex_signals.release();
        state.account_detail_generation = state.account_detail_generation.wrapping_add(1);
        state.generation = state.generation.wrapping_add(1);
    }

    fn request_account_details(self: &Arc<Self>, provider: Provider, force_refresh: bool) {
        if !matches!(provider, Provider::ClaudeCode | Provider::Codex)
            || !self.alive.load(Ordering::SeqCst)
            || !quotascope_core::settings::with(|s| s.reads_token_spend)
        {
            return;
        }
        let (generation, control) = {
            let mut state = self.snapshot.lock().unwrap();
            if state.account_detail_controls.contains_key(&provider) {
                return;
            }
            let control = Arc::new(quotascope_core::scan::Control::default());
            state
                .account_detail_controls
                .insert(provider, control.clone());
            state.generation = state.generation.wrapping_add(1);
            (state.account_detail_generation, control)
        };
        let shared = self.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                quotascope_core::scan::run(
                    control,
                    3,
                    |_| {},
                    || {
                        if force_refresh {
                            quotascope_core::ledger::invalidate_memory();
                        }
                        let ledger = quotascope_core::ledger::ledger(provider);
                        let now = quotascope_core::timeutil::now_ms();
                        let timings = quotascope_core::ledger::transcript_root(provider)
                            .map(|root| {
                                quotascope_core::model_details::read_timings(provider, &root, now)
                            })
                            .unwrap_or_default();
                        AccountDetails {
                            models: quotascope_core::model_details::models(
                                &ledger,
                                &timings.models,
                                30,
                            ),
                            cache: quotascope_core::prompt_cache::read_for(
                                provider,
                                &quotascope_core::home_dir(),
                                now,
                            ),
                            partial: ledger.has_partial_records,
                            timings_partial: timings.partial,
                        }
                    },
                )
            });
            let mut state = shared.snapshot.lock().unwrap();
            state.account_detail_controls.remove(&provider);
            if generation == state.account_detail_generation
                && shared.alive.load(Ordering::SeqCst)
                && quotascope_core::settings::with(|s| s.reads_token_spend)
            {
                if let Ok(Ok(details)) = result {
                    state.account_details.insert(provider, Arc::new(details));
                }
            }
            state.generation = state.generation.wrapping_add(1);
        });
    }

    fn request_codex_signals(
        self: &Arc<Self>,
        query: crate::codex_signal_state::Query,
        force: bool,
    ) {
        if !self.alive.load(Ordering::SeqCst)
            || !quotascope_core::settings::with(|s| s.reads_token_spend)
        {
            return;
        }
        let mut state = self.snapshot.lock().unwrap();
        let signals = &mut state.codex_signals;
        signals.select(query, force);
        if !signals.dirty || signals.worker_running || signals.failed || signals.cancelled {
            return;
        }
        signals.worker_running = true;
        state.generation = state.generation.wrapping_add(1);
        let shared = self.clone();
        drop(state);
        std::thread::spawn(move || shared.run_codex_signals());
    }

    fn run_codex_signals(self: Arc<Self>) {
        loop {
            let (query, generation, control, reader) = {
                let mut state = self.snapshot.lock().unwrap();
                let signals = &mut state.codex_signals;
                if !self.alive.load(Ordering::SeqCst)
                    || !signals.dirty
                    || signals.query.is_none()
                    || signals.failed
                    || signals.cancelled
                {
                    signals.worker_running = false;
                    signals.control = None;
                    state.generation = state.generation.wrapping_add(1);
                    return;
                }
                let control = Arc::new(quotascope_core::scan::Control::default());
                signals.control = Some(control.clone());
                (
                    signals.query.clone().expect("checked above"),
                    signals.generation,
                    control,
                    signals.reader.clone().expect("selection owns a reader"),
                )
            };
            let result = std::panic::catch_unwind(|| {
                quotascope_core::scan::run(
                    control,
                    1,
                    |_| {},
                    || {
                        let report =
                            reader
                                .lock()
                                .unwrap()
                                .read(&query.home, query.days, query.today);
                        crate::codex_signal_state::Summary::from_report(report)
                    },
                )
            });
            let allowed = quotascope_core::settings::with(|s| s.reads_token_spend);
            let mut state = self.snapshot.lock().unwrap();
            let signals = &mut state.codex_signals;
            signals.control = None;
            if signals.accepts(&query, generation) && allowed && self.alive.load(Ordering::SeqCst) {
                signals.dirty = false;
                match result {
                    Ok(Ok(summary)) => signals.result = Some(Arc::new(summary)),
                    Ok(Err(_)) => signals.cancelled = true,
                    Err(_) => {
                        signals.failed = true;
                        signals.reader = None;
                    }
                }
            } else if !allowed || !self.alive.load(Ordering::SeqCst) {
                signals.release();
            }
            state.generation = state.generation.wrapping_add(1);
        }
    }

    fn request_maintenance(self: &Arc<Self>, operation: MaintenanceOperation) {
        use quotascope_core::diagnostics::{Runtime, ScanStatus, UpdateStatus as DiagnosticUpdate};
        let runtime = {
            let mut state = self.snapshot.lock().unwrap();
            if state.maintenance_running
                || (state.spend_worker_running
                    && matches!(
                        operation,
                        MaintenanceOperation::Clear | MaintenanceOperation::Enforce
                    ))
            {
                return;
            }
            state.maintenance_running = true;
            state.maintenance_result = None;
            state.generation = state.generation.wrapping_add(1);
            let progress = state.spend_progress.clone().unwrap_or_default();
            Runtime {
                settings_visible: self.alive.load(Ordering::SeqCst),
                scan_status: if state.spend_worker_running {
                    if state.spend_cancelled
                        || state
                            .spend_control
                            .as_ref()
                            .is_some_and(|c| c.is_cancelled())
                    {
                        ScanStatus::Stopping
                    } else {
                        ScanStatus::Running
                    }
                } else if state.spend_failed {
                    ScanStatus::Failed
                } else if state.spend_cancelled {
                    ScanStatus::Cancelled
                } else if state.spend.is_some() {
                    ScanStatus::Complete
                } else {
                    ScanStatus::Idle
                },
                sources_done: progress.sources_done,
                sources_total: progress.sources_total,
                files_read: progress.files_read,
                update_status: match state.update {
                    UpdateStatus::Idle => DiagnosticUpdate::Idle,
                    UpdateStatus::Checking => DiagnosticUpdate::Checking,
                    UpdateStatus::Done(quotascope_core::updates::CheckResult::Failed) => {
                        DiagnosticUpdate::Failed
                    }
                    UpdateStatus::Done(quotascope_core::updates::CheckResult::UpToDate) => {
                        DiagnosticUpdate::UpToDate
                    }
                    UpdateStatus::Done(_) => DiagnosticUpdate::Available,
                },
                process: Default::default(),
            }
        };
        let shared = self.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| match operation {
                MaintenanceOperation::Inspect => (MaintenanceResult::Inspected, None),
                MaintenanceOperation::Clear => (
                    MaintenanceResult::Cleaned(quotascope_core::ledger::clear_statistics_cache()),
                    None,
                ),
                MaintenanceOperation::Enforce => (
                    MaintenanceResult::Cleaned(quotascope_core::statistics_cache::enforce()),
                    None,
                ),
                MaintenanceOperation::Export => {
                    let mut runtime = runtime;
                    runtime.process = crate::winutil::process_metrics();
                    match quotascope_core::diagnostics::export(runtime) {
                        Ok(path) => (MaintenanceResult::Exported, Some(path)),
                        Err(_) => (MaintenanceResult::Failed, None),
                    }
                }
            });
            let inventory = quotascope_core::statistics_cache::inspect();
            let mut state = shared.snapshot.lock().unwrap();
            state.maintenance_running = false;
            state.cache = Some(inventory);
            let (result, path) = result.unwrap_or((MaintenanceResult::Failed, None));
            state.maintenance_result = Some(result);
            if path.is_some() {
                state.diagnostic_path = path;
            }
            state.generation = state.generation.wrapping_add(1);
            let retry = state.spend_loading
                && !state.spend_worker_running
                && shared.alive.load(Ordering::SeqCst);
            drop(state);
            if retry {
                shared.request_spend(false);
            }
        });
    }

    fn release_spend(&self) {
        let mut state = self.snapshot.lock().unwrap();
        if let Some(control) = &state.spend_control {
            control.cancel();
        }
        state.spend = None;
        state.spend_analysis.release();
        state.spend_generation = state.spend_generation.wrapping_add(1);
        state.spend_loading = false;
        state.spend_failed = false;
        state.spend_cancelled = false;
        state.spend_progress = None;
        state.generation = state.generation.wrapping_add(1);
    }

    fn cancel_spend(&self) {
        let mut state = self.snapshot.lock().unwrap();
        if (!state.spend_worker_running && !state.spend_analysis.worker_running)
            || (state.spend_cancelled && state.spend_analysis.cancelled)
        {
            return;
        }
        if let Some(control) = &state.spend_control {
            control.cancel();
        }
        state.spend_generation = state.spend_generation.wrapping_add(1);
        state.spend_loading = false;
        state.spend_cancelled = true;
        state.spend_analysis.cancel();
        state.spend_failed = false;
        state.generation = state.generation.wrapping_add(1);
    }

    fn request_spend(self: &Arc<Self>, refresh: bool) {
        quotascope_core::spend_warmer::cancel_running();
        let allowed = quotascope_core::settings::with(|s| s.reads_token_spend);
        if !allowed {
            self.release_spend();
            return;
        }
        if !refresh {
            if let Some(snapshot) = quotascope_core::spend_warmer::snapshot() {
                let mut state = self.snapshot.lock().unwrap();
                state.set_spend(snapshot);
                state.spend_loading = false;
                state.generation += 1;
                return;
            }
        }
        let (generation, control) = {
            let mut state = self.snapshot.lock().unwrap();
            if state.maintenance_running {
                state.spend_loading = true;
                state.generation = state.generation.wrapping_add(1);
                return;
            }
            if state.spend_worker_running {
                // A reopened Spend page waits for the existing scan instead
                // of starting another one over the same transcript files.
                state.spend_loading = true;
                state.spend_cancelled = false;
                state.generation = state.generation.wrapping_add(1);
                return;
            }
            state.spend_generation = state.spend_generation.wrapping_add(1);
            state.generation += 1;
            state.spend_loading = true;
            state.spend_failed = false;
            state.spend_cancelled = false;
            state.spend_progress = None;
            let control = Arc::new(quotascope_core::scan::Control::default());
            state.spend_control = Some(control.clone());
            state.spend_worker_running = true;
            (state.spend_generation, control)
        };
        let shared = self.clone();
        std::thread::spawn(move || {
            let progress_shared = shared.clone();
            let result = std::panic::catch_unwind(|| {
                quotascope_core::spend::Snapshot::read_controlled(
                    control,
                    refresh,
                    move |progress| {
                        let mut state = progress_shared.snapshot.lock().unwrap();
                        if state.spend_generation == generation && state.spend_loading {
                            state.spend_progress = Some(progress);
                            state.generation = state.generation.wrapping_add(1);
                        }
                    },
                )
            });
            let allowed = quotascope_core::settings::with(|s| s.reads_token_spend);
            let mut state = shared.snapshot.lock().unwrap();
            state.spend_worker_running = false;
            state.spend_control = None;
            state.generation = state.generation.wrapping_add(1);
            if state.spend_generation != generation {
                let retry = state.spend_loading && allowed && shared.alive.load(Ordering::SeqCst);
                state.spend_loading = false;
                drop(state);
                if retry {
                    shared.request_spend(false);
                }
                return;
            }
            state.spend_loading = false;
            state.spend_failed = result.is_err();
            match result {
                Ok(Ok(snapshot)) if allowed => state.set_spend(Arc::new(snapshot)),
                Ok(Err(_)) => state.spend_cancelled = true,
                _ => {}
            }
            if !allowed {
                state.spend = None;
                state.spend_analysis.release();
            }
        });
    }

    fn request_spend_analysis(self: &Arc<Self>, request: AnalysisRequest) -> SpendAnalysisStatus {
        let allowed = quotascope_core::settings::with(|settings| settings.reads_token_spend);
        let mut state = self.snapshot.lock().unwrap();
        if !allowed
            || !self.alive.load(Ordering::SeqCst)
            || state
                .spend
                .as_ref()
                .is_none_or(|snapshot| !Arc::ptr_eq(snapshot, &request.snapshot))
        {
            return SpendAnalysisStatus::Cancelled;
        }
        let analysis = &mut state.spend_analysis;
        analysis.select(request);
        if let Some(result) = &analysis.result {
            return SpendAnalysisStatus::Ready(result.clone());
        }
        if analysis.failed {
            return SpendAnalysisStatus::Failed;
        }
        if analysis.cancelled {
            return SpendAnalysisStatus::Cancelled;
        }
        if !analysis.worker_running {
            analysis.worker_running = true;
            state.generation = state.generation.wrapping_add(1);
            let shared = self.clone();
            drop(state);
            std::thread::spawn(move || shared.run_spend_analysis());
        }
        SpendAnalysisStatus::Loading
    }

    fn run_spend_analysis(self: Arc<Self>) {
        loop {
            let (request, generation, control, previous) = {
                let mut state = self.snapshot.lock().unwrap();
                let analysis = &mut state.spend_analysis;
                if !self.alive.load(Ordering::SeqCst)
                    || analysis.request.is_none()
                    || analysis.result.is_some()
                    || analysis.failed
                    || analysis.cancelled
                {
                    analysis.worker_running = false;
                    analysis.control = None;
                    state.generation = state.generation.wrapping_add(1);
                    return;
                }
                let request = analysis.request.clone().expect("checked above");
                let previous = analysis.cache.aggregation(&request);
                let control = Arc::new(quotascope_core::scan::Control::default());
                analysis.control = Some(control.clone());
                (request, analysis.generation, control, previous)
            };
            let result = std::panic::catch_unwind(|| {
                SpendSummary::compute(request.clone(), control, previous)
            });
            let allowed = quotascope_core::settings::with(|settings| settings.reads_token_spend);
            let mut state = self.snapshot.lock().unwrap();
            let live = state
                .spend
                .as_ref()
                .is_some_and(|snapshot| Arc::ptr_eq(snapshot, &request.snapshot));
            let analysis = &mut state.spend_analysis;
            analysis.control = None;
            if analysis.accepts(&request, generation)
                && live
                && allowed
                && self.alive.load(Ordering::SeqCst)
            {
                match result {
                    Ok(Ok(summary)) => {
                        let summary = Arc::new(summary);
                        analysis.cache.insert(summary.clone());
                        analysis.result = Some(summary);
                    }
                    Ok(Err(_)) => analysis.cancelled = true,
                    Err(_) => analysis.failed = true,
                }
            } else if !allowed
                || !self.alive.load(Ordering::SeqCst)
                || (analysis.accepts(&request, generation) && !live)
            {
                analysis.release();
            }
            state.generation = state.generation.wrapping_add(1);
            // A superseded task advances straight to the newest pending key;
            // no second aggregation thread can overlap it.
        }
    }
    fn generation(&self) -> u64 {
        self.snapshot.lock().unwrap().generation
    }
    /// A copy of the latest statuses. This must not drain: the map stays
    /// put so a render that lands between pushes — an extra watcher fire,
    /// a nav — still reads the last known lines instead of an empty frame
    /// that makes every status row blink.
    fn read_status(&self) -> HashMap<String, String> {
        self.snapshot.lock().unwrap().status.clone()
    }
    fn send(&self, action: SettingsAction) {
        let _ = self.actions.send(action);
    }
}

/// The app's handle on the settings window. `show` asks the main thread —
/// where the Reactor host lives — to open it; the window is one at a time.
pub struct SettingsHost {
    shared: Arc<Shared>,
    open_tx: Sender<()>,
}

impl SettingsHost {
    pub fn new(actions: Sender<SettingsAction>, open_tx: Sender<()>) -> Self {
        SettingsHost {
            shared: Arc::new(Shared {
                snapshot: Mutex::new(SettingsSnapshot::default()),
                actions,
                alive: AtomicBool::new(false),
                show_requested: AtomicBool::new(false),
                shutdown_requested: AtomicBool::new(false),
                window_ready: AtomicBool::new(false),
            }),
            open_tx,
        }
    }

    /// The handle the main thread needs to serve the window.
    pub(crate) fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
    }

    /// One provider's status line for the Accounts page. A line that
    /// hasn't changed doesn't move the generation — the store polls every
    /// 250 ms and re-rendering the whole window four times a second for
    /// identical text is what made the rows flicker.
    pub fn set_status(&self, provider_raw: &str, text: String) {
        let mut snapshot = self.shared.snapshot.lock().unwrap();
        if snapshot.status.get(provider_raw) == Some(&text) {
            return;
        }
        snapshot.status.insert(provider_raw.to_string(), text);
        snapshot.generation += 1;
    }
    pub fn set_readings(&self, readings: &HashMap<String, quotascope_core::model::ProviderUsage>) {
        let mut state = self.shared.snapshot.lock().unwrap();
        if state.readings != *readings {
            state.readings = readings.clone();
            state.generation += 1;
        }
    }
    pub fn show_dashboard(&mut self) {
        self.shared.snapshot.lock().unwrap().dashboard_requested = true;
        self.show();
    }

    /// Something the window shows has changed out from under it.
    pub fn refresh(&self) {
        self.shared.snapshot.lock().unwrap().generation += 1;
    }

    pub fn set_update_status(&self, update: UpdateStatus) {
        let mut snapshot = self.shared.snapshot.lock().unwrap();
        snapshot.update = update;
        snapshot.generation = snapshot.generation.wrapping_add(1);
    }

    pub fn is_open(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }

    /// Opens or foregrounds the existing window.
    pub fn show(&mut self) {
        self.shared.alive.store(true, Ordering::SeqCst);
        if self.open_tx.send(()).is_err() {
            self.shared.alive.store(false, Ordering::SeqCst);
        }
    }
}

/// Starts the sole WinUI host on the main thread. Closing Settings hides
/// the window; disconnecting the tray's request channel ends the host.
pub(crate) fn serve(shared: &Arc<Shared>, open_rx: Receiver<()>) {
    // The window is up: flips the worker's gate so statuses start flowing.
    // `show` already set this for the tray path; setting it here too covers
    // a launch that skipped the host — `quotascope --settings`.
    shared.alive.store(true, Ordering::SeqCst);
    SHARED.with(|cell| *cell.borrow_mut() = Some(shared.clone()));
    let relay = shared.clone();
    std::thread::spawn(move || {
        while open_rx.recv().is_ok() {
            relay.show_requested.store(true, Ordering::SeqCst);
        }
        relay.shutdown_requested.store(true, Ordering::SeqCst);
    });
    if let Err(error) = App::run_component::<SettingsApp>(()) {
        eprintln!("Settings host: {error}");
    }
    shared.alive.store(false, Ordering::SeqCst);
    SHARED.with(|cell| *cell.borrow_mut() = None);
}

/// Destroying the final WinUI window ends its Application. Intercept the
/// native close on the owning UI thread so reopening never restarts XAML.
unsafe extern "system" fn settings_subclass(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    id: usize,
    _data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, WM_CLOSE, WM_NCDESTROY};
    if msg == WM_CLOSE {
        let hide = SHARED.with(|cell| {
            let shared = cell.borrow();
            let Some(shared) = shared.as_ref() else {
                return false;
            };
            if shared.shutdown_requested.load(Ordering::SeqCst) {
                return false;
            }
            shared.alive.store(false, Ordering::SeqCst);
            shared.release_spend();
            shared.release_account_details();
            true
        });
        if hide {
            let _ = ShowWindow(hwnd, SW_HIDE);
            return windows::Win32::Foundation::LRESULT(0);
        }
    } else if msg == WM_NCDESTROY {
        let _ = windows::Win32::UI::Shell::RemoveWindowSubclass(hwnd, Some(settings_subclass), id);
    }
    windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam)
}

fn settings_hwnd() -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, GetWindowTextW};
    unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
        let mut title = [0u16; 128];
        let len = GetWindowTextW(hwnd, &mut title);
        if String::from_utf16_lossy(&title[..len.max(0) as usize])
            == quotascope_core::localization::t("QuotaScope Settings")
        {
            *(data.0 as *mut Option<HWND>) = Some(hwnd);
            return false.into();
        }
        true.into()
    }
    let mut hwnd = None;
    unsafe {
        let _ = EnumThreadWindows(
            windows::Win32::System::Threading::GetCurrentThreadId(),
            Some(find),
            LPARAM(&mut hwnd as *mut _ as isize),
        );
    }
    hwnd
}

thread_local! {
    static SHARED: std::cell::RefCell<Option<Arc<Shared>>> =
        const { std::cell::RefCell::new(None) };
}

/// The taskbar and title-bar icon. `AppWindow.SetIcon` wants an .ico file
/// path, so the icon embedded in the exe is written into the app data
/// directory once and reused from there. `None` (the write failed) just
/// means the window keeps the generic icon.
fn window_icon() -> Option<&'static str> {
    use std::sync::OnceLock;
    static ICON: OnceLock<Option<&'static str>> = OnceLock::new();
    *ICON.get_or_init(|| {
        let path = quotascope_core::data_dir().join("app.ico");
        if !path.exists() {
            std::fs::create_dir_all(path.parent()?).ok()?;
            std::fs::write(&path, include_bytes!("../assets/app.ico")).ok()?;
        }
        Some(Box::leak(
            path.into_os_string().into_string().ok()?.into_boxed_str(),
        ))
    })
}

// --- The component ----------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    General,
    Dashboard,
    Accounts,
    Spend,
    Notifications,
    Maintenance,
    About,
}

impl Page {
    fn tag(self) -> &'static str {
        match self {
            Page::General => "general",
            Page::Dashboard => "dashboard",
            Page::Accounts => "accounts",
            Page::Spend => "spend",
            Page::Notifications => "notifications",
            Page::Maintenance => "maintenance",
            Page::About => "about",
        }
    }
    fn from_tag(tag: &str) -> Page {
        match tag {
            "dashboard" => Page::Dashboard,
            "accounts" => Page::Accounts,
            "spend" => Page::Spend,
            "notifications" => Page::Notifications,
            "maintenance" => Page::Maintenance,
            "about" => Page::About,
            _ => Page::General,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Dashboard => "Dashboard",
            Page::Accounts => "Accounts",
            Page::Spend => "Token spend",
            Page::Notifications => "Notifications",
            Page::Maintenance => "Storage and diagnostics",
            Page::About => "About",
        }
    }
    fn glyph(self) -> &'static str {
        match self {
            Page::General => "\u{E713}",
            Page::Dashboard => "\u{E80F}",
            Page::Accounts => "\u{E77B}",
            Page::Spend => "\u{E9D9}",
            Page::Notifications => "\u{EA8F}",
            Page::Maintenance => "\u{E7B8}",
            Page::About => "\u{E946}",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToggleKey {
    SidePct,
    Remaining,
    Clock,
    SecondRing,
    Forecast,
    AutoCollapse,
    FollowDisplay,
    HideTrayIcon,
    TokenSpend,
    BackgroundSpend,
    AnimatedBots,
    GlobalShortcuts,
    CodexResetCredits,
    Startup,
    Alerts,
    AlertReset,
    AlertFailure,
    AlertOutage,
    CheckUpdates,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChoiceKey {
    ProxyMode,
    Dock,
    Language,
    PanelSize,
    RailSpacing,
    ClockDirection,
    WarningThreshold,
    Interval,
    AlertThreshold,
    StatisticsCache,
}

#[derive(Clone, PartialEq)]
enum Message {
    Tick,
    SpendChoice(u8, Option<usize>),
    SpendPage(bool),
    SpendDrill(String),
    SpendRefresh,
    SpendCancel,
    Maintenance(MaintenanceOperation),
    OpenDiagnosticsFolder,
    Nav(Option<String>),
    Toggle(ToggleKey, bool),
    ToggleEnabled(usize, bool),
    ExpandAccount(usize),
    RefreshAccountDetails(usize),
    ServiceStatusRefresh(quotascope_core::service_status::Page),
    ServiceStatusOpen(quotascope_core::service_status::Page),
    SignalPeriod(Option<usize>),
    SignalRefresh,
    SignalCancel,
    ToggleExtension(usize, bool),
    RescanExtensions,
    Choice(ChoiceKey, Option<usize>),
    KeyEdit(usize, String),
    AddressEdit(usize, String),
    Search(String),
    RevealKey(usize, bool),
    Detailed(usize, bool),
    BalanceBasis(usize, Option<usize>),
    BudgetEdit(usize, String),
    FloorEdit(usize, String),
    SaveBalance(usize),
    Save(usize),
    Refresh(usize),
    RefreshAccount(String),
    ImportSession(usize),
    ProxyEdit(String),
    SaveProxy,
    ImportStorage(usize),
    AddAccount(usize),
    RemoveAccount(String),
    ImportOpenCodeConsole,
    ImportClaudeSession,
    ForgetClaudeSession,
    ForgetOpenCodeConsole,
    DeepSeekConsole(String, bool),
    DetailedAccount(String, bool),
    CopilotAuth,
    OpenGitHub,
    CheckUpdates,
    OpenUpdate(String),
    SkipUpdate(String),
}

struct SettingsApp {
    shared: Arc<Shared>,
    hwnd: Option<windows::Win32::Foundation::HWND>,
    page: Page,
    spend_group: quotascope_core::spend::Group,
    spend_sort: quotascope_core::spend::Sort,
    spend_descending: bool,
    spend_page: usize,
    spend_source: Option<String>,
    spend_model: Option<String>,
    dark: bool,
    /// Draft key text per provider raw, until Saved.
    keys: HashMap<String, String>,
    /// Draft gateway address per provider raw, until Saved.
    addresses: HashMap<String, String>,
    search: String,
    expanded_accounts: std::collections::HashSet<usize>,
    revealed_keys: std::collections::HashSet<usize>,
    budgets: HashMap<String, String>,
    floors: HashMap<String, String>,
    /// Status lines pushed by the app, per provider raw.
    status: HashMap<String, String>,
    /// The extensions a scan of the extensions folder found, and why the
    /// folders it turned away were turned away.
    extensions: Vec<quotascope_core::extension::Extension>,
    extension_problems: Vec<quotascope_core::extension::Problem>,
    poll: Option<ComponentTask>,
}

impl Component for SettingsApp {
    type Input = ();
    type Message = Message;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        if let Err(error) = crate::fonts::install_winui_font() {
            eprintln!("HarmonyOS Sans settings font: {error}");
        }
        let shared = SHARED.with(|cell| cell.borrow().clone().expect("settings shared state"));
        let scan = quotascope_core::extension::scan();
        SettingsApp {
            shared: shared.clone(),
            hwnd: None,
            page: if shared.snapshot.lock().unwrap().dashboard_requested {
                Page::Dashboard
            } else {
                Page::General
            },
            spend_group: Default::default(),
            spend_sort: Default::default(),
            spend_descending: true,
            spend_page: 0,
            spend_source: None,
            spend_model: None,
            // XAML follows the system appearance; the muted ink follows it
            // too, read once here — a reopened window re-reads.
            dark: crate::theme::panel::is_dark(),
            keys: HashMap::new(),
            addresses: quotascope_core::settings::with(|s| s.server_addresses.clone()),
            search: String::new(),
            expanded_accounts: Default::default(),
            revealed_keys: Default::default(),
            budgets: HashMap::new(),
            floors: HashMap::new(),
            status: HashMap::new(),
            extensions: scan.extensions,
            extension_problems: scan.problems,
            poll: Some(Self::spawn_watcher(&shared, context)),
        }
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        match message {
            Message::Tick => {
                if self.hwnd.is_none() {
                    if let Some(hwnd) = settings_hwnd() {
                        if unsafe {
                            windows::Win32::UI::Shell::SetWindowSubclass(
                                hwnd,
                                Some(settings_subclass),
                                1,
                                0,
                            )
                        }
                        .as_bool()
                        {
                            self.hwnd = Some(hwnd);
                            self.shared.window_ready.store(true, Ordering::SeqCst);
                        }
                    }
                }
                if self.shared.shutdown_requested.load(Ordering::SeqCst) {
                    let _ = context.window().request_close();
                } else if self.hwnd.is_some()
                    && self.shared.show_requested.swap(false, Ordering::SeqCst)
                {
                    if self.page == Page::Spend {
                        self.shared.release_spend();
                    }
                    let requested = std::mem::take(
                        &mut self.shared.snapshot.lock().unwrap().dashboard_requested,
                    );
                    self.page = if requested {
                        Page::Dashboard
                    } else {
                        Page::General
                    };
                    self.revealed_keys.clear();
                    self.keys.clear();
                    self.addresses =
                        quotascope_core::settings::with(|s| s.server_addresses.clone());
                    self.dark = crate::theme::panel::is_dark();
                    unsafe {
                        use windows::Win32::UI::WindowsAndMessaging::{
                            SetForegroundWindow, ShowWindow, SW_RESTORE,
                        };
                        let hwnd = self.hwnd.unwrap();
                        let _ = ShowWindow(hwnd, SW_RESTORE);
                        let _ = SetForegroundWindow(hwnd);
                    }
                }
                if !self.shared.alive.load(Ordering::SeqCst) {
                    self.revealed_keys.clear();
                    self.keys.clear();
                }
                self.status = self.shared.read_status();
                self.refresh_visible_service_status();
            }
            Message::Nav(tag) => {
                let previous = self.page;
                self.page = tag.as_deref().map(Page::from_tag).unwrap_or(self.page);
                if previous == Page::Spend && self.page != Page::Spend {
                    self.shared.release_spend();
                }
                if previous == Page::Accounts && self.page != Page::Accounts {
                    self.shared.release_account_details();
                }
                if self.page == Page::Spend {
                    self.shared.request_spend(false);
                }
                if self.page == Page::Maintenance {
                    self.shared
                        .request_maintenance(MaintenanceOperation::Inspect);
                }
                self.refresh_visible_service_status();
                self.revealed_keys.clear();
            }
            Message::SpendRefresh => self.shared.request_spend(true),
            Message::SpendCancel => self.shared.cancel_spend(),
            Message::Maintenance(operation) => self.shared.request_maintenance(operation),
            Message::OpenDiagnosticsFolder => {
                if let Some(path) = &self.shared.snapshot.lock().unwrap().diagnostic_path {
                    if let Some(folder) = path.parent() {
                        self.shared
                            .send(SettingsAction::OpenFolder(folder.to_owned()));
                    }
                }
            }
            Message::SpendPage(next) => {
                self.spend_page = if next {
                    self.spend_page.saturating_add(1)
                } else {
                    self.spend_page.saturating_sub(1)
                };
            }
            Message::SpendDrill(id) => {
                match self.spend_group {
                    quotascope_core::spend::Group::Agents => {
                        self.spend_source = Some(id);
                        self.spend_group = quotascope_core::spend::Group::Models;
                    }
                    quotascope_core::spend::Group::Models => {
                        self.spend_model = Some(id);
                        self.spend_group = quotascope_core::spend::Group::Days;
                    }
                    _ => {}
                }
                self.spend_page = 0;
            }
            Message::SpendChoice(key, Some(index)) => {
                use quotascope_core::spend::{Group, Sort, Span};
                match key {
                    0 => {
                        if let Some(span) = Span::ALL.get(index) {
                            quotascope_core::settings::mutate(|s| s.spend_span = *span);
                        }
                    }
                    1 => {
                        self.spend_group = [Group::Agents, Group::Models, Group::Days]
                            .get(index)
                            .copied()
                            .unwrap_or_default()
                    }
                    2 => self.spend_sort = Sort::ALL.get(index).copied().unwrap_or_default(),
                    3 => self.spend_descending = index == 0,
                    4 => {
                        let state = self.shared.snapshot.lock().unwrap();
                        self.spend_source = index.checked_sub(1).and_then(|i| {
                            state.spend.as_ref()?.sources.get(i).map(|s| s.id.clone())
                        });
                        self.spend_model = None;
                    }
                    5 => {
                        self.spend_model = index
                            .checked_sub(1)
                            .and_then(|i| self.spend_models().get(i).cloned())
                    }
                    _ => {}
                }
                self.spend_page = 0;
            }
            Message::SpendChoice(_, None) => {}
            Message::ExpandAccount(index) => {
                if !self.expanded_accounts.insert(index) {
                    self.expanded_accounts.remove(&index);
                    self.revealed_keys.remove(&index);
                    if let Some(page) = all_providers()
                        .get(index)
                        .copied()
                        .and_then(quotascope_core::service_status::Page::for_provider)
                    {
                        self.shared.release_service_status(Some(page));
                    }
                    if all_providers().get(index) == Some(&Provider::Codex) {
                        let mut state = self.shared.snapshot.lock().unwrap();
                        state.codex_signals.release();
                        state.generation = state.generation.wrapping_add(1);
                    }
                } else if let Some(provider) = all_providers().get(index) {
                    self.shared.request_account_details(*provider, false);
                    if let Some(page) =
                        quotascope_core::service_status::Page::for_provider(*provider)
                    {
                        self.shared.request_service_status(page, false);
                    }
                }
            }
            Message::ServiceStatusRefresh(page) => self.shared.request_service_status(page, true),
            Message::ServiceStatusOpen(page) => self
                .shared
                .send(SettingsAction::OpenUrl(page.address().into())),
            Message::RefreshAccountDetails(index) => {
                if let Some(provider) = all_providers().get(index) {
                    self.shared.request_account_details(*provider, true);
                }
            }
            Message::SignalPeriod(Some(index)) => {
                self.shared.request_codex_signals(
                    Self::signal_query(
                        [Some(30), Some(90), None]
                            .get(index)
                            .copied()
                            .unwrap_or(Some(30)),
                    ),
                    false,
                );
            }
            Message::SignalPeriod(None) => {}
            Message::SignalRefresh => {
                let days = self
                    .shared
                    .snapshot
                    .lock()
                    .unwrap()
                    .codex_signals
                    .query
                    .as_ref()
                    .map(|query| query.days)
                    .unwrap_or(Some(30));
                self.shared
                    .request_codex_signals(Self::signal_query(days), true);
            }
            Message::SignalCancel => {
                let mut state = self.shared.snapshot.lock().unwrap();
                state.codex_signals.cancel();
                state.generation = state.generation.wrapping_add(1);
            }
            Message::Search(text) => {
                self.search = text;
                self.revealed_keys.clear();
                let query = self.search.trim().to_lowercase();
                if !Provider::Codex
                    .display_name()
                    .to_lowercase()
                    .contains(&query)
                    && !Provider::Codex.raw().to_lowercase().contains(&query)
                {
                    let mut state = self.shared.snapshot.lock().unwrap();
                    state.codex_signals.release();
                    state.generation = state.generation.wrapping_add(1);
                }
                for page in quotascope_core::service_status::Page::ALL {
                    if !self.service_status_visible(page) {
                        self.shared.release_service_status(Some(page));
                    }
                }
                self.refresh_visible_service_status();
            }
            Message::RevealKey(index, on) => {
                if on {
                    self.revealed_keys.insert(index);
                } else {
                    self.revealed_keys.remove(&index);
                }
            }
            Message::Detailed(index, on) => {
                if let Some(provider) = all_providers().get(index) {
                    quotascope_core::settings::mutate(|s| {
                        if on {
                            s.detailed_cards.insert(provider.raw().to_string());
                        } else {
                            s.detailed_cards.remove(provider.raw());
                        }
                    });
                    self.revealed_keys.remove(&index);
                    self.shared.send(SettingsAction::Changed);
                }
            }
            Message::BalanceBasis(index, selected) => {
                if let Some(provider) = all_providers().get(index) {
                    let token = match selected.unwrap_or(0) {
                        1 => "balanceOnly",
                        2 => "budget",
                        _ => "sinceTopUp",
                    };
                    quotascope_core::settings::mutate(|s| {
                        s.balance_bases
                            .insert(provider.raw().to_string(), token.into());
                    });
                    self.shared.send(SettingsAction::Changed);
                    self.shared
                        .send(SettingsAction::RefreshProvider(provider.raw().to_string()));
                }
            }
            Message::BudgetEdit(index, text) => {
                if let Some(provider) = all_providers().get(index) {
                    self.budgets.insert(provider.raw().to_string(), text);
                }
            }
            Message::FloorEdit(index, text) => {
                if let Some(provider) = all_providers().get(index) {
                    self.floors.insert(provider.raw().to_string(), text);
                }
            }
            Message::SaveBalance(index) => {
                if let Some(provider) = all_providers().get(index) {
                    let raw = provider.raw();
                    let budget = self.budgets.get(raw).cloned().unwrap_or_else(|| {
                        quotascope_core::balance_ring::basis_for(
                            &quotascope_core::model::AccountKey::primary(*provider),
                        )
                        .1
                        .map(|n| n.to_string())
                        .unwrap_or_default()
                    });
                    let floor = self.floors.get(raw).cloned().unwrap_or_else(|| {
                        quotascope_core::settings::with(|s| {
                            s.low_balance_alerts
                                .get(raw)
                                .map(|n| n.to_string())
                                .unwrap_or_default()
                        })
                    });
                    match (positive_or_empty(&budget), positive_or_empty(&floor)) {
                        (Ok(budget), Ok(floor)) => {
                            let basis = quotascope_core::balance_ring::basis_for(
                                &quotascope_core::model::AccountKey::primary(*provider),
                            )
                            .0;
                            if basis == quotascope_core::balance_ring::Basis::Budget
                                && budget.is_none()
                            {
                                self.status.insert(
                                    raw.into(),
                                    quotascope_core::localization::t(
                                        "Enter a positive budget for My budget.",
                                    )
                                    .into(),
                                );
                                self.poll = Some(Self::spawn_watcher(&self.shared, context));
                                return;
                            }
                            quotascope_core::settings::mutate(|s| {
                                // Store an explicit per-account basis even for
                                // DeepSeek, so clearing a budget cannot revive
                                // a legacy global denominator.
                                s.balance_bases
                                    .insert(raw.to_string(), basis.token().into());
                                if let Some(n) = budget {
                                    s.balance_budgets.insert(raw.to_string(), n);
                                } else {
                                    s.balance_budgets.remove(raw);
                                }
                                if let Some(n) = floor {
                                    s.low_balance_alerts.insert(raw.to_string(), n);
                                } else {
                                    s.low_balance_alerts.remove(raw);
                                }
                            });
                            self.shared.send(SettingsAction::Changed);
                            self.shared
                                .send(SettingsAction::RefreshProvider(raw.to_string()));
                        }
                        _ => {
                            self.status.insert(
                                raw.into(),
                                quotascope_core::localization::t(
                                    "Enter a positive amount, or leave blank to turn it off.",
                                )
                                .into(),
                            );
                        }
                    }
                }
            }
            Message::Toggle(key, value) => {
                Self::apply_toggle(key, value);
                if key == ToggleKey::TokenSpend && !value {
                    self.shared.release_account_details();
                }
                if key != ToggleKey::CheckUpdates {
                    self.shared.send(SettingsAction::Changed);
                }
                if key == ToggleKey::TokenSpend {
                    if self.page == Page::Spend {
                        self.shared.request_spend(false);
                    } else {
                        self.shared.release_spend();
                    }
                }
            }
            Message::ToggleEnabled(index, value) => {
                self.revealed_keys.remove(&index);
                if let Some(provider) = all_providers().get(index) {
                    let raw = provider.raw();
                    quotascope_core::settings::mutate(move |s| {
                        if value {
                            s.enabled_accounts.insert(raw.to_string());
                        } else {
                            s.enabled_accounts.remove(raw);
                        }
                    });
                    self.shared.send(SettingsAction::Changed);
                }
            }
            Message::ToggleExtension(index, value) => {
                if let Some(extension) = self.extensions.get(index) {
                    let id = extension.account().id();
                    quotascope_core::settings::mutate(move |s| {
                        if value {
                            s.enabled_accounts.insert(id.clone());
                        } else {
                            s.enabled_accounts.remove(&id);
                        }
                    });
                    self.shared.send(SettingsAction::Changed);
                }
            }
            Message::RescanExtensions => {
                let scan = quotascope_core::extension::scan();
                self.extensions = scan.extensions;
                self.extension_problems = scan.problems;
                self.shared.send(SettingsAction::Changed);
            }
            Message::Choice(ChoiceKey::StatisticsCache, Some(index)) => {
                let blocked = {
                    let state = self.shared.snapshot.lock().unwrap();
                    state.maintenance_running || state.spend_worker_running
                };
                if !blocked {
                    if let Some(value) = quotascope_core::statistics_cache::LIMITS_MB.get(index) {
                        if quotascope_core::settings::with(|s| {
                            s.statistics_cache_limit_mb != *value
                        }) {
                            Self::apply_choice(ChoiceKey::StatisticsCache, index);
                            self.shared
                                .request_maintenance(MaintenanceOperation::Enforce);
                        }
                    }
                }
            }
            Message::Choice(ChoiceKey::StatisticsCache, None) => {}
            Message::Choice(key, selected) => {
                Self::apply_choice(key, selected.unwrap_or(0));
                if matches!(key, ChoiceKey::ProxyMode) {
                    quotascope_core::codex_rpc::shutdown();
                }
                self.shared.send(SettingsAction::Changed);
            }
            Message::KeyEdit(index, text) => {
                if let Some(provider) = all_providers().get(index) {
                    self.keys.insert(provider.raw().to_string(), text);
                }
            }
            Message::AddressEdit(index, text) => {
                if let Some(provider) = all_providers().get(index) {
                    self.addresses.insert(provider.raw().to_string(), text);
                }
            }
            Message::Save(index) => {
                if let Some(provider) = all_providers().get(index) {
                    if provider.needs_server_address() {
                        let value = self
                            .addresses
                            .get(provider.raw())
                            .cloned()
                            .unwrap_or_default();
                        if value.trim().is_empty()
                            || quotascope_core::gateway::is_usable(value.trim())
                        {
                            if let Some(key) = self.keys.get(provider.raw()) {
                                quotascope_core::secrets::set_key(provider.raw(), key.trim());
                            }
                            quotascope_core::settings::mutate(|s| {
                                if value.trim().is_empty() {
                                    s.server_addresses.remove(provider.raw());
                                } else {
                                    s.server_addresses.insert(
                                        provider.raw().to_string(),
                                        value.trim().to_string(),
                                    );
                                }
                            });
                            self.shared.send(SettingsAction::SaveKey);
                        } else {
                            self.status.insert(
                                provider.raw().to_string(),
                                quotascope_core::localization::t(
                                    "Enter a local or HTTPS gateway address.",
                                )
                                .to_string(),
                            );
                            self.revealed_keys.remove(&index);
                            self.poll = Some(Self::spawn_watcher(&self.shared, context));
                            return;
                        }
                    } else {
                        if let Some(value) = self.keys.get(provider.raw()) {
                            quotascope_core::secrets::set_key(provider.raw(), value.trim());
                        }
                        self.shared.send(SettingsAction::SaveKey);
                    }
                    self.revealed_keys.remove(&index);
                    self.shared
                        .send(SettingsAction::RefreshProvider(provider.raw().to_string()));
                }
            }
            Message::RefreshAccount(id) => {
                if let Some(account) = quotascope_core::model::AccountKey::from_id(&id) {
                    self.shared.send(SettingsAction::RefreshAccount(account));
                }
            }
            Message::Refresh(index) => {
                if let Some(provider) = all_providers().get(index) {
                    self.shared
                        .send(SettingsAction::RefreshProvider(provider.raw().to_string()));
                }
            }
            Message::ProxyEdit(text) => {
                self.addresses.insert("__proxy".into(), text);
            }
            Message::SaveProxy => {
                let text =
                    self.addresses.get("__proxy").cloned().unwrap_or_else(|| {
                        quotascope_core::settings::with(|s| s.proxy_url.clone())
                    });
                if text.trim().is_empty() || quotascope_core::proxy::valid(text.trim()) {
                    quotascope_core::settings::mutate(|s| {
                        s.proxy_url = text.trim().into();
                        s.proxy_mode = if text.trim().is_empty() {
                            "system".into()
                        } else {
                            "manual".into()
                        };
                    });
                    quotascope_core::codex_rpc::shutdown();
                    self.shared.send(SettingsAction::SaveKey);
                }
            }
            Message::ImportSession(index) => {
                self.revealed_keys.remove(&index);
                if let Some(provider) = all_providers().get(index) {
                    let raw = provider.raw();
                    match quotascope_core::providers::session_spec(*provider).and_then(|spec| {
                        quotascope_core::browser_cookies::session(spec.hosts, spec.cookies)
                    }) {
                        Some(found) => {
                            quotascope_core::secrets::set_key(raw, &found.header);
                            self.keys.remove(raw);
                            self.status.insert(
                                raw.to_string(),
                                quotascope_core::localization::t_fmt(
                                    "Imported the session from {browser}.",
                                    &[&found.browser.name()],
                                ),
                            );
                            self.shared.send(SettingsAction::SaveKey);
                            self.shared
                                .send(SettingsAction::RefreshProvider(raw.to_string()));
                        }
                        None => {
                            self.status.insert(
                                raw.to_string(),
                                quotascope_core::localization::t(
                                    "No matching session found in your browsers.",
                                )
                                .to_string(),
                            );
                        }
                    }
                }
            }
            Message::ImportClaudeSession => {
                let session =
                    quotascope_core::browser_cookies::claude_desktop_session().or_else(|| {
                        quotascope_core::browser_cookies::session(
                            &["claude.ai"],
                            quotascope_core::providers::claude_session::COOKIES,
                        )
                        .filter(|found| {
                            found
                                .header
                                .split(';')
                                .any(|p| p.trim().starts_with("sessionKey="))
                        })
                        .map(|found| found.header)
                    });
                if let Some(session) = session {
                    quotascope_core::secrets::set_key(
                        quotascope_core::providers::claude_session::SECRET,
                        &session,
                    );
                    self.shared.send(SettingsAction::SaveKey);
                    self.shared.send(SettingsAction::RefreshProvider(
                        Provider::ClaudeCode.raw().into(),
                    ));
                } else {
                    self.status.insert(
                        Provider::ClaudeCode.raw().into(),
                        quotascope_core::localization::t(
                            "No matching session found in your browsers.",
                        )
                        .into(),
                    );
                }
            }
            Message::ForgetClaudeSession => {
                quotascope_core::secrets::set_key(
                    quotascope_core::providers::claude_session::SECRET,
                    "",
                );
                self.shared.send(SettingsAction::SaveKey);
            }
            Message::ImportOpenCodeConsole => {
                use quotascope_core::opencode_console as console;
                if let Some(found) =
                    quotascope_core::browser_cookies::session(console::HOSTS, console::COOKIES)
                        .and_then(|found| {
                            console::cookie(&found.header).map(|header| (found.browser, header))
                        })
                {
                    quotascope_core::secrets::set_key(console::SECRET, &found.1);
                    self.status.insert(
                        Provider::OpenCodeGo.raw().into(),
                        quotascope_core::localization::t_fmt(
                            "Imported the session from {browser}.",
                            &[&found.0.name()],
                        ),
                    );
                    self.shared.send(SettingsAction::SaveKey);
                    self.shared.send(SettingsAction::RefreshProvider(
                        Provider::OpenCodeGo.raw().into(),
                    ));
                } else {
                    self.status.insert(
                        Provider::OpenCodeGo.raw().into(),
                        quotascope_core::localization::t(
                            "No matching session found in your browsers.",
                        )
                        .into(),
                    );
                }
            }
            Message::AddAccount(index) => {
                if let Some(provider) = all_providers().get(index) {
                    let credential = self.keys.get(provider.raw()).cloned().unwrap_or_default();
                    if quotascope_core::accounts::add(*provider, &credential).is_some() {
                        self.keys.remove(provider.raw());
                        self.shared.send(SettingsAction::SaveKey);
                        self.shared.send(SettingsAction::Changed);
                    } else {
                        self.status.insert(
                            provider.raw().into(),
                            quotascope_core::localization::t(
                                "Enter a valid credential for the additional account.",
                            )
                            .into(),
                        );
                    }
                }
            }
            Message::RemoveAccount(id) => {
                if let Some(account) = quotascope_core::model::AccountKey::from_id(&id) {
                    self.shared.invalidate_console_operation(&id);
                    quotascope_core::accounts::remove(&account);
                    self.shared.send(SettingsAction::SaveKey);
                    self.shared.send(SettingsAction::Changed);
                }
            }
            Message::ImportStorage(index) => {
                if let Some(provider) = all_providers().get(index) {
                    if let Some((session, browser)) = quotascope_core::browser_storage::windsurf() {
                        quotascope_core::secrets::set_key(provider.raw(), &session);
                        self.status.insert(
                            provider.raw().into(),
                            quotascope_core::localization::t_fmt(
                                "Imported the session from {browser}.",
                                &[&browser],
                            ),
                        );
                        self.shared.send(SettingsAction::SaveKey);
                        self.shared
                            .send(SettingsAction::RefreshProvider(provider.raw().into()));
                    } else {
                        self.status.insert(
                            provider.raw().into(),
                            quotascope_core::localization::t(
                                "No matching session found in your browsers.",
                            )
                            .into(),
                        );
                    }
                }
            }
            Message::DeepSeekConsole(id, import) => {
                if let Some(account) = quotascope_core::model::AccountKey::from_id(&id) {
                    self.shared.change_deepseek_console(account, import);
                }
            }
            Message::DetailedAccount(id, detailed) => {
                quotascope_core::settings::mutate(|s| {
                    if detailed {
                        s.detailed_cards.insert(id);
                    } else {
                        s.detailed_cards.remove(&id);
                    }
                });
                self.shared.send(SettingsAction::Changed);
            }
            Message::ForgetOpenCodeConsole => {
                quotascope_core::secrets::set_key(quotascope_core::opencode_console::SECRET, "");
                self.shared.send(SettingsAction::SaveKey);
                self.shared.send(SettingsAction::RefreshProvider(
                    Provider::OpenCodeGo.raw().into(),
                ));
            }
            Message::CopilotAuth => {
                if quotascope_core::secrets::key_for("copilot").is_some() {
                    quotascope_core::secrets::set_key("copilot", "");
                    self.shared.send(SettingsAction::SignOutCopilot);
                } else {
                    self.shared.send(SettingsAction::SignInCopilot);
                }
            }
            Message::OpenGitHub => {
                self.shared.send(SettingsAction::OpenUrl(
                    "https://github.com/Lyx721188/QuotaScope".into(),
                ));
            }
            Message::CheckUpdates => self.shared.send(SettingsAction::CheckUpdates),
            Message::OpenUpdate(url) => self.shared.send(SettingsAction::OpenUrl(url)),
            Message::SkipUpdate(version) => self.shared.send(SettingsAction::SkipUpdate(version)),
        }
        // Re-arm the watcher whichever way this update came, so the next
        // push from the app wakes us again.
        self.poll = Some(Self::spawn_watcher(&self.shared, context));
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        let nav = NavigationView::new()
            .pane_title("QuotaScope")
            // Auto collapses to an icon strip whenever the XAML decides the
            // window is narrow; a settings pane is a list of words, so the
            // pane is pinned open. The toggle button goes with it — one tap
            // on it used to fold the pane down to bare icons.
            .pane_display_mode(NavigationViewPaneDisplayMode::Left)
            .open_pane_length(200.0)
            .is_pane_toggle_button_visible(false)
            .is_back_button_visible(NavigationViewBackButtonVisible::Collapsed)
            .on_selected_tag_changed(context.callback(Message::Nav))
            .slots([
                SlotView::collection(
                    NavigationViewSlot::MenuItems,
                    [
                        Page::General,
                        Page::Dashboard,
                        Page::Accounts,
                        Page::Spend,
                        Page::Notifications,
                        Page::Maintenance,
                        Page::About,
                    ]
                    .map(|page| {
                        (
                            page.tag(),
                            NavigationViewItem::new()
                                .tag(page.tag())
                                .is_selected(self.page == page)
                                .slots([
                                    SlotView::new(
                                        NavigationViewItemSlot::Content,
                                        TextBlock::new()
                                            .text(quotascope_core::localization::t(page.label())),
                                    ),
                                    SlotView::new(
                                        NavigationViewItemSlot::Icon,
                                        FontIcon::new().glyph(page.glyph()),
                                    ),
                                ]),
                        )
                    }),
                ),
                SlotView::new(
                    NavigationViewSlot::Content,
                    // A different page needs a new scroll viewport. Ordinary
                    // updates keep its identity so typing does not reset it.
                    if self.shared.alive.load(Ordering::SeqCst) {
                        View::keyed_fragment([(
                            self.page.tag(),
                            ScrollViewer::new().content(
                                Border::new()
                                    .padding(Thickness::uniform(28.0))
                                    .content(match self.page {
                                        Page::General => self.general_view(context).into(),
                                        Page::Accounts => self.accounts_view(context),
                                        Page::Dashboard => self.dashboard_view(context),
                                        Page::Spend => self.spend_view(context),
                                        Page::Notifications => self.notifications_view(context),
                                        Page::Maintenance => self.maintenance_view(context),
                                        Page::About => self.about_view(context),
                                    }),
                            ),
                        )])
                    } else {
                        View::empty()
                    },
                ),
            ]);
        let mut visuals = WindowVisuals::new().backdrop(WindowBackdrop::Mica);
        if let Some(icon) = window_icon() {
            visuals = visuals.icon(icon);
        }
        context.window_visuals(visuals);
        context.window_title("QuotaScope Settings");
        nav.into()
    }
}

fn all_providers() -> &'static [Provider] {
    &quotascope_core::model::ALL_PROVIDERS
}

fn positive_or_empty(text: &str) -> Result<Option<f64>, ()> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n > 0.0)
        .map(Some)
        .ok_or(())
}

/// A row of siblings: the closest thing to the missing `hstack`.
fn row(children: impl IntoViews) -> View {
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(12.0)
        .children(children)
}

fn heading(text: &str) -> TextBlock {
    TextBlock::new()
        .text(quotascope_core::localization::t(text))
        .font_size(24.0)
        .font_weight(FontWeight::BOLD)
}

fn section(text: &str) -> TextBlock {
    TextBlock::new()
        .text(quotascope_core::localization::t(text))
        .font_size(16.0)
        .font_weight(FontWeight::SEMI_BOLD)
}

impl SettingsApp {
    fn service_status_visible(&self, page: quotascope_core::service_status::Page) -> bool {
        if self.page != Page::Accounts || !self.shared.alive.load(Ordering::SeqCst) {
            return false;
        }
        let query = self.search.trim().to_lowercase();
        self.expanded_accounts
            .iter()
            .any(|index| all_providers().get(*index) == Some(&page.provider()))
            && (page
                .provider()
                .display_name()
                .to_lowercase()
                .contains(&query)
                || page.provider().raw().to_lowercase().contains(&query))
    }

    fn refresh_visible_service_status(&self) {
        for page in quotascope_core::service_status::Page::ALL {
            if self.service_status_visible(page) {
                self.shared.request_service_status(page, false);
            }
        }
    }

    fn spend_models(&self) -> Vec<String> {
        let state = self.shared.snapshot.lock().unwrap();
        let Some(snapshot) = state.spend.as_ref() else {
            return Vec::new();
        };
        let mut models = state
            .spend_analysis
            .cache
            .models(snapshot, self.spend_source.as_deref())
            .unwrap_or_default()
            .to_vec();
        // Keep the chosen filter while a replacement snapshot is being
        // analyzed, including a model that now has no records. Otherwise an
        // interim one-item ComboBox can reset it to "all models".
        if let Some(model) = &self.spend_model {
            if !models.contains(model) {
                models.push(model.clone());
                models.sort();
            }
        }
        models
    }

    fn spend_choice(
        &self,
        key: u8,
        label: &str,
        options: Vec<String>,
        index: usize,
        context: &ViewContext<Self>,
    ) -> View {
        self.aligned(
            label,
            ComboBox::new()
                .min_width(180.0)
                .max_width(340.0)
                .items_source(options)
                .selected_index(index)
                .on_selection_changed(context.callback(move |i| Message::SpendChoice(key, i)))
                .into(),
        )
    }

    fn dashboard_view(&self, context: &ViewContext<Self>) -> View {
        let state = self.shared.snapshot.lock().unwrap();
        let settings = quotascope_core::settings::with(Clone::clone);
        let rows: Vec<View> = settings
            .ordered_enabled()
            .into_iter()
            .map(|account| {
                let reading = state.readings.get(&account.id());
                let headline = reading.and_then(|u| {
                    u.headline_window(
                        settings
                            .pinned_windows
                            .get(&account.id())
                            .map(String::as_str),
                    )
                });
                let percent = headline
                    .map(|w| format!("{:.0}%", w.used_fraction * 100.0))
                    .unwrap_or_else(|| "—".into());
                self.surface(
                    StackPanel::new().spacing(8.0).children((
                        row((
                            TextBlock::new()
                                .text(format!(
                                    "{}{} · {percent}",
                                    account.provider.display_name(),
                                    if account.is_primary() {
                                        String::new()
                                    } else {
                                        format!(" #{}", account.slot)
                                    }
                                ))
                                .font_size(20.0),
                            Button::new()
                                .on_click(context.message(Message::RefreshAccount(account.id())))
                                .content(quotascope_core::localization::t("Refresh")),
                            if account.is_primary() {
                                View::empty()
                            } else {
                                Button::new()
                                    .on_click(context.message(Message::RemoveAccount(account.id())))
                                    .content(quotascope_core::localization::t("Remove account"))
                                    .into()
                            },
                        )),
                        if account.provider == Provider::DeepSeek && !account.is_primary() {
                            let operation = state.console_operations.get(&account.id());
                            self.deepseek_console_controls(
                                &account,
                                operation.is_some_and(|op| op.running),
                                operation.and_then(|op| op.message.as_deref()),
                                context,
                            )
                        } else {
                            View::empty()
                        },
                        if let Some(window) = headline {
                            ProgressBar::new()
                                .maximum(100.0)
                                .value((window.used_fraction * 100.0).clamp(0.0, 100.0))
                                .into()
                        } else {
                            View::empty()
                        },
                        self.muted(
                            &reading
                                .map(|u| match u.state {
                                    quotascope_core::model::State::Unavailable(reason) => {
                                        reason.message().to_string()
                                    }
                                    _ => u.plan.clone().unwrap_or_default(),
                                })
                                .unwrap_or_default(),
                        ),
                        self.muted(
                            &reading
                                .map(|u| {
                                    let source = u.origin.as_deref().unwrap_or("unknown");
                                    let age = u.observed_at.map(|at| {
                                        (quotascope_core::timeutil::now_ms() - at).max(0) / 1000
                                    });
                                    format!(
                                        "{}: {source} · {}: {} · {}: {} s",
                                        quotascope_core::localization::t("Connection source"),
                                        quotascope_core::localization::t("Cached reading"),
                                        u.is_cached,
                                        quotascope_core::localization::t("Reading age"),
                                        age.map(|s| s.to_string()).unwrap_or_else(|| "—".into())
                                    )
                                })
                                .unwrap_or_default(),
                        ),
                    )),
                )
                .into()
            })
            .collect();
        StackPanel::new()
            .spacing(16.0)
            .children((
                heading("Dashboard"),
                if settings.animated_bots {
                    TextBlock::new()
                        .text(
                            if settings.ordered_enabled().iter().any(|a| {
                                state.readings.get(&a.id()).is_some_and(|u| {
                                    u.windows.iter().any(|w| w.used_fraction >= 0.99)
                                })
                            }) {
                                "○(•︵•)○"
                            } else if (quotascope_core::timeutil::now_ms() / 1000) % 4 == 0 {
                                "○(−‿−)○"
                            } else {
                                "○(•‿•)○"
                            },
                        )
                        .font_size(28.0)
                        .into()
                } else {
                    View::empty()
                },
                View::keyed_fragment(rows.into_iter().enumerate()),
            ))
            .into()
    }

    fn spend_view(&self, context: &ViewContext<Self>) -> View {
        use quotascope_core::spend::{Group, Sort, Span};
        let settings = quotascope_core::settings::with(|s| s.clone());
        let t = quotascope_core::localization::t;
        let mut content = vec![
            heading("Token spend").into(),
            self.muted(t(
                "Local records · API value is an estimate, not your bill.",
            ))
            .into(),
            self.toggle(
                ToggleKey::TokenSpend,
                "Read local token spend",
                settings.reads_token_spend,
                context,
            ),
        ];
        if !settings.reads_token_spend {
            content.push(
                self.muted(t(
                    "Enable local reading to analyze records on this computer.",
                ))
                .into(),
            );
            return StackPanel::new()
                .spacing(18.0)
                .children((View::keyed_fragment(content.into_iter().enumerate()),));
        }
        content.push(self.toggle(
            ToggleKey::BackgroundSpend,
            "Keep token statistics warm in the background (five minutes, 30-second scan limit)",
            settings.background_token_spend,
            context,
        ));
        let (snapshot, loading, running, failed, cancelled, progress, stopping) = {
            let state = self.shared.snapshot.lock().unwrap();
            (
                state.spend.clone(),
                state.spend_loading,
                state.spend_worker_running || state.spend_analysis.worker_running,
                state.spend_failed,
                state.spend_cancelled,
                state.spend_progress.clone(),
                state
                    .spend_control
                    .as_ref()
                    .is_some_and(|c| c.is_cancelled()),
            )
        };
        let status = if running && stopping {
            "Stopping local scan…"
        } else if loading {
            "Reading local records…"
        } else if cancelled {
            "Local scan cancelled."
        } else if failed {
            "Unable to read local records."
        } else {
            "Only records available on this computer are included."
        };
        content.push(row((
            Button::new()
                .is_enabled(!running)
                .on_click(context.message(Message::SpendRefresh))
                .content(t("Refresh")),
            Button::new()
                .is_enabled(running && !cancelled)
                .on_click(context.message(Message::SpendCancel))
                .content(t("Cancel scan")),
            self.muted(t(status)),
        )));
        if let Some(progress) = progress.filter(|_| loading || running) {
            let text = t("Scanning {source} · sources {done}/{total} · files read {files}")
                .replace("{source}", &progress.source)
                .replace("{done}", &progress.sources_done.to_string())
                .replace("{total}", &progress.sources_total.to_string())
                .replace("{files}", &progress.files_read.to_string());
            content.push(self.muted(&text).into());
        }
        if snapshot.is_some() && (loading || running || cancelled || failed) {
            content.push(self.muted(t("Previous results remain displayed.")).into());
        }
        let Some(snapshot) = snapshot else {
            return StackPanel::new()
                .spacing(18.0)
                .children((View::keyed_fragment(content.into_iter().enumerate()),));
        };
        let summary = self.shared.request_spend_analysis(AnalysisRequest {
            snapshot: snapshot.clone(),
            query: AnalysisQuery {
                span: settings.spend_span,
                today: chrono::Local::now().date_naive(),
                source: self.spend_source.clone(),
                model: self.spend_model.clone(),
                group: self.spend_group,
                sort: self.spend_sort,
                descending: self.spend_descending,
            },
        });
        let models = self.spend_models();
        let mut sources = vec![t("All agents").into()];
        sources.extend(snapshot.sources.iter().map(|s| s.title.clone()));
        let source_index = self
            .spend_source
            .as_ref()
            .and_then(|id| snapshot.sources.iter().position(|s| s.id == *id))
            .map(|i| i + 1)
            .unwrap_or(0);
        let mut model_options = vec![t("All models").into()];
        model_options.extend(models.iter().cloned());
        let model_index = self
            .spend_model
            .as_ref()
            .and_then(|id| models.iter().position(|m| m == id))
            .map(|i| i + 1)
            .unwrap_or(0);
        let translated =
            |items: &[&str]| items.iter().map(|s| t(s).to_string()).collect::<Vec<_>>();
        content.push(
            self.surface(
                StackPanel::new().spacing(12.0).children([
                    self.spend_choice(
                        0,
                        "Time range",
                        translated(&[
                            "Today",
                            "Last 7 days",
                            "Last 30 days",
                            "Last 90 days",
                            "All time",
                        ]),
                        Span::ALL
                            .iter()
                            .position(|s| *s == settings.spend_span)
                            .unwrap_or(1),
                        context,
                    ),
                    self.spend_choice(4, "Agent", sources, source_index, context),
                    self.spend_choice(5, "Model", model_options, model_index, context),
                    self.spend_choice(
                        1,
                        "Group by",
                        translated(&["Agents", "Models", "Days"]),
                        match self.spend_group {
                            Group::Agents => 0,
                            Group::Models => 1,
                            Group::Days => 2,
                        },
                        context,
                    ),
                    self.spend_choice(
                        2,
                        "Sort by",
                        translated(&[
                            "Name / date",
                            "Total tokens",
                            "API value",
                            "Input",
                            "Output",
                            "Cache read",
                            "Cache write",
                        ]),
                        Sort::ALL
                            .iter()
                            .position(|s| *s == self.spend_sort)
                            .unwrap_or(1),
                        context,
                    ),
                    self.spend_choice(
                        3,
                        "Order",
                        translated(&["Descending", "Ascending"]),
                        usize::from(!self.spend_descending),
                        context,
                    ),
                ]),
            ),
        );
        let summary = match summary {
            SpendAnalysisStatus::Ready(summary) => summary,
            status => {
                content.push(
                    self.muted(t(match status {
                        SpendAnalysisStatus::Loading => "Calculating token statistics…",
                        SpendAnalysisStatus::Failed => "Unable to calculate token statistics.",
                        _ => "Token statistics calculation cancelled.",
                    }))
                    .into(),
                );
                return StackPanel::new()
                    .spacing(18.0)
                    .children((View::keyed_fragment(content.into_iter().enumerate()),));
            }
        };
        let analysis = &summary.analysis;
        let total = analysis.total;
        let summary_grid = Grid::new()
            .columns([GridLength::Star(1.0), GridLength::Star(1.0)])
            .column_spacing(24.0)
            .children((
                StackPanel::new().spacing(6.0).children((
                    self.muted(t("Total tokens")),
                    TextBlock::new()
                        .text(total.tokens.to_string())
                        .font_size(28.0)
                        .font_weight(FontWeight::SEMI_BOLD),
                    self.muted(&format!(
                        "{} {}",
                        t("Tokens without public prices:"),
                        total.unpriced
                    )),
                    self.muted(&format!(
                        "{} {}",
                        t("Tokens without a kind breakdown:"),
                        total.unclassified
                    )),
                )),
                StackPanel::new().grid_column(1).spacing(6.0).children((
                    self.muted(t("API value")),
                    TextBlock::new()
                        .text(format!("≈ ${:.2}", total.cost))
                        .font_size(28.0)
                        .font_weight(FontWeight::SEMI_BOLD),
                    self.muted(&format!(
                        "{} ${:.2} · {} ${:.2}",
                        t("Input"),
                        total.costs.input,
                        t("Output"),
                        total.costs.output
                    )),
                    self.muted(&format!(
                        "{} ${:.2} · {} ${:.2}",
                        t("Cache read"),
                        total.costs.cache_read,
                        t("Cache write"),
                        total.costs.cache_write
                    )),
                )),
            ));
        content.push(self.surface(summary_grid.into()));
        if !summary.partial_sources.is_empty() {
            content.push(
                self.muted(&quotascope_core::localization::t_fmt(
                    "Incomplete local records: {sources}. Totals cover readable counters only.",
                    &[&summary.partial_sources.join(", ")],
                ))
                .into(),
            );
        }
        if let Some(rate) = total.cache_hit_rate() {
            content.push(
                self.muted(&format!("{} {:.0}%", t("Cache hit rate"), rate * 100.0))
                    .into(),
            );
        }
        if total.tokens == 0 {
            content.push(
                self.muted(t("No measured token records in this range."))
                    .into(),
            );
        }
        // Up to 40 calendar bins, each retaining its actual total. No samples
        // are discarded when a long span is selected.
        if !analysis.days.is_empty() {
            let bins = &summary.bins;
            let max = bins.iter().copied().max().unwrap_or(1).max(1) as f64;
            content.push(
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(4.0)
                    .height(76.0)
                    .children((View::keyed_fragment(bins.iter().enumerate().map(
                        |(i, value)| {
                            (
                                i,
                                Border::new()
                                    .width(8.0)
                                    .height((*value as f64 / max * 76.0).max(0.0))
                                    .vertical_alignment(VerticalAlignment::Bottom)
                                    .corner_radius(CornerRadius::uniform(2.0))
                                    .background(Color::argb(255, 0, 120, 212)),
                            )
                        },
                    )),))
                    .into(),
            );
            content.push(
                self.muted(&format!(
                    "{} — {}",
                    analysis.days.first().unwrap().date,
                    analysis.days.last().unwrap().date
                ))
                .into(),
            );
        }
        let hours = &summary.hours;
        let peak = hours.iter().copied().max().unwrap_or(1).max(1) as f64;
        content.push(
            TextBlock::new()
                .text(t("Tokens by local hour (recorded time buckets only)"))
                .into(),
        );
        content.push(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(4.0)
                .height(90.0)
                .children((View::keyed_fragment(hours.iter().enumerate().map(
                    |(hour, tokens)| {
                        (
                            hour,
                            StackPanel::new()
                                .vertical_alignment(VerticalAlignment::Bottom)
                                .width(24.0)
                                .children((
                                    Border::new()
                                        .height(*tokens as f64 / peak * 64.0)
                                        .background(Color::argb(255, 0, 120, 212)),
                                    TextBlock::new().text(hour.to_string()).font_size(10.0),
                                )),
                        )
                    },
                )),))
                .into(),
        );
        let columns = [
            "Name / date",
            "Input",
            "Cache write",
            "Cache read",
            "Output",
            "Total tokens",
            "API value",
        ];
        let table_row = |cells: Vec<View>| {
            Grid::new()
                .columns([
                    GridLength::Star(2.0),
                    GridLength::Star(1.0),
                    GridLength::Star(1.0),
                    GridLength::Star(1.0),
                    GridLength::Star(1.0),
                    GridLength::Star(1.0),
                    GridLength::Star(1.0),
                ])
                .column_spacing(12.0)
                .min_width(780.0)
                .children((View::keyed_fragment(
                    cells
                        .into_iter()
                        .enumerate()
                        .map(|(i, v)| (i, Border::new().grid_column(i as i32).content(v))),
                ),))
        };
        let mut table: Vec<View> = vec![table_row(
            columns
                .iter()
                .map(|label| {
                    TextBlock::new()
                        .text(t(label))
                        .font_weight(FontWeight::SEMI_BOLD)
                        .into()
                })
                .collect(),
        )
        .into()];
        let pages = analysis.rows.len().div_ceil(20).max(1);
        let page = self.spend_page.min(pages - 1);
        for entry in analysis.page(page, 20) {
            let values = entry.measures;
            let id = entry.id.clone();
            let name: View = if self.spend_group == Group::Days {
                TextBlock::new().text(&entry.title).into()
            } else {
                Button::new()
                    .on_click(context.message(Message::SpendDrill(id)))
                    .content(entry.title.clone())
            };
            let cost = if values.unpriced == values.tokens && values.tokens > 0 {
                t("Unpriced").to_string()
            } else {
                format!(
                    "≈ ${:.2}{}",
                    values.cost,
                    if values.unpriced > 0 { " *" } else { "" }
                )
            };
            table.push(
                table_row(vec![
                    name,
                    TextBlock::new().text(values.tally.input.to_string()).into(),
                    TextBlock::new()
                        .text(values.tally.cache_write.to_string())
                        .into(),
                    TextBlock::new()
                        .text(values.tally.cache_read.to_string())
                        .into(),
                    TextBlock::new()
                        .text(values.tally.output.to_string())
                        .into(),
                    TextBlock::new().text(values.tokens.to_string()).into(),
                    TextBlock::new().text(cost).into(),
                ])
                .into(),
            );
        }
        content.push(
            self.surface(
                ScrollViewer::new()
                    .horizontal_scroll_bar_visibility(ScrollBarVisibility::Auto)
                    .content(
                        StackPanel::new()
                            .spacing(10.0)
                            .children((View::keyed_fragment(table.into_iter().enumerate()),)),
                    )
                    .into(),
            ),
        );
        content.push(row((
            Button::new()
                .is_enabled(page > 0)
                .on_click(context.message(Message::SpendPage(false)))
                .content(t("Previous")),
            TextBlock::new().text(format!("{} / {}", page + 1, pages)),
            Button::new()
                .is_enabled(page + 1 < pages)
                .on_click(context.message(Message::SpendPage(true)))
                .content(t("Next")),
        )));
        content.push(
            self.muted(t("* API value excludes tokens without a published price."))
                .into(),
        );
        let coverage = snapshot
            .sources
            .iter()
            .map(|s| {
                StackPanel::new().spacing(4.0).children((
                    TextBlock::new()
                        .text(format!(
                            "{} · {}",
                            s.title,
                            t(if !s.present {
                                "Store not found"
                            } else if s.ledger.has_partial_records {
                                "Some local records couldn't be read"
                            } else if summary.coverage.get(&s.id).copied().unwrap_or(false) {
                                if s.location.contains("UsageImports") {
                                    "Recorded tokens; native and imported coverage varies"
                                } else {
                                    "Native token records"
                                }
                            } else if quotascope_core::additional_spend::CATALOG
                                .iter()
                                .any(|(id, _, _)| *id == s.id)
                                && !quotascope_core::additional_spend::native_supported(&s.id)
                            {
                                "Native format not yet supported; accepts explicit local imports"
                            } else {
                                "No token counters found"
                            })
                        ))
                        .font_weight(FontWeight::SEMI_BOLD),
                    self.muted(&s.location),
                ))
            })
            .collect::<Vec<_>>();
        content.push(
            self.surface(
                StackPanel::new()
                    .spacing(12.0)
                    .children((
                        section("Source coverage"),
                        StackPanel::new()
                            .spacing(14.0)
                            .children((View::keyed_fragment(coverage.into_iter().enumerate()),)),
                    ))
                    .into(),
            ),
        );
        StackPanel::new()
            .spacing(18.0)
            .children((View::keyed_fragment(content.into_iter().enumerate()),))
    }

    fn credential_field(
        &self,
        index: usize,
        value: String,
        cookie: bool,
        context: &ViewContext<Self>,
    ) -> View {
        StackPanel::new().spacing(6.0).children((
            PasswordBox::new()
                .password(value)
                .password_reveal_mode(if self.revealed_keys.contains(&index) {
                    PasswordRevealMode::Visible
                } else {
                    PasswordRevealMode::Hidden
                })
                .placeholder_text(quotascope_core::localization::t(if cookie {
                    "Paste the Cookie header"
                } else {
                    "Paste the API key"
                }))
                .on_password_changed(context.callback(move |text| Message::KeyEdit(index, text))),
            self.aligned(
                "Show credential",
                ToggleSwitch::new()
                    .is_on(self.revealed_keys.contains(&index))
                    .on_toggled(context.callback(move |on| Message::RevealKey(index, on)))
                    .into(),
            ),
        ))
    }

    fn balance_controls(
        &self,
        index: usize,
        provider: Provider,
        context: &ViewContext<Self>,
    ) -> View {
        let (basis, saved_budget) = quotascope_core::balance_ring::basis_for(
            &quotascope_core::model::AccountKey::primary(provider),
        );
        let raw = provider.raw();
        let budget = self
            .budgets
            .get(raw)
            .cloned()
            .unwrap_or_else(|| saved_budget.map(|n| n.to_string()).unwrap_or_default());
        let floor = self.floors.get(raw).cloned().unwrap_or_else(|| {
            quotascope_core::settings::with(|s| {
                s.low_balance_alerts
                    .get(raw)
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            })
        });
        StackPanel::new().spacing(6.0).children((
            TextBlock::new().text(quotascope_core::localization::t("Ring measures")),
            ComboBox::new().selected_index(match basis { quotascope_core::balance_ring::Basis::SinceTopUp => 0, quotascope_core::balance_ring::Basis::BalanceOnly => 1, quotascope_core::balance_ring::Basis::Budget => 2 })
                .on_selection_changed(context.callback(move |value| Message::BalanceBasis(index, value)))
                .items_source(["Since top-up", "Balance only", "My budget"].map(quotascope_core::localization::t)),
            if basis == quotascope_core::balance_ring::Basis::Budget {
                TextBox::new().text(budget).placeholder_text(quotascope_core::localization::t("Budget in the balance's currency"))
                    .on_text_changed(context.callback(move |text| Message::BudgetEdit(index, text))).into()
            } else { View::empty() },
            TextBlock::new().text(quotascope_core::localization::t("Warn below")),
            TextBox::new().text(floor).placeholder_text(quotascope_core::localization::t("Amount in the balance's currency; blank disables"))
                .on_text_changed(context.callback(move |text| Message::FloorEdit(index, text))),
            Button::new().on_click(context.message(Message::SaveBalance(index)))
                .content(quotascope_core::localization::t("Save balance settings")),
            self.muted(quotascope_core::localization::t("Applies when the provider reports a balance without its own limits. Low-balance warnings also require notifications to be enabled.")),
        ))
    }
    fn spawn_watcher(shared: &Arc<Shared>, context: &ComponentContext<Self>) -> ComponentTask {
        let watcher = shared.clone();
        let seen = watcher.generation();
        context.spawn_background(move |cancel| {
            let started = std::time::Instant::now();
            // Lifecycle requests must wake an idle window even when no
            // account statuses change. Cancelled tasks must also terminate.
            loop {
                std::thread::sleep(Duration::from_millis(250));
                if cancel.is_cancelled()
                    || !watcher.window_ready.load(Ordering::SeqCst)
                    || watcher.show_requested.load(Ordering::SeqCst)
                    || watcher.shutdown_requested.load(Ordering::SeqCst)
                    || watcher.generation() != seen
                    || (started.elapsed() >= quotascope_core::service_status::CHECK_INTERVAL
                        && watcher.alive.load(Ordering::SeqCst))
                    || (started.elapsed() >= Duration::from_secs(1)
                        && watcher.alive.load(Ordering::SeqCst)
                        && quotascope_core::settings::with(|s| s.animated_bots))
                {
                    return Message::Tick;
                }
            }
        })
    }

    /// Muted text: theme-aware, because WinUI follows the system theme.
    fn muted(&self, text: &str) -> TextBlock {
        let gray = if self.dark { 255 } else { 0 };
        TextBlock::new()
            .text(text)
            .text_wrapping(TextWrapping::Wrap)
            .font_size(13.0)
            .foreground(Brush::Solid(Color::argb(160, gray, gray, gray)))
    }

    fn apply_toggle(key: ToggleKey, value: bool) {
        quotascope_core::settings::mutate(|s| match key {
            ToggleKey::SidePct => s.side_rail_shows_percentages = value,
            ToggleKey::Remaining => s.shows_remaining = value,
            ToggleKey::Clock => s.shows_window_clock = value,
            ToggleKey::SecondRing => s.shows_second_ring = value,
            ToggleKey::Forecast => s.shows_forecast = value,
            ToggleKey::AutoCollapse => s.auto_collapse = value,
            ToggleKey::FollowDisplay => s.follows_active_display = value,
            ToggleKey::HideTrayIcon => s.hides_tray_icon = value,
            ToggleKey::TokenSpend => s.reads_token_spend = value,
            ToggleKey::BackgroundSpend => s.background_token_spend = value,
            ToggleKey::GlobalShortcuts => s.global_shortcuts = value,
            ToggleKey::AnimatedBots => s.animated_bots = value,
            ToggleKey::CodexResetCredits => s.shows_codex_reset_credits = value,
            ToggleKey::Alerts => s.wants_alerts = value,
            ToggleKey::AlertReset => s.alerts_on_reset = value,
            ToggleKey::AlertFailure => s.alerts_on_failure = value,
            ToggleKey::AlertOutage => s.alerts_on_outage = value,
            ToggleKey::CheckUpdates => s.checks_for_updates = value,
            ToggleKey::Startup => {}
        });
        if key == ToggleKey::Startup {
            crate::autostart::set_enabled(value);
        }
        if !value && matches!(key, ToggleKey::TokenSpend | ToggleKey::BackgroundSpend) {
            quotascope_core::spend_warmer::clear();
        }
    }

    fn apply_choice(key: ChoiceKey, selected: usize) {
        quotascope_core::settings::mutate(|s| match key {
            ChoiceKey::ProxyMode => {
                s.proxy_mode = match selected {
                    1 => "disabled",
                    2 => "manual",
                    _ => "system",
                }
                .into()
            }
            ChoiceKey::Language => {
                s.language = match selected {
                    1 => "en",
                    2 => "zh",
                    3 => "zh-Hant",
                    4 => "ja",
                    5 => "ko",
                    _ => "auto",
                }
                .into();
                match selected {
                    1 => quotascope_core::localization::set_language(
                        quotascope_core::localization::Language::English,
                    ),
                    2 => quotascope_core::localization::set_language(
                        quotascope_core::localization::Language::Chinese,
                    ),
                    3 => quotascope_core::localization::set_language(
                        quotascope_core::localization::Language::TraditionalChinese,
                    ),
                    4 => quotascope_core::localization::set_language(
                        quotascope_core::localization::Language::Japanese,
                    ),
                    5 => quotascope_core::localization::set_language(
                        quotascope_core::localization::Language::Korean,
                    ),
                    _ => quotascope_core::localization::detect_from_system(),
                }
            }
            ChoiceKey::Dock => match selected {
                0 => {
                    s.floating = false;
                    s.dock_side = "right".into();
                }
                1 => {
                    s.floating = false;
                    s.dock_side = "left".into();
                }
                2 => {
                    s.floating = false;
                    s.dock_side = "top".into();
                }
                3 => {
                    s.floating = false;
                    s.dock_side = "bottom".into();
                }
                4 => {
                    s.floating = true;
                    s.dock_side = "right".into();
                }
                5 => {
                    s.floating = true;
                    s.dock_side = "top".into();
                }
                _ => {}
            },
            ChoiceKey::PanelSize => {
                s.panel_size = match selected {
                    0 => "small".into(),
                    2 => "large".into(),
                    _ => "standard".into(),
                };
            }
            ChoiceKey::RailSpacing => {
                s.rail_spacing = match selected {
                    0 => "compact".into(),
                    2 => "roomy".into(),
                    _ => "standard".into(),
                };
            }
            ChoiceKey::ClockDirection => {
                s.window_clock_direction = match selected {
                    1 => "remaining".into(),
                    _ => "elapsed".into(),
                };
            }
            ChoiceKey::WarningThreshold => {
                s.warning_threshold = match selected {
                    0 => 60,
                    1 => 70,
                    3 => 80,
                    4 => 85,
                    5 => 90,
                    _ => 75,
                };
            }
            ChoiceKey::Interval => {
                s.refresh_interval = match selected {
                    1 => 30,
                    2 => 60,
                    3 => 120,
                    4 => 300,
                    5 => 600,
                    6 => 1800,
                    _ => 0,
                };
            }
            ChoiceKey::AlertThreshold => {
                s.alert_threshold = match selected {
                    1 => 80,
                    2 => 90,
                    3 => 95,
                    _ => 75,
                };
            }
            ChoiceKey::StatisticsCache => {
                if let Some(value) = quotascope_core::statistics_cache::LIMITS_MB.get(selected) {
                    s.statistics_cache_limit_mb = *value;
                }
            }
        });
    }

    fn surface(&self, content: View) -> View {
        let ink = if self.dark { 255 } else { 0 };
        Border::new()
            .background(Color::argb(if self.dark { 9 } else { 180 }, 255, 255, 255))
            .border_brush(Color::argb(18, ink, ink, ink))
            .border_thickness(Thickness::uniform(1.0))
            .corner_radius(CornerRadius::uniform(10.0))
            .padding(Thickness::uniform(18.0))
            .content(content)
            .into()
    }

    fn aligned(&self, label: &str, control: View) -> View {
        Grid::new()
            .columns([GridLength::Star(1.0), GridLength::Auto])
            .column_spacing(20.0)
            .children((
                TextBlock::new()
                    .text(quotascope_core::localization::t(label))
                    .vertical_alignment(VerticalAlignment::Center),
                Border::new()
                    .grid_column(1)
                    .min_width(150.0)
                    .content(control),
            ))
            .into()
    }

    fn toggle(&self, key: ToggleKey, label: &str, on: bool, context: &ViewContext<Self>) -> View {
        self.aligned(
            label,
            ToggleSwitch::new()
                .automation_name(quotascope_core::localization::t(label))
                .is_on(on)
                .on_toggled(context.callback(move |on| Message::Toggle(key, on)))
                .into(),
        )
    }

    fn choice(
        &self,
        key: ChoiceKey,
        label: &str,
        options: &[&str],
        selected: usize,
        context: &ViewContext<Self>,
    ) -> View {
        self.aligned(
            label,
            ComboBox::new()
                .min_width(150.0)
                .items_source(options.iter().map(|o| quotascope_core::localization::t(o)))
                .selected_index(selected)
                .on_selection_changed(context.callback(move |index| Message::Choice(key, index)))
                .into(),
        )
    }

    fn general_view(&self, context: &ViewContext<Self>) -> View {
        let s = quotascope_core::settings::with(|s| s.clone());
        let dock_selected = if s.floating {
            if s.dock_side == "top" || s.dock_side == "bottom" {
                5
            } else {
                4
            }
        } else {
            match s.dock_side.as_str() {
                "left" => 1,
                "top" => 2,
                "bottom" => 3,
                _ => 0,
            }
        };
        let size_selected = match s.panel_size.as_str() {
            "small" => 0,
            "large" => 2,
            _ => 1,
        };
        let spacing_selected = match s.rail_spacing.as_str() {
            "compact" => 0,
            "roomy" => 2,
            _ => 1,
        };
        let interval_selected = match s.refresh_interval {
            30 => 1,
            60 => 2,
            120 => 3,
            300 => 4,
            600 => 5,
            1800 => 6,
            _ => 0,
        };
        let clock_selected = match s.window_clock_direction.as_str() {
            "remaining" => 1,
            _ => 0,
        };
        let warning_selected = match s.warning_threshold {
            60 => 0,
            70 => 1,
            80 => 3,
            85 => 4,
            90 => 5,
            _ => 2,
        };
        StackPanel::new()
            .spacing(10.0)
            .children((
                heading("General"),
                self.surface(StackPanel::new()
                    .spacing(14.0)
                    .children((
                        section("Panel"),
                        self.choice(
                            ChoiceKey::Dock,
                            "Dock to",
                            &["Right", "Left", "Top", "Bottom", "Free vertical", "Free horizontal"],
                            dock_selected,
                            context,
                        ),
                        StackPanel::new().spacing(6.0).children((
                            self.choice(ChoiceKey::ProxyMode,"Proxy mode",&["System proxy","No proxy","Manual proxy"],match s.proxy_mode.as_str(){"disabled"=>1,"manual"=>2,_=>0},context),
                            self.aligned("HTTP proxy (empty uses system settings)",TextBox::new().text(self.addresses.get("__proxy").cloned().unwrap_or_else(||s.proxy_url.clone())).placeholder_text("http://127.0.0.1:7890").on_text_changed(context.callback(Message::ProxyEdit)).into()),
                            Button::new().on_click(context.message(Message::SaveProxy)).content(quotascope_core::localization::t("Save proxy and refresh")),
                            self.toggle(ToggleKey::GlobalShortcuts,"Global shortcuts: Ctrl+Shift+F10 panel, F11 dashboard, F12 refresh",s.global_shortcuts,context),
                            self.toggle(ToggleKey::AnimatedBots,"Animated dashboard buddy",s.animated_bots,context),
                            self.choice(ChoiceKey::Language,"Language",&["Automatic","English","简体中文","繁體中文","日本語","한국어"],match s.language.as_str(){"en"=>1,"zh"=>2,"zh-Hant"=>3,"ja"=>4,"ko"=>5,_=>0},context),
                        )),
                        self.choice(
                            ChoiceKey::PanelSize,
                            "Panel size",
                            &["Small", "Standard", "Large"],
                            size_selected,
                            context,
                        ),
                        self.choice(
                            ChoiceKey::RailSpacing,
                            "Ring spacing",
                            &["Tight", "Standard", "Loose"],
                            spacing_selected,
                            context,
                        ),
                        self.toggle(
                            ToggleKey::SidePct,
                            "Show percent labels",
                            s.side_rail_shows_percentages,
                            context,
                        ),
                        self.toggle(
                            ToggleKey::Remaining,
                            "Show what's left",
                            s.shows_remaining,
                            context,
                        ),
                        self.toggle(
                            ToggleKey::Clock,
                            "Show window clock",
                            s.shows_window_clock,
                            context,
                        ),
                        self.choice(
                            ChoiceKey::ClockDirection,
                            "Clock shows",
                            &["Time gone", "Time left"],
                            clock_selected,
                            context,
                        ),
                        self.toggle(
                            ToggleKey::SecondRing,
                            "Show second ring",
                            s.shows_second_ring,
                            context,
                        ),
                        self.toggle(
                            ToggleKey::Forecast,
                            "Show forecast",
                            s.shows_forecast,
                            context,
                        ),
                        self.choice(
                            ChoiceKey::WarningThreshold,
                            "Turn red past",
                            &["60%", "70%", "75%", "80%", "85%", "90%"],
                            warning_selected,
                            context,
                        ),
                        self.toggle(
                            ToggleKey::AutoCollapse,
                            "Auto-collapse when idle",
                            s.auto_collapse,
                            context,
                        ),
                        self.toggle(
                            ToggleKey::FollowDisplay,
                            "Follow the active display",
                            s.follows_active_display,
                            context,
                        ),
                        self.toggle(
                            ToggleKey::HideTrayIcon,
                            "Hide the tray icon",
                            s.hides_tray_icon,
                            context,
                        ),
                        self.muted(&quotascope_core::localization::t(
                            "The panel stays put and comes back at every launch; run QuotaScope again to reach Settings.",
                        )),
                    )).into()),
                self.surface(StackPanel::new().spacing(14.0).children((section("Refresh"),
                self.choice(
                    ChoiceKey::Interval,
                    "Refresh interval",
                    &["Automatic", "30s", "1min", "2min", "5min", "10min", "30min"],
                    interval_selected,
                    context,
                ))).into()),
                self.surface(StackPanel::new().spacing(14.0).children((section("Windows"),
                self.toggle(
                    ToggleKey::Startup,
                    "Launch at startup",
                    crate::autostart::is_enabled(),
                    context,
                ))).into()),
                self.surface(StackPanel::new().spacing(14.0).children((section("Token spend"),
                self.toggle(
                    ToggleKey::CodexResetCredits,
                    "Show Codex reset credits",
                    s.shows_codex_reset_credits,
                    context,
                ),
                self.toggle(
                    ToggleKey::TokenSpend,
                    "Read token spend",
                    s.reads_token_spend,
                    context,
                ),
                self.muted(&quotascope_core::localization::t(
                    "Read this machine's Claude Code and Codex transcripts and Antigravity conversation databases, then price known token counts at published API rates. These records never leave this machine.",
                )))).into()),
            ))
            .into()
    }

    fn provider_row(&self, index: usize, provider: Provider, context: &ViewContext<Self>) -> View {
        let raw = provider.raw();
        let enabled = quotascope_core::settings::with(|s| s.enabled_accounts.contains(raw));

        if !provider.is_ported_to_windows() {
            return self.surface(
                StackPanel::new()
                    .spacing(6.0)
                    .children((
                        TextBlock::new()
                            .text(provider.display_name())
                            .font_size(15.0)
                            .font_weight(FontWeight::SEMI_BOLD),
                        self.muted(&quotascope_core::localization::t(
                            provider
                                .windows_gap()
                                .unwrap_or("Not available on Windows."),
                        )),
                    ))
                    .into(),
            );
        }

        let status_line = self.status.get(raw).cloned();
        let key_draft = self.keys.get(raw).cloned();

        let expanded = self.expanded_accounts.contains(&index);
        let summary = status_line.as_deref().unwrap_or(if enabled {
            quotascope_core::localization::t("Waiting for usage")
        } else {
            quotascope_core::localization::t("Disabled")
        });
        let header = Grid::new()
            .columns([GridLength::Star(1.0), GridLength::Auto, GridLength::Auto])
            .column_spacing(18.0)
            .children((
                StackPanel::new()
                    .spacing(4.0)
                    .vertical_alignment(VerticalAlignment::Center)
                    .children((
                        TextBlock::new()
                            .text(provider.display_name())
                            .font_size(15.0)
                            .font_weight(FontWeight::SEMI_BOLD),
                        self.muted(summary),
                    )),
                ToggleSwitch::new()
                    .grid_column(1)
                    .min_width(100.0)
                    .vertical_alignment(VerticalAlignment::Center)
                    .is_on(enabled)
                    .on_toggled(context.callback(move |on| Message::ToggleEnabled(index, on))),
                Button::new()
                    .grid_column(2)
                    .min_width(72.0)
                    .vertical_alignment(VerticalAlignment::Center)
                    .on_click(context.message(Message::ExpandAccount(index)))
                    .content(quotascope_core::localization::t(if expanded {
                        "Collapse"
                    } else {
                        "Configure"
                    })),
            ));
        if !expanded {
            return self.surface(header.into());
        }

        let body: View = match provider {
            Provider::ClaudeCode => StackPanel::new().spacing(6.0).children((
                self.muted(quotascope_core::localization::t("Reads the login this tool already saved on this PC.")),
                row((
                    Button::new().on_click(context.message(Message::ImportClaudeSession)).content(quotascope_core::localization::t("Import Claude Desktop / browser fallback")),
                    Button::new().on_click(context.message(Message::ForgetClaudeSession)).content(quotascope_core::localization::t("Forget fallback session")),
                )),
                self.muted(quotascope_core::localization::t("Configure quotascope --statusline in Claude to use reported rate limits as a 15-minute cached fallback.")),
                self.muted(quotascope_core::localization::t("For an additional Claude account, paste exported .credentials.json. Local usage belongs to the primary account only.")),
                self.credential_field(index, key_draft.clone().unwrap_or_default(), true, context),
            )).into(),
            Provider::Codex | Provider::Grok => self
                .muted(&quotascope_core::localization::t(
                    "Reads the login this tool already saved on this PC.",
                ))
                .into(),
            Provider::Kiro => self.muted(quotascope_core::localization::t(
                "Uses kiro-cli's existing login. Sign in with kiro-cli login first, then enable Kiro and refresh. No credential needs to be pasted.",
            )).into(),
            Provider::Devin | Provider::GrokBot | Provider::AlibabaTokenPlan | Provider::Gemini | Provider::JetBrainsAi | Provider::NousPortal => self.muted(quotascope_core::localization::t(match provider {
                Provider::Devin => "Reads the plan Devin saved on this PC. Open Devin and sign in first.",
                Provider::GrokBot => "Uses Cursor's existing login to read the Grok Bot allowance.",
                Provider::AlibabaTokenPlan => "Uses Bailian CLI (bl)'s existing login, for international and mainland plans.",
                Provider::Gemini => "Uses Gemini CLI's Google login; API key and Vertex AI modes do not report this allowance.",
                Provider::JetBrainsAi => "Reads the quota JetBrains AI saved on this PC. Open the IDE to update it.",
                _ => "Uses the Nous Portal login Hermes saved. Run Hermes to renew an expired login.",
            })).into(),
            Provider::Antigravity => self
                .muted(&quotascope_core::localization::t(
                    "Reads the language server Antigravity runs while it is open — figures exist only while it is running.",
                ))
                .into(),
            Provider::Cursor => self
                .muted(&quotascope_core::localization::t(
                    "Reads the login Cursor already saved in its own database.",
                ))
                .into(),
            Provider::Copilot => {
                let signed_in = quotascope_core::secrets::key_for("copilot").is_some();
                StackPanel::new()
                    .spacing(6.0)
                    .children((
                        Button::new()
                            .on_click(context.message(Message::CopilotAuth))
                            .content(quotascope_core::localization::t(if signed_in {
                                "Sign out"
                            } else {
                                "Sign in with GitHub"
                            })),
                        self.muted(&quotascope_core::localization::t(
                            "Device-code sign-in. Requests read:user only — never your repositories. The consent page names the editor whose client it borrows; this is not an official integration.",
                        )),
                    ))
                    .into()
            }
            Provider::Windsurf => {
                StackPanel::new().spacing(6.0).children((
                    self.muted(quotascope_core::localization::t("Import the Windsurf website's localStorage session, or paste its four-field JSON bundle.")),
                    self.credential_field(index,key_draft.or_else(|| quotascope_core::secrets::key_for(raw)).unwrap_or_default(),true,context),
                    row((
                        Button::new().on_click(context.message(Message::ImportStorage(index))).content(quotascope_core::localization::t("Import from browser")),
                        Button::new().on_click(context.message(Message::Save(index))).content(quotascope_core::localization::t("Save")),
                        Button::new().on_click(context.message(Message::Refresh(index))).content(quotascope_core::localization::t("Refresh")),
                    )),
                )).into()
            }
            Provider::OpenCodeGo => {
                StackPanel::new().spacing(6.0).children((
                    TextBlock::new().text(quotascope_core::localization::t("API key")),
                    self.credential_field(index, key_draft.or_else(|| quotascope_core::secrets::key_for(raw)).unwrap_or_default(), false, context),
                    row((
                        Button::new().on_click(context.message(Message::Save(index))).content(quotascope_core::localization::t("Save")),
                        Button::new().on_click(context.message(Message::Refresh(index))).content(quotascope_core::localization::t("Refresh")),
                    )),
                    self.muted(quotascope_core::localization::t("Import the OpenCode console session to read Go quotas and actual request bills across machines. The API key remains separate.")),
                    row((
                        Button::new().on_click(context.message(Message::ImportOpenCodeConsole)).content(quotascope_core::localization::t("Import from browser")),
                        Button::new().on_click(context.message(Message::ForgetOpenCodeConsole)).content(quotascope_core::localization::t("Forget console session")),
                    )),
                )).into()
            }
            Provider::DeepSeek => {
                let (running, message) = {
                    let state = self.shared.snapshot.lock().unwrap();
                    let operation = state.console_operations.get(raw);
                    (operation.is_some_and(|op| op.running), operation.and_then(|op| op.message.clone()))
                };
                StackPanel::new().spacing(6.0).children((
                    TextBlock::new().text(quotascope_core::localization::t("API key")),
                    self.credential_field(index, key_draft.or_else(|| quotascope_core::secrets::key_for(raw)).unwrap_or_default(), false, context),
                    row((
                        Button::new().on_click(context.message(Message::Save(index))).content(quotascope_core::localization::t("Save")),
                        Button::new().on_click(context.message(Message::Refresh(index))).content(quotascope_core::localization::t("Refresh")),
                    )),
                    self.deepseek_console_controls(&quotascope_core::model::AccountKey::primary(provider), running, message.as_deref(), context),
                ))
            }
            _ if provider.needs_server_address() => {
                let value = self
                    .addresses
                    .get(raw)
                    .cloned()
                    .or_else(|| quotascope_core::settings::with(|s| s.server_addresses.get(raw).cloned()))
                    .unwrap_or_default();
                StackPanel::new()
                    .spacing(6.0)
                    .children((
                        TextBlock::new().text(quotascope_core::localization::t("API key")),
                        self.credential_field(index, key_draft.clone().or_else(|| quotascope_core::secrets::key_for(raw)).unwrap_or_default(), false, context),
                        TextBlock::new().text(quotascope_core::localization::t("Gateway address")),
                        TextBox::new()
                            .text(value)
                            .placeholder_text("https://gateway.example")
                            .on_text_changed(context.callback(move |text| Message::AddressEdit(index, text))),
                        row((
                            Button::new()
                                .on_click(context.message(Message::Save(index)))
                                .content(quotascope_core::localization::t("Save")),
                            Button::new()
                                .on_click(context.message(Message::Refresh(index)))
                                .content(quotascope_core::localization::t("Refresh")),
                        )),
                    ))
                    .into()
            }
            _ if quotascope_core::providers::session_spec(provider).is_some() => {
                let stored = key_draft
                    .or_else(|| quotascope_core::secrets::key_for(raw))
                    .unwrap_or_default();
                StackPanel::new()
                    .spacing(6.0)
                    .children((
                        self.muted(&quotascope_core::localization::t(
                            "Uses a browser session. Importing reads the site's cookies from your browsers — the one you open links with first. A pasted Cookie header works too.",
                        )),
                        row((
                            Button::new()
                                .on_click(context.message(Message::ImportSession(index)))
                                .content(quotascope_core::localization::t("Import from browser")),
                            Button::new()
                                .on_click(context.message(Message::Refresh(index)))
                                .content(quotascope_core::localization::t("Refresh")),
                        )),
                        TextBlock::new()
                            .text(quotascope_core::localization::t("Or paste a Cookie header")),
                        self.credential_field(index, stored, true, context),
                        Button::new()
                            .on_click(context.message(Message::Save(index)))
                            .content(quotascope_core::localization::t("Save")),
                    ))
                    .into()
            }
            _ => {
                let stored = key_draft
                    .or_else(|| quotascope_core::secrets::key_for(raw))
                    .unwrap_or_default();
                StackPanel::new()
                    .spacing(6.0)
                    .children((
                        TextBlock::new().text(quotascope_core::localization::t("API key")),
                        self.credential_field(index, stored, false, context),
                        row((
                            Button::new()
                                .on_click(context.message(Message::Save(index)))
                                .content(quotascope_core::localization::t("Save")),
                            Button::new()
                                .on_click(context.message(Message::Refresh(index)))
                                .content(quotascope_core::localization::t("Refresh")),
                        )),
                    ))
                    .into()
            }
        };

        self.surface(
            StackPanel::new()
                .spacing(16.0)
                .children((
                    header,
                    Border::new()
                        .height(1.0)
                        .background(Color::argb(20, 128, 128, 128)),
                    body,
                    if quotascope_core::accounts::supports(provider) {
                        StackPanel::new().spacing(6.0).children((
                            self.muted(quotascope_core::localization::t("Paste a different account credential above, then add it. The primary credential stays unchanged.")),
                            Button::new().on_click(context.message(Message::AddAccount(index))).content(quotascope_core::localization::t("Add account")),
                        )).into()
                    } else { View::empty() },
                    if matches!(
                        provider,
                        Provider::ClaudeCode
                            | Provider::Codex
                            | Provider::Grok
                            | Provider::Antigravity
                            | Provider::Cursor
                            | Provider::Kiro
                            | Provider::Devin
                            | Provider::GrokBot
                            | Provider::AlibabaTokenPlan
                            | Provider::Gemini
                            | Provider::JetBrainsAi
                            | Provider::NousPortal
                    ) {
                        Button::new()
                            .on_click(context.message(Message::Refresh(index)))
                            .content(quotascope_core::localization::t("Refresh"))
                            .into()
                    } else {
                        View::empty()
                    },
                    self.aligned(
                        "Detailed card",
                        ToggleSwitch::new()
                            .is_on(quotascope_core::settings::with(|s| {
                                s.detailed_cards.contains(raw)
                            }))
                            .on_toggled(context.callback(move |on| Message::Detailed(index, on)))
                            .into(),
                    ),
                    if provider.reports_spendable_balance() {
                        self.balance_controls(index, provider, context)
                    } else {
                        View::empty()
                    },
                    self.account_details_view(index, provider, context),
                    self.service_status_view(provider, context),
                )),
        )
    }

    fn service_status_view(&self, provider: Provider, context: &ViewContext<Self>) -> View {
        use quotascope_core::localization::{t, t_fmt};
        use quotascope_core::service_status::{Page as StatusPage, State};
        let Some(page) = StatusPage::for_provider(provider) else {
            return View::empty();
        };
        let (reading, running, failed) = {
            let state = self.shared.snapshot.lock().unwrap();
            let status = state.service_status.get(&page);
            (
                status.and_then(|s| s.reading.clone()),
                status.is_some_and(|s| s.running),
                status.is_some_and(|s| s.failed),
            )
        };
        let mut rows: Vec<(String, View)> = Vec::new();
        if let Some(reading) = &reading {
            for component in &reading.components {
                let colour = match component.state {
                    State::Operational => Color::rgb(46, 160, 97),
                    State::Maintenance => Color::rgb(91, 143, 213),
                    State::Degraded | State::PartialOutage => Color::rgb(200, 134, 35),
                    State::FullOutage => Color::rgb(220, 67, 76),
                    State::Unrecognised => Color::rgb(128, 128, 128),
                };
                rows.push((
                    component.id.clone(),
                    row((
                        TextBlock::new()
                            .text(&component.name)
                            .text_wrapping(TextWrapping::Wrap),
                        TextBlock::new()
                            .text(component.state.title())
                            .foreground(Brush::Solid(colour)),
                    )),
                ));
            }
        }
        let feedback = if running {
            t("Checking service status…")
        } else if failed {
            t("Couldn't read the status page. Any previous reading below is older; recovery is unconfirmed.")
        } else if reading.is_none() {
            t("No service status reading yet.")
        } else {
            ""
        };
        StackPanel::new().spacing(8.0).children((
            heading("Service status"),
            self.muted(&t_fmt("Published by {company}; separate from your account allowance.", &[page.company()])),
            self.muted(feedback),
            StackPanel::new().spacing(6.0).keyed_children(rows),
            self.muted(&reading.as_ref().map(|r| t_fmt("Status checked: {time}",
                &[&quotascope_core::timeutil::format_local_naive(r.checked_at)])).unwrap_or_default()),
            row((
                Button::new().is_enabled(!running).on_click(context.message(Message::ServiceStatusRefresh(page)))
                    .content(t("Refresh service status")),
                Button::new().on_click(context.message(Message::ServiceStatusOpen(page)))
                    .content(t("Open official status page")),
            )),
            self.muted(t("Checked every five minutes while this section is open. Enable service outage notifications to watch enabled providers in the background.")),
        ))
    }

    fn account_details_view(
        &self,
        index: usize,
        provider: Provider,
        context: &ViewContext<Self>,
    ) -> View {
        use quotascope_core::localization::{t, t_fmt};
        if !matches!(provider, Provider::ClaudeCode | Provider::Codex) {
            return View::empty();
        }
        if !quotascope_core::settings::with(|s| s.reads_token_spend) {
            return self
                .muted(t(
                    "Enable local reading to analyze records on this computer.",
                ))
                .into();
        }
        let (details, running) = {
            let state = self.shared.snapshot.lock().unwrap();
            (
                state.account_details.get(&provider).cloned(),
                state.account_detail_controls.contains_key(&provider),
            )
        };
        let mut rows: Vec<View> = vec![
            section("Usage by model (last 30 days)").into(),
            Button::new()
                .is_enabled(!running)
                .on_click(context.message(Message::RefreshAccountDetails(index)))
                .content(t("Refresh"))
                .into(),
        ];
        if let Some(details) = details {
            if details.partial {
                rows.push(self.muted(t("Counts may be incomplete.")).into());
            }
            if details.timings_partial {
                rows.push(self.muted(t("Some local timing records couldn't be read; speed covers readable replies only.")).into());
            }
            if details.models.is_empty() {
                rows.push(self.muted(t("No local records.")).into());
            }
            for model in &details.models {
                let hit = model
                    .cache_hit
                    .map(|v| format!("{:.1}%", v * 100.0))
                    .unwrap_or_else(|| "—".into());
                let speed = model
                    .timing
                    .speed()
                    .map(|v| format!("{v:.1}"))
                    .unwrap_or_else(|| "—".into());
                let first = model
                    .timing
                    .first_token()
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "—".into());
                let text = t_fmt("{model} · {share}% · {tokens} tokens · cache {hit}% · {speed} tokens/s · first token {first}s", &[&model.id, &format!("{:.1}", model.share * 100.0), &model.tokens.to_string(), &hit.trim_end_matches('%'), &speed, &first]);
                rows.push(
                    TextBlock::new()
                        .text(text)
                        .text_wrapping(TextWrapping::Wrap)
                        .into(),
                );
            }
            rows.push(self.muted(t("Speed covers the last 24 hours, includes request wait, and needs three measured replies.")).into());
            rows.push(section("Prompt cache sessions").into());
            let now = quotascope_core::timeutil::now_ms();
            let sessions = details.cache.alive(now);
            if sessions.is_empty() {
                rows.push(self.muted(t("No active prompt cache.")).into());
            }
            for session in sessions {
                let name = session
                    .title
                    .as_deref()
                    .or(session.project.as_deref())
                    .unwrap_or(t("Untitled conversation"));
                let minutes =
                    ((session.lapse.expires_at().saturating_sub(now) + 59_999) / 60_000).max(1);
                rows.push(
                    TextBlock::new()
                        .text(t_fmt(
                            "{session} · {minutes} min remaining",
                            &[name, &minutes.to_string()],
                        ))
                        .text_wrapping(TextWrapping::Wrap)
                        .into(),
                );
            }
            if provider == Provider::Codex {
                rows.push(self.muted(t("Documented cache eligibility is a minimum; routing can still cause a cache miss.")).into());
            }
        } else {
            rows.push(
                self.muted(t(if running {
                    "Reading local records…"
                } else {
                    "Refresh to read account details."
                }))
                .into(),
            );
        }
        if provider == Provider::Codex {
            rows.push(self.codex_signals_view(context));
        }
        let signal_index = (provider == Provider::Codex).then(|| rows.len() - 1);
        StackPanel::new()
            .spacing(8.0)
            .keyed_children(rows.into_iter().enumerate().map(move |(index, view)| {
                (
                    if Some(index) == signal_index {
                        "codex-signals".into()
                    } else {
                        index.to_string()
                    },
                    view,
                )
            }))
    }

    fn signal_query(days: Option<u32>) -> crate::codex_signal_state::Query {
        crate::codex_signal_state::Query {
            home: quotascope_core::home_dir(),
            days,
            today: chrono::Local::now().date_naive(),
        }
    }

    fn codex_signals_view(&self, context: &ViewContext<Self>) -> View {
        use quotascope_core::localization::{t, t_fmt};
        let days = self
            .shared
            .snapshot
            .lock()
            .unwrap()
            .codex_signals
            .query
            .as_ref()
            .map(|query| query.days)
            .unwrap_or(Some(30));
        self.shared
            .request_codex_signals(Self::signal_query(days), false);
        let (result, running, failed, cancelled) = {
            let state = self.shared.snapshot.lock().unwrap();
            let signals = &state.codex_signals;
            (
                signals.result.clone(),
                signals.dirty,
                signals.failed,
                signals.cancelled,
            )
        };
        let mut rows: Vec<View> = vec![
            section("Local anomaly clues").into(),
            self.muted(&t_fmt("Signal rule: {version}", &[quotascope_core::codex_signals::ALGORITHM])).into(),
            self.muted(t("Local request records show selected parameters, not the model the server actually ran. The lattice rule is an empirical clue, not proof.")).into(),
            self.aligned("Signal period", ComboBox::new().min_width(180.0)
                .items_source(["Last 30 calendar days", "Last 90 calendar days", "All local records"]
                    .into_iter().map(|key| t(key).to_string()).collect::<Vec<_>>())
                .selected_index(match days { Some(90) => 1, None => 2, _ => 0 })
                .on_selection_changed(context.callback(Message::SignalPeriod)).into()),
            row((
                Button::new().on_click(context.message(Message::SignalRefresh)).content(t("Refresh local clues")),
                Button::new().is_enabled(running).on_click(context.message(Message::SignalCancel)).content(t("Cancel local reading")),
            )),
        ];
        if running {
            rows.push(self.muted(t("Reading local clues…")).into());
        }
        if failed {
            rows.push(
                self.muted(t("Couldn't read local clues. Refresh to try again."))
                    .into(),
            );
        }
        if cancelled {
            rows.push(
                self.muted(t(
                    "Local reading cancelled; any previous complete snapshot is kept.",
                ))
                .into(),
            );
        }
        if let Some(summary) = result {
            let report = &summary.report;
            rows.push(self.muted(&t_fmt("{sessions} sessions · {judged} comparable settings · {unknown} without comparable settings", &[
                &report.sessions.to_string(), &report.judged_sessions.to_string(),
                &report.sessions.saturating_sub(report.judged_sessions).to_string()])).into());
            if report.partial {
                rows.push(
                    self.muted(t(
                        "Records are incomplete; counts below cover the readable subset.",
                    ))
                    .into(),
                );
            }
            rows.push(self.muted(t("Lattice denominator: positive reasoning replies with at least 516 tokens. Concentration requires 20 replies, 5 hits and a 5% share.")).into());
            if summary.models.is_empty() {
                rows.push(
                    self.muted(t("No measured reasoning replies in this period."))
                        .into(),
                );
            }
            for (model, counts) in &summary.models {
                let status = t(if report.partial {
                    "Incomplete records"
                } else if !counts.measurable() {
                    "Insufficient sample"
                } else if counts.concentrated() {
                    "Lattice concentration"
                } else {
                    "No concentration in these records"
                });
                let share = counts
                    .share()
                    .map(|value| format!("{:.1}", value * 100.0))
                    .unwrap_or_else(|| "—".into());
                rows.push(TextBlock::new().text(t_fmt(
                    "{model} · positive reasoning replies {responses} · lattice {hits}/{eligible} ({share}%) · {state}",
                    &[model, &counts.responses.to_string(), &counts.hits.to_string(), &counts.reached.to_string(), &share, status]))
                    .text_wrapping(TextWrapping::Wrap).into());
            }
            if summary.hidden_models > 0 {
                rows.push(
                    self.muted(&t_fmt(
                        "{count} more models are outside this view.",
                        &[&summary.hidden_models.to_string()],
                    ))
                    .into(),
                );
            }
            rows.push(section("Recorded parameter differences").into());
            rows.push(self.muted(t("Compared only with settings recorded before the turn started. Missing settings, old logs and unknown effort levels remain unjudged.")).into());
            if report.changes_count == 0 {
                rows.push(self.muted(t("No recorded differences in comparable turns; unjudged turns are not evidence of consistency.")).into());
            }
            for located in &report.changes {
                use quotascope_core::codex_signals::Kind;
                let detail = match &located.change.kind {
                    Kind::Model { asked, recorded } => t_fmt(
                        "Selected model {asked} → recorded model {recorded}",
                        &[asked, recorded],
                    ),
                    Kind::Effort { asked, recorded } => t_fmt(
                        "Selected effort {asked} → recorded effort {recorded}",
                        &[asked, recorded],
                    ),
                    Kind::Context { previous, recorded } => t_fmt(
                        "Recorded context window {previous} → {recorded}",
                        &[&previous.to_string(), &recorded.to_string()],
                    ),
                };
                let at = chrono::DateTime::from_timestamp_millis(located.change.at)
                    .map(|at| {
                        at.with_timezone(&chrono::Local)
                            .format("%m-%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                rows.push(
                    TextBlock::new()
                        .text(format!("{at} · {detail} · {}", located.session))
                        .text_wrapping(TextWrapping::Wrap)
                        .into(),
                );
            }
            if report.changes_count > report.changes.len() {
                rows.push(
                    self.muted(&t_fmt(
                        "Showing the latest {shown} of {total} recorded differences.",
                        &[
                            &report.changes.len().to_string(),
                            &report.changes_count.to_string(),
                        ],
                    ))
                    .into(),
                );
            }
        }
        StackPanel::new().spacing(8.0).keyed_children(
            rows.into_iter()
                .enumerate()
                .map(|(index, view)| (index.to_string(), view)),
        )
    }

    fn deepseek_console_controls(
        &self,
        account: &quotascope_core::model::AccountKey,
        running: bool,
        message: Option<&str>,
        context: &ViewContext<Self>,
    ) -> View {
        use quotascope_core::localization::t;
        let id = account.id();
        let detailed = quotascope_core::settings::with(|s| s.detailed_cards.contains(&id));
        StackPanel::new().spacing(6.0).children((
            self.muted(t("Import the DeepSeek console session to read 30 calendar days of account-wide tokens and actual bills. API keys stay separate.")),
            if account.is_primary() { View::empty() } else {
                self.muted(t("Sign in to the intended DeepSeek account in your browser before importing. Additional accounts never renew from another browser login.")).into()
            },
            row((
                Button::new().is_enabled(!running).on_click(context.message(Message::DeepSeekConsole(id.clone(), true))).content(t("Import from browser")),
                Button::new().is_enabled(!running).on_click(context.message(Message::DeepSeekConsole(id.clone(), false))).content(t("Forget console session")),
            )),
            self.muted(if running { t("Working…") } else { message.unwrap_or("") }),
            if account.is_primary() { View::empty() } else {
                self.aligned("Detailed cards", ToggleSwitch::new().is_on(detailed)
                    .on_toggled(context.callback(move |on| Message::DetailedAccount(id.clone(), on))).into())
            },
        ))
    }

    fn accounts_view(&self, context: &ViewContext<Self>) -> View {
        // Keyed by provider raw, so rows keep their identity as the list
        // reconciles. `keyed_children` finishes the builder, so the header
        // rows ride the same keyed list under reserved keys.
        let note = self.muted(&quotascope_core::localization::t(
            "Each provider reports its own figures by the route that product offers — QuotaScope holds no account of its own and sends nothing anywhere but to the provider you already use.",
        ));
        let mut rows: Vec<(&str, View)> = vec![
            ("__heading", heading("Accounts").into()),
            ("__note", note.into()),
            (
                "__search",
                TextBox::new()
                    .text(self.search.clone())
                    .placeholder_text(quotascope_core::localization::t("Search accounts"))
                    .on_text_changed(context.callback(Message::Search))
                    .into(),
            ),
        ];
        let query = self.search.trim().to_lowercase();
        let mut matches = 0;
        for (api, key, label) in [
            (false, "__subscriptions", "Subscriptions"),
            (true, "__api", "API accounts"),
        ] {
            let mut providers: Vec<_> = all_providers()
                .iter()
                .enumerate()
                .filter(|(_, p)| p.is_api_billing() == api)
                .filter(|(_, p)| {
                    p.display_name().to_lowercase().contains(&query)
                        || p.raw().to_lowercase().contains(&query)
                })
                .collect();
            providers.sort_by_key(|(_, p)| {
                (
                    !p.is_ported_to_windows(),
                    !quotascope_core::settings::with(|s| s.enabled_accounts.contains(p.raw())),
                )
            });
            if !providers.is_empty() {
                rows.push((key, section(label).into()));
            }
            matches += providers.len();
            for (index, provider) in providers {
                rows.push((provider.raw(), self.provider_row(index, *provider, context)));
            }
        }
        if matches == 0 {
            rows.push((
                "__empty",
                self.muted(quotascope_core::localization::t("No matching accounts."))
                    .into(),
            ));
        }
        rows.push(("__extensions", self.extensions_view(context)));
        StackPanel::new().spacing(14.0).keyed_children(rows)
    }

    /// The extensions folder's contents: what a scan found and why the
    /// folders it turned away were turned away. A scan reads manifests and
    /// runs nothing.
    fn extensions_view(&self, context: &ViewContext<Self>) -> View {
        let enabled = quotascope_core::settings::with(|s| s.enabled_accounts.clone());
        let mut rows: Vec<(&'static str, View)> = Vec::new();
        rows.push(("__heading", section("Extensions").into()));
        rows.push((
            "__note",
            self.muted(&quotascope_core::localization::t(
                "A program in the extensions folder reports one account's usage. It runs only while it is switched on here, and QuotaScope hands it no credential.",
            ))
            .into(),
        ));

        let query = self.search.trim().to_lowercase();
        for (index, extension) in self.extensions.iter().enumerate().filter(|(_, e)| {
            e.name.to_lowercase().contains(&query) || e.id.to_lowercase().contains(&query)
        }) {
            let is_on = enabled.contains(&extension.account().id());
            let key: &'static str = Box::leak(format!("__ext-{index}").into_boxed_str());
            rows.push((
                key,
                row((
                    TextBlock::new()
                        .text(extension.name.clone())
                        .font_size(15.0)
                        .font_weight(FontWeight::SEMI_BOLD),
                    ToggleSwitch::new().is_on(is_on).on_toggled(
                        context.callback(move |on| Message::ToggleExtension(index, on)),
                    ),
                ))
                .into(),
            ));
        }
        for (index, problem) in self.extension_problems.iter().enumerate() {
            let key: &'static str = Box::leak(format!("__problem-{index}").into_boxed_str());
            rows.push((
                key,
                self.muted(&format!(
                    "{} — {}",
                    problem.folder,
                    problem.reason.message()
                ))
                .into(),
            ));
        }
        if self.extensions.is_empty() && self.extension_problems.is_empty() {
            rows.push((
                "__empty",
                self.muted(&quotascope_core::localization::t("No extensions found."))
                    .into(),
            ));
        }

        rows.push((
            "__folder",
            row((
                Button::new()
                    .on_click(context.message(Message::RescanExtensions))
                    .content(quotascope_core::localization::t("Look again")),
                TextBlock::new()
                    .text(
                        quotascope_core::extension::folder()
                            .to_string_lossy()
                            .to_string(),
                    )
                    .font_size(12.0)
                    .is_text_selection_enabled(true),
            ))
            .into(),
        ));

        StackPanel::new().spacing(8.0).keyed_children(rows)
    }

    fn notifications_view(&self, context: &ViewContext<Self>) -> View {
        let s = quotascope_core::settings::with(|s| s.clone());
        let threshold_selected = match s.alert_threshold {
            80 => 1,
            90 => 2,
            95 => 3,
            _ => 0,
        };
        let follow_ups: View = if s.wants_alerts {
            StackPanel::new()
                .spacing(10.0)
                .children((
                    self.choice(
                        ChoiceKey::AlertThreshold,
                        "Warn me when a limit passes",
                        &["75%", "80%", "90%", "95%"],
                        threshold_selected,
                        context,
                    ),
                    self.toggle(
                        ToggleKey::AlertReset,
                        "when a warned window comes back",
                        s.alerts_on_reset,
                        context,
                    ),
                    self.toggle(
                        ToggleKey::AlertFailure,
                        "when checks keep failing",
                        s.alerts_on_failure,
                        context,
                    ),
                ))
                .into()
        } else {
            View::empty()
        };
        StackPanel::new()
            .spacing(10.0)
            .children((
                heading("Notifications"),
                self.muted(&quotascope_core::localization::t(
                    "All notifications are off until you turn them on.",
                )),
                self.toggle(
                    ToggleKey::Alerts,
                    "Enable notifications",
                    s.wants_alerts,
                    context,
                ),
                follow_ups,
                self.toggle(
                    ToggleKey::AlertOutage,
                    "when a provider service goes down or recovers",
                    s.alerts_on_outage,
                    context,
                ),
                self.muted(quotascope_core::localization::t("Service outage notifications use official public status pages and only watch enabled Codex, Claude Code and DeepSeek accounts. Quota alerts remain a separate switch.")),
            ))
    }

    fn maintenance_view(&self, context: &ViewContext<Self>) -> View {
        use quotascope_core::localization::t;
        let (cache, busy, scanning, result, path) = {
            let state = self.shared.snapshot.lock().unwrap();
            (
                state.cache.clone(),
                state.maintenance_running,
                state.spend_worker_running,
                state.maintenance_result.clone(),
                state.diagnostic_path.clone(),
            )
        };
        let limit = quotascope_core::settings::with(|s| {
            quotascope_core::statistics_cache::limit_mb(s.statistics_cache_limit_mb)
        });
        let selected = quotascope_core::statistics_cache::LIMITS_MB
            .iter()
            .position(|v| *v == limit)
            .unwrap_or(2);
        let size_text = cache
            .map(|cache| {
                t("Statistics cache: {size} MiB · {files} files")
                    .replace("{size}", &format!("{:.2}", cache.bytes as f64 / 1048576.0))
                    .replace("{files}", &cache.files.len().to_string())
            })
            .unwrap_or_else(|| t("Reading cache information…").into());
        let status = if busy {
            t("Working…").to_owned()
        } else {
            match result {
                Some(MaintenanceResult::Cleaned(cleanup)) => {
                    t("Removed {files} cache files · freed {size} MiB · {failed} failures")
                        .replace("{files}", &cleanup.removed_files.to_string())
                        .replace(
                            "{size}",
                            &format!("{:.2}", cleanup.freed_bytes as f64 / 1048576.0),
                        )
                        .replace("{failed}", &cleanup.failed_files.to_string())
                }
                Some(MaintenanceResult::Exported) => t("Diagnostics exported locally.").into(),
                Some(MaintenanceResult::Failed) => t(
                    "Could not complete this operation. Check folder permissions and try again.",
                )
                .into(),
                _ => String::new(),
            }
        };
        StackPanel::new().spacing(14.0).children((
            heading("Storage and diagnostics"),
            self.surface(StackPanel::new().spacing(10.0).children((
                section("Statistics cache"),
                TextBlock::new().text(size_text),
                self.aligned("Disk cache budget", ComboBox::new()
                    .is_enabled(!busy && !scanning)
                    .min_width(170.0)
                    .items_source(["No disk cache", "16 MiB", "64 MiB (default)", "256 MiB"].map(t))
                    .selected_index(selected)
                    .on_selection_changed(context.callback(|index| Message::Choice(ChoiceKey::StatisticsCache, index))).into()),
                self.muted(t("Oldest statistics caches are removed when over budget. Oversized files are not saved. Account settings, keys and original logs are kept.")),
                self.muted(t("After clearing, statistics are rebuilt on the next read. The price table is retained.")),
                if scanning { self.muted(t("Wait for the local scan to stop before clearing or changing the budget.")).into() } else { View::empty() },
                StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
                    Button::new().is_enabled(!busy).on_click(context.message(Message::Maintenance(MaintenanceOperation::Inspect))).content(t("Refresh cache information")),
                    Button::new().is_enabled(!busy && !scanning).on_click(context.message(Message::Maintenance(MaintenanceOperation::Clear))).content(t("Clear statistics cache")),
                )),
            ))),
            self.surface(StackPanel::new().spacing(10.0).children((
                section("Diagnostics"),
                self.muted(t("Export a local JSON report with version, process resources, cache sizes and scan status. Keys, account addresses and conversation contents are excluded. A new export replaces the previous report.")),
                StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
                    Button::new().is_enabled(!busy).on_click(context.message(Message::Maintenance(MaintenanceOperation::Export))).content(t("Export diagnostics")),
                    Button::new().is_enabled(path.is_some() && !busy).on_click(context.message(Message::OpenDiagnosticsFolder)).content(t("Open diagnostics folder")),
                )),
                TextBlock::new().text(path.map(|p| p.display().to_string()).unwrap_or_default()).is_text_selection_enabled(true).text_wrapping(TextWrapping::Wrap),
            ))),
            TextBlock::new().text(status).text_wrapping(TextWrapping::Wrap),
        ))
    }

    fn about_view(&self, context: &ViewContext<Self>) -> View {
        StackPanel::new()
            .spacing(10.0)
            .children((
                heading("QuotaScope"),
                self.muted(&quotascope_core::localization::t(
                    "A screen-edge monitor for your AI coding allowances.",
                )),
                TextBlock::new().text(format!(
                    "{} {} (Windows)",
                    quotascope_core::localization::t("Version"),
                    env!("CARGO_PKG_VERSION")
                )),
                TextBlock::new().text("HarmonyOS Sans · Copyright 2021 Huawei Device Co., Ltd."),
                self.updates_view(context),
                self.muted(&quotascope_core::localization::t(
                    "No QuotaScope servers, no QuotaScope account, no telemetry. Provider requests follow the Windows system proxy settings. Optional update checks contact GitHub.",
                )),
                Button::new()
                    .on_click(context.message(Message::OpenGitHub))
                    .content("github.com/Lyx721188/QuotaScope"),
            ))
            .into()
    }

    fn updates_view(&self, context: &ViewContext<Self>) -> View {
        use quotascope_core::localization::t;
        use quotascope_core::updates::CheckResult;
        let (automatic, skipped) = quotascope_core::settings::with(|s| {
            (s.checks_for_updates, s.skipped_update_version.clone())
        });
        let status = self.shared.snapshot.lock().unwrap().update.clone();
        let checking = matches!(status, UpdateStatus::Checking);
        let (text, actions): (String, View) = match status {
            UpdateStatus::Idle => (
                t("Updates are installed only when you choose.").into(),
                View::empty(),
            ),
            UpdateStatus::Checking => (t("Checking for updates…").into(), View::empty()),
            UpdateStatus::Done(CheckResult::UpToDate) => (
                t("You have the latest Windows release.").into(),
                View::empty(),
            ),
            UpdateStatus::Done(CheckResult::Failed) => (
                t("Could not check for updates. Try again later or open the releases page.").into(),
                Button::new()
                    .on_click(context.message(Message::OpenUpdate(
                        quotascope_core::updates::RELEASES_PAGE.into(),
                    )))
                    .content(t("Open releases page")),
            ),
            UpdateStatus::Done(CheckResult::Available(release)) => {
                let is_skipped = skipped.as_deref() == Some(&release.version);
                (
                    format!(
                        "{} {}{}",
                        t("Windows update available:"),
                        release.version,
                        if is_skipped {
                            t(" (reminders skipped)")
                        } else {
                            ""
                        }
                    ),
                    StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .spacing(8.0)
                        .children((
                            Button::new()
                                .on_click(context.message(Message::OpenUpdate(release.url)))
                                .content(t("Open update page")),
                            Button::new()
                                .is_enabled(!is_skipped)
                                .on_click(context.message(Message::SkipUpdate(release.version)))
                                .content(t("Skip this version")),
                        )),
                )
            }
        };
        self.surface(
            StackPanel::new().spacing(12.0).children((
                section("Updates"),
                self.toggle(
                    ToggleKey::CheckUpdates,
                    "Check for updates automatically",
                    automatic,
                    context,
                ),
                self.muted(t(
                    "Check once a day when enabled. Download and installation are your choice.",
                )),
                TextBlock::new()
                    .text(text)
                    .text_wrapping(TextWrapping::Wrap),
                Button::new()
                    .is_enabled(!checking)
                    .on_click(context.message(Message::CheckUpdates))
                    .content(t("Check for updates")),
                actions,
            )),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::positive_or_empty;

    fn analysis_request() -> quotascope_core::spend_analysis::Request {
        use quotascope_core::spend::{Group, Sort, Span};
        quotascope_core::spend_analysis::Request {
            snapshot: std::sync::Arc::new(Default::default()),
            query: quotascope_core::spend_analysis::Query {
                span: Span::Week,
                today: chrono::NaiveDate::from_ymd_opt(2026, 10, 4).unwrap(),
                source: None,
                model: None,
                group: Group::Agents,
                sort: Sort::Tokens,
                descending: true,
            },
        }
    }

    #[test]
    fn changing_filters_rejects_old_analysis_and_reuses_only_the_matching_cache() {
        let request = analysis_request();
        let mut state = super::SpendAnalysisState::default();
        state.select(request.clone());
        let old_generation = state.generation;
        let control = std::sync::Arc::new(quotascope_core::scan::Control::default());
        state.control = Some(control.clone());
        let result = std::sync::Arc::new(
            super::SpendSummary::compute(
                request.clone(),
                std::sync::Arc::new(Default::default()),
                None,
            )
            .unwrap(),
        );
        state.result = Some(result.clone());
        state.cache.insert(result.clone());
        let mut other = request.clone();
        other.query.source = Some("other".into());
        state.select(other.clone());
        assert!(control.is_cancelled());
        assert!(state.result.is_none());
        assert!(!state.accepts(&request, old_generation));
        assert!(state.accepts(&other, state.generation));
        state.select(request);
        assert!(std::sync::Arc::ptr_eq(
            state.result.as_ref().unwrap(),
            &result
        ));
    }

    #[test]
    fn closing_and_reopening_cannot_accept_a_cancelled_analysis_completion() {
        let request = analysis_request();
        let mut state = super::SpendAnalysisState::default();
        state.select(request.clone());
        state.worker_running = true;
        let control = std::sync::Arc::new(quotascope_core::scan::Control::default());
        state.control = Some(control.clone());
        let old_generation = state.generation;
        state.release();
        assert!(control.is_cancelled() && state.worker_running);
        assert!(state.request.is_none() && state.result.is_none());
        state.select(request.clone());
        assert!(!state.accepts(&request, old_generation));
        assert!(state.accepts(&request, state.generation));
        let generation = state.generation;
        state.cancel();
        assert!(!state.accepts(&request, generation));
    }

    #[test]
    fn replacing_the_raw_snapshot_invalidates_its_derived_result() {
        let request = analysis_request();
        let mut state = super::SettingsSnapshot::default();
        state.set_spend(request.snapshot.clone());
        state.spend_analysis.select(request.clone());
        let generation = state.spend_analysis.generation;
        state.set_spend(request.snapshot.clone());
        assert!(state.spend_analysis.accepts(&request, generation));
        state.set_spend(std::sync::Arc::new(Default::default()));
        assert!(!state.spend_analysis.accepts(&request, generation));
        assert!(state.spend_analysis.result.is_none());
    }

    #[test]
    fn cleanup_and_budget_changes_wait_for_a_running_scan() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let host = super::SettingsHost::new(tx, std::sync::mpsc::channel().0);
        let shared = host.shared();
        shared.snapshot.lock().unwrap().spend_worker_running = true;
        shared.request_maintenance(super::MaintenanceOperation::Clear);
        shared.request_maintenance(super::MaintenanceOperation::Enforce);
        let state = shared.snapshot.lock().unwrap();
        assert!(!state.maintenance_running);
        assert!(state.maintenance_result.is_none());
        assert_eq!(state.generation, 0);
    }

    #[test]
    fn hiding_settings_releases_spend_and_invalidates_pending_results() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let host = super::SettingsHost::new(tx, std::sync::mpsc::channel().0);
        let shared = host.shared();
        let control = std::sync::Arc::new(quotascope_core::scan::Control::default());
        {
            let mut state = shared.snapshot.lock().unwrap();
            state.spend = Some(std::sync::Arc::new(Default::default()));
            state.spend_loading = true;
            state.spend_generation = 42;
            state.spend_worker_running = true;
            state.spend_control = Some(control.clone());
            state.spend_progress = Some(Default::default());
        }
        shared.release_spend();
        let state = shared.snapshot.lock().unwrap();
        assert!(state.spend.is_none());
        assert!(!state.spend_loading);
        assert_ne!(state.spend_generation, 42);
        assert!(control.is_cancelled());
        assert!(state.spend_progress.is_none());
        assert!(
            state.spend_worker_running,
            "a second scan must wait for this worker to exit"
        );
    }

    #[test]
    fn cancelling_keeps_the_last_complete_snapshot_and_invalidates_worker_results() {
        let host =
            super::SettingsHost::new(std::sync::mpsc::channel().0, std::sync::mpsc::channel().0);
        let shared = host.shared();
        let snapshot = std::sync::Arc::new(quotascope_core::spend::Snapshot::default());
        let control = std::sync::Arc::new(quotascope_core::scan::Control::default());
        {
            let mut state = shared.snapshot.lock().unwrap();
            state.spend = Some(snapshot.clone());
            state.spend_worker_running = true;
            state.spend_loading = true;
            state.spend_control = Some(control.clone());
            state.spend_generation = 42;
        }
        shared.cancel_spend();
        let generation = shared.generation();
        shared.cancel_spend();
        assert_eq!(
            shared.generation(),
            generation,
            "duplicate cancellation is harmless"
        );
        let state = shared.snapshot.lock().unwrap();
        assert!(control.is_cancelled());
        assert!(std::sync::Arc::ptr_eq(
            state.spend.as_ref().unwrap(),
            &snapshot
        ));
        assert!(!state.spend_loading);
        assert!(state.spend_cancelled);
        assert_ne!(state.spend_generation, 42);
    }

    #[test]
    fn balance_edits_reject_nonfinite_zero_negative_and_half_typed_amounts() {
        for text in ["NaN", "inf", "-inf", "0", "-1", "12x", "-"] {
            assert!(positive_or_empty(text).is_err(), "{text}");
        }
        assert_eq!(positive_or_empty(" 1.25 "), Ok(Some(1.25)));
        assert_eq!(positive_or_empty("  "), Ok(None));
    }

    #[test]
    fn service_status_late_completion_after_leave_and_reopen_is_rejected() {
        use quotascope_core::service_status::{Page, Reading};
        let mut state = super::ServiceStatusState {
            running: true,
            ..Default::default()
        };
        let generation = state.generation;
        state.release();
        assert!(
            state.running,
            "reopening must wait for the bounded old worker"
        );
        state.complete(
            generation,
            true,
            Ok(Reading {
                page: Page::Codex,
                checked_at: 1,
                components: vec![],
            }),
        );
        assert!(!state.running);
        assert!(state.reading.is_none() && !state.failed);
        state.running = true;
        state.complete(
            state.generation,
            false,
            Ok(Reading {
                page: Page::Codex,
                checked_at: 2,
                components: vec![],
            }),
        );
        assert!(state.reading.is_none());
    }

    #[test]
    fn service_status_failed_retry_keeps_the_previous_timestamp_and_marks_it_old() {
        use quotascope_core::service_status::{Page, ReadError, Reading};
        let mut state = super::ServiceStatusState::default();
        let reading = Reading {
            page: Page::Claude,
            checked_at: 11,
            components: vec![],
        };
        state.complete(0, true, Ok(reading.clone()));
        state.complete(0, true, Err(ReadError::Unreachable));
        assert!(state.failed);
        assert_eq!(state.reading, Some(reading));
        state.complete(
            0,
            true,
            Ok(Reading {
                page: Page::Claude,
                checked_at: 22,
                components: vec![],
            }),
        );
        assert!(!state.failed);
        assert_eq!(state.reading.unwrap().checked_at, 22);
    }
}
