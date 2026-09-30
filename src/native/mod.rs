//! Native desktop integration winit doesn't provide: putting real files on the
//! clipboard (so other apps can paste them), dragging files out of the window, and
//! (on Wayland) receiving dropped files.

// The Linux backends share the MIME formats below; macOS uses AppKit directly.
#![cfg_attr(target_os = "macos", allow(dead_code))]

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;

use std::path::{Path, PathBuf};

/// What we offer to other apps.
pub struct Payload {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

impl Payload {
    fn uris(&self) -> Vec<String> {
        self.paths.iter().map(|p| file_uri(p)).collect()
    }

    /// The data for one MIME type / X11 target.
    pub fn bytes_for(&self, mime: &str) -> Vec<u8> {
        match mime {
            "text/uri-list" => self.uris().iter().map(|u| format!("{u}\r\n")).collect::<String>().into_bytes(),
            // GNOME Files, Nemo, Caja: "copy" or "cut", then one URI per line.
            "x-special/gnome-copied-files" => {
                let op = if self.cut { "cut" } else { "copy" };
                std::iter::once(op.to_string()).chain(self.uris()).collect::<Vec<_>>().join("\n").into_bytes()
            }
            // Dolphin: marks a cut.
            "application/x-kde-cutselection" => {
                if self.cut {
                    b"1".to_vec()
                } else {
                    b"0".to_vec()
                }
            }
            _ => self.paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n").into_bytes(),
        }
    }
}

pub fn file_uri(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~!$&'()*+,;=:@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        .map(|rest| {
            // Skip an optional host part ("file://localhost/...").
            let path = if rest.starts_with('/') { rest } else { rest.find('/').map_or(rest, |i| &rest[i..]) };
            PathBuf::from(crate::app::percent_decode(path))
        })
        .collect()
}

pub enum Cmd {
    Clipboard(Payload),
    Drag(Payload),
}

/// Files dropped onto our window from another app.
pub struct Dropped {
    pub paths: Vec<PathBuf>,
}

#[derive(Default)]
pub enum Native {
    #[cfg(target_os = "linux")]
    Wayland(wayland::Handle),
    #[cfg(target_os = "linux")]
    X11(x11::Handle),
    #[cfg(target_os = "macos")]
    MacOs(macos::Handle),
    #[default]
    Unsupported,
}

impl Native {
    pub fn new(frame: &eframe::Frame, ctx: &egui::Context) -> Self {
        #[cfg(target_os = "linux")]
        {
            use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
            let (Ok(d), Ok(w)) = (frame.display_handle(), frame.window_handle()) else { return Native::Unsupported };
            match (d.as_raw(), w.as_raw()) {
                (RawDisplayHandle::Wayland(d), RawWindowHandle::Wayland(w)) => {
                    // SAFETY: these are our window's live display and surface, which
                    // outlive the app state that owns this handle.
                    if let Some(h) = unsafe { wayland::start(d.display.as_ptr(), w.surface.as_ptr(), ctx.clone()) } {
                        return Native::Wayland(h);
                    }
                }
                (_, RawWindowHandle::Xlib(w)) => {
                    if let Some(h) = x11::start(w.window as u32) {
                        return Native::X11(h);
                    }
                }
                (_, RawWindowHandle::Xcb(w)) => {
                    if let Some(h) = x11::start(w.window.get()) {
                        return Native::X11(h);
                    }
                }
                _ => {}
            }
        }
        #[cfg(target_os = "macos")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(w) = frame.window_handle()
                && let RawWindowHandle::AppKit(h) = w.as_raw()
                // SAFETY: our window's live NSView.
                && let Some(h) = unsafe { macos::Handle::new(h.ns_view.as_ptr()) }
            {
                return Native::MacOs(h);
            }
        }
        let _ = (frame, ctx);
        Native::Unsupported
    }

    fn send(&self, cmd: Cmd) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Native::Wayland(h) => h.send(cmd),
            #[cfg(target_os = "linux")]
            Native::X11(h) => h.send(cmd),
            #[cfg(target_os = "macos")]
            Native::MacOs(_) => unreachable!("macOS is handled directly"),
            Native::Unsupported => {
                let _ = cmd;
                false
            }
        }
    }

    /// Puts files on the system clipboard. Returns false if unsupported.
    pub fn set_clipboard(&self, paths: &[PathBuf], cut: bool) -> bool {
        #[cfg(target_os = "macos")]
        if let Native::MacOs(h) = self {
            return h.set_clipboard(paths); // Finder has no "cut" for files; pasting copies
        }
        self.send(Cmd::Clipboard(Payload { paths: paths.to_vec(), cut }))
    }

    /// Hands a drag that left the window over to the desktop.
    pub fn start_drag(&mut self, paths: &[PathBuf]) -> bool {
        #[cfg(target_os = "macos")]
        if let Native::MacOs(h) = self {
            return h.start_drag(paths);
        }
        self.send(Cmd::Drag(Payload { paths: paths.to_vec(), cut: false }))
    }

    pub fn take_drops(&self) -> Vec<Dropped> {
        match self {
            #[cfg(target_os = "linux")]
            Native::Wayland(h) => std::mem::take(&mut *h.drops.lock().unwrap()),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_formats() {
        let p = Payload { paths: vec!["/home/me/a b.txt".into(), "/tmp/x".into()], cut: true };
        assert_eq!(p.bytes_for("text/uri-list"), b"file:///home/me/a%20b.txt\r\nfile:///tmp/x\r\n");
        assert_eq!(p.bytes_for("x-special/gnome-copied-files"), b"cut\nfile:///home/me/a%20b.txt\nfile:///tmp/x");
        assert_eq!(p.bytes_for("UTF8_STRING"), b"/home/me/a b.txt\n/tmp/x");
        assert_eq!(
            parse_uri_list("# comment\r\nfile:///home/me/a%20b.txt\r\nfile://localhost/tmp/x\r\nhttp://no\r\n"),
            vec![PathBuf::from("/home/me/a b.txt"), PathBuf::from("/tmp/x")]
        );
    }
}
