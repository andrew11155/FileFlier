//! Every user-facing action, shared by keyboard shortcuts, menus and the command palette.

use std::collections::{BTreeMap, HashMap};
use std::sync::RwLock;

use egui::{Key, KeyboardShortcut, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    CommandPalette,
    Search,
    GoToPath,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    ToggleSplit,
    SwitchPane,
    Back,
    Forward,
    Up,
    Home,
    Refresh,
    Open,
    OpenTerminal,
    ToggleHidden,
    TogglePreview,
    ToggleTheme,
    SelectAll,
    Copy,
    Cut,
    Paste,
    CopyToOtherPane,
    MoveToOtherPane,
    Trash,
    DeletePermanently,
    Rename,
    NewFolder,
    NewFile,
    CopyPath,
    ToggleBookmark,
    SortByName,
    SortBySize,
    SortByModified,
    SortByKind,
    SortByFolder,
    ReverseSort,
    Help,
    OpenInNewTab,
    ViewDetails,
    ViewList,
    ViewGrid,
    ViewMediumGrid,
    ViewLargeGrid,
    ViewExtraLargeGrid,
    ViewTree,
    ViewColumns,
    ToggleFlat,
    OpenContainingFolder,
    ToggleSidebar,
    LockTab,
    Settings,
    QuickLook,
    Undo,
    Compress,
    Extract,
    OpenWith,
    ShowTrash,
}

use Command::*;

pub const ALL: &[Command] = &[
    CommandPalette,
    Search,
    GoToPath,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    ToggleSplit,
    SwitchPane,
    Back,
    Forward,
    Up,
    Home,
    Refresh,
    Open,
    OpenTerminal,
    ToggleHidden,
    TogglePreview,
    ToggleTheme,
    SelectAll,
    Copy,
    Cut,
    Paste,
    CopyToOtherPane,
    MoveToOtherPane,
    Trash,
    DeletePermanently,
    Rename,
    NewFolder,
    NewFile,
    CopyPath,
    ToggleBookmark,
    SortByName,
    SortBySize,
    SortByModified,
    SortByKind,
    SortByFolder,
    ReverseSort,
    Help,
    OpenInNewTab,
    ViewDetails,
    ViewList,
    ViewGrid,
    ViewMediumGrid,
    ViewLargeGrid,
    ViewExtraLargeGrid,
    ViewTree,
    ViewColumns,
    ToggleFlat,
    OpenContainingFolder,
    ToggleSidebar,
    LockTab,
    Settings,
    QuickLook,
    Undo,
    Compress,
    Extract,
    OpenWith,
    ShowTrash,
];

const fn sc(modifiers: Modifiers, key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(modifiers, key)
}
const NONE: Modifiers = Modifiers::NONE;
const CTRL: Modifiers = Modifiers::COMMAND;
const SHIFT: Modifiers = Modifiers::SHIFT;
const ALT: Modifiers = Modifiers::ALT;
const CTRL_SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::COMMAND };

