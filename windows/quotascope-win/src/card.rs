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

use crate::d2d::{point, Painter, Rgba};
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
}

/// The same extra rows are measured and drawn, so toggling a detailed card
/// cannot leave its content clipped by the old flyout frame.
pub fn detail_height(m: &Metrics, data: &CardData) -> f64 {
    let lines = detail_header(data).len() + clock_lines(data).len() + history_lines(data).len();
    lines as f64 * (m.s(card::ROW_TEXT_LINE_HEIGHT) + m.s(card::ROW_INTERNAL_SPACING))
        + if has_history_chart(data) {
            m.s(58.0)
        } else {
            0.0
        }
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
    lines
}

fn clock_lines(data: &CardData) -> Vec<String> {
    if !data.detailed || matches!(data.usage.state, State::Unavailable(_)) {
        return Vec::new();
    }
    let now = quotascope_core::timeutil::now_ms();
    data.usage
        .windows
        .iter()
        .filter_map(|w| {
            w.window_clock_fraction(false, now).map(|f| {
                quotascope_core::localization::t_fmt(
                    "{window}: {percent}% of window elapsed",
                    &[&w.display_name(), &format!("{:.0}", f * 100.0)],
                )
            })
        })
        .collect()
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

fn has_history_chart(data: &CardData) -> bool {
    data.history_enabled
        && matches!(&data.history, Some(quotascope_core::history::HistoryRead::Answered { ledger, .. }) if !recent_days(ledger).is_empty())
}

fn history_lines(data: &CardData) -> Vec<String> {
    use quotascope_core::history::HistoryRead;
    use quotascope_core::localization::{t, t_fmt};
    if !data.history_enabled {
        return Vec::new();
    }
    let mut lines = vec![t("Recent usage (30 days)").to_string()];
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
                let tokens = days
                    .iter()
                    .fold(0_i64, |sum, day| sum.saturating_add(day.tokens));
                let cost: f64 = days.iter().map(|day| day.cost).sum();
                lines.push(if *account_wide {
                    t_fmt("{tokens} tokens", &[&tokens.to_string()])
                } else {
                    t_fmt(
                        "{tokens} tokens · API value ≈${cost}",
                        &[&tokens.to_string(), &format!("{cost:.2}")],
                    )
                });
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
                        &[&unpriced.to_string()],
                    ));
                }
                lines.push(format!(
                    "{} – {}",
                    days.first().unwrap().date,
                    days.last().unwrap().date
                ));
            }
            lines.push(
                t(if *account_wide {
                    "Provider statistics · all machines · no price breakdown"
                } else {
                    "Local records · API value is an estimate, not a bill"
                })
                .to_string(),
            );
        }
    }
    lines
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
    *cy += m.s(card::ROW_INTERNAL_SPACING);
    let brush = painter.brush(faded(panel::palette().text_secondary, alpha))?;
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
        &brush,
        0,
        1,
    );
    *cy += m.s(card::ROW_TEXT_LINE_HEIGHT);
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
    for text in history_lines(data) {
        draw_detail_line(painter, m, x, cy, width, &text, alpha)?;
    }
    if !has_history_chart(data) {
        return Ok(());
    }
    let Some(quotascope_core::history::HistoryRead::Answered { ledger, .. }) = &data.history else {
        return Ok(());
    };
    let days = recent_days(ledger);
    let last = days.last().unwrap().date;
    let first = days.first().unwrap().date;
    let slots = (last - first).num_days() as usize + 1;
    let maximum = days.iter().map(|day| day.tokens).max().unwrap_or(1).max(1) as f64;
    let brush = painter.brush(faded(panel::accent(), alpha))?;
    let slot_width = width / slots as f64;
    for day in days {
        let index = (day.date - first).num_days() as usize;
        let height = m.s(50.0) * day.tokens as f64 / maximum;
        if height > 0.0 {
            painter.fill_rounded_rect(
                crate::d2d::rect(
                    (x + index as f64 * slot_width) as f32,
                    (*cy + m.s(54.0) - height) as f32,
                    (slot_width * 0.75) as f32,
                    height as f32,
                ),
                0.0,
                &brush,
            );
        }
    }
    *cy += m.s(58.0);
    Ok(())
}

