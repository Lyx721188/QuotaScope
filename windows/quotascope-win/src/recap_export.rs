//! Native Direct2D/WIC share cards, with PNG and Windows clipboard formats.
use crate::d2d::{Painter, Rgba};
use quotascope_core::recap::MonthlyRecap;
use std::path::{Path, PathBuf};
use windows::core::{w, PCWSTR};
use windows::Win32::Graphics::{Direct2D::*, Imaging::*};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

const WIDTH: u32 = 1080;
const HEIGHT: u32 = 1920;
pub const TITLES: [&str; 8] = [
    "Overview",
    "Usage calendar",
    "Tools and models",
    "Year calendar",
    "Daily rhythm",
    "Subscription comparison",
    "Months",
    "Scorecard",
];

pub fn available(recap: &MonthlyRecap, monthly_usd: Option<f64>) -> Vec<usize> {
    use quotascope_core::recap::Period;
    if recap.analysis.total.tokens <= 0 {
        return Vec::new();
    }
    let mut cards = vec![0, 2];
    match recap.period {
        Period::Month => cards.push(1),
        Period::Year => cards.extend([3, 6]),
        _ => {}
    }
    if recap.timed_tokens > 0 {
        cards.push(4);
    }
    if recap.analysis.total.cost > 0.0
        && monthly_usd.and_then(|price| recap.payback(price)).is_some()
    {
        cards.push(5);
    }
    cards.push(7);
    cards
}

