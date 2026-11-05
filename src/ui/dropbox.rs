//! The Dropbox extension's dialogs: keeping folders online only, and the list of
//! folders that aren't synced on this computer.

use std::path::PathBuf;

use egui::{Align2, Sense, Stroke, Ui, pos2, vec2};

use super::popups::{button, label};
use super::{elided, text};
use crate::app::{FileFlier, display_name};

impl FileFlier {
    /// Confirms "keep online only" for `paths`. Returns true when it should close.
    pub(crate) fn dropbox_unsync_ui(&mut self, ui: &mut Ui, paths: &mut Vec<PathBuf>) -> bool {
        let pal = self.pal();
        let n = paths.len();
        let what = if n == 1 { format!("“{}”", display_name(&paths[0])) } else { format!("{n} folders") };
        label(ui, pal, &format!("Keep {what} online only?"), 16.0, Some(pal.text_strong));
        ui.add_space(6.0);
        label(
            ui,
            pal,
            "Dropbox removes the copy on this computer. Everything stays in your Dropbox and on your other devices. \
             You can sync it here again from the folder's parent: right-click an empty spot → Dropbox folders not synced here.",
            13.5,
            None,
        );
        ui.add_space(14.0);
        let mut close = false;
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 260.0).max(0.0));
            if button(ui, pal, "Cancel", false, false).clicked() {
                close = true;
            }
            ui.add_space(8.0);
            if button(ui, pal, "Keep online only", true, false).clicked() {
                crate::dropbox::set_synced(std::mem::take(paths), false, ui.ctx());
                close = true;
            }
        });
        close
    }

    /// Lists the Dropbox folders not synced here, each with "Sync here".
    pub(crate) fn dropbox_excluded_ui(&mut self, ui: &mut Ui, list: &mut Result<Vec<PathBuf>, String>) -> bool {
        let pal = self.pal();
        let mut close = false;
        label(ui, pal, "Dropbox folders not synced on this computer", 16.0, Some(pal.text_strong));
        ui.add_space(8.0);
        let mut resync = None;
        match list {
            Err(e) => label(ui, pal, &format!("Couldn't ask Dropbox: {e}"), 13.5, Some(super::danger_color(pal))),
            Ok(v) if v.is_empty() => label(ui, pal, "Every folder is synced here.", 13.5, None),
            Ok(v) => {
                let w = ui.available_width();
                let roots = crate::dropbox::roots();
                egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (i, p) in v.iter().enumerate() {
                        let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
                        let pnt = ui.painter();
                        pnt.hline(r.x_range(), r.top(), Stroke::new(1.0, pal.row_sep));
                        // Shown from the Dropbox folder down.
                        let shown = roots
                            .iter()
                            .filter_map(|root| p.strip_prefix(root).ok())
                            .min_by_key(|rel| rel.as_os_str().len())
                            .map(|rel| rel.to_string_lossy().into_owned())
                            .unwrap_or_else(|| p.to_string_lossy().into_owned());
                        crate::icons::folder_outline(
                            pnt,
                            egui::Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), vec2(17.0, 17.0)),
                            pal.text_dim,
                        );
                        let g = elided(pnt, &shown, 13.5, pal.text, w - 160.0);
                        pnt.galley(pos2(r.left() + 32.0, r.center().y - g.size().y / 2.0), g, pal.text);
                        let br = egui::Rect::from_min_size(pos2(r.right() - 110.0, r.top() + 6.0), vec2(104.0, 28.0));
                        let resp = ui.interact(br, ui.id().with(("dbx_resync", i)), Sense::click());
                        let fill = if resp.hovered() { pal.hover } else { pal.input };
                        ui.painter().rect_filled(br, pal.round(6.0), fill);
                        text(ui.painter(), br.center(), Align2::CENTER_CENTER, "Sync here", 13.0, pal.text);
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            resync = Some(i);
                        }
                    }
                });
            }
        }
        if let (Some(i), Ok(v)) = (resync, list.as_mut()) {
            let p = v.remove(i);
            crate::dropbox::set_synced(vec![p], true, ui.ctx());
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 70.0).max(0.0));
            if button(ui, pal, "Done", true, false).clicked() {
                close = true;
            }
        });
        close
    }
}
