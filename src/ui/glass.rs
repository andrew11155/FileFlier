//! Frosted-glass rendering: a soft color backdrop and translucent panels with a sheen.
//!
//! This works on every platform. Where the desktop can show through ("see-through",
//! with compositor blur on KDE Plasma / macOS) the backdrop is skipped and the
//! window background is left translucent instead.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

use crate::theme::Palette;

pub fn with_alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// A smooth radial gradient: `color` at the center fading to transparent.
fn blob(p: &Painter, center: Pos2, radius: f32, color: Color32) {
    let mut mesh = Mesh::default();
    mesh.vertices.push(Vertex { pos: center, uv: WHITE_UV, color });
    let n = 48;
    // Two rings: a gentle mid-falloff reads much softer than a single linear fade.
    let mid = with_alpha(color, color.a() as f32 / 255.0 * 0.45);
    for i in 0..n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let d = vec2(a.cos(), a.sin());
        mesh.vertices.push(Vertex { pos: center + d * radius * 0.55, uv: WHITE_UV, color: mid });
        mesh.vertices.push(Vertex { pos: center + d * radius, uv: WHITE_UV, color: Color32::TRANSPARENT });
    }
    for i in 0..n as u32 {
        let (m0, o0) = (1 + 2 * i, 2 + 2 * i);
        let (m1, o1) = (1 + 2 * ((i + 1) % n as u32), 2 + 2 * ((i + 1) % n as u32));
        mesh.add_triangle(0, m0, m1);
        mesh.add_triangle(m0, o0, o1);
        mesh.add_triangle(m0, o1, m1);
    }
    p.add(Shape::mesh(mesh));
}

/// Soft, blurred-looking color field behind the glass panels (used when the desktop
/// doesn't show through). Drifts very slowly when animations are on.
pub fn backdrop(p: &Painter, rect: Rect, pal: &Palette, time: f64, animate: bool) {
    p.rect_filled(rect, 0.0, pal.bg);
    let t = if animate { time as f32 * 0.05 } else { 0.0 };
    let (w, h) = (rect.width(), rect.height());
    let r = w.max(h);
    let strength = if pal.is_dark { 0.55 } else { 0.38 };
    let blobs = [
        (0.15 + 0.05 * t.sin(), 0.20, 0.55, pal.accent),
        (0.85, 0.30 + 0.06 * (t * 0.8).cos(), 0.50, pal.folder),
        (0.55 + 0.06 * (t * 0.6).cos(), 0.95, 0.60, pal.accent_dim),
        (0.35, 0.65 + 0.05 * (t * 1.1).sin(), 0.35, pal.text_dim),
    ];
    for (x, y, rad, c) in blobs {
        blob(p, pos2(rect.left() + w * x, rect.top() + h * y), r * rad, with_alpha(c, strength));
    }
}

/// A translucent panel with a hairline highlight border and a faint top sheen.
pub fn panel(p: &Painter, rect: Rect, fill: Color32, radius: u8, pal: &Palette) {
    let cr = CornerRadius::same(radius);
    p.rect_filled(rect, cr, fill);
    // Sheen: brighter at the top, fading out over ~60px.
    let sheen_h = rect.height().min(60.0);
    let top = Color32::from_white_alpha(if pal.is_dark { 10 } else { 40 });
    let mut mesh = Mesh::default();
    let r = Rect::from_min_size(rect.min + vec2(radius as f32 * 0.5, 1.0), vec2(rect.width() - radius as f32, sheen_h));
    for (pos, c) in [
        (r.left_top(), top),
        (r.right_top(), top),
        (r.right_bottom(), Color32::TRANSPARENT),
        (r.left_bottom(), Color32::TRANSPARENT),
    ] {
        mesh.vertices.push(Vertex { pos, uv: WHITE_UV, color: c });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(Shape::mesh(mesh));
    let edge = if pal.is_dark { Color32::from_white_alpha(22) } else { Color32::from_black_alpha(22) };
    p.rect_stroke(rect, cr, Stroke::new(1.0, edge), StrokeKind::Inside);
}
