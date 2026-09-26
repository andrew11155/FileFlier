//! The Settings dialog: themes, accent color, size, density and behavior options.

use egui::{Align2, Color32, CornerRadius, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::{elided, text};
use crate::app::FileFlier;
use crate::config::{DateStyle, Density, Startup, ViewMode};
use crate::theme::{self, ACCENTS, Palette, ThemeId};

const SCALES: [f32; 6] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5];

fn section(ui: &mut Ui, pal: &Palette, title: &str) {
    ui.add_space(14.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    text(ui.painter(), r.left_center(), Align2::LEFT_CENTER, title.to_uppercase(), 11.5, pal.text_dim);
    ui.add_space(2.0);
}

/// A settings row: label (and optional hint) on the left, the control on the right.
fn row<R>(ui: &mut Ui, pal: &Palette, label: &str, hint: &str, control: impl FnOnce(&mut Ui) -> R) -> R {
    let h = if hint.is_empty() { 38.0 } else { 48.0 };
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let p = ui.painter();
    if hint.is_empty() {
        text(p, pos2(r.left(), r.center().y), Align2::LEFT_CENTER, label, 14.0, pal.text);
    } else {
        text(p, pos2(r.left(), r.center().y - 9.0), Align2::LEFT_CENTER, label, 14.0, pal.text);
        text(p, pos2(r.left(), r.center().y + 10.0), Align2::LEFT_CENTER, hint, 12.0, pal.text_dim);
    }
    p.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, pal.row_sep));
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::right_to_left(egui::Align::Center)));
    control(&mut child)
}

/// Segmented control; returns the newly picked index.
fn segmented(ui: &mut Ui, pal: &Palette, id: Id, options: &[&str], selected: usize) -> Option<usize> {
    let widths: Vec<f32> = options
        .iter()
        .map(|o| ui.painter().layout_no_wrap(o.to_string(), super::font(13.0), pal.text).size().x + 22.0)
        .collect();
    let (r, _) = ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 4.0, 28.0), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(r, 6.0, pal.input);
    p.rect_stroke(r, 6.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let mut x = r.left() + 2.0;
    let mut picked = None;
    for (i, (label, w)) in options.iter().zip(&widths).enumerate() {
        let seg = Rect::from_min_size(pos2(x, r.top() + 2.0), vec2(*w, r.height() - 4.0));
        let resp = ui.interact(seg, id.with(i), Sense::click());
        let color = if i == selected {
            p.rect_filled(seg, 5.0, pal.accent);
            pal.on_accent
        } else {
            if resp.hovered() {
                p.rect_filled(seg, 5.0, pal.tab_hover);
            }
            pal.text
        };
        text(&p, seg.center(), Align2::CENTER_CENTER, label, 13.0, color);
        if resp.clicked() && i != selected {
            picked = Some(i);
        }
        x += w;
    }
    picked
}

/// iOS-style switch; returns true when toggled.
fn toggle(ui: &mut Ui, pal: &Palette, id: Id, on: bool) -> bool {
    let (r, _) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::hover());
    let resp = ui.interact(r, id, Sense::click());
    let t = ui.ctx().animate_bool(id, on);
    let track = if on { pal.accent } else { pal.border };
    ui.painter().rect_filled(r, CornerRadius::same(11), track);
    let knob = if on { pal.on_accent } else { pal.text_strong };
    let cx = egui::lerp(r.left() + 11.0..=r.right() - 11.0, t);
    ui.painter().circle_filled(pos2(cx, r.center().y), 8.0, knob);
    resp.clicked()
}

