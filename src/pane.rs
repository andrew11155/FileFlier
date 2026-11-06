//! A pane holds a set of tabs; each tab browses one folder.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use crate::config::{GroupBy, ViewMode};
use crate::fs_model::{self, Entry, Sort};
use crate::fuzzy;

pub struct Tab {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    /// Indices into `entries` that pass the hidden-file and quick-filter checks.
    pub visible: Vec<usize>,
    /// How the items are grouped, and where each group starts in `visible` (with its
    /// label); empty when they aren't grouped.
    pub group: GroupBy,
    pub groups: Vec<(usize, String)>,
    /// The folder whose remembered grouping `group` was read for (read again on
    /// arriving in another folder, so other tabs aren't changed meanwhile).
    pub group_for: Option<PathBuf>,
    pub selected: HashSet<PathBuf>,
    /// Cursor position, as an index into `visible`.
    pub cursor: usize,
    /// Anchor for shift-selection, as an index into `visible`.
    pub anchor: usize,
    pub filter: String,
    pub error: Option<String>,
    pub scroll_to_cursor: bool,
    /// Locked tabs never navigate away; folders open in a new tab instead.
    pub locked: bool,
    pub view: ViewMode,
    /// Tree view: how deep each entry is (0 = directly in `path`), parallel to `entries`.
    pub depth: Vec<usize>,
    /// Tree view: folders that are open.
    pub expanded: HashSet<PathBuf>,
    /// Tree view: modification times of the open folders, to notice changes inside them.
    tree_mtimes: Vec<(PathBuf, Option<SystemTime>)>,
    /// Flat view: list every file in `path` and its subfolders.
    pub flat: bool,
    /// Flat view: the scan still running.
    pub flat_scan: Option<crate::flat::Scan>,
    /// Flat view: the scan stopped at its file limit.
    pub flat_truncated: bool,
    /// Flat view: where to put the cursor once that item turns up.
    flat_keep: Option<PathBuf>,
    /// Tag view (Tags extension): the tag whose files are listed, instead of a folder.
    pub tag: Option<String>,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    dir_mtime: Option<SystemTime>,
    last_check: Instant,
}

impl Tab {
    pub fn new(path: PathBuf, show_hidden: bool, sort: Sort, view: ViewMode) -> Self {
        let mut t = Self {
            path: fs_model::canonical(&path),
            entries: Vec::new(),
            visible: Vec::new(),
            group: GroupBy::None,
            groups: Vec::new(),
            group_for: None,
            selected: HashSet::new(),
            cursor: 0,
            anchor: 0,
            filter: String::new(),
            error: None,
            scroll_to_cursor: true,
            locked: false,
            view,
            depth: Vec::new(),
            expanded: HashSet::new(),
            tree_mtimes: Vec::new(),
            flat: false,
            flat_scan: None,
            flat_truncated: false,
            flat_keep: None,
            tag: None,
            back: Vec::new(),
            forward: Vec::new(),
            dir_mtime: None,
            last_check: Instant::now(),
        };
        t.reload(show_hidden, sort);
        t
    }

    pub fn title(&self) -> String {
        if let Some(t) = &self.tag {
            return format!("Tag: {t}");
        }
        if self.path == Path::new("/") {
            return "/".into();
        }
        if dirs::home_dir().is_some_and(|h| h == self.path) {
            return "Home".into();
        }
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }

    pub fn reload(&mut self, show_hidden: bool, sort: Sort) {
        let keep = self.cursor_entry().map(|e| e.path.clone());
        if let Some(tag) = self.tag.clone() {
            // The same files, read again (who has the tag is up to `set_tag_view`).
            let paths = self.entries.iter().map(|e| e.path.clone()).collect();
            return self.set_tag_view(tag, paths, show_hidden, sort);
        }
        if self.flat {
            // Rescan in the background; files show up as they're found (see `poll_flat`).
            self.flat_keep = keep.or(self.flat_keep.take());
            self.entries.clear();
            self.depth.clear();
            self.visible.clear();
            self.tree_mtimes.clear();
            self.error = None;
            self.flat_truncated = false;
            self.flat_scan = Some(crate::flat::Scan::start(self.path.clone(), show_hidden, None));
            self.dir_mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
            self.selected.retain(|p| p.exists());
            self.refilter(show_hidden);
            return;
        }
        match fs_model::read_dir(&self.path) {
            Ok(mut entries) => {
                fs_model::sort_entries(&mut entries, sort);
                self.error = None;
                self.depth.clear();
                self.tree_mtimes.clear();
                if self.view == ViewMode::Tree {
                    let mut flat = Vec::with_capacity(entries.len());
                    self.flatten(entries, 0, sort, &mut flat);
                    self.entries = flat;
                } else {
                    self.depth = vec![0; entries.len()];
                    self.entries = entries;
                }
            }
            Err(e) => {
                self.entries.clear();
                self.depth.clear();
                self.error = Some(e.to_string());
            }
        }
        // `visible` indexes the old entries; drop it before refiltering.
        self.visible.clear();
        self.dir_mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        self.selected.retain(|p| p.exists());
        self.refilter(show_hidden);
        if let Some(p) = keep {
            self.select_path(&p);
        }
    }

