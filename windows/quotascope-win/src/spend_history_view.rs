use super::*;
impl SettingsApp {
    pub(super) fn token_activity_view(
        &self,
        activity: &quotascope_core::token_activity::TokenActivity,
        context: &ViewContext<Self>,
    ) -> View {
        use chrono::Datelike;
        use quotascope_core::localization::{t, t_fmt};
        let mut rows = vec![section("Token activity").into(), self.muted(t("The past year; follows agent and model filters, independently of the time-range picker.")).into()];
        if activity.days.is_empty() {
            rows.push(self.muted(t("No local records in this selection.")).into());
            return self.surface(
                StackPanel::new()
                    .spacing(10.0)
                    .children((View::keyed_fragment(rows.into_iter().enumerate()),)),
            );
        }
        rows.push(
            self.aligned(
                "Chart",
                ComboBox::new()
                    .automation_name(t("Activity chart"))
                    .items_source(
                        [t("Calendar grid"), t("Weekly bars"), t("Running total")]
                            .into_iter()
                            .map(str::to_string)
                            .collect::<Vec<_>>(),
                    )
                    .selected_index(Some(self.activity_mode))
                    .on_selection_changed(context.callback(Message::ActivityMode))
                    .into(),
            ),
        );
        let weeks = activity.week_totals();
        let columns = weeks.len();
        let first = activity.week_start();
        let blue = Color::argb(255, 0, 120, 212);
        if self.activity_mode == 0 {
            let cells = activity.days.iter().enumerate().map(|(i, day)| {
                let color = match day.tokens {
                    None => Color::argb(45, 128, 128, 128),
                    Some(n) => match activity.step(n) {
                        0 => Color::argb(90, 128, 128, 128),
                        1 => Color::argb(255, 170, 210, 238),
                        2 => Color::argb(255, 100, 170, 220),
                        3 => Color::argb(255, 35, 135, 200),
                        _ => blue,
                    },
                };
                let label = format!(
                    "{} · {}",
                    day.date,
                    day.tokens
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| t("Not recorded").into())
                );
                let column = (day.date.signed_duration_since(first).num_days() / 7) as i32;
                (
                    i,
                    Border::new()
                        .grid_column(column)
                        .grid_row(day.date.weekday().num_days_from_monday() as i32)
                        .height(10.0)
                        .min_height(0.0)
                        .min_width(0.0)
                        .padding(Thickness::uniform(0.0))
                        .background(color)
                        .automation_name(label)
                        .content(View::empty()),
                )
            });
            rows.push(
                Grid::new()
                    .columns(vec![GridLength::Star(1.0); columns])
                    .rows(vec![GridLength::Pixel(10.0); 7])
                    .column_spacing(2.0)
                    .row_spacing(2.0)
                    .children((View::keyed_fragment(cells),))
                    .into(),
            );
        } else if self.activity_mode == 1 {
            let maximum = weeks.iter().flatten().copied().max().unwrap_or(1).max(1) as f64;
            rows.push(
                Grid::new()
                    .columns(vec![GridLength::Star(1.0); columns])
                    .column_spacing(2.0)
                    .height(84.0)
                    .children((View::keyed_fragment(weeks.iter().enumerate().filter_map(
                        |(i, value)| {
                            value.map(|n| {
                                let date = (first + chrono::Days::new((i * 7 + 6) as u64))
                                    .min(activity.end);
                                (
                                    i,
                                    Border::new()
                                        .grid_column(i as i32)
                                        .height((n as f64 / maximum * 84.0).max(2.0))
                                        .min_height(0.0)
                                        .min_width(0.0)
                                        .vertical_alignment(VerticalAlignment::Bottom)
                                        .padding(Thickness::uniform(0.0))
                                        .background(blue)
                                        .automation_name(format!(
                                            "{} — {} · {}",
                                            first + chrono::Days::new((i * 7) as u64),
                                            date,
                                            n
                                        ))
                                        .content(View::empty()),
                                )
                            })
                        },
                    )),))
                    .into(),
            );
        } else {
            let points = activity.cumulative();
            let max = activity.total.max(1) as f64;
            let span = activity
                .end
                .signed_duration_since(activity.start)
                .num_days()
                .max(1) as f64;
            let lines = points.windows(2).enumerate().map(|(i, pair)| {
                let x = |date: chrono::NaiveDate| {
                    date.signed_duration_since(activity.start).num_days() as f64 / span * 560.0
                };
                (
                    i,
                    Line::new()
                        .x1(x(pair[0].0))
                        .y1(84.0 - pair[0].1 as f64 / max * 80.0)
                        .x2(x(pair[1].0))
                        .y2(84.0 - pair[1].1 as f64 / max * 80.0)
                        .stroke(blue)
                        .stroke_thickness(2.0),
                )
            });
            let chart = Canvas::new()
                .width(560.0)
                .height(90.0)
                .children((View::keyed_fragment(lines),));
            rows.push(
                Viewbox::new()
                    .height(90.0)
                    .stretch(Stretch::Fill)
                    .slots([SlotView::new(ViewboxSlot::Child, chart)])
                    .into(),
            );
        }
        let mut month_cells = Vec::new();
        let mut previous = 0;
        for day in &activity.days {
            if day.date.month() != previous {
                previous = day.date.month();
                let column = ((day.date.signed_duration_since(first).num_days() / 7) as usize)
                    .min(columns.saturating_sub(4));
                month_cells.push((
                    month_cells.len(),
                    TextBlock::new()
                        .grid_column(column as i32)
                        .grid_column_span(4)
                        .font_size(10.0)
                        .text(day.date.format("%Y-%m").to_string()),
                ));
            }
        }
        rows.push(
            Grid::new()
                .columns(vec![GridLength::Star(1.0); columns])
                .children((View::keyed_fragment(month_cells),))
                .into(),
        );
        let selected = self
            .activity_date
            .filter(|date| *date >= activity.start && *date <= activity.end)
            .unwrap_or(activity.end);
        let day = activity
            .days
            .iter()
            .find(|d| d.date == selected)
            .and_then(|d| d.tokens);
        rows.push(
            self.aligned(
                "Inspect day",
                ComboBox::new()
                    .automation_name(t("Activity day"))
                    .items_source(
                        activity
                            .days
                            .iter()
                            .map(|d| d.date.to_string())
                            .collect::<Vec<_>>(),
                    )
                    .selected_index(activity.days.iter().position(|d| d.date == selected))
                    .on_selection_changed(context.callback({
                        let days: Vec<_> =
                            activity.days.iter().map(|d| d.date.to_string()).collect();
                        move |index: Option<usize>| {
                            Message::ActivityDate(
                                index.and_then(|i| days.get(i)).cloned().unwrap_or_default(),
                            )
                        }
                    }))
                    .into(),
            ),
        );
        rows.push(
            TextBlock::new()
                .automation_name("activity-total")
                .text(t_fmt(
                    "{tokens} tokens · {days} active days",
                    &[
                        &activity.total.to_string(),
                        &activity.active_days.to_string(),
                    ],
                ))
                .into(),
        );
        rows.push(
            self.muted(&format!(
                "{} · {}",
                selected,
                day.map(|n| format!("{n} tokens"))
                    .unwrap_or_else(|| t("Not recorded").into())
            ))
            .into(),
        );
        rows.push(self.muted(t("Faint cells precede the first record; grey cells are quiet recorded-calendar days. Choose a day to inspect its count.")).into());
        if !activity.partial_sources.is_empty() {
            rows.push(
                self.muted(&format!(
                    "{} {}",
                    t("Incomplete records:"),
                    activity.partial_sources.join(", ")
                ))
                .into(),
            );
        }
        self.surface(
            StackPanel::new()
                .spacing(10.0)
                .children((View::keyed_fragment(rows.into_iter().enumerate()),)),
        )
    }

    pub(super) fn monthly_recap_view(
        &self,
        recap: &quotascope_core::recap::MonthlyRecap,
        months: &[chrono::NaiveDate],
        context: &ViewContext<Self>,
    ) -> View {
        use chrono::Datelike;
        use quotascope_core::localization::{t, t_fmt};
        use quotascope_core::recap::Period;
        let mut rows: Vec<View> = vec![
            section("Usage recaps").into(),
            ComboBox::new()
                .automation_name(t("Recap period"))
                .items_source([t("Daily"), t("Weekly"), t("Monthly"), t("Yearly")])
                .selected_index(Some(match self.recap_period {
                    Period::Day => 0,
                    Period::Week => 1,
                    Period::Month => 2,
                    Period::Year => 3,
                }))
                .on_selection_changed(context.callback(Message::RecapPeriod))
                .into(),
            row((
                Button::new()
                    .on_click(context.message(Message::RecapStep(false)))
                    .content(t("Previous")),
                TextBlock::new().text(format!("{} — {}", recap.month, recap.end)),
                Button::new()
                    .is_enabled(recap.end < chrono::Local::now().date_naive())
                    .on_click(context.message(Message::RecapStep(true)))
                    .content(t("Next")),
                Button::new()
                    .automation_name(t("Show monthly recap"))
                    .on_click(context.message(Message::RecapToggle))
                    .content(t(if self.recap_open {
                        "Close recap"
                    } else {
                        "Open recap"
                    })),
            ))
            .into(),
            if self.recap_period == Period::Month {
                self.aligned(
                    "Month",
                    ComboBox::new()
                        .automation_name(t("Recap month"))
                        .items_source(
                            months
                                .iter()
                                .map(|m| m.format("%Y-%m").to_string())
                                .collect::<Vec<_>>(),
                        )
                        .selected_index(months.iter().position(|m| *m == recap.month))
                        .on_selection_changed(context.callback(Message::RecapMonth))
                        .into(),
                )
            } else {
                View::empty()
            },
        ];
        if !self.recap_open {
            return self.surface(
                StackPanel::new()
                    .spacing(10.0)
                    .children((View::keyed_fragment(rows.into_iter().enumerate()),)),
            );
        }
        let notes = format!(
            "{} — {} · {}{}",
            recap.month,
            recap.through,
            if recap.running {
                t("Period in progress")
            } else {
                t("Completed period")
            },
            recap
                .recorded_from
                .map(|date| format!(" · {} {}", t("Records start:"), date))
                .unwrap_or_default()
        );
        rows.push(self.muted(&notes).into());
        if recap.observed_days == 0 {
            rows.push(self.muted(t("No local records in this selection.")).into());
            return self.surface(
                StackPanel::new()
                    .spacing(12.0)
                    .children((View::keyed_fragment(rows.into_iter().enumerate()),)),
            );
        }
        let total = recap.analysis.total;
        let change = recap
            .daily_change
            .map(|n| format!("{:+.1}%", n * 100.0))
            .unwrap_or_else(|| t("Not recorded").into());
        rows.push(self.surface(StackPanel::new().spacing(8.0).children((section("Your period in numbers"), TextBlock::new().automation_name("recap-total").font_size(30.0).font_weight(FontWeight::BOLD).text(format!("{} tokens", total.tokens)), self.muted(&t_fmt("{tools} AI tools · {active}/{observed} active days · longest streak {streak} days", &[&recap.analysis.rows.len().to_string(), &recap.active_days.to_string(), &recap.observed_days.to_string(), &recap.longest_streak.to_string()])), self.muted(&format!("{} {} · {} {:.1}", t("Daily average versus previous period:"), change, t("Tokens per observed day:"), total.tokens as f64 / recap.observed_days as f64)), self.muted(&notes)))));
        if recap.period == Period::Month {
            let offset = recap.month.weekday().num_days_from_monday() as usize;
            let cells = (1..=recap.end.day()).map(|day| {
                let date = recap.month.with_day(day).unwrap();
                let position = offset + day as usize - 1;
                (
                    day,
                    Border::new()
                        .grid_column((position % 7) as i32)
                        .grid_row((position / 7) as i32)
                        .padding(Thickness::uniform(5.0))
                        .background(Color::argb(35, 0, 120, 212))
                        .content(
                            TextBlock::new()
                                .automation_name(format!("recap-day-{}", date))
                                .text(format!(
                                    "{day}\n{}",
                                    recap
                                        .day_tokens(date)
                                        .map(|n| n.to_string())
                                        .unwrap_or_else(|| "—".into())
                                )),
                        ),
                )
            });
            rows.push(
                Grid::new()
                    .columns(vec![GridLength::Star(1.0); 7])
                    .rows(vec![GridLength::Auto; 6])
                    .column_spacing(4.0)
                    .row_spacing(4.0)
                    .children((View::keyed_fragment(cells),))
                    .into(),
            );
        } else {
            let mut buckets = std::collections::BTreeMap::<String, i64>::new();
            for day in &recap.analysis.days {
                let key = if recap.period == Period::Year {
                    day.date.format("%Y-%m").to_string()
                } else {
                    day.date.to_string()
                };
                *buckets.entry(key).or_default() += day.measures.tokens;
            }
            let max = buckets.values().copied().max().unwrap_or(1).max(1) as f64;
            for (date, tokens) in buckets {
                rows.push(
                    StackPanel::new()
                        .spacing(3.0)
                        .children((
                            self.muted(&format!("{date} · {tokens} tokens")),
                            ProgressBar::new().maximum(max).value(tokens as f64),
                        ))
                        .into(),
                );
            }
        }
        let monthly_usd = self
            .recap_budget
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0);
        let cards = crate::recap_export::available(recap, monthly_usd);
        let selected = cards
            .iter()
            .position(|kind| *kind == self.recap_card)
            .unwrap_or(0);
        let labels = cards
            .iter()
            .map(|kind| t(crate::recap_export::TITLES[*kind]))
            .collect::<Vec<_>>();
        rows.push(
            self.aligned(
                "Share card",
                ComboBox::new()
                    .items_source(labels)
                    .selected_index(Some(selected))
                    .on_selection_changed(context.callback(move |index: Option<usize>| {
                        Message::RecapCard(index.and_then(|index| cards.get(index).copied()))
                    }))
                    .into(),
            ),
        );
        rows.push(row((
            Button::new()
                .on_click(context.message(Message::RecapExport(0)))
                .content(t("Save PNG")),
            Button::new()
                .on_click(context.message(Message::RecapExport(1)))
                .content(t("Copy image")),
            Button::new()
                .on_click(context.message(Message::RecapExport(2)))
                .content(t("Save all cards")),
        )));
        if !self.recap_export_result.is_empty() {
            rows.push(self.muted(&self.recap_export_result).into());
        }
        let mut tool_rows: Vec<View> = vec![section("Tools and models").into()];
        for tool in &recap.analysis.rows {
            tool_rows.push(
                TextBlock::new()
                    .text(format!(
                        "{} · {} tokens · {}",
                        tool.title,
                        tool.measures.tokens,
                        quotascope_core::money::estimate(tool.measures.cost)
                    ))
                    .into(),
            );
        }
        tool_rows.push(self.muted(t("Top models")).into());
        for model in recap.models.iter().take(5) {
            tool_rows.push(
                self.muted(&format!(
                    "{} · {} tokens",
                    model.title, model.measures.tokens
                ))
                .into(),
            );
        }
        if recap.models.len() > 5 {
            tool_rows.push(
                self.muted(
                    &recap.models[5..]
                        .iter()
                        .map(|row| format!("{} ({})", row.title, row.measures.tokens))
                        .collect::<Vec<_>>()
                        .join(", "),
                )
                .into(),
            );
        }
        tool_rows.push(self.muted(&notes).into());
        rows.push(
            self.surface(
                StackPanel::new()
                    .spacing(8.0)
                    .children((View::keyed_fragment(tool_rows.into_iter().enumerate()),)),
            ),
        );
        let rhythm = recap
            .busiest_four_hours()
            .map(|(start, share)| {
                t_fmt(
                    "{percent}% of timed tokens fall between {start}:00 and {end}:00.",
                    &[
                        &format!("{:.1}", share * 100.0),
                        &format!("{start:02}"),
                        &format!("{:02}", (start + 4) % 24),
                    ],
                )
            })
            .unwrap_or_else(|| t("No reliable hourly records in this period.").into());
        let excluded = if recap.excluded_time_sources.is_empty() {
            String::new()
        } else {
            format!(
                "{} {}",
                t("Excluded from timing:"),
                recap.excluded_time_sources.join(", ")
            )
        };
        rows.push(self.surface(StackPanel::new().spacing(8.0).children((
            section("Rhythm of the day"),
            TextBlock::new().text(rhythm),
            self.muted(t(
                "Uses recorded quarter-hour buckets, not exact start or finish times.",
            )),
            self.muted(&excluded),
            self.muted(&notes),
        ))));
        let price = self
            .recap_budget
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n > 0.0);
        let payback = price.and_then(|price| recap.payback(price)).map(|(price, ratio)| t_fmt("Prorated price {price}; public-price equivalent {ratio} times this amount.", &[&quotascope_core::money::estimate(price), &format!("{ratio:.2}")])).unwrap_or_else(|| t("Enter a positive monthly USD price to compare; no subscription price is assumed.").into());
        let rate = recap
            .cache_hit_rate
            .map(|n| format!("{:.1}%", n * 100.0))
            .unwrap_or_else(|| t("Not recorded").into());
        let excluded = if recap.excluded_cache_sources.is_empty() {
            String::new()
        } else {
            format!(
                "{} {}",
                t("Excluded from cache rate:"),
                recap.excluded_cache_sources.join(", ")
            )
        };
        rows.push(self.surface(StackPanel::new().spacing(8.0).children((section("Public-price equivalent"), TextBlock::new().font_size(24.0).text(quotascope_core::money::estimate(total.cost)), self.muted(&format!("{} {} · {} {}", t("Tokens without public prices:"), total.unpriced, t("Tokens without a kind breakdown:"), total.unclassified)), self.muted(t("This is an API-price estimate, not your actual bill. Unpriced work is included in tokens; its money is missing, so the amount is a floor.")), self.muted(&format!("{} {}", t("Measured cache-hit rate:"), rate)), self.muted(&excluded), self.aligned("Monthly subscription price (USD)", TextBox::new().automation_name(t("Monthly subscription price (USD)")).text(self.recap_budget.clone()).placeholder_text("USD").on_text_changed(context.callback(Message::RecapBudget)).into()), self.muted(&payback), self.muted(&notes)))));
        if !recap.partial_sources.is_empty() {
            rows.push(
                self.muted(&format!(
                    "{} {}",
                    t("Incomplete records:"),
                    recap.partial_sources.join(", ")
                ))
                .into(),
            );
        }
        self.surface(
            StackPanel::new()
                .spacing(14.0)
                .children((View::keyed_fragment(rows.into_iter().enumerate()),)),
        )
    }
}
