//! GNOME's own icons (the Adwaita icon theme, by the GNOME Project), read from the icon
//! theme installed on this system and drawn as textures. Used when Settings → Icons is
//! "GNOME"; if the theme isn't installed, the hand-drawn icons are used instead.
//!
//! Folders are recolored to the folder color in use (the accent in the GNOME themes, or
//! a color the user gave the folder). Symbolic icons (the sidebar's) are drawn in any
//! color, like GTK does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use egui::{Color32, ColorImage, Painter, Rect, TextureHandle, TextureOptions, pos2, vec2};

static ON: AtomicBool = AtomicBool::new(false);

/// Turns the GNOME icons on or off (from the settings, once a frame).
pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// Whether the GNOME icons are to be drawn: chosen, and installed.
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed) && available()
}

/// Whether the Adwaita icon theme is installed.
pub fn available() -> bool {
    theme_dir().is_some()
}

fn theme_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let mut roots: Vec<PathBuf> = dirs::data_dir().into_iter().map(|d| d.join("icons")).collect();
        let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
        let data_dirs = if data_dirs.is_empty() { "/usr/local/share:/usr/share".to_string() } else { data_dirs };
        roots.extend(std::env::split_paths(&data_dirs).map(|d| d.join("icons")));
        // In the Flatpak, the host's icons (the runtime may not have Adwaita).
        roots.push(PathBuf::from("/run/host/usr/share/icons"));
        roots.push(PathBuf::from("/run/host/share/icons"));
        roots.push(PathBuf::from("/usr/share/icons"));
        roots.into_iter().map(|r| r.join("Adwaita")).find(|d| d.join("scalable/places/folder.svg").is_file())
    })
    .as_deref()
}

/// The SVG source of an icon, by its path in the theme (read once).
fn source(rel: &'static str) -> Option<Arc<str>> {
    type Sources = HashMap<&'static str, Option<Arc<str>>>;
    static SOURCES: Mutex<Option<Sources>> = Mutex::new(None);
    let mut g = SOURCES.lock().ok()?;
    g.get_or_insert_with(HashMap::new)
        .entry(rel)
        .or_insert_with(|| std::fs::read_to_string(theme_dir()?.join(rel)).ok().map(Arc::from))
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

/// Adwaita draws its folders in shades of this blue (GNOME's default accent). Each shade
/// keeps its distance to white when the folder is recolored.
const REF: [u8; 3] = [0x35, 0x84, 0xe4];
const FOLDER_SHADES: [&str; 5] = ["438de6", "62a0ea", "a4caee", "afd4ff", "c0d5ea"];

fn shade(base: Color32, orig: [u8; 3]) -> [u8; 3] {
    let b = [base.r(), base.g(), base.b()];
    std::array::from_fn(|i| {
        let room = 255.0 - REF[i] as f32;
        let t = (255.0 - orig[i] as f32) / room;
        (255.0 - (255.0 - b[i] as f32) * t).clamp(0.0, 255.0).round() as u8
    })
}

fn recolor(svg: &str, base: Color32) -> String {
    let mut s = svg.to_string();
    for hex in FOLDER_SHADES {
        let v = u32::from_str_radix(hex, 16).unwrap_or(0);
        let [r, g, b] = shade(base, [(v >> 16) as u8, (v >> 8) as u8, v as u8]);
        let new = format!("{r:02x}{g:02x}{b:02x}");
        s = s
            .replace(&format!("#{hex}"), &format!("#{new}"))
            .replace(&format!("#{}", hex.to_uppercase()), &format!("#{new}"));
    }
    s
}

/// Rendered icons by (theme path, folder color, size in pixels).
type Key = (&'static str, Option<[u8; 3]>, u32);
static TEXTURES: Mutex<Option<HashMap<Key, Option<TextureHandle>>>> = Mutex::new(None);

fn texture(p: &Painter, rel: &'static str, folder: Option<Color32>, px: u32) -> Option<TextureHandle> {
    let key = (rel, folder.map(|c| [c.r(), c.g(), c.b()]), px);
    let mut g = TEXTURES.lock().ok()?;
    let map = g.get_or_insert_with(HashMap::new);
    if let Some(t) = map.get(&key) {
        return t.clone();
    }
    // Color changes animate through many shades; don't keep them all.
    if map.len() > 600 {
        map.clear();
    }
    let svg = source(rel)?;
    let svg = match folder {
        Some(c) => recolor(&svg, c),
        None => svg.to_string(),
    };
    let tex = rasterize(&svg, px, folder.is_none()).map(|img| p.ctx().load_texture(rel, img, TextureOptions::LINEAR));
    map.insert(key, tex.clone());
    tex
}

/// Renders an SVG at `px`×`px`. A `mask` keeps only its shape, in white, to be tinted.
fn rasterize(svg: &str, px: u32, mask: bool) -> Option<ColorImage> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height());
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let mut data = pixmap.take();
    if mask {
        for [r, g, b, a] in data.as_chunks_mut::<4>().0 {
            (*r, *g, *b) = (*a, *a, *a);
        }
    }
    Some(ColorImage::from_rgba_premultiplied([px as usize, px as usize], &data))
}

