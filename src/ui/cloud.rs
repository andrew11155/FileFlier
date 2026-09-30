//! "Add cloud storage": pick a provider, sign in with the browser, done.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use egui::{Align2, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::popups::{button, label};
use crate::app::FileFlier;
use crate::cloud::{self, Provider, Rclone, Setup, SignIn};

enum Stage {
    /// Looking for rclone and FUSE (runs commands, so on a thread).
    Checking(Arc<Mutex<Option<Setup>>>),
    Choose(Rclone),
    /// Google Drive: GNOME Online Accounts or rclone, optionally with your own client ID.
    Google {
        rclone: Rclone,
        id: String,
        secret: String,
    },
    /// rclone isn't installed: offer to download it.
    Offer,
    Downloading {
        progress: Arc<Mutex<String>>,
        result: Arc<Mutex<Option<Result<(), String>>>>,
    },
    Blocked(Setup),
    SigningIn(SignIn),
    Failed {
        message: String,
        rclone: Option<Rclone>,
    },
}

pub struct CloudDialog {
    stage: Stage,
    /// GNOME Settings is around, so GNOME Online Accounts can be offered too.
    gnome: bool,
}

impl CloudDialog {
    pub fn new(ctx: &egui::Context) -> Self {
        CloudDialog { stage: check(ctx), gnome: gnome_settings_available() }
    }
}

impl Drop for CloudDialog {
    /// Closing the dialog mid sign-in stops it.
    fn drop(&mut self) {
        if let Stage::SigningIn(s) = &self.stage {
            s.cancel.store(true, Ordering::SeqCst);
        }
    }
}

fn check(ctx: &egui::Context) -> Stage {
    let slot = Arc::new(Mutex::new(None));
    let (s, c) = (slot.clone(), ctx.clone());
    std::thread::spawn(move || {
        *s.lock().unwrap() = Some(cloud::setup());
        c.request_repaint();
    });
    Stage::Checking(slot)
}

fn gnome_settings_available() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("GNOME"))
}

fn open_gnome_accounts() {
    let args = ["gnome-control-center", "online-accounts"];
    let _ = if crate::ops::in_flatpak() {
        std::process::Command::new("flatpak-spawn").arg("--host").args(args).spawn()
    } else {
        std::process::Command::new(args[0]).arg(args[1]).spawn()
    };
}

enum Next {
    Stage(Stage),
    Done(cloud::Account),
    Close,
}

