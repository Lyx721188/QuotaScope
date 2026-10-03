//! The orchestrator: three windows (panel, tray, settings), the store
//! worker, and the message loop that keeps them in step. Everything the
//! windows decide travels through one channel; everything the store
//! produces comes back through one poll.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};

use quotascope_core::model::{Provider, ProviderUsage, State};
use quotascope_core::store::{Command, StoreHandle, Update};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::panel::{PanelEvent, PanelWindow, RailEntry};
use crate::settings_app::{SettingsAction, SettingsHost, UpdateStatus};
use crate::tray::{DashboardEntry, TrayCommand, TrayIcon};

pub enum AppMsg {
    Tray(TrayCommand),
    Panel(PanelEvent),
    Settings(SettingsAction),
    StorePoll,
    /// A second launch asked for the settings window.
    OpenSettings,
    OpenDashboard,
    RefreshAll,
    DeviceFlowDone(Result<String, String>),
    /// The value estimates for the transcript-backed accounts, worked out off
    /// the UI path: account id -> window id -> the card's estimate line.
    Estimates(
        u64,
        std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    ),
    Histories(u64, HashMap<String, quotascope_core::history::HistoryRead>),
    PromptCache(
        u64,
        HashMap<Provider, quotascope_core::prompt_cache::CacheReading>,
    ),
    CodexDetails(u64, quotascope_core::codex_account::AccountDetails),
    UpdateChecked(bool, quotascope_core::updates::CheckResult),
}

pub struct App {
    panel: Box<PanelWindow>,
    tray: Box<TrayIcon>,
    settings: SettingsHost,
    store: StoreHandle,
    tx: Sender<AppMsg>,
    rx: Receiver<AppMsg>,
    readings: HashMap<String, ProviderUsage>,
    refreshing: std::collections::HashSet<String>,
    /// The transcript ledgers' value estimates, keyed like the readings. Kept
    /// beside the rail entries so the card can draw them without the card
    /// itself ever touching the file system.
    estimates: HashMap<String, HashMap<String, String>>,
    estimates_running: bool,
    estimates_dirty: bool,
    estimates_at: Option<std::time::Instant>,
    histories: HashMap<String, quotascope_core::history::HistoryRead>,
    histories_running: bool,
    histories_control: Option<std::sync::Arc<quotascope_core::scan::Control>>,
    histories_dirty: bool,
    histories_generation: u64,
    histories_at: Option<std::time::Instant>,
    prompt_cache: HashMap<Provider, quotascope_core::prompt_cache::CacheReading>,
    prompt_cache_running: bool,
    prompt_cache_at: Option<std::time::Instant>,
    codex_details: Option<quotascope_core::codex_account::AccountDetails>,
    codex_details_running: bool,
    codex_details_at: Option<std::time::Instant>,
    update_check_running: bool,
    update_notified: Option<String>,
    maintenance_at: std::time::Instant,
}

/// Long enough that moving between rings does not rescan, short enough that
/// "Today" is today's — upstream's card keeps the same lifetime.
const ESTIMATES_LIFETIME: std::time::Duration = std::time::Duration::from_secs(5 * 60);

