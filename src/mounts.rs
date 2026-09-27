//! Discovers drives, network shares and cloud mounts for the sidebar, and maps
//! `smb://`-style addresses onto GNOME (GVFS) mount folders.
//!
//! Scanning runs on a background thread: `statvfs` or `readdir` on a network share
//! whose server went away can block for a long time, and must never freeze the UI.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountKind {
    Root,
    Removable,
    Network,
    Cloud,
    /// A drive UDisks knows about that isn't mounted yet (click to mount).
    Unmounted,
}

#[derive(Clone, Debug)]
pub struct Mount {
    pub path: PathBuf,
    pub name: String,
    pub kind: MountKind,
    /// Free and total space (local filesystems only).
    pub space: Option<crate::app::DiskSpace>,
    /// The UDisks volume behind it, for mounting and ejecting.
    pub volume: Option<crate::udisks::Volume>,
}

const NETWORK_FS: &[&str] =
    &["cifs", "smb3", "smbfs", "nfs", "nfs4", "fuse.sshfs", "sshfs", "9p", "davfs", "fuse.davfs2"];
/// FUSE filesystems that are plumbing rather than user storage.
const IGNORED_FUSE: &[&str] = &[
    "fuse.portal",
    "fuse.gvfsd-fuse",
    "fuse.xdg-document-portal",
    "fuse.flatpak-portal",
    "fuse.lxcfs",
    "fuse.snapfuse",
    "fuse.appimagekit",
    "fusectl",
];

fn unescape(s: &str) -> String {
    // /proc/mounts escapes space, tab, newline and backslash as octal.
    s.replace("\\040", " ").replace("\\011", "\t").replace("\\012", "\n").replace("\\134", "\\")
}

/// Mounts that are an app's internals rather than user storage: running AppImages
/// (which mount themselves at /tmp/.mount_*) and anything mounted on a hidden folder.
fn is_internal(device: &str, mountpoint: &str, fstype: &str) -> bool {
    let appimage = |s: &str| s.to_ascii_lowercase().ends_with(".appimage");
    let hidden = Path::new(mountpoint).components().any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
    appimage(device) || appimage(fstype) || hidden
}

fn classify(mountpoint: &str, fstype: &str) -> Option<MountKind> {
    if NETWORK_FS.contains(&fstype) {
        return Some(MountKind::Network);
    }
    if fstype.starts_with("fuse.") && !IGNORED_FUSE.contains(&fstype) && !mountpoint.starts_with("/run/user/") {
        return Some(MountKind::Cloud); // rclone, onedriver, google-drive-ocamlfuse, ...
    }
    if mountpoint.starts_with("/media/") || mountpoint.starts_with("/run/media/") || mountpoint.starts_with("/mnt/") {
        return Some(MountKind::Removable);
    }
    None
}

/// Parses /proc/mounts text into (mountpoint, kind) pairs worth showing.
pub fn parse_proc_mounts(text: &str) -> Vec<(PathBuf, MountKind)> {
    let mut out: Vec<(PathBuf, MountKind)> = Vec::new();
    for line in text.lines() {
        let mut f = line.split_whitespace();
        let (Some(dev), Some(mp), Some(fstype)) = (f.next(), f.next(), f.next()) else { continue };
        let mp = unescape(mp);
        if is_internal(&unescape(dev), &mp, fstype) {
            continue;
        }
        if let Some(kind) = classify(&mp, fstype)
            && !out.iter().any(|(p, _)| p.as_os_str() == mp.as_str())
        {
            out.push((PathBuf::from(mp), kind));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Parses a GVFS mount folder name like `smb-share:server=nas,share=media`.
fn gvfs_fields(name: &str) -> Option<(&str, Vec<(&str, &str)>)> {
    let (scheme, rest) = name.split_once(':')?;
    let fields = rest.split(',').filter_map(|kv| kv.split_once('=')).collect();
    Some((scheme, fields))
}

/// Human-friendly label for a GVFS mount folder.
pub fn gvfs_label(name: &str) -> String {
    let Some((scheme, fields)) = gvfs_fields(name) else { return name.to_string() };
    let get = |k: &str| fields.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
    let host = get("host").or(get("server")).unwrap_or("");
    match scheme {
        "smb-share" => format!("{} on {}", get("share").unwrap_or("share"), host),
        "google-drive" => format!("Google Drive ({})", get("user").unwrap_or(host)),
        "onedrive" => format!("OneDrive ({})", get("user").unwrap_or(host)),
        "sftp" | "ftp" | "ftps" => match get("user") {
            Some(u) => format!("{u}@{host}"),
            None => host.to_string(),
        },
        "dav" | "davs" => format!("WebDAV {host}"),
        "nfs" => format!("NFS {host}"),
        "afp-volume" => format!("{} on {}", get("volume").unwrap_or("volume"), host),
        _ => format!("{scheme} {host}").trim().to_string(),
    }
}

/// The GVFS FUSE directory for this user (`/run/user/<uid>/gvfs`).
pub fn gvfs_root() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        // SAFETY: getuid has no preconditions and cannot fail.
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));
    runtime.join("gvfs")
}

