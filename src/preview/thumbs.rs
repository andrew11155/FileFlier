//! Grid-view thumbnails: a small worker pool plus the shared freedesktop thumbnail
//! cache (`~/.cache/thumbnails`), so pictures made by GNOME Files or Dolphin are
//! reused and ours help them too.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use super::Rgba;

/// Pixel size of the "large" freedesktop thumbnails.
pub const SIZE: u32 = 256;

/// The freedesktop cache sizes File Flier uses, smallest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    /// 256 px ("large"): the regular grid.
    Large,
    /// 512 px ("x-large"): the medium and large grids.
    XLarge,
    /// 1024 px ("xx-large"): the extra-large grid.
    XXLarge,
}

impl Tier {
    const ALL: [Tier; 3] = [Tier::Large, Tier::XLarge, Tier::XXLarge];

    pub fn px(self) -> u32 {
        match self {
            Tier::Large => SIZE,
            Tier::XLarge => 512,
            Tier::XXLarge => 1024,
        }
    }

    fn dir(self) -> &'static str {
        match self {
            Tier::Large => "large",
            Tier::XLarge => "x-large",
            Tier::XXLarge => "xx-large",
        }
    }

    /// Texture memory relative to a 256 px one.
    fn weight(self) -> usize {
        let r = (self.px() / SIZE) as usize;
        r * r
    }
}
const WORKERS: usize = 3;
const MAX_TEXTURES: usize = 800;

struct Job {
    key: ThumbKey,
    modified: Option<SystemTime>,
}

/// A file and the size it's wanted at.
type ThumbKey = (PathBuf, Tier);

/// A finished job; the result is None when it was skipped as no longer visible.
type Done = (ThumbKey, Option<SystemTime>, Option<Option<Rgba>>);

#[derive(Default)]
struct Shared {
    queue: Mutex<Vec<Job>>,
    cv: Condvar,
    done: Mutex<Vec<Done>>,
    /// Frame in which each path was last asked for; stale requests are skipped.
    wanted: Mutex<HashMap<ThumbKey, u64>>,
    frame: AtomicU64,
}

enum State {
    Pending,
    Ready(Option<egui::TextureHandle>),
}

pub struct Thumbs {
    shared: Arc<Shared>,
    cache: HashMap<ThumbKey, (Option<SystemTime>, State, u64)>,
    frame: u64,
}

impl Thumbs {
    pub fn new(ctx: &egui::Context) -> Self {
        let shared = Arc::new(Shared::default());
        for i in 0..WORKERS {
            let shared = shared.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("thumbs-{i}"))
                .stack_size(16 << 20)
                .spawn(move || worker(&shared, &ctx))
                .expect("spawn thumbnail worker");
        }
        Thumbs { shared, cache: HashMap::new(), frame: 0 }
    }

    /// Call once per frame: collects finished thumbnails.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        self.shared.frame.store(self.frame, Ordering::Relaxed);
        let done = std::mem::take(&mut *self.shared.done.lock().unwrap());
        for (key, modified, result) in done {
            match result {
                // Skipped because it scrolled away: forget it so it's requested again later.
                None => {
                    self.cache.remove(&key);
                }
                Some(img) => {
                    let tex = img.map(|i| {
                        ctx.load_texture(
                            format!("thumb:{:?}:{}", key.1, key.0.display()),
                            i.to_color_image(),
                            egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear)),
                        )
                    });
                    self.cache.insert(key, (modified, State::Ready(tex), self.frame));
                }
            }
        }
        // Bigger textures count for more (a 1024 px one is sixteen 256 px ones).
        let weight = |k: &ThumbKey| k.1.weight();
        let total: usize = self.cache.keys().map(weight).sum();
        if total > MAX_TEXTURES {
            let mut ages: Vec<(u64, ThumbKey)> = self.cache.iter().map(|(k, (_, _, f))| (*f, k.clone())).collect();
            ages.sort();
            let mut left = total;
            for (_, k) in ages {
                if left <= MAX_TEXTURES * 3 / 4 {
                    break;
                }
                left -= weight(&k);
                self.cache.remove(&k);
            }
        }
    }

    /// The thumbnail for a file at size `tier`, requesting it if needed.
    /// None = not (yet) available. While a big one is on its way, a smaller
    /// one is returned if it's already loaded.
    pub fn get(&mut self, path: &Path, modified: Option<SystemTime>, tier: Tier) -> Option<egui::TextureHandle> {
        let frame = self.frame;
        let key = (path.to_path_buf(), tier);
        if let Some((m, state, last)) = self.cache.get_mut(&key)
            && *m == modified
        {
            *last = frame;
            return match state {
                State::Ready(t) => t.clone(),
                State::Pending => {
                    self.shared.wanted.lock().unwrap().insert(key, frame);
                    self.ready_small(path, modified, tier)
                }
            };
        }
        self.cache.insert(key.clone(), (modified, State::Pending, frame));
        self.shared.wanted.lock().unwrap().insert(key.clone(), frame);
        self.shared.queue.lock().unwrap().push(Job { key, modified });
        self.shared.cv.notify_one();
        self.ready_small(path, modified, tier)
    }

    /// The biggest loaded thumbnail smaller than `tier`.
    fn ready_small(&self, path: &Path, modified: Option<SystemTime>, tier: Tier) -> Option<egui::TextureHandle> {
        Tier::ALL.iter().rev().filter(|t| **t < tier).find_map(|t| match self.cache.get(&(path.to_path_buf(), *t)) {
            Some((m, State::Ready(Some(tex)), _)) if *m == modified => Some(tex.clone()),
            _ => None,
        })
    }
}

