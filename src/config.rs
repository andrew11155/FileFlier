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
    pub bookmarks: Vec<PathBuf>,
    pub show_hidden: bool,
    pub dark_mode: bool,
    pub split: bool,
    pub show_preview: bool,
    pub sort: Sort,
    pub recent: Vec<PathBuf>,
    pub view: ViewMode,
    pub show_sidebar: bool,
    pub sidebar_width: f32,
    pub split_ratio: f32,
    pub collapsed: Vec<String>,
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
            bookmarks: Vec::new(),
            show_hidden: false,
            dark_mode: true,
            split: false,
            show_preview: false,
            sort: Sort::default(),
            recent: Vec::new(),
            view: ViewMode::Details,
            show_sidebar: true,
            sidebar_width: 230.0,
            split_ratio: 0.5,
            collapsed: vec!["Recents".into()],
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

    pub fn palette(&self) -> &'static crate::theme::Palette {
        crate::theme::palette(self.theme(), self.accent_color())
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
