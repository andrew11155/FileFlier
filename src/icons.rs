//! Hand-drawn vector icons, so the UI looks the same everywhere without icon fonts.

use egui::epaint::{CubicBezierShape, PathShape, PathStroke};
use egui::{Color32, Painter, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};

use crate::theme::Palette;

fn line(p: &Painter, pts: &[Pos2], c: Color32, w: f32) {
    p.add(PathShape::line(pts.to_vec(), PathStroke::new(w, c)));
}

/// Maps unit coordinates (0..1) inside `r` to screen positions.
fn at(r: Rect, x: f32, y: f32) -> Pos2 {
    pos2(r.left() + r.width() * x, r.top() + r.height() * y)
}

fn square(r: Rect) -> Rect {
    let s = r.width().min(r.height());
    Rect::from_center_size(r.center(), vec2(s, s))
}

// --------------------------------------------------------------- files

pub fn folder(p: &Painter, r: Rect, pal: &Palette) {
    let r = square(r);
    let back = Rect::from_min_max(at(r, 0.06, 0.16), at(r, 0.94, 0.84));
    // Tab on the back sheet.
    p.add(PathShape::convex_polygon(
        vec![at(r, 0.06, 0.2), at(r, 0.1, 0.14), at(r, 0.38, 0.14), at(r, 0.46, 0.24), at(r, 0.06, 0.24)],
        pal.folder_back,
        Stroke::NONE,
    ));
    p.rect_filled(back.with_min_y(at(r, 0.0, 0.22).y), 2.0, pal.folder_back);
    let front = Rect::from_min_max(at(r, 0.06, 0.32), at(r, 0.94, 0.84));
    p.rect_filled(front, 2.0, pal.folder);
    // Soft highlight along the front's top edge.
    p.line_segment(
        [at(r, 0.1, 0.35), at(r, 0.9, 0.35)],
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 70)),
    );
}

pub fn folder_outline(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    line(
        p,
        &[
            at(r, 0.1, 0.2),
            at(r, 0.4, 0.2),
            at(r, 0.48, 0.3),
            at(r, 0.9, 0.3),
            at(r, 0.9, 0.82),
            at(r, 0.1, 0.82),
            at(r, 0.1, 0.2),
        ],
        c,
        1.4,
    );
}

pub fn file_outline(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    line(
        p,
        &[at(r, 0.24, 0.1), at(r, 0.58, 0.1), at(r, 0.78, 0.3), at(r, 0.78, 0.9), at(r, 0.24, 0.9), at(r, 0.24, 0.1)],
        c,
        1.4,
    );
    line(p, &[at(r, 0.58, 0.1), at(r, 0.58, 0.3), at(r, 0.78, 0.3)], c, 1.2);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Plain,
    Image,
    Audio,
    Video,
    Archive,
    Code,
    Document,
    Executable,
}

impl FileKind {
    pub fn from_ext(ext: &str) -> Self {
        match ext {
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" | "ico" | "tif" | "tiff" => Self::Image,
            "mp3" | "flac" | "wav" | "ogg" | "m4a" | "opus" | "aac" => Self::Audio,
            "mp4" | "mkv" | "webm" | "avi" | "mov" | "wmv" => Self::Video,
            "zip" | "tar" | "gz" | "xz" | "bz2" | "7z" | "rar" | "zst" | "deb" | "rpm" | "tgz" => Self::Archive,
            "rs" | "c" | "h" | "cpp" | "hpp" | "py" | "js" | "ts" | "go" | "java" | "sh" | "toml" | "json" | "yaml"
            | "yml" | "html" | "css" | "lua" | "rb" | "zig" | "kt" | "cs" => Self::Code,
            "pdf" | "doc" | "docx" | "odt" | "md" | "txt" | "rtf" | "csv" | "xls" | "xlsx" => Self::Document,
            "appimage" | "bin" | "run" | "exe" => Self::Executable,
            _ => Self::Plain,
        }
    }

    fn color(self) -> Option<Color32> {
        Some(match self {
            Self::Plain => return None,
            Self::Image => Color32::from_rgb(38, 132, 214),
            Self::Audio => Color32::from_rgb(214, 86, 150),
            Self::Video => Color32::from_rgb(142, 90, 220),
            Self::Archive => Color32::from_rgb(196, 138, 60),
            Self::Code => Color32::from_rgb(60, 170, 110),
            Self::Document => Color32::from_rgb(110, 128, 150),
            Self::Executable => Color32::from_rgb(210, 80, 70),
        })
    }
}

