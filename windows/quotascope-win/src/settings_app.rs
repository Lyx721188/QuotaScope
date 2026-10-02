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

/// Messages the settings window sends the app. The app owns the panel, the
/// store and the tray; the window only describes what the user asked for.
pub enum SettingsAction {
    /// A setting changed; the app re-reads everything it drives.
    Changed,
    RefreshProvider(String),
    SaveKey,
    SignInCopilot,
    SignOutCopilot,
    OpenUrl(String),
}

/// What the app pushes to the window while it is open. The poller watches
/// the generation counter and wakes the component when it moves.
#[derive(Default)]
struct SettingsSnapshot {
    generation: u64,
    status: HashMap<String, String>,
    spend: Option<Arc<quotascope_core::spend::Snapshot>>,
    spend_generation: u64,
    spend_loading: bool,
    spend_failed: bool,
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
    fn request_spend(self: &Arc<Self>) {
        let allowed = quotascope_core::settings::with(|s| s.reads_token_spend);
        let generation = {
            let mut state = self.snapshot.lock().unwrap();
            if allowed && state.spend_loading {
                return;
            }
            state.spend_generation = state.spend_generation.wrapping_add(1);
            state.generation += 1;
            state.spend_loading = allowed;
            state.spend_failed = false;
            if !allowed {
                state.spend = None;
                return;
            }
            state.spend_generation
        };
        let shared = self.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(quotascope_core::spend::Snapshot::read_native);
            let allowed = quotascope_core::settings::with(|s| s.reads_token_spend);
            let mut state = shared.snapshot.lock().unwrap();
            if state.spend_generation != generation {
                return;
            }
            state.spend_loading = false;
            state.spend_failed = result.is_err();
            state.spend = if allowed {
                result.ok().map(Arc::new)
            } else {
                None
            };
            state.generation += 1;
        });
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

    /// Something the window shows has changed out from under it.
    pub fn refresh(&self) {
        self.shared.snapshot.lock().unwrap().generation += 1;
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
            shared.snapshot.lock().unwrap().generation += 1;
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
    Accounts,
    Spend,
    Notifications,
    About,
}

