//! The desktop's own icons (the icon theme GNOME uses, e.g. Adwaita or Yaru), looked up
//! as GTK does and drawn as textures. Used when Settings → Icons is "GNOME", so folders
//! and the sidebar look just like they do in GNOME Files; if no icon theme can be
//! found, the hand-drawn icons are used instead.
//!
//! Folders are drawn as the theme has them, or recolored to a color the user gave the
//! folder. Symbolic icons (the sidebar's) are drawn in any color, like GTK does.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once, OnceLock};

use egui::{Color32, ColorImage, Painter, Rect, TextureHandle, TextureOptions, pos2, vec2};

/// Icon names File Flier draws from the theme.
const NAMES: &[&str] = &[
    "folder",
    "folder-documents",
    "folder-download",
    "folder-music",
    "folder-pictures",
    "folder-videos",
    "folder-publicshare",
    "folder-templates",
    "folder-remote",
    "user-home",
    "user-desktop",
    "user-home-symbolic",
    "user-desktop-symbolic",
    "folder-documents-symbolic",
    "folder-download-symbolic",
    "folder-music-symbolic",
    "folder-pictures-symbolic",
    "folder-videos-symbolic",
    "folder-symbolic",
    "folder-remote-symbolic",
    "user-trash-symbolic",
    "user-bookmarks-symbolic",
    "document-open-recent-symbolic",
    "drive-harddisk-symbolic",
    "network-server-symbolic",
    "pan-down-symbolic",
    "pan-end-symbolic",
    "list-add-symbolic",
    "media-eject-symbolic",
    "window-close-symbolic",
    "window-minimize-symbolic",
    "window-maximize-symbolic",
    "window-restore-symbolic",
    "org.gnome.Nautilus",
    "system-file-manager",
];

static ON: AtomicBool = AtomicBool::new(false);

/// Turns the theme's icons on or off (from the settings, once a frame). The theme is
/// indexed in the background the first time; `ctx` is repainted when it's ready.
pub fn set_enabled(on: bool, ctx: &egui::Context) {
    ON.store(on, Ordering::Relaxed);
    // Indexed even when off: native window buttons and the title bar's Files icon use it too.
    static START: Once = Once::new();
    START.call_once(|| {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = INDEX.set(Index::build(&theme_name()));
            ctx.request_repaint();
        });
    });
}

/// Whether the theme's icons are to be drawn: chosen, and found.
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed) && ready()
}

/// Whether the icon theme has been indexed and found.
fn ready() -> bool {
    INDEX.get().is_some_and(|i| i.found)
}

/// Whether an icon theme was found (true while it's still being looked up).
pub fn available() -> bool {
    INDEX.get().is_none_or(|i| i.found)
}

/// The icon theme GNOME uses (`icon-theme` in GNOME's settings), or Adwaita.
pub fn theme_name() -> String {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "icon-theme"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().trim_matches('\'').to_string())
            .filter(|s| !s.is_empty() && !s.contains('/'))
            .unwrap_or_else(|| "Adwaita".to_string())
    })
    .clone()
}

/// The theme's folder icon for special folders (Music, Downloads, …), by path.
pub fn special_folder(path: &Path) -> Option<&'static str> {
    static DIRS: OnceLock<Vec<(PathBuf, &'static str)>> = OnceLock::new();
    let dirs = DIRS.get_or_init(|| {
        let home = dirs::home_dir();
        let mut v: Vec<(PathBuf, &'static str)> = home.iter().map(|h| (h.clone(), "user-home")).collect();
        let others = [
            (dirs::desktop_dir(), "user-desktop"),
            (dirs::document_dir(), "folder-documents"),
            (dirs::download_dir(), "folder-download"),
            (dirs::audio_dir(), "folder-music"),
            (dirs::picture_dir(), "folder-pictures"),
            (dirs::video_dir(), "folder-videos"),
            (dirs::public_dir(), "folder-publicshare"),
            (dirs::template_dir(), "folder-templates"),
        ];
        // Unset XDG folders point at home; home keeps its own icon.
        v.extend(others.into_iter().filter_map(|(d, n)| d.filter(|d| Some(d) != home.as_ref()).map(|d| (d, n))));
        v
    });
    dirs.iter().find(|(d, _)| d == path).map(|(_, n)| *n)
}

// ------------------------------------------------------------ theme lookup

/// How one theme directory's icons are sized (from the theme's `index.theme`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sizing {
    Fixed(u32),
    Scalable(u32, u32),
    Threshold(u32, u32),
}