pub fn file(p: &Painter, r: Rect, kind: FileKind, pal: &Palette) {
    let r = square(r);
    let (l, t, rr, b) = (0.2, 0.08, 0.8, 0.92);
    let fold = 0.2;
    let body = vec![at(r, l, t), at(r, rr - fold, t), at(r, rr, t + fold * 0.9), at(r, rr, b), at(r, l, b)];
    p.add(PathShape::convex_polygon(body, pal.page, Stroke::new(1.0, Color32::from_black_alpha(60))));
    p.add(PathShape::convex_polygon(
        vec![at(r, rr - fold, t), at(r, rr, t + fold * 0.9), at(r, rr - fold, t + fold * 0.9)],
        Color32::from_rgb(196, 202, 210),
        Stroke::NONE,
    ));
    let Some(c) = kind.color() else {
        for y in [0.45, 0.58, 0.71] {
            p.line_segment([at(r, 0.3, y), at(r, 0.7, y)], Stroke::new(1.0, Color32::from_rgb(170, 176, 184)));
        }
        return;
    };
    let badge = Rect::from_min_max(at(r, 0.28, 0.42), at(r, 0.72, 0.82));
    p.rect_filled(badge, 1.5, c);
    match kind {
        Kind::Image => {
            // Little mountain + sun.
            p.add(PathShape::convex_polygon(
                vec![at(badge, 0.08, 0.9), at(badge, 0.42, 0.42), at(badge, 0.7, 0.9)],
                Color32::WHITE,
                Stroke::NONE,
            ));
            p.circle_filled(at(badge, 0.72, 0.3), badge.width() * 0.12, Color32::WHITE);
        }
        Kind::Audio => {
            let s = Stroke::new(1.2, Color32::WHITE);
            p.line_segment([at(badge, 0.62, 0.2), at(badge, 0.62, 0.72)], s);
            p.circle_filled(at(badge, 0.48, 0.74), badge.width() * 0.13, Color32::WHITE);
        }
        Kind::Video => {
            p.add(PathShape::convex_polygon(
                vec![at(badge, 0.36, 0.25), at(badge, 0.72, 0.5), at(badge, 0.36, 0.75)],
                Color32::WHITE,
                Stroke::NONE,
            ));
        }
        Kind::Archive => {
            for y in [0.2, 0.4, 0.6] {
                p.rect_filled(
                    Rect::from_center_size(at(badge, 0.5, y), vec2(badge.width() * 0.22, 1.6)),
                    0.0,
                    Color32::WHITE,
                );
            }
        }
        Kind::Code => {
            line(p, &[at(badge, 0.4, 0.28), at(badge, 0.2, 0.5), at(badge, 0.4, 0.72)], Color32::WHITE, 1.2);
            line(p, &[at(badge, 0.6, 0.28), at(badge, 0.8, 0.5), at(badge, 0.6, 0.72)], Color32::WHITE, 1.2);
        }
        Kind::Document | Kind::Executable | Kind::Plain => {
            for y in [0.3, 0.5, 0.7] {
                p.line_segment([at(badge, 0.2, y), at(badge, 0.8, y)], Stroke::new(1.0, Color32::WHITE));
            }
        }
    }
}
use FileKind as Kind;

// --------------------------------------------------------------- glyphs

pub fn arrow_left(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    line(p, &[at(r, 0.18, 0.5), at(r, 0.84, 0.5)], c, 1.6);
    line(p, &[at(r, 0.46, 0.2), at(r, 0.16, 0.5), at(r, 0.46, 0.8)], c, 1.6);
}

pub fn arrow_right(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    line(p, &[at(r, 0.16, 0.5), at(r, 0.82, 0.5)], c, 1.6);
    line(p, &[at(r, 0.54, 0.2), at(r, 0.84, 0.5), at(r, 0.54, 0.8)], c, 1.6);
}

