//! Self-update from GitHub releases. Checks the latest release, and can install it:
//! a plain binary replaces itself, a Flatpak downloads its bundle and hands it to
//! the host's `flatpak`. Everything runs on worker threads; the UI only polls.
//!
//! The only data sent is a `User-Agent: file-flier/<version>` header. Builds that
//! set `FILE_FLIER_NO_UPDATER` at compile time (e.g. for Flathub) omit all of it.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::config::Config;

/// False when built with `FILE_FLIER_NO_UPDATER` set: no checks, no UI.
pub const ENABLED: bool = option_env!("FILE_FLIER_NO_UPDATER").is_none();
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const API_URL: &str = "https://api.github.com/repos/andrew11155/File-Flier/releases/latest";
const BINARY_ASSET: &str = "file-flier-x86_64-linux.tar.gz";
const FLATPAK_ASSET: &str = "file-flier.flatpak";
/// macOS app bundle (universal: Apple silicon and Intel), zipped with `ditto`.
const MAC_ASSET: &str = "File-Flier-macos.zip";
/// Refuse downloads larger than this.
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallKind {
    /// Running in a Flatpak sandbox.
    Flatpak,
    /// A user-writable binary that can replace itself.
    Binary,
    /// A macOS app bundle in a folder we can write to (e.g. /Applications).
    MacApp(PathBuf),
    /// Installed by a package manager or a build tree; only the release page is offered.
    Unmanaged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex SHA-256 from GitHub's `digest` field, when it provides one.
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Version without the leading `v`.
    pub version: String,
    pub title: String,
    pub notes: String,
    pub page: String,
    pub assets: Vec<Asset>,
}

/// How an installed update finishes.
#[derive(Clone, Debug)]
pub enum Outcome {
    /// The binary was replaced; restart `PathBuf` to run it.
    Restart(PathBuf),
    /// Anything else, shown to the user as is.
    Message(String),
}

/// Download progress shared with the UI.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    /// Set once the download is complete and the update is being applied.
    pub installing: std::sync::atomic::AtomicBool,
}

pub enum Status {
    Idle,
    Checking,
    UpToDate,
    Available(Box<Release>),
    Failed(String),
}

pub enum Install {
    Idle,
    Running(Arc<Progress>),
    Done(Outcome),
    Failed(String),
}

enum Msg {
    Checked(Result<Release, String>),
    Installed(Result<Outcome, String>),
}

pub struct Updater {
    pub status: Status,
    pub install: Install,
    pub kind: InstallKind,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    auto_done: bool,
}

