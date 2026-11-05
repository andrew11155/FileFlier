//! Columns (Miller) view. The middle column is the pane's normal one-column
//! list, so selection, rename, drag and drop and file operations all work as
//! usual. To its left are the folders above it, each with the way down
//! highlighted; to its right, what's inside the folder under the cursor (or
//! details of the file under it). `→` steps into a folder, `←` back out.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Align2, Id, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

use super::pane_view::{file_kind, type_label};
use super::{elided, text};
use crate::app::{Action, FileFlier, format_time, human_size};
use crate::fs_model::{self, Entry};
use crate::icons;

/// Narrowest a column gets before older parent columns are dropped.
const COL_W: f32 = 230.0;

impl FileFlier {
    /// Draw the side columns in `rect` and return where the middle column goes.
    pub(crate) fn columns_ui(&mut self, ui: &mut Ui, idx: usize, rect: Rect) -> Rect {
        let fit = (rect.width() / COL_W).floor() as usize;
        if fit < 2 {
            return rect; // too narrow: just the list
        }
        let tab = self.panes[idx].tab();
        let path = tab.path.clone();
        let cursor = tab.cursor_entry().cloned();
        // ancestors[j] is the (j+1)th folder up; its highlighted item is the jth.
        let ancestors: Vec<PathBuf> = path.ancestors().map(Path::to_path_buf).collect();
        let n_left = (ancestors.len() - 1).min(fit - 2);
        let w = rect.width() / (n_left + 2) as f32;
        let col = |k: usize| {
            Rect::from_min_max(
                pos2(rect.left() + k as f32 * w, rect.top()),
                pos2(rect.left() + (k + 1) as f32 * w, rect.bottom()),
            )
        };
        let pal = self.pal();
        for k in 0..n_left {
            let j = n_left - 1 - k;
            let r = col(k);
            self.side_column(ui, idx, ("parent", k), r, &ancestors[j + 1], Some(&ancestors[j]));
            ui.painter().vline(r.right(), r.y_range(), Stroke::new(1.0, pal.border));
        }
        let middle = col(n_left);
        let right = col(n_left + 1);
        ui.painter().vline(right.left(), right.y_range(), Stroke::new(1.0, pal.border));
        match cursor {
            Some(e) if e.is_dir => self.side_column(ui, idx, ("child", 0), right, &e.path, None),
            Some(e) => self.file_column(ui, right, &e),
            None => {}
        }
        middle
    }