pub fn arrow_up(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    line(p, &[at(r, 0.5, 0.18), at(r, 0.5, 0.84)], c, 1.6);
    line(p, &[at(r, 0.2, 0.46), at(r, 0.5, 0.16), at(r, 0.8, 0.46)], c, 1.6);
}

pub fn chevron_down(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    line(p, &[at(r, 0.28, 0.4), at(r, 0.5, 0.62), at(r, 0.72, 0.4)], c, 1.4);
}

pub fn chevron_right(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    line(p, &[at(r, 0.4, 0.28), at(r, 0.62, 0.5), at(r, 0.4, 0.72)], c, 1.4);
}

pub fn sort_arrow(p: &Painter, r: Rect, c: Color32, descending: bool) {
    let r = square(r);
    if descending {
        line(p, &[at(r, 0.5, 0.18), at(r, 0.5, 0.82)], c, 1.3);
        line(p, &[at(r, 0.28, 0.6), at(r, 0.5, 0.82), at(r, 0.72, 0.6)], c, 1.3);
    } else {
        line(p, &[at(r, 0.5, 0.82), at(r, 0.5, 0.18)], c, 1.3);
        line(p, &[at(r, 0.28, 0.4), at(r, 0.5, 0.18), at(r, 0.72, 0.4)], c, 1.3);
    }
}

pub fn lock(p: &Painter, r: Rect, c: Color32, locked: bool) {
    let r = square(r);
    let body = Rect::from_min_max(at(r, 0.22, 0.46), at(r, 0.78, 0.86));
    p.rect_stroke(body, 2.0, Stroke::new(1.5, c), StrokeKind::Middle);
    p.circle_filled(at(r, 0.5, 0.66), 1.4, c);
    // Shackle; an open lock lifts the right leg.
    let right_leg = if locked { 0.46 } else { 0.34 };
    p.add(CubicBezierShape::from_points_stroke(
        [at(r, 0.32, 0.46), at(r, 0.3, 0.1), at(r, 0.7, 0.1), at(r, 0.68, 0.3)],
        false,
        Color32::TRANSPARENT,
        Stroke::new(1.5, c),
    ));
    line(p, &[at(r, 0.68, 0.3), at(r, 0.68, right_leg)], c, 1.5);
}

pub fn bookmark(p: &Painter, r: Rect, c: Color32, filled: bool) {
    let r = square(r);
    let pts = vec![at(r, 0.26, 0.14), at(r, 0.74, 0.14), at(r, 0.74, 0.86), at(r, 0.5, 0.66), at(r, 0.26, 0.86)];
    if filled {
        // Concave shape: fill as two convex halves.
        p.add(PathShape::convex_polygon(vec![pts[0], pts[1], pts[2], pts[3]], c, Stroke::NONE));
        p.add(PathShape::convex_polygon(vec![pts[0], pts[3], pts[4]], c, Stroke::NONE));
    }
    p.add(PathShape::closed_line(pts, Stroke::new(1.5, c)));
}

pub fn dots_vertical(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    for y in [0.25, 0.5, 0.75] {
        p.circle_filled(at(r, 0.5, y), 1.6, c);
    }
}

pub fn search(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    p.circle_stroke(at(r, 0.42, 0.42), r.width() * 0.26, Stroke::new(1.5, c));
    line(p, &[at(r, 0.61, 0.61), at(r, 0.86, 0.86)], c, 1.8);
}

pub fn plus(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    line(p, &[at(r, 0.5, 0.2), at(r, 0.5, 0.8)], c, 1.5);
    line(p, &[at(r, 0.2, 0.5), at(r, 0.8, 0.5)], c, 1.5);
}

pub fn close(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    line(p, &[at(r, 0.25, 0.25), at(r, 0.75, 0.75)], c, 1.4);
    line(p, &[at(r, 0.75, 0.25), at(r, 0.25, 0.75)], c, 1.4);
}

pub fn minimize(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    line(p, &[at(r, 0.22, 0.5), at(r, 0.78, 0.5)], c, 1.0);
}

