//! Left sidebar: filter box plus collapsible Recents / Bookmarks / Storage / Places.

use std::path::PathBuf;

use egui::{Align2, CursorIcon, Id, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

use super::{elided, search_box, text};
use crate::app::{Action, DragPaths, FileFlier, MenuItem, display_name};
use crate::fuzzy;
use crate::icons::{self, Place};

const ITEM_H: f32 = 30.0;

struct Item {
    icon: Place,
    label: String,
    path: PathBuf,
    usage: Option<f32>,
}

impl FileFlier {
    pub(crate) fn sidebar_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let pal = self.pal();
        ui.painter().rect_filled(rect, 0.0, pal.sidebar);
        ui.painter().vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, pal.border));

        // Resize handle on the right edge.
        let handle = Rect::from_min_max(pos2(rect.right() - 3.0, rect.top()), pos2(rect.right() + 3.0, rect.bottom()));
        let h = ui.interact(handle, Id::new("sidebar_resize"), Sense::drag());
        if h.hovered() || h.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if h.dragged() {
            self.cfg.sidebar_width = (self.cfg.sidebar_width + h.drag_delta().x).clamp(160.0, 420.0);
        }
        if h.drag_stopped() {
            self.cfg.save();
        }

        let filter_r = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(rect.width() - 16.0, 30.0));
        let mut filter = std::mem::take(&mut self.sidebar_filter);
        search_box(ui, filter_r, pal, &mut filter, Id::new("sidebar_filter"), "Filter...", false);
        self.sidebar_filter = filter;

        let current = self.tab().path.clone();
        let home = dirs::home_dir();
        // Fall back to ~/Name when XDG user dirs aren't configured.
        let std_dir = |d: Option<PathBuf>, name: &str| d.or_else(|| home.as_ref().map(|h| h.join(name)));
        let places: Vec<Item> = [
            (Place::Home, home.as_ref().map(|h| display_name(h)).unwrap_or_else(|| "Home".into()), home.clone()),
            (Place::Desktop, "Desktop".into(), std_dir(dirs::desktop_dir(), "Desktop")),
            (Place::Downloads, "Downloads".into(), std_dir(dirs::download_dir(), "Downloads")),
            (Place::Documents, "Documents".into(), std_dir(dirs::document_dir(), "Documents")),
            (Place::Music, "Music".into(), std_dir(dirs::audio_dir(), "Music")),
            (Place::Pictures, "Pictures".into(), std_dir(dirs::picture_dir(), "Pictures")),
            (Place::Videos, "Videos".into(), std_dir(dirs::video_dir(), "Videos")),
            (Place::Trash, "Trash".into(), dirs::data_dir().map(|d| d.join("Trash/files"))),
        ]
        .into_iter()
        .filter_map(|(icon, label, path)| {
            path.filter(|p| p.is_dir()).map(|path| Item { icon, label, path, usage: None })
        })
        .collect();
        let bookmarks: Vec<Item> = self
            .cfg
            .bookmarks
            .iter()
            .map(|b| Item { icon: Place::Folder, label: display_name(b), path: b.clone(), usage: None })
            .collect();
        let recents: Vec<Item> = self
            .cfg
            .recent
            .iter()
            .filter(|p| p.is_dir())
            .take(10)
            .map(|p| Item { icon: Place::Folder, label: display_name(p), path: p.clone(), usage: None })
            .collect();
        let storage: Vec<Item> = self
            .mounts
            .1
            .iter()
            .map(|m| Item { icon: Place::Drive, label: m.name.clone(), path: m.path.clone(), usage: m.used })
            .collect();

        let q = self.sidebar_filter.trim().to_string();
        let filt = |items: Vec<Item>| -> Vec<Item> {
            items.into_iter().filter(|i| q.is_empty() || fuzzy::score(&q, &i.label).is_some()).collect()
        };
        let sections = [
            ("Recents", Place::Recent, filt(recents)),
            ("Bookmarks", Place::Bookmark, filt(bookmarks)),
            ("Storage", Place::Drive, filt(storage)),
            ("Places", Place::Folder, filt(places)),
        ];

        let list =
            Rect::from_min_max(pos2(rect.left(), filter_r.bottom() + 8.0), pos2(rect.right() - 1.0, rect.bottom()));
        let mut toggles: Vec<String> = Vec::new();
        let mut actions: Vec<Action> = Vec::new();
        let force_copy = ui.input(|i| i.modifiers.command);
        ui.scope_builder(UiBuilder::new().max_rect(list).id_salt("sidebar"), |ui| {
            ui.set_clip_rect(list);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let w = ui.available_width();
                for (title, icon, items) in sections {
                    let collapsed = self.cfg.collapsed.iter().any(|c| c == title) && q.is_empty();
                    // Section header.
                    let (hr, hresp) = ui.allocate_exact_size(vec2(w, ITEM_H + 2.0), Sense::click());
                    let hr = hr.shrink2(vec2(6.0, 1.0));
                    let p = ui.painter();
                    if hresp.hovered() {
                        p.rect_filled(hr, 4.0, pal.tab_hover);
                    }
                    let ir = Rect::from_center_size(pos2(hr.left() + 16.0, hr.center().y), vec2(17.0, 17.0));
                    match icon {
                        Place::Folder => {
                            let r = ir.shrink(1.0);
                            p.rect_stroke(r, 2.0, Stroke::new(1.4, pal.text_dim), egui::StrokeKind::Middle);
                            p.hline(r.x_range(), r.top() + 4.0, Stroke::new(1.2, pal.text_dim));
                        }
                        other => icons::place(p, ir, other, pal),
                    }
                    text(p, pos2(hr.left() + 34.0, hr.center().y), Align2::LEFT_CENTER, title, 14.0, pal.text);
                    let cr = Rect::from_center_size(pos2(hr.right() - 14.0, hr.center().y), vec2(16.0, 16.0));
                    if collapsed {
                        icons::chevron_right(p, cr, pal.text_dim);
                    } else {
                        icons::chevron_down(p, cr, pal.text_dim);
                    }
                    if hresp.clicked() {
                        toggles.push(title.to_string());
                    }
                    if collapsed {
                        continue;
                    }
                    if items.is_empty() && title == "Bookmarks" {
                        let (r, _) = ui.allocate_exact_size(vec2(w, ITEM_H - 4.0), Sense::hover());
                        text(
                            ui.painter(),
                            pos2(r.left() + 40.0, r.center().y),
                            Align2::LEFT_CENTER,
                            "Ctrl+D to bookmark",
                            12.5,
                            pal.text_faint,
                        );
                    }
                    for item in &items {
                        let h = if item.usage.is_some() { ITEM_H + 6.0 } else { ITEM_H };
                        let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
                        let r = r.shrink2(vec2(6.0, 1.0));
                        let is_current = item.path == current;
                        let drop_hover = resp.dnd_hover_payload::<DragPaths>().is_some();
                        let p = ui.painter();
                        if is_current {
                            p.rect_filled(r, 4.0, pal.tab_hover);
                        } else if resp.hovered() {
                            p.rect_filled(r, 4.0, pal.hover);
                        }
                        if drop_hover {
                            p.rect_stroke(r, 4.0, Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
                        }
                        let text_y = if item.usage.is_some() { r.top() + 13.0 } else { r.center().y };
                        let ir = Rect::from_center_size(pos2(r.left() + 40.0, text_y), vec2(18.0, 18.0));
                        icons::place(p, ir, item.icon, pal);
                        let color = if is_current { pal.text_strong } else { pal.text };
                        let g = elided(p, &item.label, 14.0, color, r.right() - (r.left() + 58.0) - 6.0);
                        p.galley(pos2(r.left() + 58.0, text_y - g.size().y / 2.0), g, color);
                        if let Some(u) = item.usage {
                            let bar = Rect::from_min_size(
                                pos2(r.left() + 58.0, text_y + 11.0),
                                vec2((r.width() - 74.0).max(20.0), 3.0),
                            );
                            p.rect_filled(bar, 1.0, pal.border);
                            let full = u > 0.9;
                            let fill = if full { egui::Color32::from_rgb(200, 70, 60) } else { pal.accent };
                            p.rect_filled(
                                Rect::from_min_size(bar.min, vec2(bar.width() * u.clamp(0.0, 1.0), 3.0)),
                                1.0,
                                fill,
                            );
                        }
                        let resp = resp.on_hover_text(item.path.to_string_lossy());
                        if resp.clicked() {
                            actions.push(Action::Navigate(item.path.clone()));
                        }
                        if resp.middle_clicked() {
                            actions.push(Action::OpenInNewTab(item.path.clone()));
                        }
                        if resp.secondary_clicked() {
                            // Bookmarks can be removed from their context menu.
                            let pos = resp.interact_pointer_pos().unwrap_or(r.center());
                            let mut items =
                                vec![MenuItem::Go(item.path.clone()), MenuItem::NewTabAt(item.path.clone())];
                            if title == "Bookmarks" {
                                items.push(MenuItem::Sep);
                                items.push(MenuItem::Unbookmark(item.path.clone()));
                            }
                            actions.push(Action::OpenMenu(pos, items));
                        }
                        if let Some(pl) = resp.dnd_release_payload::<DragPaths>() {
                            actions.push(Action::Drop { paths: pl.0.clone(), dest: item.path.clone(), force_copy });
                        }
                    }
                    ui.add_space(6.0);
                }
            });
        });
        for t in toggles {
            if let Some(i) = self.cfg.collapsed.iter().position(|c| *c == t) {
                self.cfg.collapsed.remove(i);
            } else {
                self.cfg.collapsed.push(t);
            }
            self.cfg.save();
        }
        self.actions.extend(actions);
    }
}
