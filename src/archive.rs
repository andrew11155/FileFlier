//! Compress to .zip and "Extract Here".
//!
//! Extraction is careful with hostile archives: entries can't escape the target
//! folder (no absolute paths, no `..`), symlinks are created last so nothing is
//! written through them, setuid bits are dropped, and nothing is ever overwritten.
//! Everything is unpacked into a hidden temporary folder first, which is removed
//! if anything goes wrong.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ops;

/// Archive formats we can unpack, by file name.
pub fn can_extract(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let native = [".zip", ".jar", ".cbz", ".tar", ".tar.gz", ".tgz", ".gz"].iter().any(|e| name.ends_with(e));
    let via_bsdtar = [
        ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tbz", ".tar.zst", ".tzst", ".7z", ".rar", ".cbr", ".xz", ".iso",
        ".cpio", ".ar", ".lzh", ".cab",
    ]
    .iter()
    .any(|e| name.ends_with(e));
    native || (via_bsdtar && crate::preview::have("bsdtar"))
}

/// Archive name without its archive extension(s): "photos.tar.gz" -> "photos".
pub fn stem(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into());
    let lower = name.to_lowercase();
    for ext in [".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst"] {
        if lower.ends_with(ext) {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    match name.rsplit_once('.') {
        Some((s, _)) if !s.is_empty() => s.to_string(),
        _ => name,
    }
}

/// Unpacks `archive` next to it. A single top-level item is placed directly in
/// `dest`; several go into a new folder named after the archive. Returns what was created.
pub fn extract(archive: &Path, dest: &Path, progress: &Mutex<String>) -> Result<PathBuf, String> {
    let tmp = dest.join(format!(".{}.extracting-{}", stem(archive), std::process::id()));
    std::fs::create_dir(&tmp).map_err(|e| format!("Couldn't create a folder here: {e}"))?;
    let result = unpack(archive, &tmp, progress).and_then(|()| place(archive, &tmp, dest));
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    result
}

fn place(archive: &Path, tmp: &Path, dest: &Path) -> Result<PathBuf, String> {
    let entries: Vec<PathBuf> =
        std::fs::read_dir(tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
    if entries.is_empty() {
        return Err("The archive is empty".into());
    }
    if let [only] = entries.as_slice() {
        let name = only.file_name().unwrap().to_string_lossy().into_owned();
        let target = ops::unique_dest(dest, &name);
        ops::rename_noreplace(only, &target).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir(tmp);
        Ok(target)
    } else {
        let target = ops::unique_dest(dest, &stem(archive));
        ops::rename_noreplace(tmp, &target).map_err(|e| e.to_string())?;
        Ok(target)
    }
}

fn unpack(archive: &Path, out: &Path, progress: &Mutex<String>) -> Result<(), String> {
    let lower = archive.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let open = || std::fs::File::open(archive).map_err(|e| e.to_string());
    if [".zip", ".jar", ".cbz"].iter().any(|e| lower.ends_with(e)) {
        unzip(open()?, out, progress)
    } else if lower.ends_with(".tar") {
        untar(open()?, out)
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        untar(flate2::read::GzDecoder::new(open()?), out)
    } else if lower.ends_with(".gz") {
        // A single gzipped file.
        let name = stem(archive);
        let mut f = new_file(&out.join(&name), 0o644)?;
        std::io::copy(&mut flate2::read::GzDecoder::new(open()?), &mut f).map_err(|e| e.to_string())?;
        Ok(())
    } else {
        bsdtar(archive, out)
    }
}

fn new_file(path: &Path, mode: u32) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode & 0o777)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn unzip(file: std::fs::File, out: &Path, progress: &Mutex<String>) -> Result<(), String> {
    let mut z = zip::ZipArchive::new(file).map_err(|_| "This archive is damaged or unsupported".to_string())?;
    let mut links = Vec::new();
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        if e.encrypted() {
            return Err("This archive is password-protected".into());
        }
        let Some(rel) = e.enclosed_name() else { continue }; // absolute or `..` paths
        let target = out.join(&rel);
        *progress.lock().unwrap() = rel.to_string_lossy().into_owned();
        if e.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        if e.is_symlink() {
            let mut t = String::new();
            e.read_to_string(&mut t).map_err(|e| e.to_string())?;
            links.push((target, t));
            continue;
        }
        let mode = e.unix_mode().unwrap_or(0o644);
        let mut f = new_file(&target, mode)?;
        std::io::copy(&mut e, &mut f).map_err(|e| e.to_string())?;
    }
    // Links last, and only ones that stay inside the archive's folder.
    for (target, t) in links {
        let resolved = normalize(&target.parent().unwrap().join(&t));
        if Path::new(&t).is_relative() && resolved.starts_with(out) {
            let _ = std::os::unix::fs::symlink(&t, &target);
        }
    }
    Ok(())
}

/// Lexically resolves `.` and `..` (without touching the filesystem).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

fn untar(reader: impl Read, out: &Path) -> Result<(), String> {
    let mut ar = tar::Archive::new(reader);
    ar.set_preserve_permissions(false);
    ar.set_unpack_xattrs(false);
    ar.set_overwrite(false);
    // `unpack` refuses paths that escape `out`, including through symlinks.
    ar.unpack(out).map_err(|e| format!("Couldn't extract: {e}"))
}

fn bsdtar(archive: &Path, out: &Path) -> Result<(), String> {
    if !crate::preview::have("bsdtar") {
        return Err("Extracting this format needs bsdtar (libarchive-tools)".into());
    }
    // bsdtar refuses absolute paths, `..` and writing through symlinks by default.
    let status = std::process::Command::new("bsdtar")
        .args(["-x", "--no-same-owner", "--no-same-permissions", "-f"])
        .arg(archive)
        .arg("-C")
        .arg(out)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if status.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&status.stderr);
        Err(format!("Couldn't extract: {}", err.lines().last().unwrap_or("unknown error").trim()))
    }
}

