//! Draws a preview (shared by the inspector and Quick Look): zoomable images and
//! pages, highlighted code, documents on "paper", tables, archive listings, folders.

use egui::{
    Align2, Color32, CursorIcon, Id, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};

use super::{elided, text, wrapped};
use crate::fs_model::Entry;
use crate::icons::{self, FileKind};
use crate::preview::{Block, Content, Shown};
use crate::theme::Palette;
use crate::ui::pane_view::file_kind;

/// Zoom/pan state of one preview surface; reset when the item changes.
#[derive(Default)]
pub struct ViewState {
    key: Option<(std::path::PathBuf, usize)>,
    /// 1.0 = fit to the area.
    pub zoom: f32,
    /// Point of the image (0..1) shown at the center of the area.
    center: Vec2,
}

impl ViewState {
    fn sync(&mut self, path: &std::path::Path, page: usize) {
        if self.key.as_ref().is_none_or(|(p, pg)| p != path || *pg != page) {
            self.key = Some((path.to_path_buf(), page));
            self.reset();
        }
    }

    pub fn reset(&mut self) {
        self.zoom = 1.0;
        self.center = vec2(0.5, 0.5);
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(1.0, 40.0);
        if self.zoom <= 1.0 {
            self.center = vec2(0.5, 0.5);
        }
    }
}

pub enum Nav {
    Page(usize),
}

pub struct Style {
    /// Quick Look: larger text, folder tiles instead of a list.
    pub big: bool,
}

/// Draws `shown` (or a spinner / icon) for `e` inside `rect`.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    e: &Entry,
    shown: Option<&Shown>,
    loading: bool,
    view: &mut ViewState,
    pal: &Palette,
    style: Style,
) -> Option<Nav> {
    let Some(shown) = shown else {
        ui.put(Rect::from_center_size(rect.center(), vec2(28.0, 28.0)), egui::Spinner::new().color(pal.text_dim));
        return None;
    };
    let mut nav = None;
    let page = match &shown.loaded.content {
        Content::Page { index, .. } => *index,
        _ => 0,
    };
    view.sync(&e.path, page);
    match &shown.loaded.content {
        Content::Image(_) => {
            if let Some(tex) = &shown.tex {
                zoom_image(ui, rect, tex, view, pal, if style.big { 2.0 } else { 1.0 }, false);
            }
        }
        Content::Page { index, count, .. } => {
            if let Some(tex) = &shown.tex {
                zoom_image(ui, rect, tex, view, pal, 8.0, true);
            }
            if *count > 1 {
                nav = page_nav(ui, rect, *index, *count, pal);
            }
        }
        Content::Text { text: t, syntax, truncated } => text_view(ui, rect, e, t, syntax, *truncated, pal, &style),
        Content::Table { rows, truncated } => table_view(ui, rect, e, rows, *truncated, pal),
        Content::Document { thumb, blocks } => {
            document_view(ui, rect, e, shown.tex.as_ref().filter(|_| thumb.is_some()), blocks, pal, &style)
        }
        Content::Dir(entries) => {
            if style.big {
                dir_tiles(ui, rect, e, entries, pal)
            } else {
                dir_list(ui, rect, e, entries, pal)
            }
        }
        Content::Listing(items) => listing_view(ui, rect, e, items, pal),
        Content::Icon(note) => big_icon(ui, rect, e, note.as_deref(), pal.text_dim, pal),
        Content::Error(err) => big_icon(ui, rect, e, Some(err), Color32::from_rgb(230, 120, 110), pal),
    }
    if loading || shown.refining {
        let r = Rect::from_center_size(rect.right_top() + vec2(-16.0, 16.0), vec2(14.0, 14.0));
        ui.put(r, egui::Spinner::new().size(14.0).color(pal.text_dim));
    }
    nav
}