pub fn maximize(p: &Painter, r: Rect, c: Color32, restored: bool) {
    let r = square(r);
    if restored {
        p.rect_stroke(
            Rect::from_min_max(at(r, 0.22, 0.34), at(r, 0.66, 0.78)),
            1.0,
            Stroke::new(1.0, c),
            StrokeKind::Middle,
        );
        line(
            p,
            &[at(r, 0.34, 0.34), at(r, 0.34, 0.22), at(r, 0.78, 0.22), at(r, 0.78, 0.66), at(r, 0.66, 0.66)],
            c,
            1.0,
        );
    } else {
        p.rect_stroke(
            Rect::from_min_max(at(r, 0.24, 0.24), at(r, 0.76, 0.76)),
            1.0,
            Stroke::new(1.0, c),
            StrokeKind::Middle,
        );
    }
}

pub fn sliders(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    for (y, knob) in [(0.24, 0.66), (0.5, 0.34), (0.76, 0.58)] {
        line(p, &[at(r, 0.14, y), at(r, 0.86, y)], c, 1.2);
        let k = Rect::from_center_size(at(r, knob, y), vec2(r.width() * 0.18, r.height() * 0.2));
        p.rect_filled(k, 1.0, c);
    }
}

pub fn sidebar_toggle(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    line(p, &[at(r, 0.84, 0.18), at(r, 0.84, 0.82)], c, 1.5);
    line(p, &[at(r, 0.14, 0.5), at(r, 0.7, 0.5)], c, 1.5);
    line(p, &[at(r, 0.44, 0.24), at(r, 0.7, 0.5), at(r, 0.44, 0.76)], c, 1.5);
}

pub fn clock(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    p.circle_stroke(r.center(), r.width() * 0.36, Stroke::new(1.4, c));
    line(p, &[at(r, 0.5, 0.28), at(r, 0.5, 0.5), at(r, 0.66, 0.6)], c, 1.4);
}

pub fn drive(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    let body = Rect::from_min_max(at(r, 0.1, 0.36), at(r, 0.9, 0.72));
    p.rect_stroke(body, 2.0, Stroke::new(1.4, c), StrokeKind::Middle);
    p.circle_filled(at(r, 0.76, 0.54), 1.3, c);
    line(p, &[at(r, 0.18, 0.36), at(r, 0.3, 0.2), at(r, 0.7, 0.2), at(r, 0.82, 0.36)], c, 1.2);
}

pub fn layout_list(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    p.rect_stroke(Rect::from_min_max(at(r, 0.2, 0.14), at(r, 0.8, 0.86)), 1.5, Stroke::new(1.3, c), StrokeKind::Middle);
    for y in [0.38, 0.62] {
        line(p, &[at(r, 0.2, y), at(r, 0.8, y)], c, 1.1);
    }
}

pub fn logo(p: &Painter, r: Rect, pal: &Palette) {
    // A paper plane: File *Flier*.
    let r = square(r);
    let c = pal.text;
    p.add(PathShape::convex_polygon(vec![at(r, 0.08, 0.46), at(r, 0.92, 0.12), at(r, 0.46, 0.56)], c, Stroke::NONE));
    p.add(PathShape::convex_polygon(
        vec![at(r, 0.46, 0.56), at(r, 0.92, 0.12), at(r, 0.66, 0.88)],
        c.gamma_multiply(0.7),
        Stroke::NONE,
    ));
    p.add(PathShape::convex_polygon(
        vec![at(r, 0.46, 0.56), at(r, 0.52, 0.84), at(r, 0.6, 0.7)],
        c.gamma_multiply(0.45),
        Stroke::NONE,
    ));
}

/// Small colored glyphs for the sidebar's Places section.
#[derive(Clone, Copy)]
pub enum Place {
    Home,
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
    Trash,
    Folder,
    Bookmark,
    Recent,
    Drive,
    Network,
    Cloud,
}

