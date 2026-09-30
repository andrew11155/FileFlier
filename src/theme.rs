//! Colors and styling. The palette mirrors File Pilot's dark theme.

use egui::{Color32, CornerRadius, Stroke, Visuals, style::ScrollStyle, vec2};

#[derive(Clone, Copy)]
pub struct Palette {
    pub is_dark: bool,
    pub titlebar: Color32,
    pub bg: Color32,
    pub sidebar: Color32,
    pub row_sep: Color32,
    pub hover: Color32,
    pub tab_hover: Color32,
    pub input: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_strong: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_dim: Color32,
    pub grid_sel: Color32,
    pub popup: Color32,
    pub close_hover: Color32,
    pub folder: Color32,
    pub folder_back: Color32,
    /// Text drawn on top of `accent` (white or near-black, whichever reads better).
    pub on_accent: Color32,
}

pub const DARK: Palette = Palette {
    is_dark: true,
    titlebar: Color32::from_rgb(26, 28, 32),
    bg: Color32::from_rgb(19, 21, 24),
    sidebar: Color32::from_rgb(25, 27, 31),
    row_sep: Color32::from_rgb(30, 33, 37),
    hover: Color32::from_rgb(33, 37, 43),
    tab_hover: Color32::from_rgb(39, 43, 50),
    input: Color32::from_rgb(27, 30, 34),
    border: Color32::from_rgb(48, 53, 60),
    text: Color32::from_rgb(230, 232, 236),
    text_strong: Color32::from_rgb(250, 251, 252),
    text_dim: Color32::from_rgb(148, 155, 165),
    text_faint: Color32::from_rgb(94, 100, 110),
    // A vivid azure: the selection is the brightest thing on screen.
    accent: Color32::from_rgb(20, 132, 222),
    accent_dim: Color32::from_rgb(20, 66, 102),
    grid_sel: Color32::from_rgb(19, 39, 57),
    popup: Color32::from_rgb(28, 31, 36),
    close_hover: Color32::from_rgb(210, 45, 40),
    folder: Color32::from_rgb(255, 209, 92),
    folder_back: Color32::from_rgb(236, 168, 44),
    on_accent: Color32::WHITE,
};

pub const LIGHT: Palette = Palette {
    is_dark: false,
    titlebar: Color32::from_rgb(232, 234, 237),
    bg: Color32::from_rgb(250, 250, 251),
    sidebar: Color32::from_rgb(241, 243, 245),
    row_sep: Color32::from_rgb(236, 238, 240),
    hover: Color32::from_rgb(232, 236, 240),
    tab_hover: Color32::from_rgb(222, 225, 229),
    input: Color32::from_rgb(255, 255, 255),
    border: Color32::from_rgb(208, 212, 216),
    text: Color32::from_rgb(32, 34, 37),
    text_strong: Color32::from_rgb(0, 0, 0),
    text_dim: Color32::from_rgb(98, 104, 110),
    text_faint: Color32::from_rgb(160, 165, 170),
    accent: Color32::from_rgb(0, 114, 214),
    accent_dim: Color32::from_rgb(184, 214, 242),
    grid_sel: Color32::from_rgb(220, 235, 250),
    popup: Color32::from_rgb(252, 252, 253),
    close_hover: Color32::from_rgb(196, 43, 28),
    folder: Color32::from_rgb(255, 202, 72),
    folder_back: Color32::from_rgb(230, 160, 30),
    on_accent: Color32::WHITE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ThemeId {
    Dark,
    Light,
    Midnight,
    Nord,
    Dracula,
    CatppuccinMocha,
    Gruvbox,
    SolarizedLight,
}

impl ThemeId {
    pub const ALL: [ThemeId; 8] = [
        ThemeId::Dark,
        ThemeId::Light,
        ThemeId::Midnight,
        ThemeId::Nord,
        ThemeId::Dracula,
        ThemeId::CatppuccinMocha,
        ThemeId::Gruvbox,
        ThemeId::SolarizedLight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ThemeId::Dark => "File Pilot Dark",
            ThemeId::Light => "Light",
            ThemeId::Midnight => "Midnight",
            ThemeId::Nord => "Nord",
            ThemeId::Dracula => "Dracula",
            ThemeId::CatppuccinMocha => "Catppuccin Mocha",
            ThemeId::Gruvbox => "Gruvbox",
            ThemeId::SolarizedLight => "Solarized Light",
        }
    }
}

/// Accent colors offered in Settings, in addition to each theme's own.
pub const ACCENTS: [(&str, Color32); 8] = [
    ("Teal", Color32::from_rgb(0, 119, 165)),
    ("Blue", Color32::from_rgb(52, 120, 246)),
    ("Purple", Color32::from_rgb(137, 87, 229)),
    ("Pink", Color32::from_rgb(214, 76, 150)),
    ("Red", Color32::from_rgb(210, 64, 58)),
    ("Orange", Color32::from_rgb(230, 126, 34)),
    ("Green", Color32::from_rgb(46, 160, 67)),
    ("Yellow", Color32::from_rgb(230, 185, 30)),
];

fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn luminance(c: Color32) -> f32 {
    let f = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
}

/// Colors a theme is defined by; everything else is derived.
struct Spec {
    is_dark: bool,
    bg: u32,
    titlebar: u32,
    sidebar: u32,
    input: u32,
    border: u32,
    text: u32,
    text_dim: u32,
    accent: u32,
    folder: u32,
}

fn from_spec(s: Spec) -> Palette {
    let (bg, text) = (hex(s.bg), hex(s.text));
    let titlebar = hex(s.titlebar);
    let folder = hex(s.folder);
    let accent = hex(s.accent);
    let mut p = Palette {
        is_dark: s.is_dark,
        titlebar,
        bg,
        sidebar: hex(s.sidebar),
        row_sep: mix(bg, hex(s.border), 0.35),
        hover: mix(bg, text, if s.is_dark { 0.07 } else { 0.05 }),
        tab_hover: mix(titlebar, text, 0.10),
        input: hex(s.input),
        border: hex(s.border),
        text,
        text_strong: mix(text, if s.is_dark { Color32::WHITE } else { Color32::BLACK }, 0.6),
        text_dim: hex(s.text_dim),
        text_faint: mix(hex(s.text_dim), bg, 0.45),
        accent,
        accent_dim: accent,
        grid_sel: accent,
        popup: mix(titlebar, bg, 0.3),
        close_hover: Color32::from_rgb(196, 43, 28),
        folder,
        folder_back: mix(folder, Color32::BLACK, 0.12),
        on_accent: Color32::WHITE,
    };
    set_accent(&mut p, accent);
    p
}

fn set_accent(p: &mut Palette, accent: Color32) {
    p.accent = accent;
    p.accent_dim = mix(p.bg, accent, if p.is_dark { 0.40 } else { 0.30 });
    p.grid_sel = mix(p.bg, accent, 0.16);
    // Pick whichever text color contrasts more with the accent (WCAG contrast ratio).
    let dark_text = Color32::from_rgb(20, 20, 24);
    let contrast = |a: Color32, b: Color32| {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    };
    p.on_accent =
        if contrast(accent, dark_text) > contrast(accent, Color32::WHITE) { dark_text } else { Color32::WHITE };
}

fn build(theme: ThemeId) -> Palette {
    match theme {
        ThemeId::Dark => DARK,
        ThemeId::Light => LIGHT,
        ThemeId::Midnight => from_spec(Spec {
            is_dark: true,
            bg: 0x000000,
            titlebar: 0x0b0b0c,
            sidebar: 0x0b0b0c,
            input: 0x121214,
            border: 0x2a2a2e,
            text: 0xd8d8dc,
            text_dim: 0x8a8a92,
            accent: 0x3b82f6,
            folder: 0xfdd76c,
        }),
        ThemeId::Nord => from_spec(Spec {
            is_dark: true,
            bg: 0x2e3440,
            titlebar: 0x272c36,
            sidebar: 0x272c36,
            input: 0x3b4252,
            border: 0x434c5e,
            text: 0xe5e9f0,
            text_dim: 0x9aa3b5,
            accent: 0x5e81ac,
            folder: 0xebcb8b,
        }),
        ThemeId::Dracula => from_spec(Spec {
            is_dark: true,
            bg: 0x282a36,
            titlebar: 0x21222c,
            sidebar: 0x21222c,
            input: 0x343746,
            border: 0x44475a,
            text: 0xf8f8f2,
            text_dim: 0xa4a8c4,
            accent: 0x9b6bf2,
            folder: 0xf1fa8c,
        }),
        ThemeId::CatppuccinMocha => from_spec(Spec {
            is_dark: true,
            bg: 0x1e1e2e,
            titlebar: 0x181825,
            sidebar: 0x181825,
            input: 0x313244,
            border: 0x45475a,
            text: 0xcdd6f4,
            text_dim: 0xa6adc8,
            accent: 0x8c6fd8,
            folder: 0xf9e2af,
        }),
        ThemeId::Gruvbox => from_spec(Spec {
            is_dark: true,
            bg: 0x282828,
            titlebar: 0x1d2021,
            sidebar: 0x1d2021,
            input: 0x3c3836,
            border: 0x504945,
            text: 0xebdbb2,
            text_dim: 0xa89984,
            accent: 0x458588,
            folder: 0xfabd2f,
        }),
        ThemeId::SolarizedLight => from_spec(Spec {
            is_dark: false,
            bg: 0xfdf6e3,
            titlebar: 0xeee8d5,
            sidebar: 0xf5efdc,
            input: 0xfffbf0,
            border: 0xd8d0b8,
            text: 0x3c4a50,
            text_dim: 0x7b8a8b,
            accent: 0x268bd2,
            folder: 0xe6b422,
        }),
    }
}

