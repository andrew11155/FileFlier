//! Subfolder listings for the sidebar's folder tree, read on background threads so a
//! slow network or cloud mount never stalls the window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a listing is trusted before it's quietly re-read.
const FRESH: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Listing {
    dirs: Option<Vec<PathBuf>>,
    read_at: Option<Instant>,
    loading: bool,
}

#[derive(Default)]
pub struct DirTree {
    cache: Arc<Mutex<HashMap<PathBuf, Listing>>>,
}

impl DirTree {
    /// Subfolders of `dir` (sorted like the file list), or None while the first read
    /// is still running. Stale listings are refreshed in the background.
    pub fn children(&self, dir: &Path, show_hidden: bool, ctx: &egui::Context) -> Option<Vec<PathBuf>> {
        let mut cache = self.cache.lock().unwrap();
        let l = cache.entry(dir.to_path_buf()).or_default();
        let stale = l.read_at.is_none_or(|t| t.elapsed() > FRESH);
        if stale && !l.loading {
            l.loading = true;
            let (cache, dir, ctx) = (self.cache.clone(), dir.to_path_buf(), ctx.clone());
            std::thread::spawn(move || {
                let dirs = list_dirs(&dir);
                if let Some(l) = cache.lock().unwrap().get_mut(&dir) {
                    if l.dirs.as_ref() != Some(&dirs) {
                        ctx.request_repaint();
                    }
                    l.dirs = Some(dirs);
                    l.read_at = Some(Instant::now());
                    l.loading = false;
                }
            });
        }
        l.dirs.as_ref().map(|d| d.iter().filter(|p| show_hidden || !is_hidden(p)).cloned().collect())
    }

    /// True if `dir` was already read and has no (visible) subfolders. Never starts a read.
    pub fn known_empty(&self, dir: &Path, show_hidden: bool) -> bool {
        let cache = self.cache.lock().unwrap();
        cache.get(dir).and_then(|l| l.dirs.as_ref()).is_some_and(|d| d.iter().all(|p| !show_hidden && is_hidden(p)))
    }
}

fn is_hidden(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

/// Folders (including symlinks to folders) directly inside `dir`, by name, case-insensitively.
pub fn list_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() || (t.is_symlink() && e.path().is_dir())))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_cached_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_folders_sorted() {
        let d = std::env::temp_dir().join(format!("file-flier-dirtree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["b", "A", ".hidden"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        std::fs::write(d.join("file.txt"), "").unwrap();
        let names: Vec<String> =
            list_dirs(&d).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, [".hidden", "A", "b"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
