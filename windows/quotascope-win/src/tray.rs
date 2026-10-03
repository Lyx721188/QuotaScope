//! The tray icon and its menu — the Windows counterpart of the macOS
//! status item. Left click opens the graphical dashboard; right click opens the
//! menu. Balloon notifications ride the same icon.

use std::sync::mpsc::Sender;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::winutil;

pub enum TrayCommand {
    TogglePanel,
    OpenSettings,
    OpenDashboard,
    RefreshAll,
    OpenUsagePage(&'static str),
    Exit,
}

const ID_TOGGLE: u32 = 1001;
const ID_SETTINGS: u32 = 1002;
const ID_REFRESH: u32 = 1003;
const ID_EXIT: u32 = 1004;
const ID_DASHBOARD: u32 = 1005;
const ID_USAGE_BASE: u32 = 2000;

pub struct DashboardEntry {
    pub title: String,
    pub summary: String,
    pub usage_page: Option<&'static str>,
}

pub struct TrayIcon {
    pub hwnd: HWND,
    data: NOTIFYICONDATAW,
    /// Whether the icon is currently in the tray. `hides_tray_icon` removes
    /// it and puts it back without recreating the window the menu and the
    /// balloons live on.
    added: bool,
    commands: Sender<TrayCommand>,
    /// The app's poll tick: the tray window carries the slow timer, and its
    /// firings become store polls.
    poll: Option<Sender<()>>,
    dashboard: Vec<DashboardEntry>,
}

/// The tray's icon: the embedded application mark when it loads, the old
/// runtime-drawn QuotaScope ring when it somehow doesn't.
fn build_icon() -> HICON {
    // The application icon, embedded by build.rs as resource #1 — the real
    // mark, sized for the tray's own small-icon metric.
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON).max(16);
        let handle = LoadImageW(
            Some(winutil::hinstance()),
            windows::core::PCWSTR(1usize as _),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        )
        .unwrap_or_default();
        if !handle.is_invalid() {
            return HICON(handle.0);
        }
    }
    // A 32×32 ring drawn with GDI into a colour bitmap. Direct2D would be
    // nicer, but the tray wants an HICON, and four GDI calls make one.
    unsafe {
        let size = 32i32;
        let hdc = windows::Win32::Graphics::Gdi::CreateCompatibleDC(None);
        let bmi = windows::Win32::Graphics::Gdi::BITMAPINFO {
            bmiHeader: windows::Win32::Graphics::Gdi::BITMAPINFOHEADER {
                biSize: std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>()
                    as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: windows::Win32::Graphics::Gdi::BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bitmap) = windows::Win32::Graphics::Gdi::CreateDIBSection(
            Some(hdc),
            &bmi,
            windows::Win32::Graphics::Gdi::DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        ) else {
            let _ = windows::Win32::Graphics::Gdi::DeleteDC(hdc);
            return HICON::default();
        };
        let old = windows::Win32::Graphics::Gdi::SelectObject(hdc, bitmap.into());

        // Transparent background, then the ring: green stroke, square caps
        // drawn as a thick circle with a hole.
        let pixels = bits as *mut [u8; 4];
        let cx = 15.5f32;
        let cy = 15.5f32;
        let outer = 12.5f32;
        let inner = 7.5f32;
        for y in 0..size as usize {
            for x in 0..size as usize {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let alpha = if d <= outer && d >= inner { 255 } else { 0 };
                let fade = if d > outer - 1.5 {
                    ((outer - d) * 170.0).clamp(0.0, 255.0) as u32
                } else if d < inner + 1.5 {
                    ((d - inner) * 170.0).clamp(0.0, 255.0) as u32
                } else {
                    255
                };
                *pixels.add(y * size as usize + x) = [
                    0,   // blue
                    230, // green (0.90)
                    140, // red (0.55)
                    ((alpha as f32 * fade as f32) / 255.0) as u8,
                ];
            }
        }
        let _ = windows::Win32::Graphics::Gdi::SelectObject(hdc, old);
        let _ = windows::Win32::Graphics::Gdi::DeleteDC(hdc);

        let mut icon_info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: bitmap,
            hbmColor: bitmap,
        };
        // A mask bitmap is required: a plain all-zero one suffices for an
        // alpha icon.
        let mask = windows::Win32::Graphics::Gdi::CreateBitmap(size, size, 1, 1, None);
        icon_info.hbmMask = mask;
        let icon = CreateIconIndirect(&icon_info).unwrap_or_default();
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(bitmap.into());
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(mask.into());
        icon
    }
}