/// The palette for a theme, optionally with a custom accent. Palettes are built once
/// and cached for the life of the program (there are only a few dozen combinations).
pub fn palette(theme: ThemeId, accent: Option<Color32>) -> &'static Palette {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    type Key = (ThemeId, Option<[u8; 3]>);
    static CACHE: OnceLock<Mutex<HashMap<Key, &'static Palette>>> = OnceLock::new();
    let key = (theme, accent.map(|c| [c.r(), c.g(), c.b()]));
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    cache.entry(key).or_insert_with(|| {
        let mut p = build(theme);
        if let Some(a) = accent {
            set_accent(&mut p, a);
        }
        Box::leak(Box::new(p))
    })
}

pub fn apply(ctx: &egui::Context, p: &Palette, ui_scale: f32, animations: bool) {
    let dark = p.is_dark;
    ctx.set_zoom_factor(ui_scale.clamp(0.8, 1.5));
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.popup;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.extreme_bg_color = p.input;
    v.faint_bg_color = p.row_sep;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, p.on_accent);
    v.text_cursor.stroke = Stroke::new(1.5, p.text_strong);
    for (w, fill) in [
        (&mut v.widgets.noninteractive, p.bg),
        (&mut v.widgets.inactive, p.border),
        (&mut v.widgets.hovered, p.tab_hover),
        (&mut v.widgets.active, p.accent),
        (&mut v.widgets.open, p.tab_hover),
    ] {
        w.corner_radius = CornerRadius::same(4);
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.inactive.weak_bg_fill = p.input;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    v.popup_shadow = egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(110) };
    v.window_shadow = v.popup_shadow;
    ctx.set_visuals_of(if dark { egui::Theme::Dark } else { egui::Theme::Light }, v);
    ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = vec2(6.0, 4.0);
        s.spacing.button_padding = vec2(8.0, 4.0);
        s.spacing.interact_size.y = 24.0;
        let mut scroll = ScrollStyle::solid();
        scroll.bar_width = 8.0;
        scroll.bar_inner_margin = 2.0;
        scroll.bar_outer_margin = 2.0;
        s.spacing.scroll = scroll;
        s.animation_time = if animations { 0.12 } else { 0.0 };
        s.scroll_animation =
            if animations { egui::style::ScrollAnimation::default() } else { egui::style::ScrollAnimation::none() };
        use egui::{FontFamily::Proportional, FontId, TextStyle};
        s.text_styles.insert(TextStyle::Body, FontId::new(14.0, Proportional));
        s.text_styles.insert(TextStyle::Button, FontId::new(14.0, Proportional));
        s.text_styles.insert(TextStyle::Small, FontId::new(11.5, Proportional));
    });
}

/// Font family used for headings and labels that need weight ([`crate::ui::bold`]).
pub const SEMIBOLD: &str = "semibold";

/// Inter (SIL Open Font License, see `assets/fonts/Inter-OFL.txt`), bundled so text
/// looks the same crisp way on every distro. One variable font gives both weights.
static INTER: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");

/// Installs Inter as the UI font (regular and semibold), with the system's sans
/// font and egui's defaults behind it for scripts Inter doesn't cover.
pub fn load_fonts(ctx: &egui::Context) {
    use egui::epaint::text::{FontData, FontDefinitions, FontFamily, FontTweak, VariationCoords};
    let tweak = |weight: f32| FontTweak { coords: VariationCoords::new([(b"wght", weight)]), ..Default::default() };
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("inter".into(), FontData::from_static(INTER).tweak(tweak(420.0)).into());
    fonts.font_data.insert("inter-semibold".into(), FontData::from_static(INTER).tweak(tweak(620.0)).into());
    let mut fallback = Vec::new();
    const SYSTEM: &[&str] = &[
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        // macOS: broad Unicode coverage (CJK and more).
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/Library/Fonts/Arial Unicode.ttf",
    ];
    if let Some(bytes) = SYSTEM.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("system-ui".into(), FontData::from_owned(bytes).into());
        fallback.push("system-ui".to_string());
    }
    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let regular: Vec<String> = std::iter::once("inter".to_string()).chain(fallback).chain(defaults).collect();
    let semibold = std::iter::once("inter-semibold".to_string()).chain(regular.iter().cloned()).collect();
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), semibold);
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_builds_and_accent_text_is_readable() {
        for t in ThemeId::ALL {
            let p = palette(t, None);
            assert_ne!(p.bg, p.text, "{t:?}");
            for (_, a) in ACCENTS {
                let p = palette(t, Some(a));
                assert_eq!(p.accent, a);
                // Contrast between accent and its text should be at least ~3:1.
                let (l1, l2) = (luminance(p.accent), luminance(p.on_accent));
                let ratio = (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05);
                assert!(ratio >= 3.0, "{t:?} {a:?} ratio {ratio}");
            }
        }
        // Cached: same pointer on repeat lookups.
        assert!(std::ptr::eq(palette(ThemeId::Nord, None), palette(ThemeId::Nord, None)));
    }
}
