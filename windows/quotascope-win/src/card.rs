//! The detail card, ported from `UsageDetailCard`.
//!
//! The card is no longer a self-painted bubble inside the panel window —
//! it is the content of its own Mica flyout, the way a WinUI flyout sits on
//! the surface beneath it. So this module draws **content only**: the
//! header, the limit rows, the footnote. The window's name has the top row
//! to itself; spent and reset pair on the line below the bar.
//!
//! Layout units: everything is inset from the flyout's origin by
//! `card::PADDING` on all sides.

use quotascope_core::model::{ProviderUsage, State, Unavailability};
use std::collections::HashMap;
use windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT_NORMAL;

use crate::d2d::{Painter, Rgba};
use crate::geometry::{card, Metrics};
use crate::theme::panel;
use quotascope_core::model::usage_tint;

#[derive(Clone)]
pub struct CardData {
    pub usage: ProviderUsage,
    /// What the card calls it: the account's label.
    pub title: String,
    pub monogram: String,
    /// The provider's mark, keyed like the ring's — the header draws it in
    /// place of the monogram when known.
    pub icon: Option<String>,
    pub shows_remaining: bool,
    pub shows_forecast: bool,
    /// Where a figure stops being calm and starts warning: the reader's own
    /// line, not a constant.
    pub warning_fraction: f64,
    /// Per window id, the value estimate the transcript ledger supports —
    /// "Estimated value ≈$220 · ≈$57 used". Absent wherever the inputs
    /// cannot support the figure.
    pub value_lines: HashMap<String, String>,
    pub detailed: bool,
    pub history_enabled: bool,
    pub history: Option<quotascope_core::history::HistoryRead>,
    pub prompt_cache: Option<quotascope_core::prompt_cache::CacheReading>,
    pub codex_details: Option<quotascope_core::codex_account::AccountDetails>,
    pub shows_codex_reset_credits: bool,
}

fn account_notes(data: &CardData) -> Vec<String> {
    use quotascope_core::codex_account::ResetCredits;
    use quotascope_core::localization::{t, t_fmt};
    if data.usage.provider() != quotascope_core::model::Provider::Codex {
        return Vec::new();
    }
    let mut lines = Vec::new();
    if data.shows_codex_reset_credits {
        match data.codex_details.as_ref().map(|d| &d.reset_credits) {
            Some(ResetCredits::Available {
                count,
                next_expiry_ms,
            }) => {
                lines.push(t_fmt("Reset credits: {count}", &[&count.to_string()]));
                if *count > 0 {
                    if let Some(at) = next_expiry_ms {
                        lines.push(t_fmt(
                            "Next expiry: {time}",
                            &[&quotascope_core::timeutil::reset_text(*at)],
                        ));
                    }
                }
            }
            Some(ResetCredits::CodexMissing) => {
                lines.push(t("Install Codex CLI to read reset credits.").into())
            }
            Some(ResetCredits::Unreported) => lines.push(t("Reset credits: not available.").into()),
            None => lines.push(t("Reading reset credits…").into()),
        }
    }
    if data.detailed {
        if let Some(usage) = data.codex_details.as_ref().and_then(|d| d.usage.as_ref()) {
            if let Some(tokens) = usage.lifetime_tokens {
                lines.push(t_fmt(
                    "Account lifetime: {tokens} tokens",
                    &[&short_tokens(tokens)],
                ));
            }
            if let Some(tokens) = usage.peak_daily_tokens {
                lines.push(t_fmt("Peak day: {tokens} tokens", &[&short_tokens(tokens)]));
            }
            if let Some(days) = usage.current_streak_days {
                lines.push(t_fmt("Current streak: {days} days", &[&days.to_string()]));
            }
            if let Some(days) = usage.longest_streak_days {
                lines.push(t_fmt("Longest streak: {days} days", &[&days.to_string()]));
            }
        }
    }
    lines
}

fn detail_header(data: &CardData) -> Vec<String> {
    if !data.detailed {
        return Vec::new();
    }
    let mut lines = Vec::new();
    if let Some(plan) = data.usage.plan.as_ref().filter(|p| !p.is_empty()) {
        lines.push(plan.clone());
    }
    if matches!(data.usage.state, State::Live) {
        if let Some(at) = data.usage.observed_at {
            lines.push(quotascope_core::localization::t_fmt(
                "Updated {time}",
                &[&quotascope_core::timeutil::relative_text(at)],
            ));
        }
    }
    if lines.is_empty() {
        lines
    } else {
        vec![lines.join(" · ")]
    }
}

fn recent_days(
    ledger: &quotascope_core::ledger::UsageLedger,
) -> Vec<&quotascope_core::ledger::LedgerDay> {
    let today = chrono::Local::now().date_naive();
    let start = today - chrono::Days::new(29);
    ledger
        .days
        .iter()
        .filter(|day| day.date >= start && day.date <= today)
        .collect()
}

#[cfg(test)]
fn has_history_chart(data: &CardData) -> bool {
    data.history_enabled
        && matches!(&data.history, Some(quotascope_core::history::HistoryRead::Answered { ledger, .. }) if !recent_days(ledger).is_empty())
}

struct ActivityFigure {
    label: &'static str,
    tokens: i64,
    cost: Option<f64>,
}

struct Activity {
    heading: &'static str,
    figures: Vec<ActivityFigure>,
    notes: Vec<String>,
    footer: Option<&'static str>,
    priced: bool,
}