impl FileFlier {
    /// Draws the dialog. Returns true when it should close.
    pub(crate) fn cloud_ui(&mut self, ui: &mut Ui, d: &mut CloudDialog) -> bool {
        let pal = self.pal();
        let ctx = ui.ctx().clone();
        let mut next: Option<Next> = None;
        label(ui, pal, "Add cloud storage", 16.0, Some(pal.text_strong));

        let spinner = |ui: &mut Ui, msg: &str| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(16.0).color(pal.text_dim));
                ui.add_space(6.0);
                label(ui, pal, msg, 13.5, Some(pal.text_dim));
            });
            ui.add_space(8.0);
        };

        match &mut d.stage {
            Stage::Checking(slot) => {
                spinner(ui, "Getting ready…");
                if let Some(setup) = slot.lock().unwrap().take() {
                    next = Some(Next::Stage(match setup {
                        Setup::Ready(r) => Stage::Choose(r),
                        Setup::NeedsDownload => Stage::Offer,
                        other => Stage::Blocked(other),
                    }));
                }
            }
            Stage::Choose(rclone) => {
                label(
                    ui,
                    pal,
                    "You'll sign in on the provider's own page in your web browser.",
                    13.5,
                    Some(pal.text_dim),
                );
                ui.add_space(8.0);
                for p in Provider::ALL {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
                    let painter = ui.painter();
                    painter.rect_filled(r, 8.0, if resp.hovered() { pal.tab_hover } else { pal.input });
                    painter.rect_stroke(
                        r,
                        8.0,
                        Stroke::new(1.0, if resp.hovered() { pal.accent } else { pal.border }),
                        StrokeKind::Inside,
                    );
                    provider_logo(
                        painter,
                        egui::Rect::from_center_size(pos2(r.left() + 30.0, r.center().y), vec2(26.0, 26.0)),
                        p,
                    );
                    super::text_bold(
                        painter,
                        pos2(r.left() + 56.0, r.center().y - 8.0),
                        Align2::LEFT_CENTER,
                        p.name(),
                        14.5,
                        pal.text_strong,
                    );
                    let sub = match p {
                        Provider::GoogleDrive => "Google account, including Google Workspace",
                        Provider::OneDrive => "Microsoft account, personal or work/school",
                        Provider::Dropbox => "Dropbox account",
                    };
                    super::text(
                        painter,
                        pos2(r.left() + 56.0, r.center().y + 10.0),
                        Align2::LEFT_CENTER,
                        sub,
                        12.0,
                        pal.text_dim,
                    );
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        next = Some(Next::Stage(if p == Provider::GoogleDrive {
                            Stage::Google { rclone: rclone.clone(), id: String::new(), secret: String::new() }
                        } else {
                            Stage::SigningIn(SignIn::start(rclone.clone(), p, None, ctx.clone()))
                        }));
                    }
                    ui.add_space(8.0);
                }
                label(
                    ui,
                    pal,
                    "File Flier never sees your password. Files open straight from the cloud and are cached while you use them.",
                    12.0,
                    Some(pal.text_faint),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if button(ui, pal, "Cancel", false, false).clicked() {
                        next = Some(Next::Close);
                    }
                    if d.gnome && button(ui, pal, "Use GNOME Online Accounts instead", false, false).clicked() {
                        open_gnome_accounts();
                        next = Some(Next::Close);
                    }
                });
            }
            Stage::Google { rclone, id, secret } => {
                if d.gnome {
                    label(
                        ui,
                        pal,
                        "On GNOME, the most reliable way to add Google Drive is GNOME Online Accounts: it uses Google's own sign-in, and the drive shows up here automatically.",
                        13.5,
                        None,
                    );
                    ui.add_space(4.0);
                    if button(ui, pal, "Open GNOME Online Accounts", true, false).clicked() {
                        open_gnome_accounts();
                        next = Some(Next::Close);
                    }
                    ui.add_space(10.0);
                }
                label(
                    ui,
                    pal,
                    "Google is retiring the shared sign-in that rclone uses for Google Drive during 2026. It still works for now. If signing in fails, create your own free Google client ID and paste it below (optional):",
                    13.0,
                    Some(pal.text_dim),
                );
                if ui.link("How to make your own client ID (about 5 minutes)").clicked() {
                    let _ = open::that("https://rclone.org/drive/#making-your-own-client-id");
                }
                ui.add_space(6.0);
                let field = |ui: &mut Ui, v: &mut String, hint: &str| {
                    ui.add(
                        egui::TextEdit::singleline(v)
                            .hint_text(hint)
                            .desired_width(f32::INFINITY)
                            .font(super::font(13.0))
                            .margin(vec2(8.0, 6.0)),
                    );
                    ui.add_space(4.0);
                };
                field(ui, id, "Client ID (optional)");
                field(ui, secret, "Client secret (optional)");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let own = !id.trim().is_empty() && !secret.trim().is_empty();
                    if button(ui, pal, "Sign in with Google", !d.gnome, false).clicked() {
                        let client = own.then(|| (id.clone(), secret.clone()));
                        next = Some(Next::Stage(Stage::SigningIn(SignIn::start(
                            rclone.clone(),
                            Provider::GoogleDrive,
                            client,
                            ctx.clone(),
                        ))));
                    }
                    if button(ui, pal, "Back", false, false).clicked() {
                        next = Some(Next::Stage(Stage::Choose(rclone.clone())));
                    }
                });
            }
            Stage::Offer => {
                label(
                    ui,
                    pal,
                    "File Flier connects to cloud storage with rclone, a free and widely used open-source tool. It isn't installed yet.",
                    13.5,
                    None,
                );
                label(
                    ui,
                    pal,
                    "Download the official build from rclone.org now? It's about 25 MB and is checked against rclone's published checksum.",
                    13.0,
                    Some(pal.text_dim),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if button(ui, pal, "Download rclone", true, false).clicked() {
                        let progress = Arc::new(Mutex::new(String::new()));
                        let result = Arc::new(Mutex::new(None));
                        let (p, r, c) = (progress.clone(), result.clone(), ctx.clone());
                        std::thread::spawn(move || {
                            let res = cloud::download(&p);
                            *r.lock().unwrap() = Some(res);
                            c.request_repaint();
                        });
                        next = Some(Next::Stage(Stage::Downloading { progress, result }));
                    }
                    if button(ui, pal, "Cancel", false, false).clicked() {
                        next = Some(Next::Close);
                    }
                });
            }
            Stage::Downloading { progress, result } => {
                let msg = progress.lock().unwrap().clone();
                spinner(ui, if msg.is_empty() { "Downloading rclone…" } else { &msg });
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                if let Some(res) = result.lock().unwrap().take() {
                    next = Some(Next::Stage(match res {
                        Ok(()) => check(&ctx),
                        Err(message) => Stage::Failed { message, rclone: None },
                    }));
                }
            }
            Stage::Blocked(setup) => {
                match setup {
                    Setup::NeedsHostAccess(cmd) => {
                        label(
                            ui,
                            pal,
                            "Cloud drives are connected by rclone on your system, outside File Flier's Flatpak sandbox. Allow that once by running this in a terminal, then restart File Flier:",
                            13.5,
                            None,
                        );
                        ui.add_space(4.0);
                        code_box(ui, pal, cmd);
                    }
                    _ => {
                        label(
                            ui,
                            pal,
                            "Cloud drives need FUSE, which isn't installed. Install the \"fuse3\" package, then try again:",
                            13.5,
                            None,
                        );
                        ui.add_space(4.0);
                        code_box(
                            ui,
                            pal,
                            "sudo dnf install fuse3      # Fedora\nsudo apt install fuse3      # Ubuntu, Debian",
                        );
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if button(ui, pal, "Try again", true, false).clicked() {
                        next = Some(Next::Stage(check(&ctx)));
                    }
                    if button(ui, pal, "Close", false, false).clicked() {
                        next = Some(Next::Close);
                    }
                    if d.gnome && button(ui, pal, "Use GNOME Online Accounts", false, false).clicked() {
                        open_gnome_accounts();
                        next = Some(Next::Close);
                    }
                });
            }
            Stage::SigningIn(s) => {
                spinner(ui, &format!("Waiting for you to sign in to {} in your browser…", s.provider.name()));
                label(ui, pal, "Come back here when the browser says you're done.", 12.5, Some(pal.text_faint));
                ui.add_space(8.0);
                let url = s.url.lock().unwrap().clone();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if let Some(u) = &url
                        && button(ui, pal, "Open the browser again", false, false).clicked()
                    {
                        let _ = open::that(u);
                    }
                    if button(ui, pal, "Cancel", false, false).clicked() {
                        s.cancel.store(true, Ordering::SeqCst);
                    }
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
                let done = s.result.lock().unwrap().take();
                match done {
                    Some(Ok(a)) => next = Some(Next::Done(a)),
                    Some(Err(e)) if s.cancel.load(Ordering::SeqCst) => {
                        let _ = e;
                        next = Some(Next::Close);
                    }
                    Some(Err(message)) => {
                        let rclone = match cloud::setup() {
                            Setup::Ready(r) => Some(r),
                            _ => None,
                        };
                        next = Some(Next::Stage(Stage::Failed { message, rclone }));
                    }
                    None => {}
                }
            }
            Stage::Failed { message, rclone } => {
                label(ui, pal, "That didn't work.", 13.5, None);
                label(ui, pal, message, 13.0, Some(egui::Color32::from_rgb(230, 120, 110)));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if button(ui, pal, "Try again", true, false).clicked() {
                        next = Some(Next::Stage(match rclone.take() {
                            Some(r) => Stage::Choose(r),
                            None => check(&ctx),
                        }));
                    }
                    if button(ui, pal, "Close", false, false).clicked() {
                        next = Some(Next::Close);
                    }
                });
            }
        }

        match next {
            Some(Next::Stage(s)) => {
                d.stage = s;
                false
            }
            Some(Next::Done(a)) => {
                self.info(format!("{} connected", a.name));
                self.navigate(a.mount);
                true
            }
            Some(Next::Close) => true,
            None => false,
        }
    }

    /// Unmounts a cloud account; with `sign_out`, also forgets it.
    pub(crate) fn cloud_disconnect(&mut self, name: String, sign_out: bool) {
        let Some(a) = cloud::accounts().into_iter().find(|a| a.name == name) else { return };
        // Leave the drive first so it isn't busy.
        for i in 0..2 {
            if self.panes[i].tab().path.starts_with(&a.mount)
                && let Some(home) = dirs::home_dir()
            {
                self.panes[i].tab_mut().navigate(home, self.cfg.show_hidden, self.cfg.sort);
            }
        }
        let label = if sign_out { format!("Signing out of {name}") } else { format!("Disconnecting {name}") };
        self.start_job(label, move |_| {
            if sign_out {
                match cloud::setup() {
                    Setup::Ready(r) => r.remove(&a)?,
                    _ => return Err("rclone isn't available to sign out".into()),
                }
                Ok((format!("Signed out of {name}. Your files are still in the cloud."), None))
            } else {
                cloud::unmount(&a.mount)?;
                Ok((format!("{name} disconnected"), None))
            }
        });
    }
}

