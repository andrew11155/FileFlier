//! Alt+double-click: the desktop's Properties window for an item (GNOME Files' on
//! GNOME), asked for over the freedesktop `org.freedesktop.FileManager1` D-Bus
//! interface, as other apps do.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Items whose Properties window couldn't be opened (the app shows its own info instead).
static FAILED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Opens the Properties window for `path`, in the background (the file manager may
/// have to start first).
pub fn show(path: PathBuf, ctx: &egui::Context) {
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        if call(&path).is_err() {
            if let Ok(mut f) = FAILED.lock() {
                f.push(path);
            }
            ctx.request_repaint();
        }
    });
}

fn call(path: &Path) -> zbus::Result<()> {
    let conn = zbus::blocking::Connection::session()?;
    conn.call_method(
        Some("org.freedesktop.FileManager1"),
        "/org/freedesktop/FileManager1",
        Some("org.freedesktop.FileManager1"),
        "ShowItemProperties",
        &(vec![crate::native::file_uri(path)], ""),
    )?;
    Ok(())
}

/// Items that failed since the last call.
pub fn failed() -> Vec<PathBuf> {
    FAILED.lock().map(|mut f| std::mem::take(&mut *f)).unwrap_or_default()
}