impl Activity {
    fn height(&self, m: &Metrics) -> f64 {
        m.s(card::CONTENT_SPACING + 1.0 + 10.0 + 13.0)
            + if self.figures.is_empty() {
                0.0
            } else {
                m.s(10.0
                    + 13.0
                    + 2.0
                    + 17.0
                    + if self.priced { 15.0 } else { 0.0 }
                    + 10.0
                    + 30.0
                    + 4.0
                    + 11.0)
            }
            + self.notes.len() as f64 * m.s(8.0 + 14.0)
            + if self.footer.is_some() {
                m.s(10.0 + 13.0)
            } else {
                0.0
            }
    }
}

fn short_tokens(tokens: i64) -> String {
    let n = tokens.max(0) as f64;
    for (base, suffix) in [(1e9, "B"), (1e6, "M"), (1e3, "K")] {
        if n >= base {
            return format!("{:.1}{suffix}", n / base);
        }
    }
    tokens.max(0).to_string()
}

fn activity(data: &CardData) -> Option<Activity> {
    use quotascope_core::history::HistoryRead;
    use quotascope_core::localization::{t, t_fmt};
    if !data.history_enabled {
        return None;
    }
    let account_wide = matches!(
        &data.history,
        Some(HistoryRead::Answered {
            account_wide: true,
            ..
        })
    ) || matches!(
        data.usage.provider(),
        quotascope_core::model::Provider::Zai | quotascope_core::model::Provider::GlmCoding
    );
    let mut section = Activity {
        heading: t(if account_wide {
            "Whole account"
        } else {
            "On this PC"
        }),
        figures: Vec::new(),
        notes: Vec::new(),
        footer: None,
        priced: false,
    };
    let lines = &mut section.notes;
    match &data.history {
        None => lines.push(t("Reading history…").into()),
        Some(HistoryRead::NotConfigured) => lines.push(t("Add an API key to read history.").into()),
        Some(HistoryRead::Failed(reason)) => {
            lines.push(t("Couldn't read the history.").into());
            lines.push(reason.message().to_string());
        }
        Some(HistoryRead::Answered {
            ledger,
            account_wide,
        }) => {
            if !account_wide {
                if let Some(rate) =
                    ledger.cache_hit_rate_calendar(30, chrono::Local::now().date_naive())
                {
                    lines.push(t_fmt(
                        "Cache hit rate: {percent}%",
                        &[&format!("{:.0}", rate * 100.0)],
                    ));
                }
            }
            let days = recent_days(ledger);
            if days.is_empty() {
                lines.push(
                    t(if *account_wide {
                        "No history in the last 30 days."
                    } else {
                        "No readable local history in the last 30 days."
                    })
                    .into(),
                );
            } else {
                let cost: f64 = days.iter().map(|day| day.cost).sum();
                section.priced = !account_wide && cost > 0.0;
                let today = chrono::Local::now().date_naive();
                for (label, span) in [("Today", 1), ("7 days", 7), ("30 days", 30)] {
                    let cutoff = today - chrono::Days::new(span - 1);
                    let selected: Vec<_> = days.iter().filter(|day| day.date >= cutoff).collect();
                    let tokens = selected
                        .iter()
                        .fold(0_i64, |sum, day| sum.saturating_add(day.tokens));
                    let cost: f64 = selected.iter().map(|day| day.cost).sum();
                    section.figures.push(ActivityFigure {
                        label: t(label),
                        tokens,
                        cost: (section.priced && cost > 0.0).then_some(cost),
                    });
                }
                let mut models: std::collections::BTreeMap<&str, i64> = Default::default();
                for day in &days {
                    for (model, tokens) in &day.models {
                        let n = models.entry(model).or_default();
                        *n = n.saturating_add(*tokens);
                    }
                }
                if let Some((model, _)) = models.into_iter().max_by_key(|(_, tokens)| *tokens) {
                    let name = ledger
                        .model_names
                        .get(model)
                        .map(String::as_str)
                        .unwrap_or(model);
                    lines.push(t_fmt("Most used: {model}", &[name]));
                }
                let unpriced = days
                    .iter()
                    .fold(0_i64, |sum, day| sum.saturating_add(day.unpriced_tokens));
                if !account_wide && unpriced > 0 {
                    lines.push(t_fmt(
                        "{tokens} tokens have no published price",
                        &[&short_tokens(unpriced)],
                    ));
                }
            }
            section.footer = Some(t(if *account_wide {
                "Provider statistics · all machines · no price breakdown"
            } else {
                "Local records · API value is an estimate, not a bill"
            }));
        }
    }
    if data.usage.provider() == quotascope_core::model::Provider::ClaudeCode {
        let now = quotascope_core::timeutil::now_ms();
        if let Some(reading) = &data.prompt_cache {
            let alive = reading.alive(now);
            if let Some(first) = alive.first() {
                section.notes.push(t_fmt(
                    "Prompt cache: {count} chats",
                    &[&alive.len().to_string()],
                ));
                let minutes =
                    ((first.lapse.expires_at().saturating_sub(now) + 59_999) / 60_000).max(1);
                let tier = if first.lapse.lifetime_ms == 3_600_000 {
                    "1h"
                } else {
                    "5m"
                };
                section.notes.push(t_fmt(
                    "{tier} cache · expires in {minutes} min",
                    &[tier, &minutes.to_string()],
                ));
            } else if reading.latest_lapsed(now).is_some() {
                section.notes.push(t("Prompt cache has expired.").into());
                section
                    .notes
                    .push(t("The next message will rebuild the cache.").into());
            } else {
                section.notes.push(t("No active prompt cache.").into());
                section
                    .notes
                    .push(t("No recent cache tier was reported.").into());
            }
        } else {
            section.notes.push(t("Reading prompt cache…").into());
            section
                .notes
                .push(t("Cache lifetime comes from local records.").into());
        }
    }
    Some(section)
}

