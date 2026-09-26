//! Colors and styling. The palette mirrors File Pilot's dark theme.

use egui::{Color32, CornerRadius, Stroke, Visuals, style::ScrollStyle, vec2};

#[derive(Clone, Copy)]
pub struct Palette {
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
    pub page: Color32,
}

pub const DARK: Palette = Palette {
    titlebar: Color32::from_rgb(30, 32, 34),
    bg: Color32::from_rgb(23, 25, 27),
    sidebar: Color32::from_rgb(30, 33, 35),
    row_sep: Color32::from_rgb(33, 35, 38),
    hover: Color32::from_rgb(36, 39, 42),
    tab_hover: Color32::from_rgb(38, 42, 43),
    input: Color32::from_rgb(30, 32, 34),
    border: Color32::from_rgb(54, 58, 59),
    text: Color32::from_rgb(218, 220, 223),
    text_strong: Color32::from_rgb(245, 246, 247),
    text_dim: Color32::from_rgb(142, 147, 153),
    text_faint: Color32::from_rgb(92, 97, 102),
    accent: Color32::from_rgb(0, 119, 165),
    accent_dim: Color32::from_rgb(12, 62, 84),
    grid_sel: Color32::from_rgb(14, 31, 39),
    popup: Color32::from_rgb(28, 32, 33),
    close_hover: Color32::from_rgb(196, 43, 28),
    folder: Color32::from_rgb(253, 215, 108),
    folder_back: Color32::from_rgb(232, 178, 62),
    page: Color32::from_rgb(236, 239, 243),
};

pub const LIGHT: Palette = Palette {
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
    accent: Color32::from_rgb(0, 119, 165),
    accent_dim: Color32::from_rgb(190, 222, 238),
    grid_sel: Color32::from_rgb(222, 238, 246),
    popup: Color32::from_rgb(252, 252, 253),
    close_hover: Color32::from_rgb(196, 43, 28),
    folder: Color32::from_rgb(250, 200, 80),
    folder_back: Color32::from_rgb(222, 165, 45),
    page: Color32::from_rgb(255, 255, 255),
};

pub fn palette(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}

pub fn apply(ctx: &egui::Context, dark: bool) {
    let p = palette(dark);
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
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
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
        s.animation_time = 0.08;
        use egui::{FontFamily::Proportional, FontId, TextStyle};
        s.text_styles.insert(TextStyle::Body, FontId::new(14.0, Proportional));
        s.text_styles.insert(TextStyle::Button, FontId::new(14.0, Proportional));
        s.text_styles.insert(TextStyle::Small, FontId::new(11.5, Proportional));
    });
}

/// Prefer a crisp system UI font when one is installed; egui's bundled fonts stay as fallback.
pub fn load_system_font(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/opentype/inter/Inter-Regular.otf",
        "/usr/share/fonts/truetype/inter/Inter-Regular.ttf",
        "/usr/share/fonts/inter/Inter-Regular.otf",
        "/usr/share/fonts/TTF/Inter-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf",
        "/usr/share/fonts/cantarell/Cantarell-VF.otf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    ];
    let Some(bytes) = CANDIDATES.iter().find_map(|p| std::fs::read(p).ok()) else { return };
    ctx.add_font(egui::epaint::text::FontInsert::new(
        "system-ui",
        egui::FontData::from_owned(bytes),
        vec![egui::epaint::text::InsertFontFamily {
            family: egui::FontFamily::Proportional,
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));
}