fn worker(shared: &Shared, ctx: &egui::Context) {
    loop {
        let job = {
            let mut q = shared.queue.lock().unwrap();
            loop {
                // Newest first: those are the ones on screen right now.
                if let Some(j) = q.pop() {
                    break j;
                }
                q = shared.cv.wait(q).unwrap();
            }
        };
        let frame = shared.frame.load(Ordering::Relaxed);
        let last_wanted = shared.wanted.lock().unwrap().get(&job.key).copied().unwrap_or(0);
        let (path, tier) = (&job.key.0, job.key.1);
        let result = if last_wanted + 3 < frame {
            None
        } else {
            let img = cached_at(path, job.modified, tier).or_else(|| {
                let img = super::thumbnail(path, tier.px())?;
                store(path, job.modified, &img, tier);
                Some(img)
            });
            Some(img)
        };
        shared.wanted.lock().unwrap().remove(&job.key);
        shared.done.lock().unwrap().push((job.key, job.modified, result));
        ctx.request_repaint();
    }
}

// ------------------------------------------------------------------ freedesktop cache

fn thumb_root() -> Option<PathBuf> {
    // In Flatpak, XDG_CACHE_HOME is private to the sandbox; share the real one.
    if crate::ops::in_flatpak() {
        let host = std::env::var_os("HOST_XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".cache")))?;
        return Some(host.join("thumbnails"));
    }
    Some(dirs::cache_dir()?.join("thumbnails"))
}

/// Percent-encodes a path for a file:// URI the way GLib does.
pub fn uri_encode(path: &str) -> String {
    encode_bytes(path.as_bytes())
}

fn encode_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || b"/-_.~!$&'()*+,;=:@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn uri_of(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    format!("file://{}", encode_bytes(path.as_os_str().as_bytes()))
}

fn cache_file(path: &Path, tier: Tier) -> Option<(PathBuf, String)> {
    use md5::Digest;
    let uri = uri_of(path);
    let name = format!("{}.png", super::hex(&md5::Md5::digest(uri.as_bytes())));
    Some((thumb_root()?.join(tier.dir()).join(name), uri))
}

fn mtime_secs(m: Option<SystemTime>) -> Option<u64> {
    m?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// A valid cached thumbnail of one size (its recorded MTime must match the file's).
fn cached_at(path: &Path, modified: Option<SystemTime>, tier: Tier) -> Option<Rgba> {
    let (file, _) = cache_file(path, tier)?;
    let want = mtime_secs(modified)?;
    let data = std::fs::read(&file).ok()?;
    let decoder = png::Decoder::new(std::io::Cursor::new(&data));
    let reader = decoder.read_info().ok()?;
    let mtime = reader
        .info()
        .uncompressed_latin1_text
        .iter()
        .find(|t| t.keyword == "Thumb::MTime")
        .and_then(|t| t.text.trim().parse::<u64>().ok())?;
    if mtime != want {
        return None;
    }
    let img = image::load_from_memory_with_format(&data, image::ImageFormat::Png).ok()?;
    Some(Rgba::from_image(img, tier.px()))
}

fn store(path: &Path, modified: Option<SystemTime>, img: &Rgba, tier: Tier) {
    let Some(root) = thumb_root() else { return };
    // Never thumbnail the thumbnail cache itself.
    if path.starts_with(&root) {
        return;
    }
    let (Some((file, uri)), Some(mtime)) = (cache_file(path, tier), mtime_secs(modified)) else { return };
    let dir = file.parent().unwrap();
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, img.w, img.h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let ok = enc.add_text_chunk("Thumb::URI".into(), uri).is_ok()
            && enc.add_text_chunk("Thumb::MTime".into(), mtime.to_string()).is_ok()
            && enc.add_text_chunk("Software".into(), "File Flier".into()).is_ok();
        if !ok {
            return;
        }
        let Ok(mut w) = enc.write_header() else { return };
        if w.write_image_data(&img.px).is_err() {
            return;
        }
    }
    // Write atomically with private permissions, as the spec asks.
    let tmp = file.with_extension(format!("{}.tmp", std::process::id()));
    let written = (|| {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        f.write_all(&buf)?;
        std::fs::rename(&tmp, &file)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_their_own_cache_folders() {
        let p = Path::new("/tmp/photo.jpg");
        let dirs: Vec<PathBuf> = Tier::ALL.iter().map(|t| cache_file(p, *t).unwrap().0).collect();
        assert!(dirs[0].parent().unwrap().ends_with("thumbnails/large"));
        assert!(dirs[1].parent().unwrap().ends_with("thumbnails/x-large"));
        assert!(dirs[2].parent().unwrap().ends_with("thumbnails/xx-large"));
        assert!(dirs.iter().all(|d| d.file_name() == dirs[0].file_name()));
        assert_eq!(Tier::ALL.map(Tier::px), [256, 512, 1024]);
        assert_eq!(Tier::XXLarge.weight(), 16);
    }

    #[test]
    fn uris_match_glib() {
        assert_eq!(uri_of(Path::new("/home/me/My Photo #1.jpg")), "file:///home/me/My%20Photo%20%231.jpg");
        assert_eq!(uri_of(Path::new("/tmp/é")), "file:///tmp/%C3%A9");
    }
}
