//! The orchestrator: three windows (panel, tray, settings), the store
//! worker, and the message loop that keeps them in step. Everything the
//! windows decide travels through one channel; everything the store
//! produces comes back through one poll.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};

use quotascope_core::model::{Provider, ProviderUsage, State};
use quotascope_core::service_status::{
    OutageMemory, Page as StatusPage, ReadError as ServiceError, Reading as ServiceReading,
};
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
    CodexDetails(u64, quotascope_core::codex_account::AccountDetails),
    UpdateChecked(
        quotascope_core::updates::Trigger,
        quotascope_core::updates::CheckResult,
    ),
    ServiceChecked(u64, Vec<(StatusPage, Result<ServiceReading, ServiceError>)>),
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
    codex_details: Option<quotascope_core::codex_account::AccountDetails>,
    codex_details_running: bool,
    codex_details_at: Option<std::time::Instant>,
    details_generation: u64,
    update_check_running: bool,
    update_result_at: Option<std::time::Instant>,
    update_notified: Option<String>,
    outage_memory: OutageMemory,
    outage_pages: std::collections::HashSet<StatusPage>,
    outage_generation: u64,
    outage_running: bool,
    outage_at: Option<std::time::Instant>,
    maintenance_at: std::time::Instant,
}

/// Long enough that moving between rings does not refetch, short enough that
/// a reset-credit count stays fresh.
const DETAILS_LIFETIME: std::time::Duration = std::time::Duration::from_secs(5 * 60);

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
            codex_details: None,
            codex_details_running: false,
            codex_details_at: None,
            details_generation: 0,
            update_check_running: false,
            update_result_at: None,
            update_notified: None,
            outage_memory: OutageMemory::load(
                &quotascope_core::data_dir().join("status-alerts.json"),
            ),
            outage_pages: Default::default(),
            outage_generation: 0,
            outage_running: false,
            outage_at: None,
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
                AppMsg::CodexDetails(generation, reading) => {
                    self.codex_details_running = false;
                    if generation == self.details_generation {
                        self.codex_details = Some(reading);
                        self.codex_details_at = Some(std::time::Instant::now());
                        self.rebuild_entries();
                    }
                }
                AppMsg::UpdateChecked(trigger, result) => {
                    self.update_check_running = false;
                    self.update_result_at = Some(std::time::Instant::now());
                    if let quotascope_core::updates::CheckResult::Available(release) = &result {
                        let remind = quotascope_core::settings::with(|s| {
                            s.checks_for_updates
                                && s.skipped_update_version.as_deref() != Some(&release.version)
                        });
                        if trigger == quotascope_core::updates::Trigger::Daily
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
                AppMsg::ServiceChecked(generation, results) => {
                    self.outage_running = false;
                    // Reconcile switches before considering a late result.
                    self.sync_outage_pages();
                    if generation == self.outage_generation {
                        for (page, result) in results {
                            if !self.outage_pages.contains(&page) {
                                continue;
                            }
                            let Ok(reading) = result else { continue };
                            let before = self.outage_memory.clone();
                            let change = self.outage_memory.changes(page, &reading.components);
                            if before != self.outage_memory {
                                self.outage_memory
                                    .save(&quotascope_core::data_dir().join("status-alerts.json"));
                            }
                            if !change.is_empty() {
                                let (title, body) = change.notification_text(page);
                                self.tray
                                    .show_balloon(&title, &body, !change.worse.is_empty());
                            }
                        }
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
                self.invalidate_details();
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
                self.invalidate_details();
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
                self.invalidate_details();
                self.panel.reload_settings();
                self.tray
                    .set_hidden(quotascope_core::settings::with(|s| s.hides_tray_icon));
                sync_extension_names();
                self.store.send(Command::SettingsChanged);
                self.settings.refresh();
                self.rebuild_entries();
            }
            SettingsAction::RefreshProvider(raw) => {
                self.invalidate_details();
                if let Some(provider) = Provider::from_raw(&raw) {
                    self.refreshing
                        .insert(quotascope_core::model::AccountKey::primary(provider).id());
                    self.store.send(Command::RefreshAccount(
                        quotascope_core::model::AccountKey::primary(provider),
                    ));
                    self.mark_refreshing();
                }
            }
            SettingsAction::ForgetReading(raw) => {
                if let Some(account) = quotascope_core::model::AccountKey::from_id(&raw) {
                    self.readings.remove(&raw);
                    self.store.send(Command::ForgetAccount(account));
                    self.invalidate_details();
                    self.rebuild_entries();
                }
            }
            SettingsAction::SaveKey => {
                self.readings.retain(|_, r| {
                    !r.provider().uses_api_key()
                        || r.credential_context.as_deref()
                            == Some(
                                quotascope_core::provider_sites::current_context(&r.account)
                                    .as_str(),
                            )
                });
                self.rebuild_entries();
                self.invalidate_details();
                self.store.send(Command::SettingsChanged);
            }
            SettingsAction::RefreshAccount(account) => {
                self.invalidate_details();
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
            SettingsAction::TestNotification => self.tray.test_notification(),
            SettingsAction::CheckUpdates => {
                self.check_updates(quotascope_core::updates::Trigger::Manual)
            }
            SettingsAction::ProbeUpdates => {
                self.check_updates(quotascope_core::updates::Trigger::About)
            }
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
                        if reading
                            .credential_context
                            .as_deref()
                            .is_some_and(|context| {
                                context
                                    != quotascope_core::provider_sites::current_context(
                                        &reading.account,
                                    )
                            })
                        {
                            continue;
                        }
                        self.refreshing.remove(&reading.account.id());
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
        quotascope_core::money::ensure_rates();
        if quotascope_core::money::take_changed() {
            self.rebuild_entries();
            self.settings.refresh();
        }
        self.update_settings_status();
        if self.panel.is_visible() {
            self.maybe_refresh_codex_details();
        }
        if self.maintenance_at.elapsed() >= std::time::Duration::from_secs(30) {
            self.maintenance_at = std::time::Instant::now();
            quotascope_core::notices::note_active();
        }
        self.check_updates(quotascope_core::updates::Trigger::Daily);
        self.maybe_check_services();
    }

    fn sync_outage_pages(&mut self) {
        let pages = quotascope_core::settings::with(quotascope_core::service_status::watched_pages);
        if pages != self.outage_pages {
            self.outage_generation = self.outage_generation.wrapping_add(1);
            self.outage_at = None;
            self.outage_pages = pages;
        }
        // Also prune disk-restored pages on the very first launch, even when
        // both the current and initial watched sets are empty.
        let before = self.outage_memory.clone();
        self.outage_memory.keep_only(&self.outage_pages);
        if before != self.outage_memory {
            self.outage_memory
                .save(&quotascope_core::data_dir().join("status-alerts.json"));
        }
    }

    fn maybe_check_services(&mut self) {
        self.sync_outage_pages();
        if self.outage_running
            || self.outage_pages.is_empty()
            || self
                .outage_at
                .is_some_and(|at| at.elapsed() < quotascope_core::service_status::CHECK_INTERVAL)
        {
            return;
        }
        self.outage_running = true;
        self.outage_at = Some(std::time::Instant::now());
        let generation = self.outage_generation;
        let pages: Vec<_> = StatusPage::ALL
            .into_iter()
            .filter(|page| self.outage_pages.contains(page))
            .collect();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let results = pages
                .into_iter()
                .map(|page| {
                    let result = std::panic::catch_unwind(|| {
                        quotascope_core::service_status::read(page, false)
                    })
                    .unwrap_or(Err(ServiceError::Unreadable));
                    (page, result)
                })
                .collect();
            let _ = tx.send(AppMsg::ServiceChecked(generation, results));
        });
    }

    fn check_updates(&mut self, trigger: quotascope_core::updates::Trigger) {
        let now = quotascope_core::timeutil::now_ms();
        let due = quotascope_core::settings::with(|s| {
            quotascope_core::updates::should_check(
                trigger,
                s.checks_for_updates,
                s.last_update_check_at,
                now,
                self.update_result_at.map(|at| at.elapsed()),
            )
        });
        if self.update_check_running || !due {
            return;
        }
        self.update_check_running = true;
        quotascope_core::settings::mutate(|s| s.last_update_check_at = Some(now));
        self.settings.set_update_status(UpdateStatus::Checking);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = quotascope_core::updates::check(env!("CARGO_PKG_VERSION"));
            let _ = tx.send(AppMsg::UpdateChecked(trigger, result));
        });
    }

    fn invalidate_details(&mut self) {
        self.details_generation = self.details_generation.wrapping_add(1);
        self.codex_details = None;
        self.codex_details_at = None;
        self.rebuild_entries();
    }

    fn maybe_refresh_codex_details(&mut self) {
        let enabled = quotascope_core::settings::with(|s| {
            s.enabled_accounts.contains("codex")
                && (s.shows_codex_reset_credits || s.detailed_cards.contains("codex"))
        });
        if !enabled
            || self.codex_details_running
            || self
                .codex_details_at
                .is_some_and(|at| at.elapsed() < DETAILS_LIFETIME)
        {
            return;
        }
        self.codex_details_running = true;
        let generation = self.details_generation;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let reading = quotascope_core::codex_account::fetch();
            let _ = tx.send(AppMsg::CodexDetails(generation, reading));
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
                entry.ring.robot = crate::robot::style(&account.id(), &settings);
                entry.title = crate::panel::account_title(account.provider, &account, &settings);
                if account.is_primary() && account.provider == Provider::Codex {
                    entry.codex_details = self.codex_details.clone();
                }
                crate::panel::split_entry(entry, &settings)
            })
            .flatten()
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
