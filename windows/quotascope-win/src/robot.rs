//! Agent Robot Avatar geometry and poses, via Pulse 338c796.
//! Copyright (c) 2026 CX ART Lab, MIT; see THIRD_PARTY_NOTICES.md.
use crate::d2d::{Painter, PathBuilder, Rgba};
use crate::rings::RingModel;
use windows::Win32::Graphics::Direct2D::Common::D2D1_FILL_MODE_ALTERNATE;
use windows_numerics::Vector2;

#[derive(Clone)]
pub struct Style {
    pub color: Rgba,
    pub personality: u8,
    pub shape: u8,
    pub seed: u64,
    pub animated: bool,
    pub gesture_at: i64,
    pub gesture: u8,
}
pub fn style(id: &str, s: &quotascope_core::settings::AppSettings) -> Option<Style> {
    let config = s.robot_avatars.get(id)?;
    if !config.enabled {
        return None;
    }
    let seed = id.bytes().fold(1469598103934665603u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(1099511628211)
    });
    let index = s
        .ordered_enabled()
        .iter()
        .position(|a| a.id() == id)
        .unwrap_or(seed as usize);
    let chosen = quotascope_core::settings::AppSettings::parse_hex_color(&config.color);
    let mut color = chosen
        .map(|c| Rgba::new(c[0], c[1], c[2], 1.0))
        .unwrap_or_else(|| {
            Rgba::from_hex([0x7299ff, 0xc58fff, 0xea8eaf, 0x66c5d4, 0xe59d75][index % 5])
        });
    if chosen.is_none() && !crate::theme::panel::is_dark() {
        color.r *= 0.6;
        color.g *= 0.6;
        color.b *= 0.6;
    }
    Some(Style {
        color,
        personality: match config.personality.as_str() {
            "calm" => 0,
            "curious" => 1,
            "lively" => 2,
            _ => (index % 3) as u8,
        },
        shape: match config.shape.as_str() {
            "square" => 1,
            "round" => 2,
            _ => 0,
        },
        seed,
        animated: true,
        gesture_at: 0,
        gesture: 0,
    })
}

pub fn draw(
    p: &Painter,
    center: Vector2,
    size: f64,
    model: &RingModel,
    style: &Style,
    now_ms: f64,
) -> windows::core::Result<()> {
    let asleep = !model.has_reading && !(style.animated && (model.is_busy || model.is_refreshing));
    let spent = model.is_spent && !(style.animated && (model.is_busy || model.is_refreshing));
    let animated = style.animated && !asleep && !spent;
    let t = if animated {
        now_ms / 1000.0 + (style.seed % 10000) as f64 / 1000.0
    } else {
        0.0
    };
    let speed = [0.65, 1.0, 1.5][style.personality.min(2) as usize];
    let happy = style.animated && style.gesture == 1 && now_ms - (style.gesture_at as f64) < 1400.0;
    let affirm =
        style.animated && style.gesture == 2 && now_ms - (style.gesture_at as f64) < 1000.0;
    let working = style.animated && model.is_busy;
    let refreshing = style.animated && model.is_refreshing;
    let glance = if working {
        (t * 5.0).sin() * 10.0
    } else if refreshing {
        (t * 7.0).sin() * 6.0
    } else if animated {
        (t * speed * 0.65).sin() * 9.0
    } else {
        0.0
    };
    let look_y = if working { (t * 5.0).cos() * 9.0 } else { 0.0 };
    let idle_phase = (t * speed).rem_euclid(18.0);
    let surprise = animated
        && !working
        && !refreshing
        && !spent
        && style.personality == 2
        && idle_phase > 16.0;
    let inspect = animated
        && !working
        && !refreshing
        && !spent
        && style.personality == 1
        && idle_phase > 15.0;
    let bored = animated
        && !working
        && !refreshing
        && !spent
        && style.personality == 0
        && idle_phase > 14.0;
    let blink = animated && (t * speed).rem_euclid(4.6) < 0.14;
    let drop = if asleep {
        14.0
    } else if spent {
        8.0
    } else if affirm {
        ((now_ms - style.gesture_at as f64).max(0.0) / 170.0).sin() * 4.0
    } else {
        0.0
    };
    let tilt: f64 = if inspect {
        0.10
    } else if bored {
        -0.06
    } else {
        0.0
    };
    let scale = size / 188.0;
    let project = |x: f64, y: f64| {
        let x = x - 120.0;
        let y = y - 105.0 + drop;
        (
            center.X as f64 + (x * tilt.cos() - y * tilt.sin()) * scale,
            center.Y as f64 + (x * tilt.sin() + y * tilt.cos()) * scale,
        )
    };
    let mut path = PathBuilder::new(p.engine, D2D1_FILL_MODE_ALTERNATE)?;
    let outlines = outline(style.shape);
    for (i, (x, y)) in outlines.iter().enumerate() {
        let (x, y) = project(*x, *y);
        if i == 0 {
            path.begin_at(x, y);
        } else {
            path.line(x, y);
        }
    }
    path.end_closed();
    // Cut the eyes through the head, so Mica remains visible through them.
    let eye_w = if surprise { 34.0 } else { 27.0 };
    let eye_h = if blink {
        2.0
    } else if asleep {
        3.5
    } else if bored {
        13.0
    } else if surprise {
        34.0
    } else {
        29.0
    };
    for (index, cx) in [86.0, 154.0].into_iter().enumerate() {
        for i in 0..48 {
            let angle = i as f64 / 48.0 * std::f64::consts::TAU;
            let x = eye_w * angle.cos();
            let mut y = eye_h * angle.sin();
            if spent {
                y = y.max(-5.0 + x * if index == 0 { -0.28 } else { 0.28 });
            }
            if happy {
                y = y.min(0.0);
            }
            if asleep {
                y += 10.5;
            }
            let wave = if refreshing {
                (t * 7.0 + index as f64 * 1.5).sin() * 8.0
            } else {
                look_y
            };
            let (x, y) = project(cx + x + glance, 126.0 + y + wave);
            if i == 0 {
                path.begin_at(x, y);
            } else {
                path.line(x, y);
            }
        }
        path.end_closed();
    }
    let geometry = path.finish()?;
    let brush = p.brush(style.color)?;
    p.fill_geometry(&geometry, &brush);
    let antenna_x = 120.0
        + if working {
            (t * 5.0).sin() * 8.0
        } else if animated {
            (t * 2.0).sin() * 3.0
        } else {
            0.0
        };
    let (x, y) = project(antenna_x, 12.0);
    let antenna = crate::d2d::circle_geometry(
        p.engine,
        crate::d2d::point(x as f32, y as f32),
        (15.0 * scale) as f32,
    )?;
    p.fill_geometry(&antenna, &brush);
    Ok(())
}

