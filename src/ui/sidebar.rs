//! Left sidebar: filter box plus collapsible Bookmarks / Places / Storage / Recents,
//! with folders expandable into a tree of what is really on disk.

use std::path::PathBuf;

use egui::{Align2, CursorIcon, Id, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

use super::{elided, search_box, text};
use crate::app::{Action, DragPaths, DropTarget, FileFlier, MenuItem, display_name};
use crate::fuzzy;
use crate::icons::{self, Place};
use crate::mounts::MountKind;

const ITEM_H: f32 = 30.0;
/// Rows inside a folder tree are a little shorter, so more of the tree fits.
const CHILD_H: f32 = 26.0;
const INDENT: f32 = 14.0;
/// Folders with more subfolders than this show the rest as "N more…".
const MAX_CHILDREN: usize = 150;

#[derive(Default)]
struct Item {
    icon: Place,
    label: String,
    path: PathBuf,
    space: Option<crate::app::DiskSpace>,
    /// UDisks volume: unmounted ones mount on click, removable ones get an eject button.
    volume: Option<crate::udisks::Volume>,
    /// Cloud account signed in through File Flier.
    cloud: Option<crate::cloud::Account>,
    /// A cloud account that isn't connected (click to connect).
    offline: bool,
    /// The "Add cloud storage…" row.
    add_cloud: bool,
    /// Nesting level in a folder tree (0 = the section's own rows).
    depth: usize,
    /// A folder that can be expanded to show its subfolders.
    tree: bool,
    /// Whether its subfolders are showing.
    open: bool,
    /// The "N more…" row of a big folder (`path` is that folder).
    more: bool,
    /// Which tree the row belongs to (section and root folder), so the same folder can
    /// appear in two trees without sharing open/closed state or widget IDs.
    tree_key: String,
}

/// Identifies one folder tree in the sidebar.
fn tree_key(section: &str, root: &std::path::Path) -> String {
    format!("{section}|{}", root.display())
}

impl Item {
    fn folder(icon: Place, label: String, path: PathBuf) -> Self {
        Item { icon, label, path, tree: true, ..Default::default() }
    }
}

impl FileFlier {
    /// Adds `item` to `out`, followed by its subfolders (recursively) if it's open.
    fn push_tree(
        &self,
        section: &str,
        mut item: Item,
        icons_for: &[(PathBuf, Place)],
        ctx: &egui::Context,
        out: &mut Vec<Item>,
    ) {
        item.tree_key = section.to_string();
        let key = (section.to_string(), item.path.clone());
        if !item.tree || !self.sidebar_open.contains(&key) {
            if item.tree && self.dir_tree.known_empty(&item.path, self.cfg.show_hidden) {
                item.tree = false;
            }
            out.push(item);
            return;
        }
        let children = self.dir_tree.children(&item.path, self.cfg.show_hidden, ctx);
        if children.as_ref().is_some_and(|c| c.is_empty()) {
            item.tree = false;
        }
        item.open = item.tree;
        let (depth, parent) = (item.depth + 1, item.path.clone());
        out.push(item);
        let Some(children) = children else { return };
        for c in children.iter().take(MAX_CHILDREN) {
            let icon = icons_for.iter().find(|(p, _)| p == c).map(|(_, i)| *i).unwrap_or(Place::Folder);
            let child = Item { depth, ..Item::folder(icon, display_name(c), c.clone()) };
            self.push_tree(section, child, icons_for, ctx, out);
        }
        if children.len() > MAX_CHILDREN {
            out.push(Item {
                icon: Place::Folder,
                label: format!("{} more…", children.len() - MAX_CHILDREN),
                path: parent,
                depth,
                more: true,
                ..Default::default()
            });
        }
    }

    pub(crate) fn sidebar_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let pal = self.pal();
        let fx = self.fx();
        let ctx = ui.ctx().clone();
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
        // The user folders this system actually has (from XDG user-dirs / macOS), shown
        // with their own icons wherever they appear in the tree.
        // (Falls back to ~/Name when XDG user dirs aren't configured.)
        let icons_for: Vec<(PathBuf, Place)> = [
            (dirs::desktop_dir(), "Desktop", Place::Desktop),
            (dirs::document_dir(), "Documents", Place::Documents),
            (dirs::download_dir(), "Downloads", Place::Downloads),
            (dirs::audio_dir(), "Music", Place::Music),
            (dirs::picture_dir(), "Pictures", Place::Pictures),
            (dirs::video_dir(), if cfg!(target_os = "macos") { "Movies" } else { "Videos" }, Place::Videos),
        ]
        .into_iter()
        .filter_map(|(p, name, i)| {
            p.or_else(|| home.as_ref().map(|h| h.join(name))).filter(|p| Some(p) != home.as_ref()).map(|p| (p, i))
        })
        .collect();

        // Places, like Dolphin's or GNOME Files': Home and the standard folders that
        // exist here (under their real, possibly translated, names), the folders
        // bookmarked in the desktop's file manager, then Trash. Each one expands.
        let mut place_roots: Vec<Item> = Vec::new();
        if let Some(h) = &home {
            place_roots.push(Item::folder(Place::Home, "Home".into(), h.clone()));
        }
        for (p, icon) in &icons_for {
            if p.is_dir() {
                place_roots.push(Item::folder(*icon, display_name(p), p.clone()));
            }
        }
        if cfg!(target_os = "macos") {
            place_roots.push(Item::folder(Place::Folder, "Applications".into(), PathBuf::from("/Applications")));
        }
        for (label, p) in self.dir_tree.system_bookmarks() {
            let shown = place_roots.iter().any(|i| i.path == p) || self.cfg.bookmarks.contains(&p);
            let trash = crate::trashview::trash_dir().is_some_and(|t| t == p);
            if !shown && !trash {
                let cloud = self.mounts.iter().any(|m| m.cloud.is_some() || m.kind == MountKind::Cloud)
                    && crate::mounts::is_remote_path(&p, &self.mounts);
                place_roots.push(Item::folder(if cloud { Place::Cloud } else { Place::Folder }, label, p));
            }
        }
        let bookmark_roots: Vec<Item> =
            self.cfg.bookmarks.iter().map(|b| Item::folder(Place::Folder, display_name(b), b.clone())).collect();
        let storage_roots: Vec<Item> = self
            .mounts
            .iter()
            .map(|m| {
                let icon = match m.kind {
                    MountKind::Network => Place::Network,
                    MountKind::Cloud | MountKind::CloudOffline => Place::Cloud,
                    MountKind::Root | MountKind::Removable | MountKind::Unmounted => Place::Drive,
                };
                let offline = m.kind == MountKind::CloudOffline;
                let unmounted = m.kind == MountKind::Unmounted;
                Item {
                    icon,
                    label: m.name.clone(),
                    path: m.path.clone(),
                    space: m.space,
                    volume: m.volume.clone(),
                    cloud: m.cloud.clone(),
                    offline,
                    tree: !offline && !unmounted && !m.path.as_os_str().is_empty(),
                    ..Default::default()
                }
            })
            .collect();

        // Open the tree down to the current folder when it changes (under the closest root).
        let reveal = self.sidebar_revealed.as_ref() != Some(&current);
        if reveal {
            self.sidebar_revealed = Some(current.clone());
            let best = place_roots
                .iter()
                .map(|i| ("Places", &i.path))
                .chain(storage_roots.iter().filter(|i| i.tree).map(|i| ("Storage", &i.path)))
                .filter(|(_, p)| current.starts_with(p))
                .max_by_key(|(_, p)| p.components().count());
            if let Some((section, root)) = best
                && let Ok(rel) = current.strip_prefix(root)
            {
                let hidden = rel.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
                if self.cfg.show_hidden || !hidden {
                    let mut p = root.clone();
                    for c in rel.components() {
                        self.sidebar_open.insert((tree_key(section, root), p.clone()));
                        p.push(c);
                    }
                }
            }
        }

        let mut places: Vec<Item> = Vec::new();
        for r in place_roots {
            self.push_tree(&tree_key("Places", &r.path), r, &icons_for, &ctx, &mut places);
        }
        if let Some(t) = crate::trashview::trash_dir() {
            places.push(Item { icon: Place::Trash, label: "Trash".into(), path: t, ..Default::default() });
        }
        let mut bookmarks: Vec<Item> = Vec::new();
        for r in bookmark_roots {
            self.push_tree(&tree_key("Bookmarks", &r.path), r, &icons_for, &ctx, &mut bookmarks);
        }
        let mut storage: Vec<Item> = Vec::new();
        for r in storage_roots {
            self.push_tree(&tree_key("Storage", &r.path), r, &icons_for, &ctx, &mut storage);
        }
        if crate::cloud::SUPPORTED {
            storage.push(Item {
                icon: Place::Cloud,
                label: "Add cloud storage…".into(),
                add_cloud: true,
                ..Default::default()
            });
        }
        let recents: Vec<Item> = self
            .cfg
            .recent
            .iter()
            .filter(|p| p.is_dir())
            .take(10)
            .map(|p| Item { icon: Place::Folder, label: display_name(p), path: p.clone(), ..Default::default() })
            .collect();

        let q = self.sidebar_filter.trim().to_string();
        let filt = |items: Vec<Item>| -> Vec<Item> {
            items.into_iter().filter(|i| q.is_empty() || fuzzy::score(&q, &i.label).is_some()).collect()
        };
        let sections = [
            ("Bookmarks", Place::Bookmark, filt(bookmarks)),
            ("Places", Place::Folder, filt(places)),
            ("Storage", Place::Drive, filt(storage)),
            ("Recents", Place::Recent, filt(recents)),
        ];

        let list =
            Rect::from_min_max(pos2(rect.left(), filter_r.bottom() + 8.0), pos2(rect.right() - 1.0, rect.bottom()));
        let mut toggles: Vec<String> = Vec::new();
        let mut tree_toggles: Vec<(String, PathBuf, bool)> = Vec::new();
        let mut new_bookmarks: Vec<PathBuf> = Vec::new();
        let mut actions: Vec<Action> = Vec::new();
        let mut drop_target: Option<DropTarget> = None;
        let force_copy = ui.input(|i| i.modifiers.command);
        let dragging = egui::DragAndDrop::has_payload_of_type::<DragPaths>(&ctx);
        ui.scope_builder(UiBuilder::new().max_rect(list).id_salt("sidebar"), |ui| {
            ui.set_clip_rect(list);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                super::dnd::drag_scroll(ui, list);
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
                    // Folders dropped on the Bookmarks header get bookmarked.
                    if title == "Bookmarks"
                        && let Some(pl) = hresp.dnd_hover_payload::<DragPaths>()
                        && pl.0.iter().any(|p| p.is_dir())
                    {
                        p.rect_stroke(hr, 4.0, Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
                        if let Some(pl) = hresp.dnd_release_payload::<DragPaths>() {
                            new_bookmarks.extend(pl.0.iter().filter(|p| p.is_dir()).cloned());
                        }
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
                        let hint = if dragging { "Drop folders here" } else { "Ctrl+D to bookmark" };
                        text(
                            ui.painter(),
                            pos2(r.left() + 40.0, r.center().y),
                            Align2::LEFT_CENTER,
                            super::keys(hint),
                            12.5,
                            pal.text_faint,
                        );
                    }
                    for item in &items {
                        let h = if item.space.is_some() {
                            ITEM_H + 6.0
                        } else if item.depth > 0 {
                            CHILD_H
                        } else {
                            ITEM_H
                        };
                        let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
                        let r = r.shrink2(vec2(6.0, 1.0));
                        let is_current = item.path == current && !item.more;
                        let unmounted = item.volume.as_ref().is_some_and(|v| v.mount_points.is_empty())
                            || item.offline
                            || item.add_cloud;
                        let droppable = !unmounted && !item.more;
                        let drop_hover = droppable && resp.dnd_hover_payload::<DragPaths>().is_some();
                        let indent = item.depth as f32 * INDENT;
                        let p = ui.painter();
                        if is_current {
                            p.rect_filled(r, 4.0, pal.tab_hover);
                            // Accent marker for where you are.
                            let m = Rect::from_center_size(pos2(r.left() + 1.5, r.center().y), vec2(3.0, h * 0.5));
                            p.rect_filled(m, 1.5, pal.accent);
                            if reveal {
                                ui.scroll_to_rect(r, None);
                            }
                        } else if resp.hovered() {
                            p.rect_filled(r, 4.0, pal.hover);
                        }
                        let is_trash = matches!(item.icon, Place::Trash) && item.depth == 0 && !item.tree;
                        if drop_hover {
                            p.rect_stroke(r, 4.0, Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
                            drop_target =
                                Some(if is_trash { DropTarget::Trash } else { DropTarget::Folder(item.path.clone()) });
                            if !is_trash {
                                // Resting on a folder opens it (and its subfolders in the tree).
                                let (t, fire) = super::dnd::spring(
                                    ui.ctx(),
                                    Id::new(("spring_sb", title, &item.tree_key, &item.path)),
                                    super::dnd::SPRING_DELAY,
                                );
                                p.rect_filled(r, 4.0, pal.accent.gamma_multiply(0.30 * t));
                                if fire {
                                    if item.tree && !item.open {
                                        tree_toggles.push((item.tree_key.clone(), item.path.clone(), true));
                                    }
                                    actions.push(Action::Navigate(item.path.clone()));
                                }
                            }
                        }
                        let text_y = if item.space.is_some() { r.top() + 13.0 } else { r.center().y };
                        // Disclosure arrow for folders that can be expanded.
                        let chev = Rect::from_center_size(pos2(r.left() + 13.0 + indent, text_y), vec2(18.0, 18.0));
                        let mut chev_clicked = false;
                        if item.tree {
                            let cresp =
                                ui.interact(chev, Id::new(("sb_tree", &item.tree_key, &item.path)), Sense::click());
                            let c = if cresp.hovered() { pal.text_strong } else { pal.text_faint };
                            let cr = chev.shrink(4.0);
                            if item.open {
                                icons::chevron_down(ui.painter(), cr, c);
                            } else {
                                icons::chevron_right(ui.painter(), cr, c);
                            }
                            if cresp.clicked() {
                                tree_toggles.push((item.tree_key.clone(), item.path.clone(), !item.open));
                                chev_clicked = true;
                            }
                        }
                        let p = ui.painter();
                        let isz = if item.depth > 0 { 16.0 } else { 18.0 };
                        let ir = Rect::from_center_size(pos2(r.left() + 32.0 + indent, text_y), vec2(isz, isz));
                        if item.add_cloud {
                            let c = if resp.hovered() { pal.text_strong } else { pal.text_dim };
                            icons::plus(p, ir.shrink(2.0), c);
                        } else if !item.more {
                            icons::place(p, ir, item.icon, pal);
                        }
                        let ejectable = item.volume.as_ref().is_some_and(|v| v.removable && !v.mount_points.is_empty());
                        let color = if is_current {
                            pal.text_strong
                        } else if unmounted || item.more {
                            pal.text_dim
                        } else {
                            pal.text
                        };
                        let eject_w = if ejectable { 26.0 } else { 0.0 };
                        let label_x = r.left() + 48.0 + indent;
                        let size = if item.depth > 0 { 13.5 } else { 14.0 };
                        let g = elided(p, &item.label, size, color, r.right() - label_x - 6.0 - eject_w);
                        p.galley(pos2(label_x, text_y - g.size().y / 2.0), g, color);
                        if let Some(u) = item.space.map(|s| s.used()) {
                            let bar = Rect::from_min_size(
                                pos2(label_x, text_y + 11.0),
                                vec2((r.right() - label_x - 16.0).max(20.0), 3.0),
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
                        // Cloud drives are named by account; their mount folder is an internal detail.
                        let place =
                            if item.cloud.is_some() { item.label.clone() } else { item.path.display().to_string() };
                        let tip = match item.space {
                            Some(s) => format!(
                                "{}\n{} free of {}",
                                place,
                                crate::app::human_size(s.free),
                                crate::app::human_size(s.total)
                            ),
                            None => place,
                        };
                        let tip = if item.add_cloud {
                            "Sign in to Google Drive, OneDrive or Dropbox".to_string()
                        } else if item.more {
                            format!("Open {}", item.path.display())
                        } else if item.offline {
                            format!("{} — not connected. Click to connect.", item.label)
                        } else if unmounted {
                            format!("{} — not mounted. Click to mount.", item.label)
                        } else {
                            tip
                        };
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
                        if resp.clicked() && (eject_clicked || chev_clicked) {
                            // handled by the eject button / disclosure arrow
                        } else if resp.clicked() && item.add_cloud {
                            actions.push(Action::AddCloud);
                        } else if resp.clicked() && item.offline {
                            if let Some(a) = &item.cloud {
                                actions.push(Action::CloudConnect(a.name.clone()));
                            }
                        } else if resp.clicked() && unmounted {
                            if let Some(v) = &item.volume {
                                actions.push(Action::MountVolume(v.object.clone()));
                            }
                        } else if resp.clicked() {
                            if is_trash {
                                actions.push(Action::Run(crate::commands::Command::ShowTrash));
                            } else {
                                actions.push(Action::Navigate(item.path.clone()));
                            }
                        } else if resp.double_clicked() && item.tree {
                            tree_toggles.push((item.tree_key.clone(), item.path.clone(), !item.open));
                        }
                        if unmounted {
                            // Nothing to open or drop onto yet; a signed-in account can still be signed out.
                            if resp.secondary_clicked()
                                && let Some(a) = &item.cloud
                            {
                                let pos = resp.interact_pointer_pos().unwrap_or(r.center());
                                actions.push(Action::OpenMenu(pos, vec![MenuItem::CloudSignOut(a.name.clone())]));
                            }
                            continue;
                        }
                        if item.more {
                            continue;
                        }
                        if resp.middle_clicked() {
                            actions.push(Action::OpenInNewTab(item.path.clone()));
                        }
                        if resp.secondary_clicked() {
                            // Bookmarks can be removed from their context menu.
                            let pos = resp.interact_pointer_pos().unwrap_or(r.center());
                            let mut items = vec![MenuItem::NewTabAt(item.path.clone())];
                            if item.cloud.is_none() {
                                // (A cloud drive's mount folder is an internal path; clicking it opens it.)
                                items.insert(0, MenuItem::Go(item.path.clone()));
                            }
                            if title == "Bookmarks" && item.depth == 0 {
                                items.push(MenuItem::Sep);
                                items.push(MenuItem::Unbookmark(item.path.clone()));
                            }
                            if let Some(a) = &item.cloud {
                                items.push(MenuItem::Sep);
                                items.push(MenuItem::CloudDisconnect(a.name.clone()));
                                items.push(MenuItem::CloudSignOut(a.name.clone()));
                            }
                            actions.push(Action::OpenMenu(pos, items));
                        }
                        if let Some(pl) = resp.dnd_release_payload::<DragPaths>() {
                            if is_trash {
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
        for (section, path, open) in tree_toggles {
            if open {
                self.sidebar_open.insert((section, path));
            } else {
                // Closing a folder closes everything under it too.
                self.sidebar_open.retain(|(s, p)| !(*s == section && p.starts_with(&path)));
            }
        }
        if !new_bookmarks.is_empty() {
            for b in new_bookmarks {
                if !self.cfg.bookmarks.contains(&b) {
                    self.cfg.bookmarks.push(b);
                }
            }
            self.cfg.save();
        }
        if drop_target.is_some() {
            self.drop_target = drop_target;
        }
        self.actions.extend(actions);
    }
}
