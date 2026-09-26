//! Popup menus and dialogs: searchable command menus, palette, GoTo, search,
//! rename/create/delete and the shortcut cheat sheet.

use std::path::PathBuf;
use std::time::Duration;

use egui::{Align, Align2, Color32, Id, Key, Modifiers, Order, RichText, Sense, Stroke, TextEdit, Ui, pos2, vec2};

use super::{RowIcon, badge, font, menu_row, popup_frame, search_box, text};
use crate::app::{Action, Dialog, FileFlier, MenuItem, PaletteItem, display_name, step};
use crate::commands::{self, Command};
use crate::icons::FileKind;
use crate::search::{self, Search};
use crate::{fuzzy, ops};

/// Up / Down / Enter / Tab for popup lists.
fn list_keys(ctx: &egui::Context) -> (bool, bool, bool, bool) {
    ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.key_pressed(Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Tab),
        )
    })
}

fn tilde(p: &std::path::Path) -> String {
    if let Some(h) = dirs::home_dir()
        && let Ok(rest) = p.strip_prefix(&h)
    {
        return if rest.as_os_str().is_empty() { "~".into() } else { format!("~/{}", rest.display()) };
    }
    p.display().to_string()
}

fn modal<R>(
    ctx: &egui::Context,
    id: &str,
    width: f32,
    pal: &crate::theme::Palette,
    fx: &super::Fx,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let id = Id::new(("dialog", id));
    let area = egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, vec2(0.0, 72.0));
    egui::Modal::new(id)
        .area(area)
        .frame(popup_frame(pal, fx))
        .backdrop_color(Color32::from_black_alpha(70))
        .show(ctx, |ui| {
            super::animate_in(ui, id.with("appear"), fx, 12.0);
            ui.set_width(width.min(ctx.content_rect().width() - 40.0));
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
            add(ui)
        })
        .inner
}

fn input_row(ui: &mut Ui, pal: &crate::theme::Palette, value: &mut String, id: Id, hint: &str) -> egui::Response {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    let (resp, _) = search_box(ui, r, pal, value, id, hint, false);
    ui.add_space(4.0);
    resp
}

fn plain_input(ui: &mut Ui, pal: &crate::theme::Palette, value: &mut String, id: Id) -> egui::Response {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 5.0, pal.input);
    p.rect_stroke(r, 5.0, Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
    let inner = r.shrink2(vec2(10.0, 1.0));
    ui.put(
        inner,
        TextEdit::singleline(value)
            .id(id)
            .frame(egui::Frame::NONE)
            .margin(vec2(0.0, 0.0))
            .vertical_align(Align::Center)
            .desired_width(inner.width())
            .font(font(14.0))
            .text_color(pal.text),
    )
}

fn button(ui: &mut Ui, pal: &crate::theme::Palette, label: &str, primary: bool, danger: bool) -> egui::Response {
    let g = ui.painter().layout_no_wrap(label.to_string(), font(14.0), pal.text);
    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 32.0, 30.0), Sense::click());
    let fill = match (primary, danger, resp.hovered()) {
        (_, true, false) => Color32::from_rgb(180, 50, 45),
        (_, true, true) => Color32::from_rgb(205, 60, 52),
        (true, _, false) => pal.accent,
        (true, _, true) => pal.accent.gamma_multiply(1.2),
        (false, _, false) => pal.input,
        (false, _, true) => pal.tab_hover,
    };
    ui.painter().rect_filled(r, 5.0, fill);
    if !primary && !danger {
        ui.painter().rect_stroke(r, 5.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    }
    let color = if danger {
        Color32::WHITE
    } else if primary {
        pal.on_accent
    } else {
        pal.text
    };
    text(ui.painter(), r.center(), Align2::CENTER_CENTER, label, 14.0, color);
    resp
}

fn label(ui: &mut Ui, pal: &crate::theme::Palette, s: &str, size: f32, color: Option<Color32>) {
    ui.add_space(4.0);
    ui.label(RichText::new(s).size(size).color(color.unwrap_or(pal.text)));
    ui.add_space(4.0);
}

