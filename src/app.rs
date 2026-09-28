//! Application state, command dispatch, input handling and top-level layout.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use egui::{Id, Key, Modifiers, Pos2, Rect, Ui, pos2};

use crate::commands::{self, Command};
use crate::config::{Config, ViewMode};
use crate::counts::ItemCounter;
use crate::fs_model::SortKey;
use crate::mounts::{self, Mount, MountWatcher};
use crate::pane::{Pane, Tab};
use crate::search::Search;
use crate::theme;
use crate::undo::{self, UndoOp};
use crate::{fuzzy, ops};

pub const TITLE_H: f32 = 40.0;
pub const CONTROLS_W: f32 = 138.0;

pub struct Clip {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

/// A long-running file operation on a worker thread.
pub struct Job {
    pub label: String,
    pub progress: Arc<Mutex<String>>,
    rx: Receiver<JobResult>,
}

/// A finished job: a message plus, when it can be reversed, how to undo it.
type JobResult = Result<(String, Option<UndoOp>), String>;

/// An in-progress inline rename (F2) in a list row.
pub struct Renaming {
    pub pane: usize,
    pub path: PathBuf,
    pub text: String,
    pub init: bool,
}

pub enum Dialog {
    Palette {
        query: String,
        cursor: usize,
    },
    GoTo {
        text: String,
        cursor: usize,
    },
    Search {
        query: String,
        root: PathBuf,
        search: Option<Search>,
        cursor: usize,
    },
    Rename {
        path: PathBuf,
        name: String,
        init: bool,
        error: Option<String>,
    },
    Create {
        dir: PathBuf,
        name: String,
        folder: bool,
        error: Option<String>,
    },
    ConfirmDelete {
        paths: Vec<PathBuf>,
    },
    /// A paste/drop where some names already exist in the destination.
    Conflict(Conflict),
    Trash(crate::trashview::TrashView),
    OpenWith(crate::openwith::OpenWithView),
    Help,
    Settings,
}

/// What to do when a pasted item's name is already taken.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Resolve {
    KeepBoth,
    Replace,
    Skip,
}

pub struct Conflict {
    pub paths: Vec<PathBuf>,
    pub dest: PathBuf,
    pub cut: bool,
    /// Indices into `paths` whose name exists in `dest`.
    pub conflicts: Vec<usize>,
    /// Which conflict is being asked about.
    pub pos: usize,
    pub choices: Vec<Resolve>,
    pub apply_all: bool,
}

/// A popup command menu (right-click menu, ⋮ menu, filter options).
pub struct Menu {
    pub pos: Pos2,
    pub query: String,
    pub cursor: usize,
    pub items: Vec<MenuItem>,
    pub opened: Instant,
}

#[derive(Clone)]
pub enum MenuItem {
    Cmd(Command),
    Go(PathBuf),
    NewTabAt(PathBuf),
    Unbookmark(PathBuf),
    Sep,
}

/// Payload for dragging files around inside the app.
pub struct DragPaths(pub Vec<PathBuf>);

/// Things the UI asks the app to do after drawing (avoids borrow conflicts).
pub enum Action {
    Activate(usize),
    Navigate(PathBuf),
    OpenEntries,
    OpenInNewTab(PathBuf),
    Run(Command),
    SelectTab(usize, usize),
    CloseTab(usize, usize),
    NewTabIn(usize),
    MoveTab(usize, usize, usize),
    Drop { paths: Vec<PathBuf>, dest: PathBuf, force_copy: bool },
    OpenMenu(Pos2, Vec<MenuItem>),
    TrashPaths(Vec<PathBuf>),
    MountVolume(String),
    EjectVolume(crate::udisks::Volume),
}

#[derive(Clone, Copy, Default)]
pub struct PaneGeom {
    pub cols: usize,
    pub page_rows: usize,
}

pub struct FileFlier {
    pub cfg: Config,
    pub panes: [Pane; 2],
    pub active: usize,
    pub clipboard: Option<Clip>,
    pub dialog: Option<Dialog>,
    pub menu: Option<Menu>,
    pub status: Option<(String, Instant, bool)>,
    pub job: Option<Job>,
    /// Background previews (inspector, Quick Look) and grid thumbnails.
    pub pv: crate::preview::Previewer,
    pub insp_view: crate::ui::ViewState,
    pub ql_view: crate::ui::ViewState,
    /// Page shown for multi-page documents, per item.
    pub preview_page: (PathBuf, usize),
    pub mounts: Vec<Mount>,
    mount_watcher: MountWatcher,
    /// A remote address (smb://…) to open once the background `gio mount` finishes.
    pending_uri: Option<String>,
    pub actions: Vec<Action>,
    pub geom: [PaneGeom; 2],
    pub sidebar_filter: String,
    /// Cached child counts for folders shown in the Items column.
    pub item_counts: ItemCounter,
    /// A terminal just launched through `flatpak-spawn`, watched briefly for failure.
    terminal_launch: Option<(std::process::Child, Instant)>,
    /// Last see-through state sent to the window (blur is only toggled on change).
    blur_applied: Option<bool>,
    /// Whether the window was created with transparency (needed for see-through).
    pub window_transparent: bool,
    /// The last reversible operation (Ctrl+Z).
    pub last_undo: Option<UndoOp>,
    /// Whether the current toast offers an Undo button.
    pub toast_undo: bool,
    pub quicklook: bool,
    pub renaming: Option<Renaming>,
    /// Where a drive mounted from the sidebar ended up (opened when the job finishes).
    mounted_at: Arc<Mutex<Option<PathBuf>>>,
    /// Clipboard and drag-and-drop with other apps (set up on the first frame).
    native: Option<crate::native::Native>,
    /// The current in-app drag has been handed to the desktop.
    drag_out: bool,
    /// A window move/resize was handed to the compositor, which swallows the
    /// button release; we synthesize it so egui doesn't think the button is stuck.
    pub release_after_grab: bool,
}

