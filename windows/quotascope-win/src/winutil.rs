//! Small Win32 conveniences shared by every window in the app.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITORINFO, MONITORINFOEXW,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_APP_TRAY: u32 = WM_APP + 1;
/// The detail flyout -> panel: "pointer came in" / "pointer left".
pub const WM_APP_CARD: u32 = WM_APP + 3;

pub fn process_metrics() -> quotascope_core::diagnostics::ProcessMetrics {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};
    let mut result = quotascope_core::diagnostics::ProcessMetrics::default();
    unsafe {
        let process = GetCurrentProcess();
        let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
        let size = std::mem::size_of_val(&counters) as u32;
        counters.cb = size;
        if GetProcessMemoryInfo(
            process,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            size,
        )
        .is_ok()
        {
            result.private_bytes = Some(counters.PrivateUsage as u64);
            result.working_set_bytes = Some(counters.WorkingSetSize as u64);
        }
        let mut handles = 0;
        if GetProcessHandleCount(process, &mut handles).is_ok() {
            result.handles = Some(handles);
        }
    }
    result
}

/// The instance handle, resolved once.
pub fn hinstance() -> windows::Win32::Foundation::HINSTANCE {
    unsafe { HINSTANCE(GetModuleHandleW(None).expect("GetModuleHandleW").0) }
}

/// Opts the whole process into per-monitor DPI awareness. A screen-edge
/// panel that renders itself must do its own scaling.
pub fn set_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// The work area of the monitor a point sits on, in physical pixels.
pub fn monitor_work_at(x: i32, y: i32) -> (RECT, RECT) {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        monitor_rects(monitor)
    }
}

/// (full rect, work rect) of a monitor.
pub fn monitor_rects(monitor: HMONITOR) -> (RECT, RECT) {
    unsafe {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        (info.rcMonitor, info.rcWork)
    }
}

/// The monitor's device name, which is how a display is remembered across
/// launches — the counterpart of the macOS app remembering by UUID.
pub fn monitor_name(monitor: HMONITOR) -> String {
    unsafe {
        let mut info = MONITORINFOEXW {
            monitorInfo: MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        let _ = GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        );
        String::from_utf16_lossy(
            &info.szDevice[..info.szDevice.iter().position(|c| *c == 0).unwrap_or(0)],
        )
    }
}

pub fn monitor_from_hwnd(hwnd: HWND) -> HMONITOR {
    unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) }
}

/// One-shot single-instance check. A second launch reports and exits.
fn instance_object_name(name: &str) -> Vec<u16> {
    // Debug lifecycle tests use an empty profile and a separate namespace,
    // so they cannot foreground or stop the user's installed application.
    #[cfg(debug_assertions)]
    if let Ok(id) = std::env::var("QUOTASCOPE_TEST_INSTANCE") {
        if !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return wide(&format!("{name}.Test.{id}"));
        }
    }
    wide(name)
}

pub fn acquire_single_instance() -> bool {
    let name = instance_object_name("QuotaScope.Windows.SingleInstance");
    unsafe {
        let handle =
            windows::Win32::System::Threading::CreateMutexW(None, false, PCWSTR(name.as_ptr()));
        match handle {
            Ok(handle) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                    return false;
                }
                // HANDLE is Copy, so "leaking" is automatic: never
                // closing it is what keeps the mutex alive for the
                // process lifetime.
                true
            }
            Err(_) => true,
        }
    }
}

/// A second launch is not an accident to be dismissed: opening the app
/// while it is already running opens Settings — the same answer the macOS
/// app gives on reopen. The mutex says "already running"; this named event
/// carries the request across to the first instance.
pub fn signal_open_settings() {
    use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
    unsafe {
        let name = instance_object_name("QuotaScope.Windows.OpenSettings");
        if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(name.as_ptr())) {
            let _ = SetEvent(event);
            let _ = windows::Win32::Foundation::CloseHandle(event);
        }
    }
}

/// Creates the named event and waits on it in the background, turning each
/// signal into a message on `tx`. The handle lives for the process
/// lifetime on purpose.
pub fn listen_for_open_settings(tx: std::sync::mpsc::Sender<()>) {
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
    std::thread::spawn(move || unsafe {
        let name = instance_object_name("QuotaScope.Windows.OpenSettings");
        let event = CreateEventW(None, false, false, PCWSTR(name.as_ptr()));
        let Ok(event) = event else { return };
        loop {
            let wait = WaitForSingleObject(event, INFINITE);
            if wait != WAIT_OBJECT_0 {
                break;
            }
            if tx.send(()).is_err() {
                break;
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(event);
    });
}

/// A NUL-terminated UTF-16 buffer; the vec outlives the calls it feeds.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Keeps a wide buffer alive for the duration of a call that borrows it.
pub struct TempWide {
    buffer: Vec<u16>,
}

impl TempWide {
    pub fn new(text: &str) -> Self {
        TempWide { buffer: wide(text) }
    }

    pub fn as_ptr(&self) -> PCWSTR {
        PCWSTR::from_raw(self.buffer.as_ptr())
    }
}

/// The default arrow cursor, loaded lazily.
pub fn arrow_cursor() -> HCURSOR {
    unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() }
}

/// The embedded application icon — resource #1, written by build.rs — at
/// the class-icon size. Window classes share this so Alt+Tab and any
/// chrome show the real mark.
pub fn app_class_icon() -> HICON {
    unsafe {
        let size = GetSystemMetrics(SM_CXICON).max(32);
        let handle = LoadImageW(
            Some(hinstance()),
            PCWSTR(1usize as _),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        )
        .unwrap_or_default();
        HICON(handle.0)
    }
}

pub use windows::Win32::Foundation::RECT;

/// The real Win11 window materials, in three DWM attributes: Mica behind
/// the whole client area, the frame's dark variant matching the system
/// appearance, and Windows' own corner rounding. Set **once**, at window
/// creation.
pub fn apply_system_backdrop(hwnd: HWND, dark: bool) {
    unsafe {
        let margins = windows::Win32::UI::Controls::MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        let _ = windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea(hwnd, &margins);
        // DWMSBT_MAINWINDOW is Mica.
        let backdrop: i32 = 2;
        let _ = windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            windows::Win32::Graphics::Dwm::DWMWA_SYSTEMBACKDROP_TYPE,
            &backdrop as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
        set_backdrop_dark(hwnd, dark);
        // DWMWCP_ROUND: the corner radius Windows itself applies to
        // surfaces — not a radius this app draws.
        let corner: i32 = 2;
        let _ = windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            windows::Win32::Graphics::Dwm::DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

/// Updates only the dark-mode flag of the backdrop. Re-applying the whole
/// material set on a theme change provokes DWM into rebuilding its
/// composition for the window — which the DirectComposition visual tree
/// does not survive — so theme switches touch just this flag, which Mica
/// honours live.
pub fn set_backdrop_dark(hwnd: HWND, dark: bool) {
    unsafe {
        let dark_flag: i32 = dark as i32;
        let _ = windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            windows::Win32::Graphics::Dwm::DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_flag as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}