fn big_icon(ui: &Ui, rect: Rect, e: &Entry, note: Option<&str>, note_color: Color32, pal: &Palette) {
    let p = ui.painter();
    let size = (rect.width().min(rect.height()) * 0.45).clamp(40.0, 112.0);
    let ir =
        Rect::from_center_size(rect.center() - vec2(0.0, if note.is_some() { 16.0 } else { 0.0 }), vec2(size, size));
    if e.is_dir {
        icons::folder(p, ir, pal);
    } else {
        icons::file(p, ir, file_kind(e), pal);
    }
    if let Some(n) = note {
        let g = wrapped(p, n, 13.0, note_color, rect.width() - 24.0, 3);
        p.galley(pos2(rect.center().x, ir.bottom() + 12.0), g, note_color);
    }
}

// ------------------------------------------------------------------ zoomable image

/// Image fitted into `rect`; scroll or pinch to zoom, drag to pan, double-click to toggle.
fn zoom_image(
    ui: &mut Ui,
    rect: Rect,
    tex: &egui::TextureHandle,
    view: &mut ViewState,
    pal: &Palette,
    max_upscale: f32,
    paper: bool,
) {
    let ppp = ui.ctx().pixels_per_point();
    let size = tex.size_vec2() / ppp; // 1 texture pixel = 1 screen pixel at scale 1
    let area = rect.shrink(if paper { 10.0 } else { 4.0 });
    let fit = (area.width() / size.x).min(area.height() / size.y).min(max_upscale).max(0.001);
    let id = Id::new(("zoom_image", rect.min.x as i32, rect.min.y as i32));
    let resp = ui.interact(rect, id, Sense::click_and_drag());

    let disp = |zoom: f32| size * fit * zoom;
    if resp.hovered() {
        let (scroll, pinch, ptr) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta(), i.pointer.hover_pos()));
        let factor = pinch * (scroll * 0.0025).exp();
        if (factor - 1.0).abs() > 1e-4 {
            let ptr = ptr.unwrap_or(rect.center());
            let before = view.center + (ptr - rect.center()) / disp(view.zoom);
            view.zoom_by(factor);
            view.center = before - (ptr - rect.center()) / disp(view.zoom);
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }
    if resp.double_clicked() {
        if view.zoom > 1.01 {
            view.reset();
        } else {
            let ptr = resp.interact_pointer_pos().unwrap_or(rect.center());
            let before = view.center + (ptr - rect.center()) / disp(view.zoom);
            // Zoom to actual pixels, or at least 2.5x.
            view.zoom_by((1.0 / fit).max(2.5));
            view.center = before - (ptr - rect.center()) / disp(view.zoom);
        }
    }
    if resp.dragged() && view.zoom > 1.0 {
        view.center -= resp.drag_delta() / disp(view.zoom);
    }
    // Keep the image covering the area when zoomed in.
    let d = disp(view.zoom);
    for (c, dim, avail) in [(&mut view.center.x, d.x, area.width()), (&mut view.center.y, d.y, area.height())] {
        if dim <= avail {
            *c = 0.5;
        } else {
            let half = avail / 2.0 / dim;
            *c = c.clamp(half, 1.0 - half);
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(if view.zoom > 1.0 {
            if resp.dragged() { CursorIcon::Grabbing } else { CursorIcon::Grab }
        } else {
            CursorIcon::ZoomIn
        });
    }

    let img_rect = Rect::from_center_size(area.center() - (view.center - vec2(0.5, 0.5)) * d, d);
    let p = ui.painter().with_clip_rect(area.intersect(ui.clip_rect()));
    if paper {
        p.add(
            egui::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(60) }
                .as_shape(img_rect, 2),
        );
    } else {
        checkerboard(&p, img_rect.intersect(area), pal);
    }
    p.image(tex.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);

    // Zoom controls while hovering (or zoomed).
    if resp.hovered() || view.zoom > 1.0 {
        zoom_controls(ui, rect, view, pal, fit);
    }
}