/// Miniature preview of a theme: title bar, sidebar and a few rows with a selection.
fn theme_card(ui: &mut Ui, current: &Palette, id: ThemeId, accent: Option<Color32>, selected: bool) -> Response {
    let t = theme::palette(id, accent);
    let (r, resp) = ui.allocate_exact_size(vec2(138.0, 108.0), Sense::click());
    let p = ui.painter();
    let preview = Rect::from_min_size(r.min, vec2(r.width(), 80.0));
    p.rect_filled(preview, 6.0, t.bg);
    let title = Rect::from_min_size(preview.min, vec2(preview.width(), 14.0));
    p.rect_filled(title, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, t.titlebar);
    p.rect_filled(
        Rect::from_min_size(title.min + vec2(24.0, 4.0), vec2(34.0, 10.0)),
        CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 },
        t.bg,
    );
    let side = Rect::from_min_max(pos2(preview.left(), title.bottom()), pos2(preview.left() + 34.0, preview.bottom()));
    p.rect_filled(side, CornerRadius { nw: 0, ne: 0, sw: 6, se: 0 }, t.sidebar);
    for i in 0..3 {
        let y = side.top() + 8.0 + i as f32 * 12.0;
        p.rect_filled(Rect::from_min_size(pos2(side.left() + 6.0, y), vec2(20.0, 4.0)), 2.0, t.text_dim);
    }
    for i in 0..5 {
        let y = title.bottom() + 6.0 + i as f32 * 12.0;
        let row = Rect::from_min_size(pos2(side.right() + 5.0, y), vec2(preview.right() - side.right() - 10.0, 10.0));
        if i == 1 {
            p.rect_filled(row, 2.0, t.accent);
        }
        p.rect_filled(Rect::from_min_size(row.min + vec2(3.0, 2.0), vec2(8.0, 6.0)), 1.0, t.folder);
        let fg = if i == 1 { t.on_accent } else { t.text };
        p.rect_filled(Rect::from_min_size(row.min + vec2(15.0, 3.5), vec2(38.0 + (i * 7 % 20) as f32, 3.0)), 1.5, fg);
    }
    let stroke = if selected {
        Stroke::new(2.0, current.accent)
    } else if resp.hovered() {
        Stroke::new(1.0, current.text_dim)
    } else {
        Stroke::new(1.0, current.border)
    };
    p.rect_stroke(preview, 6.0, stroke, StrokeKind::Outside);
    let color = if selected { current.text_strong } else { current.text };
    let g = elided(p, id.name(), 13.0, color, r.width());
    p.galley(pos2(r.center().x - g.size().x / 2.0, preview.bottom() + 8.0), g, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn swatch(
    ui: &mut Ui,
    pal: &Palette,
    color: Option<Color32>,
    theme_accent: Color32,
    selected: bool,
    tip: &str,
) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    let p = ui.painter();
    let c = r.center();
    match color {
        Some(col) => {
            p.circle_filled(c, 11.0, col);
        }
        None => {
            // "Theme default": the theme's accent with a small 'auto' ring.
            p.circle_filled(c, 11.0, theme_accent);
            p.circle_stroke(c, 6.0, Stroke::new(1.5, Color32::from_white_alpha(200)));
        }
    }
    if selected {
        p.circle_stroke(c, 14.0, Stroke::new(2.0, pal.text_strong));
    } else if resp.hovered() {
        p.circle_stroke(c, 14.0, Stroke::new(1.0, pal.text_dim));
    }
    resp.on_hover_text(tip).clicked()
}

