//! The Ratings and Tags extensions' UI: star rows, tag dots, the "Tags…" dialog
//! for the selection, and the tag manager (add, rename, recolor, delete).

use std::path::PathBuf;

use egui::{Align2, Color32, Id, Key, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

use super::popups::{button, label, plain_input};
use super::text;
use crate::app::{FileFlier, display_name};
use crate::meta::{TAG_COLORS, TagDef};
use crate::theme::Palette;

/// Star yellow, readable on dark and light themes.
pub const STAR: Color32 = Color32::from_rgb(0xF5, 0xB7, 0x2E);

/// A five-pointed star centred on `c` with outer radius `r`.
pub fn paint_star(p: &Painter, c: Pos2, r: f32, fill: Option<Color32>, stroke: Color32) {
    let pts: Vec<Pos2> = (0..10)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let rad = if i % 2 == 0 { r } else { r * 0.45 };
            c + vec2(a.cos(), a.sin()) * rad
        })
        .collect();
    if let Some(f) = fill {
        // A star isn't convex: fill the middle pentagon, then each point.
        let inner: Vec<Pos2> = pts.iter().skip(1).step_by(2).copied().collect();
        p.add(Shape::convex_polygon(inner, f, Stroke::NONE));
        for i in 0..5 {
            let tip = pts[i * 2];
            let (l, rt) = (pts[(i * 2 + 9) % 10], pts[i * 2 + 1]);
            p.add(Shape::convex_polygon(vec![tip, rt, l], f, Stroke::NONE));
        }
    }
    p.add(Shape::closed_line(pts, Stroke::new(1.1, stroke)));
}

/// Five stars from `left`, centred on `cy`, `size` apart. Returns the stars
/// clicked (1-5; clicking the current rating clears it) if any.
pub fn star_row(ui: &mut Ui, id: Id, left: f32, cy: f32, size: f32, rating: u8, pal: &Palette) -> Option<u8> {
    let full = Rect::from_min_size(pos2(left, cy - size / 2.0), vec2(size * 5.0, size));
    let resp = ui.interact(full, id, Sense::click());
    // Hovering previews the rating a click would give.
    let hover = resp.hover_pos().map(|p| (((p.x - left) / size).floor() as i32 + 1).clamp(1, 5) as u8);
    let shown = hover.unwrap_or(rating);
    let p = ui.painter();
    for i in 0..5u8 {
        let c = pos2(left + size * (i as f32 + 0.5), cy);
        let on = i < shown;
        let fill = on.then(|| if hover.is_some() && i >= rating { STAR.gamma_multiply(0.6) } else { STAR });
        paint_star(p, c, size * 0.42, fill, if on { STAR } else { pal.text_faint });
    }
    let tip = match rating {
        0 => "Click to rate".to_string(),
        n => format!("Rated {n} of 5 · click a star to change, the same one to clear"),
    };
    let resp = resp.on_hover_text(tip);
    if resp.clicked()
        && let Some(h) = hover
    {
        return Some(if h == rating { 0 } else { h });
    }
    None
}

/// Small colored dots for `tags`, right-aligned to `right`. Returns how wide they are.
pub fn paint_tag_dots(p: &Painter, right: f32, cy: f32, tags: &[&TagDef], pal: &Palette) -> f32 {
    let mut x = right - 5.0;
    for t in tags.iter().rev().take(5) {
        p.circle_filled(pos2(x, cy), 4.5, t.color32());
        p.circle_stroke(pos2(x, cy), 4.5, Stroke::new(1.0, pal.bg));
        x -= 8.0;
    }
    right - x
}

/// The tag manager's state.
#[derive(Default)]
pub struct ManageTags {
    pub new_name: String,
    /// The tag being renamed, and the name typed so far.
    pub renaming: Option<(String, String)>,
    /// A tag whose Delete was clicked once (click again to confirm).
    pub confirm_delete: Option<String>,
    pub error: Option<String>,
}

fn color_choices(ui: &mut Ui, id: Id, current: [u8; 3]) -> Option<[u8; 3]> {
    let mut picked = None;
    for (i, c) in TAG_COLORS.iter().enumerate() {
        let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
        let col = Color32::from_rgb(c[0], c[1], c[2]);
        let rad = if resp.hovered() { 8.5 } else { 7.5 };
        ui.painter().circle_filled(r.center(), rad, col);
        if *c == current {
            ui.painter().circle_stroke(r.center(), 9.5, Stroke::new(1.5, ui.visuals().strong_text_color()));
        }
        if ui.interact(r, id.with(i), Sense::click()).clicked() {
            picked = Some(*c);
        }
    }
    // Any color at all.
    let mut custom = current;
    if egui::color_picker::color_edit_button_srgb(ui, &mut custom).on_hover_text("Custom color").changed() {
        picked = Some(custom);
    }
    picked
}

