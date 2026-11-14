//! Pulse's default solid-black rail, translated from DockBerthShape.
// Portions adapted from Pulse, Copyright (c) 2026 qunqin24, Apache-2.0.
use crate::d2d::{D2DEngine, PathBuilder};
use crate::geometry::Edge;
use windows::core::Interface;
use windows::Win32::Graphics::Direct2D::Common::D2D1_FILL_MODE_ALTERNATE;
use windows::Win32::Graphics::Direct2D::ID2D1PathGeometry;

pub fn shape(
    engine: &D2DEngine,
    width: f64,
    height: f64,
    scale: f64,
    edge: Edge,
    docked: bool,
    openness: f64,
) -> windows::core::Result<ID2D1PathGeometry> {
    if !docked {
        return crate::d2d::rounded_rect_geometry(
            engine,
            crate::d2d::rect(0.0, 0.0, width as f32, height as f32),
            (width.min(height) / 2.0) as f32,
        );
    }
    let (w, h) = if edge.is_vertical() {
        (width, height)
    } else {
        (height, width)
    };
    let openness = openness.clamp(0.0, 1.0);
    let full_w = w;
    let full_h = h;
    let w = 6.0 * scale + (w - 6.0 * scale) * openness;
    let h = 96.0 * scale + (h - 96.0 * scale) * openness;
    let f = (24.0 * scale * openness).min(h / 2.0);
    let r = ((6.0 + 20.0 * openness) * scale)
        .min(w)
        .min((h - 2.0 * f) / 2.0)
        .max(0.0);
    let fw = (38.0 * scale * openness).min(w - r).max(0.0);
    let mut points = vec![(r, f), (w - fw, f)];
    let turn = unit_turn();
    points.extend(
        turn.iter()
            .skip(1)
            .map(|&(x, y)| (w - fw + x * fw, f - y * f)),
    );
    points.push((w, h));
    points.extend(turn.iter().skip(1).map(|&(x, y)| (w - y * fw, h - x * f)));
    points.push((r, h - f));
    for step in 1..=48 {
        let t = step as f64 / 48.0 * std::f64::consts::FRAC_PI_2;
        points.push((r - r * t.sin().sqrt(), h - f - r + r * t.cos().sqrt()));
    }
    points.push((0.0, f + r));
    for step in 1..=48 {
        let t = step as f64 / 48.0 * std::f64::consts::FRAC_PI_2;
        points.push((r - r * t.cos().sqrt(), f + r - r * t.sin().sqrt()));
    }
    let along_offset = (full_h - h) / 2.0;
    let transform = |(x, y): (f64, f64)| match edge {
        Edge::Right => (full_w - w + x, along_offset + y),
        Edge::Left => (w - x, along_offset + y),
        Edge::Top => (along_offset + y, w - x),
        Edge::Bottom => (along_offset + h - y, full_w - w + x),
    };
    let mut path = PathBuilder::new(engine, D2D1_FILL_MODE_ALTERNATE)?;
    let (x, y) = transform(points[0]);
    path.begin_at(x, y);
    for point in points.into_iter().skip(1) {
        let (x, y) = transform(point);
        path.line(x, y);
    }
    path.finish()
}

// The 32 midpoint samples of upstream's curvature-eased flare.
fn unit_turn() -> Vec<(f64, f64)> {
    let mut result = vec![(0.0, 0.0)];
    let (mut x, mut y) = (0.0, 0.0);
    for i in 0..32 {
        let s = (i as f64 + 0.5) / 32.0;
        let eased = (1.0 - (std::f64::consts::PI * s).cos()) / 2.0;
        let heading = std::f64::consts::FRAC_PI_2 * (0.5 * s + 0.5 * eased);
        x += heading.cos() / 32.0;
        y += heading.sin() / 32.0;
        result.push((x, y));
    }
    for point in &mut result {
        point.0 /= x;
        point.1 /= y;
    }
    result
}

/// Ring contents follow the upstream's delayed fade-in and faster fade-out.
/// `from` allows a reversal to continue from the currently visible value.
pub fn content_opacity(hidden: bool, from: f64, elapsed_ms: i64) -> f64 {
    let elapsed = elapsed_ms.max(0) as f64;
    let (delay, duration, target) = if hidden {
        (0.0, 100.0, 0.0)
    } else {
        (120.0, 180.0, 1.0)
    };
    let t = ((elapsed - delay) / duration).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t) * (1.0 - t);
    from.clamp(0.0, 1.0) + (target - from.clamp(0.0, 1.0)) * eased
}

