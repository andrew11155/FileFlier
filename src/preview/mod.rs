//! Previews for the inspector, Quick Look and grid thumbnails.
//!
//! Everything is decoded on background threads. Pure-Rust decoders run in-process
//! (behind `catch_unwind`); C libraries (libheif) and the PDF renderer run in a
//! short-lived helper process (`file-flier --preview-helper ...`) so a malformed
//! file can never take the app down. `ffmpeg`/`ffprobe` (present in the Flatpak
//! runtime) handle video and a few image formats, and LibreOffice, when installed,
//! renders real page previews of office documents.

mod docs;
pub mod helper;
mod images;
mod media;
mod misc;
pub mod thumbs;

use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use crate::fs_model::Entry;

/// A decoded image in straight (non-premultiplied) RGBA.
#[derive(Clone)]
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl Rgba {
    pub fn from_image(img: image::DynamicImage, max: u32) -> Self {
        let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
        let buf = img.into_rgba8();
        Rgba { w: buf.width(), h: buf.height(), px: buf.into_raw() }
    }

    pub fn to_color_image(&self) -> egui::ColorImage {
        egui::ColorImage::from_rgba_unmultiplied([self.w as usize, self.h as usize], &self.px)
    }
}

/// One block of a text document preview.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(u8, String),
    Para(String),
    Bullet(String),
    Row(Vec<String>),
    Slide(usize),
}

#[derive(Clone)]
pub struct ListItem {
    pub name: String,
    pub size: Option<u64>,
    pub is_dir: bool,
}

#[derive(Clone)]
pub enum Content {
    /// Photos, drawings, video frames, covers, font samples...
    Image(Rgba),
    /// A page of a PDF (or of an office document converted to PDF).
    Page {
        img: Rgba,
        index: usize,
        count: usize,
    },
    Text {
        text: String,
        syntax: String,
        truncated: bool,
    },
    Table {
        rows: Vec<Vec<String>>,
        truncated: bool,
    },
    Document {
        thumb: Option<Rgba>,
        blocks: Vec<Block>,
    },
    Dir(Vec<Entry>),
    Listing(Vec<ListItem>),
    /// Nothing to draw beyond the file's icon, optionally with a note.
    Icon(Option<String>),
    Error(String),
}

#[derive(Clone)]
pub struct Loaded {
    pub content: Content,
    /// Type-specific details (dimensions, camera, duration, author...).
    pub info: Vec<(String, String)>,
    /// A better "Kind" than the extension-based one, if known.
    pub kind: Option<String>,
    pub times: Times,
}

/// Timestamps and mode, read on the worker (stat can block on network mounts).
#[derive(Clone, Copy, Default)]
pub struct Times {
    pub created: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub mode: Option<u32>,
}

impl Loaded {
    pub fn new(content: Content) -> Self {
        Loaded { content, info: Vec::new(), kind: None, times: Times::default() }
    }
    fn with_info(mut self, info: Vec<(String, String)>) -> Self {
        self.info = info;
        self
    }
    fn error(e: impl std::fmt::Display) -> Self {
        Loaded::new(Content::Error(e.to_string()))
    }
}

pub fn row(k: &str, v: impl Into<String>) -> (String, String) {
    (k.to_string(), v.into())
}

// ------------------------------------------------------------------ file types

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Raster,
    Heif,
    Jxl,
    Svg,
    CameraRaw,
    Pdf,
    Video,
    Audio,
    Word,
    Sheet,
    Slides,
    OdfDrawing,
    Rtf,
    LegacyWord,
    LegacyOffice,
    Epub,
    IWork,
    Zip,
    Tar,
    TarGz,
    Font,
    Csv,
    Other,
}

