//! Cloud storage accounts (Google Drive, OneDrive, Dropbox) through rclone.
//!
//! Signing in runs `rclone config create`, which opens the browser for the
//! provider's own login page; rclone keeps the token. Each account is then
//! mounted with `rclone mount` (FUSE) into a folder of ours, so every feature of
//! File Flier (previews, copy, drag-out to other apps) works on it like a local
//! folder, and `statvfs` on it reports the account's quota for the usage bar.
//!
//! Accounts live in File Flier's own rclone config, separate from any rclone
//! setup the user already has. If rclone isn't installed, the official build is
//! downloaded from rclone.org and checked against its published SHA-256.
//! In Flatpak, rclone runs on the host (FUSE mounts can't be made from inside
//! the sandbox), which needs the same opt-in permission as "Open terminal here".

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::ops::{flatpak_can_spawn_on_host, in_flatpak};

/// Cloud accounts need FUSE and rclone builds, which File Flier sets up on Linux.
pub const SUPPORTED: bool = cfg!(target_os = "linux");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    GoogleDrive,
    OneDrive,
    Dropbox,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::GoogleDrive, Provider::OneDrive, Provider::Dropbox];

    pub fn name(self) -> &'static str {
        match self {
            Provider::GoogleDrive => "Google Drive",
            Provider::OneDrive => "OneDrive",
            Provider::Dropbox => "Dropbox",
        }
    }

    /// rclone backend name.
    fn backend(self) -> &'static str {
        match self {
            Provider::GoogleDrive => "drive",
            Provider::OneDrive => "onedrive",
            Provider::Dropbox => "dropbox",
        }
    }

    fn from_backend(s: &str) -> Option<Self> {
        Provider::ALL.into_iter().find(|p| p.backend() == s)
    }

    /// Extra `key=value` settings for `rclone config create`.
    fn options(self) -> &'static [&'static str] {
        match self {
            // Full access, like the Drive desktop app (not just files rclone created).
            Provider::GoogleDrive => &["scope=drive"],
            Provider::OneDrive | Provider::Dropbox => &[],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    /// rclone remote name, also shown in the sidebar ("Google Drive", "OneDrive 2").
    pub name: String,
    pub provider: Provider,
    pub mount: PathBuf,
}

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("file-flier")
}

/// File Flier's own rclone config (tokens are stored here, readable only by you).
pub fn config_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("file-flier").join("rclone.conf")
}

/// Where accounts are mounted. Hidden, so an unmounted (empty) folder is never
/// mistaken for the cloud.
pub fn mount_root() -> PathBuf {
    data_dir().join("cloud")
}

fn downloaded_rclone() -> PathBuf {
    data_dir().join("bin").join("rclone")
}

/// Reads accounts from our rclone config (INI: `[name]`, `type = drive`, `token = ...`).
/// Accounts without a token are skipped: rclone writes the entry before sign-in ends.
pub fn parse_accounts(conf: &str, root: &Path) -> Vec<Account> {
    let mut out = Vec::new();
    // (name, provider, signed in)
    let mut cur: Option<(String, Option<Provider>, bool)> = None;
    let mut finish = |c: Option<(String, Option<Provider>, bool)>| {
        if let Some((name, Some(provider), true)) = c {
            out.push(Account { mount: root.join(&name), name, provider });
        }
    };
    for line in conf.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            finish(cur.take());
            cur = Some((name.to_string(), None, false));
        } else if let (Some(c), Some((k, v))) = (&mut cur, line.split_once('=')) {
            match k.trim() {
                "type" => c.1 = Provider::from_backend(v.trim()),
                "token" => c.2 = v.contains("access_token"),
                _ => {}
            }
        }
    }
    finish(cur);
    out
}

pub fn accounts() -> Vec<Account> {
    parse_accounts(&std::fs::read_to_string(config_path()).unwrap_or_default(), &mount_root())
}

/// Whether the account's token was saved (sign-in finished).
fn has_token(name: &str) -> bool {
    accounts().iter().any(|a| a.name == name)
}

