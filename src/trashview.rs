//! Listing, restoring and permanently deleting items in the system trash
//! (the freedesktop trash that GNOME Files, Dolphin and Nemo share).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

pub struct Entry {
    pub item: trash::TrashItem,
    /// Bytes for files; None for folders.
    pub size: Option<u64>,
    /// Number of items for folders.
    pub entries: Option<usize>,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.entries.is_some()
    }
    pub fn name(&self) -> String {
        self.item.name.to_string_lossy().into_owned()
    }
    pub fn original_parent(&self) -> PathBuf {
        self.item.original_parent.clone()
    }
}

/// The dialog's state.
#[derive(Default)]
pub struct TrashView {
    pub items: Vec<Entry>,
    pub loading: Option<Receiver<Result<Vec<Entry>, String>>>,
    pub error: Option<String>,
    /// Selected items, by trash id.
    pub selected: std::collections::HashSet<std::ffi::OsString>,
    pub filter: String,
    pub confirm: Option<Confirm>,
    /// A restore/delete job is running; reload the list when it's done.
    pub reload_when_idle: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    DeleteSelected,
    Empty,
}

impl TrashView {
    pub fn open() -> Self {
        let mut v = TrashView::default();
        v.reload();
        v
    }

    pub fn reload(&mut self) {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(list());
        });
        self.loading = Some(rx);
    }

    /// Picks up a finished background listing.
    pub fn poll(&mut self) {
        if let Some(rx) = &self.loading
            && let Ok(res) = rx.try_recv()
        {
            self.loading = None;
            match res {
                Ok(mut items) => {
                    items.sort_by_key(|e| std::cmp::Reverse(e.item.time_deleted));
                    let ids: std::collections::HashSet<_> = items.iter().map(|e| e.item.id.clone()).collect();
                    self.selected.retain(|id| ids.contains(id));
                    self.items = items;
                    self.error = None;
                }
                Err(e) => self.error = Some(e),
            }
        }
    }

    pub fn selected_items(&self) -> Vec<trash::TrashItem> {
        self.items.iter().filter(|e| self.selected.contains(&e.item.id)).map(|e| e.item.clone()).collect()
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn list() -> Result<Vec<Entry>, String> {
    let items = trash::os_limited::list().map_err(|e| format!("Couldn't read the trash: {e}"))?;
    Ok(items
        .into_iter()
        .map(|item| {
            let (size, entries) = match trash::os_limited::metadata(&item).map(|m| m.size) {
                Ok(trash::TrashItemSize::Bytes(b)) => (Some(b), None),
                Ok(trash::TrashItemSize::Entries(n)) => (None, Some(n)),
                Err(_) => (None, None),
            };
            Entry { item, size, entries }
        })
        .collect())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn list() -> Result<Vec<Entry>, String> {
    Err("Open the Trash in Finder to see what's in it".into())
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn restore(items: Vec<trash::TrashItem>) -> Result<usize, String> {
    let n = items.len();
    trash::os_limited::restore_all(items).map_err(|e| match e {
        trash::Error::RestoreCollision { path, .. } => {
            format!("Couldn't restore: “{}” already exists", path.display())
        }
        e => format!("Couldn't restore: {e}"),
    })?;
    Ok(n)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn purge(items: Vec<trash::TrashItem>) -> Result<usize, String> {
    let n = items.len();
    trash::os_limited::purge_all(items).map_err(|e| format!("Couldn't delete: {e}"))?;
    Ok(n)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn restore(_: Vec<trash::TrashItem>) -> Result<usize, String> {
    Err("Use Put Back in Finder".into())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn purge(_: Vec<trash::TrashItem>) -> Result<usize, String> {
    Err("Empty the Trash from Finder".into())
}