    /// Turn the flat view on or off.
    pub fn set_flat(&mut self, on: bool, show_hidden: bool, sort: Sort) {
        if self.flat != on {
            self.flat = on;
            self.flat_scan = None;
            self.flat_truncated = false;
            self.reload(show_hidden, sort);
        }
    }

    /// Flat view: take in files the scan has found. Returns true if anything changed.
    pub fn poll_flat(&mut self, show_hidden: bool, sort: Sort) -> bool {
        let Some(scan) = &mut self.flat_scan else { return false };
        let mut found = scan.poll();
        let done = scan.done;
        if done {
            self.flat_truncated = scan.truncated;
            self.flat_scan = None;
        }
        if found.is_empty() {
            return done;
        }
        let keep = self.cursor_entry().map(|e| e.path.clone()).or(self.flat_keep.take());
        fs_model::sort_entries(&mut found, sort);
        self.entries = fs_model::merge_sorted(std::mem::take(&mut self.entries), found, sort);
        self.depth = vec![0; self.entries.len()];
        self.settle(keep, show_hidden);
        true
    }

    /// Sort again without re-reading the disk where that's slow (the flat view).
    pub fn resort(&mut self, show_hidden: bool, sort: Sort) {
        if !self.flat {
            return self.reload(show_hidden, sort);
        }
        let keep = self.cursor_entry().map(|e| e.path.clone());
        fs_model::sort_entries(&mut self.entries, sort);
        self.settle(keep, show_hidden);
    }

    /// `entries` changed under `visible`: rebuild it, keeping the cursor on `keep`
    /// without scrolling (the user may be scrolling while files arrive).
    fn settle(&mut self, keep: Option<PathBuf>, show_hidden: bool) {
        let scroll = self.scroll_to_cursor;
        self.visible.clear();
        self.refilter(show_hidden);
        if let Some(p) = keep {
            match self.visible.iter().position(|&i| self.entries[i].path == p) {
                Some(pos) => {
                    self.cursor = pos;
                    self.anchor = pos;
                }
                None => self.flat_keep = Some(p), // not found yet
            }
        }
        self.scroll_to_cursor = scroll;
    }

    /// Tag view: list `paths` (the files tagged `tag`).
    pub fn set_tag_view(&mut self, tag: String, paths: Vec<PathBuf>, show_hidden: bool, sort: Sort) {
        let keep = self.cursor_entry().map(|e| e.path.clone());
        if self.tag.as_ref() != Some(&tag) {
            self.filter.clear();
            self.selected.clear();
            self.cursor = 0;
            self.anchor = 0;
        }
        self.tag = Some(tag);
        self.flat = false;
        self.flat_scan = None;
        self.error = None;
        self.tree_mtimes.clear();
        self.entries = paths.iter().filter_map(|p| Entry::from_path(p)).collect();
        fs_model::sort_entries(&mut self.entries, sort);
        self.depth = vec![0; self.entries.len()];
        self.selected.retain(|p| paths.contains(p));
        self.settle(keep, show_hidden);
        self.flat_keep = None;
    }

    /// Flat and tag views: the folder an item is in, relative to the folder being viewed.
    pub fn rel_folder(&self, e: &Entry) -> String {
        let parent = e.path.parent().unwrap_or(&self.path);
        if self.tag.is_some() {
            // Tagged files are anywhere: show where, from home.
            return match dirs::home_dir().and_then(|h| parent.strip_prefix(h).ok().map(Path::to_path_buf)) {
                Some(r) if r.as_os_str().is_empty() => "~".into(),
                Some(r) => format!("~/{}", r.display()),
                None => parent.display().to_string(),
            };
        }
        match parent.strip_prefix(&self.path) {
            Ok(r) if r.as_os_str().is_empty() => "—".into(),
            Ok(r) => format!("{}/", r.display()),
            Err(_) => parent.display().to_string(),
        }
    }

