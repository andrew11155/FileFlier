//! Custom-drawn UI. Everything is laid out in explicit rects so the look can
//! match File Pilot closely rather than following egui's default widget style.

mod chrome;
mod cloud;
pub mod glass;
mod inspector;
mod pane_view;
mod popups;
mod preview_view;
mod quicklook;
mod settings;
mod sidebar;
mod update;

use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align, Align2, Color32, FontId, Galley, Id, Painter, Rect, Response, RichText, Sense, Stroke, StrokeKind, TextEdit,
    Ui, pos2, vec2,
};

pub use cloud::CloudDialog;
pub use preview_view::ViewState;

use crate::icons::{self, FileKind};
use crate::theme::Palette;

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Shortcut hints as the platform writes them: "Ctrl+D" stays on Linux and
/// becomes "⌘D" on macOS (⌥ for Alt, ⇧ for Shift).
pub fn keys(s: &str) -> String {
    if cfg!(target_os = "macos") {
        s.replace("Ctrl+", "⌘").replace("Alt+", "⌥").replace("Shift+", "⇧")
    } else {
        s.to_string()
    }
}

/// The semibold UI font, for headings and emphasis.
pub fn bold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name(crate::theme::SEMIBOLD.into()))
}

/// Like [`text`], in semibold.
pub fn text_bold(p: &Painter, pos: egui::Pos2, align: Align2, s: impl ToString, size: f32, color: Color32) -> Rect {
    p.text(pos, align, s.to_string(), bold(size), color)
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
    wrapped_in(p, s, font(size), color, max_w, rows)
}

/// [`wrapped`] with an explicit font.
pub fn wrapped_in(p: &Painter, s: &str, font: FontId, color: Color32, max_w: f32, rows: usize) -> Arc<Galley> {
    let mut job = LayoutJob::simple(s.to_string(), font, color, max_w);
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
    if hover_text.is_empty() { resp } else { resp.on_hover_text(keys(hover_text)) }
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
    Trash,
}

/// Red used for destructive commands (Move to Trash, Delete).
pub fn danger_color(pal: &Palette) -> Color32 {
    if pal.is_dark { Color32::from_rgb(255, 107, 99) } else { Color32::from_rgb(200, 40, 40) }
}

pub fn paint_row_icon(p: &Painter, r: Rect, icon: RowIcon, pal: &Palette) {
    match icon {
        RowIcon::None => {}
        RowIcon::Folder => icons::folder(p, r, pal),
        RowIcon::File(k) => icons::file(p, r, k, pal),
        RowIcon::Trash => icons::trash(p, r, danger_color(pal)),
    }
}

/// A row in a popup list: icon, label, and right-aligned shortcut badges.
pub fn menu_row(ui: &mut Ui, pal: &Palette, selected: bool, icon: RowIcon, label: &str, badges: &[String]) -> Response {
    menu_row_ex(ui, pal, selected, icon, label, badges, false)
}

/// `danger` rows (Move to Trash, Delete) are drawn in red so they're easy to find.
pub fn menu_row_ex(
    ui: &mut Ui,
    pal: &Palette,
    selected: bool,
    icon: RowIcon,
    label: &str,
    badges: &[String],
    danger: bool,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let p = ui.painter();
    let red = danger_color(pal);
    if selected {
        // A deeper red than the text color, so white text stays readable.
        p.rect_filled(rect, 4.0, if danger { Color32::from_rgb(196, 48, 43) } else { pal.accent });
    } else if resp.hovered() {
        p.rect_filled(rect, 4.0, if danger { red.gamma_multiply(0.18) } else { pal.tab_hover });
    }
    let mut x = rect.left() + 10.0;
    if !matches!(icon, RowIcon::None) {
        let ir = Rect::from_center_size(pos2(x + 9.0, rect.center().y), vec2(18.0, 18.0));
        if danger && selected {
            icons::trash(p, ir, Color32::WHITE);
        } else {
            paint_row_icon(p, ir, icon, pal);
        }
        x += 26.0;
    }
    let mut right = rect.right() - 8.0;
    for b in badges.iter().rev() {
        right -= badge(p, right, rect.center().y, b, pal, selected) + 6.0;
    }
    let color = match (selected, danger) {
        (true, true) => Color32::WHITE,
        (true, false) => pal.on_accent,
        (false, true) => red,
        (false, false) => pal.text,
    };
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
/// Visual-effects settings for this frame.
#[derive(Clone, Copy)]
pub struct Fx {
    pub glass: bool,
    pub see_through: bool,
    pub opacity: f32,
    pub animations: bool,
}

impl Fx {
    /// Duration for a transition; zero when animations are off.
    pub fn dur(&self, secs: f32) -> f32 {
        if self.animations { secs } else { 0.0 }
    }
}

/// Ease-out cubic: fast start, gentle landing.
pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// 0→1 progress of an "appear" transition for something identified by `id`. It
/// restarts whenever the thing wasn't shown on the previous frame, so a popup
/// animates every time it opens.
pub fn appear(ctx: &egui::Context, id: egui::Id, dur: f32) -> f32 {
    let frame = ctx.cumulative_frame_nr();
    let now = ctx.input(|i| i.time);
    let (start, last) = ctx.data(|d| d.get_temp::<(f64, u64)>(id)).unwrap_or((now, 0));
    let start = if last + 1 < frame { now } else { start };
    ctx.data_mut(|d| d.insert_temp(id, (start, frame)));
    if dur <= 0.0 {
        return 1.0;
    }
    let t = ((now - start) as f32 / dur).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    ease_out(t)
}

/// Fades a popup in and slides it up by `rise` pixels while it appears.
pub fn animate_in(ui: &mut Ui, id: egui::Id, fx: &Fx, rise: f32) {
    let t = appear(ui.ctx(), id, fx.dur(0.16));
    ui.multiply_opacity(t);
    let offset = egui::emath::TSTransform::from_translation(vec2(0.0, (1.0 - t) * rise));
    ui.ctx().set_transform_layer(ui.layer_id(), offset);
}

/// Paints a pane/sidebar background: solid normally, a floating translucent
/// panel in glass mode.
pub fn surface(p: &Painter, rect: Rect, fill: Color32, fx: &Fx, pal: &Palette) {
    if fx.glass {
        glass::panel(p, rect, glass::with_alpha(fill, fx.opacity), 10, pal);
    } else {
        p.rect_filled(rect, 0.0, fill);
    }
}

pub fn popup_frame(pal: &Palette, fx: &Fx) -> egui::Frame {
    // Popups sit over the app's own content, which can't be blurred, so they stay
    // nearly opaque to keep text readable.
    let fill = if fx.glass { glass::with_alpha(pal.popup, 0.96) } else { pal.popup };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(8)
        .inner_margin(6)
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(120) })
}