impl Command {
    pub fn label(self) -> &'static str {
        match self {
            CommandPalette => "Command palette",
            Search => "Search in folder (recursive)",
            GoToPath => "Go to path…",
            NewTab => "New tab",
            CloseTab => "Close tab",
            NextTab => "Next tab",
            PrevTab => "Previous tab",
            ToggleSplit => "Toggle split view",
            SwitchPane => "Switch pane",
            Back => "Back",
            Forward => "Forward",
            Up => "Parent folder",
            Home => "Home folder",
            Refresh => "Refresh",
            Open => "Open",
            OpenTerminal => "Open terminal here",
            ToggleHidden => "Toggle hidden files",
            TogglePreview => "Toggle preview panel",
            ToggleTheme => "Toggle light/dark theme",
            SelectAll => "Select all",
            Copy => "Copy",
            Cut => "Cut",
            Paste => "Paste",
            CopyToOtherPane => "Copy selection to other pane",
            MoveToOtherPane => "Move selection to other pane",
            Trash => "Move to trash",
            DeletePermanently => "Delete permanently",
            Rename => "Rename",
            NewFolder => "New folder",
            NewFile => "New file",
            CopyPath => "Copy path to clipboard",
            ToggleBookmark => "Bookmark / unbookmark current folder",
            SortByName => "Sort by name",
            SortBySize => "Sort by size",
            SortByModified => "Sort by date modified",
            SortByKind => "Sort by type",
            SortByFolder => "Sort by folder",
            ReverseSort => "Reverse sort order",
            Help => "Keyboard shortcuts",
            OpenInNewTab => "Open in new tab",
            ViewDetails => "View: details",
            ViewList => "View: list",
            ViewGrid => "View: grid",
            ViewMediumGrid => "View: medium grid",
            ViewLargeGrid => "View: large grid",
            ViewExtraLargeGrid => "View: extra large grid",
            ViewTree => "View: tree",
            ViewColumns => "View: columns",
            ToggleFlat => "Flat view (all files in subfolders)",
            OpenContainingFolder => "Open containing folder",
            ToggleSidebar => "Toggle sidebar",
            LockTab => "Lock / unlock tab",
            Settings => "Settings",
            QuickLook => "Quick Look",
            Undo => "Undo",
            Compress => "Compress to ZIP",
            Extract => "Extract here",
            OpenWith => "Open with…",
            ShowTrash => "Show trash",
        }
    }

    /// The name the keymap is saved under (stable: the variant's name).
    pub fn id(self) -> String {
        format!("{self:?}")
    }

    pub fn from_id(id: &str) -> Option<Command> {
        ALL.iter().copied().find(|c| c.id() == id)
    }

    /// Shortcuts that trigger this command: the user's own, if they changed them, else the
    /// defaults. The first one is shown in menus.
    pub fn shortcuts(self) -> Vec<KeyboardShortcut> {
        let custom = KEYMAP.read().ok().and_then(|m| m.get(&self).cloned());
        custom.unwrap_or_else(|| self.default_shortcuts())
    }

    /// Whether the user changed this command's shortcuts.
    pub fn is_customized(self) -> bool {
        KEYMAP.read().is_ok_and(|m| m.contains_key(&self))
    }

    pub fn default_shortcuts(self) -> Vec<KeyboardShortcut> {
        match self {
            CommandPalette => vec![sc(CTRL_SHIFT, Key::P), sc(CTRL, Key::K)],
            Search => vec![sc(CTRL, Key::F)],
            GoToPath => vec![sc(CTRL, Key::L)],
            NewTab => vec![sc(CTRL, Key::T)],
            CloseTab => vec![sc(CTRL, Key::W)],
            NextTab => vec![sc(CTRL, Key::Tab)],
            PrevTab => vec![sc(CTRL_SHIFT, Key::Tab)],
            ToggleSplit => vec![sc(CTRL, Key::Backslash)],
            SwitchPane => vec![sc(NONE, Key::Tab)],
            Back => vec![sc(ALT, Key::ArrowLeft)],
            Forward => vec![sc(ALT, Key::ArrowRight)],
            Up => vec![sc(NONE, Key::Backspace), sc(ALT, Key::ArrowUp)],
            Home => vec![sc(ALT, Key::Home)],
            Refresh => vec![sc(CTRL, Key::R)],
            Open => vec![sc(NONE, Key::Enter)],
            OpenTerminal => vec![sc(CTRL_SHIFT, Key::T)],
            ToggleHidden => vec![sc(CTRL, Key::H), sc(CTRL, Key::Period)],
            TogglePreview => vec![sc(NONE, Key::F3), sc(CTRL, Key::I)],
            ToggleTheme => vec![],
            SelectAll => vec![sc(CTRL, Key::A)],
            Copy => vec![sc(CTRL, Key::C)],
            Cut => vec![sc(CTRL, Key::X)],
            Paste => vec![sc(CTRL, Key::V)],
            CopyToOtherPane => vec![sc(NONE, Key::F5)],
            MoveToOtherPane => vec![sc(NONE, Key::F6)],
            Trash => vec![sc(NONE, Key::Delete)],
            DeletePermanently => vec![sc(SHIFT, Key::Delete)],
            Rename => vec![sc(NONE, Key::F2)],
            NewFolder => vec![sc(CTRL_SHIFT, Key::N)],
            NewFile => vec![sc(CTRL, Key::N)],
            CopyPath => vec![sc(CTRL_SHIFT, Key::C)],
            ToggleBookmark => vec![sc(CTRL, Key::D)],
            SortByName => vec![sc(CTRL, Key::Num1)],
            SortBySize => vec![sc(CTRL, Key::Num2)],
            SortByModified => vec![sc(CTRL, Key::Num3)],
            SortByKind => vec![sc(CTRL, Key::Num4)],
            SortByFolder => vec![sc(CTRL, Key::Num5)],
            ReverseSort => vec![],
            Help => vec![sc(NONE, Key::F1)],
            OpenInNewTab => vec![sc(CTRL, Key::Enter)],
            // Alt+1…8 pick each view, in the order the view menu lists them.
            ViewDetails => vec![sc(ALT, Key::Num1), sc(CTRL_SHIFT, Key::D)],
            ViewList => vec![sc(ALT, Key::Num2), sc(CTRL_SHIFT, Key::L)],
            ViewTree => vec![sc(ALT, Key::Num3), sc(CTRL_SHIFT, Key::E)],
            ViewColumns => vec![sc(ALT, Key::Num4), sc(CTRL_SHIFT, Key::M)],
            ViewGrid => vec![sc(ALT, Key::Num5), sc(CTRL_SHIFT, Key::G)],
            ViewMediumGrid => vec![sc(ALT, Key::Num6)],
            ViewLargeGrid => vec![sc(ALT, Key::Num7), sc(CTRL_SHIFT, Key::B)],
            ViewExtraLargeGrid => vec![sc(ALT, Key::Num8)],
            ToggleFlat => vec![sc(CTRL_SHIFT, Key::F)],
            OpenContainingFolder => vec![],
            ToggleSidebar => vec![sc(CTRL, Key::B)],
            LockTab => vec![],
            Settings => vec![sc(CTRL, Key::Comma)],
            QuickLook => vec![sc(NONE, Key::Space)],
            Undo => vec![sc(CTRL, Key::Z)],
            Compress | Extract | ShowTrash => vec![],
            OpenWith => vec![sc(CTRL_SHIFT, Key::O)],
        }
    }

    /// All shortcuts, formatted for key badges.
    pub fn shortcut_texts(self, ctx: &egui::Context) -> Vec<String> {
        self.shortcuts().iter().map(|s| format_shortcut(ctx, s)).collect()
    }
}

