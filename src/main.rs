//! File Flier — a fast, keyboard-driven file manager for Linux and macOS, inspired by File Pilot.
#![allow(clippy::enum_variant_names)]

mod app;
mod archive;
mod cloud;
mod commands;
mod config;
mod counts;
mod fs_model;
mod fuzzy;
mod icons;
mod mounts;
mod native;
mod net;
mod openwith;
mod ops;
mod pane;
mod preview;
mod search;
mod theme;
mod trashview;
mod udisks;
mod ui;
mod undo;
mod updater;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--preview-helper") {
        std::process::exit(preview::helper::main(&args[2..]));
    }
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
    #[cfg(target_os = "macos")]
    {
        // Apps opened from Finder or the Dock get a minimal PATH; add Homebrew's so
        // tools like ffmpeg are found for previews.
        let path = std::env::var("PATH").unwrap_or_default();
        let mut parts: Vec<&str> = path.split(':').filter(|p| !p.is_empty()).collect();
        for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
            if !parts.contains(&extra) {
                parts.push(extra);
            }
        }
        // SAFETY: called at startup before any other threads exist.
        unsafe { std::env::set_var("PATH", parts.join(":")) };
    }
    let start = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    // Only ask for a transparent window when "see-through" glass is actually on:
    // some drivers (e.g. certain NVIDIA/X11 setups) misbehave with transparent windows.
    let transparent = config::Config::load().see_through();
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("File Flier")
            .with_app_id("io.github.andrew11155.FileFlier")
            .with_transparent(transparent)
            .with_inner_size([1180.0, 740.0])
            .with_min_inner_size([560.0, 360.0]),
        ..Default::default()
    };
    // Linux: our own title bar and window buttons. macOS: the native window with its
    // traffic-light buttons, and our title bar drawn underneath a transparent one.
    #[cfg(target_os = "macos")]
    {
        options.viewport = options
            .viewport
            .with_decorations(true)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    #[cfg(not(target_os = "macos"))]
    {
        options.viewport = options.viewport.with_decorations(false);
    }
    eframe::run_native(
        "File Flier",
        options,
        Box::new(move |cc| Ok(Box::new(app::FileFlier::new(cc, start, transparent)))),
    )
}
