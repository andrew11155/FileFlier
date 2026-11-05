//! Persistent settings stored at ~/.config/file-flier/config.json.

use std::path::PathBuf;

use crate::fs_model::Sort;
use crate::theme::ThemeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ViewMode {
    #[default]
    Details,
    List,
    Grid,
    /// Grids with bigger tiles and thumbnails.
    MediumGrid,
    LargeGrid,
    ExtraLargeGrid,
    /// Details with folders that expand in place (arrow keys open and close them).
    Tree,
    /// Miller columns: parent folders to the left, the folder under the cursor to the right.
    Columns,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
    Spacious,
}

impl Density {
    pub fn row_height(self) -> f32 {
        match self {
            Density::Compact => 26.0,
            Density::Comfortable => 32.0,
            Density::Spacious => 38.0,
        }
    }
}

/// What the title bar shows in its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum TitleIcon {
    /// File Flier's paper plane.
    #[default]
    Logo,
    /// The Files app's icon from the desktop's icon theme.
    Files,
    /// A folder.
    Folder,
    /// Nothing.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum DateStyle {
    /// 2026-09-26 14:03
    #[default]
    Iso,
    /// "5 min ago", "Yesterday 14:03"
    Relative,
    /// Sep 26, 2026
    Friendly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Startup {
    #[default]
    Home,
    RestoreSession,
}

/// Open tabs, saved so they can be restored on the next start.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Session {
    pub panes: Vec<Vec<PathBuf>>,
    pub active_tabs: Vec<usize>,
    pub active_pane: usize,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    /// None in configs written before themes existed; see `theme()`.
    pub theme: Option<ThemeId>,
    /// Custom accent color (RGB); None uses the theme's own.
    pub accent: Option<[u8; 3]>,
    pub ui_scale: f32,
    pub density: Density,
    pub date_style: DateStyle,
    pub show_item_counts: bool,
    pub startup: Startup,
    pub session: Session,
    /// Smooth transitions; off = everything snaps instantly.
    pub animations: bool,
    /// Frosted glass: translucent floating panels over a soft backdrop.
    pub glass: bool,
    /// Panel opacity in glass mode (0.4 = very see-through, 0.95 = nearly solid).
    pub glass_opacity: f32,
    /// Let the desktop show through (blurred by the compositor where supported).
    /// None = automatic: on for KDE Plasma and macOS, off elsewhere.
    pub glass_see_through: Option<bool>,
    pub bookmarks: Vec<PathBuf>,
    pub show_hidden: bool,
    pub dark_mode: bool,
    /// Old two-pane split (read only to migrate to `layout`).
    pub split: bool,
    pub show_preview: bool,
    pub sort: Sort,
    pub recent: Vec<PathBuf>,
    pub view: ViewMode,
    pub show_sidebar: bool,
    /// The details view's columns after Name, by id, in order (see `MetaCol`).
    pub column_order: Vec<String>,
    /// Tooltips on hover.
    pub tooltips: bool,
    /// The bar at the bottom of each tab (Ctrl+J), and which counters it shows.
    pub show_bottom_bar: bool,
    pub show_folder_count: bool,
    pub show_file_count: bool,
    pub show_scroll_pct: bool,
    pub sidebar_width: f32,
    /// Old split position (read only to migrate to `layout`).
    pub split_ratio: f32,
    /// How panes are tiled. Pane 0 is where a new window starts; the others open at home
    /// unless the session is restored.
    pub layout: Option<crate::layout::Layout>,
    /// Folder colors chosen by the user: folder path -> color slot (red, orange, …).
    pub folder_colors: std::collections::BTreeMap<String, u8>,
    /// Which scheme the folder color slots are drawn in (see `ui::tint::SCHEMES`).
    pub folder_scheme: u8,
    /// Draw folders and the sidebar with GNOME's icons (the installed Adwaita theme).
    pub gnome_icons: bool,
    /// The icon in the title bar's top-left corner.
    pub title_icon: TitleIcon,
    /// Window buttons as GNOME and Ubuntu draw them, in the desktop's button layout.
    pub native_buttons: bool,
    /// Rounder corners; None follows the theme (on in the GNOME themes).
    pub rounded: Option<bool>,
    /// A GNOME Files-like sidebar; None follows the theme.
    pub gnome_sidebar: Option<bool>,
    /// Text drawn as GTK draws it; None follows the theme.
    pub gtk_text: Option<bool>,
    /// Sidebar folders have an arrow that expands them into a tree of their subfolders.
    pub sidebar_trees: bool,
    /// The order of the sidebar's groups, by title (dragged into place). Groups not
    /// listed follow in their usual order.
    pub sidebar_order: Vec<String>,
    /// Sidebar groups turned off in Settings, by title.
    pub sidebar_hidden: Vec<String>,
    /// Shortcuts the user changed: command id -> keys ("Ctrl+Shift+P"). Others use the defaults.
    pub keymap: std::collections::BTreeMap<String, Vec<String>>,
    /// Extensions switched on or off, by id (see `ext::Ext`). Missing ones are on.
    pub extensions: std::collections::BTreeMap<String, bool>,
    pub collapsed: Vec<String>,
    pub inspector_width: f32,
    /// Render office documents as pages with LibreOffice when it's installed.
    pub office_previews: bool,
    /// Set once the preview panel has been turned on for existing users.
    pub preview_panel_intro: bool,
    /// Look for a new release on GitHub shortly after startup, at most once a day.
    pub check_updates: bool,
    /// Unix time of the last update check.
    pub last_update_check: u64,
    /// A release the user chose not to be reminded about.
    pub skipped_version: Option<String>,
}