fn draw_detail_line(
    painter: &Painter,
    m: &Metrics,
    x: f64,
    cy: &mut f64,
    width: f64,
    text: &str,
    alpha: f32,
) -> windows::core::Result<()> {
    *cy += m.s(4.0);
    let brush = painter.brush(faded(panel::palette().text_secondary, alpha))?;
    painter.text(
        text,
        crate::d2d::rect(x as f32, *cy as f32, width as f32, m.s(13.0) as f32),
        m.s(card::FOOTNOTE_FONT) as f32,
        DWRITE_FONT_WEIGHT_NORMAL,
        &brush,
        0,
        1,
    );
    *cy += m.s(13.0);
    Ok(())
}

fn draw_history(
    painter: &Painter,
    m: &Metrics,
    x: f64,
    cy: &mut f64,
    width: f64,
    data: &CardData,
    alpha: f32,
) -> windows::core::Result<()> {
    let Some(section) = activity(data) else {
        return Ok(());
    };
    let palette = panel::palette();
    let primary = painter.brush(faded(palette.text_primary, alpha))?;
    let secondary = painter.brush(faded(palette.text_secondary, alpha))?;
    let muted = painter.brush(faded(palette.text_disabled, alpha))?;
    let rule = painter.brush(faded(palette.track.with_alpha(0.10), alpha))?;
    *cy += m.s(card::CONTENT_SPACING);
    painter.fill_rounded_rect(
        crate::d2d::rect(x as f32, *cy as f32, width as f32, m.s(1.0) as f32),
        0.0,
        &rule,
    );
    *cy += m.s(1.0 + 10.0);
    painter.text(
        section.heading,
        crate::d2d::rect(
            x as f32,
            *cy as f32,
            (width * 0.65) as f32,
            m.s(13.0) as f32,
        ),
        m.s(10.5) as f32,
        windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT_MEDIUM,
        &secondary,
        0,
        1,
    );
    painter.text(
        "token",
        crate::d2d::rect(
            (x + width * 0.65) as f32,
            *cy as f32,
            (width * 0.35) as f32,
            m.s(13.0) as f32,
        ),
        m.s(10.0) as f32,
        DWRITE_FONT_WEIGHT_NORMAL,
        &muted,
        2,
        1,
    );
    *cy += m.s(13.0);
    if !section.figures.is_empty() {
        *cy += m.s(10.0);
        let column = (width - m.s(16.0)) / 3.0;
        for (index, figure) in section.figures.iter().enumerate() {
            let left = x + index as f64 * (column + m.s(8.0));
            painter.text(
                figure.label,
                crate::d2d::rect(left as f32, *cy as f32, column as f32, m.s(13.0) as f32),
                m.s(10.5) as f32,
                DWRITE_FONT_WEIGHT_NORMAL,
                &secondary,
                0,
                1,
            );
            painter.text(
                &short_tokens(figure.tokens),
                crate::d2d::rect(
                    left as f32,
                    (*cy + m.s(15.0)) as f32,
                    column as f32,
                    m.s(17.0) as f32,
                ),
                m.s(14.0) as f32,
                windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT_SEMI_BOLD,
                &primary,
                0,
                1,
            );
            if let Some(cost) = figure.cost {
                painter.text(
                    &format!("≈${cost:.2}"),
                    crate::d2d::rect(
                        left as f32,
                        (*cy + m.s(34.0)) as f32,
                        column as f32,
                        m.s(13.0) as f32,
                    ),
                    m.s(10.0) as f32,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    &secondary,
                    0,
                    1,
                );
            }
        }
        *cy += m.s(32.0 + if section.priced { 15.0 } else { 0.0 } + 10.0);
        let Some(quotascope_core::history::HistoryRead::Answered { ledger, .. }) = &data.history
        else {
            unreachable!()
        };
        let days = recent_days(ledger);
        let today = chrono::Local::now().date_naive();
        let first = today - chrono::Days::new(29);
        let maximum = days.iter().map(|day| day.tokens).max().unwrap_or(1).max(1) as f64;
        let slot_width = width / 30.0;
        let bar_width = slot_width * 0.65;
        for index in 0..30 {
            let date = first + chrono::Days::new(index);
            let tokens = days
                .iter()
                .find(|day| day.date == date)
                .map(|day| day.tokens)
                .unwrap_or(0)
                .max(0);
            let height = if tokens > 0 {
                (m.s(30.0) * tokens as f64 / maximum).max(bar_width)
            } else {
                m.s(1.5)
            };
            let brush = painter.brush(faded(
                palette.text_primary.with_alpha(if date == today {
                    0.85
                } else if tokens > 0 {
                    0.32
                } else {
                    0.12
                }),
                alpha,
            ))?;
            painter.fill_rounded_rect(
                crate::d2d::rect(
                    (x + index as f64 * slot_width) as f32,
                    (*cy + m.s(30.0) - height) as f32,
                    bar_width as f32,
                    height as f32,
                ),
                (bar_width / 2.0).min(height / 2.0) as f32,
                &brush,
            );
        }
        *cy += m.s(30.0 + 4.0);
        painter.text(
            &first.format("%m/%d").to_string(),
            crate::d2d::rect(x as f32, *cy as f32, (width / 2.0) as f32, m.s(11.0) as f32),
            m.s(9.0) as f32,
            DWRITE_FONT_WEIGHT_NORMAL,
            &muted,
            0,
            1,
        );
        painter.text(
            &today.format("%m/%d").to_string(),
            crate::d2d::rect(
                (x + width / 2.0) as f32,
                *cy as f32,
                (width / 2.0) as f32,
                m.s(11.0) as f32,
            ),
            m.s(9.0) as f32,
            DWRITE_FONT_WEIGHT_NORMAL,
            &muted,
            2,
            1,
        );
        *cy += m.s(11.0);
    }
    for note in &section.notes {
        *cy += m.s(8.0);
        painter.text(
            note,
            crate::d2d::rect(x as f32, *cy as f32, width as f32, m.s(14.0) as f32),
            m.s(10.5) as f32,
            DWRITE_FONT_WEIGHT_NORMAL,
            &secondary,
            0,
            1,
        );
        *cy += m.s(14.0);
    }
    if let Some(footer) = section.footer {
        *cy += m.s(10.0);
        painter.text(
            footer,
            crate::d2d::rect(x as f32, *cy as f32, width as f32, m.s(13.0) as f32),
            m.s(9.5) as f32,
            DWRITE_FONT_WEIGHT_NORMAL,
            &muted,
            0,
            1,
        );
        *cy += m.s(13.0);
    }
    Ok(())
}

