//! File ratings and tags (the Ratings and Tags extensions).
//!
//! Both are stored on the file itself as extended attributes, with the keys KDE's
//! Dolphin uses (`user.baloo.rating`, `user.xdg.tags`), so they follow a file that's
//! moved or renamed in any app. File Flier also keeps an index of every rated or
//! tagged file in `~/.config/file-flier/meta.json`: the sidebar's tag list is built
//! from it (no disk scan), and it holds tags for drives that can't store
//! attributes (FAT/exFAT USB sticks, some network shares). Where a drive does
//! support attributes they win, so a tag removed in another app is gone here too.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Ratings known so far, for sorting (which can't reach `Meta`): from the index,
/// from attributes read while sorting, and kept up to date as files are rated.
static RATINGS: Mutex<Option<HashMap<PathBuf, u8>>> = Mutex::new(None);

fn remember_rating(p: &Path, stars: u8) {
    if let Ok(mut g) = RATINGS.lock() {
        g.get_or_insert_with(HashMap::new).insert(p.to_path_buf(), stars);
    }
}

/// A file's rating for sorting by rating. Each file's attribute is read once.
pub fn sort_rating(p: &Path) -> u8 {
    if let Ok(g) = RATINGS.lock()
        && let Some(r) = g.as_ref().and_then(|m| m.get(p))
    {
        return *r;
    }
    let r = xattr::get(p, RATING_ATTR).ok().flatten().map_or(0, |v| parse_rating(&v));
    remember_rating(p, r);
    r
}

/// Baloo stores 0-10, two per star.
fn parse_rating(v: &[u8]) -> u8 {
    std::str::from_utf8(v).ok().and_then(|s| s.trim().parse::<u8>().ok()).map_or(0, |r| r.div_ceil(2).min(5))
}

const RATING_ATTR: &str = "user.baloo.rating";
const TAGS_ATTR: &str = "user.xdg.tags";

/// Colors new tags get, in turn.
pub const TAG_COLORS: [[u8; 3]; 8] = [
    [0x35, 0x84, 0xE4],
    [0x2E, 0xC2, 0x7E],
    [0xF6, 0xD3, 0x2D],
    [0xFF, 0x78, 0x00],
    [0xE0, 0x1B, 0x24],
    [0x91, 0x41, 0xAC],
    [0x98, 0x6A, 0x44],
    [0x5E, 0x5C, 0x64],
];

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TagDef {
    pub name: String,
    pub color: [u8; 3],
}

impl TagDef {
    pub fn color32(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.color[0], self.color[1], self.color[2])
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FileMeta {
    /// Stars, 0 (none) to 5.
    #[serde(skip_serializing_if = "is_zero")]
    pub rating: u8,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Couldn't be written to the file (read-only file, or no attribute support):
    /// only the index has it, so what the file says doesn't count.
    #[serde(skip_serializing_if = "is_false")]
    pub index_only: bool,
}

fn is_zero(n: &u8) -> bool {
    *n == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl FileMeta {
    fn is_empty(&self) -> bool {
        self.rating == 0 && self.tags.is_empty()
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Meta {
    /// Every tag, in the order the sidebar shows them.
    pub tags: Vec<TagDef>,
    /// Rated or tagged files.
    pub files: BTreeMap<PathBuf, FileMeta>,
    /// Where the index is saved (None = not saved, in tests).
    #[serde(skip)]
    file: Option<PathBuf>,
    /// Files whose attributes have been read this session.
    #[serde(skip)]
    seen: HashSet<PathBuf>,
}

/// Why a tag name was refused.
pub fn check_tag_name(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        Err("Tags need a name")
    } else if name.contains(',') {
        Err("Tag names can't contain commas")
    } else {
        Ok(())
    }
}

impl Meta {
    pub fn load() -> Self {
        let file = dirs::config_dir().map(|d| d.join("file-flier").join("meta.json"));
        let mut m: Meta = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        m.file = file;
        for (p, f) in &m.files {
            remember_rating(p, f.rating);
        }
        m
    }

    fn save(&self) {
        let Some(f) = &self.file else { return };
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            // Write then rename, so a crash never leaves half a file.
            let tmp = f.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, f);
            }
        }
    }

    pub fn tag(&self, name: &str) -> Option<&TagDef> {
        self.tags.iter().find(|t| t.name == name)
    }