impl FileFlier {
    /// Draws the settings body. Returns true when the dialog should close.
    pub(crate) fn settings_ui(&mut self, ui: &mut Ui) -> bool {
        let pal = self.pal();
        let ctx = ui.ctx().clone();
        let mut close = false;
        let mut restyle = false;
        let mut resort = false;
        let mut refilter = false;

        // Use most of the window height; the list scrolls beyond that.
        let body_h = (ctx.content_rect().height() - 190.0).clamp(240.0, 640.0);

        // Header.
        let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
        text(ui.painter(), hr.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, "Settings", 18.0, pal.text_strong);
        let cr = Rect::from_center_size(pos2(hr.right() - 16.0, hr.center().y), vec2(28.0, 28.0));
        if super::icon_button(ui, cr, Id::new("settings_close"), true, pal, "Close (Esc)", crate::icons::close)
            .clicked()
        {
            close = true;
        }

        egui::ScrollArea::vertical().max_height(body_h).min_scrolled_height(body_h).auto_shrink([false, true]).show(
            ui,
            |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                let theme_now = self.cfg.theme();

                section(ui, pal, "Theme");
                let accent = self.cfg.accent_color();
                let per_row = ((ui.available_width() + 10.0) / 148.0).floor().max(1.0) as usize;
                for chunk in ThemeId::ALL.chunks(per_row) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        for &t in chunk {
                            if theme_card(ui, pal, t, accent, t == theme_now).clicked() && t != theme_now {
                                self.cfg.theme = Some(t);
                                restyle = true;
                            }
                        }
                    });
                    ui.add_space(8.0);
                }

                section(ui, pal, "Accent color");
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let theme_accent = theme::palette(theme_now, None).accent;
                    if swatch(ui, pal, None, theme_accent, self.cfg.accent.is_none(), "Theme default") {
                        self.cfg.accent = None;
                        restyle = true;
                    }
                    for (name, c) in ACCENTS {
                        let rgb = [c.r(), c.g(), c.b()];
                        if swatch(ui, pal, Some(c), theme_accent, self.cfg.accent == Some(rgb), name) {
                            self.cfg.accent = Some(rgb);
                            restyle = true;
                        }
                    }
                });

                section(ui, pal, "Layout");
                let scale_idx = SCALES.iter().position(|s| (s - self.cfg.ui_scale).abs() < 0.01).unwrap_or(2);
                let labels = ["80%", "90%", "100%", "110%", "125%", "150%"];
                if let Some(i) = row(ui, pal, "Interface size", "Also Ctrl + / Ctrl − / Ctrl 0", |ui| {
                    segmented(ui, pal, Id::new("scale"), &labels, scale_idx)
                }) {
                    self.cfg.ui_scale = SCALES[i];
                    restyle = true;
                }
                let dens = [Density::Compact, Density::Comfortable, Density::Spacious];
                let di = dens.iter().position(|d| *d == self.cfg.density).unwrap_or(1);
                if let Some(i) = row(ui, pal, "Row density", "", |ui| {
                    segmented(ui, pal, Id::new("density"), &["Compact", "Comfortable", "Spacious"], di)
                }) {
                    self.cfg.density = dens[i];
                    restyle = true;
                }
                let views = [ViewMode::Details, ViewMode::List, ViewMode::Grid];
                let vi = views.iter().position(|v| *v == self.cfg.view).unwrap_or(0);
                if let Some(i) = row(ui, pal, "Default view", "For new tabs", |ui| {
                    segmented(ui, pal, Id::new("view"), &["Details", "List", "Grid"], vi)
                }) {
                    self.cfg.view = views[i];
                    restyle = true;
                }

                section(ui, pal, "Browsing");
                if row(ui, pal, "Show hidden files", "Files starting with a dot (Ctrl+H)", |ui| {
                    toggle(ui, pal, Id::new("hidden"), self.cfg.show_hidden)
                }) {
                    self.cfg.show_hidden = !self.cfg.show_hidden;
                    refilter = true;
                }
                if row(ui, pal, "Folders first", "", |ui| toggle(ui, pal, Id::new("ff"), self.cfg.sort.folders_first)) {
                    self.cfg.sort.folders_first = !self.cfg.sort.folders_first;
                    resort = true;
                }
                if row(ui, pal, "Count items in folders", "Turn off if network drives feel slow", |ui| {
                    toggle(ui, pal, Id::new("counts"), self.cfg.show_item_counts)
                }) {
                    self.cfg.show_item_counts = !self.cfg.show_item_counts;
                    restyle = true;
                }
                let styles = [DateStyle::Iso, DateStyle::Relative, DateStyle::Friendly];
                let si = styles.iter().position(|d| *d == self.cfg.date_style).unwrap_or(0);
                if let Some(i) = row(ui, pal, "Dates", "", |ui| {
                    segmented(ui, pal, Id::new("dates"), &["2026-09-26 14:03", "5 min ago", "Sep 26, 2026"], si)
                }) {
                    self.cfg.date_style = styles[i];
                    restyle = true;
                }

                section(ui, pal, "Startup");
                let starts = [Startup::Home, Startup::RestoreSession];
                let st = starts.iter().position(|s| *s == self.cfg.startup).unwrap_or(0);
                if let Some(i) = row(ui, pal, "When File Flier starts", "", |ui| {
                    segmented(ui, pal, Id::new("startup"), &["Open home folder", "Restore last session"], st)
                }) {
                    self.cfg.startup = starts[i];
                    restyle = true;
                }

                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    let (r, resp) = ui.allocate_exact_size(vec2(150.0, 30.0), Sense::click());
                    ui.painter().rect_filled(r, 5.0, if resp.hovered() { pal.tab_hover } else { pal.input });
                    ui.painter().rect_stroke(r, 5.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
                    text(ui.painter(), r.center(), Align2::CENTER_CENTER, "Reset appearance", 13.5, pal.text);
                    if resp.clicked() {
                        self.cfg.theme = Some(ThemeId::Dark);
                        self.cfg.accent = None;
                        self.cfg.ui_scale = 1.0;
                        self.cfg.density = Density::Comfortable;
                        restyle = true;
                    }
                });
                ui.add_space(8.0);
            },
        );

        if restyle || resort || refilter {
            self.apply_settings(&ctx);
        }
        if resort {
            self.reload_all();
        } else if refilter {
            let h = self.cfg.show_hidden;
            for pane in &mut self.panes {
                for tab in &mut pane.tabs {
                    tab.refilter(h);
                }
            }
        }
        close
    }
}