/// The card's total size for a reading of this shape — what the flyout
/// window has to be, in design units.
pub fn body_size(m: &Metrics, data: &CardData) -> (f64, f64) {
    let w = m.s(card::WIDTH) + m.s(card::PADDING) * 2.0;
    let mut h = m.s(card::PADDING * 2.0 + card::HEADER_HEIGHT)
        + detail_header(data).len() as f64 * m.s(4.0 + 13.0);
    match &data.usage.state {
        State::Unavailable(reason) => {
            h += m.s(card::CONTENT_SPACING) + message_height(m, &unavailable_message(data, reason));
        }
        State::Live | State::Stale => {
            for window in &data.usage.windows {
                h += m.s(card::CONTENT_SPACING) + card::row_height(m, false);
                if data.value_lines.contains_key(&window.id) {
                    h += m.s(card::ROW_INTERNAL_SPACING + card::ROW_TEXT_LINE_HEIGHT);
                }
                if forecast_line(window, data).is_some() {
                    h += m.s(card::ROW_INTERNAL_SPACING + card::ROW_TEXT_LINE_HEIGHT);
                }
            }
            if data.usage.windows.is_empty() {
                h += m.s(card::CONTENT_SPACING)
                    + if data.usage.credit_balance.is_some() {
                        m.s(card::ROW_TEXT_LINE_HEIGHT)
                    } else {
                        message_height(m, quotascope_core::localization::t("No limits reported."))
                    };
            }
        }
    }
    h += account_notes(data).len() as f64 * m.s(card::ROW_INTERNAL_SPACING + 4.0 + 13.0);
    if let Some(section) = activity(data) {
        h += section.height(m);
    }
    if matches!(data.usage.state, State::Stale) {
        h += m.s(card::CONTENT_SPACING + card::ROW_TEXT_LINE_HEIGHT);
    }
    (w, h)
}

fn unavailable_message(data: &CardData, reason: &Unavailability) -> String {
    if *reason == Unavailability::NotOnWindows {
        quotascope_core::localization::t(
            data.usage
                .provider()
                .windows_gap()
                .unwrap_or(reason.message()),
        )
        .to_string()
    } else {
        reason.message().to_string()
    }
}

fn message_height(m: &Metrics, text: &str) -> f64 {
    wrapped_lines(text, m.s(card::WIDTH), m.s(card::MESSAGE_FONT)).len() as f64
        * m.s(card::MESSAGE_FONT)
        * 1.3
}

/// Scales a colour's alpha by the card's entrance fade.
fn faded(c: Rgba, alpha: f32) -> Rgba {
    c.with_alpha(c.a * alpha)
}

