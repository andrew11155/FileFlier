//! The Dropbox extension: talks to the Dropbox desktop app (`dropboxd`) over its local
//! command socket, as its own Nautilus extension does. That gives the right-click
//! items Dropbox offers for a file (copy link, share, version history, ...), sync
//! status badges, and selective sync ("don't sync this folder on this computer").
//!
//! Nothing here talks to the internet itself; Dropbox does. When Dropbox isn't
//! installed or running, every call just fails and nothing is shown.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A file's sync state, as Dropbox reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sync {
    UpToDate,
    Syncing,
    Unsyncable,
}

/// A right-click item Dropbox offers: label, tooltip, and the verb to run it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuOption {
    pub label: String,
    pub tooltip: String,
    pub verb: String,
}

fn socket_path() -> Option<PathBuf> {
    Some(std::env::var_os("HOME").map(PathBuf::from)?.join(".dropbox/command_socket"))
}

/// One command to dropboxd: `name`, then tab-separated `key value...` lines. The reply
/// is "ok" and lines of the same shape, keyed by their first field.
fn call(name: &str, args: &[(&str, Vec<String>)]) -> Result<HashMap<String, Vec<String>>, String> {
    let sock = socket_path().ok_or("no home folder")?;
    let mut s = UnixStream::connect(&sock).map_err(|_| "Dropbox isn't running".to_string())?;
    let timeout = Some(Duration::from_secs(5));
    s.set_read_timeout(timeout).ok();
    s.set_write_timeout(timeout).ok();
    let mut msg = format!("{name}\n");
    for (k, v) in args {
        msg.push_str(k);
        for x in v {
            msg.push('\t');
            msg.push_str(x);
        }
        msg.push('\n');
    }
    msg.push_str("done\n");
    s.write_all(msg.as_bytes()).map_err(|e| e.to_string())?;
    parse_reply(BufReader::new(s).lines().map_while(Result::ok))
}

fn parse_reply(lines: impl Iterator<Item = String>) -> Result<HashMap<String, Vec<String>>, String> {
    let mut lines = lines;
    let status = lines.next().ok_or("no reply from Dropbox")?;
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for line in lines {
        if line == "done" {
            break;
        }
        let mut f = line.split('\t').map(str::to_string);
        if let Some(k) = f.next() {
            out.insert(k, f.collect());
        }
    }
    if status != "ok" {
        let why = out.get("message").map(|m| m.join(" ")).unwrap_or(status);
        return Err(why);
    }
    Ok(out)
}

fn path_arg(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| p.to_string_lossy().into_owned()).collect()
}

/// The Dropbox folders on this computer, from `~/.dropbox/info.json`. A team account's
/// folder sits inside the team's own folder, which is included too.
pub fn roots() -> &'static [PathBuf] {
    static ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return vec![] };
        std::fs::read_to_string(home.join(".dropbox/info.json")).map(|s| parse_roots(&s)).unwrap_or_default()
    })
}

fn parse_roots(json: &str) -> Vec<PathBuf> {
    let Ok(serde_json::Value::Object(accounts)) = serde_json::from_str(json) else { return vec![] };
    let mut out = Vec::new();
    for a in accounts.values() {
        let Some(p) = a.get("path").and_then(|p| p.as_str()).map(PathBuf::from) else { continue };
        if a.get("is_team").and_then(|t| t.as_bool()).unwrap_or(false)
            && let Some(parent) = p.parent()
            && !out.iter().any(|o: &PathBuf| o == parent)
        {
            out.push(parent.to_path_buf());
        }
        out.push(p);
    }
    out
}

/// Whether `p` is in a Dropbox folder (or is one).
pub fn inside(p: &Path) -> bool {
    roots().iter().any(|r| p.starts_with(r))
}

/// The right-click items Dropbox offers for `paths`. Its own "Right-click actions..."
/// settings item is left out.
pub fn options(paths: &[PathBuf]) -> Vec<MenuOption> {
    let Ok(r) = call("icon_overlay_context_options", &[("paths", path_arg(paths))]) else { return vec![] };
    r.get("options").map(|o| parse_options(o)).unwrap_or_default()
}

