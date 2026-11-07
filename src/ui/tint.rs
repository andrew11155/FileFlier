//! Folder colors: eight hues to tint folder icons with (right-click a folder),
//! in one of several schemes (Settings → Folder colors). Changing a color, or
//! the scheme, crossfades the icons to the new one with a small pop.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use egui::{Color32, Painter, Rect};

use crate::icons;
use crate::theme::Palette;

/// How many color slots there are. A folder keeps its slot when the scheme changes.
pub const SLOTS: usize = 8;

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub struct Scheme {
    pub name: &'static str,
    /// What each slot is called in this scheme (the tooltips in the Color row).
    pub names: [&'static str; SLOTS],
    /// The slot colors; `None` for Mono, which is made from the accent color.
    colors: Option<[Color32; SLOTS]>,
}

pub const SCHEMES: [Scheme; 5] = [
    // Soft enough for dark and light themes alike.
    Scheme {
        name: "Pastel",
        names: ["Red", "Orange", "Yellow", "Green", "Teal", "Blue", "Purple", "Pink"],
        colors: Some([
            rgb(0xF4A6A6),
            rgb(0xF7C59F),
            rgb(0xF6E3A1),
            rgb(0xB8E0B0),
            rgb(0xA8DDD6),
            rgb(0xA9C8F0),
            rgb(0xC9B5EC),
            rgb(0xF2B5D4),
        ]),
    },
    // A strong, saturated rainbow.
    Scheme {
        name: "Vibrant",
        names: ["Red", "Orange", "Yellow", "Green", "Teal", "Blue", "Purple", "Pink"],
        colors: Some([
            rgb(0xE5484D),
            rgb(0xF76B15),
            rgb(0xFFC53D),
            rgb(0x30A46C),
            rgb(0x12A594),
            rgb(0x0090FF),
            rgb(0x8E4EC6),
            rgb(0xD6409F),
        ]),
    },
    // Clay, ochre and greens: warm and natural.
    Scheme {
        name: "Earth",
        names: ["Terracotta", "Clay", "Ochre", "Sand", "Olive", "Moss", "Sage", "Bark"],
        colors: Some([
            rgb(0xD0805F),
            rgb(0xC9A27E),
            rgb(0xD9A845),
            rgb(0xE3D3A6),
            rgb(0xA3A65A),
            rgb(0x7E9A62),
            rgb(0xA9BFA0),
            rgb(0x8C6E5A),
        ]),
    },
    // Blues, teals and sea greens.
    Scheme {
        name: "Ocean",
        names: ["Navy", "Blue", "Sky", "Ice", "Teal", "Seafoam", "Kelp", "Slate"],
        colors: Some([
            rgb(0x3D5A8A),
            rgb(0x4F86C6),
            rgb(0x8EC3E8),
            rgb(0xC8E3EE),
            rgb(0x3FA7A0),
            rgb(0x8FD3BF),
            rgb(0x5F8F6E),
            rgb(0x7D8DA3),
        ]),
    },
    // Tints and shades of the accent color, so it follows the theme.
    Scheme {
        name: "Mono",
        names: ["Lightest", "Lighter", "Light", "Soft", "Accent", "Deep", "Darker", "Darkest"],
        colors: None,
    },
];

/// The colors of scheme `i` (the first one if out of range), for `accent`.
pub fn scheme(i: u8, accent: Color32) -> [Color32; SLOTS] {
    let s = SCHEMES.get(i as usize).unwrap_or(&SCHEMES[0]);
    s.colors.unwrap_or_else(|| mono(accent))
}

/// Eight steps from a pale tint of `accent`, through it, to a dark shade.
fn mono(accent: Color32) -> [Color32; SLOTS] {
    let (white, black) = (Color32::WHITE, Color32::from_rgb(10, 10, 14));
    [
        lerp(accent, white, 0.72),
        lerp(accent, white, 0.52),
        lerp(accent, white, 0.32),
        lerp(accent, white, 0.14),
        accent,
        lerp(accent, black, 0.22),
        lerp(accent, black, 0.40),
        lerp(accent, black, 0.56),
    ]
}

/// How long the change of color takes, in seconds.
const ANIM: f64 = 0.45;

/// Folders whose color just changed: the color they had, and when.
pub type TintAnims = HashMap<PathBuf, (Option<Color32>, f64)>;

pub fn color_of(colors: &BTreeMap<String, u8>, scheme_i: u8, accent: Color32, path: &Path) -> Option<Color32> {
    let i = *colors.get(path.to_str()?)?;
    scheme(scheme_i, accent).get(i as usize).copied()
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
        let to = color_of(self.colors, self.scheme, pal.accent, path);
        let special = if crate::system_icons::enabled() { crate::system_icons::special_folder(path) } else { None };
        let Some(&(from, start)) = self.anims.get(path) else {
            return icons::folder_icon(p, r, pal, to, special);
        };
        let t = ((self.now - start) / ANIM).clamp(0.0, 1.0) as f32;
        if t >= 1.0 {
            return icons::folder_icon(p, r, pal, to, special);
        }
        self.animating.set(true);
        let e = crate::ui::ease_out(t);
        let c = lerp(from.unwrap_or(pal.folder), to.unwrap_or(pal.folder), e);
        // A small pop: grow a little and settle back.
        let pop = 1.0 + 0.16 * (std::f32::consts::PI * t).sin() * (1.0 - t * 0.5);
        icons::folder_icon(p, Rect::from_center_size(r.center(), r.size() * pop), pal, Some(c), special);
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCENT: Color32 = Color32::from_rgb(53, 132, 228);

    #[test]
    fn schemes_are_distinct() {
        let all: Vec<_> = (0..SCHEMES.len() as u8).map(|i| scheme(i, ACCENT)).collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "two schemes are the same");
            }
            // Within a scheme, every slot is its own color.
            for (j, c) in a.iter().enumerate() {
                assert!(!a[j + 1..].contains(c), "{} repeats a color", SCHEMES[i].name);
            }
        }
    }

    #[test]
    fn mono_follows_the_accent() {
        let m = scheme(4, ACCENT);
        assert_eq!(m[4], ACCENT);
        assert_ne!(scheme(4, Color32::from_rgb(200, 60, 60)), m);
        // Light to dark.
        let lum = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        assert!(m.windows(2).all(|w| lum(w[0]) > lum(w[1])));
    }

    #[test]
    fn looks_up_and_blends() {
        let mut colors = BTreeMap::new();
        colors.insert("/home/me/Photos".to_string(), 5u8);
        let photos = Path::new("/home/me/Photos");
        assert_eq!(color_of(&colors, 0, ACCENT, photos), Some(scheme(0, ACCENT)[5]));
        assert_eq!(color_of(&colors, 3, ACCENT, photos), Some(rgb(0x8FD3BF)));
        assert_eq!(color_of(&colors, 0, ACCENT, Path::new("/home/me")), None);
        assert_eq!(scheme(99, ACCENT), scheme(0, ACCENT));
        let (a, b) = (Color32::from_rgb(0, 100, 200), Color32::from_rgb(200, 100, 0));
        assert_eq!(lerp(a, b, 0.5), Color32::from_rgb(100, 100, 100));
        assert_eq!(lerp(a, b, 0.0), a);
    }
}