fn render(
    recap: &MonthlyRecap,
    kind: usize,
    monthly_usd: Option<f64>,
) -> windows::core::Result<(IWICImagingFactory, IWICBitmap)> {
    use windows::Win32::Graphics::Direct2D::Common::*;
    use windows::Win32::Graphics::DirectWrite::*;
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let bitmap = factory.CreateBitmap(
            WIDTH,
            HEIGHT,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapCacheOnLoad,
        )?;
        let engine = crate::d2d::global_engine();
        let target = engine.factory.CreateWicBitmapRenderTarget(
            &bitmap,
            &D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            },
        )?;
        target.BeginDraw();
        target.SetTransform(&crate::d2d::scale_matrix(0.9, 0.9));
        target.Clear(Some(&crate::d2d::color(Rgba::from_hex(0x101827))));
        let painter = Painter {
            rt: &target,
            engine,
        };
        let ink = painter.brush(Rgba::from_hex(0xe8eef8))?;
        let muted = painter.brush(Rgba::from_hex(0xa4b5ce))?;
        let accent = painter.brush(Rgba::from_hex(0x7299ff))?;
        let text = |value: &str, y: f32, size: f32, bold: bool| {
            painter.text(
                value,
                crate::d2d::rect(72.0, y, 1056.0, size * 1.7),
                size,
                if bold {
                    DWRITE_FONT_WEIGHT_BOLD
                } else {
                    DWRITE_FONT_WEIGHT_NORMAL
                },
                &ink,
                0,
                0,
            );
        };
        text(
            &format!(
                "QuotaScope · {}",
                quotascope_core::localization::t(TITLES[kind.min(7)])
            ),
            52.0,
            26.0,
            true,
        );
        text(
            &if recap.month == recap.through {
                recap.month.to_string()
            } else {
                format!("{} — {}", recap.month, recap.through)
            },
            104.0,
            30.0,
            false,
        );
        let total = recap.analysis.total;
        let money = if total.tokens > 0 && total.unpriced >= total.tokens {
            quotascope_core::localization::t("Unpriced").to_string()
        } else {
            quotascope_core::money::estimate(total.cost)
        };
        match kind {
            1 => {
                let count = (recap.end - recap.month).num_days().max(0) as usize + 1;
                let columns = if count > 31 { 26 } else { 7 };
                let step = 1000.0 / columns as f32;
                let height = if count > 31 { 86.0 } else { 220.0 };
                let maximum = recap
                    .analysis
                    .days
                    .iter()
                    .map(|d| d.measures.tokens)
                    .max()
                    .unwrap_or(1)
                    .max(1) as f64;
                for index in 0..count.min(366) {
                    let date = recap.month + chrono::Days::new(index as u64);
                    let tokens = recap.day_tokens(date);
                    let intensity = tokens.map(|n| (n as f64 / maximum).sqrt()).unwrap_or(0.0);
                    let brush = painter.brush(Rgba::new(
                        0.3,
                        0.48,
                        0.95,
                        if tokens.is_none() {
                            0.04
                        } else {
                            (0.16 + intensity * 0.84) as f32
                        },
                    ))?;
                    let position = index
                        + chrono::Datelike::weekday(&recap.month).num_days_from_monday() as usize;
                    let x = 72.0 + (position % columns) as f32 * step;
                    let position = index
                        + chrono::Datelike::weekday(&recap.month).num_days_from_monday() as usize;
                    let y = 360.0 + (position / columns) as f32 * height;
                    painter.fill_rounded_rect(
                        crate::d2d::rect(x, y, step - 6.0, height - 6.0),
                        4.0,
                        &brush,
                    );
                    if count <= 31 {
                        painter.text(
                            &format!(
                                "{}\n{}",
                                date.format("%m-%d"),
                                tokens.map(|n| n.to_string()).unwrap_or_else(|| "—".into())
                            ),
                            crate::d2d::rect(x + 7.0, y + 4.0, step - 18.0, height - 12.0),
                            16.0,
                            DWRITE_FONT_WEIGHT_NORMAL,
                            &ink,
                            0,
                            0,
                        );
                    }
                }
            }
            3 => {
                use chrono::Datelike;
                let maximum = recap
                    .analysis
                    .days
                    .iter()
                    .map(|day| day.measures.tokens)
                    .max()
                    .unwrap_or(1)
                    .max(1) as f64;
                for month in 1..=12 {
                    let start =
                        chrono::NaiveDate::from_ymd_opt(recap.month.year(), month, 1).unwrap();
                    let end = quotascope_core::recap::next_month(start) - chrono::Days::new(1);
                    let left = 72.0 + ((month - 1) % 3) as f32 * 350.0;
                    let top = 310.0 + ((month - 1) / 3) as f32 * 360.0;
                    painter.text(
                        &start.format("%Y-%m").to_string(),
                        crate::d2d::rect(left, top, 320.0, 40.0),
                        25.0,
                        DWRITE_FONT_WEIGHT_BOLD,
                        &ink,
                        0,
                        0,
                    );
                    for day in 1..=end.day() {
                        let date = start.with_day(day).unwrap();
                        let count = recap.day_tokens(date);
                        let position = start.weekday().num_days_from_monday() + day - 1;
                        let x = left + (position % 7) as f32 * 44.0;
                        let y = top + 57.0 + (position / 7) as f32 * 44.0;
                        let intensity = count.map(|n| (n as f64 / maximum).sqrt()).unwrap_or(0.0);
                        let brush = painter.brush(Rgba::new(
                            0.3,
                            0.48,
                            0.95,
                            if count.is_none() {
                                0.04
                            } else {
                                (0.16 + intensity * 0.84) as f32
                            },
                        ))?;
                        painter.fill_rounded_rect(crate::d2d::rect(x, y, 36.0, 36.0), 4.0, &brush);
                    }
                }
            }
            2 => {
                let rows = &recap.analysis.rows;
                let maximum = rows
                    .iter()
                    .map(|r| r.measures.tokens)
                    .max()
                    .unwrap_or(1)
                    .max(1) as f64;
                for (index, row) in rows.iter().take(6).enumerate() {
                    let y = 340.0 + index as f32 * 150.0;
                    text(
                        &format!("{} · {} tokens", row.title, row.measures.tokens),
                        y,
                        24.0,
                        false,
                    );
                    painter.fill_rounded_rect(
                        crate::d2d::rect(
                            72.0,
                            y + 43.0,
                            (1000.0 * row.measures.tokens as f64 / maximum) as f32,
                            10.0,
                        ),
                        5.0,
                        &accent,
                    );
                }
                text(
                    quotascope_core::localization::t("Top models"),
                    1350.0,
                    36.0,
                    true,
                );
                for (index, model) in recap.models.iter().take(5).enumerate() {
                    text(
                        &format!("{} · {} tokens", model.title, model.measures.tokens),
                        1430.0 + index as f32 * 68.0,
                        26.0,
                        false,
                    );
                }
            }
            4 => {
                let maximum = recap.hours.iter().copied().max().unwrap_or(1).max(1) as f64;
                for (hour, tokens) in recap.hours.iter().enumerate() {
                    let x = 72.0 + hour as f32 * 43.0;
                    let height = (*tokens as f64 / maximum * 780.0) as f32;
                    painter.fill_rounded_rect(
                        crate::d2d::rect(x, 1460.0 - height, 31.0, height),
                        4.0,
                        &accent,
                    );
                    painter.text(
                        &hour.to_string(),
                        crate::d2d::rect(x, 1490.0, 36.0, 30.0),
                        16.0,
                        DWRITE_FONT_WEIGHT_NORMAL,
                        &muted,
                        1,
                        0,
                    );
                }
                text(
                    &quotascope_core::localization::t_fmt(
                        "{tokens} timed tokens",
                        &[&recap.timed_tokens.to_string()],
                    ),
                    350.0,
                    36.0,
                    true,
                );
            }
            5 => {
                text(
                    quotascope_core::localization::t("Estimated API value"),
                    350.0,
                    36.0,
                    false,
                );
                text(&money, 450.0, 110.0, true);
                if let Some((paid, multiple)) = monthly_usd.and_then(|price| recap.payback(price)) {
                    text(
                        &format!(
                            "{} · {}",
                            quotascope_core::localization::t("Prorated subscription price"),
                            quotascope_core::money::estimate(paid)
                        ),
                        760.0,
                        36.0,
                        false,
                    );
                    text(&format!("{multiple:.2}×"), 970.0, 110.0, true);
                }
                text(
                    &quotascope_core::localization::t_fmt(
                        "{tokens} tokens without a public price",
                        &[&total.unpriced.to_string()],
                    ),
                    1450.0,
                    28.0,
                    false,
                );
            }
            6 => {
                use chrono::Datelike;
                let mut totals = [0i64; 12];
                for day in &recap.analysis.days {
                    totals[day.date.month0() as usize] += day.measures.tokens;
                }
                let peak = totals.iter().copied().max().unwrap_or(1).max(1) as f64;
                for (month, tokens) in totals.iter().enumerate() {
                    let y = 330.0 + month as f32 * 110.0;
                    let start =
                        chrono::NaiveDate::from_ymd_opt(recap.month.year(), month as u32 + 1, 1)
                            .unwrap();
                    let end = quotascope_core::recap::next_month(start) - chrono::Days::new(1);
                    let recorded = start <= recap.through
                        && recap.recorded_from.is_some_and(|first| end >= first);
                    text(
                        &format!(
                            "{:02} · {}",
                            month + 1,
                            if recorded {
                                format!("{tokens} tokens")
                            } else {
                                "—".into()
                            }
                        ),
                        y,
                        28.0,
                        false,
                    );
                    painter.fill_rounded_rect(
                        crate::d2d::rect(
                            72.0,
                            y + 54.0,
                            (1000.0 * *tokens as f64 / peak) as f32,
                            16.0,
                        ),
                        8.0,
                        &accent,
                    );
                }
            }
            7 => {
                for (index, (label, value)) in [
                    ("Total tokens", total.tokens.to_string()),
                    ("Active days", recap.active_days.to_string()),
                    ("Observed days", recap.observed_days.to_string()),
                    ("Longest streak", recap.longest_streak.to_string()),
                    ("Estimated API value", money.clone()),
                ]
                .into_iter()
                .enumerate()
                {
                    let y = 320.0 + index as f32 * 260.0;
                    text(quotascope_core::localization::t(label), y, 32.0, false);
                    text(&value, y + 64.0, 72.0, true);
                }
            }
            _ => {
                text(&format!("{} tokens", total.tokens), 380.0, 116.0, true);
                text(
                    &quotascope_core::localization::t_fmt(
                        "{tools} AI tools · {active} active / {observed} observed days",
                        &[
                            &recap.analysis.rows.len().to_string(),
                            &recap.active_days.to_string(),
                            &recap.observed_days.to_string(),
                        ],
                    ),
                    650.0,
                    28.0,
                    false,
                );
                text(
                    &quotascope_core::localization::t_fmt(
                        "Longest streak · {days} days",
                        &[&recap.longest_streak.to_string()],
                    ),
                    830.0,
                    28.0,
                    false,
                );
                text(
                    &format!(
                        "{} · {}",
                        quotascope_core::localization::t("Estimated API value"),
                        money
                    ),
                    1080.0,
                    38.0,
                    true,
                );
                if let Some(model) = recap.models.first() {
                    text(
                        &quotascope_core::localization::t_fmt(
                            "Top model · {model}",
                            &[&model.title],
                        ),
                        1360.0,
                        26.0,
                        false,
                    );
                }
            }
        }
        painter.text(
            quotascope_core::localization::t(
                "Estimated from recorded usage and known public prices.",
            ),
            crate::d2d::rect(72.0, 1970.0, 1056.0, 40.0),
            19.0,
            DWRITE_FONT_WEIGHT_NORMAL,
            &muted,
            0,
            0,
        );
        painter.text(
            &format!(
                "{} · {} · QuotaScope",
                recap.through,
                quotascope_core::localization::t_fmt(
                    "{tokens} tokens without a public price",
                    &[&total.unpriced.to_string()]
                )
            ),
            crate::d2d::rect(72.0, 2040.0, 1056.0, 32.0),
            18.0,
            DWRITE_FONT_WEIGHT_NORMAL,
            &muted,
            0,
            0,
        );
        target.EndDraw(None, None)?;
        Ok((factory, bitmap))
    }
}

