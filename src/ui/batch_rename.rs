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

/// A file to rename, read once when the dialog opens.
struct File {
    path: PathBuf,
    name: String,
    is_dir: bool,
    /// The creation (or else last-change) day, for the [Creation date] tag.
    date: String,
}

/// The new names and the renames they need, for one set of inputs.
type Preview = (Vec<String>, Result<Vec<(PathBuf, PathBuf)>, String>);

/// The dialog's state.
pub struct BatchRename {
    /// The files, in the order they're shown (the order they're numbered in).
    files: Vec<File>,
    replace_mode: bool,
    template: String,
    find: String,
    with: String,
    pub(crate) add_open: bool,
    error: Option<String>,
    init: bool,
    /// The preview for the inputs it was made from; remade only when they change.
    preview: Option<((bool, String, String, String), Preview)>,
}

impl BatchRename {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        let files = paths
            .into_iter()
            .map(|path| File { name: display_name(&path), is_dir: path.is_dir(), date: created(&path), path })
            .collect();
        BatchRename {
            files,
            replace_mode: false,
            template: ORIGINAL.to_string(),
            find: String::new(),
            with: String::new(),
            add_open: false,
            error: None,
            init: true,
            preview: None,
        }
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// The new name of each file, in order (the same name if it doesn't change).
    fn new_names(&self) -> Vec<String> {
        self.files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let (stem, ext) = split_ext(&f.name, f.is_dir);
                let stem = if self.replace_mode {
                    if self.find.is_empty() { stem.to_string() } else { stem.replace(&self.find, &self.with) }
                } else {
                    fill_template(&self.template, stem, i, &f.date)
                };
                format!("{}{ext}", stem.trim())
            })
            .collect()
    }

    /// The new names and the renames, remade only when the inputs changed.
    fn preview(&mut self) -> &Preview {
        let key = (self.replace_mode, self.template.clone(), self.find.clone(), self.with.clone());
        if self.preview.as_ref().is_none_or(|(k, _)| *k != key) {
            let names = self.new_names();
            let paths: Vec<PathBuf> = self.files.iter().map(|f| f.path.clone()).collect();
            let plan = plan(&paths, &names);
            self.preview = Some((key, (names, plan)));
        }
        &self.preview.as_ref().expect("just made").1
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

/// The template with its tags filled in for file number `i` (from 0). Tags are
/// replaced in one pass, so a file name that itself contains "[1, 2, 3]" stays as it is.
fn fill_template(template: &str, stem: &str, i: usize, date: &str) -> String {
    let n = i + 1;
    let tags = [
        (ORIGINAL, stem.to_string()),
        (NUM1, n.to_string()),
        (NUM2, format!("{n:02}")),
        (NUM3, format!("{n:03}")),
        (DATE, date.to_string()),
    ];
    let mut out = String::new();
    let mut rest = template;
    'outer: while !rest.is_empty() {
        for (tag, value) in &tags {
            if let Some(after) = rest.strip_prefix(tag) {
                out.push_str(value);
                rest = after;
                continue 'outer;
            }
        }
        let c = rest.chars().next().expect("not empty");
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
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
        if name.len() > 255 {
            return Err(format!("“{}…” is too long for a file name", name.chars().take(40).collect::<String>()));
        }
        if name.starts_with('.') && !display_name(p).starts_with('.') {
            return Err(format!("“{name}” would become a hidden file"));
        }
        let to = p.with_file_name(name);
        if !seen.insert(to.clone()) {
            return Err(format!("Two files would be named “{name}”"));
        }
        if to == *p {
            continue;
        }
        if let Ok(there) = to.symlink_metadata()
            && !sources.contains(&to)
            && !same_file(&there, p)
        {
            return Err(format!("“{name}” already exists"));
        }
        out.push((p.clone(), to));
    }
    Ok(out)
}

