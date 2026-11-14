//! Asynchronous browser imports, guarded against changed credentials and sites.
use super::*;

impl Shared {
    pub(super) fn invalidate_session_imports(&self, provider: Option<Provider>) {
        let mut state = self.snapshot.lock().unwrap();
        let selected = provider.map(|p| format!("browser:{}", p.raw()));
        state.console_operations.retain(|key, _| {
            !key.starts_with("browser:") || selected.as_ref().is_some_and(|id| id != key)
        });
        state.generation = state.generation.wrapping_add(1);
    }

    pub(super) fn import_session(self: &Arc<Self>, provider: Provider) {
        let Some(spec) = quotascope_core::providers::session_spec(provider) else {
            return;
        };
        let account = quotascope_core::model::AccountKey::primary(provider);
        let credential = quotascope_core::provider_sites::current_context(&account);
        let browser = quotascope_core::settings::with(|s| s.import_browser.clone());
        let id = format!("browser:{}", provider.raw());
        let generation = {
            let mut state = self.snapshot.lock().unwrap();
            if state
                .console_operations
                .get(&id)
                .is_some_and(|job| job.running)
            {
                return;
            }
            state.generation = state.generation.wrapping_add(1);
            let generation = state.generation;
            state.console_operations.insert(
                id.clone(),
                ConsoleOperation {
                    generation,
                    running: true,
                    message: Some(
                        quotascope_core::localization::t("Importing browser session…").into(),
                    ),
                },
            );
            generation
        };
        let shared = self.clone();
        std::thread::spawn(move || {
            let found = std::panic::catch_unwind(|| {
                quotascope_core::browser_cookies::session_for_browser(
                    spec.hosts,
                    spec.cookies,
                    &browser,
                )
                .and_then(|mut found| {
                    if provider == Provider::OpenDesign {
                        found.header =
                            quotascope_core::providers::open_design::normalize(&found.header)?;
                    }
                    Some(found)
                })
            })
            .ok()
            .flatten();
            let mut state = shared.snapshot.lock().unwrap();
            if !state
                .console_operations
                .get(&id)
                .is_some_and(|job| job.generation == generation)
            {
                return;
            }
            let valid = shared.alive.load(Ordering::SeqCst)
                && quotascope_core::settings::with(|s| s.import_browser == browser)
                && credential == quotascope_core::provider_sites::current_context(&account);
            let (saved, message) = if !valid {
                (
                    false,
                    quotascope_core::localization::t(
                        "Browser import cancelled because the account or browser choice changed.",
                    )
                    .into(),
                )
            } else if let Some(found) = found {
                let slot = quotascope_core::provider_sites::slot(provider.raw());
                quotascope_core::secrets::set_key(&slot, &found.header);
                let saved = quotascope_core::secrets::key_for(&slot).as_deref()
                    == Some(found.header.as_str());
                let message = if saved {
                    quotascope_core::localization::t_fmt(
                        "Imported the session from {browser}.",
                        &[quotascope_core::localization::t(found.browser.name())],
                    )
                } else {
                    quotascope_core::localization::t("Could not save the imported session.").into()
                };
                (saved, message)
            } else {
                (false, quotascope_core::localization::t("No readable browser session was found. Sign in to the selected browser. For protected Chrome or Edge cookies, select All browsers and connect OpenCLI Browser Bridge, or paste a Cookie header.").into())
            };
            let job = state.console_operations.get_mut(&id).unwrap();
            job.running = false;
            job.message = Some(message);
            state.generation = state.generation.wrapping_add(1);
            drop(state);
            if saved {
                shared.send(SettingsAction::SaveKey);
                shared.send(SettingsAction::RefreshProvider(provider.raw().into()));
            }
        });
    }
}

impl SettingsApp {
    pub(super) fn session_import_state(&self, provider: Provider) -> (bool, String) {
        self.shared
            .snapshot
            .lock()
            .unwrap()
            .console_operations
            .get(&format!("browser:{}", provider.raw()))
            .map(|job| (job.running, job.message.clone().unwrap_or_default()))
            .unwrap_or_default()
    }
}