impl Updater {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self { status: Status::Idle, install: Install::Idle, kind: install_kind(), tx, rx, auto_done: false }
    }

    /// The release to offer, unless the user skipped it.
    pub fn offered<'a>(&'a self, cfg: &Config) -> Option<&'a Release> {
        match &self.status {
            Status::Available(r) if cfg.skipped_version.as_deref() != Some(r.version.as_str()) => Some(r),
            _ => None,
        }
    }

    /// Whether a downloaded update is waiting for a restart.
    pub fn restart_pending(&self) -> bool {
        matches!(self.install, Install::Done(Outcome::Restart(_)))
    }

    pub fn checking(&self) -> bool {
        matches!(self.status, Status::Checking)
    }

    pub fn installing(&self) -> bool {
        matches!(self.install, Install::Running(_))
    }

    pub fn check(&mut self, ctx: &egui::Context) {
        if !ENABLED || self.checking() {
            return;
        }
        self.status = Status::Checking;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Checked(fetch_latest()));
            ctx.request_repaint();
        });
    }

    /// Checks shortly after startup, at most once a day, if the user hasn't turned it off.
    pub fn auto_check(&mut self, cfg: &mut Config, ctx: &egui::Context) {
        if self.auto_done || !ENABLED || !cfg.check_updates {
            return;
        }
        if ctx.input(|i| i.time) < 4.0 {
            ctx.request_repaint_after(Duration::from_secs(4));
            return;
        }
        self.auto_done = true;
        if now_secs().saturating_sub(cfg.last_update_check) >= CHECK_INTERVAL_SECS {
            self.check(ctx);
        }
    }

    /// Downloads and applies `release` on a worker thread.
    pub fn start_install(&mut self, release: &Release, ctx: &egui::Context) {
        if self.installing() {
            return;
        }
        let progress = Arc::new(Progress::default());
        self.install = Install::Running(progress.clone());
        let (tx, ctx, release, kind) = (self.tx.clone(), ctx.clone(), release.clone(), self.kind.clone());
        std::thread::spawn(move || {
            let ctx2 = ctx.clone();
            let res = apply(&release, &kind, &progress, &move || ctx2.request_repaint());
            let _ = tx.send(Msg::Installed(res));
            ctx.request_repaint();
        });
    }

    /// Collects finished work. Returns a message (and whether it's an error) to toast.
    pub fn poll(&mut self, cfg: &mut Config) -> Vec<(String, bool)> {
        let mut notices = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Checked(res) => {
                    cfg.last_update_check = now_secs();
                    cfg.save();
                    self.status = match res {
                        Ok(r) if is_newer(&r.version, VERSION) => Status::Available(Box::new(r)),
                        Ok(_) => Status::UpToDate,
                        Err(e) => Status::Failed(e),
                    };
                }
                Msg::Installed(Ok(outcome)) => {
                    notices.push(match &outcome {
                        Outcome::Restart(_) => ("Update installed. Restart File Flier to use it.".to_string(), false),
                        Outcome::Message(m) => (m.clone(), false),
                    });
                    self.install = Install::Done(outcome);
                }
                Msg::Installed(Err(e)) => {
                    notices.push((format!("Update failed: {e}"), true));
                    self.install = Install::Failed(e);
                }
            }
        }
        notices
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------- versions

/// Parses `v1.2.3` / `1.2` (missing parts are 0). Ignores any `-pre` or `+build` suffix.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let mut next = |required: bool| match parts.next() {
        Some(p) => p.parse::<u64>().ok().map(Some),
        None => (!required).then_some(None),
    };
    let major = next(true)?.unwrap_or(0);
    let minor = next(false)?.unwrap_or(0);
    let patch = next(false)?.unwrap_or(0);
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether `latest` is a higher version than `current`. Unparseable versions never are.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

// ---------------------------------------------------------------- install type

pub fn install_kind() -> InstallKind {
    if Path::new("/.flatpak-info").exists() {
        return InstallKind::Flatpak;
    }
    let Ok(exe) = std::env::current_exe() else { return InstallKind::Unmanaged };
    let writable = |d: &Path| {
        std::ffi::CString::new(d.as_os_str().as_encoded_bytes())
            // SAFETY: a valid NUL-terminated path; `access` only reads it.
            .is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
    };
    if cfg!(target_os = "macos") {
        return match app_bundle(&exe) {
            Some(app) if app.parent().is_some_and(writable) => InstallKind::MacApp(app),
            _ => InstallKind::Unmanaged,
        };
    }
    classify(&exe, exe.parent().is_some_and(writable))
}

/// The `.app` bundle an executable lives in (`X.app/Contents/MacOS/x`).
fn app_bundle(exe: &Path) -> Option<PathBuf> {
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension().is_some_and(|e| e == "app") && exe.parent()?.ends_with("Contents/MacOS"))
        .then(|| app.to_path_buf())
}

/// Package-manager paths and cargo build trees are left alone.
fn classify(exe: &Path, dir_writable: bool) -> InstallKind {
    const SYSTEM: [&str; 6] = ["/usr", "/bin", "/sbin", "/opt", "/nix", "/snap"];
    let comps: Vec<_> = exe.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let in_build_tree = comps.windows(2).any(|w| w[0] == "target" && (w[1] == "debug" || w[1] == "release"));
    if !dir_writable || in_build_tree || SYSTEM.iter().any(|p| exe.starts_with(p)) {
        InstallKind::Unmanaged
    } else {
        InstallKind::Binary
    }
}