fn parse_options(fields: &[String]) -> Vec<MenuOption> {
    fields
        .iter()
        .filter_map(|f| {
            let mut p = f.splitn(3, '~');
            let (label, tooltip, verb) = (p.next()?, p.next()?, p.next()?);
            (verb != "context_menu_settings" && !label.is_empty()).then(|| MenuOption {
                label: label.replace("...", "…"),
                tooltip: tooltip.to_string(),
                verb: verb.to_string(),
            })
        })
        .collect()
}

/// What finished in the background, for the app to show.
pub enum Event {
    /// A shared link, to put on the clipboard.
    Link(String),
    Info(String),
    Error(String),
}

static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());

fn post(e: Event, ctx: &egui::Context) {
    if let Ok(mut v) = EVENTS.lock() {
        v.push(e);
    }
    ctx.request_repaint();
}

/// Takes what finished since the last call.
pub fn events() -> Vec<Event> {
    EVENTS.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
}

/// Runs a Dropbox item (off the UI thread: Dropbox may ask its servers first).
pub fn run(paths: Vec<PathBuf>, verb: String, ctx: &egui::Context) {
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let e = if verb == "copy_link" {
            // Ask for the link and copy it ourselves: Dropbox's own clipboard copy
            // doesn't reach Wayland apps.
            match paths.first().map(|p| call("get_shared_link", &[("path", path_arg(std::slice::from_ref(p)))])) {
                Some(Ok(r)) => match r.get("link").and_then(|l| l.first()) {
                    Some(link) if !link.is_empty() => Event::Link(link.clone()),
                    _ => Event::Error("Dropbox didn't give a link for this item".into()),
                },
                Some(Err(e)) => Event::Error(format!("Couldn't get a Dropbox link: {e}")),
                None => return,
            }
        } else {
            match call("icon_overlay_context_action", &[("paths", path_arg(&paths)), ("verb", vec![verb])]) {
                Ok(_) => return,
                Err(e) => Event::Error(format!("Dropbox: {e}")),
            }
        };
        post(e, &ctx);
    });
}

/// Folders set not to sync on this computer (selective sync).
pub fn excluded() -> Result<Vec<PathBuf>, String> {
    let r = call("get_ignore_set", &[])?;
    let mut v: Vec<PathBuf> = r.get("ignore_set").into_iter().flatten().map(PathBuf::from).collect();
    v.sort();
    Ok(v)
}

/// Stops syncing `paths` here (Dropbox removes the local copies; they stay online),
/// or, with `sync`, starts syncing them again.
pub fn set_synced(paths: Vec<PathBuf>, sync: bool, ctx: &egui::Context) {
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let n = paths.len();
        let cmd = if sync { "ignore_set_remove" } else { "ignore_set_add" };
        let e = match call(cmd, &[("paths", path_arg(&paths))]) {
            Ok(_) if sync => Event::Info(format!("Syncing {n} folder{} on this computer again", plural(n))),
            Ok(_) => Event::Info(format!("{n} folder{} kept online only, not on this computer", plural(n))),
            Err(e) => Event::Error(format!("Dropbox: {e}")),
        };
        post(e, &ctx);
    });
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

// ------------------------------------------------------------ sync badges

struct Cache {
    states: Mutex<HashMap<PathBuf, (Option<Sync>, Instant)>>,
    ask: Mutex<Sender<PathBuf>>,
}

/// How long a reported state is trusted before asking again.
const FRESH: Duration = Duration::from_secs(4);

fn cache(ctx: &egui::Context) -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| {
        let (tx, rx) = channel::<PathBuf>();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            while let Ok(p) = rx.recv() {
                let state = call("icon_overlay_file_status", &[("path", path_arg(std::slice::from_ref(&p)))])
                    .ok()
                    .and_then(|r| r.get("status").and_then(|s| s.first()).map(|s| parse_status(s)))
                    .flatten();
                let c = CACHE.get().expect("set before the thread asks");
                let mut m = c.states.lock().unwrap();
                let old = m.insert(p, (state, Instant::now()));
                if old.map(|o| o.0) != Some(state) {
                    ctx.request_repaint();
                }
            }
        });
        Cache { states: Mutex::new(HashMap::new()), ask: Mutex::new(tx) }
    })
}

fn parse_status(s: &str) -> Option<Sync> {
    match s {
        "up to date" => Some(Sync::UpToDate),
        "syncing" => Some(Sync::Syncing),
        "unsyncable" => Some(Sync::Unsyncable),
        _ => None,
    }
}

