use super::*;
#[derive(Default)]
pub(super) struct ConnectionState {
    generation: u64,
    running: bool,
    context: String,
    outcome: String,
    replies: Vec<quotascope_core::connection_diagnostics::Reply>,
}
impl Shared {
    pub(super) fn release_connections(&self) {
        let mut state = self.snapshot.lock().unwrap();
        for check in state.connections.values_mut() {
            check.generation = check.generation.wrapping_add(1);
            check.replies.clear();
            check.outcome.clear();
        }
    }
    pub(super) fn check_connection(self: &Arc<Self>, provider: Provider) {
        if !self.alive.load(Ordering::SeqCst) {
            return;
        }
        let account = quotascope_core::model::AccountKey::primary(provider);
        let credential = quotascope_core::provider_sites::current_context(&account);
        let generation = {
            let mut state = self.snapshot.lock().unwrap();
            let check = state.connections.entry(provider).or_default();
            if check.running {
                return;
            }
            check.generation = check.generation.wrapping_add(1);
            check.running = true;
            check.replies.clear();
            check.outcome.clear();
            check.context = credential.clone();
            let generation = check.generation;
            state.generation = state.generation.wrapping_add(1);
            generation
        };
        let shared = self.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                quotascope_core::connection_diagnostics::capture(|| {
                    let keys = quotascope_core::providers::KeyRing::load();
                    let services = quotascope_core::providers::Services::new();
                    services
                        .for_provider(provider)
                        .map(|service| service.fetch(&keys))
                })
            });
            let mut state = shared.snapshot.lock().unwrap();
            let check = state.connections.entry(provider).or_default();
            check.running = false;
            if check.generation == generation
                && shared.alive.load(Ordering::SeqCst)
                && credential == quotascope_core::provider_sites::current_context(&account)
            {
                match result {
                    Ok((Some(usage), replies)) => {
                        check.outcome = match usage.state {
                            quotascope_core::model::State::Unavailable(reason) => {
                                quotascope_core::localization::t(reason.message()).into()
                            }
                            _ => quotascope_core::localization::t("Connection succeeded").into(),
                        };
                        check.replies = replies;
                    }
                    _ => {
                        check.outcome =
                            quotascope_core::localization::t("Connection check failed").into()
                    }
                }
            }
            state.generation = state.generation.wrapping_add(1);
        });
    }
}
impl SettingsApp {
    pub(super) fn connection_view(&self, provider: Provider, context: &ViewContext<Self>) -> View {
        use quotascope_core::localization::t;
        let state = self.shared.snapshot.lock().unwrap();
        let check = state.connections.get(&provider).filter(|c| {
            c.context
                == quotascope_core::provider_sites::current_context(
                    &quotascope_core::model::AccountKey::primary(provider),
                )
        });
        let mut rows:Vec<View>=vec![section("Connection diagnostics").into(),self.muted(t("Replies are masked, kept in memory until the next check, and excluded from diagnostic exports. Local CLI routes may have no HTTP reply.")).into(),Button::new().is_enabled(!check.is_some_and(|c|c.running)).on_click(context.message(Message::ConnectionCheck(provider))).content(t("Check connection")).into()];
        if let Some(check) = check {
            rows.push(self.muted(&check.outcome).into());
            for reply in &check.replies {
                rows.push(
                    TextBlock::new()
                        .text(format!(
                            "{} {} · {} · {}\n{}",
                            reply.method,
                            reply.address,
                            reply
                                .status
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| "Network error".into()),
                            reply.content_type,
                            reply.body
                        ))
                        .text_wrapping(TextWrapping::Wrap)
                        .font_size(11.0)
                        .into(),
                );
            }
        }
        StackPanel::new()
            .spacing(6.0)
            .children((View::keyed_fragment(rows.into_iter().enumerate()),))
    }
}