/// A new account name: "Google Drive", then "Google Drive 2", ...
pub fn next_name(provider: Provider, existing: &[Account]) -> String {
    let base = provider.name();
    (1..)
        .map(|i| if i == 1 { base.to_string() } else { format!("{base} {i}") })
        .find(|n| !existing.iter().any(|a| &a.name == n))
        .unwrap()
}

pub fn is_mounted(path: &Path) -> bool {
    let text = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let want = path.to_string_lossy().replace(' ', "\\040");
    text.lines().any(|l| l.split_whitespace().nth(1) == Some(want.as_str()))
}

// ------------------------------------------------------------------ running rclone

/// How rclone is run on this system.
#[derive(Clone, Debug)]
pub struct Rclone {
    program: PathBuf,
    /// Through `flatpak-spawn --host`.
    host: bool,
}

#[derive(Clone, Debug)]
pub enum Setup {
    Ready(Rclone),
    /// rclone isn't installed; offer to download it.
    NeedsDownload,
    /// Flatpak without host access; carries the command that grants it.
    NeedsHostAccess(String),
    /// No FUSE on this system (`fusermount3`), so drives can't be mounted.
    NoFuse,
}

fn host_command(program: &str) -> Command {
    let mut c = Command::new("flatpak-spawn");
    c.args(["--host", program]);
    c
}

/// `command -v <name>` on the host (or here, outside Flatpak).
fn find_program(name: &str) -> Option<PathBuf> {
    if in_flatpak() {
        let out = host_command("sh").args(["-c", &format!("command -v {name}")]).output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return (out.status.success() && !s.is_empty()).then(|| PathBuf::from(s));
    }
    std::env::var_os("PATH")?.to_str()?.split(':').map(|d| Path::new(d).join(name)).find(|p| p.is_file())
}

/// Checks what's needed to use cloud accounts. Runs commands: call off the UI thread.
pub fn setup() -> Setup {
    let host = in_flatpak();
    if host && !flatpak_can_spawn_on_host() {
        return Setup::NeedsHostAccess(format!(
            "flatpak override --user --talk-name=org.freedesktop.Flatpak {}",
            std::env::var("FLATPAK_ID").unwrap_or_else(|_| "io.github.andrew11155.FileFlier".into())
        ));
    }
    if find_program("fusermount3").is_none() && find_program("fusermount").is_none() {
        return Setup::NoFuse;
    }
    if let Some(program) = find_program("rclone") {
        return Setup::Ready(Rclone { program, host });
    }
    let program = downloaded_rclone();
    if program.is_file() {
        return Setup::Ready(Rclone { program, host });
    }
    Setup::NeedsDownload
}

impl Rclone {
    fn command(&self) -> Command {
        let mut c = if self.host { host_command(&self.program.to_string_lossy()) } else { Command::new(&self.program) };
        c.arg("--config").arg(config_path());
        c
    }