impl FileFlier {
    pub fn new(cc: &eframe::CreationContext<'_>, start: Option<PathBuf>, window_transparent: bool) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        theme::load_fonts(&cc.egui_ctx);
        let mut cfg = Config::load();
        if !cfg.preview_panel_intro {
            // 0.4 made the preview panel much more useful: show it once to everyone.
            cfg.preview_panel_intro = true;
            cfg.show_preview = true;
            cfg.save();
        }
        crate::preview::warm_up();
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let cli_start = start.and_then(|p| std::fs::canonicalize(p).ok()).filter(|p| p.is_dir());
        let restore = cli_start.is_none() && cfg.startup == crate::config::Startup::RestoreSession;
        let start = cli_start.unwrap_or_else(|| home.clone());
        let mut panes = [
            Pane::new(start, cfg.show_hidden, cfg.sort, cfg.view),
            Pane::new(home.clone(), cfg.show_hidden, cfg.sort, cfg.view),
        ];
        let mut active = 0;
        if restore {
            // Reopen last session's tabs; folders that no longer exist are skipped.
            for (i, paths) in cfg.session.panes.iter().take(2).enumerate() {
                let tabs: Vec<Tab> = paths
                    .iter()
                    .filter(|p| p.is_dir())
                    .map(|p| Tab::new(p.clone(), cfg.show_hidden, cfg.sort, cfg.view))
                    .collect();
                if !tabs.is_empty() {
                    let want = cfg.session.active_tabs.get(i).copied().unwrap_or(0);
                    panes[i].active = want.min(tabs.len() - 1);
                    panes[i].tabs = tabs;
                }
            }
            active = if cfg.split { cfg.session.active_pane.min(1) } else { 0 };
        }
        theme::apply(&cc.egui_ctx, cfg.palette(), cfg.ui_scale, cfg.animations);
        Self {
            cfg,
            panes,
            active,
            clipboard: None,
            dialog: None,
            menu: None,
            status: None,
            job: None,
            pv: crate::preview::Previewer::new(&cc.egui_ctx),
            insp_view: Default::default(),
            ql_view: Default::default(),
            preview_page: (PathBuf::new(), 0),
            mounts: Vec::new(),
            mount_watcher: MountWatcher::start(cc.egui_ctx.clone()),
            pending_uri: None,
            actions: Vec::new(),
            geom: [PaneGeom { cols: 1, page_rows: 15 }; 2],
            sidebar_filter: String::new(),
            item_counts: ItemCounter::start(cc.egui_ctx.clone()),
            terminal_launch: None,
            blur_applied: None,
            window_transparent,
            last_undo: None,
            toast_undo: false,
            quicklook: false,
            renaming: None,
            mounted_at: Default::default(),
            native: None,
            drag_out: false,
            release_after_grab: false,
        }
    }