// ---------------------------------------------------------------- release info

fn agent(global_timeout: Duration) -> ureq::Agent {
    crate::net::agent(global_timeout)
}

fn fetch_latest() -> Result<Release, String> {
    let mut resp = agent(Duration::from_secs(30))
        .get(API_URL)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Could not reach GitHub: {e}"))?;
    let body = resp.body_mut().read_to_string().map_err(|e| format!("Could not read GitHub's reply: {e}"))?;
    parse_release(&body)
}

#[derive(serde::Deserialize)]
struct RawRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(serde::Deserialize)]
struct RawAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    digest: Option<String>,
}

pub fn parse_release(json: &str) -> Result<Release, String> {
    let raw: RawRelease = serde_json::from_str(json).map_err(|e| format!("Unexpected reply from GitHub: {e}"))?;
    parse_version(&raw.tag_name).ok_or_else(|| format!("Unrecognized version tag \"{}\"", raw.tag_name))?;
    let version = raw.tag_name.trim().trim_start_matches(['v', 'V']).to_string();
    Ok(Release {
        title: raw.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| raw.tag_name.clone()),
        notes: clean_notes(raw.body.as_deref().unwrap_or("")),
        page: raw.html_url,
        assets: raw
            .assets
            .into_iter()
            .map(|a| Asset {
                sha256: a.digest.as_deref().and_then(parse_digest),
                name: a.name,
                url: a.browser_download_url,
                size: a.size,
            })
            .collect(),
        version,
    })
}

/// `"sha256:<64 hex>"` to lowercase hex; anything else (other algorithms, junk) is None.
pub fn parse_digest(s: &str) -> Option<String> {
    let hex = s.trim().strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hex.to_ascii_lowercase())
}

/// The asset this install type should download, if the release has one.
pub fn select_asset<'a>(release: &'a Release, kind: &InstallKind) -> Option<&'a Asset> {
    let name = match kind {
        InstallKind::Flatpak => FLATPAK_ASSET,
        InstallKind::Binary if cfg!(target_arch = "x86_64") => BINARY_ASSET,
        InstallKind::MacApp(_) => MAC_ASSET,
        _ => return None,
    };
    release.assets.iter().find(|a| a.name == name)
}

/// Release notes as plain text: drops heading/bold/code markers, bullets become "•".
pub fn clean_notes(md: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in md.replace('\r', "").lines() {
        let t = line.trim_end();
        let indent = t.len() - t.trim_start().len();
        let mut t = t.trim_start();
        if t.starts_with('#') {
            t = t.trim_start_matches('#').trim_start();
        }
        let mut s = String::new();
        if let Some(rest) = t.strip_prefix("* ").or_else(|| t.strip_prefix("- ")) {
            s.push_str(&" ".repeat(indent.min(6)));
            s.push_str("• ");
            t = rest;
        }
        s.push_str(&strip_inline(t));
        // At most one blank line in a row.
        if s.is_empty() && out.last().is_none_or(String::is_empty) {
            continue;
        }
        out.push(s);
    }
    out.join("\n").trim().to_string()
}