    pub fn rating(&self, p: &Path) -> u8 {
        self.files.get(p).map_or(0, |m| m.rating)
    }

    pub fn tags_of(&self, p: &Path) -> &[String] {
        self.files.get(p).map_or(&[], |m| &m.tags)
    }

    /// Read `p`'s attributes the first time it's shown, picking up ratings and
    /// tags set in other apps (or removed there).
    pub fn observe(&mut self, p: &Path) {
        if self.seen.contains(p) {
            return;
        }
        self.seen.insert(p.to_path_buf());
        if self.files.get(p).is_some_and(|m| m.index_only) {
            return;
        }
        // Network and phone mounts (gvfs) can be slow to ask; they keep what the index says.
        if p.starts_with("/run/user") {
            return;
        }
        let (Ok(rating), Ok(tags)) = (xattr::get(p, RATING_ATTR), xattr::get(p, TAGS_ATTR)) else {
            return; // no attribute support here: the index is all there is
        };
        let found = FileMeta {
            rating: rating.map_or(0, |v| parse_rating(&v)),
            tags: tags.map(|v| split_tags(&String::from_utf8_lossy(&v))).unwrap_or_default(),
            index_only: false,
        };
        let changed = self.files.get(p).cloned().unwrap_or_default() != found;
        for t in &found.tags {
            self.ensure_tag(t);
        }
        if changed {
            self.put(p, found);
            self.save();
        }
    }

    /// Forget which files have been read, so they're read again (Ctrl+R).
    pub fn reread(&mut self) {
        self.seen.clear();
        if let Ok(mut g) = RATINGS.lock() {
            *g = Some(self.files.iter().map(|(p, f)| (p.clone(), f.rating)).collect());
        }
    }

    fn put(&mut self, p: &Path, m: FileMeta) {
        remember_rating(p, m.rating);
        if m.is_empty() {
            self.files.remove(p);
        } else {
            self.files.insert(p.to_path_buf(), m);
        }
    }

    /// Store `m` on the file (where the drive allows) and in the index.
    fn write(&mut self, p: &Path, mut m: FileMeta) {
        let rating = (m.rating > 0).then(|| (m.rating.min(5) * 2).to_string());
        let tags = (!m.tags.is_empty()).then(|| m.tags.join(","));
        let ok =
            xattr::set(p, RATING_ATTR, rating.as_deref()).is_ok() & xattr::set(p, TAGS_ATTR, tags.as_deref()).is_ok();
        m.index_only = !ok;
        self.seen.insert(p.to_path_buf());
        self.put(p, m);
    }

    /// Give `paths` a rating of `stars` (0 clears it).
    pub fn set_rating(&mut self, paths: &[PathBuf], stars: u8) {
        for p in paths {
            let mut m = self.files.get(p).cloned().unwrap_or_default();
            m.rating = stars.min(5);
            self.write(p, m);
        }
        self.save();
    }

    /// Add `tag` to `paths`, or take it off them.
    pub fn set_tag(&mut self, paths: &[PathBuf], tag: &str, on: bool) {
        self.ensure_tag(tag);
        for p in paths {
            let mut m = self.files.get(p).cloned().unwrap_or_default();
            let has = m.tags.iter().any(|t| t == tag);
            if on && !has {
                m.tags.push(tag.to_string());
            } else if !on && has {
                m.tags.retain(|t| t != tag);
            } else {
                continue;
            }
            self.write(p, m);
        }
        self.save();
    }

    /// Make sure there's a tag called `name` (new ones get the next color).
    fn ensure_tag(&mut self, name: &str) {
        if self.tag(name).is_none() {
            let color = TAG_COLORS[self.tags.len() % TAG_COLORS.len()];
            self.tags.push(TagDef { name: name.to_string(), color });
        }
    }