    pub fn pal(&self) -> &'static theme::Palette {
        self.cfg.palette()
    }

    pub fn fx(&self) -> crate::ui::Fx {
        crate::ui::Fx {
            glass: self.cfg.glass,
            // See-through needs a transparent window, which is only created at startup.
            see_through: self.cfg.see_through() && self.window_transparent,
            opacity: self.cfg.glass_opacity.clamp(0.4, 0.97),
            animations: self.cfg.animations,
        }
    }

    /// Re-applies colors and scale after a settings change, and saves.
    pub fn apply_settings(&mut self, ctx: &egui::Context) {
        theme::apply(ctx, self.cfg.palette(), self.cfg.ui_scale, self.cfg.animations);
        self.cfg.save();
    }
    pub fn tab(&self) -> &Tab {
        self.panes[self.active].tab()
    }
    pub fn tab_mut(&mut self) -> &mut Tab {
        self.panes[self.active].tab_mut()
    }
    fn other_pane_dir(&self) -> PathBuf {
        self.panes[1 - self.active].tab().path.clone()
    }

    pub fn info(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now(), false));
        self.toast_undo = false;
    }

    /// A toast with an Undo button for `op` (also available via Ctrl+Z).
    pub fn info_undoable(&mut self, msg: impl Into<String>, op: UndoOp) {
        self.info(msg);
        self.last_undo = Some(op);
        self.toast_undo = true;
    }

    pub fn undo_last(&mut self) {
        let Some(op) = self.last_undo.take() else {
            self.info("Nothing to undo");
            return;
        };
        let what = op.describe();
        match undo::undo(op) {
            Ok(msg) => self.info(msg),
            Err(e) => self.error(format!("Couldn't undo {what}: {e}")),
        }
        self.reload_all();
    }
    pub fn error(&mut self, msg: impl Into<String>) {
        self.toast_undo = false;
        self.status = Some((msg.into(), Instant::now(), true));
    }

    pub fn navigate(&mut self, path: PathBuf) {
        if !path.is_dir() {
            self.error(format!("Not a folder: {}", path.display()));
            return;
        }
        if self.tab().locked && path != self.tab().path {
            self.open_tab(path);
            return;
        }
        let (h, s) = (self.cfg.show_hidden, self.cfg.sort);
        self.tab_mut().navigate(path, h, s);
        self.after_navigate();
    }

    fn after_navigate(&mut self) {
        let p = self.tab().path.clone();
        self.cfg.push_recent(p);
        self.cfg.save();
    }

    pub fn reload_all(&mut self) {
        let (h, s) = (self.cfg.show_hidden, self.cfg.sort);
        self.item_counts.clear();
        for pane in &mut self.panes {
            for tab in &mut pane.tabs {
                tab.reload(h, s);
            }
        }
    }

    pub fn open_entries(&mut self) {
        let targets = self.tab().targets();
        if let [single] = targets.as_slice()
            && single.is_dir()
        {
            self.navigate(single.clone());
            return;
        }
        let files: Vec<PathBuf> = targets.into_iter().filter(|p| !p.is_dir()).collect();
        if files.len() > 25 {
            self.error(format!("Not opening {} files at once; select 25 or fewer", files.len()));
            return;
        }
        for p in files {
            if let Err(e) = open::that_detached(&p) {
                self.error(format!("Could not open {}: {e}", p.display()));
            }
        }
    }

    pub(crate) fn start_job<F>(&mut self, label: String, work: F)
    where
        F: FnOnce(&Mutex<String>) -> JobResult + Send + 'static,
    {
        if self.job.is_some() {
            self.error("Another operation is still running");
            return;
        }
        let progress = Arc::new(Mutex::new(String::new()));
        let (tx, rx) = channel();
        let p = progress.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work(&p));
        });
        self.job = Some(Job { label, progress, rx });
    }

    /// Copies or moves `paths` into `dest`, first asking what to do about names
    /// that already exist there.
    pub fn transfer(&mut self, paths: Vec<PathBuf>, dest: PathBuf, cut: bool) {
        if paths.is_empty() {
            return;
        }
        let conflicts: Vec<usize> = paths
            .iter()
            .enumerate()
            .filter(|(_, src)| {
                let Some(name) = src.file_name() else { return false };
                let target = dest.join(name);
                // Copying into its own folder is a plain duplicate ("name (2)").
                target != **src && std::fs::symlink_metadata(&target).is_ok()
            })
            .map(|(i, _)| i)
            .collect();
        let choices = vec![Resolve::KeepBoth; paths.len()];
        if conflicts.is_empty() {
            self.transfer_resolved(paths, dest, cut, choices);
        } else {
            self.dialog =
                Some(Dialog::Conflict(Conflict { paths, dest, cut, conflicts, pos: 0, choices, apply_all: false }));
        }
    }

    /// Copies or moves `paths` into `dest` on a worker thread. "Replace" moves the
    /// existing item to the trash first, so nothing is ever lost.
    pub fn transfer_resolved(&mut self, paths: Vec<PathBuf>, dest: PathBuf, cut: bool, choices: Vec<Resolve>) {
        let (paths, choices): (Vec<PathBuf>, Vec<Resolve>) =
            paths.into_iter().zip(choices).filter(|(_, c)| *c != Resolve::Skip).unzip();
        if paths.is_empty() {
            return;
        }
        let verb = if cut { "Moving" } else { "Copying" };
        let label = format!("{verb} {} item{} to {}", paths.len(), plural(paths.len()), display_name(&dest));
        self.start_job(label, move |progress| {
            let mut errors = Vec::new();
            let mut done = 0;
            let mut pairs = Vec::new();
            let mut replaced = Vec::new();
            let started = std::time::SystemTime::now();
            for (src, choice) in paths.iter().zip(&choices) {
                let Some(name) = src.file_name() else { continue };
                *progress.lock().unwrap() = name.to_string_lossy().into_owned();
                if cut && src.parent() == Some(dest.as_path()) {
                    continue; // moving onto itself is a no-op
                }
                let target = if *choice == Resolve::Replace {
                    let target = dest.join(name);
                    if src.starts_with(&target) {
                        errors.push(format!(
                            "{}: can't replace a folder with something inside it",
                            name.to_string_lossy()
                        ));
                        continue;
                    }
                    if std::fs::symlink_metadata(&target).is_ok() {
                        if let Err(e) = ops::trash(std::slice::from_ref(&target)) {
                            errors.push(format!(
                                "{}: couldn't move the old one to the trash ({e})",
                                name.to_string_lossy()
                            ));
                            continue;
                        }
                        replaced.push(target.clone());
                    }
                    target
                } else {
                    ops::unique_dest(&dest, &name.to_string_lossy())
                };
                let res = if cut { ops::move_path(src, &target) } else { ops::copy_recursive(src, &target) };
                match res {
                    Ok(()) => {
                        done += 1;
                        pairs.push((src.clone(), target));
                    }
                    Err(e) => errors.push(format!("{}: {e}", name.to_string_lossy())),
                }
            }
            if errors.is_empty() {
                let mut msg = format!("{} {done} item{}", if cut { "Moved" } else { "Copied" }, plural(done));
                if !replaced.is_empty() {
                    msg.push_str(&format!(
                        " (replaced {} — the old one{} went to the trash)",
                        replaced.len(),
                        if replaced.len() == 1 { "" } else { "s" }
                    ));
                }
                let mut ops_done = Vec::new();
                if !replaced.is_empty() {
                    ops_done.push(undo::trashed(&replaced, started));
                }
                if !pairs.is_empty() {
                    ops_done.push(UndoOp::Transfer { pairs, moved: cut });
                }
                let op = match ops_done.len() {
                    0 => None,
                    1 => ops_done.pop(),
                    _ => Some(UndoOp::Many(ops_done)),
                };
                Ok((msg, op))
            } else {
                Err(errors.join("; "))
            }
        });
    }

    /// Opens a network address such as `smb://nas/share` through GNOME's GVFS mounts.
    pub fn open_uri(&mut self, uri: &str) {
        let uri = uri.trim().trim_end_matches('/').to_string();
        if let Some(path) = mounts::resolve_uri(&uri, &mounts::gvfs_root()) {
            self.navigate(path);
            return;
        }
        if self.pending_uri.is_some() {
            return; // already tried mounting; don't loop
        }
        // Not mounted yet: ask GVFS to mount it (uses saved passwords / guest access).
        let has_gio = !ops::in_flatpak()
            && std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("gio").is_file()));
        if !has_gio {
            self.error(format!(
                "{uri} isn't mounted. Open it once in your desktop's file manager (or mount it via /etc/fstab), then try again."
            ));
            return;
        }
        self.pending_uri = Some(uri.clone());
        let target = uri.clone();
        self.start_job(format!("Connecting to {uri}"), move |_| {
            let out = std::process::Command::new("gio")
                .args(["mount", &target])
                .stdin(std::process::Stdio::null())
                .output()
                .map_err(|e| e.to_string())?;
            if out.status.success() {
                Ok((format!("Connected to {target}"), None))
            } else {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                Err(format!(
                    "Couldn't connect to {target}: {err}. Shares that need a password must be opened once in your desktop's file manager first."
                ))
            }
        });
    }

    /// Ends an inline rename: applies it (undoable) or cancels. On error the field
    /// stays open so the name can be fixed.
    pub fn finish_rename(&mut self, commit: bool) {
        let Some(mut rn) = self.renaming.take() else { return };
        if !commit {
            return;
        }
        let name = rn.text.trim().to_string();
        let target = rn.path.with_file_name(&name);
        if target == rn.path {
            return;
        }
        let res = ops::validate_name(&name).and_then(|()| {
            ops::rename_noreplace(&rn.path, &target).map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    format!("“{name}” already exists")
                } else {
                    e.to_string()
                }
            })
        });
        match res {
            Ok(()) => {
                self.info_undoable(
                    format!("Renamed to {name}"),
                    UndoOp::Rename { from: rn.path.clone(), to: target.clone() },
                );
                self.reload_all();
                self.panes[rn.pane].tab_mut().select_path(&target);
            }
            Err(e) => {
                self.error(e);
                rn.init = true; // refocus the field
                self.renaming = Some(rn);
            }
        }
    }

    pub fn show_trash(&mut self) {
        self.dialog = Some(Dialog::Trash(crate::trashview::TrashView::open()));
    }

    pub fn open_with(&mut self, ctx: &egui::Context) {
        let files: Vec<PathBuf> = self.tab().targets().into_iter().filter(|p| !p.is_dir()).collect();
        if files.is_empty() {
            self.error("Select a file to open with another app");
            return;
        }
        if ops::in_flatpak() {
            // The sandbox can't see or start host apps; the desktop shows its own chooser.
            let file = files[0].clone();
            self.start_job("Opening the app chooser".into(), move |_| {
                crate::openwith::portal_open_with(&file)?;
                Ok((String::new(), None))
            });
            return;
        }
        self.dialog = Some(Dialog::OpenWith(crate::openwith::OpenWithView::new(files, ctx)));
    }

    pub fn trash_paths(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let when = SystemTime::now();
        match ops::trash(&paths) {
            Ok(()) => {
                let msg = format!("Moved {} item{} to trash", paths.len(), plural(paths.len()));
                if undo::CAN_RESTORE_TRASH {
                    self.info_undoable(msg, UndoOp::Trash { originals: paths, when });
                } else {
                    self.info(msg);
                }
            }
            Err(e) => self.error(format!("Trash failed: {e}")),
        }
        self.reload_all();
    }

    pub fn start_delete(&mut self, paths: Vec<PathBuf>) {
        let n = paths.len();
        self.start_job(format!("Deleting {n} item{}", plural(n)), move |progress| {
            let mut errors = Vec::new();
            for p in &paths {
                *progress.lock().unwrap() = display_name(p);
                if let Err(e) = ops::remove_permanently(p) {
                    errors.push(format!("{}: {e}", p.display()));
                }
            }
            if errors.is_empty() {
                Ok((format!("Deleted {n} item{}", plural(n)), None))
            } else {
                Err(errors.join("; "))
            }
        });
    }

    /// Drag-and-drop: move within the same filesystem, copy across (or when Ctrl is held).
    fn drop_paths(&mut self, paths: Vec<PathBuf>, dest: PathBuf, force_copy: bool) {
        use std::os::unix::fs::MetadataExt;
        let paths: Vec<PathBuf> =
            paths.into_iter().filter(|p| p.parent() != Some(dest.as_path()) && !dest.starts_with(p)).collect();
        if paths.is_empty() {
            return;
        }
        let dev = |p: &Path| std::fs::symlink_metadata(p).map(|m| m.dev()).ok();
        let same_fs = dev(&dest).is_some() && paths.iter().all(|p| dev(p) == dev(&dest));
        self.transfer(paths, dest, same_fs && !force_copy);
    }

    pub fn run(&mut self, cmd: Command, ctx: &egui::Context) {
        use Command::*;
        let (h, s) = (self.cfg.show_hidden, self.cfg.sort);
        match cmd {
            CommandPalette => self.dialog = Some(Dialog::Palette { query: String::new(), cursor: 0 }),
            Search => {
                let root = self.tab().path.clone();
                self.dialog = Some(Dialog::Search { query: String::new(), root, search: None, cursor: 0 });
            }
            GoToPath => {
                let mut text = self.tab().path.to_string_lossy().into_owned();
                if !text.ends_with('/') {
                    text.push('/');
                }
                self.dialog = Some(Dialog::GoTo { text, cursor: 0 });
            }
            NewTab => {
                let p = self.tab().path.clone();
                self.open_tab(p);
            }
            OpenInNewTab => {
                let p = self.tab().cursor_entry().filter(|e| e.is_dir).map(|e| e.path.clone());
                let p = p.unwrap_or_else(|| self.tab().path.clone());
                self.open_tab(p);
            }
            CloseTab => {
                let pane = &mut self.panes[self.active];
                let idx = pane.active;
                pane.close_tab(idx);
            }
            NextTab => {
                let pane = &mut self.panes[self.active];
                pane.active = (pane.active + 1) % pane.tabs.len();
            }
            PrevTab => {
                let pane = &mut self.panes[self.active];
                pane.active = (pane.active + pane.tabs.len() - 1) % pane.tabs.len();
            }
            ToggleSplit => {
                self.cfg.split = !self.cfg.split;
                if !self.cfg.split {
                    self.active = 0;
                }
                self.cfg.save();
            }
            SwitchPane => {
                if self.cfg.split {
                    self.active = 1 - self.active;
                }
            }
            Back => {
                self.tab_mut().go_back(h, s);
                self.after_navigate();
            }
            Forward => {
                self.tab_mut().go_forward(h, s);
                self.after_navigate();
            }
            Up => {
                if let Some(parent) = self.tab().path.parent().map(Path::to_path_buf) {
                    if self.tab().locked {
                        self.open_tab(parent);
                    } else {
                        self.tab_mut().go_up(h, s);
                        self.after_navigate();
                    }
                }
            }
            Home => {
                if let Some(home) = dirs::home_dir() {
                    self.navigate(home);
                }
            }
            Refresh => self.reload_all(),
            Open => self.open_entries(),
            OpenTerminal => {
                let dir = self.tab().path.clone();
                match ops::open_terminal(&dir) {
                    Ok(child) => self.terminal_launch = child.map(|c| (c, Instant::now())),
                    Err(ops::TerminalError::Failed(e)) => self.error(e),
                    Err(ops::TerminalError::NeedsPermission(cmd)) => {
                        ctx.copy_text(cmd);
                        self.error(
                            "Terminal access is off. A command to enable it was copied: run it in a terminal, then restart File Flier.",
                        );
                    }
                }
            }
            ToggleHidden => {
                self.cfg.show_hidden = !self.cfg.show_hidden;
                self.cfg.save();
                let h = self.cfg.show_hidden;
                for pane in &mut self.panes {
                    for tab in &mut pane.tabs {
                        tab.refilter(h);
                    }
                }
                self.info(if h { "Showing hidden files" } else { "Hiding hidden files" });
            }
            TogglePreview => {
                self.cfg.show_preview = !self.cfg.show_preview;
                self.cfg.save();
            }
            ToggleSidebar => {
                self.cfg.show_sidebar = !self.cfg.show_sidebar;
                self.cfg.save();
            }
            ToggleTheme => {
                // Flip between the default dark and light themes.
                let dark = self.pal().is_dark;
                self.cfg.theme = Some(if dark { theme::ThemeId::Light } else { theme::ThemeId::Dark });
                self.apply_settings(ctx);
            }
            Settings => self.dialog = Some(Dialog::Settings),
            ViewDetails | ViewList | ViewGrid => {
                let view = match cmd {
                    ViewDetails => ViewMode::Details,
                    ViewList => ViewMode::List,
                    _ => ViewMode::Grid,
                };
                self.tab_mut().view = view;
                self.tab_mut().scroll_to_cursor = true;
                self.cfg.view = view;
                self.cfg.save();
            }
            LockTab => {
                let tab = self.tab_mut();
                tab.locked = !tab.locked;
                let locked = tab.locked;
                self.info(if locked { "Tab locked: folders open in new tabs" } else { "Tab unlocked" });
            }
            SelectAll => self.tab_mut().select_all(),
            Copy | Cut => {
                let paths = self.tab().targets();
                if !paths.is_empty() {
                    let cut = cmd == Cut;
                    let n = paths.len();
                    self.info(format!("{} {n} item{}", if cut { "Cut" } else { "Copied" }, plural(n)));
                    // Put real files on the system clipboard so other apps (file
                    // managers, chat apps, browsers) can paste them; plain paths otherwise.
                    let native = self.native.as_ref().is_some_and(|n| n.set_clipboard(&paths, cut));
                    if !native {
                        ctx.copy_text(paths_text(&paths));
                    }
                    self.clipboard = Some(Clip { paths, cut });
                }
            }
            Paste => {
                if let Some(clip) = self.clipboard.take() {
                    let dest = self.tab().path.clone();
                    let (paths, cut) = (clip.paths.clone(), clip.cut);
                    if !cut {
                        self.clipboard = Some(clip); // copies can be pasted repeatedly
                    }
                    self.transfer(paths, dest, cut);
                } else {
                    self.info("Clipboard is empty");
                }
            }
            CopyToOtherPane | MoveToOtherPane => {
                if !self.cfg.split {
                    self.error("Open split view (Ctrl+\\) to copy between panes");
                    return;
                }
                let paths = self.tab().targets();
                let dest = self.other_pane_dir();
                self.transfer(paths, dest, cmd == MoveToOtherPane);
            }
            Trash => {
                let paths = self.tab().targets();
                self.trash_paths(paths);
            }
            DeletePermanently => {
                let paths = self.tab().targets();
                if !paths.is_empty() {
                    self.dialog = Some(Dialog::ConfirmDelete { paths });
                }
            }
            Rename => {
                if let Some(e) = self.tab().cursor_entry() {
                    let (path, name) = (e.path.clone(), e.name.clone());
                    if self.tab().view == ViewMode::Grid {
                        self.dialog = Some(Dialog::Rename { path, name, init: true, error: None });
                    } else {
                        // Rename in place, right in the row.
                        self.quicklook = false;
                        self.tab_mut().scroll_to_cursor = true;
                        self.renaming = Some(Renaming { pane: self.active, path, text: name, init: true });
                    }
                }
            }
            QuickLook => {
                self.quicklook = !self.quicklook && self.tab().cursor_entry().is_some();
            }
            Undo => self.undo_last(),
            Compress => {
                let paths = self.tab().targets();
                if paths.is_empty() {
                    return;
                }
                let dir = self.tab().path.clone();
                let name =
                    if paths.len() == 1 { format!("{}.zip", display_name(&paths[0])) } else { "Archive.zip".into() };
                let out = ops::unique_dest(&dir, &name);
                let n = paths.len();
                self.start_job(format!("Compressing {n} item{}", plural(n)), move |progress| {
                    crate::archive::compress(&paths, &out, progress)?;
                    Ok((format!("Created {}", display_name(&out)), Some(UndoOp::Create { path: out })))
                });
            }
            Extract => {
                let archives: Vec<PathBuf> =
                    self.tab().targets().into_iter().filter(|p| crate::archive::can_extract(p)).collect();
                if archives.is_empty() {
                    self.error("Select an archive (zip, tar, 7z...) to extract");
                    return;
                }
                let dir = self.tab().path.clone();
                let n = archives.len();
                self.start_job(format!("Extracting {n} archive{}", plural(n)), move |progress| {
                    let mut created = Vec::new();
                    for a in &archives {
                        created.push(
                            crate::archive::extract(a, &dir, progress)
                                .map_err(|e| format!("{}: {e}", display_name(a)))?,
                        );
                    }
                    let msg = match created.as_slice() {
                        [one] => format!("Extracted to {}", display_name(one)),
                        _ => format!("Extracted {} archives", created.len()),
                    };
                    let op = match created.len() {
                        1 => UndoOp::Create { path: created.pop().unwrap() },
                        _ => UndoOp::Many(created.into_iter().map(|path| UndoOp::Create { path }).collect()),
                    };
                    Ok((msg, Some(op)))
                });
            }
            OpenWith => self.open_with(ctx),
            ShowTrash => self.show_trash(),
            NewFolder | NewFile => {
                let dir = self.tab().path.clone();
                let folder = cmd == NewFolder;
                let name = if folder { "New Folder" } else { "New File.txt" };
                let name = ops::unique_dest(&dir, name).file_name().unwrap().to_string_lossy().into_owned();
                self.dialog = Some(Dialog::Create { dir, name, folder, error: None });
            }
            CopyPath => {
                ctx.copy_text(paths_text(&self.tab().targets()));
                self.info("Path copied to clipboard");
            }
            ToggleBookmark => {
                let p = self.tab().path.clone();
                self.toggle_bookmark(p);
            }
            SortByName | SortBySize | SortByModified | SortByKind => {
                let key = match cmd {
                    SortByName => SortKey::Name,
                    SortBySize => SortKey::Size,
                    SortByModified => SortKey::Modified,
                    _ => SortKey::Kind,
                };
                self.set_sort(key);
            }
            ReverseSort => {
                self.cfg.sort.descending = !self.cfg.sort.descending;
                self.cfg.save();
                self.reload_all();
            }
            Help => self.dialog = Some(Dialog::Help),
        }
    }

    /// Handles Ctrl+V: our own clipboard if it's still current, otherwise
    /// any file paths or file:// URIs copied from another application.
    fn paste_text(&mut self, text: &str, ctx: &egui::Context) {
        if self.clipboard.as_ref().is_some_and(|c| paths_text(&c.paths) == text.trim_end()) {
            self.run(Command::Paste, ctx);
            return;
        }
        let paths: Vec<PathBuf> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| match l.strip_prefix("file://") {
                Some(rest) => PathBuf::from(percent_decode(rest)),
                None => PathBuf::from(l),
            })
            .filter(|p| p.is_absolute() && std::fs::symlink_metadata(p).is_ok())
            .collect();
        if paths.is_empty() {
            self.info("Clipboard doesn't contain any files");
            return;
        }
        let dest = self.tab().path.clone();
        self.transfer(paths, dest, false);
    }

    fn set_sort(&mut self, key: SortKey) {
        if self.cfg.sort.key == key {
            self.cfg.sort.descending = !self.cfg.sort.descending;
        } else {
            self.cfg.sort.key = key;
            self.cfg.sort.descending = matches!(key, SortKey::Size | SortKey::Modified);
        }
        self.cfg.save();
        self.reload_all();
    }

    pub fn toggle_bookmark(&mut self, p: PathBuf) {
        if let Some(i) = self.cfg.bookmarks.iter().position(|b| b == &p) {
            self.cfg.bookmarks.remove(i);
            self.info(format!("Removed bookmark {}", display_name(&p)));
        } else {
            self.info(format!("Bookmarked {}", display_name(&p)));
            self.cfg.bookmarks.push(p);
        }
        self.cfg.save();
    }

    pub fn open_tab(&mut self, path: PathBuf) {
        let (h, s, v) = (self.cfg.show_hidden, self.cfg.sort, self.tab().view);
        let pane = &mut self.panes[self.active];
        pane.tabs.insert(pane.active + 1, Tab::new(path, h, s, v));
        pane.active += 1;
    }

    // ------------------------------------------------------------------ input

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Keyboard-first: don't let Tab park focus on buttons.
        if let Some(id) = ctx.memory(|m| m.focused()) {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }

        // Quick Look: Space/Esc close, Enter opens, ←/→ step through items.
        if self.quicklook {
            if ctx.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Escape) || i.consume_key(Modifiers::NONE, Key::Space)
            }) {
                self.quicklook = false;
                return;
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                self.quicklook = false;
                self.open_entries();
                return;
            }
            let page = self.preview_page.1;
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::PageDown)) {
                self.preview_page.1 = page + 1;
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::PageUp)) {
                self.preview_page.1 = page.saturating_sub(1);
            }
            if ctx
                .input_mut(|i| i.consume_key(Modifiers::NONE, Key::Plus) || i.consume_key(Modifiers::NONE, Key::Equals))
            {
                self.ql_view.zoom_by(1.5);
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Minus)) {
                self.ql_view.zoom_by(1.0 / 1.5);
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Num0)) {
                self.ql_view.reset();
            }
            if self.geom[self.active].cols <= 1 {
                let tab = self.tab_mut();
                let (cur, last) = (tab.cursor, tab.visible.len().saturating_sub(1));
                if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight)) {
                    tab.move_cursor((cur + 1).min(last), false);
                }
                if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft)) {
                    tab.move_cursor(cur.saturating_sub(1), false);
                }
            }
        }

        // egui turns Ctrl+C/X/V into clipboard events rather than key presses.
        let clip_events: Vec<egui::Event> = ctx.input(|i| {
            i.events
                .iter()
                .filter(|e| matches!(e, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)))
                .cloned()
                .collect()
        });
        let shift = ctx.input(|i| i.modifiers.shift);
        for e in clip_events {
            match e {
                egui::Event::Copy if shift => self.run(Command::CopyPath, ctx),
                egui::Event::Copy => self.run(Command::Copy, ctx),
                egui::Event::Cut => self.run(Command::Cut, ctx),
                egui::Event::Paste(text) => self.paste_text(&text, ctx),
                _ => {}
            }
        }

        // Backspace edits the quick filter while one is active.
        if !self.tab().filter.is_empty() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace)) {
            let h = self.cfg.show_hidden;
            let tab = self.tab_mut();
            tab.filter.pop();
            tab.refilter(h);
            return;
        }

        // More specific shortcuts first so Ctrl+Shift+T isn't eaten by Ctrl+T.
        let mut all: Vec<(Command, egui::KeyboardShortcut)> =
            commands::ALL.iter().flat_map(|&c| c.shortcuts().into_iter().map(move |s| (c, s))).collect();
        all.sort_by_key(|(_, s)| {
            let m = s.modifiers;
            std::cmp::Reverse(m.shift as u8 + m.alt as u8 + (m.command || m.ctrl) as u8)
        });
        let filtering = !self.tab().filter.is_empty();
        for (cmd, shortcut) in all {
            // While typing a filter, Space is part of the filter, not Quick Look.
            if cmd == Command::QuickLook && filtering {
                continue;
            }
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                self.run(cmd, ctx);
                return;
            }
        }

        let PaneGeom { cols, page_rows } = self.geom[self.active];
        let cols = cols.max(1);
        let page = page_rows.max(1) * cols;
        let tab = self.panes[self.active].tab_mut();
        let n = tab.visible.len();
        let last = n.saturating_sub(1);
        let mut keys = vec![
            (Key::ArrowDown, (tab.cursor + cols).min(last)),
            (Key::ArrowUp, tab.cursor.saturating_sub(cols)),
            (Key::PageDown, (tab.cursor + page).min(last)),
            (Key::PageUp, tab.cursor.saturating_sub(page)),
            (Key::Home, 0),
            (Key::End, last),
        ];
        if cols > 1 {
            keys.push((Key::ArrowRight, (tab.cursor + 1).min(last)));
            keys.push((Key::ArrowLeft, tab.cursor.saturating_sub(1)));
        }
        let mut moves: Vec<(usize, bool)> = Vec::new();
        ctx.input_mut(|i| {
            for (key, target) in keys {
                if i.consume_key(Modifiers::SHIFT, key) {
                    moves.push((target, true));
                } else if i.consume_key(Modifiers::NONE, key) {
                    moves.push((target, false));
                }
            }
        });
        for (to, extend) in moves {
            tab.move_cursor(to, extend);
        }

        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            if tab.filter.is_empty() {
                tab.selected.clear();
            } else {
                tab.filter.clear();
                tab.refilter(self.cfg.show_hidden);
            }
        }
        if cols == 1 {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight))
                && let Some(e) = tab.cursor_entry().filter(|e| e.is_dir)
            {
                let p = e.path.clone();
                self.navigate(p);
                return;
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft)) {
                self.run(Command::Up, ctx);
                return;
            }
        }

        // Typing filters the current folder instantly.
        let typed: String = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect()
        });
        let typed: String = typed.chars().filter(|c| !c.is_control()).collect();
        // A leading space shouldn't start a filter.
        let blank_start = tab.filter.is_empty() && typed.trim().is_empty();
        if !typed.is_empty() && !blank_start {
            tab.filter.push_str(&typed);
            tab.refilter(self.cfg.show_hidden);
            tab.cursor = 0;
            tab.anchor = 0;
            tab.selected.clear();
        }
    }

    /// Drags that leave the window go to the desktop; files dropped from other
    /// apps (Wayland) are copied into the current folder.
    fn native_dnd(&mut self, ctx: &egui::Context) {
        let Some(native) = &self.native else { return };
        let payload = egui::DragAndDrop::payload::<DragPaths>(ctx);
        match payload {
            Some(p) if !self.drag_out => {
                // The pointer left the window mid-drag (winit may stop reporting
                // positions outside it, so a missing hover position counts too).
                let inside = ctx.content_rect().shrink(2.0);
                let outside = ctx.input(|i| match i.pointer.hover_pos() {
                    None => i.pointer.latest_pos().is_some(),
                    Some(pos) => !inside.contains(pos),
                });
                if outside && native.start_drag(&p.0) {
                    // The desktop owns the drag now; dropping back on File Flier cancels it.
                    self.drag_out = true;
                    egui::DragAndDrop::clear_payload(ctx);
                }
            }
            None => self.drag_out = false,
            _ => {}
        }
        let drops = native.take_drops();
        for d in drops {
            let dest = self.tab().path.clone();
            let paths: Vec<PathBuf> = d.paths.into_iter().filter(|p| p.parent() != Some(dest.as_path())).collect();
            if !paths.is_empty() {
                self.transfer(paths, dest, false);
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            let dest = self.tab().path.clone();
            self.transfer(dropped, dest, false);
        }
    }

    /// Remembers Ctrl+/- zoom changes and the open tabs (for "restore last session").
    fn persist_ui_state(&mut self, ctx: &egui::Context) {
        let zoom = ctx.zoom_factor();
        if (zoom - self.cfg.ui_scale).abs() > 0.001 {
            self.cfg.ui_scale = zoom.clamp(0.8, 1.5);
            if (zoom - self.cfg.ui_scale).abs() > 0.001 {
                ctx.set_zoom_factor(self.cfg.ui_scale);
            }
            self.cfg.save();
        }
        let n = if self.cfg.split { 2 } else { 1 };
        let session = crate::config::Session {
            panes: self.panes[..n].iter().map(|p| p.tabs.iter().map(|t| t.path.clone()).collect()).collect(),
            active_tabs: self.panes[..n].iter().map(|p| p.active).collect(),
            active_pane: self.active,
        };
        if session != self.cfg.session {
            self.cfg.session = session;
            self.cfg.save();
        }
    }

    fn poll_background(&mut self, ctx: &egui::Context) {
        self.pv.poll(ctx);
        self.pv.thumbs.begin_frame(ctx);
        if let Some((child, started)) = &mut self.terminal_launch {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.terminal_launch = None;
                    match status.code() {
                        Some(0) | None => {}
                        Some(127) => self.error("No terminal app found on your system (set $TERMINAL)"),
                        Some(_) => self.error("Could not open a terminal on the host"),
                    }
                }
                // Terminals that stay attached are fine; stop watching after a few seconds.
                Ok(None) if started.elapsed() < Duration::from_secs(3) => {
                    ctx.request_repaint_after(Duration::from_millis(100))
                }
                _ => {
                    // Reap it whenever it exits so it doesn't linger as a zombie.
                    if let Some((mut child, _)) = self.terminal_launch.take() {
                        std::thread::spawn(move || child.wait());
                    }
                }
            }
        }
        if let Some(job) = &self.job {
            match job.rx.try_recv() {
                Ok(res) => {
                    self.job = None;
                    let ok = res.is_ok();
                    match res {
                        Ok((msg, Some(op))) => self.info_undoable(msg, op),
                        Ok((msg, None)) if !msg.is_empty() => self.info(msg),
                        Ok(_) => {}
                        Err(e) => self.error(e),
                    }
                    self.reload_all();
                    let mounted = self.mounted_at.lock().unwrap().take();
                    if let Some(p) = mounted {
                        self.navigate(p);
                    }
                    if let Some(uri) = self.pending_uri.take()
                        && ok
                    {
                        self.open_uri(&uri);
                    }
                }
                Err(_) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        self.item_counts.poll();
        self.mounts = self.mount_watcher.get();
        let (h, s) = (self.cfg.show_hidden, self.cfg.sort);
        for pane in &mut self.panes {
            let tab = pane.tab_mut();
            // Don't poll folders on network/cloud mounts every second: a stalled server
            // would block the UI. Refresh those with Ctrl+R.
            if !mounts::is_remote_path(&tab.path, &self.mounts) && tab.check_changed(h, s) {
                self.item_counts.clear();
            }
        }
        ctx.request_repaint_after(Duration::from_millis(1000));
    }

    fn apply_actions(&mut self, ctx: &egui::Context) {
        for a in std::mem::take(&mut self.actions) {
            match a {
                Action::Activate(i) => self.active = i,
                Action::Navigate(p) => self.navigate(p),
                Action::OpenEntries => self.open_entries(),
                Action::OpenInNewTab(p) => self.open_tab(p),
                Action::Run(c) => self.run(c, ctx),
                Action::SelectTab(p, t) => {
                    self.active = p;
                    self.panes[p].active = t;
                }
                Action::CloseTab(p, t) => {
                    self.panes[p].close_tab(t);
                }
                Action::NewTabIn(p) => {
                    self.active = p;
                    self.run(Command::NewTab, ctx);
                }
                Action::MoveTab(p, from, to) => {
                    let pane = &mut self.panes[p];
                    if from < pane.tabs.len() && to < pane.tabs.len() && from != to {
                        let t = pane.tabs.remove(from);
                        pane.tabs.insert(to, t);
                        pane.active = to;
                    }
                }
                Action::Drop { paths, dest, force_copy } => self.drop_paths(paths, dest, force_copy),
                Action::TrashPaths(paths) => self.trash_paths(paths),
                Action::MountVolume(object) => {
                    let slot = self.mounted_at.clone();
                    self.start_job("Mounting drive".into(), move |_| {
                        let path = crate::udisks::mount(&object)?;
                        *slot.lock().unwrap() = Some(path.clone());
                        Ok((format!("Mounted at {}", path.display()), None))
                    });
                }
                Action::EjectVolume(vol) => {
                    // Leave the drive first so it isn't busy.
                    for i in 0..2 {
                        if vol.mount_points.iter().any(|m| self.panes[i].tab().path.starts_with(m))
                            && let Some(home) = dirs::home_dir()
                        {
                            self.panes[i].tab_mut().navigate(home, self.cfg.show_hidden, self.cfg.sort);
                        }
                    }
                    let all: Vec<crate::udisks::Volume> = self.mounts.iter().filter_map(|m| m.volume.clone()).collect();
                    let name = vol.label.clone();
                    self.start_job(format!("Ejecting {name}"), move |_| {
                        crate::udisks::eject(&vol, &all)?;
                        Ok((format!("{name} can be unplugged safely"), None))
                    });
                }
                Action::OpenMenu(pos, items) => {
                    self.menu = Some(Menu { pos, query: String::new(), cursor: 0, items, opened: Instant::now() })
                }
            }
        }
    }

    /// Items for the command palette: commands plus bookmarked / recent folders.
    pub fn palette_items(&self, query: &str) -> Vec<PaletteItem> {
        let q = query.trim();
        let mut scored: Vec<(i32, PaletteItem)> = commands::ALL
            .iter()
            .filter(|c| **c != Command::CommandPalette)
            .filter_map(|&c| fuzzy::score(q, c.label()).map(|s| (s + 5, PaletteItem::Cmd(c))))
            .collect();
        let mut seen = std::collections::HashSet::new();
        for p in self.cfg.bookmarks.iter().chain(self.cfg.recent.iter()) {
            if seen.insert(p.clone())
                && p.is_dir()
                && !q.is_empty()
                && let Some(s) = fuzzy::score(q, &display_name(p))
            {
                scored.push((s, PaletteItem::Go(p.clone())));
            }
        }
        if !q.is_empty() {
            scored.sort_by_key(|a| std::cmp::Reverse(a.0));
        }
        scored.into_iter().map(|(_, i)| i).collect()
    }

    /// Completions for the GoTo box: sub-folders of the typed parent, then recents.
    pub fn goto_items(&self, text: &str) -> Vec<PathBuf> {
        let typed = expand_tilde(text.trim());
        let (dir, partial) = if text.ends_with('/') {
            (typed.clone(), String::new())
        } else {
            let partial = typed.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            (typed.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/")), partial)
        };
        let mut out: Vec<(i32, PathBuf)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let hidden_ok = self.cfg.show_hidden || partial.starts_with('.') || !name.starts_with('.');
                if hidden_ok
                    && e.path().is_dir()
                    && let Some(s) = fuzzy::score(&partial, &name)
                {
                    out.push((s, e.path()));
                }
            }
        }
        out.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| crate::fs_model::natural_cmp(&display_name(&a.1), &display_name(&b.1)))
        });
        let mut paths: Vec<PathBuf> = out.into_iter().map(|(_, p)| p).take(60).collect();
        if !text.contains('/') || text.trim().is_empty() {
            let q = text.trim();
            for p in self.cfg.bookmarks.iter().chain(&self.cfg.recent) {
                if !paths.contains(p) && p.is_dir() && fuzzy::score(q, &p.to_string_lossy()).is_some() {
                    paths.push(p.clone());
                }
            }
        }
        paths
    }
}