impl ViewMode {
    /// The grids, smallest first.
    pub const GRIDS: [ViewMode; 4] =
        [ViewMode::Grid, ViewMode::MediumGrid, ViewMode::LargeGrid, ViewMode::ExtraLargeGrid];

    pub fn is_grid(self) -> bool {
        Self::GRIDS.contains(&self)
    }
}

impl Config {
    pub fn ext_on(&self, e: crate::ext::Ext) -> bool {
        self.extensions.get(e.id()).copied().unwrap_or(true)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: None,
            accent: None,
            ui_scale: 1.0,
            density: Density::Comfortable,
            date_style: DateStyle::Iso,
            show_item_counts: true,
            startup: Startup::Home,
            session: Session::default(),
            animations: true,
            glass: false,
            glass_opacity: 0.72,
            glass_see_through: None,
            bookmarks: Vec::new(),
            show_hidden: false,
            dark_mode: true,
            split: false,
            show_preview: false,
            sort: Sort::default(),
            recent: Vec::new(),
            view: ViewMode::Details,
            show_sidebar: true,
            column_order: Vec::new(),
            tooltips: true,
            show_bottom_bar: true,
            show_folder_count: true,
            show_file_count: true,
            show_scroll_pct: true,
            sidebar_width: 230.0,
            split_ratio: 0.5,
            layout: None,
            folder_colors: Default::default(),
            folder_scheme: 0,
            gnome_icons: false,
            title_icon: TitleIcon::Logo,
            native_buttons: false,
            rounded: None,
            gnome_sidebar: None,
            gtk_text: None,
            sidebar_trees: true,
            sidebar_order: Vec::new(),
            sidebar_hidden: Vec::new(),
            keymap: Default::default(),
            extensions: Default::default(),
            collapsed: vec!["Recents".into()],
            inspector_width: 340.0,
            office_previews: true,
            preview_panel_intro: false,
            check_updates: true,
            last_update_check: 0,
            skipped_version: None,
        }
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("file-flier").join("config.json"))
}

impl Config {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn theme(&self) -> ThemeId {
        self.theme.unwrap_or(if self.dark_mode { ThemeId::Dark } else { ThemeId::Light })
    }

    pub fn accent_color(&self) -> Option<egui::Color32> {
        self.accent.map(|[r, g, b]| egui::Color32::from_rgb(r, g, b))
    }

    /// Whether the desktop should show through in glass mode.
    /// Whether the window's own corners are rounded (needs a transparent window, made at
    /// startup). macOS rounds its windows itself.
    pub fn round_window(&self) -> bool {
        !cfg!(target_os = "macos") && self.palette().rounded
    }

    pub fn see_through(&self) -> bool {
        self.glass
            && self.glass_see_through.unwrap_or_else(|| {
                // Only default it on where the compositor blurs behind the window;
                // elsewhere a sharp desktop behind text hurts readability.
                cfg!(target_os = "macos")
                    || std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("KDE"))
            })
    }

    pub fn palette(&self) -> &'static crate::theme::Palette {
        let look =
            crate::theme::Look { rounded: self.rounded, gnome_sidebar: self.gnome_sidebar, gtk_text: self.gtk_text };
        crate::theme::palette_with(self.theme(), self.accent_color(), look)
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Write-then-rename so a crash mid-save can't leave a corrupt config.
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }

    pub fn push_recent(&mut self, dir: PathBuf) {
        self.recent.retain(|p| p != &dir);
        self.recent.insert(0, dir);
        self.recent.truncate(30);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_configs_still_load() {
        // A config from before themes/settings existed keeps its light mode and sort.
        let old = r#"{"dark_mode": false, "sort": {"key": "Size", "descending": true}, "bookmarks": ["/tmp"]}"#;
        let c: Config = serde_json::from_str(old).unwrap();
        assert_eq!(c.theme(), ThemeId::Light);
        assert!(c.sort.descending && c.sort.folders_first);
        assert_eq!(c.bookmarks, vec![PathBuf::from("/tmp")]);
        assert_eq!(c.ui_scale, 1.0);
    }
}