impl App {
    /// Starts the app on the calling thread — a worker, since the main
    /// thread belongs to the WinUI settings host. `open_settings` asks the
    /// main thread to open the window; `shared_tx` hands back the state
    /// that window reads.
    pub fn start(
        open_settings: Sender<()>,
        shared_tx: std::sync::mpsc::Sender<std::sync::Arc<crate::settings_app::Shared>>,
    ) -> App {
        let (tx, rx) = channel::<AppMsg>();
        // Reclaim pre-existing oversized statistics without blocking either UI.
        std::thread::spawn(quotascope_core::statistics_cache::enforce);

        // The windows speak their own vocabularies; forwarder threads fold
        // them into the one channel the loop listens to.
        let (panel_tx, panel_rx) = channel::<PanelEvent>();
        let (settings_tx, settings_rx) = channel::<SettingsAction>();
        let (tray_tx, tray_rx) = channel::<TrayCommand>();
        let (poll_tx, poll_rx) = channel::<()>();

        // Forwarders into the main channel.
        std::thread::spawn({
            let tx = tx.clone();
            move || {
                for event in panel_rx {
                    if tx.send(AppMsg::Panel(event)).is_err() {
                        return;
                    }
                }
            }
        });
        std::thread::spawn({
            let tx = tx.clone();
            move || {
                for action in settings_rx {
                    if tx.send(AppMsg::Settings(action)).is_err() {
                        return;
                    }
                }
            }
        });
        std::thread::spawn({
            let tx = tx.clone();
            move || {
                for command in tray_rx {
                    if tx.send(AppMsg::Tray(command)).is_err() {
                        return;
                    }
                }
            }
        });
        std::thread::spawn({
            let tx = tx.clone();
            move || {
                for tick in poll_rx {
                    let _ = tick;
                    if tx.send(AppMsg::StorePoll).is_err() {
                        return;
                    }
                }
            }
        });
        // A second launch asks through the named event; this end opens the
        // settings window.
        {
            let (signal_tx, signal_rx) = channel::<()>();
            crate::winutil::listen_for_open_settings(signal_tx);
            std::thread::spawn({
                let tx = tx.clone();
                move || {
                    for _ in signal_rx {
                        if tx.send(AppMsg::OpenSettings).is_err() {
                            return;
                        }
                    }
                }
            });
        }

        for (name, dashboard) in [
            ("QuotaScope.Windows.OpenDashboard", true),
            ("QuotaScope.Windows.RefreshAll", false),
        ] {
            let (signal_tx, signal_rx) = channel::<()>();
            crate::winutil::listen_for_action(name, signal_tx);
            let tx = tx.clone();
            std::thread::spawn(move || {
                for _ in signal_rx {
                    if tx
                        .send(if dashboard {
                            AppMsg::OpenDashboard
                        } else {
                            AppMsg::RefreshAll
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }

        // First-run resolution happens exactly once, at launch, on the UI
        // path — `--json` never stamps anything.
        quotascope_core::settings::mutate(|s| s.resolve_first_run());
        sync_extension_names();

        let panel = PanelWindow::new(panel_tx);
        let mut tray = TrayIcon::new(tray_tx);
        tray.set_poll(poll_tx);
        let settings = SettingsHost::new(settings_tx, open_settings);
        let store = StoreHandle::start();

        let _ = shared_tx.send(settings.shared());

        let mut app = App {
            panel,
            tray,
            settings,
            store,
            tx: tx.clone(),
            rx,
            readings: HashMap::new(),
            refreshing: std::collections::HashSet::new(),
            estimates: HashMap::new(),
            estimates_running: false,
            estimates_dirty: true,
            estimates_at: None,
            histories: HashMap::new(),
            histories_running: false,
            histories_control: None,
            histories_dirty: true,
            histories_generation: 0,
            histories_at: None,
            prompt_cache: HashMap::new(),
            prompt_cache_running: false,
            prompt_cache_at: None,
            codex_details: None,
            codex_details_running: false,
            codex_details_at: None,
            update_check_running: false,
            update_notified: None,
            maintenance_at: std::time::Instant::now(),
        };

        // Paint the rail from the cache before the first round trip, so it
        // is not blank on a cold start.
        let initial = quotascope_core::store::initial_readings_from_cache();
        for reading in &initial {
            app.readings.insert(reading.account.id(), reading.clone());
        }
        app.rebuild_entries();
        app.panel.show();
        app.tray
            .set_hidden(quotascope_core::settings::with(|s| s.hides_tray_icon));

        // Timers: the panel's frame clock, and the store's poll.
        unsafe {
            let _ = SetTimer(Some(app.tray.hwnd), 2, 250, None);
        }

        // Something happening is a reason to look now.
        app.store.send(Command::RefreshAll);
        app
    }

    /// The message loop. Timers wake it; everything else arrives through
    /// the channel.
    pub fn run(&mut self) {
        unsafe {
            let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                self.drain();
            }
        }
        self.store.send(Command::Shutdown);
        quotascope_core::codex_rpc::shutdown();
        quotascope_core::spend_warmer::clear();
    }

    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                AppMsg::Tray(command) => self.handle_tray(command),
                AppMsg::Panel(event) => self.handle_panel(event),
                AppMsg::Settings(action) => self.handle_settings(action),
                AppMsg::StorePoll => self.poll_store(),
                AppMsg::OpenSettings => self.settings.show(),
                AppMsg::OpenDashboard => self.settings.show_dashboard(),
                AppMsg::RefreshAll => self.handle_tray(TrayCommand::RefreshAll),
                AppMsg::Estimates(generation, map) => {
                    self.estimates_running = false;
                    if generation == self.histories_generation {
                        self.estimates_at = Some(std::time::Instant::now());
                        self.estimates = map;
                        self.rebuild_entries();
                    }
                }
                AppMsg::Histories(generation, map) => {
                    self.histories_running = false;
                    self.histories_control = None;
                    if generation == self.histories_generation {
                        self.histories_at = Some(std::time::Instant::now());
                        self.histories = map;
                        self.rebuild_entries();
                    }
                }
                AppMsg::PromptCache(generation, reading) => {
                    self.prompt_cache_running = false;
                    if generation == self.histories_generation {
                        self.prompt_cache = reading;
                        self.prompt_cache_at = Some(std::time::Instant::now());
                        self.rebuild_entries();
                    }
                }
                AppMsg::CodexDetails(generation, reading) => {
                    self.codex_details_running = false;
                    if generation == self.histories_generation {
                        self.codex_details = Some(reading);
                        self.codex_details_at = Some(std::time::Instant::now());
                        self.rebuild_entries();
                    }
                }
                AppMsg::UpdateChecked(manual, result) => {
                    self.update_check_running = false;
                    if let quotascope_core::updates::CheckResult::Available(release) = &result {
                        let remind = quotascope_core::settings::with(|s| {
                            s.checks_for_updates
                                && s.skipped_update_version.as_deref() != Some(&release.version)
                        });
                        if !manual
                            && remind
                            && self.update_notified.as_deref() != Some(&release.version)
                        {
                            self.tray.show_balloon(
                                "QuotaScope",
                                &format!(
                                    "{} {} · {}",
                                    quotascope_core::localization::t("Windows update available:"),
                                    release.version,
                                    quotascope_core::localization::t(
                                        "Open Settings → About to choose an update."
                                    )
                                ),
                                false,
                            );
                            self.update_notified = Some(release.version.clone());
                        }
                    }
                    self.settings.set_update_status(UpdateStatus::Done(result));
                }
                AppMsg::DeviceFlowDone(result) => match result {
                    Ok(token) => {
                        quotascope_core::secrets::set_key("copilot", &token);
                        self.tray.show_balloon(
                            "QuotaScope",
                            &quotascope_core::localization::t("Signed in to GitHub.").to_string(),
                            false,
                        );
                        self.store.send(Command::SettingsChanged);
                        self.settings.refresh();
                    }
                    Err(text) => {
                        self.tray.show_balloon("QuotaScope", &text, true);
                    }
                },
            }
        }
    }

    fn handle_tray(&mut self, command: TrayCommand) {
        match command {
            TrayCommand::TogglePanel => {
                if self.panel.is_visible() {
                    self.panel.hide();
                } else {
                    self.panel.show();
                }
            }
            TrayCommand::OpenSettings => {
                self.settings.show();
            }
            TrayCommand::OpenDashboard => self.settings.show_dashboard(),
            TrayCommand::OpenUsagePage(url) => open_in_browser(url),
            TrayCommand::RefreshAll => {
                self.invalidate_histories();
                let accounts = quotascope_core::settings::with(|s| {
                    s.ordered_enabled()
                        .into_iter()
                        .map(|a| a.id())
                        .collect::<Vec<_>>()
                });
                self.refreshing.extend(accounts);
                self.store.send(Command::RefreshAll);
                self.mark_refreshing();
            }
            TrayCommand::Exit => unsafe {
                PostQuitMessage(0);
            },
        }
    }

    fn handle_panel(&mut self, event: PanelEvent) {
        match event {
            PanelEvent::RefreshAccount(account) => {
                self.invalidate_histories();
                self.refreshing.insert(account.id());
                self.store.send(Command::RefreshAccount(account));
                self.mark_refreshing();
            }
            PanelEvent::PositionChanged => {
                // Nothing to do: the panel persisted its own position.
            }
        }
    }

    fn handle_settings(&mut self, action: SettingsAction) {
        match action {
            SettingsAction::Changed => {
                self.tray
                    .set_global_shortcuts(quotascope_core::settings::with(|s| s.global_shortcuts));
                self.invalidate_histories();
                self.panel.reload_settings();
                self.tray
                    .set_hidden(quotascope_core::settings::with(|s| s.hides_tray_icon));
                sync_extension_names();
                self.estimates_dirty = true;
                self.store.send(Command::SettingsChanged);
                self.settings.refresh();
                self.rebuild_entries();
            }
            SettingsAction::RefreshProvider(raw) => {
                self.invalidate_histories();
                if let Some(provider) = Provider::from_raw(&raw) {
                    self.refreshing
                        .insert(quotascope_core::model::AccountKey::primary(provider).id());
                    self.store.send(Command::RefreshAccount(
                        quotascope_core::model::AccountKey::primary(provider),
                    ));
                    self.mark_refreshing();
                }
            }
            SettingsAction::SaveKey => {
                self.invalidate_histories();
                self.store.send(Command::SettingsChanged);
            }
            SettingsAction::RefreshAccount(account) => {
                self.invalidate_histories();
                self.refreshing.insert(account.id());
                self.store.send(Command::RefreshAccount(account));
                self.mark_refreshing();
            }
            SettingsAction::SignInCopilot => self.start_device_flow(),
            SettingsAction::SignOutCopilot => {
                self.store.send(Command::SettingsChanged);
                self.settings.refresh();
            }
            SettingsAction::OpenUrl(url) => open_in_browser(&url),
            SettingsAction::OpenFolder(path) => open_in_browser(&path.to_string_lossy()),
            SettingsAction::CheckUpdates => self.check_updates(true),
            SettingsAction::SkipUpdate(version) => {
                quotascope_core::settings::mutate(|s| s.skipped_update_version = Some(version));
                self.settings.refresh();
            }
        }
    }

    /// The GitHub device flow, on its own thread: ask for a code, put it
    /// in the clipboard and the browser, poll until done. The result comes
    /// back through the channel like everything else.
    fn start_device_flow(&mut self) {
        let tx = self.tx.clone();
        let http = quotascope_core::http::HttpClient::new();
        std::thread::spawn(move || {
            let run = || -> Result<String, String> {
                let prompt = quotascope_core::providers::device_login::start(&http)
                    .map_err(|e| e.to_string())?;
                crate::clipboard::set_text(&prompt.user_code);
                open_in_browser(&prompt.verification_url);
                // The clipboard is the convenience, not the message: the
                // user still types the code, which is the point.
                quotascope_core::providers::device_login::await_token(&http, &prompt)
                    .map_err(|e| e.to_string())
            };
            let _ = tx.send(AppMsg::DeviceFlowDone(run()));
        });
        self.tray.show_balloon(
            "QuotaScope",
            &quotascope_core::localization::t("Device code copied. Finish signing in at github.com/login/device — the code is on your clipboard.").to_string(),
            false,
        );
        self.settings.refresh();
    }

    fn mark_refreshing(&mut self) {
        for entry in self.panel.entries_mut() {
            if self.refreshing.contains(&entry.account.id()) {
                entry.ring.is_refreshing = true;
            }
        }
        self.panel.redraw();
    }

    fn poll_store(&mut self) {
        self.panel.follow_pointer_if_enabled();

        while let Some(update) = self.store.poll() {
            match update {
                Update::Readings(readings) => {
                    for reading in readings {
                        self.refreshing.remove(&reading.account.id());
                        if matches!(
                            reading.account.provider,
                            Provider::ClaudeCode | Provider::Codex
                        ) {
                            // New windows may be priced differently now.
                            self.estimates_dirty = true;
                        }
                        self.readings.insert(reading.account.id(), reading);
                    }
                    self.rebuild_entries();
                }
                Update::Alert(event) => {
                    let (title, text) = event.notification_text();
                    let warning =
                        !matches!(event, quotascope_core::alerts::AlertEvent::Reset { .. });
                    self.tray.show_balloon(&title, &text, warning);
                }
            }
        }
        self.update_settings_status();
        if self.panel.is_visible() {
            self.maybe_refresh_estimates();
            self.maybe_refresh_histories();
            self.maybe_refresh_prompt_cache();
            self.maybe_refresh_codex_details();
        }
        if self.maintenance_at.elapsed() >= std::time::Duration::from_secs(30) {
            self.maintenance_at = std::time::Instant::now();
            if self.settings.is_open() {
                quotascope_core::spend_warmer::cancel_running();
            } else {
                quotascope_core::spend_warmer::tick();
            }
            quotascope_core::ledger::release_expired_memory(quotascope_core::settings::with(|s| {
                s.reads_token_spend
            }));
        }
        self.check_updates(false);
    }

    fn check_updates(&mut self, manual: bool) {
        let now = quotascope_core::timeutil::now_ms();
        let due = quotascope_core::settings::with(|s| {
            s.checks_for_updates
                && s.last_update_check_at
                    .is_none_or(|at| now < at || now.saturating_sub(at) >= 24 * 60 * 60 * 1000)
        });
        if self.update_check_running || (!manual && !due) {
            return;
        }
        self.update_check_running = true;
        quotascope_core::settings::mutate(|s| s.last_update_check_at = Some(now));
        self.settings.set_update_status(UpdateStatus::Checking);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = quotascope_core::updates::check(env!("CARGO_PKG_VERSION"));
            let _ = tx.send(AppMsg::UpdateChecked(manual, result));
        });
    }

    fn invalidate_histories(&mut self) {
        if let Some(control) = &self.histories_control {
            control.cancel();
        }
        self.histories_dirty = true;
        self.histories_generation = self.histories_generation.wrapping_add(1);
        self.histories.clear();
        self.estimates.clear();
        self.estimates_at = None;
        self.estimates_dirty = true;
        self.prompt_cache.clear();
        self.prompt_cache_at = None;
        self.codex_details = None;
        self.codex_details_at = None;
        self.rebuild_entries();
    }

    fn maybe_refresh_prompt_cache(&mut self) {
        let enabled = quotascope_core::settings::with(|s| {
            s.reads_token_spend
                && ["claudeCode", "codex"]
                    .iter()
                    .any(|id| s.enabled_accounts.contains(*id) && s.detailed_cards.contains(*id))
        });
        if !enabled
            || !self.panel.needs_prompt_cache()
            || self.prompt_cache_running
            || self
                .prompt_cache_at
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(5))
        {
            return;
        }
        self.prompt_cache_running = true;
        let generation = self.histories_generation;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let reading = [Provider::ClaudeCode, Provider::Codex]
                .into_iter()
                .map(|provider| {
                    (
                        provider,
                        quotascope_core::prompt_cache::read_for(
                            provider,
                            &quotascope_core::home_dir(),
                            quotascope_core::timeutil::now_ms(),
                        ),
                    )
                })
                .collect();
            let _ = tx.send(AppMsg::PromptCache(generation, reading));
        });
    }

    fn maybe_refresh_codex_details(&mut self) {
        let (enabled, detailed) = quotascope_core::settings::with(|s| {
            let detailed = s.detailed_cards.contains("codex");
            (
                s.enabled_accounts.contains("codex") && (s.shows_codex_reset_credits || detailed),
                detailed,
            )
        });
        if !enabled
            || self.codex_details_running
            || self
                .codex_details_at
                .is_some_and(|at| at.elapsed() < ESTIMATES_LIFETIME)
        {
            return;
        }
        self.codex_details_running = true;
        let generation = self.histories_generation;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let reading = quotascope_core::codex_account::fetch(detailed);
            let _ = tx.send(AppMsg::CodexDetails(generation, reading));
        });
    }

    /// Reading records and querying statistics must never block a hover or
    /// the settings thread. Generation checks discard replies for old keys.
    fn maybe_refresh_histories(&mut self) {
        if self.histories_running {
            return;
        }
        let settings = quotascope_core::settings::with(|s| s.clone());
        let subjects: Vec<_> = settings
            .ordered_enabled()
            .into_iter()
            .filter(|a| a.is_primary() || a.provider == Provider::DeepSeek)
            .filter(|a| settings.detailed_cards.contains(&a.id()))
            .filter(|a| a.provider.provides_history())
            .filter(|a| {
                settings.reads_token_spend
                    || matches!(
                        a.provider,
                        Provider::Zai
                            | Provider::GlmCoding
                            | Provider::OpenCodeGo
                            | Provider::DeepSeek
                    )
            })
            .collect();
        if subjects.is_empty() {
            if !self.histories.is_empty() {
                self.histories.clear();
                self.rebuild_entries();
            }
            self.histories_dirty = false;
            return;
        }
        if !self.histories_dirty
            && self
                .histories_at
                .is_some_and(|at| at.elapsed() < ESTIMATES_LIFETIME)
        {
            return;
        }
        self.histories_running = true;
        self.histories_dirty = false;
        let generation = self.histories_generation;
        let tx = self.tx.clone();
        let control = std::sync::Arc::new(quotascope_core::scan::Control::default());
        self.histories_control = Some(control.clone());
        std::thread::spawn(move || {
            let stop = control.clone();
            let map = quotascope_core::scan::run(
                control,
                0,
                |_| {},
                || {
                    subjects
                        .into_iter()
                        .take_while(|_| !stop.is_cancelled())
                        .map(|a| (a.id(), quotascope_core::history::read_account(&a)))
                        .collect()
                },
            )
            .unwrap_or_default();
            let _ = tx.send(AppMsg::Histories(generation, map));
        });
    }

    /// Works out the value estimates off the UI path, the way upstream reads
    /// its ledgers when a card opens: on a worker, with the results coming
    /// back through the channel. A cold first scan can walk a few hundred
    /// megabytes of transcripts; nothing here may block the message loop.
    fn maybe_refresh_estimates(&mut self) {
        if self.estimates_running {
            return;
        }
        if !quotascope_core::settings::with(|s| s.reads_token_spend) {
            if !self.estimates.is_empty() {
                self.estimates.clear();
                self.rebuild_entries();
            }
            self.estimates_dirty = false;
            return;
        }
        let subjects: Vec<ProviderUsage> = self
            .readings
            .values()
            .filter(|r| r.account.is_primary())
            .filter(|r| {
                matches!(
                    r.account.provider,
                    Provider::ClaudeCode | Provider::Codex | Provider::Antigravity
                )
            })
            .cloned()
            .collect();
        if subjects.is_empty() {
            return;
        }
        let fresh = self
            .estimates_at
            .map(|at| at.elapsed() >= ESTIMATES_LIFETIME)
            .unwrap_or(true);
        if !self.estimates_dirty && !fresh {
            return;
        }
        self.estimates_running = true;
        self.estimates_dirty = false;
        let tx = self.tx.clone();
        let generation = self.histories_generation;
        std::thread::spawn(move || {
            let now = quotascope_core::timeutil::now_ms();
            let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
            let mut elsewhere = quotascope_core::elsewhere::Watch::load();
            quotascope_core::ledger::invalidate_memory();
            for reading in subjects {
                let ledger = quotascope_core::ledger::ledger(reading.account.provider);
                elsewhere.observe(&reading, &ledger, now);
                // Only Antigravity can say which of its logged models spent a
                // given window; every other provider's windows are priced as
                // account-wide or not at all.
                let rule = match reading.account.provider {
                    Provider::Antigravity => quotascope_core::providers::antigravity::ESTIMATE,
                    _ => quotascope_core::estimate::Rule::default(),
                };
                let mut lines = HashMap::new();
                for window in &reading.windows {
                    if elsewhere.marked(&reading, window) {
                        lines.insert(
                            window.id.clone(),
                            quotascope_core::localization::t(
                                "Usage outside this computer was detected; value estimate paused.",
                            )
                            .to_string(),
                        );
                        continue;
                    }
                    if let Some(text) = quotascope_core::estimate::window_text_at(
                        window,
                        &ledger,
                        reading.observed_at,
                        now,
                        rule,
                    ) {
                        lines.insert(window.id.clone(), text);
                    }
                }
                out.insert(reading.account.id(), lines);
            }
            elsewhere.save();
            let _ = tx.send(AppMsg::Estimates(generation, out));
        });
    }

    fn rebuild_entries(&mut self) {
        let settings = quotascope_core::settings::with(|s| s.clone());
        let entries: Vec<RailEntry> = settings
            .ordered_enabled()
            .into_iter()
            .map(|account| {
                let mut entry = match self.readings.get(&account.id()) {
                    Some(reading) => {
                        RailEntry::from_reading(reading, &settings, settings.shows_remaining)
                    }
                    None => RailEntry::placeholder(account.provider, &settings),
                };
                if self.refreshing.contains(&account.id()) {
                    entry.ring.is_refreshing = true;
                }
                entry.account = account.clone();
                entry.title = crate::panel::account_title(account.provider, &account, &settings);
                entry.value_lines = self
                    .estimates
                    .get(&account.id())
                    .cloned()
                    .unwrap_or_default();
                entry.history = self.histories.get(&account.id()).cloned();
                if account.is_primary()
                    && matches!(account.provider, Provider::ClaudeCode | Provider::Codex)
                {
                    entry.prompt_cache = self.prompt_cache.get(&entry.account.provider).cloned();
                }
                if account.is_primary() && account.provider == Provider::Codex {
                    entry.codex_details = self.codex_details.clone();
                }
                entry
            })
            .collect();
        self.panel.set_entries(entries);
        self.tray.set_dashboard(
            self.panel
                .entries_mut()
                .iter()
                .map(|entry| {
                    let summary = entry
                        .usage
                        .as_ref()
                        .map(|reading| match &reading.state {
                            State::Unavailable(reason) => reason.message().to_string(),
                            State::Live | State::Stale => {
                                let figure = reading
                                    .headline_window(
                                        settings
                                            .pinned_windows
                                            .get(&entry.account.id())
                                            .map(String::as_str),
                                    )
                                    .map(|window| window.percent_text(settings.shows_remaining))
                                    .or_else(|| reading.credit_balance.clone())
                                    .unwrap_or_else(|| {
                                        quotascope_core::localization::t("No reading").to_string()
                                    });
                                if matches!(reading.state, State::Stale) {
                                    format!(
                                        "{} · {}",
                                        figure,
                                        quotascope_core::localization::t(
                                            "Reading may be out of date"
                                        )
                                    )
                                } else {
                                    figure
                                }
                            }
                        })
                        .unwrap_or_else(|| {
                            quotascope_core::localization::t("No reading").to_string()
                        });
                    DashboardEntry {
                        title: entry.title.clone(),
                        summary,
                        usage_page: entry.account.provider.usage_page(),
                    }
                })
                .collect(),
        );
    }

    fn update_settings_status(&mut self) {
        if !self.settings.is_open() {
            return;
        }
        self.settings.set_readings(&self.readings);
        for (id, reading) in &self.readings {
            let provider_raw = reading.provider().raw().to_string();
            let _ = id;
            let text = match &reading.state {
                State::Live => {
                    let plan = reading
                        .plan
                        .as_ref()
                        .map(|p| format!(" · {p}"))
                        .unwrap_or_default();
                    let headline = reading.headline_window(None);
                    match headline {
                        Some(window) => format!(
                            "{}{} · {}",
                            window.percent_text(false),
                            plan,
                            quotascope_core::timeutil::relative_text(
                                reading.observed_at.unwrap_or(0)
                            )
                        ),
                        None => reading.credit_balance.clone().unwrap_or_else(|| {
                            quotascope_core::localization::t("No reading").to_string()
                        }),
                    }
                }
                State::Stale => {
                    quotascope_core::localization::t("Reading may be out of date").to_string()
                }
                State::Unavailable(reason) => reason.message().to_string(),
            };
            self.settings.set_status(&provider_raw, text);
        }
    }
}

pub fn open_in_browser(url: &str) {
    unsafe {
        let url_wide = crate::winutil::TempWide::new(url);
        let verb = crate::winutil::TempWide::new("open");
        ShellExecuteW(
            None,
            verb.as_ptr(),
            url_wide.as_ptr(),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

/// Remembers what each discovered extension calls itself, so the rail's
/// card and `--json` can use the program's own word.
fn sync_extension_names() {
    let scan = quotascope_core::extension::scan();
    quotascope_core::settings::mutate(|s| {
        for extension in &scan.extensions {
            let id = extension.account().id();
            s.extension_names.insert(id, extension.name.clone());
        }
        // A folder that has gone takes its name with it.
        let live: std::collections::HashSet<String> =
            scan.extensions.iter().map(|e| e.account().id()).collect();
        let stale: Vec<String> = s
            .extension_names
            .keys()
            .filter(|id| !live.contains(*id))
            .cloned()
            .collect();
        for id in stale {
            s.extension_names.remove(&id);
        }
    });
}
