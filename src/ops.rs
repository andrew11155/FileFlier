//! File operations: copy, move, trash, create, rename.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Finds a free name in `dir` based on `name`, e.g. "a.txt" -> "a (2).txt".
pub fn unique_dest(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() && fs::symlink_metadata(&candidate).is_err() {
        return candidate;
    }
    let p = Path::new(name);
    let (stem, ext) = match (p.file_stem(), p.extension()) {
        (Some(s), Some(e)) if !name.starts_with('.') || name.matches('.').count() > 1 => {
            (s.to_string_lossy().into_owned(), format!(".{}", e.to_string_lossy()))
        }
        _ => (name.to_string(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|c| fs::symlink_metadata(c).is_err())
        .expect("infinite iterator")
}

/// `p` with its parent directory resolved through symlinks (`p` itself need not exist).
fn resolved(p: &Path) -> PathBuf {
    match (p.parent(), p.file_name()) {
        (Some(parent), Some(name)) => fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf()).join(name),
        _ => p.to_path_buf(),
    }
}

/// True if `dst` is `src` or lies inside it, even when reached through symlinks.
fn is_inside(dst: &Path, src: &Path) -> bool {
    let src = fs::canonicalize(src).unwrap_or_else(|_| src.to_path_buf());
    resolved(dst).starts_with(&src) || dst.starts_with(&src)
}

/// Copies a regular file without ever overwriting an existing `dst`.
fn copy_file(src: &Path, dst: &Path, perms: fs::Permissions) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut from = fs::File::open(src)?;
    // create_new is atomic: it fails instead of clobbering a file that appeared meanwhile.
    let mut to = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(dst)?;
    let res = io::copy(&mut from, &mut to).and_then(|_| to.set_permissions(perms));
    if res.is_err() {
        let _ = fs::remove_file(dst); // don't leave a truncated copy behind
    }
    res
}

/// Recursively copies `src` to `dst`, which must not exist yet. Symlinks are copied as
/// links (never followed), and special files (pipes, sockets, devices) are refused
/// because reading them can block forever.
pub fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        let target = fs::read_link(src)?;
        std::os::unix::fs::symlink(target, dst)?;
    } else if ft.is_dir() {
        if is_inside(dst, src) {
            return Err(io::Error::other("cannot copy a folder into itself"));
        }
        fs::create_dir(dst)?; // not create_dir_all: never merge into an existing folder
        for item in fs::read_dir(src)? {
            let item = item?;
            copy_recursive(&item.path(), &dst.join(item.file_name()))?;
        }
        fs::set_permissions(dst, meta.permissions())?;
    } else if ft.is_file() {
        copy_file(src, dst, meta.permissions())?;
    } else {
        return Err(io::Error::other(format!(
            "{} is a special file (pipe, socket or device) and can't be copied",
            src.display()
        )));
    }
    Ok(())
}

/// Renames without replacing an existing `dst` (atomically, via renameat2).
pub fn rename_noreplace(src: &Path, dst: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(io::Error::other);
    let (s, d) = (c(src)?, c(dst)?);
    // SAFETY: both arguments are valid NUL-terminated paths.
    let r = unsafe { libc::renameat2(libc::AT_FDCWD, s.as_ptr(), libc::AT_FDCWD, d.as_ptr(), libc::RENAME_NOREPLACE) };
    if r == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        // Filesystem doesn't support the flag (some FUSE/network filesystems): best effort.
        Some(libc::EINVAL) | Some(libc::ENOSYS) => {
            if fs::symlink_metadata(dst).is_ok() {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            fs::rename(src, dst)
        }
        _ => Err(err),
    }
}

pub fn move_path(src: &Path, dst: &Path) -> io::Result<()> {
    if is_inside(dst, src) {
        return Err(io::Error::other("cannot move a folder into itself"));
    }
    match rename_noreplace(src, dst) {
        Ok(()) => Ok(()),
        // Cross-device: copy everything first, and only delete the source if that fully succeeded.
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            copy_recursive(src, dst)?;
            remove_permanently(src)
        }
        Err(e) => Err(e),
    }
}

pub fn remove_permanently(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) }
}

pub fn trash(paths: &[PathBuf]) -> Result<(), String> {
    trash::delete_all(paths).map_err(|e| e.to_string())
}

pub fn validate_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err("Invalid name".into());
    }
    if name.contains('/') || name.contains('\0') {
        return Err("Names cannot contain '/'".into());
    }
    Ok(())
}

/// Terminal emulators to try, most common first (Ptyxis and Konsole are the
/// defaults on Bazzite / Fedora Atomic GNOME and KDE).
const TERMINALS: &[&str] = &[
    "x-terminal-emulator",
    "ptyxis",
    "kgx",
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "kitty",
    "alacritty",
    "wezterm",
    "foot",
    "tilix",
    "xterm",
];

/// True when running inside a Flatpak sandbox.
pub fn in_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some()
}

