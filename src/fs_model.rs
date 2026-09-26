//! Directory listing model.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// A regular file (not a pipe, socket or device). Only these are ever read.
    pub is_file: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl Entry {
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().into_owned();
        let lmeta = fs::symlink_metadata(path).ok()?;
        let is_symlink = lmeta.file_type().is_symlink();
        // Follow symlinks for type/size, but fall back to the link itself if it's dangling.
        let meta = if is_symlink { fs::metadata(path).unwrap_or(lmeta) } else { lmeta };
        Some(Self {
            name,
            path: path.to_path_buf(),
            is_dir: meta.is_dir(),
            is_symlink,
            is_file: meta.is_file(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified: meta.modified().ok(),
        })
    }

    pub fn is_hidden(&self) -> bool {
        self.name.starts_with('.')
    }

    pub fn extension(&self) -> String {
        if self.is_dir {
            return String::new();
        }
        Path::new(&self.name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Modified,
    Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Sort {
    pub key: SortKey,
    pub descending: bool,
    pub folders_first: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self { key: SortKey::Name, descending: false, folders_first: true }
    }
}

pub fn read_dir(path: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in fs::read_dir(path)? {
        let Ok(item) = item else { continue };
        if let Some(e) = Entry::from_path(&item.path()) {
            out.push(e);
        }
    }
    Ok(out)
}

/// Natural-ish, case-insensitive name comparison ("file2" < "file10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(ca), Some(cb)) => {
                let ord = ca.to_lowercase().cmp(cb.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

pub fn sort_entries(entries: &mut [Entry], sort: Sort) {
    entries.sort_by(|a, b| {
        // Folders come first (if enabled), regardless of direction.
        let dirs = b.is_dir.cmp(&a.is_dir);
        if sort.folders_first && dirs != Ordering::Equal {
            return dirs;
        }
        let ord = match sort.key {
            SortKey::Name => natural_cmp(&a.name, &b.name),
            SortKey::Size => a.size.cmp(&b.size).then_with(|| natural_cmp(&a.name, &b.name)),
            SortKey::Modified => a.modified.cmp(&b.modified),
            SortKey::Kind => a.extension().cmp(&b.extension()).then_with(|| natural_cmp(&a.name, &b.name)),
        };
        if sort.descending { ord.reverse() } else { ord }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "File2", "file1", "a"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["a", "file1", "File2", "file10"]);
    }
}
