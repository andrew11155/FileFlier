//! Custom-drawn UI. Everything is laid out in explicit rects so the look can
//! match File Pilot closely rather than following egui's default widget style.

mod chrome;
mod inspector;
mod pane_view;
mod popups;
mod settings;
mod sidebar;

use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align, Align2, Color32, FontId, Galley, Id, Painter, Rect, Response, RichText, Sense, Stroke, StrokeKind, TextEdit,
    Ui, pos2, vec2,
};

use crate::icons::{self, FileKind};
use crate::theme::Palette;

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn text(p: &Painter, pos: egui::Pos2, align: Align2, s: impl ToString, size: f32, color: Color32) -> Rect {
    p.text(pos, align, s.to_string(), font(size), color)
}

/// Single-line text truncated with an ellipsis to `max_w`.
pub fn elided(p: &Painter, s: &str, size: f32, color: Color32, max_w: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(s.to_string(), font(size), color);
    job.wrap = TextWrapping::truncate_at_width(max_w.max(8.0));
    p.layout_job(job)
}

/// Text wrapped to at most `rows` lines, ellipsized.
pub fn wrapped(p: &Painter, s: &str, size: f32, color: Color32, max_w: f32, rows: usize) -> Arc<Galley> {
    let mut job = LayoutJob::simple(s.to_string(), font(size), color, max_w);
    job.wrap =
        TextWrapping { max_width: max_w, max_rows: rows, break_anywhere: false, overflow_character: Some('…') };
    job.halign = Align::Center;
    p.layout_job(job)
}

/// A flat icon button: no frame, subtle rounded hover.
pub fn icon_button(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    enabled: bool,
    pal: &Palette,
    hover_text: &str,
    draw: impl FnOnce(&Painter, Rect, Color32),
) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(rect, id, sense);
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, pal.tab_hover);
    }
    let color = if !enabled {
        pal.text_faint
    } else if resp.hovered() {
        pal.text_strong
    } else {
        pal.text
    };
    let icon = Rect::from_center_size(rect.center(), vec2(18.0, 18.0));
    draw(ui.painter(), icon, color);
    if hover_text.is_empty() { resp } else { resp.on_hover_text(hover_text) }
}

/// Rounded input box with a search glyph, like File Pilot's filter fields.
/// Returns the text edit response and, if requested, the trailing options button.
pub fn search_box(
    ui: &mut Ui,
    rect: Rect,
    pal: &Palette,
    value: &mut String,
    id: Id,
    hint: &str,
    with_options: bool,
) -> (Response, Option<Response>) {
    let focused = ui.memory(|m| m.has_focus(id));
    let p = ui.painter();
    p.rect_filled(rect, 5.0, pal.input);
    let border = if focused { pal.accent } else { pal.border };
    p.rect_stroke(rect, 5.0, Stroke::new(1.0, border), StrokeKind::Inside);
    let icon = Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(15.0, 15.0));
    icons::search(p, icon, pal.text_dim);
    let right_pad = if with_options { 34.0 } else { 8.0 };
    let edit_rect = Rect::from_min_max(
        pos2(rect.left() + 30.0, rect.top() + 1.0),
        pos2(rect.right() - right_pad, rect.bottom() - 1.0),
    );
    let resp = ui.put(
        edit_rect,
        TextEdit::singleline(value)
            .id(id)
            .frame(egui::Frame::NONE)
            .margin(vec2(0.0, 0.0))
            .vertical_align(Align::Center)
            .desired_width(edit_rect.width())
            .font(font(14.0))
            .text_color(pal.text)
            .hint_text(RichText::new(hint).color(pal.text_faint)),
    );
    let opts = with_options.then(|| {
        let r = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(26.0, rect.height() - 6.0));
        icon_button(ui, r, id.with("opts"), true, pal, "Filter options", icons::sliders)
    });
    (resp, opts)
}

/// Keyboard-shortcut badge, drawn right-aligned ending at `right`. Returns its width.
pub fn badge(p: &Painter, right: f32, cy: f32, s: &str, pal: &Palette, on_accent: bool) -> f32 {
    let color = if on_accent { pal.on_accent.gamma_multiply(0.9) } else { pal.text_dim };
    let g = p.layout_no_wrap(s.to_string(), font(11.5), color);
    let w = g.size().x + 14.0;
    let r = Rect::from_min_max(pos2(right - w, cy - 10.0), pos2(right, cy + 10.0));
    let stroke = if on_accent { pal.on_accent.gamma_multiply(0.5) } else { pal.border };
    p.rect_stroke(r, 4.0, Stroke::new(1.0, stroke), StrokeKind::Inside);
    p.galley(r.center() - g.size() / 2.0, g, color);
    w
}

#[derive(Clone, Copy)]
pub enum RowIcon {
    None,
    Folder,
    File(FileKind),
}

pub fn paint_row_icon(p: &Painter, r: Rect, icon: RowIcon, pal: &Palette) {
    match icon {
        RowIcon::None => {}
        RowIcon::Folder => icons::folder(p, r, pal),
        RowIcon::File(k) => icons::file(p, r, k, pal),
    }
}

/// A row in a popup list: icon, label, and right-aligned shortcut badges.
pub fn menu_row(ui: &mut Ui, pal: &Palette, selected: bool, icon: RowIcon, label: &str, badges: &[String]) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 4.0, pal.accent);
    } else if resp.hovered() {
        p.rect_filled(rect, 4.0, pal.tab_hover);
    }
    let mut x = rect.left() + 10.0;
    if !matches!(icon, RowIcon::None) {
        paint_row_icon(p, Rect::from_center_size(pos2(x + 9.0, rect.center().y), vec2(18.0, 18.0)), icon, pal);
        x += 26.0;
    }
    let mut right = rect.right() - 8.0;
    for b in badges.iter().rev() {
        right -= badge(p, right, rect.center().y, b, pal, selected) + 6.0;
    }
    let color = if selected { pal.on_accent } else { pal.text };
    let g = elided(p, label, 14.0, color, right - x - 8.0);
    p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, color);
    resp
}

pub fn checkbox(p: &Painter, r: Rect, checked: bool, pal: &Palette, on_accent: bool) {
    let r = Rect::from_center_size(r.center(), vec2(16.0, 16.0));
    let stroke = if on_accent { pal.on_accent } else { pal.text_dim };
    if checked {
        p.rect_filled(r, 3.0, if on_accent { pal.on_accent.gamma_multiply(0.15) } else { pal.accent });
    }
    p.rect_stroke(r, 3.0, Stroke::new(1.3, stroke), StrokeKind::Inside);
    if checked {
        let pts = vec![
            pos2(r.left() + 3.5, r.center().y),
            pos2(r.left() + 6.5, r.bottom() - 4.0),
            pos2(r.right() - 3.5, r.top() + 4.0),
        ];
        p.add(egui::Shape::line(pts, Stroke::new(1.6, pal.on_accent)));
    }
}

/// Frame used by all popups and dialogs.
pub fn popup_frame(pal: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(pal.popup)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(8)
        .inner_margin(6)
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(120) })
}