pub fn save_at(
    recap: &MonthlyRecap,
    kind: usize,
    monthly_usd: Option<f64>,
    path: &Path,
) -> Result<(), String> {
    let (factory, bitmap) = render(recap, kind, monthly_usd).map_err(|e| e.to_string())?;
    unsafe {
        let stream = factory.CreateStream().map_err(|e| e.to_string())?;
        let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        stream
            .InitializeFromFilename(PCWSTR(wide.as_ptr()), 0x40000000)
            .map_err(|e| e.to_string())?;
        let encoder = factory
            .CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())
            .map_err(|e| e.to_string())?;
        encoder
            .Initialize(&stream, WICBitmapEncoderNoCache)
            .map_err(|e| e.to_string())?;
        let mut frame = None;
        encoder
            .CreateNewFrame(&mut frame, std::ptr::null_mut())
            .map_err(|e| e.to_string())?;
        let frame = frame.ok_or("No PNG encoder frame")?;
        frame.Initialize(None).map_err(|e| e.to_string())?;
        frame.SetSize(WIDTH, HEIGHT).map_err(|e| e.to_string())?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame
            .SetPixelFormat(&mut format)
            .map_err(|e| e.to_string())?;
        frame
            .WriteSource(&bitmap, std::ptr::null())
            .map_err(|e| e.to_string())?;
        frame.Commit().map_err(|e| e.to_string())?;
        encoder.Commit().map_err(|e| e.to_string())?;
    }
    Ok(())
}
use std::os::windows::ffi::OsStrExt;

