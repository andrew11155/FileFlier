//! Extensions: optional features you switch on and off in *Settings → Extensions*.
//!
//! They're built in for now. Each one is described here (id, name, what it
//! does) and the app asks `Config::ext_on` before showing anything of it, so a
//! plugin system can later list and toggle loaded plugins the same way.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ext {
    Ratings,
    Tags,
    BatchRename,
    Dropbox,
}

impl Ext {
    pub const ALL: [Ext; 4] = [Ext::Ratings, Ext::Tags, Ext::BatchRename, Ext::Dropbox];

    /// Key in the config's `extensions` map.
    pub fn id(self) -> &'static str {
        match self {
            Ext::Ratings => "ratings",
            Ext::Tags => "tags",
            Ext::BatchRename => "batch_rename",
            Ext::Dropbox => "dropbox",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Ext::Ratings => "Ratings",
            Ext::Tags => "Tags",
            Ext::BatchRename => "Batch rename",
            Ext::Dropbox => "Dropbox",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Ext::Ratings => "Rate files with 1 to 5 stars: a Rating column, stars in the preview panel and menu",
            Ext::Tags => "Tag files with colored tags and find them again from the Tags list in the sidebar",
            Ext::Dropbox => {
                "With the Dropbox app running: sync badges, and Ctrl+right-click for Dropbox's own menu (copy link, share, version history) and choosing which folders sync on this computer"
            }
            Ext::BatchRename => {
                "Rename several selected files at once: from a template with numbers and dates, or by finding and replacing text"
            }
        }
    }
}