/// The card's total size for a reading of this shape — what the flyout
/// window has to be, in design units.
pub fn body_size(
    m: &Metrics,
    windows_count: usize,
    footnote: bool,
    forecast: bool,
    estimate_lines: usize,
) -> (f64, f64) {
    let w = m.s(card::WIDTH) + m.s(card::PADDING) * 2.0;
    let h = m.s(card::PADDING) * 2.0
        + m.s(card::HEADER_HEIGHT)
        + windows_count as f64 * (m.s(card::CONTENT_SPACING) + card::row_height(m, forecast))
        + estimate_lines as f64
            * (m.s(card::ROW_INTERNAL_SPACING) + m.s(card::ROW_TEXT_LINE_HEIGHT))
        + if footnote {
            m.s(card::CONTENT_SPACING) + m.s(card::ROW_TEXT_LINE_HEIGHT)
        } else {
            0.0
        }
        // Room for a wrapped unavailable message even with no rows.
        .max(
            m.s(card::PADDING) * 2.0
                + m.s(card::HEADER_HEIGHT)
                + m.s(card::ROW_TEXT_LINE_HEIGHT) * 4.0,
        );
    (w, h)
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
            let message = if *reason == Unavailability::NotOnWindows {
                data.usage
                    .account
                    .provider
                    .windows_gap()
                    .unwrap_or(reason.message())
                    .to_string()
            } else {
                reason.message().to_string()
            };
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

    for line in clock_lines(data) {
        draw_detail_line(painter, m, inset_x, &mut cy, inset_w, &line, alpha)?;
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
    }

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
    painter.text(
        &window.display_name(),
        crate::d2d::rect(
            x as f32,
            *cy as f32,
            width as f32,
            m.s(card::ROW_TEXT_LINE_HEIGHT) as f32,
        ),
        m.s(card::ROW_FONT) as f32,
        DWRITE_FONT_WEIGHT_NORMAL,
        &title_brush,
        0,
        1,
    );
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
    if data.shows_forecast && !spent_color {
        if let Some(burn) =
            quotascope_core::model::burn_rate::reading(window, quotascope_core::timeutil::now_ms())
        {
            *cy += m.s(card::ROW_INTERNAL_SPACING);
            let (text, color) = if let Some(ms) = burn.time_to_exhaustion_ms {
                (
                    quotascope_core::localization::t_fmt(
                        "Runs out in {t}",
                        &[&quotascope_core::model::burn_rate::approximate(ms)],
                    ),
                    Rgba::from(usage_tint::WARNING).with_alpha(0.9),
                )
            } else if burn.exhausts_before_reset {
                (
                    quotascope_core::localization::t("Won't last the window").to_string(),
                    Rgba::from(usage_tint::WARNING).with_alpha(0.9),
                )
            } else {
                (
                    quotascope_core::localization::t("Expected to last the window").to_string(),
                    palette.text_disabled,
                )
            };
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
    }

    Ok(())
}

/// A capsule: two half-circle ends and a body. Cheaper than a path per bar,
/// and exactly the shape a progress capsule is.
fn draw_capsule(
    painter: &Painter,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
) {
    let r = height / 2.0;
    if width <= height {
        painter.fill_ellipse(point(x + width / 2.0, y + r), width / 2.0, brush);
        return;
    }
    painter.fill_ellipse(point(x + r, y + r), r, brush);
    painter.fill_ellipse(point(x + width - r, y + r), r, brush);
    painter.fill_rounded_rect(
        crate::d2d::rect(
            (x + r) as f32,
            y as f32,
            (width - height) as f32,
            height as f32,
        ),
        (height / 2.0) as f32,
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
            let expiry = quotascope_core::localization::t_fmt(
                "expires {time}",
                &[&quotascope_core::timeutil::reset_text(at)],
            );
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
    let mut line = String::new();
    let mut lines: Vec<String> = Vec::new();
    // Rough width estimate: a little over half the font size per character,
    // conservative for the CJK-heavy strings this table carries.
    let chars_per_line = ((width / (font * 0.62)).floor() as usize).max(8);
    for word in text.split_whitespace() {
        let candidate_len = line.chars().count() + word.chars().count() + 1;
        if candidate_len > chars_per_line && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines.truncate(4);
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
        }
    }

    #[test]
    fn account_statistics_never_show_a_money_column_and_reserve_chart_height() {
        let today = chrono::Local::now().date_naive().to_string();
        let history = quotascope_core::history::parse_reply(
            &json!({"success":true,"code":200,"data":{"x_time":[today],"tokensUsage":[1000]}}),
        );
        let mut payload = data(history);
        assert!(has_history_chart(&payload));
        assert!(history_lines(&payload)
            .iter()
            .all(|line| !line.contains('$')));
        let m = Metrics::from_settings(&Default::default());
        let loaded_height = detail_height(&m, &payload);
        payload.history = None;
        assert!(!has_history_chart(&payload));
        assert!(loaded_height > detail_height(&m, &payload) + m.s(58.0));
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
        assert!(history_lines(&payload)
            .iter()
            .any(|line| line.contains("$0.12")));
        payload.history_enabled = false;
        assert!(history_lines(&payload).is_empty());
        assert!(!has_history_chart(&payload));
    }
}