    fn run(&self, args: &[&str]) -> Result<String, String> {
        let out = self.command().args(args).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(last_error(&String::from_utf8_lossy(&out.stderr)))
        }
    }

    /// Signs in to a new account: rclone opens the browser and waits for the login.
    /// `url` receives the local sign-in address, so the UI can offer to open it again.
    pub fn sign_in(
        &self,
        provider: Provider,
        client: Option<&(String, String)>,
        name: &str,
        url: &Arc<Mutex<Option<String>>>,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        if let Some(parent) = config_path().parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut args: Vec<String> =
            ["config", "create", name, provider.backend()].iter().map(|s| s.to_string()).collect();
        args.extend(provider.options().iter().map(|s| s.to_string()));
        if let Some((id, secret)) = client {
            // The user's own OAuth app (see rclone.org/drive/#making-your-own-client-id).
            args.push(format!("client_id={}", id.trim()));
            args.push(format!("client_secret={}", secret.trim()));
        }
        let mut child = self
            .command()
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Couldn't start rclone: {e}"))?;
        // rclone prints the sign-in address and its errors on stderr.
        let log = Arc::new(Mutex::new(String::new()));
        let readers: Vec<_> = [
            child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
            child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        .map(|r| {
            let (log, url) = (log.clone(), url.clone());
            std::thread::spawn(move || {
                for line in BufReader::new(r).lines().map_while(Result::ok) {
                    if let Some(i) = line.find("http://127.0.0.1:") {
                        *url.lock().unwrap() = Some(line[i..].split_whitespace().next().unwrap_or("").to_string());
                    }
                    let mut l = log.lock().unwrap();
                    l.push_str(&line);
                    l.push('\n');
                }
            })
        })
        .collect();
        let started = Instant::now();
        let status = loop {
            if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
                break Some(st);
            }
            if cancel.load(Ordering::SeqCst) || started.elapsed() > Duration::from_secs(15 * 60) {
                // SIGTERM (not SIGKILL) so flatpak-spawn forwards it to rclone on the host.
                // SAFETY: plain kill(2) on our own child's pid.
                unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(150));
        };
        for r in readers {
            let _ = r.join();
        }
        let ok = status.is_some_and(|s| s.success()) && has_token(name);
        if !ok {
            // Don't leave a half-configured account behind.
            let _ = self.run(&["config", "delete", name]);
            return Err(match status {
                None => "Sign-in cancelled".into(),
                Some(_) => {
                    let l = log.lock().unwrap();
                    let e = last_error(&l);
                    if e.is_empty() { "Sign-in didn't finish".into() } else { e }
                }
            });
        }
        Ok(())
    }

    /// Mounts an account (returns once the folder is ready).
    pub fn mount(&self, a: &Account) -> Result<(), String> {
        if is_mounted(&a.mount) {
            return Ok(());
        }
        std::fs::create_dir_all(&a.mount).map_err(|e| e.to_string())?;
        let remote = format!("{}:", a.name);
        let mp = a.mount.to_string_lossy().into_owned();
        self.run(&[
            "mount",
            &remote,
            &mp,
            "--daemon",
            "--volname",
            &a.name,
            // Files are cached locally while open, so apps can edit them normally.
            "--vfs-cache-mode",
            "full",
            "--vfs-cache-max-size",
            "5G",
            "--dir-cache-time",
            "1m",
        ])?;
        // In Flatpak the mount is made on the host; it reaches the sandbox through mount
        // propagation, which systemd systems have. Say so if it doesn't show up.
        let start = Instant::now();
        while !is_mounted(&a.mount) {
            if start.elapsed() > Duration::from_secs(6) {
                return Err(format!(
                    "{} is connected, but File Flier's Flatpak sandbox can't see it on this system",
                    a.name
                ));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(())
    }

    /// Removes an account: unmounts it and deletes its saved token.
    pub fn remove(&self, a: &Account) -> Result<(), String> {
        unmount(&a.mount)?;
        self.run(&["config", "delete", &a.name])?;
        let _ = std::fs::remove_dir(&a.mount); // only if empty
        Ok(())
    }
}

/// Unmounts a cloud folder (safe to call when it isn't mounted).
pub fn unmount(path: &Path) -> Result<(), String> {
    if !is_mounted(path) {
        return Ok(());
    }
    let p = path.to_string_lossy().into_owned();
    let mut last = String::new();
    for prog in ["fusermount3", "fusermount"] {
        let mut c = if in_flatpak() { host_command(prog) } else { Command::new(prog) };
        match c.args(["-u", &p]).output() {
            Ok(o) if o.status.success() => return Ok(()),
            Ok(o) => last = String::from_utf8_lossy(&o.stderr).into_owned(),
            Err(_) => continue,
        }
    }
    Err(if last.contains("busy") {
        "It's in use. Close any files from it, then try again.".into()
    } else {
        last_error(&last)
    })
}

/// The most useful line of rclone's error output, in plain words where we can.
fn last_error(log: &str) -> String {
    let line = log
        .lines()
        .rev()
        .find(|l| ["CRITICAL", "ERROR", "Failed", "error"].iter().any(|k| l.contains(k)))
        .or_else(|| log.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .trim();
    friendly(strip_log_prefix(line))
}

/// Drops rclone's "2026/09/29 10:00:00 ERROR : " prefix.
fn strip_log_prefix(line: &str) -> &str {
    let mut l = line;
    let b = l.as_bytes();
    if b.len() > 20 && b[4] == b'/' && b[7] == b'/' && b[13] == b':' {
        l = l[20..].trim_start();
    }
    for level in ["CRITICAL", "ERROR", "NOTICE", "INFO", "DEBUG"] {
        if let Some(rest) = l.strip_prefix(level) {
            return rest.trim_start_matches([' ', ':']).trim();
        }
    }
    l.trim()
}

/// Turns the errors people actually hit into advice.
fn friendly(msg: &str) -> String {
    let m = msg.to_lowercase();
    let any = |ks: &[&str]| ks.iter().any(|k| m.contains(k));
    if any(&["no such host", "network is unreachable", "i/o timeout", "connection refused", "tls handshake"]) {
        "Couldn't reach the service. Check your internet connection.".into()
    } else if any(&["invalid_client", "deleted_client", "disabled_client", "unauthorized_client"]) {
        "Sign-in was refused. Sign out (right-click the drive) and add it again; for Google, use your own client ID."
            .into()
    } else if any(&["invalid_grant", "token expired", "maybe token expired", "401", "unauthenticated"]) {
        "Sign-in expired. Right-click the drive, choose Sign out, then add it again.".into()
    } else {
        msg.to_string()
    }
}

// ------------------------------------------------------------------ download

fn arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("amd64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

fn http_get(url: &str, max: u64, progress: &dyn Fn(u64)) -> Result<Vec<u8>, String> {
    let resp =
        crate::net::agent(Duration::from_secs(300)).get(url).call().map_err(|e| format!("Download failed: {e}"))?;
    let mut reader = resp.into_body().into_reader();
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("Download failed: {e}"))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() as u64 > max {
            return Err("Download is larger than expected".into());
        }
        progress(out.len() as u64);
    }
    Ok(out)
}

/// The expected hash for `file` from rclone's (PGP-signed) SHA256SUMS listing.
pub fn sum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, name) = l.split_once(char::is_whitespace)?;
        (name.trim() == file && hash.len() == 64).then(|| hash.to_lowercase())
    })
}

