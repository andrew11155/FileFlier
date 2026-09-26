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
    let start = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("File Flier")
            .with_app_id("file-flier")
            .with_decorations(false)
            .with_inner_size([1180.0, 740.0])
            .with_min_inner_size([560.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native("File Flier", options, Box::new(move |cc| Ok(Box::new(app::FileFlier::new(cc, start)))))
}