pub enum PaletteItem {
    Cmd(Command),
    Go(PathBuf),
}

impl eframe::App for FileFlier {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.release_after_grab {
            self.release_after_grab = false;
            let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap_or_default();
            raw.events.insert(
                0,
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: raw.modifiers,
                },
            );
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.fx().see_through {
            return [0.0; 4]; // let the (compositor-blurred) desktop show through
        }
        let c = self.pal().bg;
        [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, 1.0]
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.native.is_none() {
            self.native = Some(crate::native::Native::new(frame, &ctx));
        }
        self.native_dnd(&ctx);
        let pal = self.pal();
        self.poll_background(&ctx);
        self.persist_ui_state(&ctx);

        // Text fields that own the keyboard while focused.
        let text_ids = [
            Id::new(("filter", 0usize)),
            Id::new(("filter", 1usize)),
            Id::new("sidebar_filter"),
            Id::new("inline_rename"),
        ];
        let focused = ctx.memory(|m| m.focused());
        let typing = focused.is_some_and(|f| text_ids.contains(&f));
        if self.dialog.is_none() && self.menu.is_none() {
            if typing {
                let renaming = focused == Some(Id::new("inline_rename"));
                let done = ctx.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::Enter))
                    || (!renaming && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)));
                if done {
                    ctx.memory_mut(|m| m.surrender_focus(focused.unwrap()));
                }
            } else {
                self.handle_keys(&ctx);
            }
            self.handle_dropped_files(&ctx);
        }

        let title = format!("{} — File Flier", self.tab().path.display());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        // ---- layout
        let full = ui.max_rect();
        let fx = self.fx();
        // Ask the compositor to blur what's behind the window (KDE Plasma, macOS;
        // a no-op elsewhere). Only called when the setting changes.
        if self.blur_applied != Some(fx.see_through) {
            if let Some(w) = frame.winit_window() {
                w.set_blur(fx.see_through);
            }
            self.blur_applied = Some(fx.see_through);
        }
        if fx.see_through {
            // A light tint keeps text readable over the blurred desktop.
            ui.painter().rect_filled(full, 0.0, crate::ui::glass::with_alpha(pal.bg, fx.opacity * 0.45));
        } else if fx.glass {
            crate::ui::glass::backdrop(ui.painter(), full, pal, ctx.input(|i| i.time), fx.animations);
            if fx.animations {
                ctx.request_repaint_after(Duration::from_millis(66));
            }
        } else {
            ui.painter().rect_filled(full, 0.0, pal.bg);
        }
        let body = Rect::from_min_max(pos2(full.left(), full.top() + TITLE_H), full.max);
        let sb_w = if self.cfg.show_sidebar { self.cfg.sidebar_width.clamp(160.0, 420.0) } else { 0.0 };
        let insp_w = if self.cfg.show_preview {
            self.cfg.inspector_width.clamp(240.0, 900.0).min(full.width() * 0.5)
        } else {
            0.0
        };
        let sidebar = Rect::from_min_max(body.min, pos2(body.left() + sb_w, body.bottom()));
        let inspector = Rect::from_min_max(pos2(body.right() - insp_w, body.top()), body.max);
        let panes_area = Rect::from_min_max(pos2(sidebar.right(), body.top()), pos2(inspector.left(), body.bottom()));
        let pane_rects: Vec<Rect> = if self.cfg.split {
            let x = panes_area.left() + panes_area.width() * self.cfg.split_ratio.clamp(0.2, 0.8);
            vec![
                Rect::from_min_max(panes_area.min, pos2(x, panes_area.bottom())),
                Rect::from_min_max(pos2(x + 1.0, panes_area.top()), panes_area.max),
            ]
        } else {
            vec![panes_area]
        };

        // Glass mode floats each area as a separate panel with small gaps.
        let g = if fx.glass { 6.0 } else { 0.0 };
        let inset = |r: Rect, l: f32, rt: f32| {
            Rect::from_min_max(pos2(r.left() + l, r.top()), pos2(r.right() - rt, r.bottom() - g))
        };
        let sidebar = inset(sidebar, g, g / 2.0);
        let inspector = inset(inspector, g / 2.0, g);
        let n = pane_rects.len();
        let pane_rects: Vec<Rect> = pane_rects
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let left = if i == 0 && sb_w == 0.0 { g } else { g / 2.0 };
                let right = if i + 1 == n && insp_w == 0.0 { g } else { g / 2.0 };
                inset(*r, left, right)
            })
            .collect();

        if self.cfg.show_sidebar {
            self.sidebar_ui(ui, sidebar);
        }
        for (i, r) in pane_rects.iter().enumerate() {
            if fx.glass {
                crate::ui::surface(ui.painter(), *r, pal.bg, &fx, pal);
            }
            self.pane_ui(ui, i, *r);
        }
        if self.cfg.split {
            self.split_divider(ui, panes_area, pane_rects[0].right() + g / 2.0, !fx.glass);
        }
        if self.cfg.show_preview {
            self.inspector_ui(ui, inspector);
        }
        let title_rect = Rect::from_min_max(full.min, pos2(full.right(), full.top() + TITLE_H));
        let strips: Vec<(usize, f32, f32)> =
            pane_rects.iter().enumerate().map(|(i, r)| (i, r.left(), r.right())).collect();
        self.title_bar(ui, title_rect, sb_w, &strips);
        self.resize_edges(ui, full);

        self.quicklook_ui(&ctx);
        self.menu_ui(&ctx);
        self.dialogs(&ctx);
        self.toasts(ui, full);
        self.drag_overlay(&ctx);
        self.apply_actions(&ctx);
    }
}