/// Draw the upstream bubble's 250-point body and its curved 20-point tail.
pub fn bubble(
    painter: &crate::d2d::Painter,
    width: f64,
    height: f64,
    scale: f64,
    edge: Edge,
    pointer: f64,
    alpha: f32,
    glass: bool,
    backdrop: Option<&windows::Win32::Graphics::Direct2D::ID2D1Bitmap>,
) -> windows::core::Result<()> {
    let vertical = edge.is_vertical();
    let (w, h) = if vertical {
        (width, height)
    } else {
        (height, width)
    };
    let leads = matches!(edge, Edge::Left | Edge::Bottom);
    let pointer_width = 20.0 * scale;
    let body_x = if leads { pointer_width } else { 0.0 };
    let body_width = (w - pointer_width).max(0.0);
    let radius = (20.0 * scale).min(body_width / 2.0).min(h / 2.0);
    let half = (20.0 * scale).min((h / 2.0 - radius).max(0.0));
    let ring = pointer.clamp(0.0, h);
    let centre = ring.clamp(radius + half, (h - radius - half).max(radius + half));
    let tip = ring.clamp(centre - half, centre + half);
    let base_x = if leads { body_x } else { body_x + body_width };
    let tip_x = if leads { 0.0 } else { w };
    let reach = tip_x - base_x;
    let sweep = if leads { -half } else { half };
    let transform = |x: f64, y: f64| if vertical { (x, y) } else { (y, w - x) };
    let body = if vertical {
        crate::d2d::rect(body_x as f32, 0.0, body_width as f32, h as f32)
    } else {
        crate::d2d::rect(
            0.0,
            (w - body_x - body_width) as f32,
            h as f32,
            body_width as f32,
        )
    };
    let brush = painter.brush(if glass {
        crate::d2d::Rgba::new(0.05, 0.07, 0.09, 0.22)
    } else {
        crate::d2d::Rgba::new(0.0, 0.0, 0.0, alpha)
    })?;
    let mut path = PathBuilder::new(painter.engine, D2D1_FILL_MODE_ALTERNATE)?;
    let (x, y) = transform(base_x, centre - sweep);
    path.begin_at(x, y);
    let a = transform(base_x + reach * 0.24, centre - sweep * 0.44);
    let b = transform(base_x + reach * 0.55, (tip - sweep * 0.24).clamp(0.0, h));
    let c = transform(tip_x, tip);
    path.curve(a.0, a.1, b.0, b.1, c.0, c.1);
    let a = transform(base_x + reach * 0.55, (tip + sweep * 0.24).clamp(0.0, h));
    let b = transform(base_x + reach * 0.24, centre + sweep * 0.44);
    let c = transform(base_x, centre + sweep);
    path.curve(a.0, a.1, b.0, b.1, c.0, c.1);
    let a = transform(base_x - reach * 0.08, centre + sweep);
    let b = transform(base_x - reach * 0.08, centre - sweep);
    path.line(a.0, a.1);
    path.line(b.0, b.1);
    let geometry = path.finish()?;
    // Union the rounded body and pointer before painting either layer. Drawing
    // them separately doubles their alpha in the overlap and creates a seam.
    let body_shape = crate::d2d::rounded_rect_geometry(painter.engine, body, radius as f32)?;
    let union = unsafe {
        let combined = painter.engine.factory.CreatePathGeometry()?;
        let sink = combined.Open()?;
        body_shape.CombineWithGeometry(
            &geometry,
            windows::Win32::Graphics::Direct2D::D2D1_COMBINE_MODE_UNION,
            None,
            0.25,
            &sink,
        )?;
        sink.Close()?;
        combined
    };
    if let Some(bitmap) = backdrop {
        crate::liquid_glass::paint_masked(
            painter,
            bitmap,
            &union.cast()?,
            width as f32,
            height as f32,
            alpha,
        );
    }
    if glass {
        use windows::Win32::Graphics::Direct2D::{
            ID2D1Layer, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_LAYER_OPTIONS_NONE,
            D2D1_LAYER_PARAMETERS,
        };
        let layer = D2D1_LAYER_PARAMETERS {
            contentBounds: crate::d2d::rect(0.0, 0.0, width as f32, height as f32),
            geometricMask: std::mem::ManuallyDrop::new(None),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: crate::d2d::identity_matrix(),
            opacity: alpha.clamp(0.0, 1.0),
            opacityBrush: std::mem::ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS_NONE,
        };
        unsafe {
            painter.rt.PushLayer(&layer, None::<&ID2D1Layer>);
        }
    }
    painter.fill_geometry(&union, &brush);
    if glass {
        unsafe {
            painter.rt.PopLayer();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn content_fade_has_upstream_delays_and_reverses_without_a_jump() {
        use super::content_opacity;
        assert_eq!(content_opacity(false, 0.0, 120), 0.0);
        assert_eq!(content_opacity(false, 0.0, 300), 1.0);
        assert_eq!(content_opacity(true, 1.0, 100), 0.0);
        let partial = content_opacity(false, 0.0, 210);
        assert!((content_opacity(true, partial, 0) - partial).abs() < 1e-9);
        assert!(content_opacity(true, partial, 50) < partial);
    }
    #[test]
    fn flare_turn_is_normalized_monotone_and_symmetric() {
        let points = super::unit_turn();
        assert_eq!(points.len(), 33);
        assert_eq!(points[0], (0.0, 0.0));
        let end = points[32];
        assert!((end.0 - 1.0).abs() < 1e-12 && (end.1 - 1.0).abs() < 1e-12);
        for i in 1..points.len() {
            assert!(points[i].0 > points[i - 1].0 && points[i].1 > points[i - 1].1);
            assert!((points[i].0 - (1.0 - points[32 - i].1)).abs() < 1e-12);
        }
    }
}