    /// The contents of `dir`, for a side column (re-read every couple of seconds).
    fn listing(&mut self, dir: &Path) -> Arc<Vec<Entry>> {
        let fresh = self.column_cache.get(dir).filter(|(t, _)| t.elapsed() < Duration::from_secs(2));
        if let Some((_, list)) = fresh {
            return list.clone();
        }
        let mut list = fs_model::read_dir(dir).unwrap_or_default();
        fs_model::sort_entries(&mut list, self.cfg.sort);
        let show_hidden = self.cfg.show_hidden;
        list.retain(|e| show_hidden || !e.is_hidden());
        let list = Arc::new(list);
        if self.column_cache.len() > 64 {
            self.column_cache.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(10));
        }
        self.column_cache.insert(dir.to_path_buf(), (Instant::now(), list.clone()));
        list
    }

    /// One read-only column listing `dir`. Clicking an item goes to it.
    fn side_column(
        &mut self,
        ui: &mut Ui,
        idx: usize,
        key: (&str, usize),
        r: Rect,
        dir: &Path,
        highlight: Option<&Path>,
    ) {
        let pal = self.pal();
        let list = self.listing(dir);
        let now = ui.input(|i| i.time);
        let row_h = self.cfg.density.row_height();
        let id = Id::new(("miller", idx, key));
        // Scroll the highlighted item into view when it changes.
        let hl_index = highlight.and_then(|h| list.iter().position(|e| e.path == h));
        let last: Option<PathBuf> = ui.ctx().data(|d| d.get_temp(id.with("hl")));
        let mut area = egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]);
        if last.as_deref() != highlight
            || ui.ctx().data(|d| d.get_temp::<PathBuf>(id.with("dir"))).as_deref() != Some(dir)
        {
            let off = hl_index.map_or(0.0, |i| (i as f32 * row_h - r.height() / 2.0 + row_h).max(0.0));
            area = area.vertical_scroll_offset(off);
            ui.ctx().data_mut(|d| {
                d.insert_temp(id.with("hl"), highlight.map(Path::to_path_buf));
                d.insert_temp(id.with("dir"), dir.to_path_buf());
            });
        }
        let mut actions = Vec::new();
        ui.scope_builder(UiBuilder::new().max_rect(r.shrink2(vec2(0.0, 2.0))), |ui| {
            ui.set_clip_rect(r);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            if list.is_empty() {
                text(
                    ui.painter(),
                    pos2(r.center().x, r.top() + 24.0),
                    Align2::CENTER_CENTER,
                    "Empty folder",
                    13.0,
                    pal.text_faint,
                );
                return;
            }
            area.show_rows(ui, row_h, list.len(), |ui, range| {
                for i in range {
                    let e = &list[i];
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                    let item = row.shrink2(vec2(6.0, 1.0));
                    let resp = ui.interact(item, id.with(i), Sense::click());
                    let on_path = Some(e.path.as_path()) == highlight;
                    let p = ui.painter();
                    if on_path {
                        p.rect_filled(item, 4.0, pal.accent_dim);
                    } else if resp.hovered() {
                        p.rect_filled(item, 4.0, pal.hover);
                    }
                    let color = if e.is_hidden() { pal.text.gamma_multiply(0.55) } else { pal.text };
                    let icon = Rect::from_center_size(pos2(item.left() + 16.0, item.center().y), vec2(18.0, 18.0));
                    if e.is_dir {
                        super::tint::Tints::new(&self.cfg.folder_colors, self.cfg.folder_scheme, &self.tint_anims, now)
                            .folder(p, icon, pal, &e.path);
                        let cr = Rect::from_center_size(pos2(item.right() - 10.0, item.center().y), vec2(12.0, 12.0));
                        icons::chevron_right(p, cr, pal.text_dim);
                    } else {
                        icons::file(p, icon, file_kind(e), pal);
                    }
                    let g = elided(p, &e.name, 13.5, color, item.right() - 24.0 - (item.left() + 34.0));
                    p.galley(pos2(item.left() + 34.0, item.center().y - g.size().y / 2.0), g, color);
                    if resp.clicked() {
                        actions.push(Action::Activate(idx));
                        actions.push(Action::Reveal(e.path.clone()));
                    }
                    if resp.double_clicked() {
                        actions.push(Action::Activate(idx));
                        if e.is_dir {
                            actions.push(Action::Navigate(e.path.clone()));
                        } else {
                            actions.push(Action::Reveal(e.path.clone()));
                            actions.push(Action::OpenEntries);
                        }
                    }
                    resp.on_hover_text(e.path.to_string_lossy());
                }
            });
        });
        self.actions.extend(actions);
    }

    /// The right-hand column when the cursor is on a file: what it is.
    fn file_column(&mut self, ui: &mut Ui, r: Rect, e: &Entry) {
        let pal = self.pal();
        let p = ui.painter().with_clip_rect(r);
        let cx = r.center().x;
        let icon = Rect::from_center_size(pos2(cx, r.top() + 70.0), vec2(72.0, 72.0));
        icons::file(&p, icon, file_kind(e), pal);
        let mut y = icon.bottom() + 18.0;
        let g = elided(&p, &e.name, 14.5, pal.text_strong, r.width() - 24.0);
        p.galley(pos2(cx - g.size().x / 2.0, y), g, pal.text_strong);
        y += 26.0;
        let mut lines = vec![type_label(e), human_size(e.size)];
        if let Some(m) = e.modified {
            lines.push(format!("Modified {}", format_time(m, self.cfg.date_style)));
        }
        for l in lines {
            text(&p, pos2(cx, y), Align2::CENTER_TOP, l, 13.0, pal.text_dim);
            y += 20.0;
        }
    }
}