/// Subtle checkerboard behind images so transparency is visible.
fn checkerboard(p: &egui::Painter, r: Rect, pal: &Palette) {
    if r.width() <= 0.0 || r.height() <= 0.0 {
        return;
    }
    let c = if pal.is_dark { Color32::from_white_alpha(6) } else { Color32::from_black_alpha(8) };
    let s = 8.0;
    let mut y = r.top();
    let mut row = 0;
    while y < r.bottom() {
        let mut x = r.left() + if row % 2 == 0 { 0.0 } else { s };
        while x < r.right() {
            p.rect_filled(
                Rect::from_min_max(pos2(x, y), pos2((x + s).min(r.right()), (y + s).min(r.bottom()))),
                0.0,
                c,
            );
            x += 2.0 * s;
        }
        y += s;
        row += 1;
    }
}

fn pill(ui: &Ui, r: Rect, pal: &Palette) {
    let p = ui.painter();
    p.rect_filled(r, r.height() / 2.0, pal.popup.gamma_multiply(0.94));
    p.rect_stroke(r, r.height() / 2.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
}

fn pill_button(ui: &mut Ui, r: Rect, id: Id, label: &str, pal: &Palette, tip: &str) -> bool {
    let resp = ui.interact(r, id, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r.shrink(2.0), r.height() / 2.0, pal.hover);
    }
    text(ui.painter(), r.center(), Align2::CENTER_CENTER, label, 14.0, pal.text);
    resp.on_hover_text(tip).clicked()
}

fn zoom_controls(ui: &mut Ui, rect: Rect, view: &mut ViewState, pal: &Palette, fit: f32) {
    if rect.width() < 150.0 {
        return;
    }
    let bar = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 22.0), vec2(142.0, 28.0));
    pill(ui, bar, pal);
    let b = |i: f32, w: f32| Rect::from_min_size(pos2(bar.left() + i, bar.top()), vec2(w, bar.height()));
    let base = Id::new(("zoomctl", rect.min.x as i32));
    if pill_button(ui, b(4.0, 30.0), base.with(0), "−", pal, "Zoom out") {
        view.zoom_by(1.0 / 1.5);
    }
    let pct = format!("{:.0}%", fit * view.zoom * 100.0);
    if pill_button(ui, b(34.0, 56.0), base.with(1), &pct, pal, "Actual size / fit (double-click the image)") {
        if view.zoom > 1.01 { view.reset() } else { view.zoom_by((1.0 / fit).max(1.0)) }
    }
    if pill_button(ui, b(90.0, 30.0), base.with(2), "+", pal, "Zoom in (scroll or pinch)") {
        view.zoom_by(1.5);
    }
    let fit_r = b(118.0, 22.0);
    if view.zoom > 1.01 && pill_button(ui, fit_r, base.with(3), "×", pal, "Back to fit") {
        view.reset();
    }
}

fn page_nav(ui: &mut Ui, rect: Rect, index: usize, count: usize, pal: &Palette) -> Option<Nav> {
    let bar = Rect::from_center_size(pos2(rect.center().x, rect.top() + 20.0), vec2(128.0, 26.0));
    pill(ui, bar, pal);
    let base = Id::new(("pagenav", rect.min.x as i32));
    let mut nav = None;
    let prev = Rect::from_min_size(bar.min, vec2(30.0, bar.height()));
    let next = Rect::from_min_size(pos2(bar.right() - 30.0, bar.top()), vec2(30.0, bar.height()));
    if index > 0 && pill_button(ui, prev, base.with(0), "‹", pal, "Previous page (Page Up)") {
        nav = Some(Nav::Page(index - 1));
    }
    if index + 1 < count && pill_button(ui, next, base.with(1), "›", pal, "Next page (Page Down)") {
        nav = Some(Nav::Page(index + 1));
    }
    text(ui.painter(), bar.center(), Align2::CENTER_CENTER, format!("{} / {}", index + 1, count), 12.5, pal.text);
    nav
}

// ------------------------------------------------------------------ text & documents