/// Zips `paths` (files and folders, recursively) into `out`. Symlinks are stored
/// as links; pipes and devices are skipped.
pub fn compress(paths: &[PathBuf], out: &Path, progress: &Mutex<String>) -> Result<(), String> {
    use zip::write::SimpleFileOptions;
    let tmp = out.with_file_name(format!(".{}.part", out.file_name().unwrap().to_string_lossy()));
    let file = new_file(&tmp, 0o644)?;
    let result = (|| {
        let mut z = zip::ZipWriter::new(std::io::BufWriter::new(file));
        let base = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut stack: Vec<(PathBuf, String)> =
            paths.iter().filter_map(|p| Some((p.clone(), p.file_name()?.to_string_lossy().into_owned()))).collect();
        stack.reverse();
        while let Some((path, name)) = stack.pop() {
            let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            *progress.lock().unwrap() = name.clone();
            use std::os::unix::fs::PermissionsExt;
            let mode = meta.permissions().mode() & 0o777;
            let mut opts = base.unix_permissions(mode);
            if let Some(t) = meta.modified().ok().and_then(zip_time) {
                opts = opts.last_modified_time(t);
            }
            if meta.file_type().is_symlink() {
                let target = std::fs::read_link(&path).map_err(|e| e.to_string())?;
                z.add_symlink(&name, target.to_string_lossy(), opts).map_err(|e| e.to_string())?;
            } else if meta.is_dir() {
                z.add_directory(format!("{name}/"), opts).map_err(|e| e.to_string())?;
                let mut children: Vec<_> =
                    std::fs::read_dir(&path).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
                children.sort();
                for c in children.into_iter().rev() {
                    let child = format!("{name}/{}", c.file_name().unwrap().to_string_lossy());
                    stack.push((c, child));
                }
            } else if meta.is_file() {
                z.start_file(&name, opts.large_file(meta.len() >= 0xFFFF_FFFF)).map_err(|e| e.to_string())?;
                let mut f = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                std::io::copy(&mut f, &mut z).map_err(|e| e.to_string())?;
            }
        }
        let mut w = z.finish().map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })();
    match result {
        Ok(()) => ops::rename_noreplace(&tmp, out).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// A file's modification time as a zip timestamp (local time, 2-second precision).
fn zip_time(t: std::time::SystemTime) -> Option<zip::DateTime> {
    use chrono::{Datelike, Timelike};
    let d: chrono::DateTime<chrono::Local> = t.into();
    zip::DateTime::from_date_and_time(
        u16::try_from(d.year()).ok()?,
        d.month() as u8,
        d.day() as u8,
        d.hour() as u8,
        d.minute() as u8,
        d.second() as u8,
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("file-flier-arch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn stems() {
        assert_eq!(stem(Path::new("/a/photos.tar.gz")), "photos");
        assert_eq!(stem(Path::new("/a/Report.zip")), "Report");
        assert_eq!(stem(Path::new("/a/.hidden")), ".hidden");
    }

    #[test]
    fn roundtrip_and_placement() {
        let d = tmpdir("rt");
        let p = Mutex::new(String::new());
        std::fs::create_dir_all(d.join("src/folder/sub")).unwrap();
        std::fs::write(d.join("src/folder/a.txt"), "hello").unwrap();
        std::fs::write(d.join("src/folder/sub/b.txt"), "world").unwrap();
        std::os::unix::fs::symlink("a.txt", d.join("src/folder/link")).unwrap();
        std::fs::write(d.join("src/single.txt"), "one").unwrap();

        // One folder in the archive -> extracted as that folder.
        compress(&[d.join("src/folder")], &d.join("f.zip"), &p).unwrap();
        let out = extract(&d.join("f.zip"), &d, &p).unwrap();
        assert_eq!(out, d.join("folder"));
        assert_eq!(std::fs::read_to_string(d.join("folder/sub/b.txt")).unwrap(), "world");
        assert_eq!(std::fs::read_link(d.join("folder/link")).unwrap(), Path::new("a.txt"));

        // Two items -> a new folder named after the archive; never overwrites.
        compress(&[d.join("src/folder"), d.join("src/single.txt")], &d.join("two.zip"), &p).unwrap();
        assert_eq!(extract(&d.join("two.zip"), &d, &p).unwrap(), d.join("two"));
        assert!(d.join("two/single.txt").is_file());
        assert_eq!(extract(&d.join("f.zip"), &d, &p).unwrap(), d.join("folder (2)"));
        // No temporary folders left behind.
        assert!(
            !std::fs::read_dir(&d).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("extracting"))
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn hostile_zip_stays_inside() {
        use zip::write::SimpleFileOptions;
        let d = tmpdir("evil");
        let f = std::fs::File::create(d.join("evil.zip")).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let o = SimpleFileOptions::default();
        z.start_file("../escaped.txt", o).unwrap();
        z.write_all(b"x").unwrap();
        z.add_symlink("out", "/etc", o).unwrap();
        z.start_file("ok.txt", o).unwrap();
        z.write_all(b"fine").unwrap();
        z.finish().unwrap();
        std::fs::create_dir(d.join("dest")).unwrap();
        let out = extract(&d.join("evil.zip"), &d.join("dest"), &Mutex::new(String::new())).unwrap();
        assert!(!d.join("escaped.txt").exists());
        assert!(!d.join("dest/out").exists(), "absolute symlink must not be created");
        assert_eq!(out, d.join("dest/ok.txt"));
        std::fs::remove_dir_all(d).unwrap();
    }
}