impl TrayIcon {
    pub fn new(commands: Sender<TrayCommand>) -> Box<TrayIcon> {
        let class_name = w!("QuotaScopeTrayWindow");
        unsafe {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(tray_wndproc),
                lpszClassName: class_name,
                hInstance: winutil::hinstance(),
                hIcon: winutil::app_class_icon(),
                ..Default::default()
            };
            RegisterClassW(&wc);
        }

        let mut tray = Box::new(TrayIcon {
            hwnd: HWND::default(),
            data: NOTIFYICONDATAW::default(),
            added: false,
            commands,
            poll: None,
            dashboard: Vec::new(),
        });

        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class_name,
                w!("QuotaScope"),
                WS_OVERLAPPEDWINDOW & !WS_VISIBLE,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(winutil::hinstance()),
                None,
            )
            .expect("tray window");
            tray.hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*tray as *const TrayIcon as isize);
            tray.set_global_shortcuts(quotascope_core::settings::with(|s| s.global_shortcuts));

            let mut data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                uCallbackMessage: winutil::WM_APP_TRAY,
                hIcon: build_icon(),
                ..Default::default()
            };
            let tip: Vec<u16> = "QuotaScope".encode_utf16().collect();
            data.szTip[..tip.len()].copy_from_slice(&tip);
            tray.added = Shell_NotifyIconW(NIM_ADD, &mut data).as_bool();
            tray.data = data;
        }
        tray
    }

    /// Puts the icon in the tray, or takes it out. The setting's promise is
    /// kept honestly: the menu and the balloons go with the icon, and the
    /// way back is the next launch — of the app, or of Settings itself.
    pub fn set_hidden(&mut self, hidden: bool) {
        unsafe {
            if hidden {
                if self.added {
                    let _ = Shell_NotifyIconW(NIM_DELETE, &mut self.data);
                    self.added = false;
                }
            } else if !self.added {
                let mut data = self.data;
                data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
                if Shell_NotifyIconW(NIM_ADD, &mut data).as_bool() {
                    self.data = data;
                    self.added = true;
                }
            }
        }
    }

    /// Balloons need the icon; with it hidden they are dropped rather than
    /// silently re-adding the icon the reader asked to remove.
    pub fn shows_icon(&self) -> bool {
        self.added
    }

    pub fn set_poll(&mut self, poll: Sender<()>) {
        self.poll = Some(poll);
    }

    pub fn set_global_shortcuts(&self, enabled: bool) {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            RegisterHotKey, UnregisterHotKey, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
        };
        unsafe {
            for (id, key) in [(1, 0x79_u32), (2, 0x7A), (3, 0x7B)] {
                let _ = UnregisterHotKey(Some(self.hwnd), id);
                if enabled {
                    let _ = RegisterHotKey(
                        Some(self.hwnd),
                        id,
                        MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                        key,
                    );
                }
            }
        }
    }

    pub fn set_dashboard(&mut self, entries: Vec<DashboardEntry>) {
        self.dashboard = entries;
    }

    pub fn show_balloon(&mut self, title: &str, text: &str, warning: bool) {
        if !self.added {
            return;
        }
        unsafe {
            let mut data = self.data;
            data.uFlags = NIF_INFO;
            let title_w: Vec<u16> = title.encode_utf16().take(63).collect();
            let text_w: Vec<u16> = text.encode_utf16().take(255).collect();
            data.szInfoTitle[..title_w.len()].copy_from_slice(&title_w);
            data.szInfo[..text_w.len()].copy_from_slice(&text_w);
            data.dwInfoFlags = if warning {
                windows::Win32::UI::Shell::NIIF_WARNING
            } else {
                windows::Win32::UI::Shell::NIIF_INFO
            };
            let _ = Shell_NotifyIconW(NIM_MODIFY, &mut data);
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        self.set_global_shortcuts(false);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &mut self.data);
            let _ = KillTimer(Some(self.hwnd), 2);
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
            let _ = DestroyIcon(self.data.hIcon);
        }
    }
}

