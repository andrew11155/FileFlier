//! The Settings dialog: themes, accent color, size, density and behavior options.

use egui::{Align2, Color32, CornerRadius, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::{elided, text};
use crate::app::FileFlier;
use crate::config::{DateStyle, Density, Startup, TitleIcon, ViewMode};
use crate::theme::{self, ACCENTS, Palette, ThemeId};

const SCALES: [f32; 6] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5];

fn section(ui: &mut Ui, pal: &Palette, title: &str) {
    ui.add_space(14.0);
    if title != "Theme" {
        // A line between sections.
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, pal.border));
        ui.add_space(10.0);
    }
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    super::text_bold(ui.painter(), r.left_center(), Align2::LEFT_CENTER, title.to_uppercase(), 11.5, pal.text_dim);
    ui.add_space(2.0);
}

/// A bottom bar setting: label, hint, and the config flag it flips.
type BarToggle = (&'static str, &'static str, fn(&mut crate::config::Config) -> &mut bool);

/// A Theme / On / Off setting row: id, label, hint, option names and current value.
type Tweak<'a> = (&'a str, &'a str, &'a str, [&'a str; 3], Option<bool>);

/// Whether GNOME is set to dark style (`color-scheme` is "prefer-dark").
fn desktop_prefers_dark() -> bool {
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| s.contains("dark"))
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
    p.rect_filled(r, pal.round(6.0), pal.input);
    p.rect_stroke(r, pal.round(6.0), Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let mut x = r.left() + 2.0;
    let mut picked = None;
    for (i, (label, w)) in options.iter().zip(&widths).enumerate() {
        let seg = Rect::from_min_size(pos2(x, r.top() + 2.0), vec2(*w, r.height() - 4.0));
        let resp = ui.interact(seg, id.with(i), Sense::click());
        let color = if i == selected {
            p.rect_filled(seg, pal.round(5.0), pal.accent);
            pal.on_accent
        } else {
            if resp.hovered() {
                p.rect_filled(seg, pal.round(5.0), pal.tab_hover);
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
    ui.painter().rect_filled(r, CornerRadius::same(pal.round(11.0) as u8), track);
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
    p.rect_filled(preview, t.round(6.0), t.bg);
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
            p.rect_filled(row, t.round(2.0), t.accent);
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
    p.rect_stroke(preview, t.round(6.0), stroke, StrokeKind::Outside);
    let color = if selected { current.text_strong } else { current.text };
    let g = elided(p, id.name(), 13.0, color, r.width());
    p.galley(pos2(r.center().x - g.size().x / 2.0, preview.bottom() + 8.0), g, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A folder color scheme: its eight colors on folder icons, on the current theme.
/// One folder color scheme as a row: its name, then its folders in a line.
fn scheme_row(ui: &mut Ui, pal: &Palette, name: &str, colors: &[Color32; 8], selected: bool) -> Response {
    let (label_w, step) = (84.0, 28.0);
    let w = (24.0 + label_w + step * colors.len() as f32).min(ui.available_width());
    let (r, resp) = ui.allocate_exact_size(vec2(w, 36.0), Sense::click());
    let p = ui.painter();
    let fill = if selected { pal.accent.gamma_multiply(0.14) } else { pal.bg };
    p.rect_filled(r, pal.round(6.0), fill);
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let color = if selected { pal.text_strong } else { pal.text };
    let g = elided(p, name, 13.0, color, label_w - 8.0);
    p.galley(pos2(r.left() + 12.0, r.center().y - g.size().y / 2.0), g, color);
    for (i, &c) in colors.iter().enumerate() {
        let center = pos2(r.left() + 12.0 + label_w + step * (i as f32 + 0.5), r.center().y);
        // On hover, the folders lift a little in turn.
        let lift = hover * 2.0 * ((i as f32 * 0.8 + ui.input(|i| i.time) as f32 * 6.0).sin() * 0.5 + 0.5);
        let icon = Rect::from_center_size(center - vec2(0.0, lift), vec2(22.0, 22.0));
        crate::icons::folder_tinted(p, icon, pal, Some(c));
    }
    if hover > 0.0 {
        ui.ctx().request_repaint();
    }
    let stroke = if selected {
        Stroke::new(2.0, pal.accent)
    } else if resp.hovered() {
        Stroke::new(1.0, pal.text_dim)
    } else {
        Stroke::new(1.0, pal.border)
    };
    p.rect_stroke(r, pal.round(6.0), stroke, StrokeKind::Inside);
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
    /// The Updates rows: current version with a check button, and the auto-check toggle.
    fn updates_section(&mut self, ui: &mut Ui) {
        use crate::updater::{Status, VERSION};
        let pal = self.pal();
        let ctx = ui.ctx().clone();
        let (hint, err) = match &self.updater.status {
            Status::Idle => ("Checks GitHub for a newer release".to_string(), false),
            Status::Checking => ("Checking…".to_string(), false),
            Status::UpToDate => ("You're up to date".to_string(), false),
            Status::Available(r) => (format!("Version {} available", r.version), false),
            Status::Failed(e) => (e.clone(), true),
        };
        let available = matches!(self.updater.status, Status::Available(_));
        let busy = self.updater.checking();
        let clicked = row(ui, pal, &format!("Version {VERSION}"), "", |ui| {
            let label = if available { "View update" } else { "Check for updates" };
            let g = ui.painter().layout_no_wrap(label.to_string(), super::font(13.5), pal.text);
            let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 28.0, 28.0), Sense::click());
            let fill = if busy {
                pal.input
            } else if available {
                pal.accent
            } else if resp.hovered() {
                pal.tab_hover
            } else {
                pal.input
            };
            let color = if available && !busy { pal.on_accent } else { pal.text };
            ui.painter().rect_filled(r, pal.round(5.0), fill);
            ui.painter().rect_stroke(r, pal.round(5.0), Stroke::new(1.0, pal.border), StrokeKind::Inside);
            text(ui.painter(), r.center(), Align2::CENTER_CENTER, label, 13.5, color);
            // The status sits to the left of the button.
            let color = if err { Color32::from_rgb(255, 140, 130) } else { pal.text_dim };
            // Leave room for the "Version X.Y.Z" label on the left; full text on hover.
            let g = elided(ui.painter(), &hint, 12.5, color, (ui.available_width() - 150.0).max(40.0));
            let elided_hint = g.elided;
            let (sr, sresp) = ui.allocate_exact_size(g.size(), Sense::hover());
            ui.painter().galley(sr.min, g, color);
            if elided_hint {
                sresp.on_hover_text(&hint);
            }
            resp.clicked() && !busy
        });
        if clicked {
            if available {
                self.cfg.skipped_version = None; // asking for it means it's wanted
                self.cfg.save();
                self.dialog = Some(crate::app::Dialog::Update);
            } else {
                self.updater.check(&ctx);
            }
        }
        if row(
            ui,
            pal,
            "Check for updates automatically",
            "At most once a day. Nothing about you or your files is sent",
            |ui| toggle(ui, pal, Id::new("autoupdate"), self.cfg.check_updates),
        ) {
            self.cfg.check_updates = !self.cfg.check_updates;
            self.cfg.save();
        }
    }

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
        super::text_bold(
            ui.painter(),
            hr.left_center() + vec2(20.0, 0.0),
            Align2::LEFT_CENTER,
            "Settings",
            18.0,
            pal.text_strong,
        );
        let cr = Rect::from_center_size(pos2(hr.right() - 28.0, hr.center().y), vec2(28.0, 28.0));
        if super::icon_button(ui, cr, Id::new("settings_close"), true, pal, "Close (Esc)", crate::icons::close)
            .clicked()
        {
            close = true;
        }

        egui::ScrollArea::vertical().max_height(body_h).min_scrolled_height(body_h).auto_shrink([false, true]).show(
            ui,
            |ui| {
                egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 24, top: 0, bottom: 0 }).show(
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

                        // Everything that makes File Flier look like the desktop (GNOME / Ubuntu).
                        section(ui, pal, "Desktop look");
                        let hint = "The GNOME theme with all of the below, and the Files icon";
                        if row(ui, pal, "Native Ubuntu look", hint, |ui| {
                            super::popups::button(ui, pal, "Apply", true, false)
                        })
                        .clicked()
                        {
                            let dark = desktop_prefers_dark();
                            self.cfg.theme = Some(if dark { ThemeId::AdwaitaDark } else { ThemeId::AdwaitaLight });
                            self.cfg.gnome_icons = true;
                            self.cfg.native_buttons = !cfg!(target_os = "macos");
                            self.cfg.title_icon = TitleIcon::Files;
                            (self.cfg.rounded, self.cfg.gnome_sidebar, self.cfg.gtk_text) = (None, None, None);
                            crate::system_icons::set_enabled(true, &ctx);
                            restyle = true;
                        }
                        let hint = if crate::system_icons::available() {
                            format!(
                                "GNOME uses your desktop's icon theme ({}), as GNOME Files does",
                                crate::system_icons::theme_name()
                            )
                        } else {
                            "GNOME needs an icon theme, and none is installed".to_string()
                        };
                        let sel = usize::from(self.cfg.gnome_icons);
                        if let Some(i) = row(ui, pal, "Icons", &hint, |ui| {
                            segmented(ui, pal, Id::new("icon_set"), &["File Flier", "GNOME"], sel)
                        }) {
                            self.cfg.gnome_icons = i == 1;
                            crate::system_icons::set_enabled(self.cfg.gnome_icons, &ctx);
                            self.cfg.save();
                        }
                        const TITLE_ICONS: [TitleIcon; 4] =
                            [TitleIcon::Logo, TitleIcon::Files, TitleIcon::Folder, TitleIcon::None];
                        let ti = TITLE_ICONS.iter().position(|t| *t == self.cfg.title_icon).unwrap_or(0);
                        if let Some(i) =
                            row(ui, pal, "Title bar icon", "Top left; Files is the Files app's icon", |ui| {
                                segmented(
                                    ui,
                                    pal,
                                    Id::new("title_icon"),
                                    &["File Flier", "Files", "Folder", "None"],
                                    ti,
                                )
                            })
                        {
                            self.cfg.title_icon = TITLE_ICONS[i];
                            self.cfg.save();
                        }
                        if !cfg!(target_os = "macos") {
                            let hint = "Native: round, as in Ubuntu and GNOME, where your desktop puts them";
                            let sel = usize::from(self.cfg.native_buttons);
                            if let Some(i) = row(ui, pal, "Window buttons", hint, |ui| {
                                segmented(ui, pal, Id::new("win_buttons"), &["File Flier", "Native"], sel)
                            }) {
                                self.cfg.native_buttons = i == 1;
                                self.cfg.save();
                            }
                        }
                        // Theme / On / Off: the GNOME themes turn these on.
                        const TRI: [Option<bool>; 3] = [None, Some(true), Some(false)];
                        let tri = |v: Option<bool>| TRI.iter().position(|t| *t == v).unwrap_or(0);
                        let tweaks: [Tweak; 3] = [
                            (
                                "rounded",
                                "Corners",
                                "Rows, tabs, buttons, menus and the window (after a restart)",
                                ["Theme", "Round", "Square"],
                                self.cfg.rounded,
                            ),
                            (
                                "gnome_sidebar",
                                "Sidebar",
                                "GNOME: the desktop's font, roomier rows, no accent bar",
                                ["Theme", "GNOME", "File Flier"],
                                self.cfg.gnome_sidebar,
                            ),
                            (
                                "gtk_text",
                                "Text",
                                "GTK draws light text on dark backgrounds thinner",
                                ["Theme", "GTK", "File Flier"],
                                self.cfg.gtk_text,
                            ),
                        ];
                        for (id, label, hint, options, value) in tweaks {
                            if let Some(i) =
                                row(ui, pal, label, hint, |ui| segmented(ui, pal, Id::new(id), &options, tri(value)))
                            {
                                let v = TRI[i];
                                match id {
                                    "rounded" => self.cfg.rounded = v,
                                    "gnome_sidebar" => self.cfg.gnome_sidebar = v,
                                    _ => self.cfg.gtk_text = v,
                                }
                                restyle = true;
                            }
                        }

                        section(ui, pal, "Folder colors");
                        let hint = "Right-click a folder to color it. Its color follows the scheme.";
                        let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
                        text(ui.painter(), hr.left_center(), Align2::LEFT_CENTER, hint, 12.5, pal.text_dim);
                        ui.add_space(6.0);
                        for (i, s) in super::tint::SCHEMES.iter().enumerate() {
                            let colors = super::tint::scheme(i as u8, pal.accent);
                            let selected = i == self.cfg.folder_scheme as usize;
                            if scheme_row(ui, pal, s.name, &colors, selected).clicked() && !selected {
                                self.set_folder_scheme(i as u8, &ctx);
                            }
                            ui.add_space(2.0);
                        }

                        section(ui, pal, "Effects");

                        if row(ui, pal, "Animations", "Smooth transitions for lists, menus and previews", |ui| {
                            toggle(ui, pal, Id::new("anim"), self.cfg.animations)
                        }) {
                            self.cfg.animations = !self.cfg.animations;

                            restyle = true;
                        }

                        if row(ui, pal, "Tooltips", "Hints that appear when you hover over buttons and items", |ui| {
                            toggle(ui, pal, Id::new("tooltips"), self.cfg.tooltips)
                        }) {
                            self.cfg.tooltips = !self.cfg.tooltips;
                            restyle = true;
                        }

                        if row(ui, pal, "Frosted glass", "Translucent, floating panels over a soft backdrop", |ui| {
                            toggle(ui, pal, Id::new("glass"), self.cfg.glass)
                        }) {
                            self.cfg.glass = !self.cfg.glass;

                            restyle = true;
                        }

                        if self.cfg.glass {
                            const LEVELS: [f32; 4] = [0.5, 0.62, 0.72, 0.85];

                            let li = LEVELS
                                .iter()
                                .enumerate()
                                .min_by(|a, b| {
                                    (a.1 - self.cfg.glass_opacity)
                                        .abs()
                                        .total_cmp(&(b.1 - self.cfg.glass_opacity).abs())
                                })
                                .map_or(2, |(i, _)| i);

                            if let Some(i) = row(ui, pal, "Glass strength", "", |ui| {
                                segmented(ui, pal, Id::new("glass_lvl"), &["Airy", "Light", "Balanced", "Solid"], li)
                            }) {
                                self.cfg.glass_opacity = LEVELS[i];

                                restyle = true;
                            }

                            let st = match self.cfg.glass_see_through {
                                None => 0,

                                Some(true) => 1,

                                Some(false) => 2,
                            };

                            if let Some(i) = row(
                                ui,
                                pal,
                                "Show desktop behind window",
                                if self.cfg.see_through() && !self.window_transparent {
                                    "Restart File Flier to apply"
                                } else {
                                    "Blurred on KDE Plasma and macOS · Auto turns it on only there"
                                },
                                |ui| segmented(ui, pal, Id::new("see_through"), &["Auto", "On", "Off"], st),
                            ) {
                                self.cfg.glass_see_through = [None, Some(true), Some(false)][i];

                                restyle = true;
                            }
                        }

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
                        let views = [
                            ViewMode::Details,
                            ViewMode::List,
                            ViewMode::Grid,
                            ViewMode::MediumGrid,
                            ViewMode::LargeGrid,
                            ViewMode::ExtraLargeGrid,
                            ViewMode::Tree,
                            ViewMode::Columns,
                        ];
                        let vi = views.iter().position(|v| *v == self.cfg.view).unwrap_or(0);
                        if let Some(i) = row(ui, pal, "Default view", "For new tabs", |ui| {
                            segmented(
                                ui,
                                pal,
                                Id::new("view"),
                                &["Details", "List", "Grid", "M", "L", "XL", "Tree", "Columns"],
                                vi,
                            )
                        }) {
                            self.cfg.view = views[i];
                            restyle = true;
                        }
                        let tiles: [(crate::ext::Ext, BarToggle); 2] = [
                            (
                                crate::ext::Ext::Ratings,
                                ("Ratings on grid tiles", "Ctrl+Shift+R switches ratings and tags", |c| {
                                    &mut c.grid_ratings
                                }),
                            ),
                            (crate::ext::Ext::Tags, ("Tags on grid tiles", "", |c| &mut c.grid_tags)),
                        ];
                        for (ext, (name, hint, get)) in tiles {
                            if !self.cfg.ext_on(ext) {
                                continue;
                            }
                            let on = *get(&mut self.cfg);
                            if row(ui, pal, name, hint, |ui| toggle(ui, pal, Id::new(("grid_meta", name)), on)) {
                                *get(&mut self.cfg) = !on;
                                self.cfg.save();
                            }
                        }

                        section(ui, pal, "Bottom bar");
                        let bars: [BarToggle; 4] = [
                            ("Bottom bar", "Ctrl+J shows or hides it", |c| &mut c.show_bottom_bar),
                            ("Folder count", "", |c| &mut c.show_folder_count),
                            ("File count", "", |c| &mut c.show_file_count),
                            ("Scroll position", "How far down the list you are, in %", |c| &mut c.show_scroll_pct),
                        ];
                        for (i, (name, hint, get)) in bars.into_iter().enumerate() {
                            if i > 0 && !self.cfg.show_bottom_bar {
                                break;
                            }
                            let on = *get(&mut self.cfg);
                            if row(ui, pal, name, hint, |ui| toggle(ui, pal, Id::new(("bottom_bar", name)), on)) {
                                *get(&mut self.cfg) = !on;
                                self.cfg.save();
                            }
                        }

                        section(ui, pal, "Sidebar");
                        let mut groups = vec!["Bookmarks", "Places", "Storage", "Recents"];
                        if self.cfg.ext_on(crate::ext::Ext::Tags) {
                            groups.push("Tags");
                        }
                        for g in groups {
                            let on = !self.cfg.sidebar_hidden.iter().any(|h| h == g);
                            if row(ui, pal, g, "", |ui| toggle(ui, pal, Id::new(("sb_group", g)), on)) {
                                if on {
                                    self.cfg.sidebar_hidden.push(g.to_string());
                                } else {
                                    self.cfg.sidebar_hidden.retain(|h| h != g);
                                }
                                self.cfg.save();
                            }
                        }
                        if row(ui, pal, "Folder trees", "An arrow expands a folder into its subfolders", |ui| {
                            toggle(ui, pal, Id::new("sb_trees"), self.cfg.sidebar_trees)
                        }) {
                            self.cfg.sidebar_trees = !self.cfg.sidebar_trees;
                            self.cfg.save();
                        }

                        section(ui, pal, "Browsing");
                        if row(ui, pal, "Show hidden files", &super::keys("Files starting with a dot (Ctrl+H)"), |ui| {
                            toggle(ui, pal, Id::new("hidden"), self.cfg.show_hidden)
                        }) {
                            self.cfg.show_hidden = !self.cfg.show_hidden;
                            refilter = true;
                        }
                        if row(ui, pal, "Folders first", "", |ui| {
                            toggle(ui, pal, Id::new("ff"), self.cfg.sort.folders_first)
                        }) {
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

                        section(ui, pal, "Previews");
                        if row(ui, pal, "Show preview panel", "Big preview and file details on the right (F3)", |ui| {
                            toggle(ui, pal, Id::new("pvpanel"), self.cfg.show_preview)
                        }) {
                            self.cfg.show_preview = !self.cfg.show_preview;
                            self.cfg.save();
                        }
                        let lo_hint = match crate::preview::libreoffice_available() {
                            Some(true) => "Real pages for Word, Excel, PowerPoint and OpenDocument files",
                            Some(false) if crate::ops::in_flatpak() && !crate::ops::flatpak_can_spawn_on_host() => {
                                "Needs the same host access as \"Open terminal here\""
                            }
                            Some(false) => "Install LibreOffice to see real pages (text previews work without it)",
                            None => "Checking for LibreOffice…",
                        };
                        if row(ui, pal, "Office page previews", lo_hint, |ui| {
                            toggle(ui, pal, Id::new("lopages"), self.cfg.office_previews)
                        }) {
                            self.cfg.office_previews = !self.cfg.office_previews;
                            self.cfg.save();
                        }

                        section(ui, pal, "Extensions");
                        for e in crate::ext::Ext::ALL {
                            let on = self.cfg.ext_on(e);
                            if row(ui, pal, e.name(), e.description(), |ui| {
                                toggle(ui, pal, Id::new(("ext", e.id())), on)
                            }) {
                                self.cfg.extensions.insert(e.id().to_string(), !on);
                                self.cfg.save();
                            }
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

                        if crate::updater::ENABLED {
                            section(ui, pal, "Updates");
                            self.updates_section(ui);
                        }

                        ui.add_space(16.0);
                        ui.horizontal(|ui| {
                            let (r, resp) = ui.allocate_exact_size(vec2(150.0, 30.0), Sense::click());
                            ui.painter().rect_filled(
                                r,
                                pal.round(5.0),
                                if resp.hovered() { pal.tab_hover } else { pal.input },
                            );
                            ui.painter().rect_stroke(
                                r,
                                pal.round(5.0),
                                Stroke::new(1.0, pal.border),
                                StrokeKind::Inside,
                            );
                            text(ui.painter(), r.center(), Align2::CENTER_CENTER, "Reset appearance", 13.5, pal.text);
                            if resp.clicked() {
                                self.cfg.theme = Some(ThemeId::Dark);
                                self.cfg.accent = None;
                                self.cfg.ui_scale = 1.0;
                                self.cfg.density = Density::Comfortable;
                                self.cfg.animations = true;
                                self.cfg.glass = false;
                                self.cfg.glass_opacity = 0.72;
                                self.cfg.glass_see_through = None;
                                restyle = true;
                            }
                        });
                        ui.add_space(8.0);
                    },
                );
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