/// A shortcut as shown on a key badge.
pub fn format_shortcut(ctx: &egui::Context, s: &KeyboardShortcut) -> String {
    ctx.format_shortcut(s).replace("Period", ".").replace("Backslash", "\\")
}

/// The user's shortcuts, for the commands they changed (an empty list = no shortcut).
static KEYMAP: RwLock<Option<HashMap<Command, Vec<KeyboardShortcut>>>> = RwLock::new(None);

trait KeymapExt {
    fn get(&self, c: &Command) -> Option<&Vec<KeyboardShortcut>>;
    fn contains_key(&self, c: &Command) -> bool;
}

impl KeymapExt for std::sync::RwLockReadGuard<'_, Option<HashMap<Command, Vec<KeyboardShortcut>>>> {
    fn get(&self, c: &Command) -> Option<&Vec<KeyboardShortcut>> {
        self.as_ref()?.get(c)
    }
    fn contains_key(&self, c: &Command) -> bool {
        self.as_ref().is_some_and(|m| m.contains_key(c))
    }
}

/// Load the saved keymap (command id -> shortcuts like "Ctrl+Shift+P").
pub fn set_keymap(saved: &BTreeMap<String, Vec<String>>) {
    let map = saved
        .iter()
        .filter_map(|(id, keys)| Some((Command::from_id(id)?, keys.iter().filter_map(|k| parse(k)).collect())))
        .collect();
    if let Ok(mut m) = KEYMAP.write() {
        *m = Some(map);
    }
}

/// A shortcut in the saved form, e.g. "Ctrl+Shift+P" (Ctrl is Cmd on macOS).
pub fn to_string(s: &KeyboardShortcut) -> String {
    let m = s.modifiers;
    let mut out = String::new();
    for (on, name) in [(m.command || m.ctrl, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift")] {
        if on {
            out.push_str(name);
            out.push('+');
        }
    }
    out + s.logical_key.name()
}

pub fn parse(s: &str) -> Option<KeyboardShortcut> {
    let (mods, key) = s.rsplit_once('+').map_or(("", s), |(m, k)| (m, k));
    // "Ctrl++" would split oddly; keys with '+' in their name are spelled "Plus".
    let key = Key::from_name(key)?;
    let mut m = Modifiers::NONE;
    for part in mods.split('+').filter(|p| !p.is_empty()) {
        match part {
            "Ctrl" | "Cmd" => m |= Modifiers::COMMAND,
            "Alt" => m |= Modifiers::ALT,
            "Shift" => m |= Modifiers::SHIFT,
            _ => return None,
        }
    }
    Some(KeyboardShortcut::new(m, key))
}

/// The shortcut for a key press: Ctrl and Cmd both count as the command key.
pub fn from_press(modifiers: Modifiers, key: Key) -> KeyboardShortcut {
    let mut m = Modifiers::NONE;
    if modifiers.command || modifiers.ctrl {
        m |= Modifiers::COMMAND;
    }
    if modifiers.alt {
        m |= Modifiers::ALT;
    }
    if modifiers.shift {
        m |= Modifiers::SHIFT;
    }
    KeyboardShortcut::new(m, key)
}

/// Two shortcuts that a key press would trigger alike.
pub fn same(a: &KeyboardShortcut, b: &KeyboardShortcut) -> bool {
    to_string(a) == to_string(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_round_trip() {
        for c in ALL {
            for s in c.default_shortcuts() {
                let text = to_string(&s);
                assert!(same(&parse(&text).unwrap(), &s), "{text}");
            }
        }
        assert_eq!(to_string(&parse("Ctrl+Shift+P").unwrap()), "Ctrl+Shift+P");
        assert!(parse("Hyper+P").is_none());
        assert!(parse("Ctrl+Nope").is_none());
    }

    #[test]
    fn every_command_is_listed_once() {
        // Guards against adding a variant but forgetting it in ALL (it'd be unreachable).
        let count = ALL.len();
        let unique: std::collections::HashSet<_> = ALL.iter().map(|c| format!("{c:?}")).collect();
        assert_eq!(unique.len(), count);
        assert_eq!(count, ShowTrash as usize + 1);
    }
}
