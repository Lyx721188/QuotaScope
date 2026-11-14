//! Account-scoped, cancellable histories. Results never outlive their credential.
use super::*;
use quotascope_core::{history::HistoryRead, model::AccountKey};

#[derive(Default)]
pub(super) struct AccountHistoryState {
    generation: u64,
    context: String,
    attempted: Option<std::time::Instant>,
    control: Option<Arc<quotascope_core::scan::Control>>,
    reading: Option<Arc<HistoryRead>>,
}

fn local(provider: Provider) -> bool {
    matches!(
        provider,
        Provider::ClaudeCode | Provider::Codex | Provider::Antigravity
    ) || quotascope_core::history::local_source(provider).is_some()
}

impl Shared {
    pub(super) fn release_histories(&self, provider: Option<Provider>) {
        let mut state = self.snapshot.lock().unwrap();
        for (key, history) in &mut state.account_histories {
            if provider.is_none_or(|p| {
                AccountKey::from_id(key).is_some_and(|account| account.provider == p)
            }) {
                if let Some(control) = &history.control {
                    control.cancel();
                }
                history.reading = None;
                history.attempted = None;
                history.generation = history.generation.wrapping_add(1);
            }
        }
        state.generation = state.generation.wrapping_add(1);
    }

    pub(super) fn request_history(self: &Arc<Self>, provider: Provider, force: bool) {
        self.request_history_for(AccountKey::primary(provider), force);
    }
    pub(super) fn request_history_for(self: &Arc<Self>, account: AccountKey, force: bool) {
        let provider = account.provider;
        if !provider.provides_history()
            || !self.alive.load(Ordering::SeqCst)
            || (local(provider) && !quotascope_core::settings::with(|s| s.reads_token_spend))
        {
            return;
        }
        let credential = quotascope_core::provider_sites::current_context(&account);
        let (generation, control) = {
            let mut state = self.snapshot.lock().unwrap();
            let history = state.account_histories.entry(account.id()).or_default();
            if history.context != credential {
                if let Some(control) = &history.control {
                    control.cancel();
                }
                history.generation = history.generation.wrapping_add(1);
                history.reading = None;
                history.attempted = None;
                history.context = credential.clone();
            }
            // Wait for the cancelled scan to exit before admitting another one.
            if history.control.is_some()
                || (!force
                    && history
                        .attempted
                        .is_some_and(|at| at.elapsed() < Duration::from_secs(300)))
            {
                return;
            }
            let control = Arc::new(quotascope_core::scan::Control::default());
            history.control = Some(control.clone());
            history.attempted = Some(std::time::Instant::now());
            let generation = history.generation;
            state.generation = state.generation.wrapping_add(1);
            (generation, control)
        };
        let shared = self.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                quotascope_core::scan::run(
                    control.clone(),
                    3,
                    |_| {},
                    || {
                        if force && local(provider) {
                            quotascope_core::ledger::invalidate_memory();
                        }
                        quotascope_core::history::read_account(&account)
                    },
                )
            });
            let mut state = shared.snapshot.lock().unwrap();
            let history = state.account_histories.entry(account.id()).or_default();
            history.control = None;
            if history.generation == generation
                && shared.alive.load(Ordering::SeqCst)
                && credential == quotascope_core::provider_sites::current_context(&account)
                && (!local(provider) || quotascope_core::settings::with(|s| s.reads_token_spend))
            {
                history.reading = Some(Arc::new(match result {
                    Ok(Ok(reading)) => reading,
                    _ => HistoryRead::Failed(quotascope_core::model::Unavailability::Unreachable),
                }));
            }
            state.generation = state.generation.wrapping_add(1);
        });
    }
}

impl SettingsApp {
    pub(super) fn quota_progress(&self, fraction: f64) -> View {
        if !quotascope_core::settings::with(|s| s.progress_bars == "segmented") {
            return ProgressBar::new()
                .maximum(100.0)
                .value(fraction.clamp(0.0, 1.0) * 100.0)
                .into();
        }
        let accent = crate::theme::panel::accent();
        let color = Color::argb(
            255,
            (accent.r * 255.0) as u8,
            (accent.g * 255.0) as u8,
            (accent.b * 255.0) as u8,
        );
        Grid::new()
            .height(6.0)
            .columns(vec![GridLength::Star(1.0); 48])
            .column_spacing(2.0)
            .children((View::keyed_fragment((0..48).map(|index| {
                let lit = (fraction.clamp(0.0, 1.0) * 48.0 - index as f64).clamp(0.0, 1.0);
                (
                    index as usize,
                    Grid::new()
                        .grid_column(index)
                        .columns([GridLength::Star(lit), GridLength::Star(1.0 - lit)])
                        .children((
                            Border::new().background(color),
                            Border::new()
                                .grid_column(1)
                                .background(Color::argb(35, 128, 128, 128)),
                        )),
                )
            })),))
    }

