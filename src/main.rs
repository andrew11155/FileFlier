//! File Flier — a fast, keyboard-driven file manager for Linux, inspired by File Pilot.
#![allow(clippy::enum_variant_names)]

mod app;
mod commands;
mod config;
mod fs_model;
mod fuzzy;
mod icons;
mod ops;
mod pane;
mod search;
mod theme;
mod ui;

fn main() -> eframe::Result {
    if ops::in_flatpak() {
        // Inside Flatpak, XDG_DATA_HOME points into the sandbox (~/.var/app/...), so trashed
        // files would land in a private trash. Use the real ~/.local/share, which the
        // manifest grants access to, so the desktop's trash sees them.
        let host = std::env::var_os("HOST_XDG_DATA_HOME")
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".local/share").into()));
        if let Some(host) = host {
            // SAFETY: called at startup before any other threads exist.
            unsafe { std::env::set_var("XDG_DATA_HOME", host) };
        }
    }
    let start = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("File Flier")
            .with_app_id("io.github.andrew11155.FileFlier")
            .with_decorations(false)
            .with_inner_size([1180.0, 740.0])
            .with_min_inner_size([560.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native("File Flier", options, Box::new(move |cc| Ok(Box::new(app::FileFlier::new(cc, start)))))
}