// ---------------------------------------------------------------- helpers

pub fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

pub fn display_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned())
}

pub fn step(cursor: usize, up: bool, down: bool, len: usize) -> usize {
    let mut c = cursor.min(len.saturating_sub(1));
    if up {
        c = c.saturating_sub(1);
    }
    if down && c + 1 < len {
        c += 1;
    }
    c
}

fn paths_text(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n")
}

/// Decodes %XX escapes in file:// URIs.
pub fn percent_decode(s: &str) -> std::ffi::OsString {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        if bytes[i] == b'%'
            && let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    // Raw bytes: Linux file names needn't be valid UTF-8.
    std::os::unix::ffi::OsStringExt::from_vec(out)
}

pub fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix('~')
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest.trim_start_matches('/'));
    }
    PathBuf::from(s)
}

pub fn human_size(bytes: u64) -> String {
    humansize::format_size(bytes, humansize::DECIMAL.decimal_places(1).space_after_value(true)).replace("KB", "kB")
}

pub fn format_time(t: SystemTime, style: crate::config::DateStyle) -> String {
    use crate::config::DateStyle;
    let dt: chrono::DateTime<chrono::Local> = t.into();
    match style {
        DateStyle::Iso => dt.format("%Y-%m-%d %H:%M").to_string(),
        DateStyle::Friendly => dt.format("%b %-d, %Y").to_string(),
        DateStyle::Relative => relative_time(dt, chrono::Local::now()),
    }
}