impl FileFlier {
    /// Rate `paths` with `stars` (0 clears the rating).
    pub fn rate(&mut self, paths: &[PathBuf], stars: u8) {
        if paths.is_empty() {
            return;
        }
        self.meta.set_rating(paths, stars);
        if self.cfg.sort.key == crate::fs_model::SortKey::Rating {
            self.resort_all();
        }
        let what = if paths.len() == 1 { display_name(&paths[0]) } else { format!("{} items", paths.len()) };
        self.info(match stars {
            0 => format!("Removed the rating from {what}"),
            1 => format!("Rated {what} 1 star"),
            n => format!("Rated {what} {n} stars"),
        });
    }

    /// "Tags…" for the selection: tick the tags the items should have, or add a new one.
    /// Returns true to close.
    pub(crate) fn tag_files_ui(
        &mut self,
        ui: &mut Ui,
        paths: &[PathBuf],
        new_tag: &mut String,
        error: &mut Option<String>,
    ) -> bool {
        let pal = self.pal();
        let mut close = false;
        let title = match paths {
            [one] => format!("Tags for “{}”", display_name(one)),
            _ => format!("Tags for {} items", paths.len()),
        };
        label(ui, pal, &title, 15.0, None);
        ui.add_space(4.0);
        let tags = self.meta.tags.clone();
        if tags.is_empty() {
            label(ui, pal, "No tags yet. Type a name below to make one.", 13.0, Some(pal.text_dim));
        }
        let mut toggle: Option<(String, bool)> = None;
        for t in &tags {
            let have = paths.iter().filter(|p| self.meta.tags_of(p).contains(&t.name)).count();
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(r, 5.0, pal.hover);
            }
            // Tick box: full, partly (some of the items), or empty.
            let bx = Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(16.0, 16.0));
            if have == 0 {
                p.rect_stroke(bx, 4.0, Stroke::new(1.3, pal.text_dim), egui::StrokeKind::Inside);
            } else {
                p.rect_filled(bx, 4.0, pal.accent);
                if have == paths.len() {
                    super::paint_row_icon(p, bx.shrink(1.0), super::RowIcon::Check(true), pal);
                } else {
                    p.hline(bx.shrink(4.0).x_range(), bx.center().y, Stroke::new(2.0, pal.on_accent));
                }
            }
            p.circle_filled(pos2(r.left() + 40.0, r.center().y), 6.0, t.color32());
            text(p, pos2(r.left() + 54.0, r.center().y), Align2::LEFT_CENTER, &t.name, 14.0, pal.text);
            let n = self.meta.count(&t.name);
            text(p, pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, n.to_string(), 12.5, pal.text_faint);
            if resp.clicked() {
                toggle = Some((t.name.clone(), have < paths.len()));
            }
        }
        if let Some((name, on)) = toggle {
            self.meta.set_tag(paths, &name, on);
            self.refresh_tag_views();
        }
        ui.add_space(8.0);
        let resp = plain_input(ui, pal, new_tag, Id::new("new_tag"));
        if new_tag.is_empty() && !resp.has_focus() {
            text(
                ui.painter(),
                resp.rect.left_center() + vec2(10.0, 0.0),
                Align2::LEFT_CENTER,
                "New tag…",
                14.0,
                pal.text_faint,
            );
        }
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if let Some(e) = error.as_ref() {
            label(ui, pal, e, 13.0, Some(Color32::from_rgb(240, 120, 110)));
        }
        ui.add_space(8.0);
        let mut add = enter;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            add |= button(ui, pal, "Add tag", !new_tag.trim().is_empty(), false).clicked();
            if button(ui, pal, "Manage tags…", false, false).clicked() {
                self.actions.push(crate::app::Action::Run(crate::commands::Command::ManageTags));
                close = true;
            }
            if button(ui, pal, "Done", new_tag.trim().is_empty(), false).clicked() {
                close = true;
            }
        });
        if add && !new_tag.trim().is_empty() {
            let name = new_tag.trim().to_string();
            match crate::meta::check_tag_name(&name) {
                Ok(()) => {
                    self.meta.set_tag(paths, &name, true);
                    self.refresh_tag_views();
                    new_tag.clear();
                    *error = None;
                    resp.request_focus();
                }
                Err(e) => *error = Some(e.to_string()),
            }
        }
        close
    }

    /// The tag manager. Returns true to close.
    pub(crate) fn manage_tags_ui(&mut self, ui: &mut Ui, st: &mut ManageTags) -> bool {
        let pal = self.pal();
        let mut close = false;
        label(ui, pal, "Tags", 15.0, None);
        label(
            ui,
            pal,
            "Rename, recolor or delete tags. Deleting a tag takes it off every file.",
            13.0,
            Some(pal.text_dim),
        );
        ui.add_space(4.0);
        let tags = self.meta.tags.clone();
        let mut changed = false;
        for t in &tags {
            ui.horizontal(|ui| {
                ui.set_min_height(34.0);
                ui.spacing_mut().item_spacing.x = 6.0;
                let id = Id::new(("tag_color", &t.name));
                if let Some(c) = color_choices(ui, id, t.color) {
                    self.meta.set_color(&t.name, c);
                }
                ui.add_space(6.0);
                let renaming = st.renaming.as_ref().is_some_and(|(n, _)| *n == t.name);
                if renaming {
                    let (_, typed) = st.renaming.as_mut().unwrap();
                    let w = (ui.available_width() - 150.0).max(80.0);
                    let resp = ui.add_sized(vec2(w, 26.0), egui::TextEdit::singleline(typed).id(Id::new("tag_rename")));
                    resp.request_focus();
                    let save = button(ui, pal, "Save", true, false).clicked()
                        || (resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
                    if save {
                        let typed = typed.clone();
                        match self.meta.rename_tag(&t.name, &typed) {
                            Ok(()) => {
                                st.renaming = None;
                                st.error = None;
                                changed = true;
                            }
                            Err(e) => st.error = Some(e.to_string()),
                        }
                    }
                } else {
                    let n = self.meta.count(&t.name);
                    let w = (ui.available_width() - 160.0).max(60.0);
                    let (r, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
                    let p = ui.painter();
                    text(p, r.left_center(), Align2::LEFT_CENTER, &t.name, 14.0, pal.text);
                    let files = format!("{n} file{}", if n == 1 { "" } else { "s" });
                    text(p, r.right_center(), Align2::RIGHT_CENTER, files, 12.5, pal.text_faint);
                    if button(ui, pal, "Rename", false, false).clicked() {
                        st.renaming = Some((t.name.clone(), t.name.clone()));
                    }
                    let confirm = st.confirm_delete.as_deref() == Some(t.name.as_str());
                    if button(ui, pal, if confirm { "Delete?" } else { "Delete" }, false, confirm).clicked() {
                        if confirm {
                            self.meta.delete_tag(&t.name);
                            st.confirm_delete = None;
                            changed = true;
                        } else {
                            st.confirm_delete = Some(t.name.clone());
                        }
                    }
                }
            });
        }
        if tags.is_empty() {
            label(ui, pal, "No tags yet.", 13.0, Some(pal.text_dim));
        }
        ui.add_space(8.0);
        let resp = plain_input(ui, pal, &mut st.new_name, Id::new("manage_new_tag"));
        if st.new_name.is_empty() && !resp.has_focus() {
            text(
                ui.painter(),
                resp.rect.left_center() + vec2(10.0, 0.0),
                Align2::LEFT_CENTER,
                "New tag…",
                14.0,
                pal.text_faint,
            );
        }
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if let Some(e) = &st.error {
            label(ui, pal, e, 13.0, Some(Color32::from_rgb(240, 120, 110)));
        }
        ui.add_space(8.0);
        let mut add = enter;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            add |= button(ui, pal, "Add tag", !st.new_name.trim().is_empty(), false).clicked();
            if button(ui, pal, "Done", st.new_name.trim().is_empty(), false).clicked() {
                close = true;
            }
        });
        if add && !st.new_name.trim().is_empty() {
            let color = TAG_COLORS[self.meta.tags.len() % TAG_COLORS.len()];
            match self.meta.add_tag(&st.new_name, color) {
                Ok(()) => {
                    st.new_name.clear();
                    st.error = None;
                }
                Err(e) => st.error = Some(e.to_string()),
            }
        }
        if changed {
            self.refresh_tag_views();
        }
        close
    }

    /// Show every file tagged `tag` in the current tab.
    pub fn show_tag(&mut self, tag: &str) {
        self.meta.prune();
        let files = self.meta.files_with(tag);
        let (h, s) = (self.cfg.show_hidden, self.cfg.sort);
        self.tab_mut().set_tag_view(tag.to_string(), files, h, s);
    }

    /// Tags changed: tag views list their files again.
    pub fn refresh_tag_views(&mut self) {
        let (h, s) = (self.cfg.show_hidden, self.cfg.sort);
        let meta = &self.meta;
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            if let Some(tag) = tab.tag.clone() {
                tab.set_tag_view(tag.clone(), meta.files_with(&tag), h, s);
            }
        }
    }
}