/// Downloads the official rclone build into File Flier's data folder.
pub fn download(progress: &Mutex<String>) -> Result<(), String> {
    use sha2::Digest;
    let arch = arch().ok_or("rclone downloads are only set up for x86-64 and ARM64; install rclone instead")?;
    let set = |s: String| *progress.lock().unwrap() = s;
    set("Finding the latest rclone…".into());
    let version = String::from_utf8_lossy(&http_get("https://downloads.rclone.org/version.txt", 1 << 10, &|_| {})?)
        .trim()
        .trim_start_matches("rclone ")
        .to_string();
    if !version.starts_with('v') || version.contains('/') || version.len() > 20 {
        return Err("Couldn't read rclone's version".into());
    }
    let file = format!("rclone-{version}-linux-{arch}.zip");
    let base = format!("https://downloads.rclone.org/{version}");
    let sums = String::from_utf8_lossy(&http_get(&format!("{base}/SHA256SUMS"), 1 << 20, &|_| {})?).into_owned();
    let want = sum_for(&sums, &file).ok_or("rclone's checksum list doesn't include this download")?;
    let zip = http_get(&format!("{base}/{file}"), 120 << 20, &|n| {
        set(format!("Downloading rclone… {}", crate::app::human_size(n)))
    })?;
    let got: String = sha2::Sha256::digest(&zip).iter().map(|b| format!("{b:02x}")).collect();
    if got != want {
        return Err("The rclone download didn't match its checksum, so it wasn't installed".into());
    }
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
    let inner = format!("rclone-{version}-linux-{arch}/rclone");
    let mut bin = Vec::new();
    z.by_name(&inner)
        .map_err(|_| "The rclone download is missing its program")?
        .read_to_end(&mut bin)
        .map_err(|e| e.to_string())?;
    let dest = downloaded_rclone();
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = dest.with_extension("part");
    std::fs::write(&tmp, &bin).map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(())
}

