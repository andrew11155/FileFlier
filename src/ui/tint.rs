//! Folder colors: eight hues to tint folder icons with (right-click a folder),
//! in one of several schemes (Settings → Folder colors). Changing a color, or
//! the scheme, crossfades the icons to the new one with a small pop.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use egui::{Color32, Painter, Rect};

use crate::icons;
use crate::theme::Palette;

/// The eight color slots. A folder keeps its slot when the scheme changes.
pub const NAMES: [&str; 8] = ["Red", "Orange", "Yellow", "Green", "Teal", "Blue", "Purple", "Pink"];

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// The schemes: a name and a color for each slot, in `NAMES` order.
pub const SCHEMES: [(&str, [Color32; 8]); 5] = [
    // Soft enough for dark and light themes alike.
    (
        "Pastel",
        [
            rgb(0xF4A6A6),
            rgb(0xF7C59F),
            rgb(0xF6E3A1),
            rgb(0xB8E0B0),
            rgb(0xA8DDD6),
            rgb(0xA9C8F0),
            rgb(0xC9B5EC),
            rgb(0xF2B5D4),
        ],
    ),
    // The same rainbow with more color in it.
    (
        "Vivid",
        [
            rgb(0xEF6F6C),
            rgb(0xF5A25D),
            rgb(0xF2D45C),
            rgb(0x7CCB7A),
            rgb(0x4FC1B5),
            rgb(0x6AA6F0),
            rgb(0xA685E8),
            rgb(0xEC7FB6),
        ],
    ),
    // Dracula's accent colors (blue is its "comment" color).
    (
        "Dracula",
        [
            rgb(0xFF5555),
            rgb(0xFFB86C),
            rgb(0xF1FA8C),
            rgb(0x50FA7B),
            rgb(0x8BE9FD),
            rgb(0x6272A4),
            rgb(0xBD93F9),
            rgb(0xFF79C6),
        ],
    ),
    // Nord's Aurora and Frost; a muted pink to match.
    (
        "Nord",
        [
            rgb(0xBF616A),
            rgb(0xD08770),
            rgb(0xEBCB8B),
            rgb(0xA3BE8C),
            rgb(0x8FBCBB),
            rgb(0x5E81AC),
            rgb(0xB48EAD),
            rgb(0xD8A1C4),
        ],
    ),
    // Gruvbox's warm, earthy brights.
    (
        "Gruvbox",
        [
            rgb(0xFB4934),
            rgb(0xFE8019),
            rgb(0xFABD2F),
            rgb(0xB8BB26),
            rgb(0x8EC07C),
            rgb(0x83A598),
            rgb(0xB16286),
            rgb(0xD3869B),
        ],
    ),
];

/// The colors of scheme `i` (the first one if out of range).
pub fn scheme(i: u8) -> &'static [Color32; 8] {
    &SCHEMES.get(i as usize).unwrap_or(&SCHEMES[0]).1
}

/// How long the change of color takes, in seconds.
const ANIM: f64 = 0.45;

/// Folders whose color just changed: the color they had, and when.
pub type TintAnims = HashMap<PathBuf, (Option<Color32>, f64)>;

pub fn color_of(colors: &BTreeMap<String, u8>, scheme_i: u8, path: &Path) -> Option<Color32> {
    let i = *colors.get(path.to_str()?)?;
    scheme(scheme_i).get(i as usize).copied()
}

/// Everything needed to draw tinted folders during one frame.
pub struct Tints<'a> {
    pub colors: &'a BTreeMap<String, u8>,
    pub scheme: u8,
    pub anims: &'a TintAnims,
    pub now: f64,
    /// Set when an icon is mid-animation (the caller asks for another frame).
    pub animating: Cell<bool>,
}

impl<'a> Tints<'a> {
    pub fn new(colors: &'a BTreeMap<String, u8>, scheme: u8, anims: &'a TintAnims, now: f64) -> Self {
        Tints { colors, scheme, anims, now, animating: Cell::new(false) }
    }

    /// Draw the folder icon for `path` in `r`, in its color.
    pub fn folder(&self, p: &Painter, r: Rect, pal: &Palette, path: &Path) {
        let to = color_of(self.colors, self.scheme, path);
        let Some(&(from, start)) = self.anims.get(path) else {
            return icons::folder_tinted(p, r, pal, to);
        };
        let t = ((self.now - start) / ANIM).clamp(0.0, 1.0) as f32;
        if t >= 1.0 {
            return icons::folder_tinted(p, r, pal, to);
        }
        self.animating.set(true);
        let e = crate::ui::ease_out(t);
        let c = lerp(from.unwrap_or(pal.folder), to.unwrap_or(pal.folder), e);
        // A small pop: grow a little and settle back.
        let pop = 1.0 + 0.16 * (std::f32::consts::PI * t).sin() * (1.0 - t * 0.5);
        icons::folder_tinted(p, Rect::from_center_size(r.center(), r.size() * pop), pal, Some(c));
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes_are_distinct() {
        for (i, a) in SCHEMES.iter().enumerate() {
            for b in &SCHEMES[i + 1..] {
                assert_ne!(a.1, b.1, "{} and {} are the same", a.0, b.0);
            }
            // Within a scheme, every slot is its own color.
            for (j, c) in a.1.iter().enumerate() {
                assert!(!a.1[j + 1..].contains(c), "{} repeats a color", a.0);
            }
        }
    }

    #[test]
    fn looks_up_and_blends() {
        let mut colors = BTreeMap::new();
        colors.insert("/home/me/Photos".to_string(), 5u8);
        assert_eq!(color_of(&colors, 0, Path::new("/home/me/Photos")), Some(SCHEMES[0].1[5]));
        assert_eq!(color_of(&colors, 2, Path::new("/home/me/Photos")), Some(rgb(0x6272A4)));
        assert_eq!(color_of(&colors, 0, Path::new("/home/me")), None);
        assert_eq!(scheme(99), scheme(0));
        let (a, b) = (Color32::from_rgb(0, 100, 200), Color32::from_rgb(200, 100, 0));
        assert_eq!(lerp(a, b, 0.5), Color32::from_rgb(100, 100, 100));
        assert_eq!(lerp(a, b, 0.0), a);
    }
}