impl FileFlier {
    /// The shared searchable menu used for right-click, ⋮, history and filter options.
    pub(crate) fn menu_ui(&mut self, ctx: &egui::Context) {
        let pal = self.pal();
        let fx = self.fx();
        let Some(menu) = self.menu.as_mut() else { return };
        let esc = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let (up, down, enter, _) = list_keys(ctx);

        let q = menu.query.trim().to_string();
        let entries: Vec<(usize, &MenuItem)> = menu
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| match it {
                MenuItem::Sep => q.is_empty(),
                MenuItem::Cmd(c) => fuzzy::score(&q, c.label()).is_some(),
                MenuItem::Go(p) => fuzzy::score(&q, &p.to_string_lossy()).is_some(),
                MenuItem::NewTabAt(_) => fuzzy::score(&q, "Open in new tab").is_some(),
                MenuItem::Unbookmark(_) => fuzzy::score(&q, "Remove bookmark").is_some(),
            })
            .collect();
        let selectable: Vec<usize> =
            entries.iter().filter(|(_, it)| !matches!(it, MenuItem::Sep)).map(|(i, _)| *i).collect();
        menu.cursor = step(menu.cursor, up, down, selectable.len());

        let width = 320.0;
        let est_h =
            52.0 + entries.iter().map(|(_, it)| if matches!(it, MenuItem::Sep) { 9.0 } else { 30.0 }).sum::<f32>();
        let screen = ctx.content_rect();
        let pos = pos2(
            menu.pos.x.clamp(screen.left() + 4.0, (screen.right() - width - 16.0).max(screen.left())),
            menu.pos.y.clamp(screen.top() + 4.0, (screen.bottom() - est_h - 16.0).max(screen.top())),
        );
        let mut chosen: Option<MenuItem> = None;
        let mut query = std::mem::take(&mut menu.query);
        let area = egui::Area::new(Id::new("cmd_menu")).order(Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            super::animate_in(ui, Id::new("cmd_menu_appear"), &fx, -6.0);
            popup_frame(pal, &fx).show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let (r, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
                let (resp, _) =
                    search_box(ui, r, pal, &mut query, Id::new("menu_search"), "Select a command...", false);
                resp.request_focus();
                ui.add_space(6.0);
                let mut sel_i = 0;
                for (i, item) in &entries {
                    if let MenuItem::Sep = item {
                        let (r, _) = ui.allocate_exact_size(vec2(width, 9.0), Sense::hover());
                        ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, pal.border));
                        continue;
                    }
                    let selected = selectable.get(menu.cursor) == Some(i);
                    let (label, badges, icon) = match item {
                        MenuItem::Cmd(c) => (c.label().to_string(), c.shortcut_texts(ctx), RowIcon::None),
                        MenuItem::Go(p) => (tilde(p), vec![], RowIcon::Folder),
                        MenuItem::NewTabAt(_) => ("Open in new tab".into(), vec![], RowIcon::None),
                        MenuItem::Unbookmark(_) => ("Remove bookmark".into(), vec![], RowIcon::None),
                        MenuItem::Sep => unreachable!(),
                    };
                    let resp = menu_row(ui, pal, selected, icon, &label, &badges);
                    if resp.hovered() && ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                        menu.cursor = sel_i;
                    }
                    if resp.clicked() {
                        chosen = Some((*item).clone());
                    }
                    sel_i += 1;
                }
                if entries.is_empty() {
                    text(
                        ui.painter(),
                        ui.cursor().min + vec2(10.0, 14.0),
                        Align2::LEFT_CENTER,
                        "No matching commands",
                        13.5,
                        pal.text_dim,
                    );
                    ui.add_space(28.0);
                }
            });
        });
        if query != menu.query && !query.is_empty() {
            menu.cursor = 0;
        }
        menu.query = query;
        if enter && chosen.is_none() {
            chosen = selectable.get(menu.cursor).map(|&i| menu.items[i].clone());
        }
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
            && ctx.pointer_interact_pos().is_some_and(|p| !area.response.rect.contains(p))
            && menu.opened.elapsed() > Duration::from_millis(150);
        if chosen.is_some() || esc || clicked_outside {
            self.menu = None;
        }
        match chosen {
            Some(MenuItem::Cmd(c)) => self.actions.push(Action::Run(c)),
            Some(MenuItem::Go(p)) => self.actions.push(Action::Navigate(p)),
            Some(MenuItem::NewTabAt(p)) => self.actions.push(Action::OpenInNewTab(p)),
            Some(MenuItem::Unbookmark(p)) => self.toggle_bookmark(p),
            _ => {}
        }
    }

    pub(crate) fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let pal = self.pal();
        let fx = self.fx();
        let mut keep = !ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let mut run_after: Option<Command> = None;
        let mut go_after: Option<(PathBuf, Option<PathBuf>)> = None;
        let mut uri_after: Option<String> = None;

        match &mut dialog {
            Dialog::Palette { query, cursor } => {
                let items = self.palette_items(query);
                let (up, down, enter, _) = list_keys(ctx);
                *cursor = step(*cursor, up, down, items.len());
                let clicked = modal(ctx, "palette", 620.0, pal, &fx, |ui| {
                    let r = input_row(ui, pal, query, Id::new("palette_input"), "Search commands and folders...");
                    r.request_focus();
                    if r.changed() {
                        *cursor = 0;
                    }
                    let mut chosen = None;
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        for (i, item) in items.iter().enumerate().take(200) {
                            let (label, badges, icon) = match item {
                                PaletteItem::Cmd(c) => (c.label().to_string(), c.shortcut_texts(ctx), RowIcon::None),
                                PaletteItem::Go(p) => (tilde(p), vec![], RowIcon::Folder),
                            };
                            let r = menu_row(ui, pal, i == *cursor, icon, &label, &badges);
                            if i == *cursor && (up || down) {
                                r.scroll_to_me(None);
                            }
                            if r.clicked() {
                                chosen = Some(i);
                            }
                        }
                    });
                    chosen
                });
                if let Some(item) = clicked.or(enter.then_some(*cursor)).and_then(|i| items.into_iter().nth(i)) {
                    keep = false;
                    match item {
                        PaletteItem::Cmd(c) => run_after = Some(c),
                        PaletteItem::Go(p) => go_after = Some((p, None)),
                    }
                }
            }
            Dialog::GoTo { text: path_text, cursor } => {
                let items = self.goto_items(path_text);
                let (up, down, enter, tab) = list_keys(ctx);
                *cursor = step(*cursor, up, down, items.len());
                if tab && let Some(p) = items.get(*cursor) {
                    *path_text = format!("{}/", p.display());
                    *cursor = 0;
                    let id = Id::new("goto_input");
                    if let Some(mut st) = TextEdit::load_state(ctx, id) {
                        let end = egui::text::CCursor::new(path_text.chars().count());
                        st.cursor.set_char_range(Some(egui::text_selection::CCursorRange::one(end)));
                        st.store(ctx, id);
                    }
                }
                let clicked = modal(ctx, "goto", 620.0, pal, &fx, |ui| {
                    let r = input_row(
                        ui,
                        pal,
                        path_text,
                        Id::new("goto_input"),
                        "Go to folder, or smb:// / sftp:// address...",
                    );
                    r.request_focus();
                    if r.changed() {
                        *cursor = 0;
                    }
                    let mut chosen = None;
                    egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                        for (i, p) in items.iter().enumerate() {
                            let r = menu_row(ui, pal, i == *cursor, RowIcon::Folder, &tilde(p), &[]);
                            if i == *cursor && (up || down) {
                                r.scroll_to_me(None);
                            }
                            if r.clicked() {
                                chosen = Some(i);
                            }
                        }
                    });
                    if items.is_empty() {
                        label(ui, pal, "Tab completes · Enter goes", 12.5, Some(pal.text_dim));
                    }
                    chosen
                });
                let typed = crate::app::expand_tilde(path_text.trim());
                if let Some(i) = clicked {
                    keep = false;
                    go_after = Some((items[i].clone(), None));
                } else if enter && crate::mounts::is_remote_uri(path_text.trim()) {
                    keep = false;
                    uri_after = Some(path_text.trim().to_string());
                } else if enter {
                    keep = false;
                    let target = if (*cursor > 0 || !typed.is_dir()) && !items.is_empty() {
                        items[*cursor].clone()
                    } else {
                        typed
                    };
                    go_after = Some((target, None));
                }
            }
            Dialog::Search { query, root, search, cursor } => {
                if let Some(s) = search.as_mut() {
                    s.poll();
                }
                let n = search.as_ref().map_or(0, |s| s.results.len());
                let (up, down, enter, _) = list_keys(ctx);
                *cursor = step(*cursor, up, down, n);
                let mut changed = false;
                let chosen = modal(ctx, "search", 680.0, pal, &fx, |ui| {
                    let hint = format!("Search in {}", display_name(root));
                    let r = input_row(ui, pal, query, Id::new("search_input"), &hint);
                    r.request_focus();
                    changed = r.changed();
                    let status = match search.as_ref() {
                        None => "Fuzzy-search names in all sub-folders · Enter reveals · Ctrl+Enter opens".to_string(),
                        Some(s) if !s.done => format!("Searching… {} results", s.results.len()),
                        Some(s) => {
                            let lim = if s.results.len() >= search::MAX_RESULTS { " (limit reached)" } else { "" };
                            format!("{} results · {} entries scanned{lim}", s.results.len(), s.scanned)
                        }
                    };
                    label(ui, pal, &status, 12.5, Some(pal.text_dim));
                    let mut chosen = None;
                    if let Some(s) = search.as_ref() {
                        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                            for (i, hit) in s.results.iter().enumerate().take(500) {
                                let rel = hit.path.strip_prefix(&*root).unwrap_or(&hit.path);
                                let ext = hit
                                    .path
                                    .extension()
                                    .map(|e| e.to_string_lossy().to_lowercase())
                                    .unwrap_or_default();
                                let icon =
                                    if hit.is_dir { RowIcon::Folder } else { RowIcon::File(FileKind::from_ext(&ext)) };
                                let r = menu_row(ui, pal, i == *cursor, icon, &rel.display().to_string(), &[]);
                                if i == *cursor && (up || down) {
                                    r.scroll_to_me(None);
                                }
                                if r.clicked() {
                                    chosen = Some((i, false));
                                }
                                if r.double_clicked() {
                                    chosen = Some((i, true));
                                }
                            }
                        });
                    }
                    chosen
                });
                if changed {
                    *cursor = 0;
                    *search = (!query.trim().is_empty()).then(|| {
                        Search::start(root.clone(), query.trim().to_string(), self.cfg.show_hidden, ctx.clone())
                    });
                }
                let open_now = ctx.input(|i| i.modifiers.command);
                if let Some((i, open)) = chosen.or(enter.then_some((*cursor, open_now)))
                    && let Some(hit) = search.as_ref().and_then(|s| s.results.get(i))
                {
                    keep = false;
                    if open {
                        if hit.is_dir {
                            go_after = Some((hit.path.clone(), None));
                        } else if let Err(e) = open::that_detached(&hit.path) {
                            self.error(e.to_string());
                        }
                    } else if let Some(parent) = hit.path.parent() {
                        go_after = Some((parent.to_path_buf(), Some(hit.path.clone())));
                    }
                }
            }
            Dialog::Rename { path, name, init, error } => {
                let mut submit = false;
                modal(ctx, "rename", 440.0, pal, &fx, |ui| {
                    label(ui, pal, &format!("Rename “{}”", display_name(path)), 15.0, None);
                    let id = Id::new("rename_edit");
                    let resp = plain_input(ui, pal, name, id);
                    if *init {
                        *init = false;
                        resp.request_focus();
                        // Pre-select the stem so typing replaces it but keeps the extension.
                        let stem_len = match name.rfind('.') {
                            Some(i) if i > 0 => name[..i].chars().count(),
                            _ => name.chars().count(),
                        };
                        let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
                        state.cursor.set_char_range(Some(egui::text_selection::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(stem_len),
                        )));
                        state.store(ui.ctx(), id);
                    }
                    if let Some(e) = error.as_ref() {
                        label(ui, pal, e, 13.0, Some(Color32::from_rgb(240, 120, 110)));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        submit |= button(ui, pal, "Rename", true, false).clicked();
                        if button(ui, pal, "Cancel", false, false).clicked() {
                            keep = false;
                        }
                    });
                    submit |= ui.input(|i| i.key_pressed(Key::Enter));
                });
                if submit {
                    let res = ops::validate_name(name).and_then(|()| {
                        let target = path.with_file_name(name.trim());
                        if target == *path {
                            return Ok(target);
                        }
                        ops::rename_noreplace(path, &target).map(|()| target).map_err(|e| {
                            if e.kind() == std::io::ErrorKind::AlreadyExists {
                                "A file with that name already exists".to_string()
                            } else {
                                e.to_string()
                            }
                        })
                    });
                    match res {
                        Ok(target) => {
                            keep = false;
                            if target != *path {
                                let op = crate::undo::UndoOp::Rename { from: path.clone(), to: target.clone() };
                                self.info_undoable(format!("Renamed to {}", display_name(&target)), op);
                            }
                            self.reload_all();
                            self.tab_mut().select_path(&target);
                        }
                        Err(e) => *error = Some(e),
                    }
                }
            }
            Dialog::Create { dir, name, folder, error } => {
                let mut submit = false;
                modal(ctx, "create", 440.0, pal, &fx, |ui| {
                    label(ui, pal, if *folder { "New folder" } else { "New file" }, 15.0, None);
                    plain_input(ui, pal, name, Id::new("create_edit")).request_focus();
                    if let Some(e) = error.as_ref() {
                        label(ui, pal, e, 13.0, Some(Color32::from_rgb(240, 120, 110)));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        submit |= button(ui, pal, "Create", true, false).clicked();
                        if button(ui, pal, "Cancel", false, false).clicked() {
                            keep = false;
                        }
                    });
                    submit |= ui.input(|i| i.key_pressed(Key::Enter));
                });
                if submit {
                    let target = dir.join(name.trim());
                    let res = ops::validate_name(name).and_then(|()| {
                        if std::fs::symlink_metadata(&target).is_ok() {
                            return Err("Already exists".to_string());
                        }
                        let r = if *folder {
                            std::fs::create_dir(&target)
                        } else {
                            std::fs::OpenOptions::new().write(true).create_new(true).open(&target).map(|_| ())
                        };
                        r.map_err(|e| e.to_string())
                    });
                    match res {
                        Ok(()) => {
                            keep = false;
                            let op = crate::undo::UndoOp::Create { path: target.clone() };
                            self.info_undoable(format!("Created {}", display_name(&target)), op);
                            self.reload_all();
                            self.tab_mut().select_path(&target);
                        }
                        Err(e) => *error = Some(e),
                    }
                }
            }
            Dialog::ConfirmDelete { paths } => {
                let mut confirm = false;
                modal(ctx, "delete", 460.0, pal, &fx, |ui| {
                    let n = paths.len();
                    label(ui, pal, &format!("Permanently delete {n} item{}?", crate::app::plural(n)), 15.0, None);
                    label(
                        ui,
                        pal,
                        "This cannot be undone. Use Delete to move items to the trash instead.",
                        13.0,
                        Some(pal.text_dim),
                    );
                    for p in paths.iter().take(6) {
                        ui.label(RichText::new(format!("  {}", display_name(p))).size(13.0).color(pal.text));
                    }
                    if paths.len() > 6 {
                        ui.label(
                            RichText::new(format!("  …and {} more", paths.len() - 6)).size(13.0).color(pal.text_dim),
                        );
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        confirm |= button(ui, pal, "Delete", false, true).clicked();
                        if button(ui, pal, "Cancel", false, false).clicked() {
                            keep = false;
                        }
                    });
                    confirm |= ui.input(|i| i.key_pressed(Key::Enter));
                });
                if confirm {
                    keep = false;
                    let paths = std::mem::take(paths);
                    self.start_delete(paths);
                }
            }
            Dialog::Settings => {
                if modal(ctx, "settings", 700.0, pal, &fx, |ui| self.settings_ui(ui)) {
                    keep = false;
                }
            }
            Dialog::Help => {
                modal(ctx, "help", 620.0, pal, &fx, |ui| {
                    label(ui, pal, "Keyboard shortcuts", 16.0, Some(pal.text_strong));
                    egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                        let basics = [
                            ("Move cursor (Shift selects)", vec!["↑ ↓ PgUp PgDn Home End".to_string()]),
                            ("Open folder / file", vec!["Enter".into(), "→".into()]),
                            ("Parent folder", vec!["Backspace".into(), "←".into()]),
                            ("Filter this folder", vec!["Type anything".into()]),
                            ("Multi-select", vec!["Ctrl+Click".into(), "Shift+Click".into()]),
                            ("Copy / cut / paste", vec!["Ctrl+C".into(), "Ctrl+X".into(), "Ctrl+V".into()]),
                            ("Drag files (hold Ctrl to copy)", vec!["Drag".into()]),
                        ];
                        let rows = basics.into_iter().chain(
                            commands::ALL
                                .iter()
                                .filter(|c| !matches!(c, Command::Copy | Command::Cut | Command::Paste))
                                .map(|c| (c.label(), c.shortcut_texts(ctx)))
                                .filter(|(_, s)| !s.is_empty()),
                        );
                        for (desc, keys) in rows {
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                            let p = ui.painter();
                            text(p, pos2(r.left() + 8.0, r.center().y), Align2::LEFT_CENTER, desc, 13.5, pal.text);
                            let mut right = r.right() - 8.0;
                            for k in keys.iter().rev() {
                                right -= badge(p, right, r.center().y, k, pal, false) + 6.0;
                            }
                            p.hline(r.x_range(), r.bottom(), Stroke::new(1.0, pal.row_sep));
                        }
                    });
                    ui.add_space(8.0);
                    if button(ui, pal, "Close", false, false).clicked() {
                        keep = false;
                    }
                });
            }
        }

        if keep {
            self.dialog = Some(dialog);
        }
        if let Some(c) = run_after {
            self.run(c, ctx);
        }
        if let Some(uri) = uri_after {
            self.open_uri(&uri);
        }
        if let Some((dir, reveal)) = go_after {
            self.navigate(dir);
            if let Some(p) = reveal {
                self.tab_mut().select_path(&p);
            }
        }
    }
}