impl Page {
    fn tag(self) -> &'static str {
        match self {
            Page::General => "general",
            Page::Accounts => "accounts",
            Page::Spend => "spend",
            Page::Notifications => "notifications",
            Page::About => "about",
        }
    }
    fn from_tag(tag: &str) -> Page {
        match tag {
            "accounts" => Page::Accounts,
            "spend" => Page::Spend,
            "notifications" => Page::Notifications,
            "about" => Page::About,
            _ => Page::General,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Accounts => "Accounts",
            Page::Spend => "Token spend",
            Page::Notifications => "Notifications",
            Page::About => "About",
        }
    }
    fn glyph(self) -> &'static str {
        match self {
            Page::General => "\u{E713}",
            Page::Accounts => "\u{E77B}",
            Page::Spend => "\u{E9D9}",
            Page::Notifications => "\u{EA8F}",
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
    CodexResetCredits,
    Startup,
    Alerts,
    AlertReset,
    AlertFailure,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChoiceKey {
    Dock,
    PanelSize,
    RailSpacing,
    ClockDirection,
    WarningThreshold,
    Interval,
    AlertThreshold,
}

#[derive(Clone, PartialEq)]
enum Message {
    Tick,
    SpendChoice(u8, Option<usize>),
    SpendPage(bool),
    SpendDrill(String),
    SpendRefresh,
    Nav(Option<String>),
    Toggle(ToggleKey, bool),
    ToggleEnabled(usize, bool),
    ExpandAccount(usize),
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
    ImportSession(usize),
    CopilotAuth,
    OpenGitHub,
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
            page: Page::General,
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
                    self.page = Page::General;
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
            }
            Message::Nav(tag) => {
                self.page = tag.as_deref().map(Page::from_tag).unwrap_or(self.page);
                if self.page == Page::Spend {
                    self.shared.request_spend();
                }
                self.revealed_keys.clear();
            }
            Message::SpendRefresh => self.shared.request_spend(),
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
                }
            }
            Message::Search(text) => {
                self.search = text;
                self.revealed_keys.clear();
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
                self.shared.send(SettingsAction::Changed);
                if key == ToggleKey::TokenSpend {
                    self.shared.request_spend();
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
            Message::Choice(key, selected) => {
                Self::apply_choice(key, selected.unwrap_or(0));
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
            Message::Refresh(index) => {
                if let Some(provider) = all_providers().get(index) {
                    self.shared
                        .send(SettingsAction::RefreshProvider(provider.raw().to_string()));
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
                        Page::Accounts,
                        Page::Spend,
                        Page::Notifications,
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
                    View::keyed_fragment([(
                        self.page.tag(),
                        ScrollViewer::new().content(
                            Border::new().padding(Thickness::uniform(28.0)).content(
                                match self.page {
                                    Page::General => self.general_view(context).into(),
                                    Page::Accounts => self.accounts_view(context),
                                    Page::Spend => self.spend_view(context),
                                    Page::Notifications => self.notifications_view(context),
                                    Page::About => self.about_view(context),
                                },
                            ),
                        ),
                    )]),
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
    fn spend_models(&self) -> Vec<String> {
        let state = self.shared.snapshot.lock().unwrap();
        let Some(snapshot) = state.spend.as_ref() else {
            return Vec::new();
        };
        snapshot
            .sources
            .iter()
            .filter(|s| self.spend_source.as_ref().is_none_or(|id| *id == s.id))
            .flat_map(|s| s.ledger.days.iter().flat_map(|d| d.models.keys().cloned()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
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

    fn spend_view(&self, context: &ViewContext<Self>) -> View {
        use quotascope_core::spend::{Group, Sort, Span};
        let settings = quotascope_core::settings::with(|s| s.clone());
        let t = quotascope_core::localization::t;
        let mut content = vec![
            heading("Token spend").into(),
            self.muted("Local records · API value is an estimate, not your bill.")
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
                self.muted("Enable local reading to analyze records on this computer.")
                    .into(),
            );
            return StackPanel::new()
                .spacing(18.0)
                .children((View::keyed_fragment(content.into_iter().enumerate()),));
        }
        let (snapshot, loading, failed) = {
            let state = self.shared.snapshot.lock().unwrap();
            (state.spend.clone(), state.spend_loading, state.spend_failed)
        };
        content.push(row((
            Button::new()
                .is_enabled(!loading)
                .on_click(context.message(Message::SpendRefresh))
                .content(t("Refresh")),
            self.muted(if loading {
                "Reading local records…"
            } else if failed {
                "Unable to read local records."
            } else {
                "Only records available on this computer are included."
            }),
        )));
        let Some(snapshot) = snapshot else {
            return StackPanel::new()
                .spacing(18.0)
                .children((View::keyed_fragment(content.into_iter().enumerate()),));
        };
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
        let analysis = snapshot.analyze(
            settings.spend_span,
            chrono::Local::now().date_naive(),
            self.spend_source.as_deref(),
            self.spend_model.as_deref(),
            self.spend_group,
            self.spend_sort,
            self.spend_descending,
        );
        let total = analysis.total;
        let summary = Grid::new()
            .columns([GridLength::Star(1.0), GridLength::Star(1.0)])
            .column_spacing(24.0)
            .children((
                StackPanel::new().spacing(6.0).children((
                    self.muted("Total tokens"),
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
                    self.muted("API value"),
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
        content.push(self.surface(summary.into()));
        if let Some(rate) = total.cache_hit_rate() {
            content.push(
                self.muted(&format!("{} {:.0}%", t("Cache hit rate"), rate * 100.0))
                    .into(),
            );
        }
        if total.tokens == 0 {
            content.push(
                self.muted("No measured token records in this range.")
                    .into(),
            );
        }
        // Up to 40 calendar bins, each retaining its actual total. No samples
        // are discarded when a long span is selected.
        if !analysis.days.is_empty() {
            let size = analysis.days.len().div_ceil(40).max(1);
            let bins: Vec<_> = analysis
                .days
                .chunks(size)
                .map(|days| days.iter().map(|d| d.measures.tokens).sum::<i64>())
                .collect();
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
            self.muted("* API value excludes tokens without a published price.")
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
                            } else if s.ledger.days.iter().any(|d| d.tokens > 0) {
                                "Native token records"
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
            // Lifecycle requests must wake an idle window even when no
            // account statuses change. Cancelled tasks must also terminate.
            loop {
                std::thread::sleep(Duration::from_millis(250));
                if cancel.is_cancelled()
                    || !watcher.window_ready.load(Ordering::SeqCst)
                    || watcher.show_requested.load(Ordering::SeqCst)
                    || watcher.shutdown_requested.load(Ordering::SeqCst)
                    || watcher.generation() != seen
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
            ToggleKey::CodexResetCredits => s.shows_codex_reset_credits = value,
            ToggleKey::Alerts => s.wants_alerts = value,
            ToggleKey::AlertReset => s.alerts_on_reset = value,
            ToggleKey::AlertFailure => s.alerts_on_failure = value,
            ToggleKey::Startup => {}
        });
        if key == ToggleKey::Startup {
            crate::autostart::set_enabled(value);
        }
    }

    fn apply_choice(key: ChoiceKey, selected: usize) {
        quotascope_core::settings::mutate(|s| match key {
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
        let dock_selected = match s.dock_side.as_str() {
            "left" => 1,
            "top" => 2,
            _ => 0,
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
                            &["Right", "Left", "Top"],
                            dock_selected,
                            context,
                        ),
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
            Provider::ClaudeCode | Provider::Codex | Provider::Grok => self
                .muted(&quotascope_core::localization::t(
                    "Reads the login this tool already saved on this PC.",
                ))
                .into(),
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
                    if matches!(
                        provider,
                        Provider::ClaudeCode
                            | Provider::Codex
                            | Provider::Grok
                            | Provider::Antigravity
                            | Provider::Cursor
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
                ))
                .into(),
        )
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
            ))
            .into()
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
                self.muted(&quotascope_core::localization::t(
                    "No QuotaScope servers, no QuotaScope account, no telemetry. Requests go to the providers you already use and follow the Windows system proxy settings.",
                )),
                Button::new()
                    .on_click(context.message(Message::OpenGitHub))
                    .content("github.com/Lyx721188/QuotaScope"),
            ))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::positive_or_empty;

    #[test]
    fn balance_edits_reject_nonfinite_zero_negative_and_half_typed_amounts() {
        for text in ["NaN", "inf", "-inf", "0", "-1", "12x", "-"] {
            assert!(positive_or_empty(text).is_err(), "{text}");
        }
        assert_eq!(positive_or_empty(" 1.25 "), Ok(Some(1.25)));
        assert_eq!(positive_or_empty("  "), Ok(None));
    }
}