fn relative_time(dt: chrono::DateTime<chrono::Local>, now: chrono::DateTime<chrono::Local>) -> String {
    let secs = (now - dt).num_seconds();
    if secs < 0 {
        return dt.format("%b %-d, %Y").to_string(); // in the future: just show the date
    }
    let days = (now.date_naive() - dt.date_naive()).num_days();
    match secs {
        0..60 => "Just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        _ if days == 0 => format!("{} h ago", secs / 3600),
        _ if days == 1 => format!("Yesterday {}", dt.format("%H:%M")),
        _ if days < 7 => dt.format("%A %H:%M").to_string(),
        _ if dt.format("%Y").to_string() == now.format("%Y").to_string() => dt.format("%b %-d").to_string(),
        _ => dt.format("%b %-d, %Y").to_string(),
    }
}

pub fn format_mode(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    format!("{s} ({:o})", mode & 0o777)
}

/// Fraction of a filesystem that's in use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiskSpace {
    pub free: u64,
    pub total: u64,
}

impl DiskSpace {
    /// Fraction of the filesystem in use.
    pub fn used(&self) -> f32 {
        1.0 - self.free as f32 / self.total as f32
    }
}

pub fn disk_space(path: &Path) -> Option<DiskSpace> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` is a properly sized out-parameter.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 || st.f_blocks == 0 {
        return None;
    }
    let unit = if st.f_frsize > 0 { st.f_frsize } else { st.f_bsize } as u64;
    Some(DiskSpace { free: st.f_bavail as u64 * unit, total: st.f_blocks as u64 * unit })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_file_uris() {
        assert_eq!(percent_decode("/home/me/My%20File.txt"), "/home/me/My File.txt");
        assert_eq!(percent_decode("/a%2"), "/a%2");
        assert_eq!(percent_decode("/a%é"), "/a%é");
        // Non-UTF-8 bytes survive exactly.
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(percent_decode("/x%FF").as_bytes(), b"/x\xff");
    }

    #[test]
    fn list_stepping() {
        assert_eq!(step(0, true, false, 5), 0);
        assert_eq!(step(4, false, true, 5), 4);
        assert_eq!(step(2, false, true, 5), 3);
        assert_eq!(step(9, false, false, 3), 2);
    }

    #[test]
    fn relative_dates() {
        use chrono::TimeZone;
        let now = chrono::Local.with_ymd_and_hms(2026, 9, 26, 15, 0, 0).unwrap();
        let ago = |s: i64| relative_time(now - chrono::Duration::seconds(s), now);
        assert_eq!(ago(10), "Just now");
        assert_eq!(ago(5 * 60), "5 min ago");
        assert_eq!(ago(3 * 3600), "3 h ago");
        assert_eq!(ago(20 * 3600), "Yesterday 19:00");
        assert_eq!(relative_time(chrono::Local.with_ymd_and_hms(2025, 1, 2, 9, 0, 0).unwrap(), now), "Jan 2, 2025");
    }

    #[test]
    fn sizes_look_like_file_pilot() {
        assert_eq!(human_size(412), "412 B");
        assert_eq!(human_size(10_600), "10.6 kB");
    }
}
