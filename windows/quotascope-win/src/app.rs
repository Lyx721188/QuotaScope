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
use crate::settings_app::{SettingsAction, SettingsHost};
use crate::tray::{DashboardEntry, TrayCommand, TrayIcon};

pub enum AppMsg {
    Tray(TrayCommand),
    Panel(PanelEvent),
    Settings(SettingsAction),
    StorePoll,
    /// A second launch asked for the settings window.
    OpenSettings,
    DeviceFlowDone(Result<String, String>),
    /// The value estimates for the transcript-backed accounts, worked out off
    /// the UI path: account id -> window id -> the card's estimate line.
    Estimates(std::collections::HashMap<String, std::collections::HashMap<String, String>>),
    Histories(u64, HashMap<String, quotascope_core::history::HistoryRead>),
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
    histories_dirty: bool,
    histories_generation: u64,
    histories_at: Option<std::time::Instant>,
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
            histories_dirty: true,
            histories_generation: 0,
            histories_at: None,
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
            let _ = SetTimer(Some(app.panel.hwnd), 1, 30, None);
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
    }

    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                AppMsg::Tray(command) => self.handle_tray(command),
                AppMsg::Panel(event) => self.handle_panel(event),
                AppMsg::Settings(action) => self.handle_settings(action),
                AppMsg::StorePoll => self.poll_store(),
                AppMsg::OpenSettings => self.settings.show(),
                AppMsg::Estimates(map) => {
                    self.estimates_running = false;
                    self.estimates_at = Some(std::time::Instant::now());
                    self.estimates = map;
                    self.rebuild_entries();
                }
                AppMsg::Histories(generation, map) => {
                    self.histories_running = false;
                    if generation == self.histories_generation {
                        self.histories_at = Some(std::time::Instant::now());
                        self.histories = map;
                        self.rebuild_entries();
                    }
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
                    unsafe {
                        let _ = ShowWindow(self.panel.hwnd, SW_HIDE);
                    }
                } else {
                    self.panel.show();
                }
            }
            TrayCommand::OpenSettings => {
                self.settings.show();
            }
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
            SettingsAction::SignInCopilot => self.start_device_flow(),
            SettingsAction::SignOutCopilot => {
                self.store.send(Command::SettingsChanged);
                self.settings.refresh();
            }
            SettingsAction::OpenUrl(url) => open_in_browser(&url),
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
        self.maybe_refresh_estimates();
        self.maybe_refresh_histories();
    }

    fn invalidate_histories(&mut self) {
        self.histories_dirty = true;
        self.histories_generation = self.histories_generation.wrapping_add(1);
        self.histories.clear();
        self.rebuild_entries();
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
            .filter(|a| settings.detailed_cards.contains(&a.id()))
            .filter(|a| a.provider.provides_history())
            .filter(|a| {
                settings.reads_token_spend
                    || matches!(a.provider, Provider::Zai | Provider::GlmCoding)
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
        std::thread::spawn(move || {
            let map = subjects
                .into_iter()
                .map(|a| (a.id(), quotascope_core::history::read(a.provider)))
                .collect();
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
            .filter(|r| matches!(r.account.provider, Provider::ClaudeCode | Provider::Codex))
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
        std::thread::spawn(move || {
            let now = quotascope_core::timeutil::now_ms();
            let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
            for reading in subjects {
                let ledger = quotascope_core::ledger::ledger(reading.account.provider);
                let mut lines = HashMap::new();
                for window in &reading.windows {
                    if let Some(text) = quotascope_core::estimate::window_text(window, &ledger, now)
                    {
                        lines.insert(window.id.clone(), text);
                    }
                }
                out.insert(reading.account.id(), lines);
            }
            let _ = tx.send(AppMsg::Estimates(out));
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
                entry.value_lines = self
                    .estimates
                    .get(&account.id())
                    .cloned()
                    .unwrap_or_default();
                entry.history = self.histories.get(&account.id()).cloned();
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