#[derive(Clone, Debug)]
struct Candidate {
    path: PathBuf,
    sizing: Sizing,
    scale: u32,
    /// Position of its theme in the inheritance chain: earlier themes win.
    theme: usize,
}

static INDEX: OnceLock<Index> = OnceLock::new();

struct Index {
    icons: HashMap<&'static str, Vec<Candidate>>,
    found: bool,
}

/// The directories icon themes are installed in, in lookup order.
fn base_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = dirs::home_dir() {
        v.push(h.join(".icons"));
    }
    if let Some(d) = dirs::data_dir() {
        v.push(d.join("icons"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let data_dirs = if data_dirs.is_empty() { "/usr/local/share:/usr/share".to_string() } else { data_dirs };
    v.extend(std::env::split_paths(&data_dirs).map(|d| d.join("icons")));
    // In the Flatpak, the host's themes.
    v.push(PathBuf::from("/run/host/usr/share/icons"));
    v.push(PathBuf::from("/usr/share/icons"));
    let mut seen = HashSet::new();
    v.retain(|d| seen.insert(d.clone()));
    v
}

/// A parsed `index.theme`: its parents and its directories with their sizing.
#[derive(Default)]
struct ThemeIndex {
    inherits: Vec<String>,
    dirs: Vec<(String, Sizing, u32)>,
}

fn parse_index(text: &str) -> ThemeIndex {
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut cur = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            cur = s.to_string();
        } else if let Some((k, v)) = line.split_once('=') {
            sections.entry(cur.clone()).or_default().insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    let list = |s: Option<&String>| -> Vec<String> {
        s.map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default()
    };
    let head = sections.get("Icon Theme");
    let mut names = list(head.and_then(|h| h.get("Directories")));
    names.extend(list(head.and_then(|h| h.get("ScaledDirectories"))));
    let mut seen = HashSet::new();
    names.retain(|n| seen.insert(n.clone()));
    let dirs = names
        .into_iter()
        .filter_map(|n| {
            let s = sections.get(&n)?;
            let num = |k: &str| s.get(k).and_then(|v| v.parse::<u32>().ok());
            let size = num("Size")?;
            let sizing = match s.get("Type").map(String::as_str) {
                Some("Fixed") => Sizing::Fixed(size),
                Some("Scalable") => Sizing::Scalable(num("MinSize").unwrap_or(size), num("MaxSize").unwrap_or(size)),
                _ => Sizing::Threshold(size, num("Threshold").unwrap_or(2)),
            };
            Some((n, sizing, num("Scale").unwrap_or(1).max(1)))
        })
        .collect();
    ThemeIndex { inherits: list(head.and_then(|h| h.get("Inherits"))), dirs }
}

impl Index {
    fn build(theme: &str) -> Index {
        let bases = base_dirs();
        let read = |name: &str| -> Option<ThemeIndex> {
            bases
                .iter()
                .find_map(|b| std::fs::read_to_string(b.join(name).join("index.theme")).ok())
                .map(|t| parse_index(&t))
        };
        // The theme, then what it inherits (depth first), then Adwaita and hicolor.
        let mut chain: Vec<(String, ThemeIndex)> = Vec::new();
        let mut todo = vec![theme.to_string()];
        let mut seen = HashSet::new();
        while let Some(t) = todo.pop() {
            if !seen.insert(t.clone()) {
                continue;
            }
            let Some(ix) = read(&t) else { continue };
            todo.extend(ix.inherits.iter().rev().filter(|p| p.as_str() != "hicolor").cloned());
            chain.push((t, ix));
        }
        for t in ["Adwaita", "hicolor"] {
            if seen.insert(t.to_string())
                && let Some(ix) = read(t)
            {
                chain.push((t.to_string(), ix));
            }
        }
        let wanted: HashMap<&str, &'static str> = NAMES.iter().map(|n| (*n, *n)).collect();
        let mut icons: HashMap<&'static str, Vec<Candidate>> = HashMap::new();
        for (i, (name, ix)) in chain.iter().enumerate() {
            for base in &bases {
                let root = base.join(name);
                if !root.is_dir() {
                    continue;
                }
                for (dir, sizing, scale) in &ix.dirs {
                    let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
                    for e in rd.flatten() {
                        let path = e.path();
                        let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("");
                        if ext != "svg" && ext != "png" {
                            continue;
                        }
                        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
                        if let Some(&n) = wanted.get(stem) {
                            let c = Candidate { path, sizing: *sizing, scale: *scale, theme: i };
                            icons.entry(n).or_default().push(c);
                        }
                    }
                }
            }
        }
        let found = icons.contains_key("folder");
        Index { icons, found }
    }

    /// The file to draw `name` from at `px` pixels, as GTK picks it: from the first
    /// theme that has the icon, the size that matches, or else the closest one.
    fn lookup(&self, name: &str, px: u32) -> Option<&Candidate> {
        let all = self.icons.get(name)?;
        let first = all.iter().map(|c| c.theme).min()?;
        all.iter().filter(|c| c.theme == first).min_by_key(|c| distance(c.sizing, c.scale, px))
    }
}

/// How far a directory's size is from `px` (0 = a match). Smaller icons count double,
/// as scaling them up blurs them.
fn distance(sizing: Sizing, scale: u32, px: u32) -> u32 {
    let (lo, hi) = match sizing {
        Sizing::Fixed(s) => (s * scale, s * scale),
        Sizing::Scalable(min, max) => (min * scale, max * scale),
        Sizing::Threshold(s, t) => ((s.saturating_sub(t)) * scale, (s + t) * scale),
    };
    if px < lo {
        lo - px
    } else if px > hi {
        (px - hi) * 2
    } else {
        0
    }
}

// ------------------------------------------------------------ drawing

/// Rendered icons by (name, folder color, size in pixels).
type Key = (&'static str, Option<[u8; 3]>, u32);
static TEXTURES: Mutex<Option<HashMap<Key, Option<TextureHandle>>>> = Mutex::new(None);

fn texture(p: &Painter, name: &'static str, kind: Kind, px: u32) -> Option<TextureHandle> {
    let tint = match kind {
        Kind::Folder(Some(c)) => Some([c.r(), c.g(), c.b()]),
        _ => None,
    };
    let key = (name, tint, px);
    let mut g = TEXTURES.lock().ok()?;
    let map = g.get_or_insert_with(HashMap::new);
    if let Some(t) = map.get(&key) {
        return t.clone();
    }
    // Color changes animate through many shades; don't keep them all.
    if map.len() > 600 {
        map.clear();
    }
    let c = INDEX.get()?.lookup(name, px)?;
    let mut rgba = load(&c.path, px)?;
    match kind {
        Kind::Symbolic => mask(&mut rgba),
        Kind::Folder(Some(color)) => recolor(&mut rgba, color),
        Kind::Folder(None) => {}
    }
    let img = ColorImage::from_rgba_unmultiplied([px as usize, px as usize], &rgba);
    let tex = Some(p.ctx().load_texture(name, img, TextureOptions::LINEAR));
    map.insert(key, tex.clone());
    tex
}

#[derive(Clone, Copy)]
enum Kind {
    Folder(Option<Color32>),
    Symbolic,
}

/// Reads an SVG or PNG icon as `px`×`px` straight (not premultiplied) RGBA.
fn load(path: &Path, px: u32) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok()?;
    if path.extension().is_some_and(|e| e == "svg") {
        rasterize(&data, px)
    } else {
        let img = image::load_from_memory(&data).ok()?.to_rgba8();
        let img = if img.dimensions() == (px, px) {
            img
        } else {
            image::imageops::resize(&img, px, px, image::imageops::FilterType::Lanczos3)
        };
        Some(img.into_raw())
    }
}

fn rasterize(svg: &[u8], px: u32) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height());
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let mut data = pixmap.take();
    // tiny-skia's pixels are premultiplied.
    for [r, g, b, a] in data.as_chunks_mut::<4>().0 {
        if *a > 0 && *a < 255 {
            let un = |c: u8| ((c as u32 * 255 + *a as u32 / 2) / *a as u32).min(255) as u8;
            (*r, *g, *b) = (un(*r), un(*g), un(*b));
        }
    }
    Some(data)
}