pub fn ext_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn classify(path: &Path) -> Kind {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.ends_with(".tar.gz") {
        return Kind::TarGz;
    }
    match ext_of(path).as_str() {
        "png" | "jpg" | "jpeg" | "jpe" | "jfif" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "ico" | "tga" | "qoi"
        | "pnm" | "pbm" | "pgm" | "ppm" | "pam" | "hdr" | "exr" | "dds" => Kind::Raster,
        "heic" | "heif" | "hif" | "avif" => Kind::Heif,
        "jxl" => Kind::Jxl,
        "svg" | "svgz" => Kind::Svg,
        "cr2" | "cr3" | "crw" | "nef" | "nrw" | "arw" | "srf" | "sr2" | "dng" | "raf" | "orf" | "rw2" | "pef"
        | "srw" | "x3f" | "erf" | "kdc" | "dcr" | "mrw" | "3fr" | "iiq" | "rwl" => Kind::CameraRaw,
        "pdf" | "ai" => Kind::Pdf,
        "mp4" | "m4v" | "mkv" | "webm" | "mov" | "avi" | "wmv" | "flv" | "mpg" | "mpeg" | "m2ts" | "mts" | "ts"
        | "3gp" | "ogv" | "vob" | "asf" => Kind::Video,
        "mp3" | "flac" | "ogg" | "oga" | "opus" | "m4a" | "m4b" | "aac" | "wav" | "aiff" | "aif" | "wv" | "ape"
        | "mpc" | "wma" | "spx" | "dsf" => Kind::Audio,
        "docx" | "docm" | "dotx" | "dotm" | "odt" | "ott" => Kind::Word,
        "xlsx" | "xlsm" | "xltx" | "ods" | "ots" => Kind::Sheet,
        "pptx" | "pptm" | "potx" | "ppsx" | "odp" | "otp" => Kind::Slides,
        "odg" | "otg" => Kind::OdfDrawing,
        "rtf" => Kind::Rtf,
        "doc" | "dot" => Kind::LegacyWord,
        "xls" | "xlt" | "ppt" | "pps" | "pot" | "pub" | "vsd" | "wpd" | "wps" => Kind::LegacyOffice,
        "epub" => Kind::Epub,
        "pages" | "numbers" | "key" => Kind::IWork,
        "zip" | "jar" | "apk" | "xpi" | "whl" | "cbz" | "nupkg" | "aar" | "ipa" | "vsix" | "crx" => Kind::Zip,
        "tar" => Kind::Tar,
        "tgz" => Kind::TarGz,
        "ttf" | "otf" | "ttc" | "otc" => Kind::Font,
        "csv" | "tsv" => Kind::Csv,
        _ => Kind::Other,
    }
}

impl Kind {
    /// Office formats LibreOffice can turn into real page previews. Modern
    /// spreadsheets are left out: the built-in table view reads better.
    fn is_office(self) -> bool {
        matches!(self, Kind::Word | Kind::Slides | Kind::OdfDrawing | Kind::Rtf | Kind::LegacyWord | Kind::LegacyOffice)
    }

    /// Whether grid view can show a picture for this type.
    pub fn has_thumbnail(self) -> bool {
        matches!(
            self,
            Kind::Raster
                | Kind::Heif
                | Kind::Jxl
                | Kind::Svg
                | Kind::CameraRaw
                | Kind::Pdf
                | Kind::Video
                | Kind::Audio
                | Kind::Word
                | Kind::Sheet
                | Kind::Slides
                | Kind::OdfDrawing
                | Kind::Epub
                | Kind::IWork
                | Kind::Zip
        )
    }
}

// ------------------------------------------------------------------ loading

pub struct Options {
    /// Try LibreOffice for page previews of office documents.
    pub office_pages: bool,
    /// Skip expensive work (folder sizes) on network mounts.
    pub remote: bool,
}

/// Context for one load: lets long jobs bail out once the user moved on,
/// and send an improved result later (e.g. LibreOffice pages after the text).
pub struct Cx<'a> {
    cancel: &'a dyn Fn() -> bool,
    emit: &'a dyn Fn(Loaded),
}

impl Cx<'_> {
    pub fn cancelled(&self) -> bool {
        (self.cancel)()
    }
}