pub fn export(
    recap: &MonthlyRecap,
    kind: usize,
    monthly_usd: Option<f64>,
) -> Result<PathBuf, String> {
    let folder = quotascope_core::data_dir().join("Exports");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join(format!(
        "QuotaScope-{:?}-{}-{}-{}.png",
        recap.period,
        recap.month,
        kind,
        quotascope_core::timeutil::now_ms()
    ));
    save_at(recap, kind, monthly_usd, &path)?;
    Ok(path)
}

pub fn copy(
    recap: &MonthlyRecap,
    kind: usize,
    monthly_usd: Option<f64>,
    owner: windows::Win32::Foundation::HWND,
) -> Result<(), String> {
    use windows::Win32::System::{DataExchange::*, Memory::*};
    let path = export(recap, kind, monthly_usd)?;
    let png = std::fs::read(&path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(path);
    let (_, bitmap) = render(recap, kind, monthly_usd).map_err(|e| e.to_string())?;
    let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    unsafe {
        bitmap
            .CopyPixels(std::ptr::null(), WIDTH * 4, &mut pixels)
            .map_err(|e| e.to_string())?;
    }
    let mut dib = Vec::with_capacity(40 + pixels.len());
    dib.extend(40u32.to_le_bytes());
    dib.extend((WIDTH as i32).to_le_bytes());
    dib.extend((-(HEIGHT as i32)).to_le_bytes());
    dib.extend(1u16.to_le_bytes());
    dib.extend(32u16.to_le_bytes());
    dib.extend(0u32.to_le_bytes());
    dib.extend((pixels.len() as u32).to_le_bytes());
    dib.extend([0u8; 16]);
    dib.extend(pixels);
    unsafe fn put(format: u32, bytes: &[u8]) -> windows::core::Result<()> {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
        let pointer = GlobalLock(memory);
        if pointer.is_null() {
            let _ = windows::Win32::Foundation::GlobalFree(Some(memory));
            return Err(windows::core::Error::from_win32());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(memory);
        if let Err(error) =
            SetClipboardData(format, Some(windows::Win32::Foundation::HANDLE(memory.0)))
        {
            let _ = windows::Win32::Foundation::GlobalFree(Some(memory));
            return Err(error);
        }
        Ok(())
    }
    unsafe {
        OpenClipboard(Some(owner)).map_err(|e| e.to_string())?;
        struct Close;
        impl Drop for Close {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseClipboard();
                }
            }
        }
        let _close = Close;
        EmptyClipboard().map_err(|e| e.to_string())?;
        put(RegisterClipboardFormatW(w!("PNG")), &png).map_err(|e| e.to_string())?;
        put(windows::Win32::System::Ole::CF_DIB.0 as u32, &dib).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    #[ignore = "renders native Direct2D/WIC cards on the Windows desktop"]
    fn render_all_periods_to_native_png() {
        use quotascope_core::{
            ledger,
            model_prices::ModelPrice,
            recap::Period,
            spend::{Snapshot, Source},
        };
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        let mut buckets = BTreeMap::new();
        for month in 1..=11 {
            for day in [1, 3, 7, 11] {
                buckets.insert(
                    format!("2026-{month:02}-{day:02} 10:00"),
                    BTreeMap::from([(
                        "synthetic-model".into(),
                        ledger::TokenTally {
                            input: (month * day * 1000) as i64,
                            output: 5000,
                            cache_read: 500,
                            ..Default::default()
                        },
                    )]),
                );
            }
        }
        let prices = BTreeMap::from([(
            "synthetic-model".into(),
            ModelPrice {
                input: 2.0,
                output: 8.0,
                cache_read: Some(0.2),
                cache_write: None,
                name: Some("Synthetic model".into()),
            },
        )]);
        let snapshot = Snapshot {
            sources: vec![Source {
                id: "fixture".into(),
                title: "Synthetic agent".into(),
                location: String::new(),
                present: true,
                ledger: ledger::priced(&buckets, &prices, None),
            }],
        };
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        let folder = std::env::var_os("QUOTASCOPE_RENDER_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("quotascope-png-{}", std::process::id()))
            });
        std::fs::create_dir_all(&folder).unwrap();
        use quotascope_core::localization::{set_language, Language};
        for (language, tag) in [
            (Language::English, "en"),
            (Language::Chinese, "zh"),
            (Language::Russian, "ru"),
        ] {
            set_language(language);
            for period in [Period::Day, Period::Week, Period::Month, Period::Year] {
                let date = if period == Period::Day {
                    today - chrono::Days::new(1)
                } else {
                    today
                };
                let recap = MonthlyRecap::build_period(&snapshot, period, date, today, None, None);
                for kind in available(&recap, Some(20.0)) {
                    let path = folder.join(format!("{tag}-{period:?}-{kind}.png"));
                    save_at(&recap, kind, Some(20.0), &path).unwrap();
                    let bytes = std::fs::read(path).unwrap();
                    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
                    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), WIDTH);
                    assert_eq!(
                        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
                        HEIGHT
                    );
                }
            }
        }
        set_language(Language::English);
    }
}