    /// Add `entries` to `out`, each open folder followed by its contents.
    fn flatten(&mut self, entries: Vec<Entry>, depth: usize, sort: Sort, out: &mut Vec<Entry>) {
        for e in entries {
            let open = e.is_dir && depth < 64 && self.expanded.contains(&e.path);
            let path = e.path.clone();
            out.push(e);
            self.depth.push(depth);
            if open {
                let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                self.tree_mtimes.push((path.clone(), mtime));
                // An unreadable folder just shows nothing inside.
                if let Ok(mut children) = fs_model::read_dir(&path) {
                    fs_model::sort_entries(&mut children, sort);
                    self.flatten(children, depth + 1, sort, out);
                }
            }
        }
    }

    /// Tree view: open or close a folder in place.
    pub fn set_expanded(&mut self, path: &Path, open: bool, show_hidden: bool, sort: Sort) {
        let changed = if open { self.expanded.insert(path.to_path_buf()) } else { self.expanded.remove(path) };
        if changed {
            let cursor = self.cursor_entry().map(|e| e.path.clone());
            self.reload(show_hidden, sort);
            // Closing a folder with the cursor inside it moves the cursor to the folder.
            match cursor {
                Some(c) if !open && c.starts_with(path) => self.select_path(path),
                Some(c) => self.select_path(&c),
                None => {}
            }
        }
    }

    pub fn depth_of(&self, entry: usize) -> usize {
        self.depth.get(entry).copied().unwrap_or(0)
    }

    /// Tree view: the row (index into `visible`) of the folder that contains row `vi`.
    pub fn parent_row(&self, vi: usize) -> Option<usize> {
        let d = self.depth_of(*self.visible.get(vi)?);
        (0..vi).rev().find(|&j| self.depth_of(self.visible[j]) < d)
    }

    /// Cheap auto-refresh: re-read the folder if its mtime changed.
    pub fn check_changed(&mut self, show_hidden: bool, sort: Sort) -> bool {
        // A flat view covers a whole tree; it's rescanned after file operations and Ctrl+R.
        if self.flat || self.tag.is_some() {
            return false;
        }
        if self.last_check.elapsed().as_millis() < 1000 {
            return false;
        }
        self.last_check = Instant::now();
        let m = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        let inner = self.tree_mtimes.iter().any(|(p, t)| std::fs::metadata(p).and_then(|m| m.modified()).ok() != *t);
        if m != self.dir_mtime || inner {
            self.reload(show_hidden, sort);
            return true;
        }
        false
    }