/// Builds the preview for `path`, possibly calling `cx.emit` more than once.
fn load(path: &Path, is_dir: bool, page: usize, opts: &Options, cx: &Cx) {
    if is_dir {
        return misc::load_dir(path, opts, cx);
    }
    let kind = classify(path);
    if page > 0 {
        // Page flips: PDFs directly, office documents through their converted PDF.
        let pdf = if kind == Kind::Pdf { Some(path.to_path_buf()) } else { docs::converted_pdf(path) };
        if let Some(pdf) = pdf {
            return (cx.emit)(pdf_page(&pdf, page, 2000));
        }
    }
    let loaded = match kind {
        Kind::Raster | Kind::Heif | Kind::Jxl | Kind::Svg | Kind::CameraRaw => images::load(path, kind),
        Kind::Pdf => pdf_page(path, 0, 2000),
        Kind::Video => media::load_video(path, cx),
        Kind::Audio => media::load_audio(path),
        Kind::Zip | Kind::Tar | Kind::TarGz => misc::load_archive(path, kind),
        Kind::Font => misc::load_font(path),
        Kind::Csv => misc::load_csv(path),
        Kind::Other => misc::load_text_or_binary(path),
        _ => docs::load(path, kind),
    };
    let loaded_ok = !matches!(loaded.content, Content::Error(_));
    (cx.emit)(loaded.clone());
    if kind.is_office()
        && opts.office_pages
        && !cx.cancelled()
        && let Some(pdf) = docs::convert_with_libreoffice(path, cx)
    {
        let mut paged = pdf_page(&pdf, 0, 2000);
        if matches!(paged.content, Content::Page { .. }) {
            // Keep the document's own details (author, word count...).
            if loaded_ok {
                paged.info = loaded.info;
                paged.kind = loaded.kind;
            }
            (cx.emit)(paged);
        }
    }
}

fn pdf_page(pdf: &Path, page: usize, max: u32) -> Loaded {
    match helper::run(helper::Op::Pdf, pdf, max, page) {
        Ok(out) => {
            let count = out.count.max(1);
            let mut l = Loaded::new(Content::Page { img: out.img, index: page.min(count - 1), count });
            l.info = out.info;
            l.info.insert(0, row("Pages", count.to_string()));
            l
        }
        Err(e) => Loaded::error(e),
    }
}