unsafe extern "system" fn tray_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if state == 0 {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let tray = &mut *(state as *mut TrayIcon);

    match msg {
        WM_HOTKEY => {
            let command = match wparam.0 {
                1 => Some(TrayCommand::TogglePanel),
                2 => Some(TrayCommand::OpenDashboard),
                3 => Some(TrayCommand::RefreshAll),
                _ => None,
            };
            if let Some(command) = command {
                let _ = tray.commands.send(command);
            }
            LRESULT(0)
        }
        m if m == winutil::WM_APP_TRAY => {
            let event = (lparam.0 & 0xFFFF) as u32;
            match event {
                WM_LBUTTONUP => {
                    let _ = tray.commands.send(TrayCommand::OpenDashboard);
                    LRESULT(0)
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    if tray.shows_icon() {
                        show_menu(hwnd, &tray.dashboard);
                    }
                    LRESULT(0)
                }
                _ => LRESULT(0),
            }
        }
        WM_TIMER => {
            if let Some(poll) = tray.poll.as_ref() {
                let _ = poll.send(());
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as u32;
            let command = match id {
                ID_TOGGLE => Some(TrayCommand::TogglePanel),
                ID_SETTINGS => Some(TrayCommand::OpenSettings),
                ID_REFRESH => Some(TrayCommand::RefreshAll),
                ID_EXIT => Some(TrayCommand::Exit),
                ID_DASHBOARD => Some(TrayCommand::OpenDashboard),
                _ => id
                    .checked_sub(ID_USAGE_BASE)
                    .and_then(|index| tray.dashboard.get(index as usize))
                    .and_then(|entry| entry.usage_page)
                    .map(TrayCommand::OpenUsagePage),
            };
            if let Some(command) = command {
                let _ = tray.commands.send(command);
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn show_menu(hwnd: HWND, dashboard: &[DashboardEntry]) {
    let menu = match CreatePopupMenu() {
        Ok(menu) => menu,
        Err(_) => return,
    };
    for entry in dashboard {
        // Ampersands are mnemonics in native menus; account names are text.
        let text =
            winutil::wide(&format!("{} · {}", entry.title, entry.summary).replace('&', "&&"));
        let _ = AppendMenuW(
            menu,
            MF_STRING | MF_GRAYED,
            0,
            windows::core::PCWSTR(text.as_ptr()),
        );
    }
    if !dashboard.is_empty() {
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, windows::core::PCWSTR::null());
    }
    if dashboard.iter().any(|entry| entry.usage_page.is_some()) {
        if let Ok(pages) = CreatePopupMenu() {
            for (index, entry) in dashboard.iter().enumerate() {
                if entry.usage_page.is_some() && index < (u16::MAX as u32 - ID_USAGE_BASE) as usize
                {
                    let text = winutil::wide(&entry.title.replace('&', "&&"));
                    let _ = AppendMenuW(
                        pages,
                        MF_STRING,
                        ID_USAGE_BASE as usize + index,
                        windows::core::PCWSTR(text.as_ptr()),
                    );
                }
            }
            let text = winutil::wide(quotascope_core::localization::t("Open usage page"));
            if AppendMenuW(
                menu,
                MF_STRING | MF_POPUP,
                pages.0 as usize,
                windows::core::PCWSTR(text.as_ptr()),
            )
            .is_err()
            {
                let _ = DestroyMenu(pages);
            }
        }
    }
    // Each UTF-16 buffer must outlive its AppendMenuW, so they are all
    // materialised before any menu item is appended.
    let entries: [(u32, Option<&str>); 7] = [
        (
            ID_DASHBOARD,
            Some(quotascope_core::localization::t("Dashboard")),
        ),
        (
            ID_TOGGLE,
            Some(quotascope_core::localization::t("Show panel")),
        ),
        (
            ID_REFRESH,
            Some(quotascope_core::localization::t("Refresh all")),
        ),
        (0, None),
        (
            ID_SETTINGS,
            Some(quotascope_core::localization::t("Settings")),
        ),
        (0, None),
        (ID_EXIT, Some(quotascope_core::localization::t("Exit"))),
    ];
    let buffers: Vec<Vec<u16>> = entries
        .iter()
        .map(|(_, text)| text.map(crate::winutil::wide).unwrap_or_default())
        .collect();
    for ((id, text), buffer) in entries.iter().zip(&buffers) {
        match text {
            Some(_) => {
                let _ = AppendMenuW(
                    menu,
                    MF_STRING,
                    *id as usize,
                    windows::core::PCWSTR::from_raw(buffer.as_ptr()),
                );
            }
            None => {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, windows::core::PCWSTR::null());
            }
        }
    }

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    // The menu's window must be foreground for TrackPopupMenu to dismiss
    // on an outside click; the classic tray-menu dance.
    let _ = SetForegroundWindow(hwnd);
    let _ = TrackPopupMenuEx(
        menu,
        (TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RIGHTALIGN).0 as u32,
        pt.x,
        pt.y,
        hwnd,
        Some(std::ptr::null::<TPMPARAMS>()),
    );
    let _ = DestroyMenu(menu);
}
