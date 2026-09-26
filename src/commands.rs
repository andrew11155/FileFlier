//! Every user-facing action, shared by keyboard shortcuts, menus and the command palette.

use egui::{Key, KeyboardShortcut, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    ReverseSort,
    Help,
    OpenInNewTab,
    ViewDetails,
    ViewList,
    ViewGrid,
    ToggleSidebar,
    LockTab,
    Settings,
    QuickLook,
    Undo,
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
    ReverseSort,
    Help,
    OpenInNewTab,
    ViewDetails,
    ViewList,
    ViewGrid,
    ToggleSidebar,
    LockTab,
    Settings,
    QuickLook,
    Undo,
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
            TogglePreview => "Toggle inspector",
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
            ReverseSort => "Reverse sort order",
            Help => "Keyboard shortcuts",
            OpenInNewTab => "Open in new tab",
            ViewDetails => "View: details",
            ViewList => "View: list",
            ViewGrid => "View: grid",
            ToggleSidebar => "Toggle sidebar",
            LockTab => "Lock / unlock tab",
            Settings => "Settings",
            QuickLook => "Quick Look",
            Undo => "Undo",
        }
    }

    /// Shortcuts that trigger this command. The first one is shown in the UI.
    pub fn shortcuts(self) -> Vec<KeyboardShortcut> {
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
            ReverseSort => vec![],
            Help => vec![sc(NONE, Key::F1)],
            OpenInNewTab => vec![sc(CTRL, Key::Enter)],
            ViewDetails => vec![sc(CTRL_SHIFT, Key::D)],
            ViewList => vec![sc(CTRL_SHIFT, Key::L)],
            ViewGrid => vec![sc(CTRL_SHIFT, Key::G)],
            ToggleSidebar => vec![sc(CTRL, Key::B)],
            LockTab => vec![],
            Settings => vec![sc(CTRL, Key::Comma)],
            QuickLook => vec![sc(NONE, Key::Space)],
            Undo => vec![sc(CTRL, Key::Z)],
        }
    }

    /// All shortcuts, formatted for key badges.
    pub fn shortcut_texts(self, ctx: &egui::Context) -> Vec<String> {
        self.shortcuts()
            .iter()
            .map(|s| ctx.format_shortcut(s).replace("Period", ".").replace("Backslash", "\\"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_listed_once() {
        // Guards against adding a variant but forgetting it in ALL (it'd be unreachable).
        let count = ALL.len();
        let unique: std::collections::HashSet<_> = ALL.iter().map(|c| format!("{c:?}")).collect();
        assert_eq!(unique.len(), count);
        assert_eq!(count, Undo as usize + 1);
    }
}