/// Draws the card's content with its padding inset from `origin`. `alpha`
/// is the entrance fade: 0 invisible, 1 fully there.
pub fn draw_card(
    painter: &Painter,
    m: &Metrics,
    origin: (f64, f64),
    data: &CardData,
    alpha: f32,
) -> windows::core::Result<()> {
    let palette = panel::palette();
    let inset_x = origin.0 + m.s(card::PADDING);
    let inset_w = m.s(card::WIDTH);
    let mut cy = origin.1 + m.s(card::PADDING);

    // Header: the mark and the title, one line, always — the card's height
    // is budgeted, and a wrapped header would slice off against the edge.
    let title_brush = painter.brush(faded(palette.text_primary, alpha))?;
    let mark_size = m.s(card::HEADER_ICON) as f32;
    let mark_y = (cy + (m.s(card::HEADER_HEIGHT) - m.s(card::HEADER_ICON)) / 2.0) as f32;
    let mut drew_mark = false;
    if let Some(key) = &data.icon {
        if let Some(bitmap) = crate::assets::bitmap_for(painter.engine, key) {
            let dest = crate::d2d::rect(inset_x as f32, mark_y, mark_size, mark_size);
            if let Ok(ctx) = windows::core::Interface::cast::<
                windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
            >(painter.rt)
            {
                unsafe {
                    ctx.DrawBitmap(
                        &bitmap,
                        Some(&dest),
                        alpha,
                        windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                        None,
                        None,
                    );
                }
                drew_mark = true;
            }
        }
    }
    if !drew_mark {
        painter.text(
            &data.monogram,
            crate::d2d::rect(
                inset_x as f32,
                cy as f32,
                (m.s(card::HEADER_ICON) + 4.0) as f32,
                m.s(card::HEADER_HEIGHT) as f32,
            ),
            (m.s(card::HEADER_ICON) * 0.8) as f32,
            windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT_SEMI_BOLD,
            &title_brush,
            0,
            1,
        );
    }
    painter.text(
        &quotascope_core::localization::t_fmt("{provider} Usage", &[&data.title]),
        crate::d2d::rect(
            (inset_x + m.s(card::HEADER_ICON) + 8.0) as f32,
            cy as f32,
            (inset_w - m.s(card::HEADER_ICON) - 8.0) as f32,
            m.s(card::HEADER_HEIGHT) as f32,
        ),
        m.s(card::TITLE_FONT) as f32,
        windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT_SEMI_BOLD,
        &title_brush,
        0,
        1,
    );
    cy += m.s(card::HEADER_HEIGHT);
    for line in detail_header(data) {
        draw_detail_line(painter, m, inset_x, &mut cy, inset_w, &line, alpha)?;
    }

    match &data.usage.state {
        State::Unavailable(reason) => {
            cy += m.s(card::CONTENT_SPACING);
            let message = unavailable_message(data, reason);
            let message_brush = painter.brush(faded(palette.text_secondary, alpha))?;
            draw_wrapped(
                painter,
                &message,
                inset_x,
                &mut cy,
                inset_w,
                m.s(card::MESSAGE_FONT),
                &message_brush,
            );
        }
        State::Live | State::Stale => {
            for window in &data.usage.windows {
                cy += m.s(card::CONTENT_SPACING);
                draw_progress_row(painter, m, inset_x, &mut cy, inset_w, window, data, alpha)?;
            }

            // A card with only a title reads as a card that failed to load:
            // DeepSeek on "balance only" reports money and no limits by
            // design, and the money is then the whole reading.
            if data.usage.windows.is_empty() {
                if let Some(balance) = &data.usage.credit_balance {
                    cy += m.s(card::CONTENT_SPACING);
                    let _ = draw_value_row(
                        painter,
                        m,
                        inset_x,
                        &mut cy,
                        inset_w,
                        &quotascope_core::localization::t("Credit balance").to_string(),
                        balance,
                        alpha,
                    );
                } else {
                    cy += m.s(card::CONTENT_SPACING);
                    let message_brush = painter.brush(faded(palette.text_secondary, alpha))?;
                    draw_wrapped(
                        painter,
                        quotascope_core::localization::t("No limits reported."),
                        inset_x,
                        &mut cy,
                        inset_w,
                        m.s(card::MESSAGE_FONT),
                        &message_brush,
                    );
                }
            }
        }
    }

    for note in account_notes(data) {
        cy += m.s(card::ROW_INTERNAL_SPACING);
        draw_detail_line(painter, m, inset_x, &mut cy, inset_w, &note, alpha)?;
    }
    draw_history(painter, m, inset_x, &mut cy, inset_w, data, alpha)?;

    // The "as of" line: how much to trust the figures.
    if let State::Stale = data.usage.state {
        let stamp = data
            .usage
            .observed_at
            .map(|at| {
                quotascope_core::localization::t_fmt(
                    "As of {time}",
                    &[&quotascope_core::timeutil::relative_text(at)],
                )
            })
            .unwrap_or_else(|| {
                quotascope_core::localization::t("Reading may be out of date").to_string()
            });
        let footnote_brush = painter.brush(faded(palette.text_disabled, alpha))?;
        cy += m.s(card::CONTENT_SPACING);
        painter.text(
            &stamp,
            crate::d2d::rect(
                inset_x as f32,
                cy as f32,
                inset_w as f32,
                m.s(card::ROW_TEXT_LINE_HEIGHT) as f32,
            ),
            m.s(card::FOOTNOTE_FONT) as f32,
            DWRITE_FONT_WEIGHT_NORMAL,
            &footnote_brush,
            0,
            0,
        );
        cy += m.s(card::ROW_TEXT_LINE_HEIGHT);
    }

    debug_assert!((cy + m.s(card::PADDING) - body_size(m, data).1 - origin.1).abs() < 0.1);

    Ok(())
}