fn scroll_area(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        egui::ScrollArea::both().id_salt(id).auto_shrink(false).show(ui, add);
    });
}

#[allow(clippy::too_many_arguments)]
fn text_view(ui: &mut Ui, rect: Rect, e: &Entry, t: &str, syntax: &str, truncated: bool, pal: &Palette, style: &Style) {
    let p = ui.painter();
    p.rect_filled(rect, 6.0, pal.bg);
    let size = if style.big { 13.0 } else { 12.0 };
    scroll_area(ui, rect.shrink(8.0), ("pv_text", &e.path), |ui| {
        if syntax.is_empty() || t.len() > 300_000 {
            ui.add(egui::Label::new(RichText::new(t).monospace().size(size).color(pal.text)).extend());
        } else {
            let mut theme = egui_extras::syntax_highlighting::CodeTheme::from_style(ui.style());
            theme = theme.with_font_size(size);
            let job = egui_extras::syntax_highlighting::highlight(ui.ctx(), ui.style(), &theme, t, syntax);
            ui.add(egui::Label::new(job).extend());
        }
        if truncated {
            ui.add_space(6.0);
            ui.label(RichText::new("… preview shortened").size(12.0).italics().color(pal.text_dim));
        }
    });
}

const PAPER: Color32 = Color32::from_rgb(252, 252, 250);
const INK: Color32 = Color32::from_rgb(32, 33, 36);
const INK_DIM: Color32 = Color32::from_rgb(110, 112, 118);

fn document_view(
    ui: &mut Ui,
    rect: Rect,
    e: &Entry,
    thumb: Option<&egui::TextureHandle>,
    blocks: &[Block],
    pal: &Palette,
    style: &Style,
) {
    let _ = pal;
    let paper = rect.shrink(if style.big { 6.0 } else { 2.0 });
    ui.painter().add(
        egui::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(50) }.as_shape(paper, 4),
    );
    ui.painter().rect_filled(paper, 4.0, PAPER);
    let scale = if style.big { 1.0 } else { 0.86 };
    scroll_area(ui, paper.shrink2(vec2(if style.big { 36.0 } else { 16.0 }, 14.0)), ("pv_doc", &e.path), |ui| {
        ui.spacing_mut().item_spacing.y = 6.0 * scale;
        let w = ui.available_width();
        ui.set_width(w);
        if let Some(t) = thumb {
            let s = t.size_vec2();
            let k = (w / s.x).min(1.5);
            ui.add(egui::Image::new(t).fit_to_exact_size(s * k).corner_radius(3));
            ui.add_space(6.0);
        }
        if blocks.is_empty() {
            ui.label(RichText::new("This document is empty").size(14.0 * scale).color(INK_DIM));
        }
        for b in blocks {
            match b {
                Block::Heading(level, t) => {
                    let size = match level {
                        1 => 22.0,
                        2 => 18.0,
                        _ => 15.5,
                    } * scale;
                    ui.add_space(4.0 * scale);
                    ui.add(egui::Label::new(RichText::new(t).size(size).strong().color(INK)).wrap());
                }
                Block::Para(t) => {
                    ui.add(egui::Label::new(RichText::new(t).size(14.0 * scale).color(INK)).wrap());
                }
                Block::Bullet(t) => {
                    ui.add(egui::Label::new(RichText::new(format!("•  {t}")).size(14.0 * scale).color(INK)).wrap());
                }
                Block::Row(cells) => {
                    let cols = cells.len().max(1);
                    ui.horizontal_top(|ui| {
                        let cw = (w / cols as f32 - 8.0).max(30.0);
                        for c in cells {
                            ui.add_sized(
                                [cw, 0.0],
                                egui::Label::new(RichText::new(c).size(12.5 * scale).color(INK)).wrap(),
                            );
                        }
                    });
                    let y = ui.cursor().top() - 2.0;
                    ui.painter().hline(ui.min_rect().x_range(), y, Stroke::new(0.5, Color32::from_black_alpha(30)));
                }
                Block::Slide(n) => {
                    ui.add_space(10.0 * scale);
                    let (r, _) = ui.allocate_exact_size(vec2(w, 18.0 * scale), Sense::hover());
                    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, Color32::from_black_alpha(35)));
                    let label = format!("  Slide {n}  ");
                    let g = ui.painter().layout_no_wrap(label, egui::FontId::proportional(11.5 * scale), INK_DIM);
                    let lr = Rect::from_center_size(r.center(), g.size());
                    ui.painter().rect_filled(lr, 0.0, PAPER);
                    ui.painter().galley(lr.min, g, INK_DIM);
                }
            }
        }
    });
}

