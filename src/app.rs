//! Application state, command dispatch, input handling and top-level layout.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use egui::{Id, Key, Modifiers, Pos2, Rect, Ui, pos2};

use crate::commands::{self, Command};
use crate::config::{Config, ViewMode};
use crate::counts::ItemCounter;
use crate::fs_model::{Entry, SortKey};
use crate::mounts::{self, Mount, MountWatcher};
use crate::pane::{Pane, Tab};
use crate::search::Search;
use crate::theme;
use crate::{fuzzy, ops};

pub const TITLE_H: f32 = 40.0;
pub const INSPECTOR_W: f32 = 320.0;
pub const CONTROLS_W: f32 = 138.0;

pub struct Clip {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

/// A long-running file operation on a worker thread.
pub struct Job {
    pub label: String,
    pub progress: Arc<Mutex<String>>,
    rx: Receiver<Result<String, String>>,
}

pub enum Dialog {
    Palette { query: String, cursor: usize },
    GoTo { text: String, cursor: usize },
    Search { query: String, root: PathBuf, search: Option<Search>, cursor: usize },
    Rename { path: PathBuf, name: String, init: bool, error: Option<String> },
    Create { dir: PathBuf, name: String, folder: bool, error: Option<String> },
    ConfirmDelete { paths: Vec<PathBuf> },
    Help,
}

pub enum Preview {
    Dir(Vec<Entry>),
    Text(String),
    Image,
    Binary,
    Error(String),
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
    pub preview: Option<(PathBuf, Option<SystemTime>, Preview)>,
    pub mounts: Vec<Mount>,
    mount_watcher: MountWatcher,
    /// A remote address (smb://…) to open once the background `gio mount` finishes.
    pending_uri: Option<String>,
    pub actions: Vec<Action>,
    pub geom: [PaneGeom; 2],
    pub sidebar_filter: String,
    /// Cached child counts for folders shown in the Items column.
    pub item_counts: ItemCounter,
}

impl FileFlier {
    pub fn new(cc: &eframe::CreationContext<'_>, start: Option<PathBuf>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        theme::load_system_font(&cc.egui_ctx);
        let cfg = Config::load();
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let start =
            start.and_then(|p| std::fs::canonicalize(p).ok()).filter(|p| p.is_dir()).unwrap_or_else(|| home.clone());
        let panes = [
            Pane::new(start, cfg.show_hidden, cfg.sort, cfg.view),
            Pane::new(home, cfg.show_hidden, cfg.sort, cfg.view),
        ];
        theme::apply(&cc.egui_ctx, cfg.dark_mode);
        Self {
            cfg,
            panes,
            active: 0,
            clipboard: None,
            dialog: None,
            menu: None,
            status: None,
            job: None,
            preview: None,
            mounts: Vec::new(),
            mount_watcher: MountWatcher::start(cc.egui_ctx.clone()),
            pending_uri: None,
            actions: Vec::new(),
            geom: [PaneGeom { cols: 1, page_rows: 15 }; 2],
            sidebar_filter: String::new(),
            item_counts: ItemCounter::start(cc.egui_ctx.clone()),
        }
    }