/// One limit row: the name on its own line, the bar, spent + reset paired
/// below, and the forecast when it is shown.
fn draw_progress_row(
    painter: &Painter,
    m: &Metrics,
    x: f64,
    cy: &mut f64,
    width: f64,
    window: &quotascope_core::model::UsageWindow,
    data: &CardData,
    alpha: f32,
) -> windows::core::Result<()> {
    let palette = panel::palette();
    let spent_color = usage_tint::is_spent(Some(window));

    // The name gets the row to itself; a scoped name plus a reset time does
    // not fit one line, and the name is the half that says which limit
    // this is.
    let title_brush = painter.brush(faded(palette.text_primary, alpha))?;
    let clock = data
        .detailed
        .then(|| window.window_clock_fraction(false, quotascope_core::timeutil::now_ms()))
        .flatten();
    painter.text(
        &window.display_name(),
        crate::d2d::rect(
            x as f32,
            *cy as f32,
            (width * if clock.is_some() { 0.67 } else { 1.0 }) as f32,
            m.s(card::ROW_TEXT_LINE_HEIGHT) as f32,
        ),
        m.s(card::ROW_FONT) as f32,
        DWRITE_FONT_WEIGHT_NORMAL,
        &title_brush,
        0,
        1,
    );
    if let Some(clock) = clock {
        let text = quotascope_core::localization::t_fmt(
            "Time {percent}%",
            &[&format!("{:.0}", clock * 100.0)],
        );
        let brush = painter.brush(faded(palette.text_disabled, alpha))?;
        painter.text(
            &text,
            crate::d2d::rect(
                (x + width * 0.67) as f32,
                *cy as f32,
                (width * 0.33) as f32,
                m.s(card::ROW_TEXT_LINE_HEIGHT) as f32,
            ),
            m.s(10.0) as f32,
            DWRITE_FONT_WEIGHT_NORMAL,
            &brush,
            2,
            1,
        );
    }
    *cy += m.s(card::ROW_TEXT_LINE_HEIGHT) + m.s(card::ROW_INTERNAL_SPACING);

    // The bar: a capsule filled to the fraction, in the system accent —
    // the whole surface speaks one colour; the figure below it carries the
    // meaning.
    let progress = if data.shows_remaining {
        window.remaining_fraction()
    } else {
        window.used_fraction.clamp(0.0, 1.0)
    };
    let bar_h = m.s(card::PROGRESS_BAR_HEIGHT) as f32;
    let bar_y = (*cy + m.s(card::PROGRESS_BAR_HEIGHT) / 2.0) as f32;
    let track_brush = painter.brush(faded(palette.bar_track, alpha))?;
    draw_capsule(
        painter,
        x as f32,
        bar_y - bar_h / 2.0,
        width as f32,
        bar_h,
        &track_brush,
    );

    let fill_w = (width * progress) as f32;
    if progress > 0.0 {
        let fill_brush = painter.brush(faded(panel::accent(), alpha))?;
        // The smallest non-zero reading still puts a dot of colour on
        // screen — the same rule the ring's round cap follows.
        let fill_w = fill_w.max(bar_h).min(width as f32);
        draw_capsule(
            painter,
            x as f32,
            bar_y - bar_h / 2.0,
            fill_w,
            bar_h,
            &fill_brush,
        );
    }
    *cy += m.s(card::PROGRESS_BAR_HEIGHT) + m.s(card::ROW_INTERNAL_SPACING);

    // The two short facts pair off: what is gone, and when it comes back.
    // The word has to follow the figure.
    let percent_text = window.percent_text(data.shows_remaining);
    let figure_label = if data.shows_remaining {
        quotascope_core::localization::t_fmt("{p} Left", &[&percent_text])
    } else {
        quotascope_core::localization::t_fmt("{p} Used", &[&percent_text])
    };
    let figure_brush = painter.brush(if spent_color {
        faded(Rgba::from(usage_tint::EXHAUSTED), alpha)
    } else if window.used_fraction >= data.warning_fraction {
        // Past the reader's line the figure itself carries the warning —
        // the bar stays the system accent either way.
        faded(Rgba::from(usage_tint::WARNING), alpha)
    } else {
        faded(palette.text_secondary, alpha)
    })?;
    painter.text(
        &figure_label,
        crate::d2d::rect(
            x as f32,
            *cy as f32,
            (width * 0.45) as f32,
            m.s(card::ROW_TEXT_LINE_HEIGHT) as f32,
        ),
        m.s(card::ROW_FONT) as f32,
        windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT_MEDIUM,
        &figure_brush,
        0,
        1,
    );

    let reset = reset_text(window);
    if !reset.is_empty() {
        let reset_brush = painter.brush(faded(palette.text_disabled, alpha))?;
        painter.text(
            &reset,
            crate::d2d::rect(
                (x + width * 0.4) as f32,
                *cy as f32,
                (width * 0.6) as f32,
                m.s(card::ROW_TEXT_LINE_HEIGHT) as f32,
            ),
            m.s(card::ROW_FONT) as f32,
            DWRITE_FONT_WEIGHT_NORMAL,
            &reset_brush,
            2,
            1,
        );
    }
    *cy += m.s(card::ROW_TEXT_LINE_HEIGHT);

    // The value estimate, when this machine's own records can price the
    // window. Labelled an estimate because it is the one figure here that
    // was inferred rather than reported.
    if let Some(text) = data.value_lines.get(&window.id) {
        *cy += m.s(card::ROW_INTERNAL_SPACING);
        let estimate_brush = painter.brush(faded(palette.text_disabled, alpha))?;
        painter.text(
            text,
            crate::d2d::rect(
                x as f32,
                *cy as f32,
                width as f32,
                m.s(card::ROW_TEXT_LINE_HEIGHT) as f32,
            ),
            m.s(card::FOOTNOTE_FONT) as f32,
            DWRITE_FONT_WEIGHT_NORMAL,
            &estimate_brush,
            0,
            0,
        );
        *cy += m.s(card::ROW_TEXT_LINE_HEIGHT);
    }

    // The forecast: the one line the provider did not say, dimmer than the
    // figures above it, absent far more often than present — and the
    // verdict without the time when the time is past the horizon.
    if let Some((text, color)) = forecast_line(window, data) {
        *cy += m.s(card::ROW_INTERNAL_SPACING);
        let burn_brush = painter.brush(faded(color, alpha))?;
        painter.text(
            &text,
            crate::d2d::rect(
                x as f32,
                *cy as f32,
                width as f32,
                m.s(card::ROW_TEXT_LINE_HEIGHT) as f32,
            ),
            m.s(card::ROW_FONT) as f32,
            DWRITE_FONT_WEIGHT_NORMAL,
            &burn_brush,
            0,
            1,
        );
        *cy += m.s(card::ROW_TEXT_LINE_HEIGHT);
    }

    Ok(())
}

