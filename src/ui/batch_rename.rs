//! The Batch rename extension: rename several files at once, as GNOME Files does,
//! from a template (the original name, automatic numbers, the creation date) or by
//! finding and replacing text. A preview shows every new name before anything changes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use egui::{Align2, Id, Key, Sense, Stroke, Ui, pos2, vec2};

use super::popups::{button, label};
use super::{elided, text};
use crate::app::{FileFlier, display_name};
use crate::theme::Palette;

pub const ORIGINAL: &str = "[Original file name]";
const NUM1: &str = "[1, 2, 3]";
const NUM2: &str = "[01, 02, 03]";
const NUM3: &str = "[001, 002, 003]";
const DATE: &str = "[Creation date]";

/// The dialog's state.
pub struct BatchRename {
    /// The files, in the order they're shown (the order they're numbered in).
    pub paths: Vec<PathBuf>,
    replace_mode: bool,
    template: String,
    find: String,
    with: String,
    pub(crate) add_open: bool,
    error: Option<String>,
    init: bool,
}

impl BatchRename {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        BatchRename {
            paths,
            replace_mode: false,
            template: ORIGINAL.to_string(),
            find: String::new(),
            with: String::new(),
            add_open: false,
            error: None,
            init: true,
        }
    }

    /// The new name of each file, in order (the same name if it doesn't change).
    pub fn new_names(&self) -> Vec<String> {
        self.paths
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let name = display_name(p);
                let (stem, ext) = split_ext(&name, p.is_dir());
                let stem = if self.replace_mode {
                    if self.find.is_empty() { stem.to_string() } else { stem.replace(&self.find, &self.with) }
                } else {
                    fill_template(&self.template, stem, i, || created(p))
                };
                format!("{}{ext}", stem.trim())
            })
            .collect()
    }
}

/// Splits "report.final.pdf" into ("report.final", ".pdf"). Folders and dotfiles keep
/// their whole name as the stem.
fn split_ext(name: &str, is_dir: bool) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && !is_dir => name.split_at(i),
        _ => (name, ""),
    }
}

/// The template with its tags filled in for file number `i` (from 0).
fn fill_template(template: &str, stem: &str, i: usize, date: impl Fn() -> String) -> String {
    let n = i + 1;
    let mut s = template
        .replace(ORIGINAL, stem)
        .replace(NUM1, &n.to_string())
        .replace(NUM2, &format!("{n:02}"))
        .replace(NUM3, &format!("{n:03}"));
    if s.contains(DATE) {
        s = s.replace(DATE, &date());
    }
    s
}

/// The day a file was created (or last changed, where creation isn't known).
fn created(p: &Path) -> String {
    let meta = std::fs::metadata(p).ok();
    let t = meta.as_ref().and_then(|m| m.created().or_else(|_| m.modified()).ok());
    t.map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d").to_string()).unwrap_or_default()
}

/// The renames to do, as (from, to): only the files whose name changes. Fails if a
/// name is invalid, two files would get the same name, or a name is taken by a file
/// that isn't being renamed.
pub fn plan(paths: &[PathBuf], names: &[String]) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let sources: HashSet<&PathBuf> = paths.iter().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (p, name) in paths.iter().zip(names) {
        crate::ops::validate_name(name)?;
        let to = p.with_file_name(name);
        if !seen.insert(to.clone()) {
            return Err(format!("Two files would be named “{name}”"));
        }
        if to == *p {
            continue;
        }
        if to.symlink_metadata().is_ok() && !sources.contains(&to) {
            return Err(format!("“{name}” already exists"));
        }
        out.push((p.clone(), to));
    }
    Ok(out)
}