/// Maps `smb://server/share/dir`, `sftp://user@host/dir`, etc. onto an existing GVFS
/// mount folder in `gvfs_dir`. Returns `None` if that share isn't mounted.
pub fn resolve_uri(uri: &str, gvfs_dir: &Path) -> Option<PathBuf> {
    let (scheme, rest) = uri.split_once("://")?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (user, host) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    let host = host.split(':').next().unwrap_or(host).to_lowercase();
    let mut path_parts = path.split('/').filter(|s| !s.is_empty());
    let share = if scheme == "smb" { path_parts.next().map(str::to_lowercase) } else { None };
    let gvfs_scheme = match scheme {
        "smb" => "smb-share",
        "sftp" | "ssh" => "sftp",
        "ftp" | "ftps" | "dav" | "davs" | "nfs" => scheme,
        _ => return None,
    };

    let entries = std::fs::read_dir(gvfs_dir).ok()?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some((s, fields)) = gvfs_fields(&name) else { continue };
        if s != gvfs_scheme {
            continue;
        }
        let get = |k: &str| fields.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_lowercase());
        let host_ok = get("host").or(get("server")).is_some_and(|h| h == host);
        let share_ok = share.is_none() || get("share") == share;
        let user_ok =
            user.is_none() || get("user").is_none_or(|u| Some(u.as_str()) == user.map(str::to_lowercase).as_deref());
        if host_ok && share_ok && user_ok {
            let mut p = e.path();
            for part in path_parts {
                p.push(crate::app::percent_decode(part));
            }
            return Some(p);
        }
    }
    None
}

pub fn is_remote_uri(s: &str) -> bool {
    ["smb://", "sftp://", "ssh://", "ftp://", "ftps://", "dav://", "davs://", "nfs://"].iter().any(|p| s.starts_with(p))
}

