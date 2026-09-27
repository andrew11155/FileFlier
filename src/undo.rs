//! One level of undo for file operations (Ctrl+Z, or the Undo button on toasts).

use std::path::PathBuf;
use std::time::SystemTime;

use crate::ops;

#[derive(Clone, Debug)]
pub enum UndoOp {
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    /// Items moved to the trash, identified by original path and when it happened.
    Trash {
        originals: Vec<PathBuf>,
        when: SystemTime,
    },
    /// (source, destination) pairs of a copy or move.
    Transfer {
        pairs: Vec<(PathBuf, PathBuf)>,
        moved: bool,
    },
    Create {
        path: PathBuf,
    },
    /// Specific items in the trash to put back (e.g. files replaced by a paste).
    Restore {
        items: Vec<trash::TrashItem>,
    },
    /// Several steps, undone last-first.
    Many(Vec<UndoOp>),
}

impl UndoOp {
    pub fn describe(&self) -> String {
        let n = |k: usize| format!("{k} item{}", if k == 1 { "" } else { "s" });
        match self {
            UndoOp::Rename { .. } => "rename".into(),
            UndoOp::Trash { originals, .. } => format!("move of {} to trash", n(originals.len())),
            UndoOp::Transfer { pairs, moved: true } => format!("move of {}", n(pairs.len())),
            UndoOp::Transfer { pairs, moved: false } => format!("copy of {}", n(pairs.len())),
            UndoOp::Create { .. } => "new item".into(),
            UndoOp::Restore { items } => format!("removal of {}", n(items.len())),
            UndoOp::Many(ops) => ops.last().map(UndoOp::describe).unwrap_or_default(),
        }
    }
}

/// Whether trashed items can be restored programmatically on this OS. macOS
/// doesn't let apps restore from the Trash (only Finder's "Put Back" can).
pub const CAN_RESTORE_TRASH: bool = cfg!(any(target_os = "linux", target_os = "windows"));

/// Reverses `op`. Never overwrites: if something now occupies the old location,
/// that item is skipped and reported.
pub fn undo(op: UndoOp) -> Result<String, String> {
    match op {
        UndoOp::Rename { from, to } => {
            ops::rename_noreplace(&to, &from).map_err(|e| format!("Couldn't undo rename: {e}"))?;
            Ok("Rename undone".into())
        }
        UndoOp::Create { path } => {
            ops::trash(std::slice::from_ref(&path))?;
            Ok("New item moved to trash".into())
        }
        UndoOp::Transfer { pairs, moved } => {
            let mut errors = Vec::new();
            if moved {
                for (src, dst) in pairs.iter().rev() {
                    if let Err(e) = ops::move_path(dst, src) {
                        errors.push(format!("{}: {e}", dst.display()));
                    }
                }
            } else {
                // Undoing a copy sends the copies to the trash (recoverable), never deletes.
                let copies: Vec<PathBuf> = pairs.into_iter().map(|(_, d)| d).collect();
                if let Err(e) = ops::trash(&copies) {
                    errors.push(e);
                }
            }
            if errors.is_empty() {
                Ok(if moved { "Move undone" } else { "Copies moved to trash" }.into())
            } else {
                Err(errors.join("; "))
            }
        }
        UndoOp::Trash { originals, when } => restore_from_trash(&originals, when),
        UndoOp::Restore { items } => restore_items(items),
        UndoOp::Many(ops) => {
            let mut msgs = Vec::new();
            for op in ops.into_iter().rev() {
                msgs.push(undo(op)?);
            }
            Ok(msgs.into_iter().next().unwrap_or_default())
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn restore_from_trash(originals: &[PathBuf], when: SystemTime) -> Result<String, String> {
    let since = when.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64 - 5).unwrap_or(0);
    let items = trash::os_limited::list().map_err(|e| e.to_string())?;
    // Newest match per original path, deleted at or after the operation.
    let mut picked: Vec<trash::TrashItem> = Vec::new();
    for orig in originals {
        let best = items
            .iter()
            .filter(|it| &it.original_path() == orig && it.time_deleted >= since)
            .max_by_key(|it| it.time_deleted);
        if let Some(it) = best {
            picked.push(it.clone());
        }
    }
    if picked.is_empty() {
        return Err("Couldn't find those items in the trash anymore".into());
    }
    let n = picked.len();
    trash::os_limited::restore_all(picked).map_err(|e| format!("Couldn't restore from trash: {e}"))?;
    Ok(format!("Restored {n} item{} from trash", if n == 1 { "" } else { "s" }))
}

/// Undo step for items just moved to the trash: pins the exact trash entries,
/// so a later trashing of the same path can't be restored by mistake.
pub fn trashed(originals: &[PathBuf], since: SystemTime) -> UndoOp {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        let from = since.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64 - 1).unwrap_or(0);
        if let Ok(all) = trash::os_limited::list() {
            let items: Vec<trash::TrashItem> = originals
                .iter()
                .filter_map(|o| {
                    all.iter()
                        .filter(|it| &it.original_path() == o && it.time_deleted >= from)
                        .max_by_key(|it| it.time_deleted)
                        .cloned()
                })
                .collect();
            if items.len() == originals.len() {
                return UndoOp::Restore { items };
            }
        }
    }
    UndoOp::Trash { originals: originals.to_vec(), when: since }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn restore_items(items: Vec<trash::TrashItem>) -> Result<String, String> {
    let n = items.len();
    trash::os_limited::restore_all(items).map_err(|e| format!("Couldn't restore from trash: {e}"))?;
    Ok(format!("Restored {n} item{} from trash", if n == 1 { "" } else { "s" }))
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn restore_items(_: Vec<trash::TrashItem>) -> Result<String, String> {
    Err("Restoring from the Trash isn't supported on this system; use Put Back in Finder".into())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn restore_from_trash(_: &[PathBuf], _: SystemTime) -> Result<String, String> {
    Err("Restoring from the Trash isn't supported on this system; use Put Back in Finder".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_rename_and_move() {
        let d = std::env::temp_dir().join(format!("file-flier-undo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.txt"), "x").unwrap();

        ops::rename_noreplace(&d.join("a.txt"), &d.join("b.txt")).unwrap();
        undo(UndoOp::Rename { from: d.join("a.txt"), to: d.join("b.txt") }).unwrap();
        assert!(d.join("a.txt").exists() && !d.join("b.txt").exists());

        ops::move_path(&d.join("a.txt"), &d.join("sub/a.txt")).unwrap();
        undo(UndoOp::Transfer { pairs: vec![(d.join("a.txt"), d.join("sub/a.txt"))], moved: true }).unwrap();
        assert!(d.join("a.txt").exists() && !d.join("sub/a.txt").exists());

        // Undo never clobbers something that took the old name meanwhile.
        ops::rename_noreplace(&d.join("a.txt"), &d.join("c.txt")).unwrap();
        std::fs::write(d.join("a.txt"), "new").unwrap();
        assert!(undo(UndoOp::Rename { from: d.join("a.txt"), to: d.join("c.txt") }).is_err());
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "new");
        std::fs::remove_dir_all(d).unwrap();
    }
}