/// Keeps only a symbolic icon's shape, in white, to be tinted when drawn.
fn mask(rgba: &mut [u8]) {
    for [r, g, b, _] in rgba.as_chunks_mut::<4>().0 {
        (*r, *g, *b) = (255, 255, 255);
    }
}

fn saturation([r, g, b]: [u8; 3]) -> f32 {
    let (max, min) = (r.max(g).max(b) as f32, r.min(g).min(b) as f32);
    if max == 0.0 { 0.0 } else { (max - min) / max }
}

/// Recolors a folder to `color`: its main color becomes `color`, and its other colored
/// shades keep their distance to white. White and grey parts stay as they are.
fn recolor(rgba: &mut [u8], color: Color32) {
    let px = rgba.as_chunks_mut::<4>().0;
    // The folder's main color: its most common solid, colored one (the front sheet).
    let mut counts: HashMap<[u8; 3], ([f32; 3], u32)> = HashMap::new();
    for [r, g, b, a] in px.iter() {
        if *a > 200 && saturation([*r, *g, *b]) > 0.2 {
            let e = counts.entry([*r >> 3, *g >> 3, *b >> 3]).or_default();
            e.0 = [e.0[0] + *r as f32, e.0[1] + *g as f32, e.0[2] + *b as f32];
            e.1 += 1;
        }
    }
    let Some((sum, n)) = counts.into_values().max_by_key(|(_, n)| *n) else { return };
    let main = sum.map(|s| s / n as f32);
    let target = [color.r(), color.g(), color.b()];
    for [r, g, b, _] in px.iter_mut() {
        let s = saturation([*r, *g, *b]);
        if s < 0.12 {
            continue;
        }
        let new = shade(target, main, [*r, *g, *b]);
        // Fade in the recoloring with saturation, so greyish edges blend.
        let k = ((s - 0.12) / 0.1).min(1.0);
        let mix = |o: u8, n: u8| (o as f32 + (n as f32 - o as f32) * k).round() as u8;
        (*r, *g, *b) = (mix(*r, new[0]), mix(*g, new[1]), mix(*b, new[2]));
    }
}