fn table_view(ui: &mut Ui, rect: Rect, e: &Entry, rows: &[Vec<String>], truncated: bool, pal: &Palette) {
    let p = ui.painter();
    p.rect_filled(rect, 6.0, pal.bg);
    if rows.is_empty() {
        text(p, rect.center(), Align2::CENTER_CENTER, "Empty sheet", 13.5, pal.text_dim);
        return;
    }
    let ncols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    // Column widths from content, within limits.
    let mut widths = vec![44.0f32; ncols];
    for r in rows.iter().take(100) {
        for (i, c) in r.iter().enumerate() {
            let chars = c.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f32;
            widths[i] = widths[i].max((chars * 7.2 + 16.0).min(240.0));
        }
    }
    let gutter = 36.0;
    let row_h = 22.0;
    let total_w = gutter + widths.iter().sum::<f32>();
    scroll_area(ui, rect.shrink(2.0), ("pv_table", &e.path), |ui| {
        let (area, _) = ui.allocate_exact_size(
            vec2(total_w, row_h * (rows.len() as f32 + if truncated { 1.0 } else { 0.0 })),
            Sense::hover(),
        );
        let p = ui.painter();
        let visible = ui.clip_rect();
        for (ri, r) in rows.iter().enumerate() {
            let y = area.top() + ri as f32 * row_h;
            if y + row_h < visible.top() || y > visible.bottom() {
                continue;
            }
            let row_rect = Rect::from_min_size(pos2(area.left(), y), vec2(total_w, row_h));
            if ri % 2 == 1 {
                p.rect_filled(row_rect, 0.0, pal.hover.gamma_multiply(0.5));
            }
            text(
                p,
                pos2(area.left() + gutter - 8.0, y + row_h / 2.0),
                Align2::RIGHT_CENTER,
                ri + 1,
                11.0,
                pal.text_faint,
            );
            let mut x = area.left() + gutter;
            for (ci, w) in widths.iter().enumerate() {
                let cell = r.get(ci).map(String::as_str).unwrap_or("");
                let first_line = cell.lines().next().unwrap_or("");
                let color = if ri == 0 { pal.text_strong } else { pal.text };
                let g = elided(p, first_line, 12.5, color, w - 10.0);
                // Right-align numbers, like a spreadsheet.
                let numeric = first_line.parse::<f64>().is_ok();
                let gx = if numeric { x + w - 5.0 - g.size().x } else { x + 5.0 };
                p.galley(pos2(gx, y + (row_h - g.size().y) / 2.0), g, color);
                x += w;
                p.vline(x, y..=y + row_h, Stroke::new(1.0, pal.row_sep));
            }
            p.hline(row_rect.x_range(), row_rect.bottom(), Stroke::new(1.0, pal.row_sep));
        }
        if truncated {
            let y = area.top() + rows.len() as f32 * row_h + row_h / 2.0;
            text(p, pos2(area.left() + gutter, y), Align2::LEFT_CENTER, "… more rows not shown", 12.0, pal.text_dim);
        }
    });
}

// ------------------------------------------------------------------ folders & archives

