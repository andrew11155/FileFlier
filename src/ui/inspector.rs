//! Right-hand inspector: previews images, text and folder contents plus metadata.

use egui::{Align2, Color32, Rect, RichText, Stroke, Ui, UiBuilder, pos2, vec2};

use super::{elided, text};
use crate::app::{FileFlier, Preview, format_mode, format_time, human_size, load_preview};
use crate::icons;
use crate::ui::pane_view::{file_kind, type_label};

impl FileFlier {
    pub(crate) fn inspector_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let pal = self.pal();
        ui.painter().rect_filled(rect, 0.0, pal.sidebar);
        ui.painter().vline(rect.left() + 0.5, rect.y_range(), Stroke::new(1.0, pal.border));

        let Some(e) = self.tab().cursor_entry().cloned() else {
            text(ui.painter(), rect.center(), Align2::CENTER_CENTER, "Nothing selected", 14.0, pal.text_dim);
            return;
        };
        let stale = self.preview.as_ref().is_none_or(|(p, m, _)| p != &e.path || *m != e.modified);
        if stale {
            self.preview = Some((e.path.clone(), e.modified, load_preview(&e)));
        }

        let inner = rect.shrink2(vec2(14.0, 12.0));
        ui.scope_builder(UiBuilder::new().max_rect(inner).id_salt("inspector"), |ui| {
            ui.set_clip_rect(rect);
            // Title row.
            let (tr, _) = ui.allocate_exact_size(vec2(inner.width(), 30.0), egui::Sense::hover());
            let ir = Rect::from_center_size(pos2(tr.left() + 12.0, tr.center().y), vec2(24.0, 24.0));
            if e.is_dir {
                icons::folder(ui.painter(), ir, pal);
            } else {
                icons::file(ui.painter(), ir, file_kind(&e), pal);
            }
            let g = elided(ui.painter(), &e.name, 16.0, pal.text_strong, tr.width() - 36.0);
            ui.painter().galley(pos2(tr.left() + 32.0, tr.center().y - g.size().y / 2.0), g, pal.text_strong);
            ui.add_space(8.0);

            // Preview area.
            let preview_h = (inner.height() * 0.55).max(120.0);
            let (pr, _) = ui.allocate_exact_size(vec2(inner.width(), preview_h), egui::Sense::hover());
            ui.painter().rect_filled(pr, 6.0, pal.bg);
            ui.painter().rect_stroke(pr, 6.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
            let content = pr.shrink(8.0);
            match &self.preview.as_ref().unwrap().2 {
                Preview::Image => {
                    let uri = format!("file://{}", e.path.display());
                    let img = egui::Image::new(uri);
                    if let Ok(poll) = img.load_for_size(ui.ctx(), content.size())
                        && let Some(sz) = poll.size()
                    {
                        let scale = (content.width() / sz.x).min(content.height() / sz.y).min(1.0);
                        img.corner_radius(4).paint_at(ui, Rect::from_center_size(content.center(), sz * scale));
                    }
                }
                Preview::Text(t) => {
                    ui.scope_builder(UiBuilder::new().max_rect(content), |ui| {
                        ui.set_clip_rect(content);
                        egui::ScrollArea::both().id_salt("insp_text").auto_shrink(false).show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(t).monospace().size(12.0).color(pal.text)).extend());
                        });
                    });
                }
                Preview::Dir(entries) => {
                    ui.scope_builder(UiBuilder::new().max_rect(content), |ui| {
                        ui.set_clip_rect(content);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        egui::ScrollArea::vertical().id_salt("insp_dir").auto_shrink(false).show_rows(
                            ui,
                            26.0,
                            entries.len(),
                            |ui, range| {
                                for c in &entries[range] {
                                    let (r, _) =
                                        ui.allocate_exact_size(vec2(ui.available_width(), 26.0), egui::Sense::hover());
                                    let ir =
                                        Rect::from_center_size(pos2(r.left() + 12.0, r.center().y), vec2(17.0, 17.0));
                                    if c.is_dir {
                                        icons::folder(ui.painter(), ir, pal);
                                    } else {
                                        icons::file(ui.painter(), ir, file_kind(c), pal);
                                    }
                                    let g = elided(ui.painter(), &c.name, 13.5, pal.text, r.width() - 34.0);
                                    ui.painter().galley(
                                        pos2(r.left() + 28.0, r.center().y - g.size().y / 2.0),
                                        g,
                                        pal.text,
                                    );
                                }
                            },
                        );
                    });
                    if entries.is_empty() {
                        text(ui.painter(), content.center(), Align2::CENTER_CENTER, "Empty folder", 13.5, pal.text_dim);
                    }
                }
                Preview::Binary => {
                    let ir = Rect::from_center_size(content.center(), vec2(72.0, 72.0));
                    icons::file(ui.painter(), ir, file_kind(&e), pal);
                }
                Preview::Error(err) => {
                    text(
                        ui.painter(),
                        content.center(),
                        Align2::CENTER_CENTER,
                        err,
                        13.0,
                        Color32::from_rgb(230, 120, 110),
                    );
                }
            }
            ui.add_space(12.0);

            // Metadata.
            let mut rows: Vec<(&str, String)> = vec![("Type", type_label(&e))];
            match &self.preview.as_ref().unwrap().2 {
                Preview::Dir(entries) => rows.push(("Contains", format!("{} items", entries.len()))),
                _ => rows.push(("Size", format!("{} ({} bytes)", human_size(e.size), e.size))),
            }
            if let Some(m) = e.modified {
                rows.push(("Modified", format_time(m, self.cfg.date_style)));
            }
            if let Ok(meta) = std::fs::symlink_metadata(&e.path) {
                use std::os::unix::fs::PermissionsExt;
                rows.push(("Permissions", format_mode(meta.permissions().mode())));
            }
            if e.is_symlink
                && let Ok(t) = std::fs::read_link(&e.path)
            {
                rows.push(("Links to", t.to_string_lossy().into_owned()));
            }
            rows.push(("Location", e.path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()));
            for (k, v) in rows {
                let (r, _) = ui.allocate_exact_size(vec2(inner.width(), 22.0), egui::Sense::hover());
                text(ui.painter(), pos2(r.left(), r.center().y), Align2::LEFT_CENTER, k, 13.0, pal.text_dim);
                let g = elided(ui.painter(), &v, 13.0, pal.text, r.width() - 96.0);
                ui.painter().galley(pos2(r.left() + 96.0, r.center().y - g.size().y / 2.0), g, pal.text);
            }
        });
    }
}