pub fn place(p: &Painter, r: Rect, kind: Place, pal: &Palette) {
    let r = square(r);
    match kind {
        Place::Home => {
            let c = Color32::from_rgb(88, 170, 230);
            p.add(PathShape::convex_polygon(vec![at(r, 0.1, 0.5), at(r, 0.5, 0.12), at(r, 0.9, 0.5)], c, Stroke::NONE));
            p.rect_filled(Rect::from_min_max(at(r, 0.22, 0.48), at(r, 0.78, 0.88)), 1.0, c);
            p.rect_filled(Rect::from_min_max(at(r, 0.42, 0.62), at(r, 0.58, 0.88)), 0.0, pal.sidebar);
        }
        Place::Desktop => {
            let c = Color32::from_rgb(40, 150, 220);
            p.rect_filled(Rect::from_min_max(at(r, 0.08, 0.16), at(r, 0.92, 0.7)), 2.0, c);
            p.rect_filled(Rect::from_min_max(at(r, 0.3, 0.78), at(r, 0.7, 0.86)), 1.0, pal.text_dim);
        }
        Place::Documents => file(p, r, FileKind::Plain, pal),
        Place::Downloads => {
            let c = Color32::from_rgb(52, 190, 120);
            line(p, &[at(r, 0.5, 0.1), at(r, 0.5, 0.66)], c, 1.8);
            line(p, &[at(r, 0.24, 0.42), at(r, 0.5, 0.68), at(r, 0.76, 0.42)], c, 1.8);
            line(p, &[at(r, 0.16, 0.88), at(r, 0.84, 0.88)], c, 1.8);
        }
        Place::Music => {
            let c = Color32::from_rgb(232, 96, 84);
            p.circle_filled(r.center(), r.width() * 0.42, c);
            p.circle_filled(r.center(), r.width() * 0.12, pal.sidebar);
        }
        Place::Pictures => {
            let c = Color32::from_rgb(40, 140, 220);
            p.rect_filled(Rect::from_min_max(at(r, 0.1, 0.18), at(r, 0.9, 0.82)), 2.0, c);
            p.add(PathShape::convex_polygon(
                vec![at(r, 0.16, 0.76), at(r, 0.44, 0.4), at(r, 0.7, 0.76)],
                Color32::WHITE,
                Stroke::NONE,
            ));
            p.circle_filled(at(r, 0.7, 0.36), r.width() * 0.08, Color32::from_rgb(255, 220, 110));
        }
        Place::Videos => {
            let c = Color32::from_rgb(140, 92, 230);
            p.rect_filled(Rect::from_min_max(at(r, 0.1, 0.18), at(r, 0.9, 0.82)), 2.0, c);
            p.add(PathShape::convex_polygon(
                vec![at(r, 0.4, 0.34), at(r, 0.66, 0.5), at(r, 0.4, 0.66)],
                Color32::WHITE,
                Stroke::NONE,
            ));
        }
        Place::Trash => {
            let c = pal.text_dim;
            line(p, &[at(r, 0.16, 0.24), at(r, 0.84, 0.24)], c, 1.5);
            line(p, &[at(r, 0.4, 0.24), at(r, 0.42, 0.12), at(r, 0.58, 0.12), at(r, 0.6, 0.24)], c, 1.3);
            line(p, &[at(r, 0.24, 0.3), at(r, 0.3, 0.88), at(r, 0.7, 0.88), at(r, 0.76, 0.3)], c, 1.5);
        }
        Place::Drive => drive(p, r, pal.text_dim),
        Place::Network => {
            // Drive with a network "stem".
            let c = pal.text_dim;
            let body = Rect::from_min_max(at(r, 0.12, 0.12), at(r, 0.88, 0.5));
            p.rect_stroke(body, 2.0, Stroke::new(1.4, c), StrokeKind::Middle);
            p.circle_filled(at(r, 0.74, 0.31), 1.3, c);
            line(p, &[at(r, 0.5, 0.5), at(r, 0.5, 0.78)], c, 1.4);
            line(p, &[at(r, 0.16, 0.84), at(r, 0.84, 0.84)], c, 1.4);
            p.circle_filled(at(r, 0.5, 0.84), 2.2, c);
        }
        Place::Cloud => {
            let c = Color32::from_rgb(88, 170, 230);
            p.circle_filled(at(r, 0.36, 0.58), r.width() * 0.2, c);
            p.circle_filled(at(r, 0.58, 0.46), r.width() * 0.26, c);
            p.circle_filled(at(r, 0.76, 0.62), r.width() * 0.16, c);
            p.rect_filled(Rect::from_min_max(at(r, 0.2, 0.6), at(r, 0.86, 0.78)), 3.0, c);
        }
        Place::Folder => folder(p, r, pal),
        Place::Bookmark => bookmark(p, r, pal.text_dim, false),
        Place::Recent => clock(p, r, pal.text_dim),
    }
}
