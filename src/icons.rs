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
    let r = snap(p, square(r));
    let lighter = |c: Color32, t: f32| mix(c, Color32::WHITE, t);
    // Back sheet with its tab.
    gradient(
        p,
        PathShape::convex_polygon(
            vec![at(r, 0.06, 0.2), at(r, 0.1, 0.13), at(r, 0.38, 0.13), at(r, 0.46, 0.23), at(r, 0.06, 0.23)],
            Color32::WHITE,
            Stroke::NONE,
        )
        .into(),
        lighter(pal.folder_back, 0.08),
        pal.folder_back,
    );
    let back = Rect::from_min_max(at(r, 0.06, 0.21), at(r, 0.94, 0.86));
    p.rect_filled(back, 2.0, pal.folder_back);
    // Front sheet: a soft top-to-bottom gradient gives it some depth.
    let front = Rect::from_min_max(at(r, 0.06, 0.32), at(r, 0.94, 0.86));
    gradient(p, egui::Shape::rect_filled(front, 2.0, Color32::WHITE), lighter(pal.folder, 0.22), pal.folder);
    let hl = (r.width() / 20.0).max(1.0);
    p.line_segment(
        [pos2(front.left() + 1.5, front.top() + hl * 0.5), pos2(front.right() - 1.5, front.top() + hl * 0.5)],
        Stroke::new(hl, Color32::from_rgba_unmultiplied(255, 255, 255, 90)),
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
    /// Unknown or binary data: a blank page.
    Plain,
    /// Plain text: a page with lines.
    Text,
    Image,
    Audio,
    Video,
    Archive,
    Code,
    Document,
    Spreadsheet,
    Presentation,
    Pdf,
    Font,
    Executable,
}

impl FileKind {
    pub fn from_ext(ext: &str) -> Self {
        match ext {
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" | "ico" | "tif" | "tiff" | "heic" | "heif"
            | "avif" | "jxl" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "raf" | "orf" | "rw2" | "tga" | "exr"
            | "hdr" | "psd" | "xcf" | "kra" => Self::Image,
            "mp3" | "flac" | "wav" | "ogg" | "m4a" | "opus" | "aac" | "aiff" | "wma" | "m4b" | "mid" | "midi" => {
                Self::Audio
            }
            "mp4" | "mkv" | "webm" | "avi" | "mov" | "wmv" | "m4v" | "mpg" | "mpeg" | "3gp" | "mts" | "ogv" => {
                Self::Video
            }
            "zip" | "tar" | "gz" | "xz" | "bz2" | "7z" | "rar" | "zst" | "deb" | "rpm" | "tgz" | "tbz2" | "txz"
            | "lz" | "lzma" | "cab" | "jar" | "apk" | "iso" | "img" | "dmg" => Self::Archive,
            "rs" | "c" | "h" | "cpp" | "cc" | "hpp" | "py" | "js" | "mjs" | "ts" | "jsx" | "tsx" | "go" | "java"
            | "sh" | "bash" | "zsh" | "fish" | "toml" | "json" | "yaml" | "yml" | "html" | "htm" | "css" | "scss"
            | "lua" | "rb" | "zig" | "kt" | "cs" | "php" | "swift" | "dart" | "scala" | "sql" | "xml" | "vue"
            | "svelte" | "nix" | "hs" | "ex" | "exs" | "el" | "vim" | "ini" | "conf" | "cfg" => Self::Code,
            "txt" | "md" | "markdown" | "log" | "rst" | "org" | "nfo" => Self::Text,
            "doc" | "docx" | "odt" | "fodt" | "rtf" | "pages" | "epub" | "wpd" | "tex" => Self::Document,
            "xls" | "xlsx" | "ods" | "fods" | "csv" | "tsv" | "numbers" => Self::Spreadsheet,
            "ppt" | "pptx" | "odp" | "fodp" | "key" => Self::Presentation,
            "pdf" | "xps" | "djvu" => Self::Pdf,
            "ttf" | "otf" | "woff" | "woff2" | "ttc" => Self::Font,
            "appimage" | "run" | "exe" | "msi" | "flatpak" | "snap" => Self::Executable,
            _ => Self::Plain,
        }
    }