/// Removes `**`, `__` and backticks, and turns `[text](url)` into `text`.
fn strip_inline(s: &str) -> String {
    let s = s.replace("**", "").replace("__", "").replace('`', "");
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        match after.find("](").and_then(|mid| after[mid + 2..].find(')').map(|end| (mid, mid + 2 + end))) {
            Some((mid, end)) if !after[..mid].contains(['[', ']']) => {
                out.push_str(&rest[..open]);
                out.push_str(&after[..mid]);
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------- downloading

/// Copies `src` to `dst`, failing past `cap` bytes; returns the lowercase hex SHA-256.
fn copy_hashed(
    mut src: impl Read,
    mut dst: impl Write,
    cap: u64,
    mut on_bytes: impl FnMut(u64),
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = src.read(&mut buf).map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > cap {
            return Err(format!("Download is larger than the {} MB limit", cap / (1024 * 1024)));
        }
        hasher.update(&buf[..n]);
        dst.write_all(&buf[..n]).map_err(|e| format!("Could not save the download: {e}"))?;
        on_bytes(total);
    }
    dst.flush().map_err(|e| format!("Could not save the download: {e}"))?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Refuses a download whose hash differs from the one GitHub reported. With no
/// reported hash there is nothing to compare against, so it passes.
fn verify_digest(actual: &str, expected: Option<&str>) -> Result<(), String> {
    match expected {
        Some(e) if !e.eq_ignore_ascii_case(actual) => {
            Err("The download doesn't match the checksum GitHub published, so it was discarded".into())
        }
        _ => Ok(()),
    }
}

/// Downloads `asset` to `dest`, verifying its checksum. `dest` only exists if it passed.
fn download(asset: &Asset, dest: &Path, progress: &Progress, repaint: &dyn Fn()) -> Result<(), String> {
    if asset.size > MAX_DOWNLOAD {
        return Err(format!("Download is larger than the {} MB limit", MAX_DOWNLOAD / (1024 * 1024)));
    }
    let mut resp = agent(Duration::from_secs(15 * 60))
        .get(&asset.url)
        .call()
        .map_err(|e| format!("Could not download {}: {e}", asset.name))?;
    let total = resp.body().content_length().unwrap_or(asset.size);
    progress.total.store(total, Ordering::Relaxed);
    let part = dest.with_extension("part");
    let result = (|| {
        let file = std::fs::File::create(&part).map_err(|e| format!("Could not write {}: {e}", part.display()))?;
        let mut last = std::time::Instant::now();
        let actual =
            copy_hashed(resp.body_mut().with_config().limit(MAX_DOWNLOAD + 1).reader(), file, MAX_DOWNLOAD, |n| {
                progress.done.store(n, Ordering::Relaxed);
                if last.elapsed() > Duration::from_millis(100) {
                    last = std::time::Instant::now();
                    repaint();
                }
            })?;
        verify_digest(&actual, asset.sha256.as_deref())?;
        std::fs::rename(&part, dest).map_err(|e| format!("Could not save {}: {e}", dest.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

// ---------------------------------------------------------------- installing

/// Writes the `file-flier` executable found in a release tarball to `dest`.
fn extract_binary(tar_gz: impl Read, dest: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tar_gz));
    let entries = archive.entries().map_err(|e| format!("Could not read the update archive: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Could not read the update archive: {e}"))?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        // Only the top-level program, not e.g. a same-named file deeper in the tree.
        if !entry.header().entry_type().is_file()
            || path.file_name().is_none_or(|n| n != "file-flier")
            || path.components().count() > 2
        {
            continue;
        }
        let mut src = entry.take(MAX_DOWNLOAD * 2);
        let mut magic = [0u8; 4];
        src.read_exact(&mut magic).map_err(|e| format!("Could not read the update archive: {e}"))?;
        if &magic != b"\x7fELF" {
            return Err("The update archive doesn't contain a Linux program".into());
        }
        let mut out = std::fs::File::create(dest).map_err(|e| format!("Could not write {}: {e}", dest.display()))?;
        out.write_all(&magic).map_err(|e| e.to_string())?;
        std::io::copy(&mut src, &mut out).map_err(|e| format!("Could not extract the update: {e}"))?;
        return out.sync_all().map_err(|e| e.to_string());
    }
    Err("The update archive doesn't contain file-flier".into())
}

fn apply(release: &Release, kind: &InstallKind, progress: &Progress, repaint: &dyn Fn()) -> Result<Outcome, String> {
    let asset = select_asset(release, kind).ok_or("This release has no download for your installation")?;
    match kind {
        InstallKind::Binary => {
            let exe = std::env::current_exe().map_err(|e| format!("Could not locate File Flier: {e}"))?;
            let dir = exe.parent().ok_or("Could not locate File Flier's folder")?;
            let archive = dir.join(".file-flier-update.tar.gz");
            let staged = dir.join(".file-flier-update");
            let result = (|| {
                download(asset, &archive, progress, repaint)?;
                progress.installing.store(true, Ordering::Relaxed);
                repaint();
                let file = std::fs::File::open(&archive).map_err(|e| e.to_string())?;
                extract_binary(file, &staged)?;
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
                // Replacing the directory entry is atomic, and safe while the old one runs.
                std::fs::rename(&staged, &exe).map_err(|e| format!("Could not replace {}: {e}", exe.display()))
            })();
            let _ = std::fs::remove_file(&archive);
            let _ = std::fs::remove_file(&staged);
            result.map(|()| Outcome::Restart(exe))
        }
        InstallKind::Flatpak => {
            let dir = dirs::download_dir().or_else(dirs::home_dir).ok_or("Could not find your Downloads folder")?;
            let bundle = dir.join(FLATPAK_ASSET);
            download(asset, &bundle, progress, repaint)?;
            progress.installing.store(true, Ordering::Relaxed);
            repaint();
            install_bundle(&bundle)
        }
        InstallKind::MacApp(app) => {
            let dir = app.parent().ok_or("Could not locate File Flier's folder")?;
            let archive = dir.join(".File-Flier-update.zip");
            let staging = dir.join(".File-Flier-update");
            let old = dir.join(".File-Flier-old.app");
            let result = (|| {
                download(asset, &archive, progress, repaint)?;
                progress.installing.store(true, Ordering::Relaxed);
                repaint();
                let _ = std::fs::remove_dir_all(&staging);
                // ditto keeps the bundle's code signature, permissions and attributes intact.
                let st = std::process::Command::new("ditto")
                    .args(["-x", "-k"])
                    .arg(&archive)
                    .arg(&staging)
                    .status()
                    .map_err(|e| format!("Could not unpack the update: {e}"))?;
                if !st.success() {
                    return Err("Could not unpack the update".into());
                }
                let new_app = std::fs::read_dir(&staging)
                    .map_err(|e| e.to_string())?
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.extension().is_some_and(|e| e == "app"))
                    .ok_or("The update doesn't contain the app")?;
                // Swap bundles: the running copy keeps working until the restart.
                let _ = std::fs::remove_dir_all(&old);
                std::fs::rename(app, &old).map_err(|e| format!("Could not replace {}: {e}", app.display()))?;
                if let Err(e) = std::fs::rename(&new_app, app) {
                    let _ = std::fs::rename(&old, app); // put the old one back
                    return Err(format!("Could not install the update: {e}"));
                }
                let _ = std::fs::remove_dir_all(&old);
                Ok(())
            })();
            let _ = std::fs::remove_file(&archive);
            let _ = std::fs::remove_dir_all(&staging);
            result.map(|()| Outcome::Restart(app.clone()))
        }
        InstallKind::Unmanaged => Err("This installation is managed by your system".into()),
    }
}

/// Installs a downloaded bundle through the host's `flatpak` when the sandbox is allowed
/// to reach it, otherwise (or if that fails) opens it in the desktop's software center.
fn install_bundle(bundle: &Path) -> Result<Outcome, String> {
    let mut why = String::new();
    if crate::ops::flatpak_can_spawn_on_host() {
        let out = std::process::Command::new("flatpak-spawn")
            .args(["--host", "flatpak", "install", "--user", "--reinstall", "--noninteractive", "--bundle"])
            .arg(bundle)
            .output();
        match out {
            Ok(o) if o.status.success() => {
                let _ = std::fs::remove_file(bundle);
                return Ok(Outcome::Message("Update installed. Restart File Flier to use it.".into()));
            }
            Ok(o) => why = String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("").trim().to_string(),
            Err(e) => why = e.to_string(),
        }
    }
    open::that(bundle).map_err(|e| {
        format!(
            "Downloaded {} but could not open it ({e}). Install it with: flatpak install --user {0}",
            bundle.display()
        )
    })?;
    let mut msg = format!(
        "Downloaded to {}. Click Install in the window that opened, then restart File Flier.",
        bundle.display()
    );
    if !crate::ops::flatpak_can_spawn_on_host() {
        msg.push_str(" To update automatically next time, allow host access:\nflatpak override --user --talk-name=org.freedesktop.Flatpak ");
        msg.push_str(&std::env::var("FLATPAK_ID").unwrap_or_default());
    } else if !why.is_empty() {
        msg.push_str(&format!(" (Automatic install failed: {why})"));
    }
    Ok(Outcome::Message(msg))
}

/// Starts the new executable with the same arguments, then exits. Only returns on failure.
pub fn restart(exe: &Path) -> String {
    // A macOS app bundle is started through Launch Services, as a new instance.
    let mut cmd = if exe.extension().is_some_and(|e| e == "app") {
        let mut c = std::process::Command::new("open");
        c.arg("-n").arg(exe).arg("--args");
        c
    } else {
        std::process::Command::new(exe)
    };
    match cmd.args(std::env::args_os().skip(1)).spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => format!("Could not restart: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(parse_version("v0.5.1"), Some((0, 5, 1)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("2.0.0-rc.1"), Some((2, 0, 0)));
        assert_eq!(parse_version("1.x.0"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version(""), None);
        assert!(is_newer("v0.5.2", "0.5.1"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.5.1", "0.5.1"));
        assert!(!is_newer("0.4.9", "0.5.1"));
        assert!(!is_newer("nightly", "0.5.1"));
    }

    #[test]
    fn install_kinds() {
        let p = |s: &str| classify(Path::new(s), true);
        assert_eq!(p("/home/me/.local/bin/file-flier"), InstallKind::Binary);
        assert_eq!(p("/usr/bin/file-flier"), InstallKind::Unmanaged);
        assert_eq!(p("/opt/file-flier/file-flier"), InstallKind::Unmanaged);
        assert_eq!(p("/home/me/src/FileFlier/target/release/file-flier"), InstallKind::Unmanaged);
        assert_eq!(classify(Path::new("/srv/tools/file-flier"), false), InstallKind::Unmanaged);
        assert_eq!(
            app_bundle(Path::new("/Applications/File Flier.app/Contents/MacOS/file-flier")),
            Some(PathBuf::from("/Applications/File Flier.app"))
        );
        assert_eq!(app_bundle(Path::new("/usr/local/bin/file-flier")), None);
    }

    const SAMPLE: &str = r###"{
        "tag_name": "v0.6.0", "name": "File Flier 0.6.0", "body": "## New\r\n* **Updater** added\r\n",
        "html_url": "https://github.com/andrew11155/File-Flier/releases/tag/v0.6.0", "draft": false,
        "assets": [
          {"name": "file-flier.flatpak", "browser_download_url": "https://example.com/a", "size": 10,
           "digest": "sha256:BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"},
          {"name": "file-flier-x86_64-linux.tar.gz", "browser_download_url": "https://example.com/b", "size": 20,
           "digest": null}
        ]}"###;

    #[test]
    fn parses_release_json() {
        let r = parse_release(SAMPLE).unwrap();
        assert_eq!((r.version.as_str(), r.title.as_str()), ("0.6.0", "File Flier 0.6.0"));
        assert_eq!(r.notes, "New\n• Updater added");
        assert_eq!(r.assets.len(), 2);
        assert_eq!(
            r.assets[0].sha256.as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert_eq!(r.assets[1].sha256, None);
        assert!(parse_release(r#"{"tag_name": "nightly", "html_url": "x"}"#).is_err());
        assert!(parse_release("not json").is_err());
    }

    #[test]
    fn parses_a_real_github_reply() {
        // Trimmed from GET /releases/latest (extra fields such as uploader are ignored).
        let json = r#"{"tag_name":"v0.5.0","name":"v0.5.0","body":"","draft":false,"prerelease":false,
            "html_url":"https://github.com/andrew11155/File-Flier/releases/tag/v0.5.0","assets":[
            {"id":1,"name":"file-flier-x86_64-linux.tar.gz","state":"uploaded","size":11584735,
             "browser_download_url":"https://github.com/andrew11155/File-Flier/releases/download/v0.5.0/file-flier-x86_64-linux.tar.gz",
             "uploader":{"login":"github-actions[bot]"},
             "digest":"sha256:1f677dcbb0f957df2481acf94242386f5ca0d664174fa0d9041c0748bbd05e60"}]}"#;
        let r = parse_release(json).unwrap();
        assert_eq!((r.version.as_str(), r.notes.as_str()), ("0.5.0", ""));
        assert_eq!(r.assets[0].size, 11584735);
        assert_eq!(
            r.assets[0].sha256.as_deref(),
            Some("1f677dcbb0f957df2481acf94242386f5ca0d664174fa0d9041c0748bbd05e60")
        );
    }

    #[test]
    fn picks_the_right_asset() {
        let r = parse_release(SAMPLE).unwrap();
        assert_eq!(select_asset(&r, &InstallKind::Flatpak).unwrap().url, "https://example.com/a");
        if cfg!(target_arch = "x86_64") {
            assert_eq!(select_asset(&r, &InstallKind::Binary).unwrap().url, "https://example.com/b");
        }
        assert!(select_asset(&r, &InstallKind::Unmanaged).is_none());
    }

    #[test]
    fn digests() {
        assert_eq!(parse_digest("sha256:abc"), None);
        assert_eq!(parse_digest("sha512:".to_string().as_str()), None);
        let mut sink = Vec::new();
        let hash = copy_hashed(&b"abc"[..], &mut sink, 100, |_| {}).unwrap();
        assert_eq!(sink, b"abc");
        assert_eq!(hash, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert!(verify_digest(&hash, Some(&hash.to_uppercase())).is_ok());
        assert!(verify_digest(&hash, None).is_ok());
        assert!(verify_digest(&hash, Some(&"0".repeat(64))).is_err());
        assert!(copy_hashed(&[0u8; 200][..], std::io::sink(), 100, |_| {}).is_err());
    }

    #[test]
    fn cleans_release_notes() {
        let md = "# Title\n\n\n\n## Fixes\n- **Bold** and `code`\n  * nested [link](https://x.y/z) ok\nplain [not a link] text\n";
        assert_eq!(clean_notes(md), "Title\n\nFixes\n• Bold and code\n  • nested link ok\nplain [not a link] text");
        assert_eq!(clean_notes(""), "");
    }

    fn tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        for (name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append_data(&mut h, name, *data).unwrap();
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar.into_inner().unwrap()).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn extracts_the_binary() {
        let dir = std::env::temp_dir().join(format!("ff-updater-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("out");
        let elf: &[u8] = b"\x7fELF-rest";
        let ok = tarball(&[("d/README.md", b"hi"), ("d/assets/file-flier", b"deep"), ("d/file-flier", elf)]);
        extract_binary(&ok[..], &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), elf);
        // Not an executable, and missing entirely.
        assert!(extract_binary(&tarball(&[("d/file-flier", b"#!/bin/sh")])[..], &dest).is_err());
        assert!(extract_binary(&tarball(&[("d/other", elf)])[..], &dest).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Hits the real GitHub API: `cargo test -- --ignored live_release`.
    #[test]
    #[ignore]
    fn live_release() {
        let r = fetch_latest().unwrap();
        println!("{} {} {} assets, notes {} chars", r.version, r.page, r.assets.len(), r.notes.len());
        assert!(r.assets.iter().all(|a| a.url.starts_with("https://")));
    }
}