/// Thumbnail-sized picture for grid view, or None.
pub fn thumbnail(path: &Path, max: u32) -> Option<Rgba> {
    let kind = classify(path);
    let r = catch_unwind(AssertUnwindSafe(|| match kind {
        Kind::Raster | Kind::Heif | Kind::Jxl | Kind::Svg | Kind::CameraRaw => images::thumbnail(path, kind, max),
        Kind::Pdf => helper::run(helper::Op::Pdf, path, max, 0).ok().map(|o| o.img),
        Kind::Video => media::video_frame(path, max, None),
        Kind::Audio => media::audio_cover(path, max),
        Kind::Zip => misc::comic_cover(path, max),
        _ => docs::embedded_thumbnail(path, kind, max).or_else(|| {
            // Office files previewed before have a LibreOffice-rendered PDF cached.
            let pdf = docs::converted_pdf(path).filter(|_| kind.is_office())?;
            helper::run(helper::Op::Pdf, &pdf, max, 0).ok().map(|o| o.img)
        }),
    }));
    r.ok().flatten()
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Key {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub page: usize,
}

struct Req {
    key: Key,
    is_dir: bool,
    opts: Options,
    generation: u64,
}

/// What the preview panel currently shows.
pub struct Shown {
    pub key: Key,
    pub loaded: Loaded,
    pub tex: Option<egui::TextureHandle>,
    /// A better result (LibreOffice pages, folder size) may still arrive.
    pub refining: bool,
}

/// Loads previews for the item under the cursor on a background thread;
/// the newest request always wins.
pub struct Previewer {
    tx: Sender<Req>,
    rx: Receiver<(Key, Loaded)>,
    generation: Arc<AtomicU64>,
    wanted: Option<Key>,
    pub shown: Option<Shown>,
    pub thumbs: thumbs::Thumbs,
}

impl Previewer {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, req_rx) = channel::<Req>();
        let (res_tx, rx) = channel();
        let generation = Arc::new(AtomicU64::new(0));
        let gen2 = generation.clone();
        let ctx2 = ctx.clone();
        std::thread::Builder::new()
            .name("preview".into())
            .stack_size(16 << 20)
            .spawn(move || {
                while let Ok(mut req) = req_rx.recv() {
                    while let Ok(newer) = req_rx.try_recv() {
                        req = newer;
                    }
                    if req.generation != gen2.load(Ordering::SeqCst) {
                        continue;
                    }
                    let cancel = || gen2.load(Ordering::SeqCst) != req.generation;
                    let times = std::fs::symlink_metadata(&req.key.path)
                        .map(|m| {
                            use std::os::unix::fs::PermissionsExt;
                            Times {
                                created: m.created().ok(),
                                accessed: m.accessed().ok(),
                                mode: Some(m.permissions().mode()),
                            }
                        })
                        .unwrap_or_default();
                    let emit = |mut l: Loaded| {
                        l.times = times;
                        if !cancel() {
                            let _ = res_tx.send((req.key.clone(), l));
                            ctx2.request_repaint();
                        }
                    };
                    let cx = Cx { cancel: &cancel, emit: &emit };
                    let r = catch_unwind(AssertUnwindSafe(|| {
                        load(&req.key.path, req.is_dir, req.key.page, &req.opts, &cx)
                    }));
                    if r.is_err() {
                        emit(Loaded::error("This file couldn't be previewed"));
                    }
                }
            })
            .expect("spawn preview thread");
        Previewer { tx, rx, generation, wanted: None, shown: None, thumbs: thumbs::Thumbs::new(ctx) }
    }

    /// Asks for a preview of `e` (page `page`); cheap to call every frame.
    pub fn want(&mut self, e: &Entry, page: usize, opts: Options) {
        let key = Key { path: e.path.clone(), modified: e.modified, page };
        if self.wanted.as_ref() == Some(&key) {
            return;
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // Never read pipes, sockets or devices: that can block forever.
        if !e.is_dir && !e.is_file {
            self.wanted = Some(key.clone());
            self.shown = Some(Shown { key, loaded: Loaded::new(Content::Icon(None)), tex: None, refining: false });
            return;
        }
        self.wanted = Some(key.clone());
        let _ = self.tx.send(Req { key, is_dir: e.is_dir, opts, generation });
    }

    pub fn is_loading(&self) -> bool {
        self.wanted.as_ref().is_some_and(|w| self.shown.as_ref().is_none_or(|s| &s.key != w))
    }

    /// Current preview for `path`, if one has arrived (possibly for another page).
    pub fn shown_for(&mut self, path: &Path) -> Option<&mut Shown> {
        self.shown.as_mut().filter(|s| s.key.path == path)
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((key, loaded)) = self.rx.try_recv() {
            if self.wanted.as_ref() != Some(&key) {
                continue;
            }
            let img = match &loaded.content {
                Content::Image(i) | Content::Page { img: i, .. } => Some(i),
                Content::Document { thumb: Some(i), .. } => Some(i),
                _ => None,
            };
            let tex = img.map(|i| {
                ctx.load_texture(
                    "preview",
                    i.to_color_image(),
                    egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear)),
                )
            });
            let refining = match &loaded.content {
                Content::Dir(_) => loaded.info.iter().any(|(_, v)| v == CALCULATING),
                _ => false,
            };
            self.shown = Some(Shown { key, loaded, tex, refining });
        }
    }
}