/// Moves `orig` from a folder whose main color is `main` to one in `target`, keeping
/// each channel's distance to white in proportion.
fn shade(target: [u8; 3], main: [f32; 3], orig: [u8; 3]) -> [u8; 3] {
    std::array::from_fn(|i| {
        let t = (255.0 - orig[i] as f32) / (255.0 - main[i]).max(8.0);
        (255.0 - (255.0 - target[i] as f32) * t).clamp(0.0, 255.0).round() as u8
    })
}

/// Draws an icon of side `side`, centered in `r` and on the pixel grid.
fn draw(p: &Painter, r: Rect, side: f32, name: &'static str, kind: Kind, tint: Color32) -> bool {
    let ppp = p.ctx().pixels_per_point();
    let px = (side * ppp).round().clamp(4.0, 1024.0);
    let Some(tex) = texture(p, name, kind, px as u32) else { return false };
    let s = px / ppp;
    let min = ((r.center() - vec2(s, s) / 2.0) * ppp).round() / ppp;
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    p.image(tex.id(), Rect::from_min_size(min, vec2(s, s)), uv, tint);
    true
}

/// Draws the theme's folder (or a special folder, by icon name), in the theme's own
/// colors or recolored to `color`. Returns false when it can't, so the caller draws its own.
pub fn folder(p: &Painter, r: Rect, color: Option<Color32>, special: Option<&'static str>) -> bool {
    if !enabled() {
        return false;
    }
    let side = r.width().min(r.height());
    let name = special.filter(|n| NAMES.contains(n)).unwrap_or("folder");
    draw(p, r, side, name, Kind::Folder(color), Color32::WHITE)
        || (name != "folder" && draw(p, r, side, "folder", Kind::Folder(color), Color32::WHITE))
}

