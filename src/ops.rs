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

pub fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        let target = fs::read_link(src)?;
        std::os::unix::fs::symlink(target, dst)?;
    } else if meta.is_dir() {
        if dst.starts_with(src) {
            return Err(io::Error::other("cannot copy a folder into itself"));
        }
        fs::create_dir_all(dst)?;
        for item in fs::read_dir(src)? {
            let item = item?;
            copy_recursive(&item.path(), &dst.join(item.file_name()))?;
        }
        fs::set_permissions(dst, meta.permissions())?;
    } else {
        fs::copy(src, dst)?;
    }
    Ok(())
}

pub fn move_path(src: &Path, dst: &Path) -> io::Result<()> {
    if dst.starts_with(src) {
        return Err(io::Error::other("cannot move a folder into itself"));
    }
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        // Cross-device: copy then remove.
        Err(e) if e.raw_os_error() == Some(18) => {
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

/// Launches a terminal emulator in `dir`.
pub fn open_terminal(dir: &Path) -> Result<(), String> {
    let preferred = std::env::var("TERMINAL").ok().filter(|t| !t.trim().is_empty());
    if in_flatpak() {
        // The host's terminals aren't visible in the sandbox; ask the host to pick one.
        let list: Vec<&str> = preferred.iter().map(String::as_str).chain(TERMINALS.iter().copied()).collect();
        let script = "for t in \"$@\"; do command -v \"$t\" >/dev/null 2>&1 && exec \"$t\"; done; exit 127";
        return std::process::Command::new("flatpak-spawn")
            .arg("--host")
            .arg(format!("--directory={}", dir.display()))
            .args(["sh", "-c", script, "sh"])
            .args(list)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Could not open a terminal on the host: {e}"));
    }
    for c in preferred.iter().map(String::as_str).chain(TERMINALS.iter().copied()) {
        if std::process::Command::new(c).current_dir(dir).spawn().is_ok() {
            return Ok(());
        }
    }
    Err("No terminal emulator found (set $TERMINAL)".into())
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
