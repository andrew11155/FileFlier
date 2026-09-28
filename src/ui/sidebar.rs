//! Left sidebar: filter box plus collapsible Recents / Bookmarks / Storage / Places.

use std::path::PathBuf;

use egui::{Align2, CursorIcon, Id, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

use super::{elided, search_box, text};
use crate::app::{Action, DragPaths, FileFlier, MenuItem, display_name};
use crate::fuzzy;
use crate::icons::{self, Place};
use crate::mounts::MountKind;

const ITEM_H: f32 = 30.0;

struct Item {
    icon: Place,
    label: String,
    path: PathBuf,
    space: Option<crate::app::DiskSpace>,
    /// UDisks volume: unmounted ones mount on click, removable ones get an eject button.
    volume: Option<crate::udisks::Volume>,
}

impl FileFlier {
    pub(crate) fn sidebar_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let pal = self.pal();
        let fx = self.fx();
        super::surface(ui.painter(), rect, pal.sidebar, &fx, pal);
        if !fx.glass {
            ui.painter().vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, pal.border));
        }

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
            (Place::Trash, "Trash".into(), dirs::data_dir().map(|d| d.join("Trash"))),
        ]
        .into_iter()
        .filter_map(|(icon, label, path)| {
            path.filter(|p| p.is_dir() || matches!(icon, Place::Trash)).map(|path| Item {
                icon,
                label,
                path,
                space: None,
                volume: None,
            })
        })
        .collect();
        let bookmarks: Vec<Item> = self
            .cfg
            .bookmarks
            .iter()
            .map(|b| Item { icon: Place::Folder, label: display_name(b), path: b.clone(), space: None, volume: None })
            .collect();
        let recents: Vec<Item> = self
            .cfg
            .recent
            .iter()
            .filter(|p| p.is_dir())
            .take(10)
            .map(|p| Item { icon: Place::Folder, label: display_name(p), path: p.clone(), space: None, volume: None })
            .collect();
        let storage: Vec<Item> = self
            .mounts
            .iter()
            .map(|m| {
                let icon = match m.kind {
                    MountKind::Network => Place::Network,
                    MountKind::Cloud => Place::Cloud,
                    MountKind::Root | MountKind::Removable | MountKind::Unmounted => Place::Drive,
                };
                Item { icon, label: m.name.clone(), path: m.path.clone(), space: m.space, volume: m.volume.clone() }
            })
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
                    let hc = if hresp.hovered() { pal.text_strong } else { pal.text_dim };
                    super::text_bold(p, pos2(hr.left() + 34.0, hr.center().y), Align2::LEFT_CENTER, title, 13.0, hc);
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
                        let h = if item.space.is_some() { ITEM_H + 6.0 } else { ITEM_H };
                        let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
                        let r = r.shrink2(vec2(6.0, 1.0));
                        let is_current = item.path == current;
                        let drop_hover = resp.dnd_hover_payload::<DragPaths>().is_some();
                        let p = ui.painter();
                        if is_current {
                            p.rect_filled(r, 4.0, pal.tab_hover);
                            // Accent marker for where you are.
                            let m = Rect::from_center_size(pos2(r.left() + 1.5, r.center().y), vec2(3.0, h * 0.5));
                            p.rect_filled(m, 1.5, pal.accent);
                        } else if resp.hovered() {
                            p.rect_filled(r, 4.0, pal.hover);
                        }
                        if drop_hover {
                            p.rect_stroke(r, 4.0, Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
                        }
                        let text_y = if item.space.is_some() { r.top() + 13.0 } else { r.center().y };
                        let ir = Rect::from_center_size(pos2(r.left() + 40.0, text_y), vec2(18.0, 18.0));
                        icons::place(p, ir, item.icon, pal);
                        let unmounted = item.volume.as_ref().is_some_and(|v| v.mount_points.is_empty());
                        let ejectable = item.volume.as_ref().is_some_and(|v| v.removable && !v.mount_points.is_empty());
                        let color = if is_current {
                            pal.text_strong
                        } else if unmounted {
                            pal.text_dim
                        } else {
                            pal.text
                        };
                        let eject_w = if ejectable { 26.0 } else { 0.0 };
                        let g = elided(p, &item.label, 14.0, color, r.right() - (r.left() + 58.0) - 6.0 - eject_w);
                        p.galley(pos2(r.left() + 58.0, text_y - g.size().y / 2.0), g, color);
                        if let Some(u) = item.space.map(|s| s.used()) {
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
                        let tip = match item.space {
                            Some(s) => format!(
                                "{}\n{} free of {}",
                                item.path.display(),
                                crate::app::human_size(s.free),
                                crate::app::human_size(s.total)
                            ),
                            None => item.path.display().to_string(),
                        };
                        let tip =
                            if unmounted { format!("{} — not mounted. Click to mount.", item.label) } else { tip };
                        let mut eject_clicked = false;
                        if ejectable && let Some(v) = &item.volume {
                            let er = Rect::from_center_size(pos2(r.right() - 14.0, text_y), vec2(22.0, 22.0));
                            let eresp = ui.interact(er, Id::new(("eject", &v.object)), Sense::click());
                            let c = if eresp.hovered() { pal.text_strong } else { pal.text_dim };
                            if eresp.hovered() {
                                ui.painter().rect_filled(er, 4.0, pal.tab_hover);
                            }
                            icons::eject(ui.painter(), er.shrink(4.0), c);
                            if eresp.on_hover_text("Eject — safe to unplug afterwards").clicked() {
                                actions.push(Action::EjectVolume(v.clone()));
                                eject_clicked = true;
                            }
                        }
                        let resp = resp.on_hover_text(tip);
                        if resp.clicked() && eject_clicked {
                            // handled by the eject button
                        } else if resp.clicked() && unmounted {
                            if let Some(v) = &item.volume {
                                actions.push(Action::MountVolume(v.object.clone()));
                            }
                        } else if resp.clicked() {
                            if matches!(item.icon, Place::Trash) {
                                actions.push(Action::Run(crate::commands::Command::ShowTrash));
                            } else {
                                actions.push(Action::Navigate(item.path.clone()));
                            }
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
                            if matches!(item.icon, Place::Trash) {
                                actions.push(Action::TrashPaths(pl.0.clone()));
                            } else {
                                actions.push(Action::Drop { paths: pl.0.clone(), dest: item.path.clone(), force_copy });
                            }
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
