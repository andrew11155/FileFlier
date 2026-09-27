//! "Open With…": finds installed apps for a file's type (freedesktop .desktop
//! files and mimeapps.list), launches them, and can make one the default.
//! Inside Flatpak the desktop's own chooser is used instead (the OpenURI portal).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct App {
    /// Desktop file id, e.g. "org.gnome.eog.desktop".
    pub id: String,
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub mimes: Vec<String>,
    pub path: PathBuf,
}

fn data_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = dirs::data_dir() {
        v.push(d);
    }
    let sys = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let sys = if sys.is_empty() { "/usr/local/share:/usr/share".to_string() } else { sys };
    v.extend(sys.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    // Flatpak-installed apps, in case XDG_DATA_DIRS doesn't list them.
    if let Some(h) = dirs::home_dir() {
        v.push(h.join(".local/share/flatpak/exports/share"));
    }
    v.push("/var/lib/flatpak/exports/share".into());
    v.dedup();
    v
}

/// Unescapes a desktop-file string value (\s \n \t \r \\).
fn unescape_value(v: &str) -> String {
    let mut out = String::new();
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('s') => out.push(' '),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn parse_desktop(id: &str, path: &Path, text: &str) -> Option<App> {
    let lang = std::env::var("LANG").unwrap_or_default();
    let lang = lang.split(['.', '@']).next().unwrap_or("").to_string();
    let short = lang.split('_').next().unwrap_or("").to_string();
    let mut in_entry = false;
    let mut kv: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            kv.entry(k.trim().to_string()).or_insert_with(|| v.trim().to_string());
        }
    }
    let get = |k: &str| kv.get(k).map(|v| unescape_value(v));
    if get("Type").as_deref() != Some("Application")
        || get("NoDisplay").as_deref() == Some("true")
        || get("Hidden").as_deref() == Some("true")
        || get("Terminal").as_deref() == Some("true")
    {
        return None;
    }
    let exec = get("Exec")?;
    if let Some(t) = get("TryExec")
        && !crate::preview::have_path(&t)
    {
        return None;
    }
    let name = get(&format!("Name[{lang}]")).or_else(|| get(&format!("Name[{short}]"))).or_else(|| get("Name"))?;
    let mimes = get("MimeType")
        .map(|m| m.split(';').filter(|s| !s.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    Some(App { id: id.to_string(), name, exec, icon: get("Icon"), mimes, path: path.to_path_buf() })
}

/// Every visible application, first definition of each id winning (user dirs first).
pub fn all_apps() -> Vec<App> {
    let mut seen = std::collections::HashSet::new();
    let mut apps = Vec::new();
    for d in data_dirs() {
        let root = d.join("applications");
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().is_none_or(|x| x != "desktop") {
                    continue;
                }
                // Desktop ids use '-' for subdirectories.
                let id = p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().replace('/', "-");
                if !seen.insert(id.clone()) {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&p)
                    && let Some(app) = parse_desktop(&id, &p, &text)
                {
                    apps.push(app);
                }
            }
        }
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

// ------------------------------------------------------------------ MIME types

struct MimeDb {
    /// "*.jpg" -> "image/jpeg" (lowercased pattern); exact names separately.
    ext: HashMap<String, (u32, String)>,
    names: HashMap<String, String>,
    parents: HashMap<String, Vec<String>>,
}

fn mime_db() -> &'static MimeDb {
    static DB: OnceLock<MimeDb> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = MimeDb { ext: HashMap::new(), names: HashMap::new(), parents: HashMap::new() };
        for d in data_dirs().iter().rev() {
            let mime = d.join("mime");
            if let Ok(text) = std::fs::read_to_string(mime.join("globs2")) {
                for line in text.lines().filter(|l| !l.starts_with('#')) {
                    let mut f = line.splitn(3, ':');
                    let (Some(w), Some(m), Some(glob)) = (f.next(), f.next(), f.next()) else { continue };
                    let glob = glob.split(':').next().unwrap_or(glob);
                    let w: u32 = w.parse().unwrap_or(50);
                    if let Some(ext) = glob.strip_prefix("*.")
                        && !ext.contains(['*', '?', '['])
                    {
                        let e = db.ext.entry(ext.to_lowercase()).or_insert((0, String::new()));
                        if w > e.0 || e.1.is_empty() {
                            *e = (w, m.to_string());
                        }
                    } else if !glob.contains(['*', '?', '[']) {
                        db.names.insert(glob.to_lowercase(), m.to_string());
                    }
                }
            }
            if let Ok(text) = std::fs::read_to_string(mime.join("subclasses")) {
                for line in text.lines() {
                    if let Some((m, p)) = line.split_once(' ') {
                        db.parents.entry(m.to_string()).or_default().push(p.to_string());
                    }
                }
            }
        }
        db
    })
}