/// Finder-style "Kind" for common types, e.g. "JPEG image", "Word document".
pub fn friendly_kind(e: &Entry) -> Option<String> {
    if e.is_dir {
        return Some("Folder".into());
    }
    let ext = ext_of(&e.path);
    let k = match ext.as_str() {
        "jpg" | "jpeg" | "jpe" | "jfif" => "JPEG image",
        "png" => "PNG image",
        "gif" => "GIF image",
        "webp" => "WebP image",
        "bmp" => "BMP image",
        "tif" | "tiff" => "TIFF image",
        "heic" | "heif" | "hif" => "HEIC image",
        "avif" => "AVIF image",
        "jxl" => "JPEG XL image",
        "svg" | "svgz" => "SVG image",
        "ico" => "Icon",
        "pdf" => "PDF document",
        "doc" | "docx" | "docm" => "Word document",
        "dot" | "dotx" | "dotm" => "Word template",
        "odt" => "OpenDocument text",
        "ott" => "OpenDocument template",
        "rtf" => "Rich text document",
        "xls" | "xlsx" | "xlsm" => "Excel spreadsheet",
        "ods" => "OpenDocument spreadsheet",
        "ppt" | "pptx" | "pptm" | "pps" | "ppsx" => "PowerPoint presentation",
        "odp" => "OpenDocument presentation",
        "odg" => "OpenDocument drawing",
        "pages" => "Pages document",
        "numbers" => "Numbers spreadsheet",
        "key" => "Keynote presentation",
        "epub" => "EPUB e-book",
        "txt" => "Plain text",
        "md" | "markdown" => "Markdown document",
        "csv" => "CSV spreadsheet",
        "html" | "htm" => "HTML document",
        "json" => "JSON file",
        "mp3" => "MP3 audio",
        "flac" => "FLAC audio",
        "wav" => "WAV audio",
        "m4a" | "aac" => "AAC audio",
        "ogg" | "oga" | "opus" => "Ogg audio",
        "mp4" | "m4v" => "MPEG-4 video",
        "mov" => "QuickTime movie",
        "mkv" => "Matroska video",
        "webm" => "WebM video",
        "avi" => "AVI video",
        "zip" => "ZIP archive",
        "tar" => "Tar archive",
        "gz" | "tgz" => "Gzip archive",
        "7z" => "7-Zip archive",
        "rar" => "RAR archive",
        "iso" => "Disk image",
        "ttf" | "otf" | "ttc" => "Font",
        "deb" => "Debian package",
        "rpm" => "RPM package",
        "flatpak" | "flatpakref" => "Flatpak",
        "appimage" => "AppImage application",
        "sh" => "Shell script",
        "py" => "Python script",
        "rs" => "Rust source",
        _ => {
            if classify(&e.path) == Kind::CameraRaw {
                return Some(format!("{} camera RAW", ext.to_uppercase()));
            }
            return None;
        }
    };
    Some(k.into())
}

/// Starts slow one-time probes (LibreOffice) in the background.
pub fn warm_up() {
    std::thread::spawn(|| {
        images::fontdb(); // the first system font scan can take a second or two
        docs::libreoffice();
    });
}

/// Whether LibreOffice page previews are available; None while still checking.
pub fn libreoffice_available() -> Option<bool> {
    docs::libreoffice_probed()
}

pub const CALCULATING: &str = "Calculating…";

// ------------------------------------------------------------------ external tools

/// Runs `cmd`, returning stdout if it exits successfully within `timeout`.
/// Kills it early when `cancel` returns true.
pub fn run_cmd(mut cmd: Command, timeout: Duration, cancel: Option<&dyn Fn() -> bool>) -> Option<Vec<u8>> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        // Cap at 512 MB: a preview never needs more.
        let _ = (&mut out).take(512 << 20).read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let buf = reader.join().ok()?;
                return status.success().then_some(buf);
            }
            Ok(None) if start.elapsed() < timeout && !cancel.is_some_and(|c| c()) => {
                std::thread::sleep(Duration::from_millis(15))
            }
            _ => {
                // SIGTERM first so flatpak-spawn can forward it to the host process.
                // SAFETY: plain kill(2) on our own child's pid.
                unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
                std::thread::sleep(Duration::from_millis(100));
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Whether `program` is on PATH (cached).
pub fn have(program: &'static str) -> bool {
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<&'static str, bool>>> = OnceLock::new();
    let map = CACHE.get_or_init(Default::default);
    if let Some(&v) = map.lock().unwrap().get(program) {
        return v;
    }
    let found = std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| is_executable(&d.join(program))));
    map.lock().unwrap().insert(program, found);
    found
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Where File Flier keeps its own cache (converted documents, LibreOffice profile).
pub fn cache_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("file-flier"))
}

/// Reads at most `max` bytes of a file.
pub fn read_head(path: &Path, max: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    std::fs::File::open(path)?.take(max as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

pub fn format_duration(secs: f64) -> String {
    let s = secs.round() as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