/// Monospace, selectable text with a Copy button.
fn code_box(ui: &mut Ui, pal: &crate::theme::Palette, s: &str) {
    egui::Frame::new().fill(pal.input).corner_radius(6).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new(s).monospace().size(12.5).color(pal.text));
    });
    ui.add_space(4.0);
    if button(ui, pal, "Copy", false, false).clicked() {
        ui.ctx()
            .copy_text(s.lines().map(|l| l.split('#').next().unwrap_or(l).trim_end()).collect::<Vec<_>>().join("\n"));
    }
}

/// Simple marks in each provider's colors (not their trademarked logos).
fn provider_logo(p: &egui::Painter, r: egui::Rect, provider: Provider) {
    use egui::Color32;
    let at = |x: f32, y: f32| pos2(r.left() + r.width() * x, r.top() + r.height() * y);
    let poly = |pts: Vec<egui::Pos2>, c: Color32| {
        p.add(egui::epaint::PathShape::convex_polygon(pts, c, Stroke::NONE));
    };
    match provider {
        Provider::GoogleDrive => {
            poly(vec![at(0.36, 0.1), at(0.64, 0.1), at(0.34, 0.62), at(0.2, 0.38)], Color32::from_rgb(15, 157, 88));
            poly(vec![at(0.64, 0.1), at(0.96, 0.66), at(0.68, 0.66), at(0.5, 0.36)], Color32::from_rgb(255, 196, 0));
            poly(vec![at(0.04, 0.66), at(0.96, 0.66), at(0.82, 0.9), at(0.18, 0.9)], Color32::from_rgb(66, 133, 244));
        }
        Provider::OneDrive => {
            let c = Color32::from_rgb(20, 114, 222);
            p.circle_filled(at(0.36, 0.58), r.width() * 0.2, c);
            p.circle_filled(at(0.58, 0.44), r.width() * 0.26, c);
            p.circle_filled(at(0.78, 0.62), r.width() * 0.17, c);
            p.rect_filled(egui::Rect::from_min_max(at(0.18, 0.6), at(0.9, 0.79)), 4.0, c);
        }
        Provider::Dropbox => {
            let c = Color32::from_rgb(0, 97, 254);
            for (cx, cy) in [(0.3, 0.3), (0.7, 0.3), (0.3, 0.62), (0.7, 0.62)] {
                let s = 0.2;
                poly(vec![at(cx, cy - s * 0.8), at(cx + s, cy), at(cx, cy + s * 0.8), at(cx - s, cy)], c);
            }
        }
    }
}