fn radio(ui: &mut Ui, pal: &Palette, label_text: &str, on: bool) -> bool {
    let g = ui.painter().layout_no_wrap(label_text.to_string(), super::font(14.0), pal.text);
    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 30.0, 28.0), Sense::click());
    let p = ui.painter();
    let c = pos2(r.left() + 10.0, r.center().y);
    if on {
        p.circle_filled(c, 8.0, pal.accent);
        p.circle_filled(c, 3.0, pal.on_accent);
    } else {
        let s = if resp.hovered() { pal.text } else { pal.text_dim };
        p.circle_stroke(c, 7.5, Stroke::new(1.5, s));
    }
    p.galley(pos2(r.left() + 24.0, r.center().y - g.size().y / 2.0), g, pal.text);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

impl FileFlier {
    /// The Batch rename dialog. Returns true when it should close.
    pub(crate) fn batch_rename_ui(&mut self, ui: &mut Ui, st: &mut BatchRename) -> bool {
        let pal = self.pal();
        let mut close = false;
        let mut submit = false;

        // Header: Cancel, the title, Rename.
        ui.horizontal(|ui| {
            if button(ui, pal, "Cancel", false, false).clicked() {
                close = true;
            }
            let title = format!("Rename {} Files", st.paths.len());
            let w = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(w - 100.0, 30.0), Sense::hover());
            super::text_bold(ui.painter(), r.center(), Align2::CENTER_CENTER, title, 15.0, pal.text_strong);
            submit |= button(ui, pal, "Rename", true, false).clicked();
        });
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - 440.0) / 2.0).max(0.0));
            if radio(ui, pal, "Rename using a template", !st.replace_mode) {
                st.replace_mode = false;
            }
            ui.add_space(12.0);
            if radio(ui, pal, "Find and replace text", st.replace_mode) {
                st.replace_mode = true;
                st.add_open = false;
            }
        });
        ui.add_space(8.0);

        let field = |ui: &mut Ui, value: &mut String, id: Id, w: f32, hint: &str| {
            ui.add_sized(vec2(w, 30.0), egui::TextEdit::singleline(value).id(id).hint_text(hint).margin(vec2(8.0, 6.0)))
        };
        if st.replace_mode {
            let w = ui.available_width();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let r = field(ui, &mut st.find, Id::new("br_find"), (w - 8.0) / 2.0, "Existing text");
                if st.init {
                    r.request_focus();
                    st.init = false;
                }
                field(ui, &mut st.with, Id::new("br_with"), (w - 8.0) / 2.0, "Replace with");
            });
        } else {
            let id = Id::new("br_template");
            let mut add_resp = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let w = ui.available_width() - 86.0;
                let r = field(ui, &mut st.template, id, w, "Name");
                if st.init {
                    r.request_focus();
                    st.init = false;
                }
                let resp = button(ui, pal, "+ Add", false, false);
                if resp.clicked() {
                    st.add_open = !st.add_open;
                }
                add_resp = Some(resp);
            });
            if st.add_open
                && let Some(anchor) = add_resp
            {
                // The Add menu: tags to insert at the cursor.
                let mut picked = None;
                let area = egui::Area::new(Id::new("br_add_menu"))
                    .order(egui::Order::Tooltip)
                    .fixed_pos(anchor.rect.right_bottom() + vec2(-200.0, 6.0))
                    .show(ui.ctx(), |ui| {
                        super::popup_frame(pal, &self.fx()).show(ui, |ui| {
                            ui.set_width(200.0);
                            let heading = |ui: &mut Ui, s: &str| {
                                let (r, _) = ui.allocate_exact_size(vec2(200.0, 26.0), Sense::hover());
                                super::text_bold(
                                    ui.painter(),
                                    r.center(),
                                    Align2::CENTER_CENTER,
                                    s,
                                    13.0,
                                    pal.text_dim,
                                );
                            };
                            let item = |ui: &mut Ui, s: &str| {
                                let (r, resp) = ui.allocate_exact_size(vec2(200.0, 30.0), Sense::click());
                                if resp.hovered() {
                                    ui.painter().rect_filled(r, pal.round(4.0), pal.hover);
                                }
                                text(
                                    ui.painter(),
                                    r.left_center() + vec2(10.0, 0.0),
                                    Align2::LEFT_CENTER,
                                    s,
                                    14.0,
                                    pal.text,
                                );
                                resp.clicked()
                            };
                            heading(ui, "Automatic Numbers");
                            for (shown, tag) in
                                [("1, 2, 3, 4", NUM1), ("01, 02, 03, 04", NUM2), ("001, 002, 003, 004", NUM3)]
                            {
                                if item(ui, shown) {
                                    picked = Some(tag);
                                }
                            }
                            heading(ui, "Metadata");
                            if item(ui, "Creation Date") {
                                picked = Some(DATE);
                            }
                            if !st.template.contains(ORIGINAL) && item(ui, "Original File Name") {
                                picked = Some(ORIGINAL);
                            }
                        });
                    });
                if let Some(tag) = picked {
                    insert_at_cursor(ui.ctx(), id, &mut st.template, tag);
                    st.add_open = false;
                } else if ui.input(|i| i.pointer.any_click())
                    && !area.response.contains_pointer()
                    && !anchor.contains_pointer()
                {
                    st.add_open = false;
                }
            }
        }
        ui.add_space(10.0);

        // Preview: every file, old name → new name.
        let names = st.new_names();
        let plan = plan(&st.paths, &names);
        let w = ui.available_width();
        egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
            for (p, new) in st.paths.iter().zip(&names) {
                let (r, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
                let pnt = ui.painter();
                pnt.hline(r.x_range(), r.top(), Stroke::new(1.0, pal.row_sep));
                let half = (r.width() - 36.0) / 2.0;
                let old = display_name(p);
                let g = elided(pnt, &old, 13.5, pal.text, half - 8.0);
                pnt.galley(pos2(r.left() + 6.0, r.center().y - g.size().y / 2.0), g, pal.text);
                text(
                    pnt,
                    pos2(r.left() + half + 18.0, r.center().y),
                    Align2::CENTER_CENTER,
                    "→",
                    15.0,
                    pal.text_strong,
                );
                let c = if *new == old { pal.text_dim } else { pal.text_strong };
                let g = elided(pnt, new, 13.5, c, half - 8.0);
                pnt.galley(pos2(r.left() + half + 36.0, r.center().y - g.size().y / 2.0), g, c);
            }
            let (r, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
            ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, pal.row_sep));
        });
        let shown_error = plan.as_ref().err().cloned().or_else(|| st.error.clone());
        if let Some(e) = shown_error {
            ui.add_space(6.0);
            label(ui, pal, &e, 13.0, Some(super::danger_color(pal)));
        }

        submit |= ui.input(|i| i.key_pressed(Key::Enter)) && !st.add_open;
        if submit && let Ok(renames) = plan {
            match self.do_batch_rename(&renames) {
                Ok(()) => close = true,
                Err(e) => st.error = Some(e),
            }
        }
        close
    }

    /// Renames, as one step to undo. Names that swap among the files go through
    /// temporary names first.
    fn do_batch_rename(&mut self, renames: &[(PathBuf, PathBuf)]) -> Result<(), String> {
        use crate::undo::UndoOp;
        if renames.is_empty() {
            return Ok(());
        }
        let sources: HashSet<&PathBuf> = renames.iter().map(|(f, _)| f).collect();
        let swaps = renames.iter().any(|(_, t)| sources.contains(t));
        let mut steps: Vec<(PathBuf, PathBuf)> = Vec::new();
        if swaps {
            for (i, (from, to)) in renames.iter().enumerate() {
                let tmp = from.with_file_name(format!(".file-flier-rename-{}-{i}", std::process::id()));
                steps.push((from.clone(), tmp.clone()));
                steps.push((tmp, to.clone()));
            }
            // All to temporary names first, then to the new ones.
            let (a, b): (Vec<_>, Vec<_>) = steps.iter().cloned().enumerate().partition(|(i, _)| i % 2 == 0);
            steps = a.into_iter().chain(b).map(|(_, s)| s).collect();
        } else {
            steps = renames.to_vec();
        }
        let mut done = Vec::new();
        let mut failed = None;
        for (from, to) in &steps {
            match crate::ops::rename_noreplace(from, to) {
                Ok(()) => done.push((from.clone(), to.clone())),
                Err(e) => {
                    failed = Some(format!("Couldn't rename “{}”: {e}", display_name(from)));
                    break;
                }
            }
        }
        for (from, to) in renames {
            if done.iter().any(|(_, t)| t == to) && to.is_dir() {
                self.carry_folder_color(from, to);
            }
        }
        if !done.is_empty() {
            let n = renames.len();
            let ops = done.into_iter().map(|(from, to)| UndoOp::Rename { from, to }).collect();
            self.info_undoable(format!("Renamed {n} item{}", if n == 1 { "" } else { "s" }), UndoOp::Many(ops));
        }
        self.reload_all();
        match failed {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// Inserts `tag` into the text field `id` at its cursor (or at the end).
fn insert_at_cursor(ctx: &egui::Context, id: Id, value: &mut String, tag: &str) {
    let state = egui::TextEdit::load_state(ctx, id);
    let at = state
        .as_ref()
        .and_then(|s| s.cursor.char_range())
        .map(|r| r.primary.index.0.min(value.chars().count()))
        .unwrap_or(value.chars().count());
    let byte = value.char_indices().nth(at).map_or(value.len(), |(b, _)| b);
    value.insert_str(byte, tag);
    if let Some(mut s) = state {
        let end = egui::text::CCursor::new(at + tag.chars().count());
        s.cursor.set_char_range(Some(egui::text_selection::CCursorRange::one(end)));
        s.store(ctx, id);
    }
    ctx.memory_mut(|m| m.request_focus(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_templates() {
        let d = || "2026-11-05".to_string();
        assert_eq!(fill_template("[Original file name]", "Scan", 0, d), "Scan");
        assert_eq!(fill_template("Trip [01, 02, 03]", "x", 4, d), "Trip 05");
        assert_eq!(fill_template("[001, 002, 003]-[Original file name]", "a", 11, d), "012-a");
        assert_eq!(fill_template("[Creation date] [1, 2, 3]", "a", 0, d), "2026-11-05 1");
        assert_eq!(split_ext("report.final.pdf", false), ("report.final", ".pdf"));
        assert_eq!(split_ext(".bashrc", false), (".bashrc", ""));
        assert_eq!(split_ext("photos.2024", true), ("photos.2024", ""));
    }

    #[test]
    fn plans_renames_and_catches_clashes() {
        let dir = std::env::temp_dir().join(format!("ff-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["a.txt", "b.txt", "other.txt"] {
            std::fs::write(dir.join(n), n).unwrap();
        }
        let paths = vec![dir.join("a.txt"), dir.join("b.txt")];
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // Unchanged names are left out.
        let p = plan(&paths, &names(&["a.txt", "c.txt"])).unwrap();
        assert_eq!(p, vec![(dir.join("b.txt"), dir.join("c.txt"))]);
        // Swapping names among the files is fine; taking another file's name isn't.
        assert!(plan(&paths, &names(&["b.txt", "a.txt"])).is_ok());
        assert!(plan(&paths, &names(&["other.txt", "c.txt"])).is_err());
        assert!(plan(&paths, &names(&["same.txt", "same.txt"])).is_err());
        assert!(plan(&paths, &names(&["", "c.txt"])).is_err());

        let mut app_free = BatchRename::new(paths.clone());
        app_free.template = "Note [1, 2, 3]".into();
        assert_eq!(app_free.new_names(), names(&["Note 1.txt", "Note 2.txt"]));
        app_free.replace_mode = true;
        app_free.find = "a".into();
        app_free.with = "z".into();
        assert_eq!(app_free.new_names(), names(&["z.txt", "b.txt"]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