/// Whether `meta` is `p` itself: a case-only rename on a drive that ignores case
/// (FAT, exFAT, NTFS) finds the file under its new name already.
fn same_file(meta: &std::fs::Metadata, p: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    p.symlink_metadata().is_ok_and(|m| m.dev() == meta.dev() && m.ino() == meta.ino())
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
            let title = format!("Rename {} Files", st.len());
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

        // Preview: every file, old name → new name (only the rows in view are laid out).
        let (names, plan) = st.preview().clone();
        let w = ui.available_width();
        egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show_rows(
            ui,
            32.0,
            st.files.len(),
            |ui, range| {
                for (f, new) in st.files[range.clone()].iter().zip(&names[range]) {
                    let (r, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
                    let pnt = ui.painter();
                    pnt.hline(r.x_range(), r.top(), Stroke::new(1.0, pal.row_sep));
                    let half = (r.width() - 36.0) / 2.0;
                    let old = f.name.clone();
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
            },
        );
        let (r, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
        ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, pal.row_sep));
        let shown_error = plan.as_ref().err().cloned().or_else(|| st.error.clone());
        if let Some(e) = shown_error {
            ui.add_space(6.0);
            label(ui, pal, &e, 13.0, Some(super::danger_color(pal)));
        }

        submit |= ui.input(|i| i.key_pressed(Key::Enter)) && !st.add_open;
        if submit && let Ok(renames) = plan {
            match self.do_batch_rename(&renames) {
                Ok(()) => close = true,
                Err(e) => {
                    st.error = Some(e);
                    st.preview = None;
                }
            }
        }
        close
    }

    /// Renames, as one step to undo. Names that swap among the files (or that differ
    /// only in case on a drive that ignores case) go through temporary names first. If
    /// a rename fails, the ones already done are put back, so it's all or nothing.
    fn do_batch_rename(&mut self, renames: &[(PathBuf, PathBuf)]) -> Result<(), String> {
        use crate::undo::UndoOp;
        if renames.is_empty() {
            return Ok(());
        }
        let swaps = renames.iter().any(|(_, t)| t.symlink_metadata().is_ok());
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
        if let Some(e) = failed {
            // Put back what was done, newest first.
            let mut stuck = Vec::new();
            for (from, to) in done.into_iter().rev() {
                if crate::ops::rename_noreplace(&to, &from).is_err() {
                    stuck.push((from, to));
                }
            }
            self.reload_all();
            if stuck.is_empty() {
                return Err(format!("{e}. Nothing was renamed."));
            }
            // Couldn't undo everything: keep it undoable with Ctrl+Z.
            stuck.reverse();
            let ops = stuck.into_iter().map(|(from, to)| UndoOp::Rename { from, to }).collect();
            self.info_undoable("Some files couldn't be renamed back", UndoOp::Many(ops));
            return Err(format!("{e}. Some files were renamed; press Ctrl+Z to undo."));
        }
        for (from, to) in renames {
            if to.is_dir() {
                self.carry_folder_color(from, to);
            }
        }
        let n = renames.len();
        let ops = done.into_iter().map(|(from, to)| UndoOp::Rename { from, to }).collect();
        self.info_undoable(format!("Renamed {n} item{}", if n == 1 { "" } else { "s" }), UndoOp::Many(ops));
        self.reload_all();
        Ok(())
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
        let d = "2026-11-05";
        assert_eq!(fill_template("[Original file name]", "Scan", 0, d), "Scan");
        assert_eq!(fill_template("Trip [01, 02, 03]", "x", 4, d), "Trip 05");
        assert_eq!(fill_template("[001, 002, 003]-[Original file name]", "a", 11, d), "012-a");
        assert_eq!(fill_template("[Creation date] [1, 2, 3]", "a", 0, d), "2026-11-05 1");
        // A name containing a tag's text is kept as it is.
        assert_eq!(fill_template("[Original file name] [1, 2, 3]", "x [1, 2, 3]", 1, d), "x [1, 2, 3] 2");
        assert_eq!(fill_template("Ünïcødé [1, 2, 3]", "a", 0, d), "Ünïcødé 1");
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
        // An empty stem would hide the file; overlong names are refused up front.
        assert!(plan(&paths, &names(&[".txt", "c.txt"])).is_err());
        assert!(plan(&paths, &names(&[&format!("{}.txt", "x".repeat(260)), "c.txt"])).is_err());

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