    /// The icon's color; `None` for the neutral paper-colored kinds.
    fn color(self) -> Option<Color32> {
        Some(match self {
            Self::Plain | Self::Text => return None,
            Self::Image => Color32::from_rgb(16, 165, 200),
            Self::Audio => Color32::from_rgb(230, 72, 142),
            Self::Video => Color32::from_rgb(139, 92, 246),
            Self::Archive => Color32::from_rgb(224, 150, 28),
            Self::Code => Color32::from_rgb(14, 163, 150),
            Self::Document => Color32::from_rgb(47, 123, 245),
            Self::Spreadsheet => Color32::from_rgb(30, 160, 84),
            Self::Presentation => Color32::from_rgb(242, 107, 42),
            Self::Pdf => Color32::from_rgb(229, 65, 60),
            Self::Font => Color32::from_rgb(120, 132, 150),
            Self::Executable => Color32::from_rgb(92, 106, 128),
        })
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// Snaps an icon's box to whole physical pixels so its straight edges stay sharp.
fn snap(p: &Painter, r: Rect) -> Rect {
    let ppp = p.pixels_per_point();
    let s = (r.width() * ppp).round() / ppp;
    let min = pos2((r.left() * ppp).round() / ppp, (r.top() * ppp).round() / ppp);
    Rect::from_min_size(min, vec2(s, s))
}

/// Paints `shape` (drawn in opaque white) with a vertical gradient, keeping its
/// anti-aliased edges: the shape is tessellated and each vertex recolored by height.
fn gradient(p: &Painter, shape: egui::Shape, top: Color32, bottom: Color32) {
    use egui::epaint::{Mesh, Tessellator};
    let bounds = shape.visual_bounding_rect();
    let options = p.ctx().options(|o| o.tessellation_options);
    let mut tess = Tessellator::new(p.pixels_per_point(), options, [1, 1], vec![]);
    let mut mesh = Mesh::default();
    tess.tessellate_shape(shape, &mut mesh);
    let h = bounds.height().max(1.0);
    for v in &mut mesh.vertices {
        let t = ((v.pos.y - bounds.top()) / h).clamp(0.0, 1.0);
        v.color = mix(top, bottom, t).linear_multiply(v.color.a() as f32 / 255.0);
    }
    p.add(mesh);
}

pub fn file(p: &Painter, r: Rect, kind: FileKind, pal: &Palette) {
    let r = snap(p, square(r));
    let (l, t, rr, b) = (0.17, 0.05, 0.83, 0.95);
    let fold = 0.26;
    let fy = t + fold * r.width() / r.height();
    let body = vec![at(r, l, t), at(r, rr - fold, t), at(r, rr, fy), at(r, rr, b), at(r, l, b)];
    let corner = vec![at(r, rr - fold, t), at(r, rr, fy), at(r, rr - fold + 0.04, fy)];
    let px = (r.width() / 18.0).max(1.0);
    let Some(c) = kind.color() else {
        // Neutral paper.
        let (paper, edge) = if pal.is_dark {
            (Color32::from_rgb(232, 235, 240), Color32::from_rgb(196, 201, 209))
        } else {
            (Color32::WHITE, Color32::from_rgb(170, 176, 186))
        };
        let outline = if pal.is_dark { Stroke::NONE } else { Stroke::new(px * 0.9, edge) };
        gradient(
            p,
            PathShape::convex_polygon(body.clone(), Color32::WHITE, Stroke::NONE).into(),
            paper,
            mix(paper, edge, 0.35),
        );
        if outline != Stroke::NONE {
            p.add(PathShape::convex_polygon(body, Color32::TRANSPARENT, outline));
        }
        p.add(PathShape::convex_polygon(corner, edge, Stroke::NONE));
        if kind == FileKind::Text {
            let s = Stroke::new(px, mix(edge, Color32::BLACK, 0.2));
            for (y, w) in [(0.46, 0.66), (0.6, 0.66), (0.74, 0.5)] {
                p.line_segment([at(r, 0.3, y), at(r, 0.3 + (w - 0.3) * 1.0, y)], s);
            }
        }
        return;
    };
    gradient(
        p,
        PathShape::convex_polygon(body, Color32::WHITE, Stroke::NONE).into(),
        mix(c, Color32::WHITE, 0.16),
        mix(c, Color32::BLACK, 0.06),
    );
    p.add(PathShape::convex_polygon(corner, mix(c, Color32::WHITE, 0.5), Stroke::NONE));
    // White glyph in the lower part of the page.
    let g = Rect::from_min_max(at(r, 0.29, 0.42), at(r, 0.71, 0.84));
    let w = Color32::WHITE;
    let s = Stroke::new(px * 1.1, w);
    match kind {
        Kind::Image => {
            p.add(PathShape::convex_polygon(
                vec![at(g, 0.0, 0.92), at(g, 0.4, 0.38), at(g, 0.78, 0.92)],
                w,
                Stroke::NONE,
            ));
            p.add(PathShape::convex_polygon(
                vec![at(g, 0.5, 0.92), at(g, 0.74, 0.6), at(g, 1.0, 0.92)],
                w.gamma_multiply(0.8),
                Stroke::NONE,
            ));
            p.circle_filled(at(g, 0.78, 0.2), g.width() * 0.14, w);
        }
        Kind::Audio => {
            p.line_segment([at(g, 0.64, 0.08), at(g, 0.64, 0.74)], s);
            p.line_segment([at(g, 0.64, 0.1), at(g, 0.9, 0.24)], s);
            p.circle_filled(at(g, 0.46, 0.76), g.width() * 0.2, w);
        }
        Kind::Video => {
            p.add(PathShape::convex_polygon(
                vec![at(g, 0.26, 0.12), at(g, 0.86, 0.5), at(g, 0.26, 0.88)],
                w,
                Stroke::NONE,
            ));
        }
        Kind::Archive => {
            // Zipper.
            let zw = g.width() * 0.2;
            for (k, y) in [0.0, 0.2, 0.4, 0.6].into_iter().enumerate() {
                let x = if k % 2 == 0 { 0.4 } else { 0.6 };
                p.rect_filled(Rect::from_center_size(at(g, x, y + 0.06), vec2(zw, px * 1.4)), 0.0, w);
            }
            p.rect_filled(Rect::from_min_max(at(g, 0.36, 0.72), at(g, 0.64, 1.0)), px, w);
        }
        Kind::Code => {
            line(p, &[at(g, 0.36, 0.2), at(g, 0.06, 0.5), at(g, 0.36, 0.8)], w, px * 1.3);
            line(p, &[at(g, 0.64, 0.2), at(g, 0.94, 0.5), at(g, 0.64, 0.8)], w, px * 1.3);
        }
        Kind::Spreadsheet => {
            let cell = Rect::from_min_max(at(g, 0.0, 0.06), at(g, 1.0, 0.94));
            p.rect_stroke(cell, 0.5, s, StrokeKind::Inside);
            p.line_segment([at(cell, 0.42, 0.0), at(cell, 0.42, 1.0)], s);
            for y in [0.36, 0.66] {
                p.line_segment([at(cell, 0.0, y), at(cell, 1.0, y)], s);
            }
        }
        Kind::Presentation => {
            for (x, h) in [(0.12, 0.45), (0.42, 0.75), (0.72, 0.3)] {
                let top = 0.95 - h;
                p.rect_filled(Rect::from_min_max(at(g, x, top), at(g, x + 0.2, 0.95)), 0.0, w);
            }
        }
        Kind::Font => {
            let size = g.height() * 1.05;
            p.text(g.center() + vec2(0.0, px * 0.3), egui::Align2::CENTER_CENTER, "A", crate::ui::bold(size), w);
        }
        Kind::Executable => {
            line(p, &[at(g, 0.06, 0.2), at(g, 0.38, 0.48), at(g, 0.06, 0.76)], w, px * 1.3);
            p.line_segment([at(g, 0.5, 0.8), at(g, 0.94, 0.8)], Stroke::new(px * 1.3, w));
        }
        Kind::Document | Kind::Pdf | Kind::Plain | Kind::Text => {
            for (y, x1) in [(0.14, 1.0), (0.42, 1.0), (0.7, 0.62)] {
                p.line_segment([at(g, 0.0, y), at(g, x1, y)], s);
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

pub fn eject(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    p.add(PathShape::convex_polygon(vec![at(r, 0.5, 0.14), at(r, 0.9, 0.6), at(r, 0.1, 0.6)], c, Stroke::NONE));
    p.rect_filled(Rect::from_min_max(at(r, 0.1, 0.72), at(r, 0.9, 0.86)), 1.0, c);
}

pub fn trash(p: &Painter, r: Rect, c: Color32) {
    let r = square(r);
    line(p, &[at(r, 0.18, 0.28), at(r, 0.82, 0.28)], c, 1.5);
    line(p, &[at(r, 0.4, 0.28), at(r, 0.42, 0.16), at(r, 0.58, 0.16), at(r, 0.6, 0.28)], c, 1.4);
    line(p, &[at(r, 0.26, 0.28), at(r, 0.32, 0.86), at(r, 0.68, 0.86), at(r, 0.74, 0.28)], c, 1.5);
    line(p, &[at(r, 0.44, 0.42), at(r, 0.45, 0.72)], c, 1.2);
    line(p, &[at(r, 0.56, 0.42), at(r, 0.55, 0.72)], c, 1.2);
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

#[cfg(test)]
mod tests {
    use super::FileKind;

    #[test]
    fn file_kinds() {
        assert!(FileKind::from_ext("pdf") == FileKind::Pdf);
        assert!(FileKind::from_ext("xlsx") == FileKind::Spreadsheet);
        assert!(FileKind::from_ext("csv") == FileKind::Spreadsheet);
        assert!(FileKind::from_ext("pptx") == FileKind::Presentation);
        assert!(FileKind::from_ext("docx") == FileKind::Document);
        assert!(FileKind::from_ext("md") == FileKind::Text);
        // Arbitrary binary data is not a program.
        assert!(FileKind::from_ext("bin") == FileKind::Plain);
        assert!(FileKind::from_ext("appimage") == FileKind::Executable);
    }
}