fn outline(shape: u8) -> &'static Vec<(f64, f64)> {
    static SHAPES: std::sync::OnceLock<[Vec<(f64, f64)>; 3]> = std::sync::OnceLock::new();
    &SHAPES.get_or_init(|| {
        let q = QUARTER;
        let mut points = q.to_vec();
        points.extend(q[..q.len() - 1].iter().rev().map(|(x, y)| (*x, -*y)));
        points.extend(q[1..].iter().map(|(x, y)| (-*x, -*y)));
        points.extend(q[1..q.len() - 1].iter().rev().map(|(x, y)| (-*x, *y)));
        let base: Vec<_> = points
            .into_iter()
            .map(|(x, y)| {
                let mut y = 120.0 + y;
                let x = 120.0 + x;
                let smooth = |v: f64| v * v * (3.0 - 2.0 * v);
                let shift = if y <= 100.0 {
                    20.0
                } else if y < 120.0 {
                    20.0 * smooth((120.0 - y) / 20.0)
                } else if y >= 140.0 {
                    -20.0
                } else {
                    -20.0 * smooth((y - 120.0) / 20.0)
                };
                y += shift;
                (x, y)
            })
            .collect();
        std::array::from_fn(|shape| {
            base.iter()
                .map(|&(x, y)| {
                    let (x, y) = if shape == 0 {
                        (x, y)
                    } else {
                        let theta = (y - 120.0).atan2(x - 120.0);
                        let exponent = if shape == 1 { 10.0 } else { 2.2 };
                        let radius = 1.0
                            / ((theta.cos().abs() / 100.0).powf(exponent)
                                + (theta.sin().abs() / 80.0).powf(exponent))
                            .powf(1.0 / exponent);
                        (120.0 + theta.cos() * radius, 120.0 + theta.sin() * radius)
                    };
                    (7.2 + x * 0.94, 7.2 + y * 0.94)
                })
                .collect()
        })
    })[shape.min(2) as usize]
}
const QUARTER: &[(f64, f64)] = &[
    (0.0, -100.0),
    (12.8, -99.95),
    (19.87, -99.8),
    (25.68, -99.56),
    (30.79, -99.22),
    (35.43, -98.78),
    (39.71, -98.24),
    (43.69, -97.6),
    (47.44, -96.86),
    (50.99, -96.03),
    (54.34, -95.1),
    (57.53, -94.07),
    (60.57, -92.93),
    (63.46, -91.7),
    (66.22, -90.37),
    (68.85, -88.94),
    (71.36, -87.41),
    (73.75, -85.78),
    (76.03, -84.04),
    (78.19, -82.2),
    (80.25, -80.25),
    (82.2, -78.19),
    (84.04, -76.03),
    (85.78, -73.75),
    (87.41, -71.36),
    (88.94, -68.85),
    (90.37, -66.22),
    (91.7, -63.46),
    (92.93, -60.57),
    (94.07, -57.53),
    (95.1, -54.34),
    (96.03, -50.99),
    (96.86, -47.44),
    (97.6, -43.69),
    (98.24, -39.71),
    (98.78, -35.43),
    (99.22, -30.79),
    (99.56, -25.68),
    (99.8, -19.87),
    (99.95, -12.8),
    (100.0, 0.0),
];