    pub fn pal(&self) -> &'static theme::Palette {
        theme::palette(self.cfg.dark_mode)
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
    }
    pub fn error(&mut self, msg: impl Into<String>) {
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

    fn open_entries(&mut self) {
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

    fn start_job<F>(&mut self, label: String, work: F)
    where
        F: FnOnce(&Mutex<String>) -> Result<String, String> + Send + 'static,
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

    /// Copies or moves `paths` into `dest` on a worker thread.
    pub fn transfer(&mut self, paths: Vec<PathBuf>, dest: PathBuf, cut: bool) {
        if paths.is_empty() {
            return;
        }
        let verb = if cut { "Moving" } else { "Copying" };
        let label = format!("{verb} {} item{} to {}", paths.len(), plural(paths.len()), display_name(&dest));
        self.start_job(label, move |progress| {
            let mut errors = Vec::new();
            let mut done = 0;
            for src in &paths {
                let Some(name) = src.file_name() else { continue };
                *progress.lock().unwrap() = name.to_string_lossy().into_owned();
                if cut && src.parent() == Some(dest.as_path()) {
                    continue; // moving onto itself is a no-op
                }
                let target = ops::unique_dest(&dest, &name.to_string_lossy());
                let res = if cut { ops::move_path(src, &target) } else { ops::copy_recursive(src, &target) };
                match res {
                    Ok(()) => done += 1,
                    Err(e) => errors.push(format!("{}: {e}", name.to_string_lossy())),
                }
            }
            if errors.is_empty() {
                Ok(format!("{} {done} item{}", if cut { "Moved" } else { "Copied" }, plural(done)))
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
                Ok(format!("Connected to {target}"))
            } else {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                Err(format!(
                    "Couldn't connect to {target}: {err}. Shares that need a password must be opened once in your desktop's file manager first."
                ))
            }
        });
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
            if errors.is_empty() { Ok(format!("Deleted {n} item{}", plural(n))) } else { Err(errors.join("; ")) }
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
                    Ok(()) => {}
                    Err(ops::TerminalError::Failed(e)) => self.error(e),
                    Err(ops::TerminalError::NeedsPermission(cmd)) => {
                        ctx.copy_text(cmd);
                        self.error(
                            "Terminal access is off. A command to enable it was copied: paste it into a terminal.",
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
                self.cfg.dark_mode = !self.cfg.dark_mode;
                self.cfg.save();
                theme::apply(ctx, self.cfg.dark_mode);
            }
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
                    // Mirror to the system clipboard so paths can be pasted elsewhere.
                    ctx.copy_text(paths_text(&paths));
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
                if paths.is_empty() {
                    return;
                }
                match ops::trash(&paths) {
                    Ok(()) => self.info(format!("Moved {} item{} to trash", paths.len(), plural(paths.len()))),
                    Err(e) => self.error(format!("Trash failed: {e}")),
                }
                self.reload_all();
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
                    self.dialog = Some(Dialog::Rename { path, name, init: true, error: None });
                }
            }
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
        for (cmd, shortcut) in all {
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

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            let dest = self.tab().path.clone();
            self.transfer(dropped, dest, false);
        }
    }

    fn poll_background(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.job {
            match job.rx.try_recv() {
                Ok(res) => {
                    self.job = None;
                    let ok = res.is_ok();
                    match res {
                        Ok(msg) => self.info(msg),
                        Err(e) => self.error(e),
                    }
                    self.reload_all();
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
            scored.sort_by(|a, b| b.0.cmp(&a.0));
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
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = self.pal().bg;
        [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, 1.0]
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let pal = self.pal();
        self.poll_background(&ctx);

        // Text fields that own the keyboard while focused.
        let text_ids = [Id::new(("filter", 0usize)), Id::new(("filter", 1usize)), Id::new("sidebar_filter")];
        let focused = ctx.memory(|m| m.focused());
        let typing = focused.is_some_and(|f| text_ids.contains(&f));
        if self.dialog.is_none() && self.menu.is_none() {
            if typing {
                let done = ctx.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::Enter))
                    || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown));
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
        ui.painter().rect_filled(full, 0.0, pal.bg);
        let body = Rect::from_min_max(pos2(full.left(), full.top() + TITLE_H), full.max);
        let sb_w = if self.cfg.show_sidebar { self.cfg.sidebar_width.clamp(160.0, 420.0) } else { 0.0 };
        let insp_w = if self.cfg.show_preview { INSPECTOR_W.min(full.width() * 0.4) } else { 0.0 };
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

        if self.cfg.show_sidebar {
            self.sidebar_ui(ui, sidebar);
        }
        for (i, r) in pane_rects.iter().enumerate() {
            self.pane_ui(ui, i, *r);
        }
        if self.cfg.split {
            self.split_divider(ui, panes_area, pane_rects[0].right());
        }
        if self.cfg.show_preview {
            self.inspector_ui(ui, inspector);
        }
        let title_rect = Rect::from_min_max(full.min, pos2(full.right(), full.top() + TITLE_H));
        let strips: Vec<(usize, f32, f32)> =
            pane_rects.iter().enumerate().map(|(i, r)| (i, r.left(), r.right())).collect();
        self.title_bar(ui, title_rect, sb_w, &strips);
        self.resize_edges(ui, full);

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

pub fn format_time(t: SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = t.into();
    dt.format("%Y-%m-%d %H:%M").to_string()
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
pub fn disk_usage(path: &Path) -> Option<f32> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` is a properly sized out-parameter.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 || st.f_blocks == 0 {
        return None;
    }
    Some(1.0 - st.f_bavail as f32 / st.f_blocks as f32)
}

pub fn load_preview(e: &Entry) -> Preview {
    if e.is_dir {
        return match crate::fs_model::read_dir(&e.path) {
            Ok(mut entries) => {
                entries.retain(|x| !x.is_hidden());
                crate::fs_model::sort_entries(&mut entries, Default::default());
                entries.truncate(300);
                Preview::Dir(entries)
            }
            Err(err) => Preview::Error(err.to_string()),
        };
    }
    // Never open pipes, sockets or devices: reading a FIFO blocks forever.
    if !e.is_file {
        return Preview::Binary;
    }
    if matches!(e.extension().as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp") {
        return Preview::Image;
    }
    use std::io::Read;
    let mut buf = vec![0u8; 64 * 1024];
    let n = match std::fs::File::open(&e.path).and_then(|mut f| f.read(&mut buf)) {
        Ok(n) => n,
        Err(err) => return Preview::Error(err.to_string()),
    };
    buf.truncate(n);
    if buf.contains(&0) {
        return Preview::Binary;
    }
    // Tolerate a multi-byte char cut off at the buffer end.
    let text = match std::str::from_utf8(&buf) {
        Ok(t) => t.to_string(),
        Err(err) if err.error_len().is_none() => String::from_utf8_lossy(&buf[..err.valid_up_to()]).into_owned(),
        Err(_) => return Preview::Binary,
    };
    Preview::Text(text.lines().take(400).collect::<Vec<_>>().join("\n"))
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
    fn sizes_look_like_file_pilot() {
        assert_eq!(human_size(412), "412 B");
        assert_eq!(human_size(10_600), "10.6 kB");
    }
}