/// Draws a symbolic icon (e.g. "user-home-symbolic") in `color`, at most 16 points big
/// as GTK draws them, when the GNOME icon set is on.
pub fn symbolic(p: &Painter, r: Rect, name: &'static str, color: Color32) -> bool {
    enabled() && symbolic_always(p, r, name, color)
}

/// Like [`symbolic`], whichever icon set is chosen (for the native window buttons).
pub fn symbolic_always(p: &Painter, r: Rect, name: &'static str, color: Color32) -> bool {
    let side = r.width().min(r.height()).min(16.0);
    ready() && draw(p, r, side, name, Kind::Symbolic, color)
}

/// Draws a full-color icon from the theme (e.g. an app's), whichever icon set is chosen.
pub fn icon(p: &Painter, r: Rect, name: &'static str) -> bool {
    let side = r.width().min(r.height());
    ready() && draw(p, r, side, name, Kind::Folder(None), Color32::WHITE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_theme_indexes() {
        let ix = parse_index(
            "[Icon Theme]\nName=Yaru\nInherits=Humanity,hicolor\nDirectories=16x16/places,scalable/places\n\
             ScaledDirectories=16x16@2x/places\n\n[16x16/places]\nSize=16\nType=Fixed\n\n\
             [16x16@2x/places]\nSize=16\nScale=2\nType=Fixed\n\n\
             [scalable/places]\nSize=16\nMinSize=16\nMaxSize=256\nType=Scalable\n",
        );
        assert_eq!(ix.inherits, ["Humanity", "hicolor"]);
        assert_eq!(ix.dirs[0], ("16x16/places".into(), Sizing::Fixed(16), 1));
        assert_eq!(ix.dirs[1], ("scalable/places".into(), Sizing::Scalable(16, 256), 1));
        assert_eq!(ix.dirs[2], ("16x16@2x/places".into(), Sizing::Fixed(16), 2));
    }

    #[test]
    fn picks_the_closest_size() {
        // A 32 px icon: 16@2x matches, 48 is close, 16 would be blurry.
        assert_eq!(distance(Sizing::Fixed(16), 2, 32), 0);
        assert!(distance(Sizing::Fixed(48), 1, 32) < distance(Sizing::Fixed(16), 1, 32));
        assert_eq!(distance(Sizing::Scalable(16, 256), 1, 100), 0);
    }

    #[test]
    fn recoloring_moves_the_main_color() {
        // A purple folder with a darker purple emblem and a white highlight.
        let mut px = vec![];
        for c in [[0x9b, 0x7b, 0xd6], [0x9b, 0x7b, 0xd6], [0x70, 0x50, 0xb0], [255, 255, 255]] {
            px.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
        let green = Color32::from_rgb(0x2e, 0xc2, 0x7e);
        recolor(&mut px, green);
        let main = [px[0], px[1], px[2]];
        assert!(main.iter().zip([0x2e, 0xc2, 0x7e]).all(|(a, b)| a.abs_diff(b) <= 8), "{main:?}");
        // The emblem stays darker than the folder; white stays white.
        assert!(px[9] < px[1]);
        assert_eq!(&px[12..15], &[255, 255, 255]);
    }

    #[test]
    fn masks_keep_only_the_shape() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="0" y="0" width="8" height="16" fill="gray"/></svg>"##;
        let mut px = rasterize(svg, 16).unwrap();
        mask(&mut px);
        assert_eq!(&px[0..4], &[255, 255, 255, 255]);
        assert_eq!(px[15 * 4 + 3], 0);
    }
}