/// The file's MIME type from its name ("application/octet-stream" if unknown).
pub fn mime_for(path: &Path) -> String {
    if path.is_dir() {
        return "inode/directory".into();
    }
    let db = mime_db();
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if let Some(m) = db.names.get(&name) {
        return m.clone();
    }
    // Longest matching extension wins ("tar.gz" before "gz").
    let mut rest = name.as_str();
    while let Some((_, ext)) = rest.split_once('.') {
        if let Some((_, m)) = db.ext.get(ext) {
            return m.clone();
        }
        rest = ext;
    }
    let text = crate::preview::read_head(path, 4096).is_ok_and(|b| !b.contains(&0) && std::str::from_utf8(&b).is_ok());
    if text { "text/plain".into() } else { "application/octet-stream".into() }
}

/// `mime` plus its ancestors (text/x-rust -> text/plain -> application/octet-stream).
pub fn mime_family(mime: &str) -> Vec<String> {
    let db = mime_db();
    let mut out = vec![mime.to_string()];
    let mut i = 0;
    while i < out.len() && out.len() < 16 {
        if let Some(ps) = db.parents.get(&out[i]) {
            for p in ps {
                if !out.contains(p) {
                    out.push(p.clone());
                }
            }
        }
        i += 1;
    }
    if mime.starts_with("text/") && !out.iter().any(|m| m == "text/plain") {
        out.push("text/plain".into());
    }
    out
}

fn mimeapps_files() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(c) = dirs::config_dir() {
        v.push(c.join("mimeapps.list"));
    }
    v.push("/etc/xdg/mimeapps.list".into());
    for d in data_dirs() {
        v.push(d.join("applications/mimeapps.list"));
        v.push(d.join("applications/defaults.list"));
    }
    v
}

