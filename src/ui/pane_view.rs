//! One browser pane: toolbar with breadcrumbs, the file view (details, list
//! or grid) and the bottom bar with the filter box and counters.

use std::path::{Path, PathBuf};

use egui::{Align2, Color32, CursorIcon, Id, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::{checkbox, elided, icon_button, search_box, text, wrapped};
use crate::app::{Action, DragPaths, FileFlier, MenuItem, PaneGeom, format_time, human_size};
use crate::commands::Command;
use crate::config::ViewMode;
use crate::counts::Count;
use crate::fs_model::{Entry, SortKey};
use crate::icons::{self, FileKind};

const TOOLBAR_H: f32 = 42.0;
const HEADER_H: f32 = 34.0;
const BOTTOM_H: f32 = 40.0;
const TILE_W: f32 = 132.0;
const TILE_H: f32 = 136.0;
const LIST_COL_W: f32 = 280.0;

enum ItemAction {
    Click(usize, Modifiers),
    Context(usize, egui::Pos2),
    Open(usize),
    NewTab(PathBuf),
    BackgroundContext(egui::Pos2),
    ClearSelection,
}

/// Column layout for the details view.
struct Cols {
    name: f32,
    ty: Option<(f32, f32)>,
    items: Option<(f32, f32)>,
    size: (f32, f32),
    modified: Option<(f32, f32)>,
}

impl Cols {
    fn new(r: Rect, show_items: bool) -> Self {
        let w = r.width();
        let mut right = r.right() - 12.0;
        let modified = (w > 700.0).then(|| {
            right -= 132.0;
            (right, right + 132.0)
        });
        let size = {
            right -= 110.0;
            (right, right + 86.0)
        };
        let items = (show_items && w > 560.0).then(|| {
            right -= 70.0;
            (right, right + 60.0)
        });
        let ty = (w > 440.0).then(|| {
            right -= 150.0;
            (right, right + 140.0)
        });
        Cols { name: right - 12.0, ty, items, size, modified }
    }
}

pub fn file_kind(e: &Entry) -> FileKind {
    FileKind::from_ext(&e.extension())
}

/// File Pilot-style type label: "File folder", "PNG File", "BASHRC File".
pub fn type_label(e: &Entry) -> String {
    if e.is_dir {
        return "File folder".into();
    }
    let ext = e.extension();
    if !ext.is_empty() {
        return format!("{} File", ext.to_uppercase());
    }
    match e.name.strip_prefix('.') {
        Some(rest) if !rest.is_empty() => format!("{} File", rest.to_uppercase()),
        _ => "File".into(),
    }
}

fn short_count(n: usize) -> String {
    match n {
        0 => "--".into(),
        n if n >= 1000 => format!("{:.1}K", n as f32 / 1000.0).replace(".0K", "K"),
        n => n.to_string(),
    }
}

impl FileFlier {
    pub(crate) fn pane_ui(&mut self, ui: &mut Ui, idx: usize, rect: Rect) {
        let pointer_in = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
        if pointer_in && ui.input(|i| i.pointer.any_pressed()) && idx != self.active {
            self.actions.push(Action::Activate(idx));
        }
        let toolbar = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + TOOLBAR_H));
        let bottom = Rect::from_min_max(pos2(rect.left(), rect.bottom() - BOTTOM_H), rect.max);
        let content = Rect::from_min_max(pos2(rect.left(), toolbar.bottom()), pos2(rect.right(), bottom.top()));
        self.toolbar_ui(ui, idx, toolbar);
        self.view_ui(ui, idx, content);
        self.bottom_ui(ui, idx, bottom);
    }

    fn toolbar_ui(&mut self, ui: &mut Ui, idx: usize, rect: Rect) {
        let pal = self.pal();
        let tab = self.panes[idx].tab();
        let (can_back, can_fwd, locked) = (tab.can_back(), tab.can_forward(), tab.locked);
        let path = tab.path.clone();
        let history: Vec<PathBuf> = tab.history().take(15).cloned().collect();
        let bookmarked = self.cfg.bookmarks.contains(&path);
        let cy = rect.center().y;
        let btn = |x: f32| Rect::from_center_size(pos2(x, cy), vec2(30.0, 30.0));
        let mut x = rect.left() + 22.0;
        let mut run = |ui: &mut Ui,
                       x: f32,
                       key: &str,
                       enabled: bool,
                       tip: &str,
                       cmd: Command,
                       draw: &dyn Fn(&egui::Painter, Rect, Color32)| {
            if icon_button(ui, btn(x), Id::new((key, idx)), enabled, pal, tip, draw).clicked() {
                self.actions.push(Action::Activate(idx));
                self.actions.push(Action::Run(cmd));
            }
        };
        run(ui, x, "back", can_back, "Back (Alt+←)", Command::Back, &icons::arrow_left);
        x += 32.0;
        run(ui, x, "fwd", can_fwd, "Forward (Alt+→)", Command::Forward, &icons::arrow_right);
        x += 32.0;
        run(ui, x, "up", path.parent().is_some(), "Parent folder (Backspace)", Command::Up, &icons::arrow_up);
        x += 32.0;
        let hist = icon_button(
            ui,
            btn(x),
            Id::new(("hist", idx)),
            !history.is_empty(),
            pal,
            "Recent locations",
            icons::chevron_down,
        );
        if hist.clicked() {
            self.actions.push(Action::Activate(idx));
            let items = history.into_iter().map(MenuItem::Go).collect();
            self.actions.push(Action::OpenMenu(hist.rect.left_bottom() + vec2(0.0, 4.0), items));
        }
        x += 24.0;
        ui.painter().vline(x, cy - 10.0..=cy + 10.0, Stroke::new(1.0, pal.border));
        x += 22.0;
        let lock_tip = if locked { "Unlock tab" } else { "Lock tab (folders open in new tabs)" };
        if icon_button(ui, btn(x), Id::new(("lock", idx)), true, pal, lock_tip, |p, r, c| {
            icons::lock(p, r, if locked { pal.accent } else { c }, locked)
        })
        .clicked()
        {
            self.actions.push(Action::Activate(idx));
            self.actions.push(Action::Run(Command::LockTab));
        }
        x += 32.0;
        let bm_tip = if bookmarked { "Remove bookmark (Ctrl+D)" } else { "Bookmark this folder (Ctrl+D)" };
        if icon_button(ui, btn(x), Id::new(("bm", idx)), true, pal, bm_tip, |p, r, c| {
            icons::bookmark(p, r, if bookmarked { pal.accent } else { c }, bookmarked)
        })
        .clicked()
        {
            self.actions.push(Action::Activate(idx));
            self.actions.push(Action::Run(Command::ToggleBookmark));
        }
        x += 24.0;
        ui.painter().vline(x, cy - 10.0..=cy + 10.0, Stroke::new(1.0, pal.border));

        // ⋮ menu on the far right.
        let more = btn(rect.right() - 22.0);
        let more_resp = icon_button(ui, more, Id::new(("more", idx)), true, pal, "More", icons::dots_vertical);
        if more_resp.clicked() {
            self.actions.push(Action::Activate(idx));
            let items = [
                Command::Settings,
                Command::ViewDetails,
                Command::ViewList,
                Command::ViewGrid,
                Command::ToggleHidden,
                Command::TogglePreview,
                Command::ToggleSidebar,
                Command::ToggleSplit,
                Command::NewFolder,
                Command::NewFile,
                Command::OpenTerminal,
                Command::ToggleTheme,
                Command::CommandPalette,
                Command::Help,
            ];
            let mut v = Vec::new();
            for (i, c) in items.into_iter().enumerate() {
                if [1, 4, 8, 11].contains(&i) {
                    v.push(MenuItem::Sep);
                }
                v.push(MenuItem::Cmd(c));
            }
            let pos = pos2(more.right() - 300.0, more.bottom() + 4.0);
            self.actions.push(Action::OpenMenu(pos, v));
        }

        // Breadcrumbs.
        let crumbs_rect =
            Rect::from_min_max(pos2(x + 8.0, rect.top() + 6.0), pos2(more.left() - 6.0, rect.bottom() - 6.0));
        self.breadcrumbs(ui, idx, crumbs_rect, &path);
    }

    fn breadcrumbs(&mut self, ui: &mut Ui, idx: usize, rect: Rect, path: &Path) {
        let pal = self.pal();
        // Paths under $HOME start at the home folder, like File Pilot's user-folder crumbs.
        let home = dirs::home_dir();
        let mut crumbs: Vec<(String, PathBuf)> = Vec::new();
        let mut acc = PathBuf::new();
        // Network shares start at the share itself ("media on nas"), not /run/user/…/gvfs.
        let gvfs = crate::mounts::gvfs_root();
        let share = path.strip_prefix(&gvfs).ok().and_then(|rel| rel.components().next()).map(|c| gvfs.join(c));
        let start = home.as_ref().filter(|h| path.starts_with(h) && h.parent().is_some());
        if let Some(share) = share {
            let label = crate::mounts::gvfs_label(&crate::app::display_name(&share));
            crumbs.push((label, share.clone()));
            acc = share.clone();
            for c in path.strip_prefix(&share).unwrap().components() {
                acc.push(c);
                crumbs.push((c.as_os_str().to_string_lossy().into_owned(), acc.clone()));
            }
        } else if let Some(h) = start {
            crumbs.push((crate::app::display_name(h), h.clone()));
            acc = h.clone();
            for c in path.strip_prefix(h).unwrap().components() {
                acc.push(c);
                crumbs.push((c.as_os_str().to_string_lossy().into_owned(), acc.clone()));
            }
        } else {
            for c in path.components() {
                acc.push(c);
                let label = match c {
                    std::path::Component::RootDir => "File System".to_string(),
                    o => o.as_os_str().to_string_lossy().into_owned(),
                };
                crumbs.push((label, acc.clone()));
            }
        }

        let bg = ui.interact(rect, Id::new(("crumbs_bg", idx)), Sense::click());
        if bg.clicked() {
            self.actions.push(Action::Activate(idx));
            self.actions.push(Action::Run(Command::GoToPath));
        }
        bg.on_hover_cursor(CursorIcon::Text);

        let painter = ui.painter().with_clip_rect(rect);
        let widths: Vec<f32> = crumbs
            .iter()
            .map(|(l, _)| painter.layout_no_wrap(l.clone(), super::font(14.0), pal.text).size().x + 12.0)
            .collect();
        let chevron = 18.0;
        let mut first = 0;
        let total = |from: usize| widths[from..].iter().sum::<f32>() + chevron * (widths.len() - from) as f32;
        while first + 1 < crumbs.len() && total(first) > rect.width() {
            first += 1;
        }
        let mut x = rect.left();
        let n = crumbs.len();
        for (i, (label, target)) in crumbs.iter().enumerate().skip(first) {
            // The leading "…" marker only shows when there's room for it.
            let marker = first > 0 && total(first) <= rect.width();
            if i > first || marker {
                let c = Rect::from_center_size(pos2(x + chevron / 2.0 - 2.0, rect.center().y), vec2(12.0, 12.0));
                if i == first {
                    text(&painter, c.center(), Align2::CENTER_CENTER, "…", 14.0, pal.text_dim);
                } else {
                    icons::chevron_right(&painter, c, pal.text_dim);
                }
                x += chevron;
            }
            let last = i + 1 == n;
            let w = widths[i];
            let r = Rect::from_min_max(pos2(x, rect.top() + 2.0), pos2(x + w, rect.bottom() - 2.0));
            let resp = ui.interact(r.intersect(rect), Id::new(("crumb", idx, i)), Sense::click());
            let drop_hover = resp.dnd_hover_payload::<DragPaths>().is_some();
            if resp.hovered() || drop_hover {
                painter.rect_filled(r, 4.0, pal.tab_hover);
            }
            let color = if last || resp.hovered() { pal.text_strong } else { pal.text_dim };
            let g = elided(&painter, label, 14.0, color, (rect.right() - r.left() - 12.0).min(w));
            painter.galley(pos2(r.left() + 6.0, r.center().y - g.size().y / 2.0), g, color);
            if resp.clicked() {
                self.actions.push(Action::Activate(idx));
                self.actions.push(Action::Navigate(target.clone()));
            }
            if resp.middle_clicked() {
                self.actions.push(Action::Activate(idx));
                self.actions.push(Action::OpenInNewTab(target.clone()));
            }
            if let Some(p) = resp.dnd_release_payload::<DragPaths>() {
                let force_copy = ui.input(|i| i.modifiers.command);
                self.actions.push(Action::Drop { paths: p.0.clone(), dest: target.clone(), force_copy });
            }
            x += w;
        }
    }

    fn view_ui(&mut self, ui: &mut Ui, idx: usize, rect: Rect) {
        let pal = self.pal();
        let is_active = idx == self.active;
        let sort = self.cfg.sort;
        let row_h = self.cfg.density.row_height();
        let date_style = self.cfg.date_style;
        let ctx = ui.ctx().clone();
        let cut_paths: Vec<PathBuf> =
            self.clipboard.as_ref().filter(|c| c.cut).map(|c| c.paths.clone()).unwrap_or_default();
        let tab = self.panes[idx].tab_mut();
        let counts = &mut self.item_counts;
        let view = tab.view;
        let n = tab.visible.len();
        let multi = tab.selected.len() > 1;

        // ---- column header (details only)
        let mut list_rect = rect;
        let cols = Cols::new(rect, self.cfg.show_item_counts);
        let mut header_actions: Vec<Action> = Vec::new();
        if view == ViewMode::Details {
            let h = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + HEADER_H));
            list_rect.min.y = h.bottom();
            let p = ui.painter();
            let cy = h.center().y;
            // Select-all checkbox.
            let cb = Rect::from_center_size(pos2(h.left() + 26.0, cy), vec2(20.0, 20.0));
            let all = n > 0 && tab.selected.len() == n;
            checkbox(p, cb, all, pal, false);
            if ui.interact(cb, Id::new(("selall", idx)), Sense::click()).clicked() {
                if all {
                    tab.selected.clear();
                } else {
                    tab.select_all();
                }
            }
            let mut header =
                |ui: &mut Ui, key: Option<SortKey>, label: &str, l: f32, r: f32, right_align: bool, sep: bool| {
                    let hr = Rect::from_min_max(pos2(l, h.top()), pos2(r, h.bottom()));
                    let resp = ui.interact(hr, Id::new(("hdr", idx, label)), Sense::click());
                    let p = ui.painter();
                    let color = if resp.hovered() { pal.text_strong } else { pal.text };
                    if sep {
                        p.vline(l - 8.0, cy - 9.0..=cy + 9.0, Stroke::new(1.0, pal.border));
                    }
                    let label_rect = if right_align {
                        text(p, pos2(r, cy), Align2::RIGHT_CENTER, label, 14.0, color)
                    } else {
                        text(p, pos2(l, cy), Align2::LEFT_CENTER, label, 14.0, color)
                    };
                    if key == Some(sort.key) {
                        let ax = if key == Some(SortKey::Name) {
                            r - 10.0
                        } else if right_align {
                            label_rect.left() - 12.0
                        } else {
                            label_rect.right() + 10.0
                        };
                        icons::sort_arrow(
                            p,
                            Rect::from_center_size(pos2(ax, cy), vec2(13.0, 13.0)),
                            pal.text,
                            sort.descending,
                        );
                    }
                    if resp.clicked()
                        && let Some(key) = key
                    {
                        header_actions.push(Action::Activate(idx));
                        header_actions.push(Action::Run(match key {
                            SortKey::Name => Command::SortByName,
                            SortKey::Size => Command::SortBySize,
                            SortKey::Modified => Command::SortByModified,
                            SortKey::Kind => Command::SortByKind,
                        }));
                    }
                };
            header(ui, Some(SortKey::Name), "Name", h.left() + 48.0, cols.name, false, false);
            if let Some((l, r)) = cols.ty {
                header(ui, Some(SortKey::Kind), "Type", l, r, false, true);
            }
            if let Some((l, r)) = cols.items {
                header(ui, None, "Items", l, r, false, true);
            }
            header(ui, Some(SortKey::Size), "Size", cols.size.0, cols.size.1, true, true);
            if let Some((l, r)) = cols.modified {
                header(ui, Some(SortKey::Modified), "Modified", l, r, false, true);
            }
        }
        self.actions.extend(header_actions);
        let tab = self.panes[idx].tab_mut();

        // ---- geometry
        let inner_w = list_rect.width() - 16.0;
        let (ncols, pitch) = match view {
            ViewMode::Details => (1, row_h),
            ViewMode::List => (((inner_w / LIST_COL_W).floor() as usize).max(1), row_h),
            ViewMode::Grid => (((inner_w / TILE_W).floor() as usize).max(1), TILE_H),
        };
        let rows = n.div_ceil(ncols);

        // Keep the cursor in view.
        let key = Id::new(("scroll", idx));
        let (off, view_h) = ctx.data(|d| d.get_temp::<(f32, f32)>(key).unwrap_or((0.0, 400.0)));
        let mut area = egui::ScrollArea::vertical().id_salt(("list", idx)).auto_shrink(false);
        if tab.scroll_to_cursor {
            tab.scroll_to_cursor = false;
            let y = (tab.cursor / ncols) as f32 * pitch;
            if y < off {
                area = area.vertical_scroll_offset(y);
            } else if y + pitch > off + view_h {
                area = area.vertical_scroll_offset(y + pitch - view_h);
            }
        }

        let mut item_actions: Vec<ItemAction> = Vec::new();
        let mut drops: Vec<(Vec<PathBuf>, PathBuf)> = Vec::new();
        let force_copy = ui.input(|i| i.modifiers.command);
        let sel_fill = if is_active { pal.accent } else { pal.accent_dim };

        // Background (empty space) interaction sits behind the items, which win where they overlap.
        let bg = ui.interact(list_rect, Id::new(("list_bg", idx)), Sense::click());
        if bg.clicked() {
            item_actions.push(ItemAction::ClearSelection);
        }
        if bg.secondary_clicked() {
            item_actions.push(ItemAction::BackgroundContext(bg.interact_pointer_pos().unwrap_or(list_rect.center())));
        }
        let out = ui
            .scope_builder(UiBuilder::new().max_rect(list_rect).id_salt(("view", idx)), |ui| {
                ui.set_clip_rect(list_rect);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                area.show_rows(ui, pitch, rows, |ui, range| {
                    for row in range {
                        let (row_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), pitch), Sense::hover());
                        for c in 0..ncols {
                            let vi = row * ncols + c;
                            let Some(&ei) = tab.visible.get(vi) else { break };
                            let e = &tab.entries[ei];
                            let item = match view {
                                ViewMode::Details => row_rect.shrink2(vec2(8.0, 0.0)),
                                ViewMode::List => {
                                    let w = inner_w / ncols as f32;
                                    Rect::from_min_size(
                                        pos2(row_rect.left() + 8.0 + c as f32 * w, row_rect.top()),
                                        vec2(w - 6.0, pitch),
                                    )
                                }
                                ViewMode::Grid => {
                                    let w = inner_w / ncols as f32;
                                    Rect::from_min_size(
                                        pos2(row_rect.left() + 8.0 + c as f32 * w, row_rect.top()),
                                        vec2(w, pitch),
                                    )
                                    .shrink2(vec2(4.0, 4.0))
                                }
                            };
                            let resp = ui.interact(item, Id::new(("item", idx, vi)), Sense::click_and_drag());
                            let selected = tab.selected.contains(&e.path);
                            let is_cursor = is_active && vi == tab.cursor;
                            let drop_target = e.is_dir && resp.dnd_hover_payload::<DragPaths>().is_some();
                            let dim = e.is_hidden() || cut_paths.contains(&e.path);
                            let p = ui.painter();

                            // Background.
                            let bg_rect = if view == ViewMode::Details { item.shrink2(vec2(0.0, 1.0)) } else { item };
                            if view == ViewMode::Grid {
                                if selected {
                                    p.rect_filled(bg_rect, 5.0, pal.grid_sel);
                                    p.rect_stroke(bg_rect, 5.0, Stroke::new(1.0, pal.accent), StrokeKind::Inside);
                                } else if resp.hovered() {
                                    p.rect_filled(bg_rect, 5.0, pal.hover);
                                }
                            } else if selected {
                                p.rect_filled(bg_rect, 4.0, sel_fill);
                            } else if resp.hovered() {
                                p.rect_filled(bg_rect, 4.0, pal.hover);
                            }
                            if is_cursor && !(selected && view != ViewMode::Grid) {
                                p.rect_stroke(bg_rect, 4.0, Stroke::new(1.0, pal.accent), StrokeKind::Inside);
                            }
                            if drop_target {
                                p.rect_stroke(bg_rect, 4.0, Stroke::new(2.0, pal.accent), StrokeKind::Inside);
                            }
                            if view != ViewMode::Grid && !selected {
                                p.hline(item.x_range(), item.bottom() - 0.5, Stroke::new(1.0, pal.row_sep));
                            }

                            let on_accent = selected && view != ViewMode::Grid && is_active;
                            let mut color = if on_accent { pal.on_accent } else { pal.text };
                            let mut dim_color =
                                if on_accent { pal.on_accent.gamma_multiply(0.8) } else { pal.text_dim };
                            if dim {
                                color = color.gamma_multiply(0.55);
                                dim_color = dim_color.gamma_multiply(0.6);
                            }

                            match view {
                                ViewMode::Details | ViewMode::List => {
                                    let icon = Rect::from_center_size(
                                        pos2(item.left() + 18.0, item.center().y),
                                        vec2(20.0, 20.0),
                                    );
                                    if multi && selected {
                                        checkbox(p, icon, true, pal, on_accent);
                                    } else if e.is_dir {
                                        icons::folder(p, icon, pal);
                                    } else {
                                        icons::file(p, icon, file_kind(e), pal);
                                    }
                                    let name_right =
                                        if view == ViewMode::Details { cols.name - 8.0 } else { item.right() - 6.0 };
                                    let label = if e.is_symlink { format!("{} ↗", e.name) } else { e.name.clone() };
                                    let g = elided(p, &label, 14.0, color, name_right - (item.left() + 40.0));
                                    p.galley(pos2(item.left() + 40.0, item.center().y - g.size().y / 2.0), g, color);
                                    if view == ViewMode::Details {
                                        let cy = item.center().y;
                                        if let Some((l, r)) = cols.ty {
                                            let g = elided(p, &type_label(e), 13.0, dim_color, r - l);
                                            p.galley(pos2(l, cy - g.size().y / 2.0), g, dim_color);
                                        }
                                        if let Some((l, _)) = cols.items {
                                            let s = match e.is_dir.then(|| counts.get(&e.path)) {
                                                Some(Count::Items(n)) => short_count(n),
                                                Some(Count::Pending) => "…".into(),
                                                _ => "--".into(),
                                            };
                                            text(p, pos2(l, cy), Align2::LEFT_CENTER, s, 13.0, dim_color);
                                        }
                                        let size = if e.is_dir { "--".to_string() } else { human_size(e.size) };
                                        text(p, pos2(cols.size.1, cy), Align2::RIGHT_CENTER, size, 13.0, dim_color);
                                        if let Some((l, r)) = cols.modified {
                                            let d = e.modified.map(|m| format_time(m, date_style)).unwrap_or_default();
                                            let g = elided(p, &d, 13.0, dim_color, r - l);
                                            p.galley(pos2(l, cy - g.size().y / 2.0), g, dim_color);
                                        }
                                    }
                                }
                                ViewMode::Grid => {
                                    let thumb = Rect::from_min_size(
                                        item.min + vec2(10.0, 8.0),
                                        vec2(item.width() - 20.0, 78.0),
                                    );
                                    let mut drew = false;
                                    if e.is_file && file_kind(e) == FileKind::Image && e.size < 25_000_000 {
                                        let img = egui::Image::new(format!("file://{}", e.path.display()));
                                        if let Ok(poll) = img.load_for_size(ui.ctx(), thumb.size())
                                            && let Some(sz) = poll.size()
                                        {
                                            let scale = (thumb.width() / sz.x).min(thumb.height() / sz.y).min(1.0);
                                            let fit = Rect::from_center_size(thumb.center(), sz * scale);
                                            img.corner_radius(3).paint_at(ui, fit);
                                            drew = true;
                                        }
                                    }
                                    let p = ui.painter();
                                    if !drew {
                                        let icon = Rect::from_center_size(thumb.center(), vec2(64.0, 64.0));
                                        if e.is_dir {
                                            icons::folder(p, icon, pal);
                                        } else {
                                            icons::file(p, icon, file_kind(e), pal);
                                        }
                                    }
                                    let g = wrapped(p, &e.name, 13.0, color, item.width() - 8.0, 2);
                                    let ty = thumb.bottom() + 6.0;
                                    // Center-aligned galleys are laid out around x = 0.
                                    p.galley(pos2(item.center().x, ty), g.clone(), color);
                                    let meta = if e.is_dir { String::new() } else { human_size(e.size) };
                                    text(
                                        p,
                                        pos2(item.center().x, ty + g.size().y + 2.0),
                                        Align2::CENTER_TOP,
                                        meta,
                                        12.0,
                                        pal.text_dim,
                                    );
                                }
                            }

                            // Interaction.
                            let mods = ui.input(|i| i.modifiers);
                            if resp.clicked() {
                                item_actions.push(ItemAction::Click(vi, mods));
                            }
                            if resp.secondary_clicked() {
                                let pos = resp.interact_pointer_pos().unwrap_or(item.center());
                                item_actions.push(ItemAction::Context(vi, pos));
                            }
                            if resp.double_clicked() {
                                item_actions.push(ItemAction::Open(vi));
                            }
                            if resp.middle_clicked() && e.is_dir {
                                item_actions.push(ItemAction::NewTab(e.path.clone()));
                            }
                            if resp.drag_started() {
                                let paths = if selected { tab.targets_selected() } else { vec![e.path.clone()] };
                                resp.dnd_set_drag_payload(DragPaths(paths));
                            }
                            if e.is_dir
                                && let Some(pl) = resp.dnd_release_payload::<DragPaths>()
                            {
                                drops.push((pl.0.clone(), e.path.clone()));
                            }
                        }
                    }
                })
            })
            .inner;
        ctx.data_mut(|d| d.insert_temp(key, (out.state.offset.y, out.inner_rect.height())));
        let content_h = rows as f32 * pitch;
        let scroll_pct = if content_h <= out.inner_rect.height() {
            0.0
        } else {
            (out.state.offset.y / (content_h - out.inner_rect.height())).clamp(0.0, 1.0)
        };
        ctx.data_mut(|d| d.insert_temp(Id::new(("scroll_pct", idx)), scroll_pct));
        self.geom[idx] =
            PaneGeom { cols: ncols, page_rows: (out.inner_rect.height() / pitch).floor().max(1.0) as usize };

        // Dropping onto the pane itself copies/moves into its folder.
        let tab = self.panes[idx].tab_mut();
        let released_here = ctx.input(|i| i.pointer.any_released())
            && ctx.pointer_interact_pos().is_some_and(|p| list_rect.contains(p));
        if released_here && let Some(pl) = egui::DragAndDrop::take_payload::<DragPaths>(&ctx) {
            drops.push((pl.0.clone(), tab.path.clone()));
        }
        for (paths, dest) in drops {
            self.actions.push(Action::Drop { paths, dest, force_copy });
        }

        if n == 0 {
            let msg = match (&tab.error, tab.filter.is_empty()) {
                (Some(e), _) => format!("⚠ {e}"),
                (None, true) => "This folder is empty".into(),
                (None, false) => "No items match the filter".into(),
            };
            text(ui.painter(), list_rect.center(), Align2::CENTER_CENTER, msg, 14.0, pal.text_dim);
        }

        for a in item_actions {
            let tab = self.panes[idx].tab_mut();
            match a {
                ItemAction::Click(vi, mods) => {
                    if mods.command {
                        tab.toggle_select(vi);
                    } else if mods.shift {
                        tab.move_cursor(vi, true);
                    } else {
                        tab.move_cursor(vi, false);
                    }
                    tab.scroll_to_cursor = false;
                }
                ItemAction::Context(vi, pos) => {
                    let is_dir = tab.entry_at(vi).is_some_and(|e| e.is_dir);
                    let already = tab.entry_at(vi).is_some_and(|e| tab.selected.contains(&e.path));
                    if already {
                        tab.cursor = vi;
                    } else {
                        tab.move_cursor(vi, false);
                    }
                    tab.scroll_to_cursor = false;
                    self.actions.push(Action::Activate(idx));
                    self.actions.push(Action::OpenMenu(pos, entry_menu(is_dir)));
                }
                ItemAction::Open(vi) => {
                    tab.move_cursor(vi, false);
                    tab.scroll_to_cursor = false;
                    self.actions.push(Action::Activate(idx));
                    self.actions.push(Action::OpenEntries);
                }
                ItemAction::NewTab(p) => {
                    self.actions.push(Action::Activate(idx));
                    self.actions.push(Action::OpenInNewTab(p));
                }
                ItemAction::BackgroundContext(pos) => {
                    tab.selected.clear();
                    self.actions.push(Action::Activate(idx));
                    self.actions.push(Action::OpenMenu(pos, background_menu()));
                }
                ItemAction::ClearSelection => tab.selected.clear(),
            }
        }
    }

    fn bottom_ui(&mut self, ui: &mut Ui, idx: usize, rect: Rect) {
        let pal = self.pal();
        let show_hidden = self.cfg.show_hidden;
        ui.painter().rect_filled(rect, 0.0, pal.bg);
        ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, pal.row_sep));
        let tab = self.panes[idx].tab_mut();

        // Filter box.
        let fw = (rect.width() * 0.42).clamp(140.0, 290.0);
        let fr = Rect::from_min_size(pos2(rect.left() + 8.0, rect.top() + 6.0), vec2(fw, rect.height() - 12.0));
        let (resp, opts) = search_box(ui, fr, pal, &mut tab.filter, Id::new(("filter", idx)), "Filter...", true);
        if resp.changed() {
            tab.refilter(show_hidden);
            tab.cursor = 0;
            tab.anchor = 0;
            tab.selected.clear();
        }
        if resp.gained_focus() {
            self.actions.push(Action::Activate(idx));
        }
        if let Some(o) = opts
            && o.clicked()
        {
            self.actions.push(Action::Activate(idx));
            let items = vec![
                MenuItem::Cmd(Command::ToggleHidden),
                MenuItem::Sep,
                MenuItem::Cmd(Command::SortByName),
                MenuItem::Cmd(Command::SortByKind),
                MenuItem::Cmd(Command::SortBySize),
                MenuItem::Cmd(Command::SortByModified),
                MenuItem::Cmd(Command::ReverseSort),
            ];
            self.actions.push(Action::OpenMenu(o.rect.left_top() - vec2(0.0, 250.0), items));
        }

        // Counters: folders shown/total, files shown/total, view toggle, scroll %.
        let tab = self.panes[idx].tab();
        let (mut dirs_shown, mut files_shown) = (0, 0);
        for &i in &tab.visible {
            if tab.entries[i].is_dir { dirs_shown += 1 } else { files_shown += 1 }
        }
        let dirs_total = tab.entries.iter().filter(|e| e.is_dir).count();
        let files_total = tab.entries.len() - dirs_total;
        let pct: f32 = ui.ctx().data(|d| d.get_temp(Id::new(("scroll_pct", idx))).unwrap_or(0.0));
        let view = tab.view;

        let p = ui.painter();
        let cy = rect.center().y;
        let pct_r = Rect::from_min_max(
            pos2(rect.right() - 54.0, rect.top() + 7.0),
            pos2(rect.right() - 8.0, rect.bottom() - 7.0),
        );
        p.rect_filled(pct_r, 4.0, pal.input);
        let fill_w = (pct_r.width() * pct).max(4.0);
        p.rect_filled(Rect::from_min_size(pct_r.min, vec2(fill_w, pct_r.height())), 4.0, pal.accent);
        p.rect_stroke(pct_r, 4.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        text(p, pct_r.center(), Align2::CENTER_CENTER, format!("{}%", (pct * 100.0).round() as i32), 12.5, pal.text);

        let d = format!("{dirs_shown}/{dirs_total}");
        let f = format!("{files_shown}/{files_total}");
        let dw = p.layout_no_wrap(d.clone(), super::font(13.5), pal.text).size().x;
        let fw2 = p.layout_no_wrap(f.clone(), super::font(13.5), pal.text).size().x;
        let group_w = 12.0 + 20.0 + dw + 14.0 + 20.0 + fw2 + 12.0 + 26.0 + 6.0;
        let g = Rect::from_min_max(
            pos2(pct_r.left() - 6.0 - group_w, pct_r.top()),
            pos2(pct_r.left() - 6.0, pct_r.bottom()),
        );
        if g.left() > fr.right() + 8.0 {
            p.rect_stroke(g, 4.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
            let mut x = g.left() + 10.0;
            icons::folder_outline(p, Rect::from_center_size(pos2(x + 9.0, cy), vec2(17.0, 17.0)), pal.text);
            x += 22.0;
            text(p, pos2(x, cy), Align2::LEFT_CENTER, &d, 13.5, pal.text);
            x += dw + 12.0;
            icons::file_outline(p, Rect::from_center_size(pos2(x + 9.0, cy), vec2(18.0, 18.0)), pal.text);
            x += 22.0;
            text(p, pos2(x, cy), Align2::LEFT_CENTER, &f, 13.5, pal.text);
            x += fw2 + 8.0;
            let lr = Rect::from_min_size(pos2(x, g.top() + 1.0), vec2(26.0, g.height() - 2.0));
            let tip = match view {
                ViewMode::Details => "View: details (click to switch)",
                ViewMode::List => "View: list (click to switch)",
                ViewMode::Grid => "View: grid (click to switch)",
            };
            if icon_button(ui, lr, Id::new(("viewtoggle", idx)), true, pal, tip, icons::layout_list).clicked() {
                self.actions.push(Action::Activate(idx));
                self.actions.push(Action::Run(match view {
                    ViewMode::Details => Command::ViewList,
                    ViewMode::List => Command::ViewGrid,
                    ViewMode::Grid => Command::ViewDetails,
                }));
            }
        }
    }
}