/// The sync state of `p` if it's in a Dropbox folder: what's known now, asking Dropbox
/// in the background when it's missing or old (a repaint follows a change).
pub fn status(p: &Path, ctx: &egui::Context) -> Option<Sync> {
    if !inside(p) {
        return None;
    }
    let c = cache(ctx);
    let mut m = c.states.lock().ok()?;
    let now = Instant::now();
    match m.get_mut(p) {
        Some((state, at)) => {
            if now.duration_since(*at) > FRESH {
                *at = now;
                let _ = c.ask.lock().map(|a| a.send(p.to_path_buf()));
            }
            // Keep checking while something syncs, so the badge turns green.
            if *state == Some(Sync::Syncing) {
                ctx.request_repaint_after(FRESH);
            }
            *state
        }
        None => {
            m.insert(p.to_path_buf(), (None, now));
            let _ = c.ask.lock().map(|a| a.send(p.to_path_buf()));
            None
        }
    }
}

/// Draws a sync badge on the bottom-right corner of an icon.
pub fn badge(p: &egui::Painter, icon: egui::Rect, state: Sync, bg: egui::Color32) {
    use egui::{Color32, Stroke, pos2, vec2};
    let d = (icon.width() * 0.42).clamp(9.0, 18.0);
    let c = icon.right_bottom() - vec2(d * 0.42, d * 0.42);
    let r = d / 2.0;
    let color = match state {
        Sync::UpToDate => Color32::from_rgb(38, 162, 105),
        Sync::Syncing => Color32::from_rgb(53, 132, 228),
        Sync::Unsyncable => Color32::from_rgb(224, 27, 36),
    };
    p.circle_filled(c, r + 1.2, bg);
    p.circle_filled(c, r, color);
    let s = Stroke::new((d * 0.13).max(1.3), Color32::WHITE);
    match state {
        Sync::UpToDate => {
            let pts = [c + vec2(-r * 0.45, 0.0), c + vec2(-r * 0.1, r * 0.35), c + vec2(r * 0.5, -r * 0.35)];
            p.line_segment([pts[0], pts[1]], s);
            p.line_segment([pts[1], pts[2]], s);
        }
        Sync::Syncing => {
            // Two arrows chasing each other round.
            let n = 10;
            for half in [0.0f32, std::f32::consts::PI] {
                let arc: Vec<_> = (0..=n)
                    .map(|i| {
                        let a = half + 0.35 + i as f32 / n as f32 * 2.2;
                        pos2(c.x + a.cos() * r * 0.5, c.y + a.sin() * r * 0.5)
                    })
                    .collect();
                p.add(egui::Shape::line(arc, s));
            }
        }
        Sync::Unsyncable => {
            let k = r * 0.38;
            p.line_segment([c + vec2(-k, -k), c + vec2(k, k)], s);
            p.line_segment([c + vec2(-k, k), c + vec2(k, -k)], s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_replies_and_options() {
        let lines = ["ok", "options\tCopy Dropbox link~Copy a link~copy_link\tShare...~Share it~share", "done"];
        let r = parse_reply(lines.iter().map(|s| s.to_string())).unwrap();
        let o = parse_options(&r["options"]);
        assert_eq!(o.len(), 2);
        assert_eq!((o[0].label.as_str(), o[0].verb.as_str()), ("Copy Dropbox link", "copy_link"));
        assert_eq!(o[1].label, "Share…");
        let settings = vec!["Right-click actions...~x~context_menu_settings".to_string()];
        assert!(parse_options(&settings).is_empty());
        let err = parse_reply(["notok", "message\tNo such file", "done"].iter().map(|s| s.to_string()));
        assert_eq!(err.unwrap_err(), "No such file");
    }

    #[test]
    fn finds_dropbox_folders() {
        let j = r#"{"business": {"path": "/home/a/Acme Dropbox/Ann", "is_team": true},
                    "personal": {"path": "/home/a/Dropbox"}}"#;
        let mut r = parse_roots(j);
        r.sort();
        assert_eq!(r, ["/home/a/Acme Dropbox", "/home/a/Acme Dropbox/Ann", "/home/a/Dropbox"].map(PathBuf::from));
        assert_eq!(parse_status("up to date"), Some(Sync::UpToDate));
        assert_eq!(parse_status("unwatched"), None);
    }
}