    pub fn refilter(&mut self, show_hidden: bool) {
        // Keep the cursor on the same item if it's still visible afterwards.
        let keep = self.cursor_entry().map(|e| e.path.clone());
        let filter = self.filter.trim();
        if self.view == ViewMode::Tree && self.depth.len() == self.entries.len() {
            self.visible = self.tree_visible(filter, show_hidden);
        } else if filter.is_empty() {
            self.visible = (0..self.entries.len()).filter(|&i| show_hidden || !self.entries[i].is_hidden()).collect();
        } else {
            // With a filter, show best matches first (hidden files included when typed for).
            let mut scored: Vec<(i32, usize)> = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| show_hidden || !e.is_hidden() || filter.starts_with('.'))
                .filter_map(|(i, e)| fuzzy::score(filter, &e.name).map(|s| (s, i)))
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            self.visible = scored.into_iter().map(|(_, i)| i).collect();
        }
        // The tree view keeps its folders' order.
        let by = if self.view == ViewMode::Tree { GroupBy::None } else { self.group };
        self.groups = crate::groups::group(by, &self.entries, &mut self.visible);
        let kept = keep.and_then(|p| self.visible.iter().position(|&i| self.entries[i].path == p));
        self.cursor = kept.unwrap_or(self.cursor).min(self.visible.len().saturating_sub(1));
        self.anchor = self.cursor;
        self.scroll_to_cursor = true;
    }

    /// The item a row up or down from `vi` (`down`), `cols` to a row, rows starting
    /// afresh at each group: the same column where there is one, else the row's last item.
    pub fn row_step(&self, vi: usize, cols: usize, down: bool) -> usize {
        let n = self.visible.len();
        if n == 0 {
            return 0;
        }
        let cols = cols.max(1);
        let mut starts: Vec<usize> = self.groups.iter().map(|g| g.0).collect();
        if starts.first() != Some(&0) {
            starts.insert(0, 0);
        }
        let g = starts.iter().rposition(|&s| s <= vi).unwrap_or(0);
        let (s, e) = (starts[g], starts.get(g + 1).copied().unwrap_or(n));
        let (row, col) = ((vi - s) / cols, (vi - s) % cols);
        if down {
            if s + (row + 1) * cols < e {
                (s + (row + 1) * cols + col).min(e - 1)
            } else if let Some(&ns) = starts.get(g + 1) {
                let ne = starts.get(g + 2).copied().unwrap_or(n);
                (ns + col).min(ne - 1)
            } else {
                n - 1
            }
        } else if row > 0 {
            s + (row - 1) * cols + col
        } else if g > 0 {
            let ps = starts[g - 1];
            let last_row = (s - 1 - ps) / cols;
            (ps + last_row * cols + col).min(s - 1)
        } else {
            vi.saturating_sub(cols)
        }
    }

    /// Tree view rows: in tree order, skipping hidden items and everything inside
    /// them; with a filter, matches plus the folders leading to them.
    fn tree_visible(&self, filter: &str, show_hidden: bool) -> Vec<usize> {
        let mut keep = vec![false; self.entries.len()];
        let mut skip_below: Option<usize> = None;
        for (i, e) in self.entries.iter().enumerate() {
            let d = self.depth[i];
            if skip_below.is_some_and(|s| d > s) {
                continue;
            }
            skip_below = None;
            if !show_hidden && e.is_hidden() && !filter.starts_with('.') {
                skip_below = Some(d);
                continue;
            }
            if filter.is_empty() || fuzzy::score(filter, &e.name).is_some() {
                keep[i] = true;
                // Show the folders a match is in.
                let mut want = d;
                for j in (0..i).rev() {
                    if want == 0 {
                        break;
                    }
                    if self.depth[j] < want {
                        keep[j] = true;
                        want = self.depth[j];
                    }
                }
            }
        }
        (0..self.entries.len()).filter(|&i| keep[i]).collect()
    }

    pub fn navigate(&mut self, path: PathBuf, show_hidden: bool, sort: Sort) {
        let path = fs_model::canonical(&path);
        if path == self.path {
            return;
        }
        let old = std::mem::replace(&mut self.path, path);
        self.back.push(old.clone());
        self.forward.clear();
        self.enter(show_hidden, sort, Some(&old));
    }

    fn enter(&mut self, show_hidden: bool, sort: Sort, came_from: Option<&Path>) {
        // Opening another folder leaves the flat and tag views.
        self.flat = false;
        self.tag = None;
        self.flat_scan = None;
        self.flat_keep = None;
        self.filter.clear();
        self.selected.clear();
        self.cursor = 0;
        self.anchor = 0;
        self.reload(show_hidden, sort);
        // When going up, put the cursor on the folder we came from.
        if let Some(from) = came_from
            && from.parent() == Some(self.path.as_path())
        {
            self.select_path(from);
            self.selected.clear();
        }
    }

    pub fn go_back(&mut self, show_hidden: bool, sort: Sort) {
        if let Some(p) = self.back.pop() {
            let old = std::mem::replace(&mut self.path, p);
            self.forward.push(old.clone());
            self.enter(show_hidden, sort, Some(&old));
        }
    }

    pub fn go_forward(&mut self, show_hidden: bool, sort: Sort) {
        if let Some(p) = self.forward.pop() {
            let old = std::mem::replace(&mut self.path, p);
            self.back.push(old.clone());
            self.enter(show_hidden, sort, Some(&old));
        }
    }

    pub fn go_up(&mut self, show_hidden: bool, sort: Sort) {
        if let Some(parent) = self.path.parent().map(Path::to_path_buf) {
            self.navigate(parent, show_hidden, sort);
        }
    }

    pub fn history(&self) -> impl Iterator<Item = &PathBuf> {
        self.back.iter().rev()
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }
    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn entry_at(&self, visible_idx: usize) -> Option<&Entry> {
        self.visible.get(visible_idx).and_then(|&i| self.entries.get(i))
    }

    pub fn cursor_entry(&self) -> Option<&Entry> {
        self.entry_at(self.cursor)
    }

    /// Moves the cursor onto `path` (if visible) and selects it alone.
    pub fn select_path(&mut self, path: &Path) {
        if let Some(pos) = self.visible.iter().position(|&i| self.entries[i].path == path) {
            self.cursor = pos;
            self.anchor = pos;
            self.selected.clear();
            self.selected.insert(path.to_path_buf());
            self.scroll_to_cursor = true;
        }
    }

    /// Selected paths in display order.
    pub fn targets_selected(&self) -> Vec<PathBuf> {
        self.visible.iter().map(|&i| &self.entries[i].path).filter(|p| self.selected.contains(*p)).cloned().collect()
    }

    /// The paths an action should apply to: the selection, or the cursor item.
    pub fn targets(&self) -> Vec<PathBuf> {
        let mut sel: Vec<PathBuf> = self
            .visible
            .iter()
            .map(|&i| &self.entries[i].path)
            .filter(|p| self.selected.contains(*p))
            .cloned()
            .collect();
        if sel.is_empty() {
            sel.extend(self.cursor_entry().map(|e| e.path.clone()));
        }
        sel
    }

    /// Moves the cursor. With `extend`, selects the range from the anchor.
    pub fn move_cursor(&mut self, to: usize, extend: bool) {
        if self.visible.is_empty() {
            return;
        }
        self.cursor = to.min(self.visible.len() - 1);
        if extend {
            let (a, b) = (self.anchor.min(self.cursor), self.anchor.max(self.cursor));
            self.selected = (a..=b).filter_map(|i| self.entry_at(i)).map(|e| e.path.clone()).collect();
        } else {
            self.anchor = self.cursor;
            self.selected.clear();
            if let Some(e) = self.cursor_entry() {
                self.selected.insert(e.path.clone());
            }
        }
        self.scroll_to_cursor = true;
    }

    pub fn toggle_select(&mut self, idx: usize) {
        if let Some(p) = self.entry_at(idx).map(|e| e.path.clone()) {
            if !self.selected.remove(&p) {
                self.selected.insert(p);
            }
            self.cursor = idx;
            self.anchor = idx;
        }
    }

    pub fn select_all(&mut self) {
        self.selected = self.visible.iter().map(|&i| self.entries[i].path.clone()).collect();
    }
}

pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active: usize,
}

impl Pane {
    pub fn new(path: PathBuf, show_hidden: bool, sort: Sort, view: ViewMode) -> Self {
        Self { tabs: vec![Tab::new(path, show_hidden, sort, view)], active: 0 }
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn close_tab(&mut self, idx: usize) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        self.tabs.remove(idx);
        if self.active >= self.tabs.len() || self.active > idx {
            self.active = self.active.saturating_sub(1);
        }
        true
    }

    /// Remove a tab to move it elsewhere. Unlike `close_tab` this can leave the
    /// pane empty, in which case the caller removes the pane.
    pub fn take_tab(&mut self, idx: usize) -> Tab {
        let t = self.tabs.remove(idx);
        if self.active > idx || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_steps_start_afresh_at_each_group() {
        let root = std::env::temp_dir().join(format!("file-flier-groups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // Two groups of 5 and 3 items, 3 to a row: [0 1 2][3 4] | [5 6 7].
        for f in ["a1", "a2", "a3", "a4", "a5", "b1", "b2", "b3"] {
            std::fs::write(root.join(f), "").unwrap();
        }
        let mut tab = Tab::new(root.clone(), false, Sort::default(), ViewMode::Grid);
        tab.group = GroupBy::Name;
        tab.refilter(false);
        assert_eq!(tab.groups.iter().map(|g| g.0).collect::<Vec<_>>(), [0, 5]);
        assert_eq!(tab.row_step(1, 3, true), 4);
        assert_eq!(tab.row_step(2, 3, true), 4); // no item below: the row's last
        assert_eq!(tab.row_step(4, 3, true), 6); // into the next group, same column
        assert_eq!(tab.row_step(6, 3, false), 4);
        assert_eq!(tab.row_step(7, 3, false), 4); // column 2 has nothing above: the row's last
        assert_eq!(tab.row_step(3, 3, false), 0);
        assert_eq!(tab.row_step(1, 3, false), 0);
        assert_eq!(tab.row_step(6, 3, true), 7);
        // Ungrouped, as before: down by a row, at most to the end.
        tab.group = GroupBy::None;
        tab.refilter(false);
        assert_eq!(tab.row_step(4, 3, true), 7);
        assert_eq!(tab.row_step(6, 3, true), 7);
        assert_eq!(tab.row_step(4, 3, false), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn flat_lists_every_file() {
        let root = std::env::temp_dir().join(format!("file-flier-flat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        for f in ["z.txt", "a/y.txt", "a/b/x.txt", ".git/config"] {
            std::fs::write(root.join(f), "").unwrap();
        }
        let s = Sort::default();
        let mut tab = Tab::new(root.clone(), false, s, ViewMode::Details);
        tab.set_flat(true, false, s);
        let start = Instant::now();
        while tab.flat_scan.is_some() && start.elapsed().as_secs() < 10 {
            tab.poll_flat(false, s);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let names = |t: &Tab| t.visible.iter().map(|&i| t.entries[i].name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&tab), ["x.txt", "y.txt", "z.txt"]);
        assert_eq!(tab.rel_folder(&tab.entries[0]), "a/b/");
        assert_eq!(tab.rel_folder(&tab.entries[2]), "—");
        let folder = Sort { key: crate::fs_model::SortKey::Folder, ..s };
        tab.resort(false, folder);
        assert_eq!(names(&tab), ["z.txt", "y.txt", "x.txt"]);
        tab.navigate(root.join("a"), false, s);
        assert!(!tab.flat);
        assert_eq!(names(&tab), ["b", "y.txt"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tag_view_lists_files_from_anywhere() {
        let root = std::env::temp_dir().join(format!("file-flier-tagview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        let (x, y) = (root.join("a/b/x.txt"), root.join("y.txt"));
        std::fs::write(&x, "").unwrap();
        std::fs::write(&y, "").unwrap();
        let s = Sort::default();
        let mut tab = Tab::new(root.clone(), false, s, ViewMode::Details);
        tab.set_tag_view("Work".into(), vec![x.clone(), y.clone(), root.join("gone.txt")], false, s);
        let names = |t: &Tab| t.visible.iter().map(|&i| t.entries[i].name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&tab), ["x.txt", "y.txt"]);
        assert_eq!(tab.title(), "Tag: Work");
        std::fs::remove_file(&y).unwrap();
        tab.reload(false, s);
        assert_eq!(names(&tab), ["x.txt"]);
        tab.navigate(root.join("a"), false, s);
        assert!(tab.tag.is_none());
        assert_eq!(names(&tab), ["b"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tree_expands_in_place() {
        let root = std::env::temp_dir().join(format!("file-flier-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/inner")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("a/inner/deep.txt"), "").unwrap();
        std::fs::write(root.join(".hidden/x.txt"), "").unwrap();
        std::fs::write(root.join("b.txt"), "").unwrap();
        let s = Sort::default();
        let mut tab = Tab::new(root.clone(), false, s, ViewMode::Tree);
        let names = |t: &Tab| t.visible.iter().map(|&i| t.entries[i].name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&tab), ["a", "b.txt"]);
        tab.set_expanded(&root.join("a"), true, false, s);
        tab.set_expanded(&root.join("a/inner"), true, false, s);
        tab.set_expanded(&root.join(".hidden"), true, false, s);
        assert_eq!(names(&tab), ["a", "inner", "deep.txt", "b.txt"]);
        assert_eq!(tab.parent_row(2), Some(1));
        assert_eq!(tab.parent_row(0), None);
        tab.filter = "deep".into();
        tab.refilter(false);
        assert_eq!(names(&tab), ["a", "inner", "deep.txt"]);
        tab.filter.clear();
        tab.refilter(false);
        tab.select_path(&root.join("a/inner/deep.txt"));
        tab.set_expanded(&root.join("a"), false, false, s);
        assert_eq!(names(&tab), ["a", "b.txt"]);
        assert_eq!(tab.cursor_entry().unwrap().name, "a");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn navigating_to_smaller_folder_keeps_indices_valid() {
        let root = std::env::temp_dir().join(format!("file-flier-pane-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("big")).unwrap();
        std::fs::create_dir_all(root.join("small")).unwrap();
        for i in 0..10 {
            std::fs::write(root.join(format!("big/f{i}")), "").unwrap();
        }
        let mut tab = Tab::new(root.join("big"), false, Sort::default(), ViewMode::Details);
        tab.move_cursor(9, false);
        tab.navigate(root.join("small"), false, Sort::default());
        assert!(tab.cursor_entry().is_none());
        tab.filter = "zzz".into();
        tab.refilter(false);
        tab.go_back(false, Sort::default());
        assert_eq!(tab.visible.len(), 10);
        std::fs::remove_dir_all(root).unwrap();
    }
}
