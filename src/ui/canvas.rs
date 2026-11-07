//! The selection canvas: when several items are selected, the preview panel and
//! Quick Look lay them all out in a grid on a canvas you can zoom (scroll or pinch,
//! around the pointer) and pan (drag). Double-click empty space to fit everything
//! again, or an item to see it alone. A toggle shows each item's stars and tags
//! (click the stars of the item under the pointer to rate it).

use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use egui::{Align2, Color32, CursorIcon, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};

use crate::app::{FileFlier, human_size};
use crate::fs_model::Entry;
use crate::icons;
use crate::preview::thumbs::Tier;
use crate::ui::pane_view::file_kind;

/// The most items laid out (a huge selection shows its first ones).
const MAX_ITEMS: usize = 400;
/// A tile's size and the gap between tiles, before zooming.
const TILE: f32 = 200.0;
const GAP: f32 = 14.0;

/// Where a canvas is zoomed and panned to.
#[derive(Default)]
pub struct CanvasView {
    zoom: f32,
    /// Offset of the content's center from the canvas's center, in points.
    pan: Vec2,
    /// Showing everything (kept fitted as the canvas changes size).
    at_fit: bool,
    /// The selection it was laid out for; a new one fits again.
    key: u64,
}

/// The selection, when more than one item is selected.
pub struct Multi {
    /// The first `MAX_ITEMS`, in listing order.
    pub items: Vec<Entry>,
    pub total: usize,
    files: usize,
    folders: usize,
    bytes: u64,
}

impl Multi {
    /// "12 items selected", and what they are ("3 folders, 9 files · 34.2 MB").
    pub fn summary(&self) -> (String, String) {
        let title = format!("{} items selected", self.total);
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut parts = Vec::new();
        if self.folders > 0 {
            parts.push(plural(self.folders, "folder", "folders"));
        }
        if self.files > 0 {
            parts.push(plural(self.files, "file", "files"));
        }
        let mut sub = parts.join(", ");
        if self.files > 0 {
            sub.push_str(&format!(" · {}", human_size(self.bytes)));
        }
        if self.total > self.items.len() {
            sub.push_str(&format!(" · first {} shown", self.items.len()));
        }
        (title, sub)
    }
}

impl FileFlier {
    /// The selected items in the active tab, if more than one is selected.
    pub(crate) fn multi_selection(&self) -> Option<Multi> {
        let tab = self.tab();
        if tab.selected.len() < 2 {
            return None;
        }
        let mut m = Multi { items: Vec::new(), total: 0, files: 0, folders: 0, bytes: 0 };
        for e in tab.visible.iter().map(|&i| &tab.entries[i]).filter(|e| tab.selected.contains(&e.path)) {
            m.total += 1;
            if e.is_dir {
                m.folders += 1;
            } else {
                m.files += 1;
                m.bytes += e.size;
            }
            if m.items.len() < MAX_ITEMS {
                m.items.push(e.clone());
            }
        }
        (m.total >= 2).then_some(m)
    }

    /// Lays `items` out on a zoomable canvas filling `rect`. Returns the item
    /// double-clicked, to be shown alone.
    pub(crate) fn selection_canvas(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        id: &str,
        view: &mut CanvasView,
        items: &[Entry],
    ) -> Option<PathBuf> {
        let pal = self.pal();
        let n = items.len().max(1);
        // Enough columns for the grid to have about the canvas's shape.
        let aspect = (rect.width() / rect.height().max(1.0)).clamp(0.1, 10.0);
        let cols = ((n as f32 * aspect).sqrt().round() as usize).clamp(1, n);
        let rows = n.div_ceil(cols);
        let step = TILE + GAP;
        let content = vec2(cols as f32 * step - GAP, rows as f32 * step - GAP);
        let fit = ((rect.width() - 32.0) / content.x).min((rect.height() - 32.0) / content.y).clamp(0.02, 2.5);

        let mut h = std::collections::hash_map::DefaultHasher::new();
        for e in items {
            e.path.hash(&mut h);
        }
        let key = h.finish();
        if view.key != key {
            view.key = key;
            view.at_fit = true;
        }
        if view.at_fit {
            view.zoom = fit;
            view.pan = Vec2::ZERO;
        }

        let resp = ui.interact(rect, Id::new((id, "canvas")), Sense::click_and_drag());
        // Zoom around the pointer: scroll, Ctrl+scroll or pinch.
        if let Some(ptr) = resp.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = pinch * (scroll * 0.004).exp();
            if (factor - 1.0).abs() > 1e-4 {
                ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
                let z = (view.zoom * factor).clamp(fit * 0.5, 6.0f32.max(fit));
                let q = (ptr - rect.center() - view.pan) / view.zoom;
                view.pan = ptr - rect.center() - q * z;
                view.zoom = z;
                view.at_fit = false;
            }
        }
        if resp.dragged() {
            view.pan += resp.drag_delta();
            view.at_fit = false;
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }

        let z = view.zoom;
        let tile_px = TILE * z;
        let px = tile_px * ui.ctx().pixels_per_point();
        let tier = if px <= 256.0 {
            Tier::Large
        } else if px <= 512.0 {
            Tier::XLarge
        } else {
            Tier::XXLarge
        };
        let ratings = self.cfg.ext_on(crate::ext::Ext::Ratings);
        let tags_on = self.cfg.ext_on(crate::ext::Ext::Tags);
        let overlay = self.cfg.canvas_meta && (ratings || tags_on);
        let names = tile_px >= 90.0;
        let mut hovered = None;
        let mut rate: Option<(PathBuf, u8)> = None;
        let mut toggle = false;
        let mut refit = false;

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            let p = ui.painter().clone();
            p.rect_filled(rect, 0.0, pal.bg);
            for (i, e) in items.iter().enumerate() {
                let q = vec2((i % cols) as f32 * step, (i / cols) as f32 * step) - content / 2.0;
                let tile = Rect::from_min_size(rect.center() + view.pan + q * z, Vec2::splat(tile_px));
                if !tile.intersects(rect) {
                    continue;
                }
                let label_h = if names { 20.0 } else { 0.0 };
                let area = Rect::from_min_max(tile.min, pos2(tile.right(), tile.bottom() - label_h));
                let hov = ui.rect_contains_pointer(tile) && ui.rect_contains_pointer(rect);
                if hov {
                    hovered = Some(i);
                }
                let side = area.width().min(area.height());
                let mut shown = Rect::from_center_size(area.center(), Vec2::splat(side * 0.6));
                let mut drew = false;
                if e.is_file
                    && crate::preview::classify(&e.path).has_thumbnail()
                    && let Some(tex) = self.pv.thumbs.get(&e.path, e.modified, tier)
                {
                    let sz = tex.size_vec2();
                    let s = (area.width() / sz.x).min(area.height() / sz.y);
                    let fitr = Rect::from_center_size(area.center(), sz * s);
                    p.add(
                        egui::Shadow { offset: [0, 1], blur: 6, spread: 0, color: Color32::from_black_alpha(50) }
                            .as_shape(fitr, 3),
                    );
                    p.image(tex.id(), fitr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    shown = fitr;
                    drew = true;
                }
                if !drew {
                    if e.is_dir {
                        let tint =
                            super::tint::color_of(&self.cfg.folder_colors, self.cfg.folder_scheme, pal.accent, &e.path);
                        icons::folder_icon(&p, shown, pal, tint, crate::system_icons::special_folder(&e.path));
                    } else {
                        icons::file(&p, shown, file_kind(e), pal);
                    }
                }
                if hov {
                    p.rect_stroke(shown.expand(3.0), pal.round(5.0), Stroke::new(2.0, pal.accent), StrokeKind::Outside);
                }
                if names {
                    let g = super::elided(&p, &e.name, 12.0, pal.text_dim, tile.width());
                    p.galley(pos2(tile.center().x - g.size().x / 2.0, tile.bottom() - 16.0), g, pal.text_dim);
                }
                if !overlay {
                    continue;
                }
                self.meta.observe(&e.path);
                if tags_on {
                    let defs: Vec<&crate::meta::TagDef> =
                        self.meta.tags_of(&e.path).iter().filter_map(|t| self.meta.tag(t)).collect();
                    if !defs.is_empty() {
                        super::tags::paint_tag_dots(&p, shown.right() - 1.0, shown.top() + 7.0, &defs, pal);
                    }
                }
                // Stars along the picture's bottom once rated, or to rate the one under
                // the pointer.
                let rating = self.meta.rating(&e.path);
                if ratings && tile_px >= 70.0 && (rating > 0 || hov) {
                    let size = (tile_px / 12.0).clamp(10.0, 17.0);
                    let cx = shown.center().x;
                    let cy = shown.bottom() - size / 2.0 - 3.0;
                    let pill = Rect::from_center_size(pos2(cx, cy), vec2(size * 5.0 + 8.0, size + 4.0));
                    p.rect_filled(pill, pal.round(size / 2.0 + 2.0), pal.bg.gamma_multiply(0.82));
                    let left = cx - size * 2.5;
                    if hov {
                        let sid = Id::new((id, "stars", &e.path));
                        if let Some(n) = super::tags::star_row(ui, sid, left, cy, size, rating, pal) {
                            rate = Some((e.path.clone(), n));
                        }
                    } else {
                        for s in 0..5u8 {
                            let on = s < rating;
                            super::tags::paint_star(
                                &p,
                                pos2(left + size * (s as f32 + 0.5), cy),
                                size * 0.42,
                                on.then_some(super::tags::STAR),
                                if on { super::tags::STAR } else { pal.text_faint },
                            );
                        }
                    }
                }
            }

            // Buttons in the top right corner: fit, and the stars and tags toggle.
            let mut x = rect.right() - 8.0;
            if ratings || tags_on {
                let r = Rect::from_min_max(pos2(x - 28.0, rect.top() + 8.0), pos2(x, rect.top() + 36.0));
                p.rect_filled(r, pal.round(6.0), pal.popup.gamma_multiply(0.92));
                let on = self.cfg.canvas_meta;
                let tip = if on { "Hide stars and tags" } else { "Show stars and tags" };
                toggle = super::icon_button(ui, r, Id::new((id, "meta")), true, pal, tip, |p, ir, c| {
                    super::tags::paint_star(
                        p,
                        ir.center(),
                        ir.width() * 0.3,
                        on.then_some(super::tags::STAR),
                        if on { super::tags::STAR } else { c },
                    );
                })
                .clicked();
                x = r.left() - 6.0;
            }
            if !view.at_fit {
                let r = Rect::from_min_max(pos2(x - 44.0, rect.top() + 8.0), pos2(x, rect.top() + 36.0));
                let b = ui.interact(r, Id::new((id, "fit")), Sense::click());
                let fill = if b.hovered() { pal.hover } else { pal.popup.gamma_multiply(0.92) };
                p.rect_filled(r, pal.round(6.0), fill);
                super::text(&p, r.center(), Align2::CENTER_CENTER, "Fit", 12.5, pal.text);
                refit = b.on_hover_text("Show everything (double-click)").clicked();
            }
        });

        if toggle {
            self.cfg.canvas_meta = !self.cfg.canvas_meta;
            self.cfg.save();
        }
        if let Some((path, n)) = rate {
            self.rate(std::slice::from_ref(&path), n);
        }
        if refit {
            view.at_fit = true;
        }
        if resp.double_clicked() {
            match hovered {
                Some(i) => return Some(items[i].path.clone()),
                None => view.at_fit = true,
            }
        }
        if refit {
            ui.ctx().request_repaint();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_a_selection() {
        let m = Multi { items: Vec::new(), total: 4, files: 3, folders: 1, bytes: 2048 };
        let (title, sub) = m.summary();
        assert_eq!(title, "4 items selected");
        assert!(sub.starts_with("1 folder, 3 files · "), "{sub}");
        assert!(sub.ends_with("first 0 shown"), "{sub}");
    }
}