fn forecast_line(
    window: &quotascope_core::model::UsageWindow,
    data: &CardData,
) -> Option<(String, Rgba)> {
    if !data.shows_forecast || usage_tint::is_spent(Some(window)) {
        return None;
    }
    let burn =
        quotascope_core::model::burn_rate::reading(window, quotascope_core::timeutil::now_ms())?;
    Some(if let Some(ms) = burn.time_to_exhaustion_ms {
        (
            quotascope_core::localization::t_fmt(
                "Runs out in {t}",
                &[&quotascope_core::model::burn_rate::approximate(ms)],
            ),
            Rgba::from(usage_tint::WARNING).with_alpha(0.9),
        )
    } else if burn.exhausts_before_reset {
        (
            quotascope_core::localization::t("Won't last the window").into(),
            Rgba::from(usage_tint::WARNING).with_alpha(0.9),
        )
    } else {
        (
            quotascope_core::localization::t("Expected to last the window").into(),
            panel::palette().text_disabled,
        )
    })
}

/// One fill keeps the translucent track's round ends free of overlap seams.
fn draw_capsule(
    painter: &Painter,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
) {
    painter.fill_rounded_rect(
        crate::d2d::rect(x, y, width, height),
        height.min(width) / 2.0,
        brush,
    );
}

fn draw_value_row(
    painter: &Painter,
    m: &Metrics,
    x: f64,
    cy: &mut f64,
    width: f64,
    title: &str,
    value: &str,
    alpha: f32,
) -> windows::core::Result<()> {
    let palette = panel::palette();
    let title_brush = painter.brush(faded(palette.text_primary, alpha))?;
    painter.text(
        title,
        crate::d2d::rect(
            x as f32,
            *cy as f32,
            (width * 0.5) as f32,
            m.s(card::ROW_TEXT_LINE_HEIGHT) as f32,
        ),
        m.s(card::ROW_FONT) as f32,
        DWRITE_FONT_WEIGHT_NORMAL,
        &title_brush,
        0,
        1,
    );
    let value_brush = painter.brush(faded(palette.text_secondary, alpha))?;
    painter.text(
        value,
        crate::d2d::rect(
            (x + width * 0.4) as f32,
            *cy as f32,
            (width * 0.6) as f32,
            m.s(card::ROW_TEXT_LINE_HEIGHT) as f32,
        ),
        m.s(card::ROW_FONT) as f32,
        windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT_MEDIUM,
        &value_brush,
        2,
        1,
    );
    *cy += m.s(card::ROW_TEXT_LINE_HEIGHT);
    Ok(())
}

/// The reset line, or the length when the provider actually stated one —
/// never a sort key dressed as a measurement. A bought pack's expiry rides
/// the same line when there is one: a balance that will shrink is a fact
/// about the future worth the space.
fn reset_text(window: &quotascope_core::model::UsageWindow) -> String {
    let base = match window.resets_at {
        Some(at) => quotascope_core::localization::t_fmt(
            "Resets {time}",
            &[&quotascope_core::timeutil::reset_text(at)],
        ),
        None => {
            if window.reports_length {
                window.length_text()
            } else {
                String::new()
            }
        }
    };
    match window.next_expiry_ms {
        Some(at) => {
            let expiry = if let Some(amount) = window.next_expiry_amount {
                quotascope_core::localization::t_fmt(
                    "{amount} credits expire {time}",
                    &[
                        &format!("{amount:.0}"),
                        &quotascope_core::timeutil::reset_text(at),
                    ],
                )
            } else {
                quotascope_core::localization::t_fmt(
                    "expires {time}",
                    &[&quotascope_core::timeutil::reset_text(at)],
                )
            };
            if base.is_empty() {
                expiry
            } else {
                format!("{base} · {expiry}")
            }
        }
        None => base,
    }
}

/// Wrapped text: breaks on spaces into lines that fit, up to four. The
/// card's messages are short; a full text layout engine is not needed for
/// them, and the row heights above are budgets, not measurements.
fn draw_wrapped(
    painter: &Painter,
    text: &str,
    x: f64,
    cy: &mut f64,
    width: f64,
    font: f64,
    brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
) {
    let lines = wrapped_lines(text, width, font);
    let lh = font * 1.3;
    for (i, text) in lines.iter().enumerate() {
        painter.text(
            text,
            crate::d2d::rect(
                x as f32,
                (*cy + lh * i as f64) as f32,
                width as f32,
                lh as f32,
            ),
            font as f32,
            DWRITE_FONT_WEIGHT_NORMAL,
            brush,
            0,
            0,
        );
    }
    *cy += lh * lines.len() as f64;
}

