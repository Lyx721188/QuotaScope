//! One optional background snapshot, released immediately when disabled.
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
#[derive(Default)]
struct State {
    control: Option<Arc<crate::scan::Control>>,
    kept: Option<(Arc<crate::spend::Snapshot>, Instant)>,
}
fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}
pub fn snapshot() -> Option<Arc<crate::spend::Snapshot>> {
    state()
        .lock()
        .unwrap()
        .kept
        .as_ref()
        .filter(|(_, at)| at.elapsed() < Duration::from_secs(300))
        .map(|(s, _)| s.clone())
}
pub fn clear() {
    let mut state = state().lock().unwrap();
    if let Some(control) = &state.control {
        control.cancel();
    }
    state.kept = None;
}
pub fn cancel_running() {
    if let Some(control) = &state().lock().unwrap().control {
        control.cancel();
    }
}
pub fn tick() {
    if !crate::settings::with(|s| s.reads_token_spend && s.background_token_spend) {
        clear();
        return;
    }
    let control = {
        let mut state = state().lock().unwrap();
        if state.control.is_some()
            || state
                .kept
                .as_ref()
                .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(300))
        {
            return;
        }
        let control = Arc::new(crate::scan::Control::default());
        state.control = Some(control.clone());
        control
    };
    std::thread::spawn(move || {
        let deadline_control = control.clone();
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = finished.clone();
        std::thread::spawn(move || {
            for _ in 0..30 {
                std::thread::sleep(Duration::from_secs(1));
                if done.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
            }
            deadline_control.cancel();
        });
        let result = std::panic::catch_unwind(|| {
            crate::spend::Snapshot::read_controlled(control, false, |_| {})
        });
        finished.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut state = state().lock().unwrap();
        state.control = None;
        if crate::settings::with(|s| s.reads_token_spend && s.background_token_spend) {
            if let Ok(Ok(snapshot)) = result {
                state.kept = Some((Arc::new(snapshot), Instant::now()));
            }
        }
    });
}
