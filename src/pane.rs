//! A pane holds a set of tabs; each tab browses one folder.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use crate::config::ViewMode;
use crate::fs_model::{self, Entry, Sort};
use crate::fuzzy;

pub struct Tab {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    /// Indices into `entries` that pass the hidden-file and quick-filter checks.
    pub visible: Vec<usize>,
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
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    dir_mtime: Option<SystemTime>,
    last_check: Instant,
}

impl Tab {
    pub fn new(path: PathBuf, show_hidden: bool, sort: Sort, view: ViewMode) -> Self {
        let mut t = Self {
            path,
            entries: Vec::new(),
            visible: Vec::new(),
            selected: HashSet::new(),
            cursor: 0,
            anchor: 0,
            filter: String::new(),
            error: None,
            scroll_to_cursor: true,
            locked: false,
            view,
            back: Vec::new(),
            forward: Vec::new(),
            dir_mtime: None,
            last_check: Instant::now(),
        };
        t.reload(show_hidden, sort);
        t
    }

    pub fn title(&self) -> String {
        if self.path == Path::new("/") {
            return "/".into();
        }
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }

    pub fn reload(&mut self, show_hidden: bool, sort: Sort) {
        let keep = self.cursor_entry().map(|e| e.path.clone());
        match fs_model::read_dir(&self.path) {
            Ok(mut entries) => {
                fs_model::sort_entries(&mut entries, sort);
                self.entries = entries;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
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

    /// Cheap auto-refresh: re-read the folder if its mtime changed.
    pub fn check_changed(&mut self, show_hidden: bool, sort: Sort) -> bool {
        if self.last_check.elapsed().as_millis() < 1000 {
            return false;
        }
        self.last_check = Instant::now();
        let m = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if m != self.dir_mtime {
            self.reload(show_hidden, sort);
            return true;
        }
        false
    }

    pub fn refilter(&mut self, show_hidden: bool) {
        // Keep the cursor on the same item if it's still visible afterwards.
        let keep = self.cursor_entry().map(|e| e.path.clone());
        let filter = self.filter.trim();
        if filter.is_empty() {
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
        let kept = keep.and_then(|p| self.visible.iter().position(|&i| self.entries[i].path == p));
        self.cursor = kept.unwrap_or(self.cursor).min(self.visible.len().saturating_sub(1));
        self.anchor = self.cursor;
        self.scroll_to_cursor = true;
    }

    pub fn navigate(&mut self, path: PathBuf, show_hidden: bool, sort: Sort) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if path == self.path {
            return;
        }
        let old = std::mem::replace(&mut self.path, path);
        self.back.push(old.clone());
        self.forward.clear();
        self.enter(show_hidden, sort, Some(&old));
    }

    fn enter(&mut self, show_hidden: bool, sort: Sort, came_from: Option<&Path>) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