/// Whether this Flatpak may run commands on the host (needed to open a terminal).
/// Reads the sandbox's own permission file, which includes user overrides.
pub fn flatpak_can_spawn_on_host() -> bool {
    let info = std::fs::read_to_string("/.flatpak-info").unwrap_or_default();
    let mut in_policy = false;
    for line in info.lines().map(str::trim) {
        if line.starts_with('[') {
            in_policy = line == "[Session Bus Policy]";
        } else if in_policy && line.replace(' ', "") == "org.freedesktop.Flatpak=talk" {
            return true;
        }
    }
    false
}

pub enum TerminalError {
    /// Flatpak without host access; carries the command that grants it.
    NeedsPermission(String),
    Failed(String),
}

/// Launches a terminal emulator in `dir`. In Flatpak, returns the `flatpak-spawn`
/// process so the caller can notice if no terminal was found on the host (exit 127).
pub fn open_terminal(dir: &Path) -> Result<Option<std::process::Child>, TerminalError> {
    let preferred = std::env::var("TERMINAL").ok().filter(|t| !t.trim().is_empty());
    if in_flatpak() {
        if !flatpak_can_spawn_on_host() {
            return Err(TerminalError::NeedsPermission(format!(
                "flatpak override --user --talk-name=org.freedesktop.Flatpak {}",
                std::env::var("FLATPAK_ID").unwrap_or_default()
            )));
        }
        // The host's terminals aren't visible in the sandbox; ask the host to pick one.
        let list: Vec<&str> = preferred.iter().map(String::as_str).chain(TERMINALS.iter().copied()).collect();
        let script = "for t in \"$@\"; do command -v \"$t\" >/dev/null 2>&1 && exec \"$t\"; done; exit 127";
        return std::process::Command::new("flatpak-spawn")
            .arg("--host")
            .arg(format!("--directory={}", dir.display()))
            .args(["sh", "-c", script, "sh"])
            .args(list)
            .spawn()
            .map(Some)
            .map_err(|e| TerminalError::Failed(format!("Could not open a terminal on the host: {e}")));
    }
    for c in preferred.iter().map(String::as_str).chain(TERMINALS.iter().copied()) {
        if std::process::Command::new(c).current_dir(dir).spawn().is_ok() {
            return Ok(None);
        }
    }
    Err(TerminalError::Failed("No terminal emulator found (set $TERMINAL)".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("file-flier-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn unique_names() {
        let d = tmpdir("unique");
        fs::write(d.join("a.txt"), "").unwrap();
        assert_eq!(unique_dest(&d, "a.txt"), d.join("a (2).txt"));
        assert_eq!(unique_dest(&d, "b.txt"), d.join("b.txt"));
        fs::write(d.join(".bashrc"), "").unwrap();
        assert_eq!(unique_dest(&d, ".bashrc"), d.join(".bashrc (2)"));
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn never_overwrites() {
        let d = tmpdir("noclobber");
        fs::write(d.join("a"), "new").unwrap();
        fs::write(d.join("b"), "precious").unwrap();
        assert!(copy_recursive(&d.join("a"), &d.join("b")).is_err());
        assert!(rename_noreplace(&d.join("a"), &d.join("b")).is_err());
        assert!(move_path(&d.join("a"), &d.join("b")).is_err());
        assert_eq!(fs::read_to_string(d.join("b")).unwrap(), "precious");
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn refuses_special_files_instead_of_hanging() {
        let d = tmpdir("fifo");
        let fifo = std::ffi::CString::new(d.join("pipe").to_str().unwrap()).unwrap();
        // SAFETY: valid path; mkfifo has no other preconditions.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let err = copy_recursive(&d.join("pipe"), &d.join("copy")).unwrap_err();
        assert!(err.to_string().contains("special file"));
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn detects_copy_into_self_through_symlink() {
        let d = tmpdir("selfcopy");
        fs::create_dir_all(d.join("real/sub")).unwrap();
        std::os::unix::fs::symlink(d.join("real"), d.join("link")).unwrap();
        // Source reached via the symlink, destination via the real path.
        assert!(copy_recursive(&d.join("link/sub"), &d.join("real/sub/inner")).is_err());
        assert!(copy_recursive(&d.join("real"), &d.join("link/sub/x")).is_err());
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn copy_and_move_tree() {
        let d = tmpdir("copy");
        fs::create_dir_all(d.join("src/inner")).unwrap();
        fs::write(d.join("src/inner/f.txt"), "hi").unwrap();
        copy_recursive(&d.join("src"), &d.join("dst")).unwrap();
        assert_eq!(fs::read_to_string(d.join("dst/inner/f.txt")).unwrap(), "hi");
        assert!(copy_recursive(&d.join("src"), &d.join("src/inner/x")).is_err());
        move_path(&d.join("dst"), &d.join("moved")).unwrap();
        assert!(!d.join("dst").exists());
        assert!(d.join("moved/inner/f.txt").exists());
        fs::remove_dir_all(d).unwrap();
    }
}
