//! Quick Look: a large preview of the item under the cursor (Space to toggle).

use egui::emath::TSTransform;
use egui::{Align2, Area, Color32, Id, Order, Rect, RichText, Sense, Stroke, StrokeKind, UiBuilder, pos2, vec2};

use super::{appear, elided, glass, text, wrapped};
use crate::app::{FileFlier, Preview, format_time, human_size, load_preview};
use crate::icons;
use crate::ui::pane_view::{file_kind, type_label};

impl FileFlier {
    pub(crate) fn quicklook_ui(&mut self, ctx: &egui::Context) {
        if !self.quicklook {
            return;
        }
        let Some(e) = self.tab().cursor_entry().cloned() else {
            self.quicklook = false;
            return;
        };
        let stale = self.preview.as_ref().is_none_or(|(p, m, _)| p != &e.path || *m != e.modified);
        if stale {
            self.preview = Some((e.path.clone(), e.modified, load_preview(&e)));
        }
        let pal = self.pal();
        let fx = self.fx();
        let date_style = self.cfg.date_style;
        let (pos, total) = (self.tab().cursor + 1, self.tab().visible.len());
        let screen = ctx.content_rect();
        let t = appear(ctx, Id::new("ql_appear"), fx.dur(0.18));
        let mut close = false;
        let mut open = false;

        // Dimmed backdrop; clicking it closes.
        Area::new(Id::new("ql_backdrop")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha((120.0 * t) as u8));
            close |= resp.clicked();
        });

        let size = vec2((screen.width() * 0.82).min(1040.0), (screen.height() * 0.84).min(740.0));
        let card = Rect::from_center_size(screen.center(), size);
        let preview = &self.preview.as_ref().unwrap().2;
        Area::new(Id::new("ql_card")).order(Order::Foreground).fixed_pos(card.min).show(ctx, |ui| {
            // Grow from 94% to full size while fading in.
            let s = 0.94 + 0.06 * t;
            let c = card.center().to_vec2();
            ui.ctx().set_transform_layer(ui.layer_id(), TSTransform::new(c * (1.0 - s), s));
            ui.multiply_opacity(t);

            let (r, _) = ui.allocate_exact_size(size, Sense::click()); // swallow clicks on the card
            let p = ui.painter().clone();
            p.add(
                egui::Shadow { offset: [0, 16], blur: 48, spread: 0, color: Color32::from_black_alpha(140) }
                    .as_shape(r, 14),
            );
            // Solid-ish even in glass mode: the app can't blur its own content behind it.
            let fill = if fx.glass { glass::with_alpha(pal.popup, 0.97) } else { pal.popup };
            if fx.glass {
                glass::panel(&p, r, fill, 14, pal);
            } else {
                p.rect_filled(r, 14.0, fill);
                p.rect_stroke(r, 14.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
            }

            // Header: icon, name, details, actions.
            let header = Rect::from_min_size(r.min, vec2(r.width(), 64.0));
            let icon = Rect::from_center_size(pos2(header.left() + 34.0, header.center().y), vec2(30.0, 30.0));
            if e.is_dir {
                icons::folder(&p, icon, pal);
            } else {
                icons::file(&p, icon, file_kind(&e), pal);
            }
            let mut meta = vec![type_label(&e)];
            if !e.is_dir {
                meta.push(human_size(e.size));
            }
            if let Preview::Image = preview {
                let img = egui::Image::new(format!("file://{}", e.path.display()));
                if let Ok(poll) = img.load_for_size(ui.ctx(), vec2(1.0, 1.0))
                    && let Some(sz) = poll.size()
                {
                    meta.push(format!("{} × {}", sz.x as u32, sz.y as u32));
                }
            }
            if let Some(m) = e.modified {
                meta.push(format_time(m, date_style));
            }
            let name_w = header.width() - 64.0 - 170.0;
            let g = elided(&p, &e.name, 17.0, pal.text_strong, name_w);
            p.galley(pos2(header.left() + 62.0, header.top() + 13.0), g, pal.text_strong);
            let g = elided(&p, &meta.join("  ·  "), 12.5, pal.text_dim, name_w);
            p.galley(pos2(header.left() + 62.0, header.top() + 37.0), g, pal.text_dim);

            let close_r = Rect::from_center_size(pos2(header.right() - 30.0, header.center().y), vec2(30.0, 30.0));
            close |= super::icon_button(ui, close_r, Id::new("ql_close"), true, pal, "Close (Space)", icons::close)
                .clicked();
            let open_r = Rect::from_min_size(pos2(close_r.left() - 96.0, header.center().y - 15.0), vec2(84.0, 30.0));
            let open_resp = ui.interact(open_r, Id::new("ql_open"), Sense::click());
            let fill = if open_resp.hovered() { pal.accent.gamma_multiply(1.15) } else { pal.accent };
            p.rect_filled(open_r, 6.0, fill);
            text(&p, open_r.center(), Align2::CENTER_CENTER, "Open", 13.5, pal.on_accent);
            open |= open_resp.on_hover_text("Open (Enter)").clicked();
            p.hline(r.x_range().shrink(1.0), header.bottom(), Stroke::new(1.0, pal.row_sep));

            // Footer: position and key hints.
            let footer = Rect::from_min_max(pos2(r.left(), r.bottom() - 34.0), r.max);
            p.hline(r.x_range().shrink(1.0), footer.top(), Stroke::new(1.0, pal.row_sep));
            text(
                &p,
                pos2(footer.left() + 18.0, footer.center().y),
                Align2::LEFT_CENTER,
                format!("{pos} of {total}"),
                12.0,
                pal.text_dim,
            );
            text(
                &p,
                pos2(footer.right() - 18.0, footer.center().y),
                Align2::RIGHT_CENTER,
                "↑ ↓ ← →  browse   ·   Enter  open   ·   Space  close",
                12.0,
                pal.text_dim,
            );

            // Body.
            let body = Rect::from_min_max(
                pos2(r.left() + 16.0, header.bottom() + 12.0),
                pos2(r.right() - 16.0, footer.top() - 12.0),
            );
            match preview {
                Preview::Image => {
                    let img = egui::Image::new(format!("file://{}", e.path.display()));
                    if let Ok(poll) = img.load_for_size(ui.ctx(), body.size())
                        && let Some(sz) = poll.size()
                    {
                        // Fit inside the body; small images may grow up to 2x.
                        let scale = (body.width() / sz.x).min(body.height() / sz.y).min(2.0);
                        img.corner_radius(6).paint_at(ui, Rect::from_center_size(body.center(), sz * scale));
                    } else {
                        ui.put(Rect::from_center_size(body.center(), vec2(32.0, 32.0)), egui::Spinner::new());
                    }
                }
                Preview::Text(t) => {
                    let well = pal.bg;
                    p.rect_filled(body, 8.0, well);
                    ui.scope_builder(UiBuilder::new().max_rect(body.shrink(12.0)), |ui| {
                        egui::ScrollArea::both().id_salt(("ql_text", &e.path)).auto_shrink(false).show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(t).monospace().size(13.0).color(pal.text)).extend());
                        });
                    });
                }
                Preview::Dir(entries) => {
                    if entries.is_empty() {
                        text(&p, body.center(), Align2::CENTER_CENTER, "Empty folder", 15.0, pal.text_dim);
                    }
                    ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
                        egui::ScrollArea::vertical().id_salt(("ql_dir", &e.path)).auto_shrink(false).show(ui, |ui| {
                            let tile = vec2(112.0, 104.0);
                            let cols = ((body.width() / tile.x).floor() as usize).max(1);
                            for chunk in entries.chunks(cols) {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    for c in chunk {
                                        let (tr, _) = ui.allocate_exact_size(tile, Sense::hover());
                                        let p = ui.painter();
                                        let ir = Rect::from_center_size(
                                            pos2(tr.center().x, tr.top() + 32.0),
                                            vec2(48.0, 48.0),
                                        );
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
                Preview::Binary | Preview::Error(_) => {
                    let ir = Rect::from_center_size(body.center() - vec2(0.0, 24.0), vec2(112.0, 112.0));
                    if e.is_dir {
                        icons::folder(&p, ir, pal);
                    } else {
                        icons::file(&p, ir, file_kind(&e), pal);
                    }
                    let msg = match preview {
                        Preview::Error(err) => err.clone(),
                        _ => "No preview available".into(),
                    };
                    text(&p, pos2(body.center().x, ir.bottom() + 28.0), Align2::CENTER_CENTER, msg, 14.0, pal.text_dim);
                }
            }
        });

        if open {
            self.quicklook = false;
            self.open_entries();
        } else if close {
            self.quicklook = false;
        }
    }
}
