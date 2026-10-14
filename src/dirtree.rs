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

/// (label, folder) pairs.
type Bookmarks = Vec<(String, PathBuf)>;

#[derive(Default)]
pub struct DirTree {
    cache: Arc<Mutex<HashMap<PathBuf, Listing>>>,
    /// The desktop file manager's own bookmarks, re-read every few seconds.
    bookmarks: Mutex<Option<(Instant, Bookmarks)>>,
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

    /// Folders bookmarked in GNOME Files / Nemo / Thunar (GTK bookmarks) and Dolphin
    /// (KDE places), so the sidebar shows the same places as the system's file manager.
    pub fn system_bookmarks(&self) -> Vec<(String, PathBuf)> {
        let mut cached = self.bookmarks.lock().unwrap();
        if let Some((at, v)) = cached.as_ref()
            && at.elapsed() < Duration::from_secs(5)
        {
            return v.clone();
        }
        // Read from the real home (in Flatpak, XDG_CONFIG_HOME points into the sandbox).
        let v = dirs::home_dir()
            .map(|h| {
                let read = |p: &str| std::fs::read_to_string(h.join(p)).unwrap_or_default();
                let mut v = parse_gtk_bookmarks(&read(".config/gtk-3.0/bookmarks"));
                for b in parse_kde_places(&read(".local/share/user-places.xbel")) {
                    if !v.iter().any(|(_, p)| *p == b.1) {
                        v.push(b);
                    }
                }
                v.retain(|(_, p)| p.is_dir());
                v
            })
            .unwrap_or_default();
        *cached = Some((Instant::now(), v.clone()));
        v
    }

    /// True if `dir` was already read and has no (visible) subfolders. Never starts a read.
    pub fn known_empty(&self, dir: &Path, show_hidden: bool) -> bool {
        let cache = self.cache.lock().unwrap();
        cache.get(dir).and_then(|l| l.dirs.as_ref()).is_some_and(|d| d.iter().all(|p| !show_hidden && is_hidden(p)))
    }
}

/// `file:///path/with%20space Optional label` lines; other schemes are skipped.
pub fn parse_gtk_bookmarks(text: &str) -> Vec<(String, PathBuf)> {
    text.lines()
        .filter_map(|l| {
            let (uri, label) = l.trim().split_once(' ').unwrap_or((l.trim(), ""));
            let path = PathBuf::from(crate::app::percent_decode(uri.strip_prefix("file://")?));
            let label = if label.trim().is_empty() { crate::app::display_name(&path) } else { label.trim().into() };
            Some((label, path))
        })
        .collect()
}

/// Local folders from KDE's `user-places.xbel`, skipping hidden entries and the
/// file-system root (which the sidebar lists under Storage).
pub fn parse_kde_places(xml: &str) -> Vec<(String, PathBuf)> {
    xml.split("<bookmark ")
        .skip(1)
        .filter_map(|b| {
            let b = &b[..b.find("</bookmark>").unwrap_or(b.len())];
            let href = b.split("href=\"").nth(1)?.split('"').next()?;
            let path = PathBuf::from(crate::app::percent_decode(href.strip_prefix("file://")?));
            if b.contains("<IsHidden>true</IsHidden>") || path == Path::new("/") {
                return None;
            }
            let title = b.split("<title>").nth(1).and_then(|t| t.split("</title>").next()).unwrap_or("");
            let title = title.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">");
            let label = if title.trim().is_empty() { crate::app::display_name(&path) } else { title };
            Some((label, path))
        })
        .collect()
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
    fn reads_desktop_bookmarks() {
        let gtk = "file:///home/me/Google%20Drive\nfile:///home/me/Projects Work\nsmb://nas/share\n";
        assert_eq!(
            parse_gtk_bookmarks(gtk),
            [
                ("Google Drive".to_string(), PathBuf::from("/home/me/Google Drive")),
                ("Work".to_string(), PathBuf::from("/home/me/Projects")),
            ]
        );
        let xbel = r#"<xbel>
 <bookmark href="file:///home/me"><title>Home</title></bookmark>
 <bookmark href="remote:/"><title>Network</title></bookmark>
 <bookmark href="file:///"><title>Root</title></bookmark>
 <bookmark href="file:///home/me/Old"><title>Old</title><info><metadata owner="http://www.kde.org"><IsHidden>true</IsHidden></metadata></info></bookmark>
 <bookmark href="file:///home/me/3D%20models"><title>3D &amp; CAD</title></bookmark>
</xbel>"#;
        assert_eq!(
            parse_kde_places(xbel),
            [
                ("Home".to_string(), PathBuf::from("/home/me")),
                ("3D & CAD".to_string(), PathBuf::from("/home/me/3D models")),
            ]
        );
    }

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