// ------------------------------------------------------------------ app side

/// Sign-in in progress, shared between the dialog and its worker thread.
pub struct SignIn {
    pub provider: Provider,
    pub url: Arc<Mutex<Option<String>>>,
    pub cancel: Arc<AtomicBool>,
    pub result: Arc<Mutex<Option<Result<Account, String>>>>,
}

impl SignIn {
    pub fn start(rclone: Rclone, provider: Provider, client: Option<(String, String)>, ctx: egui::Context) -> Self {
        let url = Arc::new(Mutex::new(None));
        let cancel = Arc::new(AtomicBool::new(false));
        let result = Arc::new(Mutex::new(None));
        let (u, c, r) = (url.clone(), cancel.clone(), result.clone());
        std::thread::spawn(move || {
            let name = next_name(provider, &accounts());
            let res = rclone.sign_in(provider, client.as_ref(), &name, &u, &c).and_then(|()| {
                let a = accounts().into_iter().find(|a| a.name == name).ok_or("The account wasn't saved")?;
                rclone.mount(&a).map(|()| a)
            });
            *r.lock().unwrap() = Some(res);
            ctx.request_repaint();
        });
        SignIn { provider, url, cancel, result }
    }
}

/// Mounts every account that isn't mounted yet (at startup). Quietly skips
/// accounts that fail; they show as disconnected and can be retried from the sidebar.
pub fn mount_all_in_background() {
    std::thread::spawn(|| {
        let accts = accounts();
        if accts.is_empty() || accts.iter().all(|a| is_mounted(&a.mount)) {
            return;
        }
        if let Setup::Ready(r) = setup() {
            for a in accts {
                let _ = r.mount(&a);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_accounts_and_names_new_ones() {
        let tok = "token = {\"access_token\":\"x\"}";
        let conf = format!(
            "[Google Drive]\ntype = drive\nscope = drive\n{tok}\n\n[mine]\ntype = s3\n{tok}\n\n[OneDrive]\ntype = onedrive\n{tok}\n\n[Dropbox]\ntype = dropbox\n"
        );
        let conf = conf.as_str();
        let a = parse_accounts(conf, Path::new("/m"));
        assert_eq!(a.len(), 2);
        assert_eq!(
            a[0],
            Account { name: "Google Drive".into(), provider: Provider::GoogleDrive, mount: "/m/Google Drive".into() }
        );
        assert_eq!(a[1].provider, Provider::OneDrive);
        assert_eq!(next_name(Provider::GoogleDrive, &a), "Google Drive 2");
        assert_eq!(next_name(Provider::Dropbox, &a), "Dropbox");
    }

    #[test]
    fn checksums_and_errors() {
        let sums = "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA1\n\nb33db81f432d32702d386d15e774610e45acb3c0df10af4f45efc113e21c0534  rclone-v1.75.1-aix-ppc64.zip\n982b5aa772841168f8e380f139e9e787b2a105403e32b94da8676a0e1c0a13ab  rclone-v1.75.1-linux-amd64.zip\n";
        assert_eq!(
            sum_for(sums, "rclone-v1.75.1-linux-amd64.zip").as_deref(),
            Some("982b5aa772841168f8e380f139e9e787b2a105403e32b94da8676a0e1c0a13ab")
        );
        assert_eq!(sum_for(sums, "rclone-v1.75.1-linux-arm64.zip"), None);
        assert_eq!(
            last_error("2026/09/29 10:00:00 NOTICE: hi\n2026/09/29 10:00:01 ERROR : gd: failed to list\n"),
            "gd: failed to list"
        );
        let expired = "2026/09/29 23:40:36 NOTICE: Google Drive: shared client_id\n2026/09/29 23:40:36 CRITICAL: Failed to create file system for \"Google Drive:\": couldn't fetch token: invalid_grant: maybe token expired?";
        assert!(last_error(expired).starts_with("Sign-in expired"));
        assert!(last_error("ERROR : dial tcp: lookup www.googleapis.com: no such host").starts_with("Couldn't reach"));
    }
}