fn dir_list(ui: &mut Ui, rect: Rect, e: &Entry, entries: &[Entry], pal: &Palette) {
    if entries.is_empty() {
        text(ui.painter(), rect.center(), Align2::CENTER_CENTER, "Empty folder", 13.5, pal.text_dim);
        return;
    }
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().id_salt(("pv_dir", &e.path)).auto_shrink(false).show_rows(
            ui,
            26.0,
            entries.len(),
            |ui, range| {
                for c in &entries[range] {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                    let ir = Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), vec2(17.0, 17.0));
                    if c.is_dir {
                        icons::folder(ui.painter(), ir, pal);
                    } else {
                        icons::file(ui.painter(), ir, file_kind(c), pal);
                    }
                    let g = elided(ui.painter(), &c.name, 13.5, pal.text, r.width() - 36.0);
                    ui.painter().galley(pos2(r.left() + 30.0, r.center().y - g.size().y / 2.0), g, pal.text);
                }
            },
        );
    });
}

fn dir_tiles(ui: &mut Ui, rect: Rect, e: &Entry, entries: &[Entry], pal: &Palette) {
    if entries.is_empty() {
        text(ui.painter(), rect.center(), Align2::CENTER_CENTER, "Empty folder", 15.0, pal.text_dim);
        return;
    }
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        egui::ScrollArea::vertical().id_salt(("pv_tiles", &e.path)).auto_shrink(false).show(ui, |ui| {
            let tile = vec2(112.0, 104.0);
            let cols = ((rect.width() / tile.x).floor() as usize).max(1);
            for chunk in entries.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for c in chunk {
                        let (tr, _) = ui.allocate_exact_size(tile, Sense::hover());
                        let p = ui.painter();
                        let ir = Rect::from_center_size(pos2(tr.center().x, tr.top() + 32.0), vec2(48.0, 48.0));
                        if c.is_dir {
                            icons::folder(p, ir, pal);
                        } else {
                            icons::file(p, ir, file_kind(c), pal);
                        }
                        let g = wrapped(p, &c.name, 12.5, pal.text, tile.x - 10.0, 2);
                        p.galley(pos2(tr.center().x, tr.top() + 62.0), g, pal.text);
                    }
                });
            }
        });
    });
}

fn listing_view(ui: &mut Ui, rect: Rect, e: &Entry, items: &[crate::preview::ListItem], pal: &Palette) {
    ui.painter().rect_filled(rect, 6.0, pal.bg);
    if items.is_empty() {
        text(ui.painter(), rect.center(), Align2::CENTER_CENTER, "Empty archive", 13.5, pal.text_dim);
        return;
    }
    let inner = rect.shrink2(vec2(6.0, 4.0));
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.set_clip_rect(inner.intersect(ui.clip_rect()));
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().id_salt(("pv_list", &e.path)).auto_shrink(false).show_rows(
            ui,
            24.0,
            items.len(),
            |ui, range| {
                for it in &items[range] {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
                    let p = ui.painter();
                    let depth = it.name.trim_end_matches('/').matches('/').count() as f32;
                    let x0 = r.left() + 6.0 + depth.min(6.0) * 12.0;
                    let ir = Rect::from_center_size(pos2(x0 + 8.0, r.center().y), vec2(15.0, 15.0));
                    let name = it.name.trim_end_matches('/').rsplit('/').next().unwrap_or(&it.name);
                    if it.is_dir {
                        icons::folder(p, ir, pal);
                    } else {
                        let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
                        icons::file(p, ir, FileKind::from_ext(&ext), pal);
                    }
                    let size_w = if it.size.is_some() { 70.0 } else { 0.0 };
                    let g = elided(p, name, 13.0, pal.text, r.right() - x0 - 24.0 - size_w);
                    p.galley(pos2(x0 + 22.0, r.center().y - g.size().y / 2.0), g, pal.text);
                    if let Some(s) = it.size {
                        let s = crate::app::human_size(s);
                        text(p, pos2(r.right() - 6.0, r.center().y), Align2::RIGHT_CENTER, s, 12.0, pal.text_dim);
                    }
                }
            },
        );
    });
}