/// The default app id for `mime`, per mimeapps.list.
pub fn default_app(mime: &str) -> Option<String> {
    for f in mimeapps_files() {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let mut in_defaults = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_defaults = line == "[Default Applications]";
            } else if in_defaults
                && let Some((k, v)) = line.split_once('=')
                && k.trim() == mime
                && let Some(id) = v.split(';').map(str::trim).find(|s| !s.is_empty())
            {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Makes `app_id` the default for `mime` in ~/.config/mimeapps.list.
pub fn set_default(app_id: &str, mime: &str) -> Result<(), String> {
    let path = dirs::config_dir().ok_or("No config folder")?.join("mimeapps.list");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let updated = with_default(&text, mime, app_id);
    let tmp = path.with_extension("list.tmp");
    std::fs::write(&tmp, updated).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// Returns `text` (a mimeapps.list) with `mime=app;` set under [Default Applications].
pub fn with_default(text: &str, mime: &str, app_id: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_defaults = false;
    let mut done = false;
    let mut saw_section = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_defaults && !done {
                out.push(format!("{mime}={app_id};"));
                done = true;
            }
            in_defaults = t == "[Default Applications]";
            saw_section |= in_defaults;
        } else if in_defaults && t.split_once('=').is_some_and(|(k, _)| k.trim() == mime) {
            if !done {
                out.push(format!("{mime}={app_id};"));
                done = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if !done {
        if !saw_section {
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push("[Default Applications]".into());
        }
        out.push(format!("{mime}={app_id};"));
    }
    out.join("\n") + "\n"
}

/// Apps that declare support for `mime` (or a parent type), default first.
pub fn recommended(apps: &[App], mime: &str) -> Vec<usize> {
    let family = mime_family(mime);
    let default = default_app(mime);
    let mut v: Vec<(usize, usize)> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let rank = family.iter().position(|m| a.mimes.contains(m))?;
            let rank = if Some(&a.id) == default.as_ref() { 0 } else { rank + 1 };
            Some((rank, i))
        })
        .collect();
    v.sort();
    v.into_iter().map(|(_, i)| i).collect()
}

// ------------------------------------------------------------------ launching

/// Splits an Exec line into arguments, honoring double quotes.
fn split_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut has = false;
    let mut it = exec.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                in_quote = !in_quote;
                has = true;
            }
            '\\' if in_quote => {
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !in_quote => {
                if has || !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        args.push(cur);
    }
    args
}

/// Command lines for opening `files` with `app` (one per file for %f/%u apps).
pub fn command_lines(app: &App, files: &[PathBuf]) -> Vec<Vec<String>> {
    let to_uri = |p: &Path| {
        use std::os::unix::ffi::OsStrExt;
        let mut out = String::from("file://");
        for &b in p.as_os_str().as_bytes() {
            if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    };
    let args = split_exec(&app.exec);
    let single = args.iter().any(|a| a.contains("%f") || a.contains("%u"));
    let has_code = args.iter().any(|a| ["%f", "%F", "%u", "%U"].iter().any(|c| a.contains(c)));
    let groups: Vec<Vec<PathBuf>> =
        if single { files.iter().map(|f| vec![f.clone()]).collect() } else { vec![files.to_vec()] };
    groups
        .into_iter()
        .map(|group| {
            let mut out = Vec::new();
            for a in &args {
                match a.as_str() {
                    "%F" => out.extend(group.iter().map(|f| f.to_string_lossy().into_owned())),
                    "%U" => out.extend(group.iter().map(|f| to_uri(f))),
                    "%i" => {
                        if let Some(i) = &app.icon {
                            out.push("--icon".into());
                            out.push(i.clone());
                        }
                    }
                    _ => {
                        let first = group.first();
                        let s = a
                            .replace("%%", "\u{0}")
                            .replace("%f", &first.map(|f| f.to_string_lossy().into_owned()).unwrap_or_default())
                            .replace("%u", &first.map(|f| to_uri(f)).unwrap_or_default())
                            .replace("%c", &app.name)
                            .replace("%k", &app.path.to_string_lossy());
                        let s = ["%d", "%D", "%n", "%N", "%v", "%m"].iter().fold(s, |s, c| s.replace(c, ""));
                        let s = s.replace('\u{0}', "%");
                        if !s.is_empty() {
                            out.push(s);
                        }
                    }
                }
            }
            if !has_code {
                out.extend(group.iter().map(|f| f.to_string_lossy().into_owned()));
            }
            out
        })
        .collect()
}

pub fn launch(app: &App, files: &[PathBuf]) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    for argv in command_lines(app, files) {
        let Some((prog, rest)) = argv.split_first() else { continue };
        let mut child = std::process::Command::new(prog)
            .args(rest)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|e| format!("Couldn't start {}: {e}", app.name))?;
        std::thread::spawn(move || child.wait());
    }
    Ok(())
}

/// Inside Flatpak: ask the desktop to show its own "Open With" chooser.
pub fn portal_open_with(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let conn = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
    let mut opts: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    opts.insert("ask", true.into());
    let fd = zbus::zvariant::Fd::from(&file);
    conn.call_method(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        Some("org.freedesktop.portal.OpenURI"),
        "OpenFile",
        &("", fd, opts),
    )
    .map_err(|e| format!("The desktop couldn't show its app chooser: {e}"))?;
    Ok(())
}

// ------------------------------------------------------------------ icons

/// Finds an app icon file (hicolor theme or pixmaps).
pub fn icon_path(icon: &str) -> Option<PathBuf> {
    let p = Path::new(icon);
    if p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    let dirs = data_dirs();
    for size in ["64x64", "48x48", "128x128", "scalable", "256x256", "96x96", "32x32"] {
        for d in &dirs {
            for ext in ["png", "svg"] {
                let f = d.join(format!("icons/hicolor/{size}/apps/{icon}.{ext}"));
                if f.is_file() {
                    return Some(f);
                }
            }
        }
    }
    for d in &dirs {
        for ext in ["png", "svg"] {
            let f = d.join(format!("pixmaps/{icon}.{ext}"));
            if f.is_file() {
                return Some(f);
            }
        }
    }
    None
}

/// State of the Open With dialog.
pub struct OpenWithView {
    pub files: Vec<PathBuf>,
    pub mime: String,
    pub apps: Vec<App>,
    /// Indices into `apps`, recommended first, then the rest.
    pub recommended: Vec<usize>,
    pub others: Vec<usize>,
    pub default: Option<String>,
    pub filter: String,
    pub cursor: usize,
    pub always: bool,
    pub icons: HashMap<String, egui::TextureHandle>,
    icon_rx: std::sync::mpsc::Receiver<(String, crate::preview::Rgba)>,
}