/// Draws an icon of side `side`, centered in `r` and on the pixel grid.
fn draw(p: &Painter, r: Rect, side: f32, rel: &'static str, folder: Option<Color32>, tint: Color32) -> bool {
    let ppp = p.ctx().pixels_per_point();
    let px = (side * ppp).round().clamp(4.0, 1024.0);
    let Some(tex) = texture(p, rel, folder, px as u32) else { return false };
    let s = px / ppp;
    let min = ((r.center() - vec2(s, s) / 2.0) * ppp).round() / ppp;
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    p.image(tex.id(), Rect::from_min_size(min, vec2(s, s)), uv, tint);
    true
}

fn folder_path(name: &str) -> &'static str {
    match name {
        "user-home" => "scalable/places/user-home.svg",
        "user-desktop" => "scalable/places/user-desktop.svg",
        "folder-documents" => "scalable/places/folder-documents.svg",
        "folder-download" => "scalable/places/folder-download.svg",
        "folder-music" => "scalable/places/folder-music.svg",
        "folder-pictures" => "scalable/places/folder-pictures.svg",
        "folder-videos" => "scalable/places/folder-videos.svg",
        "folder-publicshare" => "scalable/places/folder-publicshare.svg",
        "folder-templates" => "scalable/places/folder-templates.svg",
        "folder-remote" => "scalable/places/folder-remote.svg",
        _ => "scalable/places/folder.svg",
    }
}

/// Draws Adwaita's folder (or a special folder, by icon name) in `color`'s shades.
/// Returns false when it can't, so the caller draws its own.
pub fn folder(p: &Painter, r: Rect, color: Color32, special: Option<&str>) -> bool {
    let side = r.width().min(r.height());
    enabled() && draw(p, r, side, folder_path(special.unwrap_or("folder")), Some(color), Color32::WHITE)
}

/// Draws a symbolic icon (`symbolic/<dir>/<name>-symbolic.svg`) in `color`.
pub fn symbolic(p: &Painter, r: Rect, rel: &'static str, color: Color32) -> bool {
    // GTK draws symbolic icons at 16 px; keep them at most that big.
    let side = r.width().min(r.height()).min(16.0);
    enabled() && draw(p, r, side, rel, None, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recoloring_keeps_the_shades() {
        let blue = Color32::from_rgb(REF[0], REF[1], REF[2]);
        // In GNOME's own blue, the folder looks just like the theme's.
        for hex in FOLDER_SHADES {
            let v = u32::from_str_radix(hex, 16).unwrap();
            let o = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
            assert_eq!(shade(blue, o), o);
        }
        let svg = r##"<path fill="#438de6"/><path fill="#a4caee"/>"##;
        assert_eq!(recolor(svg, blue), svg);
        // In another color, the darkest shade is near it and the lighter ones lighter.
        let green = Color32::from_rgb(0x2e, 0xc2, 0x7e);
        let s = recolor(svg, green);
        assert!(!s.contains("438de6") && !s.contains("a4caee"));
        let back = shade(green, [0x43, 0x8d, 0xe6]);
        let face = shade(green, [0xa4, 0xca, 0xee]);
        assert!(back[1] < face[1] && back[1] >= 0xc2);
    }

    #[test]
    fn masks_keep_only_the_shape() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="0" y="0" width="8" height="16" fill="#2e3436"/></svg>"##;
        let img = rasterize(svg, 16, true).unwrap();
        assert_eq!(img.pixels[0], Color32::WHITE);
        assert_eq!(img.pixels[15], Color32::TRANSPARENT);
    }
}