fn scan() -> Vec<Mount> {
    let mut mounts = vec![Mount {
        path: PathBuf::from("/"),
        name: "File System".into(),
        kind: MountKind::Root,
        // Measure where the user's files live, not `/`: inside Flatpak `/` is the
        // sandbox's tiny tmpfs, and on Fedora Atomic / Bazzite it's a read-only image.
        volume: None,
        space: dirs::home_dir()
            .and_then(|h| crate::app::disk_space(&h))
            .or_else(|| crate::app::disk_space(Path::new("/"))),
    }];
    let text = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    for (path, kind) in parse_proc_mounts(&text) {
        // Only local disks get a usage bar: statvfs on a dead network share can hang.
        let space = if kind == MountKind::Removable { crate::app::disk_space(&path) } else { None };
        mounts.push(Mount { name: crate::app::display_name(&path), path, kind, space, volume: None });
    }
    // Drives through UDisks: attach them to mounted entries (for eject) and list the
    // ones not mounted yet.
    if let Ok(vols) = crate::udisks::list() {
        for v in &vols {
            if let Some(m) = mounts.iter_mut().find(|m| m.kind != MountKind::Root && v.mount_points.contains(&m.path)) {
                if !v.label.is_empty() && m.kind == MountKind::Removable {
                    m.name = v.label.clone();
                }
                m.volume = Some(v.clone());
            } else if v.mount_points.is_empty() {
                mounts.push(Mount {
                    path: PathBuf::new(),
                    name: v.label.clone(),
                    kind: MountKind::Unmounted,
                    space: None,
                    volume: Some(v.clone()),
                });
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(gvfs_root()) {
        let mut gvfs: Vec<Mount> = rd
            .flatten()
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let kind = if name.starts_with("google-drive") || name.starts_with("onedrive") {
                    MountKind::Cloud
                } else {
                    MountKind::Network
                };
                Mount { name: gvfs_label(&name), path: e.path(), kind, space: None, volume: None }
            })
            .collect();
        gvfs.sort_by(|a, b| a.name.cmp(&b.name));
        mounts.extend(gvfs);
    }
    mounts
}

/// Keeps an up-to-date mount list, refreshed on a background thread.
pub struct MountWatcher {
    latest: Arc<Mutex<Vec<Mount>>>,
}

impl MountWatcher {
    pub fn start(ctx: egui::Context) -> Self {
        let latest = Arc::new(Mutex::new(Vec::new()));
        let shared = latest.clone();
        std::thread::Builder::new()
            .name("mount-watcher".into())
            .spawn(move || {
                let mut last_names: Vec<PathBuf> = Vec::new();
                loop {
                    let mounts = scan();
                    let names: Vec<PathBuf> = mounts.iter().map(|m| m.path.clone()).collect();
                    *shared.lock().unwrap() = mounts;
                    if names != last_names {
                        ctx.request_repaint();
                        last_names = names;
                    }
                    std::thread::sleep(Duration::from_secs(3));
                }
            })
            .expect("spawn mount watcher");
        Self { latest }
    }

    pub fn get(&self) -> Vec<Mount> {
        self.latest.lock().map(|m| m.clone()).unwrap_or_default()
    }
}

/// True if `path` lives on a network or cloud mount (where polling can be slow or hang).
pub fn is_remote_path(path: &Path, mounts: &[Mount]) -> bool {
    mounts.iter().any(|m| matches!(m.kind, MountKind::Network | MountKind::Cloud) && path.starts_with(&m.path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_proc_mounts() {
        let text = "\
/dev/nvme0n1p3 / btrfs rw 0 0
proc /proc proc rw 0 0
//nas/media /mnt/nas cifs rw 0 0
nas:/export /srv/nfs nfs4 rw 0 0
gdrive: /var/home/me/Google\\040Drive fuse.rclone rw 0 0
gvfsd-fuse /run/user/1000/gvfs fuse.gvfsd-fuse rw 0 0
portal /run/user/1000/doc fuse.portal rw 0 0
/dev/sdb1 /run/media/me/USB ext4 rw 0 0
";
        let got = parse_proc_mounts(text);
        let find = |p: &str| got.iter().find(|(m, _)| m == Path::new(p)).map(|(_, k)| *k);
        assert_eq!(find("/mnt/nas"), Some(MountKind::Network));
        assert_eq!(find("/srv/nfs"), Some(MountKind::Network));
        assert_eq!(find("/var/home/me/Google Drive"), Some(MountKind::Cloud));
        assert_eq!(find("/run/media/me/USB"), Some(MountKind::Removable));
        assert_eq!(find("/run/user/1000/gvfs"), None);
        assert_eq!(find("/run/user/1000/doc"), None);
        assert_eq!(find("/"), None);
    }

    #[test]
    fn hides_appimage_and_hidden_mounts() {
        // Running AppImages mount themselves under /tmp/.mount_*; these aren't drives.
        let text = "\
Claude.AppImage /tmp/.mount_ClaudeXq1QkS fuse.Claude.AppImage ro,nosuid,nodev 0 0
/home/me/Apps/Obsidian-1.6.AppImage /tmp/.mount_ObsidiAbc123 fuse.Obsidian-1.6.AppImage ro 0 0
appimagekit /tmp/.mount_Old fuse.appimagekit ro 0 0
some-tool /home/me/.cache/tool-mount fuse.sometool rw 0 0
gdrive: /home/me/GoogleDrive fuse.rclone rw 0 0
";
        let got = parse_proc_mounts(text);
        assert_eq!(got, vec![(PathBuf::from("/home/me/GoogleDrive"), MountKind::Cloud)]);
    }

    #[test]
    fn labels_gvfs_mounts() {
        assert_eq!(gvfs_label("smb-share:server=nas,share=media"), "media on nas");
        assert_eq!(gvfs_label("sftp:host=server.lan,user=bob"), "bob@server.lan");
        assert_eq!(gvfs_label("google-drive:host=gmail.com,user=alice"), "Google Drive (alice)");
        assert_eq!(gvfs_label("weird"), "weird");
    }

    #[test]
    fn resolves_uris_to_gvfs_folders() {
        let dir = std::env::temp_dir().join(format!("file-flier-gvfs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("smb-share:server=nas,share=media/Movies")).unwrap();
        std::fs::create_dir_all(dir.join("sftp:host=box.lan,user=bob")).unwrap();
        assert_eq!(
            resolve_uri("smb://NAS/Media/Movies", &dir),
            Some(dir.join("smb-share:server=nas,share=media").join("Movies"))
        );
        assert_eq!(resolve_uri("sftp://bob@box.lan/", &dir), Some(dir.join("sftp:host=box.lan,user=bob")));
        assert_eq!(resolve_uri("smb://nas/other", &dir), None);
        assert_eq!(resolve_uri("smb://elsewhere/media", &dir), None);
        assert!(is_remote_uri("smb://nas") && !is_remote_uri("/home"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