fn wrapped_lines(text: &str, width: f64, font: f64) -> Vec<String> {
    let mut line = String::new();
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0.0;
    // CJK does not separate words with spaces. Budget a full em per glyph.
    for ch in text.chars() {
        let advance = font * if ch.is_ascii() { 0.56 } else { 1.0 };
        if used + advance > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            used = 0.0;
        }
        line.push(ch);
        used += advance;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines.truncate(4);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use quotascope_core::history::HistoryRead;
    use quotascope_core::model::{AccountKey, Provider};
    use serde_json::json;

    fn data(history: HistoryRead) -> CardData {
        CardData {
            usage: ProviderUsage::live_now(AccountKey::primary(Provider::Zai), Vec::new()),
            title: "z.ai".into(),
            monogram: "Z".into(),
            icon: None,
            shows_remaining: false,
            shows_forecast: false,
            warning_fraction: 0.75,
            value_lines: HashMap::new(),
            detailed: true,
            history_enabled: true,
            history: Some(history),
            prompt_cache: None,
            codex_details: None,
            shows_codex_reset_credits: false,
        }
    }

    #[test]
    fn reset_credit_metadata_is_visible_without_reading_local_transcripts() {
        use quotascope_core::codex_account::{AccountDetails, ResetCredits};
        let mut payload = data(HistoryRead::NotConfigured);
        payload.usage.account = AccountKey::primary(Provider::Codex);
        payload.history_enabled = false;
        payload.detailed = false;
        payload.shows_codex_reset_credits = true;
        let m = Metrics::from_settings(&Default::default());
        let loading_height = body_size(&m, &payload).1;
        payload.codex_details = Some(AccountDetails {
            usage: None,
            reset_credits: ResetCredits::Available {
                count: 3,
                next_expiry_ms: Some(1_900_000_000_000),
            },
        });
        assert_eq!(account_notes(&payload).len(), 2);
        assert!((body_size(&m, &payload).1 - loading_height - m.s(24.0)).abs() < 1e-8);
        assert!(activity(&payload).is_none());
        payload.codex_details.as_mut().unwrap().reset_credits = ResetCredits::Unreported;
        assert_eq!(account_notes(&payload).len(), 1);
        payload.shows_codex_reset_credits = false;
        assert!(account_notes(&payload).is_empty());
    }

    #[test]
    fn account_statistics_never_show_a_money_column_and_reserve_chart_height() {
        let today = chrono::Local::now().date_naive().to_string();
        let history = quotascope_core::history::parse_reply(
            &json!({"success":true,"code":200,"data":{"x_time":[today],"tokensUsage":[1000]}}),
        );
        let mut payload = data(history);
        assert!(has_history_chart(&payload));
        assert!(activity(&payload)
            .unwrap()
            .notes
            .iter()
            .all(|line| !line.contains('$')));
        assert!(activity(&payload)
            .unwrap()
            .figures
            .iter()
            .all(|figure| figure.cost.is_none()));
        let m = Metrics::from_settings(&Default::default());
        let loaded_height = body_size(&m, &payload).1;
        payload.history = None;
        assert!(!has_history_chart(&payload));
        assert!(loaded_height > body_size(&m, &payload).1 + m.s(30.0));
    }

    #[test]
    fn local_history_uses_calendar_cutoff_and_respects_the_opt_in() {
        let today = chrono::Local::now().date_naive();
        let mut ledger = quotascope_core::history::parse_statistics(&json!({"x_time":[(today - chrono::Days::new(30)).to_string(),today.to_string()],"tokensUsage":[900,1]})).unwrap();
        ledger.days.last_mut().unwrap().cost = 0.12;
        assert_eq!(
            recent_days(&ledger)
                .iter()
                .map(|day| day.tokens)
                .sum::<i64>(),
            1
        );
        let mut payload = data(HistoryRead::Answered {
            ledger,
            account_wide: false,
        });
        let section = activity(&payload).unwrap();
        assert_eq!(section.figures.len(), 3);
        assert!(section
            .figures
            .iter()
            .all(|figure| figure.tokens == 1 && figure.cost == Some(0.12)));
        payload.history_enabled = false;
        assert!(activity(&payload).is_none());
        assert!(!has_history_chart(&payload));
    }

    #[test]
    fn frame_reserves_only_actual_content_at_every_panel_scale() {
        let mut payload = data(HistoryRead::NotConfigured);
        payload.history_enabled = false;
        payload.detailed = false;
        payload.usage = ProviderUsage::unavailable(
            AccountKey::primary(Provider::Codex),
            Unavailability::SignInRequired,
        );
        for scale in [0.82, 1.0, 1.22] {
            let mut m = Metrics::from_settings(&Default::default());
            m.scale = scale;
            let (width, height) = body_size(&m, &payload);
            assert!((width - 250.0 * scale).abs() < 0.01);
            assert!(
                height < 110.0 * scale,
                "unavailable card must not reserve phantom limit rows"
            );
            payload.usage = ProviderUsage::live_now(
                AccountKey::primary(Provider::Codex),
                quotascope_core::providers::codex::parse_usage_response(
                    &json!({"rate_limit":{"primary_window":{"used_percent":16,"limit_window_seconds":18000},"secondary_window":{"used_percent":56,"limit_window_seconds":604800}}}),
                ),
            );
            let expected = m.s(card::PADDING * 2.0 + card::HEADER_HEIGHT)
                + 2.0 * (m.s(card::CONTENT_SPACING) + card::row_height(&m, false));
            assert!((body_size(&m, &payload).1 - expected).abs() < 0.01);
            payload.usage = ProviderUsage::unavailable(
                AccountKey::primary(Provider::Codex),
                Unavailability::SignInRequired,
            );
        }
    }

    #[test]
    fn token_figures_are_compact_and_cjk_messages_wrap() {
        assert_eq!(short_tokens(490581125), "490.6M");
        assert_eq!(short_tokens(1234), "1.2K");
        assert_eq!(short_tokens(0), "0");
        assert_eq!(short_tokens(-1), "0");
        let message = "近三十天没有可读取的本机用量记录，请检查本机登录状态。";
        let lines = wrapped_lines(message, 120.0, 12.0);
        assert!(lines.len() > 1);
        assert_eq!(lines.join(""), message);
    }
}