pub fn entry_menu(is_dir: bool) -> Vec<MenuItem> {
    use Command::*;
    let mut v = vec![MenuItem::Cmd(Open)];
    if is_dir {
        v.push(MenuItem::Cmd(OpenInNewTab));
    }
    v.extend([
        MenuItem::Cmd(OpenTerminal),
        MenuItem::Cmd(CopyPath),
        MenuItem::Sep,
        MenuItem::Cmd(Cut),
        MenuItem::Cmd(Copy),
        MenuItem::Cmd(Paste),
        MenuItem::Cmd(CopyToOtherPane),
        MenuItem::Cmd(MoveToOtherPane),
        MenuItem::Sep,
        MenuItem::Cmd(Trash),
        MenuItem::Cmd(DeletePermanently),
        MenuItem::Cmd(Rename),
        MenuItem::Sep,
        MenuItem::Cmd(TogglePreview),
    ]);
    v
}

pub fn background_menu() -> Vec<MenuItem> {
    use Command::*;
    vec![
        MenuItem::Cmd(Paste),
        MenuItem::Cmd(NewFolder),
        MenuItem::Cmd(NewFile),
        MenuItem::Sep,
        MenuItem::Cmd(OpenTerminal),
        MenuItem::Cmd(ToggleBookmark),
        MenuItem::Cmd(SelectAll),
        MenuItem::Sep,
        MenuItem::Cmd(ViewDetails),
        MenuItem::Cmd(ViewList),
        MenuItem::Cmd(ViewGrid),
        MenuItem::Cmd(ToggleHidden),
        MenuItem::Cmd(Refresh),
    ]
}