impl OpenWithView {
    pub fn new(files: Vec<PathBuf>, ctx: &egui::Context) -> Self {
        let mime = mime_for(&files[0]);
        let apps = all_apps();
        let recommended = recommended(&apps, &mime);
        let others = (0..apps.len()).filter(|i| !recommended.contains(i)).collect();
        let default = default_app(&mime);
        let (tx, icon_rx) = std::sync::mpsc::channel();
        let wanted: Vec<(String, String)> = apps.iter().filter_map(|a| Some((a.id.clone(), a.icon.clone()?))).collect();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for (id, icon) in wanted {
                if let Some(img) = icon_path(&icon).and_then(|p| crate::preview::load_icon(&p, 64))
                    && tx.send((id, img)).is_err()
                {
                    return;
                }
            }
            ctx.request_repaint();
        });
        OpenWithView {
            files,
            mime,
            apps,
            recommended,
            others,
            default,
            filter: String::new(),
            cursor: 0,
            always: false,
            icons: HashMap::new(),
            icon_rx,
        }
    }

    pub fn poll_icons(&mut self, ctx: &egui::Context) {
        while let Ok((id, img)) = self.icon_rx.try_recv() {
            let tex = ctx.load_texture(format!("appicon:{id}"), img.to_color_image(), egui::TextureOptions::LINEAR);
            self.icons.insert(id, tex);
        }
    }

    /// Visible (recommended, others) after filtering.
    pub fn visible(&self) -> (Vec<usize>, Vec<usize>) {
        let q = self.filter.trim().to_lowercase();
        let keep = |i: &&usize| q.is_empty() || self.apps[**i].name.to_lowercase().contains(&q);
        (self.recommended.iter().filter(keep).copied().collect(), self.others.iter().filter(keep).copied().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_field_codes() {
        let app = App {
            id: "x.desktop".into(),
            name: "X".into(),
            exec: r#"viewer --name "My App" %U"#.into(),
            icon: None,
            mimes: vec![],
            path: "/x.desktop".into(),
        };
        let files = vec![PathBuf::from("/tmp/a b.jpg"), PathBuf::from("/tmp/c.jpg")];
        assert_eq!(
            command_lines(&app, &files),
            vec![vec!["viewer", "--name", "My App", "file:///tmp/a%20b.jpg", "file:///tmp/c.jpg"]]
        );
        let app = App { exec: "gimp %f".into(), ..app };
        assert_eq!(command_lines(&app, &files), vec![vec!["gimp", "/tmp/a b.jpg"], vec!["gimp", "/tmp/c.jpg"]]);
        let app = App { exec: "legacy".into(), ..app };
        assert_eq!(command_lines(&app, &files[..1]), vec![vec!["legacy", "/tmp/a b.jpg"]]);
    }

    #[test]
    fn mimeapps_editing() {
        let t = "[Added Associations]\nimage/png=a.desktop;\n\n[Default Applications]\nimage/png=old.desktop;\ntext/plain=gedit.desktop;\n";
        let out = with_default(t, "image/png", "new.desktop");
        assert!(out.contains("[Default Applications]\nimage/png=new.desktop;\ntext/plain=gedit.desktop;"));
        assert!(out.contains("[Added Associations]\nimage/png=a.desktop;"));
        assert_eq!(with_default("", "a/b", "c.desktop"), "[Default Applications]\na/b=c.desktop;\n");
    }

    #[test]
    fn parses_desktop_entries() {
        let t = "[Desktop Entry]\nType=Application\nName=Viewer\nExec=viewer %f\nMimeType=image/png;image/jpeg;\n[Desktop Action new]\nName=Other\n";
        let a = parse_desktop("v.desktop", Path::new("/v.desktop"), t).unwrap();
        assert_eq!(a.name, "Viewer");
        assert_eq!(a.mimes, vec!["image/png", "image/jpeg"]);
        assert!(
            parse_desktop("t", Path::new("/t"), "[Desktop Entry]\nType=Application\nName=T\nExec=t\nNoDisplay=true\n")
                .is_none()
        );
    }
}
