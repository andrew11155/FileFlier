//! The update dialog: release notes, download progress and the final step.

use std::sync::atomic::Ordering;

use egui::{Color32, RichText, Sense, Ui, vec2};

use super::popups::{button, label};
use crate::app::{FileFlier, human_size};
use crate::updater::{self, Install, Outcome};

enum Step {
    Update,
    OpenPage(String),
    Skip(String),
    Restart(std::path::PathBuf),
}

impl FileFlier {
    /// Draws the dialog body. Returns true when it should close.
    pub(crate) fn update_ui(&mut self, ui: &mut Ui) -> bool {
        let pal = self.pal();
        let ctx = ui.ctx().clone();
        // Skipped versions are still shown here when the user opens the dialog from Settings.
        let updater::Status::Available(release) = &self.updater.status else {
            return true;
        };
        let release = release.clone();
        let mut close = false;
        let mut step = None;

        label(ui, pal, &format!("File Flier {} is available", release.version), 16.0, Some(pal.text_strong));
        label(ui, pal, &format!("You have {}", updater::VERSION), 13.0, Some(pal.text_dim));

        match &self.updater.install {
            Install::Running(progress) => {
                let done = progress.done.load(Ordering::Relaxed);
                let total = progress.total.load(Ordering::Relaxed);
                let installing = progress.installing.load(Ordering::Relaxed);
                ui.add_space(10.0);
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
                ui.painter().rect_filled(r, 4.0, pal.input);
                let frac = if installing {
                    1.0
                } else if total > 0 {
                    (done as f32 / total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let mut fill = r;
                fill.set_width(r.width() * frac);
                ui.painter().rect_filled(fill, 4.0, pal.accent);
                ui.add_space(6.0);
                let status = if installing {
                    "Installing…".to_string()
                } else if total > 0 {
                    format!("Downloading… {} of {}", human_size(done), human_size(total))
                } else {
                    "Downloading…".to_string()
                };
                label(ui, pal, &status, 13.0, Some(pal.text_dim));
                ui.add_space(6.0);
                if button(ui, pal, "Hide", false, false).clicked() {
                    close = true; // keeps downloading; the result arrives as a toast
                }
            }
            Install::Done(outcome) => {
                ui.add_space(6.0);
                match outcome {
                    Outcome::Restart(exe) => {
                        label(ui, pal, "The update is installed. Restart File Flier to start using it.", 13.5, None);
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            if button(ui, pal, "Restart now", true, false).clicked() {
                                step = Some(Step::Restart(exe.clone()));
                            }
                            if button(ui, pal, "Later", false, false).clicked() {
                                close = true;
                            }
                        });
                    }
                    Outcome::Message(msg) => {
                        ui.label(RichText::new(msg).size(13.5).color(pal.text));
                        ui.add_space(8.0);
                        if button(ui, pal, "Close", false, false).clicked() {
                            close = true;
                        }
                    }
                }
            }
            idle => {
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                    let notes = if release.notes.is_empty() { "No release notes." } else { release.notes.as_str() };
                    ui.label(RichText::new(notes).size(13.0).color(pal.text));
                });
                if let Install::Failed(e) = idle {
                    ui.add_space(8.0);
                    ui.label(RichText::new(e).size(13.0).color(Color32::from_rgb(255, 140, 130)));
                }
                let can_install = updater::select_asset(&release, &self.updater.kind).is_some();
                if !can_install {
                    ui.add_space(8.0);
                    let why = match self.updater.kind {
                        updater::InstallKind::Unmanaged => {
                            "This copy is managed by your system: update it with your package manager, or download the release."
                        }
                        _ => {
                            "This release has no download for your installation. You can get it from the release page."
                        }
                    };
                    ui.label(RichText::new(why).size(13.0).color(pal.text_dim));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if can_install {
                        if button(ui, pal, "Update", true, false).clicked() {
                            step = Some(Step::Update);
                        }
                    } else if button(ui, pal, "Open release page", true, false).clicked() {
                        step = Some(Step::OpenPage(release.page.clone()));
                    }
                    if button(ui, pal, "Later", false, false).clicked() {
                        close = true;
                    }
                    if button(ui, pal, "Skip this version", false, false).clicked() {
                        step = Some(Step::Skip(release.version.clone()));
                    }
                });
            }
        }

        match step {
            Some(Step::Update) => self.updater.start_install(&release, &ctx),
            Some(Step::OpenPage(url)) => {
                if let Err(e) = open::that_detached(&url) {
                    self.error(format!("Could not open {url}: {e}"));
                }
            }
            Some(Step::Skip(v)) => {
                self.cfg.skipped_version = Some(v);
                self.cfg.save();
                close = true;
            }
            Some(Step::Restart(exe)) => {
                self.cfg.save();
                let e = updater::restart(&exe);
                self.error(e);
            }
            None => {}
        }
        close
    }
}
