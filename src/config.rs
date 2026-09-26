//! Persistent settings stored at ~/.config/file-flier/config.json.

use std::path::PathBuf;

use crate::fs_model::Sort;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ViewMode {
    #[default]
    Details,
    List,
    Grid,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
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

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, s);
        }
    }

    pub fn push_recent(&mut self, dir: PathBuf) {
        self.recent.retain(|p| p != &dir);
        self.recent.insert(0, dir);
        self.recent.truncate(30);
    }
}