    pub(super) fn avatar_controls(&self, id: &str, context: &ViewContext<Self>) -> View {
        use quotascope_core::localization::t;
        let config = quotascope_core::settings::with(|s| {
            s.robot_avatars.get(id).cloned().unwrap_or_default()
        });
        let choice =
            |kind, label: &str, options: &[&str], selected| {
                let account = id.to_string();
                self.aligned(
                    label,
                    ComboBox::new()
                        .automation_name(t(label))
                        .items_source(options.iter().map(|s| t(s)).collect::<Vec<_>>())
                        .selected_index(Some(selected))
                        .on_selection_changed(context.callback(move |index| {
                            Message::AvatarChoice(account.clone(), kind, index)
                        }))
                        .into(),
                )
            };
        let mut rows = vec![choice(
            0,
            "Ring mark",
            &["Provider logo", "Agent Robot Avatar"],
            usize::from(config.enabled),
        )];
        if config.enabled {
            rows.push(choice(
                1,
                "Personality",
                &["Automatic", "Calm", "Curious", "Lively"],
                match config.personality.as_str() {
                    "calm" => 1,
                    "curious" => 2,
                    "lively" => 3,
                    _ => 0,
                },
            ));
            rows.push(choice(
                2,
                "Head shape",
                &["Original", "Squarer", "Rounder"],
                match config.shape.as_str() {
                    "square" => 1,
                    "round" => 2,
                    _ => 0,
                },
            ));
            let color_id = id.to_string();
            let save_id = color_id.clone();
            rows.push(
                self.aligned(
                    "Robot color",
                    TextBox::new()
                        .text(
                            self.addresses
                                .get(&format!("__robot-{id}"))
                                .cloned()
                                .unwrap_or(config.color),
                        )
                        .placeholder_text("#RRGGBB")
                        .on_text_changed(
                            context
                                .callback(move |text| Message::AvatarColor(color_id.clone(), text)),
                        )
                        .into(),
                ),
            );
            rows.push(
                Button::new()
                    .on_click(context.message(Message::SaveAvatarColor(save_id)))
                    .content(t("Save color (empty for automatic)")),
            );
        }
        StackPanel::new()
            .spacing(6.0)
            .children((View::keyed_fragment(rows.into_iter().enumerate()),))
    }
    pub(super) fn model_group_controls(
        &self,
        provider: Provider,
        context: &ViewContext<Self>,
    ) -> View {
        use quotascope_core::localization::t;
        let id = provider.raw().to_string();
        let settings = quotascope_core::settings::with(Clone::clone);
        let split = settings.split_accounts.contains(&id);
        let toggle_id = id.clone();
        let mut rows: Vec<View> = vec![self.aligned(
            "One ring per model group",
            ToggleSwitch::new()
                .automation_name(t("One ring per model group"))
                .is_on(split)
                .on_toggled(
                    context.callback(move |on| Message::SplitAccount(toggle_id.clone(), on)),
                )
                .into(),
        )];
        if split {
            let state = self.shared.snapshot.lock().unwrap();
            if let Some(usage) = state.readings.get(&id) {
                let groups: std::collections::BTreeSet<_> = usage
                    .windows
                    .iter()
                    .filter_map(|w| w.scope.clone())
                    .collect();
                for group in groups.into_iter().take(3) {
                    let on = settings
                        .shown_model_groups
                        .get(&id)
                        .is_none_or(|chosen| chosen.contains(&group));
                    let key = id.clone();
                    let name = group.clone();
                    rows.push(
                        self.aligned(
                            &group,
                            ToggleSwitch::new()
                                .automation_name(&group)
                                .is_on(on)
                                .on_toggled(context.callback(move |on| {
                                    Message::ModelGroup(key.clone(), name.clone(), on)
                                }))
                                .into(),
                        ),
                    );
                }
            }
            rows.push(
                self.muted(t("The last visible model group stays enabled."))
                    .into(),
            );
        }
        StackPanel::new()
            .spacing(6.0)
            .children((View::keyed_fragment(rows.into_iter().enumerate()),))
    }
    pub(super) fn account_history_view(
        &self,
        provider: Provider,
        context: &ViewContext<Self>,
    ) -> View {
        self.account_history_for(&AccountKey::primary(provider), context)
    }
    pub(super) fn account_history_for(
        &self,
        account: &AccountKey,
        context: &ViewContext<Self>,
    ) -> View {
        let provider = account.provider;
        use quotascope_core::localization::t;
        if !provider.provides_history() {
            return View::empty();
        }
        if local(provider) && !quotascope_core::settings::with(|s| s.reads_token_spend) {
            return StackPanel::new()
                .spacing(6.0)
                .children((
                    section("Usage history"),
                    self.muted(t(
                        "Enable local reading to analyze records on this computer.",
                    )),
                    Button::new()
                        .on_click(context.message(Message::Toggle(ToggleKey::TokenSpend, true)))
                        .content(t("Enable local reading")),
                ))
                .into();
        }
        let (reading, running) = {
            let state = self.shared.snapshot.lock().unwrap();
            let history = state.account_histories.get(&account.id());
            (
                history
                    .filter(|h| {
                        h.context == quotascope_core::provider_sites::current_context(account)
                    })
                    .and_then(|h| h.reading.clone()),
                history.is_some_and(|h| h.control.is_some()),
            )
        };
        let mut rows: Vec<View> = vec![
            section("Usage history").into(),
            Button::new()
                .is_enabled(!running)
                .on_click(context.message(Message::HistoryRefreshAccount(account.clone())))
                .content(t("Refresh"))
                .into(),
        ];
        let today = chrono::Local::now().date_naive();
        match reading.as_deref() {
            Some(HistoryRead::Credits(credits)) => {
                rows.push(
                    self.muted(t("Credits before discounts · credits deducted"))
                        .into(),
                );
                for (label, days) in [("Today", 1), ("Last 7 days", 7), ("Last 31 days", 31)] {
                    let (used, charged) = credits.total(days);
                    rows.push(
                        self.muted(&format!(
                            "{}: {} · {}",
                            t(label),
                            quotascope_core::qoder_history::number(used),
                            quotascope_core::qoder_history::number(charged)
                        ))
                        .into(),
                    );
                }
                let max = credits
                    .days
                    .iter()
                    .map(|day| day.used)
                    .fold(1.0_f64, f64::max);
                for day in &credits.days {
                    rows.push(
                        StackPanel::new()
                            .spacing(2.0)
                            .children((
                                self.muted(&format!(
                                    "{}  {} · {}",
                                    day.date,
                                    quotascope_core::qoder_history::number(day.used),
                                    quotascope_core::qoder_history::number(day.charged)
                                )),
                                ProgressBar::new().maximum(max).value(day.used),
                            ))
                            .into(),
                    );
                }
            }
            Some(HistoryRead::Answered {
                ledger,
                account_wide,
                actual_costs,
                currency,
            }) => {
                rows.push(
                    self.muted(t(if *account_wide {
                        "Provider statistics across machines"
                    } else {
                        "Local records on this computer"
                    }))
                    .into(),
                );
                if ledger.has_partial_records {
                    rows.push(self.muted(t("Counts may be incomplete.")).into());
                }
                for (label, days) in [("Today", 1), ("Last 7 days", 7), ("Last 30 days", 30)] {
                    let start = today - chrono::Days::new(days - 1);
                    let selected: Vec<_> = ledger
                        .days
                        .iter()
                        .filter(|d| d.date >= start && d.date <= today)
                        .collect();
                    let tokens: i64 = selected.iter().map(|d| d.tokens).sum();
                    let cost: f64 = selected.iter().map(|d| d.cost).sum();
                    let amount = if *actual_costs {
                        currency
                            .as_ref()
                            .map(|c| format!(" · {cost:.2} {c}"))
                            .unwrap_or_default()
                    } else if !*account_wide {
                        format!(
                            " · {} {}",
                            t("Estimated API value"),
                            quotascope_core::money::estimate(cost)
                        )
                    } else {
                        String::new()
                    };
                    rows.push(
                        self.muted(&format!("{}: {} tokens{}", t(label), tokens, amount))
                            .into(),
                    );
                }
                let start = today - chrono::Days::new(29);
                let max = ledger
                    .days
                    .iter()
                    .filter(|d| d.date >= start && d.date <= today)
                    .map(|d| d.tokens)
                    .max()
                    .unwrap_or(1)
                    .max(1) as f64;
                if let Some(first) = ledger.earliest.filter(|date| *date <= today) {
                    let mut date = first.max(start);
                    while date <= today {
                        let tokens = ledger
                            .days
                            .iter()
                            .find(|day| day.date == date)
                            .map(|day| day.tokens)
                            .unwrap_or(0);
                        rows.push(
                            StackPanel::new()
                                .spacing(2.0)
                                .children((
                                    self.muted(&format!("{date}  {tokens} tokens")),
                                    ProgressBar::new()
                                        .minimum(0.0)
                                        .maximum(max)
                                        .value(tokens as f64),
                                ))
                                .into(),
                        );
                        date = date + chrono::Days::new(1);
                    }
                }
                if ledger.days.is_empty() {
                    rows.push(self.muted(t("No usage records in this period.")).into());
                }
                if !ledger.unpriced_models.is_empty() {
                    rows.push(self.muted(t("Some models have no known price; their tokens are included without an estimate.")).into());
                }
            }
            Some(HistoryRead::Failed(reason)) => rows.push(
                self.muted(quotascope_core::localization::t(reason.message()))
                    .into(),
            ),
            Some(HistoryRead::NotConfigured) => rows.push(
                self.muted(t(
                    "Import a session or configure the account to read history.",
                ))
                .into(),
            ),
            None => rows.push(
                self.muted(t(if running {
                    "Loading…"
                } else {
                    "Refresh to read history."
                }))
                .into(),
            ),
        }
        StackPanel::new()
            .spacing(6.0)
            .children((View::keyed_fragment(rows.into_iter().enumerate()),))
            .into()
    }
}
