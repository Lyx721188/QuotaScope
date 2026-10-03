//! Exercises the real tray command handler and WinUI host in a child process.
//! Uses an isolated, empty profile, so no account requests or user writes occur.
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;

struct RunningApp(Child);
impl Drop for RunningApp {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Search {
    pid: u32,
    tray: Option<HWND>,
    settings: Option<HWND>,
    panel: Option<HWND>,
}
unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> BOOL {
    let search = &mut *(data.0 as *mut Search);
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == search.pid {
        let mut class = [0u16; 256];
        let len = GetClassNameW(hwnd, &mut class);
        if String::from_utf16_lossy(&class[..len as usize]) == "QuotaScopeTrayWindow" {
            search.tray = Some(hwnd);
        }
        if String::from_utf16_lossy(&class[..len as usize]) == "QuotaScopePanel" {
            search.panel = Some(hwnd);
        }
        let mut title = [0u16; 128];
        let len = GetWindowTextW(hwnd, &mut title);
        if String::from_utf16_lossy(&title[..len as usize]).starts_with("QuotaScope ") {
            search.settings = Some(hwnd);
        }
    }
    true.into()
}
fn windows(pid: u32) -> Search {
    let mut search = Search {
        pid,
        tray: None,
        settings: None,
        panel: None,
    };
    unsafe {
        let _ = EnumWindows(Some(find), LPARAM(&mut search as *mut _ as isize));
    }
    search
}
fn wait_for(app: &mut RunningApp, ready: impl Fn(Search) -> bool) {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            app.0.try_wait().unwrap().is_none(),
            "QuotaScope exited unexpectedly"
        );
        if ready(windows(app.0.id())) {
            return;
        }
        assert!(
            Instant::now() < until,
            "Timed out waiting for settings lifecycle"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn stay_alive(app: &mut RunningApp, duration: Duration) {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        assert!(
            app.0.try_wait().unwrap().is_none(),
            "QuotaScope exited while idle"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "launches the real Windows desktop app; run explicitly on an unlocked desktop"]
fn tray_settings_close_and_reopen_reuses_window_and_exits_cleanly() {
    // Raise this above the five-minute data-cache lifetime for a local idle
    // regression. The default keeps the packaged CI check short.
    let idle = Duration::from_secs(
        std::env::var("QUOTASCOPE_TEST_IDLE_SECONDS")
            .map(|value| value.parse::<u64>().expect("idle duration must be seconds"))
            .unwrap_or(2),
    );
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let profile = std::env::temp_dir().join(format!(
        "quotascope-settings-{}-{stamp}",
        std::process::id()
    ));
    let data = profile.join("QuotaScope");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join("settings.json"),
        r#"{"hasRun":true,"enabledAccounts":[],"language":"zh","readsTokenSpend":false}"#,
    )
    .unwrap();
    let executable = std::env::var("QUOTASCOPE_TEST_EXE")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_quotascope").into());
    let mut app = RunningApp(
        Command::new(executable)
            .env("APPDATA", &profile)
            .env("QUOTASCOPE_TEST_INSTANCE", stamp.to_string())
            .spawn()
            .unwrap(),
    );
    wait_for(&mut app, |s| s.tray.is_some());
    assert!(windows(app.0.id()).settings.is_none());
    let tray = windows(app.0.id()).tray.unwrap();
    // The empty test profile has no usage rings. Hide its panel while the
    // idle regression runs so it does not leave a blank dock on the desktop.
    unsafe {
        PostMessageW(Some(tray), WM_COMMAND, WPARAM(1001), LPARAM(0)).unwrap();
    }
    wait_for(&mut app, |s| {
        s.panel
            .is_some_and(|h| unsafe { !IsWindowVisible(h).as_bool() })
    });
    // Recreating a released composition canvas must work repeatedly.
    for _ in 0..3 {
        unsafe {
            PostMessageW(Some(tray), WM_COMMAND, WPARAM(1001), LPARAM(0)).unwrap();
        }
        wait_for(&mut app, |s| {
            s.panel
                .is_some_and(|h| unsafe { IsWindowVisible(h).as_bool() })
        });
        stay_alive(&mut app, Duration::from_millis(300));
        unsafe {
            PostMessageW(Some(tray), WM_COMMAND, WPARAM(1001), LPARAM(0)).unwrap();
        }
        wait_for(&mut app, |s| {
            s.panel
                .is_some_and(|h| unsafe { !IsWindowVisible(h).as_bool() })
        });
    }
    // Starting without --settings must also survive waiting before first open.
    stay_alive(&mut app, idle);
    println!(
        "First Settings request after {} seconds idle",
        idle.as_secs()
    );
    let mut first = None;
    for cycle in 0..3 {
        // WM_COMMAND / 1002 is the command selected by the tray's Settings item.
        unsafe {
            PostMessageW(Some(tray), WM_COMMAND, WPARAM(1002), LPARAM(0)).unwrap();
        }
        wait_for(&mut app, |s| {
            s.settings
                .is_some_and(|h| unsafe { IsWindowVisible(h).as_bool() })
        });
        let hwnd = windows(app.0.id()).settings.unwrap();
        if let Some(original) = first {
            assert_eq!(hwnd, original);
        } else {
            first = Some(hwnd);
        }
        // Preserve the packaged smoke check's 20-second survival window,
        // which catches delayed faults from missing WinUI resources.
        stay_alive(
            &mut app,
            Duration::from_secs(if cycle == 0 { 20 } else { 2 }),
        );
        unsafe {
            PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
        }
        wait_for(&mut app, |s| {
            s.settings
                .is_none_or(|h| unsafe { !IsWindowVisible(h).as_bool() })
        });
    }
    // No accounts are configured: there are no changing status generations
    // to accidentally wake the host and conceal a lost reopen request.
    println!(
        "Settings closed; waiting {} seconds before reopening",
        idle.as_secs()
    );
    stay_alive(&mut app, idle);
    unsafe {
        for _ in 0..3 {
            PostMessageW(Some(tray), WM_COMMAND, WPARAM(1002), LPARAM(0)).unwrap();
        }
    }
    wait_for(&mut app, |s| {
        s.settings
            .is_some_and(|h| unsafe { IsWindowVisible(h).as_bool() })
    });
    assert_eq!(windows(app.0.id()).settings, first);
    stay_alive(&mut app, Duration::from_secs(2));
    // The second-launch path must foreground the same Settings window too.
    unsafe {
        PostMessageW(first, WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
    }
    wait_for(&mut app, |s| {
        s.settings
            .is_some_and(|h| unsafe { !IsWindowVisible(h).as_bool() })
    });
    let exe = std::env::var("QUOTASCOPE_TEST_EXE")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_quotascope").into());
    assert!(Command::new(exe)
        .env("APPDATA", &profile)
        .env("QUOTASCOPE_TEST_INSTANCE", stamp.to_string())
        .status()
        .unwrap()
        .success());
    wait_for(&mut app, |s| {
        s.settings
            .is_some_and(|h| unsafe { IsWindowVisible(h).as_bool() })
    });
    assert_eq!(windows(app.0.id()).settings, first);
    stay_alive(&mut app, Duration::from_secs(2));
    // Exit with Settings hidden exercises the quiet shutdown path as well.
    unsafe {
        PostMessageW(first, WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
    }
    wait_for(&mut app, |s| {
        s.settings
            .is_some_and(|h| unsafe { !IsWindowVisible(h).as_bool() })
    });
    unsafe {
        PostMessageW(Some(tray), WM_COMMAND, WPARAM(1004), LPARAM(0)).unwrap();
    }
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = app.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < until,
            "Tray exit failed to stop the WinUI host"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !data.join("last-readings.json").exists(),
        "Empty profile fetched accounts"
    );
    println!("Idle reopen, repeated tray requests, second launch and hidden exit passed");
    assert_eq!(
        profile.canonicalize().unwrap().parent(),
        Some(std::env::temp_dir().canonicalize().unwrap().as_path())
    );
    std::fs::remove_dir_all(profile).unwrap();
}