    pub fn add_tag(&mut self, name: &str, color: [u8; 3]) -> Result<(), &'static str> {
        check_tag_name(name)?;
        let name = name.trim();
        if self.tag(name).is_some() {
            return Err("There's already a tag with that name");
        }
        self.tags.push(TagDef { name: name.to_string(), color });
        self.save();
        Ok(())
    }

    pub fn set_color(&mut self, name: &str, color: [u8; 3]) {
        if let Some(t) = self.tags.iter_mut().find(|t| t.name == name) {
            t.color = color;
            self.save();
        }
    }

    /// Rename a tag everywhere it's used.
    pub fn rename_tag(&mut self, old: &str, new: &str) -> Result<(), &'static str> {
        check_tag_name(new)?;
        let new = new.trim();
        if new == old {
            return Ok(());
        }
        if self.tag(new).is_some() {
            return Err("There's already a tag with that name");
        }
        let Some(t) = self.tags.iter_mut().find(|t| t.name == old) else { return Ok(()) };
        t.name = new.to_string();
        for p in self.files_with(old) {
            let mut m = self.files[&p].clone();
            for t in &mut m.tags {
                if t == old {
                    *t = new.to_string();
                }
            }
            self.write(&p, m);
        }
        self.save();
        Ok(())
    }

    /// Delete a tag and take it off every file.
    pub fn delete_tag(&mut self, name: &str) {
        let files = self.files_with(name);
        self.set_tag(&files, name, false);
        self.tags.retain(|t| t.name != name);
        self.save();
    }

    /// Files with `tag` (see `prune` for dropping ones that are gone).
    pub fn files_with(&self, tag: &str) -> Vec<PathBuf> {
        self.files.iter().filter(|(_, m)| m.tags.iter().any(|t| t == tag)).map(|(p, _)| p.clone()).collect()
    }

    /// Drop files that no longer exist from the index.
    pub fn prune(&mut self) {
        let before = self.files.len();
        self.files.retain(|p, _| p.symlink_metadata().is_ok());
        if self.files.len() != before {
            self.save();
        }
    }

    pub fn count(&self, tag: &str) -> usize {
        self.files.values().filter(|m| m.tags.iter().any(|t| t == tag)).count()
    }

    /// `from` (or something inside it) was moved or renamed in File Flier.
    pub fn moved(&mut self, from: &Path, to: &Path) {
        let keys: Vec<PathBuf> = self.files.keys().filter(|k| k.starts_with(from)).cloned().collect();
        if keys.is_empty() {
            return;
        }
        for k in keys {
            if let (Some(m), Ok(rest)) = (self.files.remove(&k), k.strip_prefix(from)) {
                let dest = if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) };
                // A move to another drive is a copy, which leaves the attributes behind.
                self.write(&dest, m);
            }
        }
        self.save();
    }
}

fn split_tags(s: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for t in s.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        if !v.iter().any(|x| x == t) {
            v.push(t.to_string());
        }
    }
    v
}

/// Extended attributes through libc (Linux and macOS).
mod xattr {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    fn c(p: &Path) -> Option<CString> {
        CString::new(p.as_os_str().as_bytes()).ok()
    }

    #[cfg(target_os = "linux")]
    const MISSING: i32 = libc::ENODATA;
    #[cfg(target_os = "macos")]
    const MISSING: i32 = libc::ENOATTR;

    fn errno() -> i32 {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }

    /// Ok(None) = the drive supports attributes but this one isn't set;
    /// Err = no attribute support (or the file can't be read).
    pub fn get(p: &Path, name: &str) -> Result<Option<Vec<u8>>, ()> {
        let (Some(path), Ok(name)) = (c(p), CString::new(name)) else { return Err(()) };
        let mut buf = vec![0u8; 1024];
        // SAFETY: valid C strings and a buffer of the length passed.
        #[cfg(target_os = "linux")]
        let n = unsafe { libc::getxattr(path.as_ptr(), name.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
        #[cfg(target_os = "macos")]
        let n = unsafe { libc::getxattr(path.as_ptr(), name.as_ptr(), buf.as_mut_ptr().cast(), buf.len(), 0, 0) };
        if n >= 0 {
            buf.truncate(n as usize);
            return Ok(Some(buf));
        }
        if errno() == MISSING { Ok(None) } else { Err(()) }
    }

    /// Set the attribute, or remove it when `value` is None.
    pub fn set(p: &Path, name: &str, value: Option<&str>) -> Result<(), ()> {
        let (Some(path), Ok(name)) = (c(p), CString::new(name)) else { return Err(()) };
        // SAFETY: valid C strings and a buffer of the length passed.
        let r = unsafe {
            match value {
                #[cfg(target_os = "linux")]
                Some(v) => libc::setxattr(path.as_ptr(), name.as_ptr(), v.as_ptr().cast(), v.len(), 0),
                #[cfg(target_os = "macos")]
                Some(v) => libc::setxattr(path.as_ptr(), name.as_ptr(), v.as_ptr().cast(), v.len(), 0, 0),
                #[cfg(target_os = "linux")]
                None => libc::removexattr(path.as_ptr(), name.as_ptr()),
                #[cfg(target_os = "macos")]
                None => libc::removexattr(path.as_ptr(), name.as_ptr(), 0),
            }
        };
        if r == 0 || (value.is_none() && errno() == MISSING) { Ok(()) } else { Err(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("file-flier-meta-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rates_tags_renames_and_moves() {
        let d = temp("a");
        let (a, b) = (d.join("a.txt"), d.join("b.txt"));
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let mut m = Meta::default();
        m.set_rating(std::slice::from_ref(&a), 4);
        m.set_tag(&[a.clone(), b.clone()], "Work", true);
        m.set_tag(std::slice::from_ref(&b), "Home", true);
        assert_eq!(m.rating(&a), 4);
        assert_eq!(m.count("Work"), 2);
        assert_eq!(m.tags_of(&b), ["Work", "Home"]);
        assert_eq!(m.tags.len(), 2);

        m.rename_tag("Work", "Job").unwrap();
        assert_eq!(m.tags_of(&a), ["Job"]);
        assert!(m.rename_tag("Job", "Home").is_err());
        assert!(m.add_tag("a,b", [0; 3]).is_err());

        let c = d.join("c.txt");
        std::fs::rename(&a, &c).unwrap();
        m.moved(&a, &c);
        assert_eq!(m.rating(&c), 4);
        assert_eq!(m.rating(&a), 0);

        m.delete_tag("Job");
        assert!(m.tags_of(&c).is_empty());
        assert_eq!(m.tags_of(&b), ["Home"]);
        // Clearing the last thing about a file drops it from the index.
        m.set_rating(std::slice::from_ref(&c), 0);
        assert!(!m.files.contains_key(&c));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn sorts_by_rating() {
        use crate::fs_model::{Entry, Sort, SortKey, sort_entries};
        let d = temp("d");
        let names = ["a.txt", "b.txt", "c.txt"];
        for n in names {
            std::fs::write(d.join(n), "").unwrap();
        }
        let mut m = Meta::default();
        m.set_rating(&[d.join("b.txt")], 5);
        m.set_rating(&[d.join("c.txt")], 2);
        let mut entries: Vec<Entry> = names.iter().filter_map(|n| Entry::from_path(&d.join(n))).collect();
        let sort = Sort { key: SortKey::Rating, descending: true, ..Sort::default() };
        sort_entries(&mut entries, sort);
        let got: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(got, ["b.txt", "c.txt", "a.txt"]);
        // Rated in another app (attribute only) still sorts in.
        if xattr::set(&d.join("a.txt"), RATING_ATTR, Some("8")).is_ok() {
            RATINGS.lock().unwrap().as_mut().unwrap().remove(&d.join("a.txt"));
            sort_entries(&mut entries, sort);
            assert_eq!(entries[1].name, "a.txt");
        }
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn read_only_files_keep_tags_in_the_index() {
        use std::os::unix::fs::PermissionsExt;
        let d = temp("c");
        let f = d.join("locked.txt");
        std::fs::write(&f, "").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o444)).unwrap();
        let mut m = Meta::default();
        m.set_tag(std::slice::from_ref(&f), "Keep", true);
        m.reread();
        m.observe(&f);
        assert_eq!(m.tags_of(&f), ["Keep"]);
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn reads_attributes_set_elsewhere() {
        let d = temp("b");
        let f = d.join("photo.jpg");
        std::fs::write(&f, "").unwrap();
        // Skip where the temp folder has no user attributes (older tmpfs).
        if xattr::set(&f, TAGS_ATTR, Some("Trip, Family,Trip")).is_err() {
            return;
        }
        xattr::set(&f, RATING_ATTR, Some("7")).unwrap();
        let mut m = Meta::default();
        m.observe(&f);
        assert_eq!(m.tags_of(&f), ["Trip", "Family"]);
        assert_eq!(m.rating(&f), 4);
        assert!(m.tag("Family").is_some());
        // Removed in another app: gone here too once it's read again.
        xattr::set(&f, TAGS_ATTR, None).unwrap();
        m.reread();
        m.observe(&f);
        assert!(m.tags_of(&f).is_empty());
        assert_eq!(m.count("Trip"), 0);
        std::fs::remove_dir_all(d).unwrap();
    }
}
