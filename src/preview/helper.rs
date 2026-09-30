//! The preview helper process: `file-flier --preview-helper <op> <path> <max> <page>`.
//!
//! Decoders that could crash or hang on a hostile file (libheif is C; PDF rendering
//! is deeply recursive) run here instead of inside the app. Output on stdout is one
//! JSON header line followed by raw RGBA pixels.

use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use super::{Rgba, row};

#[derive(Clone, Copy)]
pub enum Op {
    Heif,
    Pdf,
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::Heif => "heif",
            Op::Pdf => "pdf",
        }
    }
}

pub struct Output {
    pub img: Rgba,
    /// Page count for PDFs.
    pub count: usize,
    pub info: Vec<(String, String)>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Header {
    w: u32,
    h: u32,
    count: usize,
    info: Vec<(String, String)>,
}

/// Runs the helper and parses its output.
pub fn run(op: Op, path: &Path, max: u32, page: usize) -> Result<Output, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg("--preview-helper").arg(op.name()).arg(path).arg(max.to_string()).arg(page.to_string());
    let out = super::run_cmd(cmd, Duration::from_secs(30), None).ok_or_else(|| match op {
        Op::Heif => "This image couldn't be decoded".to_string(),
        Op::Pdf => "This PDF couldn't be rendered".to_string(),
    })?;
    let nl = out.iter().position(|&b| b == b'\n').ok_or("Bad helper output")?;
    let header: Header = serde_json::from_slice(&out[..nl]).map_err(|e| e.to_string())?;
    let px = out[nl + 1..].to_vec();
    if px.len() != header.w as usize * header.h as usize * 4 {
        return Err("Bad helper output".into());
    }
    Ok(Output { img: Rgba { w: header.w, h: header.h, px }, count: header.count, info: header.info })
}

/// Entry point in the helper process. Returns the exit code.
pub fn main(args: &[String]) -> i32 {
    let [op, path, max, page] = args else { return 2 };
    let path = Path::new(path);
    let max: u32 = max.parse().unwrap_or(1024).clamp(16, 8192);
    let page: usize = page.parse().unwrap_or(0);
    let result = match op.as_str() {
        "heif" => heif::decode(path, max),
        "pdf" => pdf(path, max, page),
        _ => Err("unknown op".into()),
    };
    match result {
        Ok(o) => {
            let header = Header { w: o.img.w, h: o.img.h, count: o.count, info: o.info };
            let mut stdout = std::io::stdout().lock();
            let ok = serde_json::to_writer(&mut stdout, &header).is_ok()
                && stdout.write_all(b"\n").is_ok()
                && stdout.write_all(&o.img.px).is_ok()
                && stdout.flush().is_ok();
            if ok { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

// ------------------------------------------------------------------ PDF

fn pdf_text(b: &[u8]) -> String {
    // PDF text strings are UTF-16BE with a BOM, or PDFDocEncoding (close to Latin-1).
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        String::from_utf16_lossy(&units)
    } else if let Ok(s) = std::str::from_utf8(b) {
        s.to_string()
    } else {
        b.iter().map(|&c| c as char).collect()
    }
    .trim()
    .to_string()
}

pub fn paper_name(w_pt: f32, h_pt: f32) -> String {
    let (a, b) = if w_pt <= h_pt { (w_pt, h_pt) } else { (h_pt, w_pt) };
    let near = |x: f32, y: f32| (a - x).abs() < 3.0 && (b - y).abs() < 3.0;
    let orient = if w_pt > h_pt { ", landscape" } else { "" };
    let named = if near(612.0, 792.0) {
        Some("US Letter")
    } else if near(595.0, 842.0) {
        Some("A4")
    } else if near(612.0, 1008.0) {
        Some("US Legal")
    } else if near(420.0, 595.0) {
        Some("A5")
    } else if near(842.0, 1191.0) {
        Some("A3")
    } else {
        None
    };
    match named {
        Some(n) => format!("{n}{orient}"),
        None => format!("{:.0} × {:.0} mm", w_pt / 72.0 * 25.4, h_pt / 72.0 * 25.4),
    }
}

fn pdf(path: &Path, max: u32, page: usize) -> Result<Output, String> {
    use hayro::hayro_syntax::{LoadPdfError, Pdf};
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let pdf = Pdf::new(data).map_err(|e| match e {
        LoadPdfError::Decryption(_) => "This PDF is password-protected".to_string(),
        _ => "This PDF couldn't be opened".to_string(),
    })?;
    let pages = pdf.pages();
    let count = pages.len();
    let p = pages.get(page.min(count.saturating_sub(1))).ok_or("This PDF has no pages")?;
    let (w, h) = p.render_dimensions();
    let scale = (max as f32 / w.max(h).max(1.0)).min(8.0);
    let settings = hayro::RenderSettings {
        x_scale: scale,
        y_scale: scale,
        bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        ..Default::default()
    };
    let pix = hayro::render(
        p,
        &hayro::RenderCache::new(),
        &hayro::hayro_interpret::InterpreterSettings::default(),
        &settings,
    );
    let (pw, ph) = (pix.width() as u32, pix.height() as u32);
    let px: Vec<u8> = pix.take_unpremultiplied().into_iter().flat_map(|c| [c.r, c.g, c.b, c.a]).collect();

    let mut info = vec![row("Page size", paper_name(w, h))];
    let m = pdf.metadata();
    for (k, v) in [("Title", &m.title), ("Author", &m.author), ("Subject", &m.subject), ("Created with", &m.creator)] {
        if let Some(v) = v.as_deref().map(pdf_text).filter(|s| !s.is_empty()) {
            info.push(row(k, v));
        }
    }
    if let Some(v) = m.producer.as_deref().map(pdf_text).filter(|s| !s.is_empty())
        && !info.iter().any(|(k, _)| k == "Created with")
    {
        info.push(row("Created with", v));
    }
    Ok(Output { img: Rgba { w: pw, h: ph, px }, count, info })
}

// ------------------------------------------------------------------ HEIF / AVIF via libheif

mod heif {
    use std::ffi::{CString, c_char, c_int, c_void};
    use std::path::Path;

    use super::{Output, Rgba, row};

    #[repr(C)]
    struct HeifError {
        code: c_int,
        subcode: c_int,
        message: *const c_char,
    }

    const COLORSPACE_RGB: c_int = 1;
    const CHROMA_INTERLEAVED_RGBA: c_int = 11;
    const CHANNEL_INTERLEAVED: c_int = 10;

    type Ctx = c_void;
    type Handle = c_void;
    type Image = c_void;

    pub fn decode(path: &Path, max: u32) -> Result<Output, String> {
        // SAFETY: we call libheif's documented C API with the signatures from heif.h,
        // check every error, and release everything we allocate. This runs in the
        // short-lived helper process, so a crash inside libheif can't affect the app.
        unsafe {
            let lib = [
                "libheif.so.1",
                "libheif.so",
                "libheif.1.dylib",
                "libheif.dylib",
                // Homebrew, which macOS doesn't search by default.
                "/opt/homebrew/lib/libheif.dylib",
                "/usr/local/lib/libheif.dylib",
            ]
            .iter()
            .find_map(|n| libloading::Library::new(*n).ok())
            .ok_or("HEIC support needs libheif, which isn't installed")?;
            macro_rules! sym {
                ($name:literal, $ty:ty) => {
                    *lib.get::<$ty>($name).map_err(|e| e.to_string())?
                };
            }
            if let Ok(init) = lib.get::<unsafe extern "C" fn(*const c_void) -> HeifError>(b"heif_init") {
                init(std::ptr::null());
            }
            let ctx_alloc = sym!(b"heif_context_alloc", unsafe extern "C" fn() -> *mut Ctx);
            let ctx_free = sym!(b"heif_context_free", unsafe extern "C" fn(*mut Ctx));
            let read_file = sym!(
                b"heif_context_read_from_file",
                unsafe extern "C" fn(*mut Ctx, *const c_char, *const c_void) -> HeifError
            );
            let primary = sym!(
                b"heif_context_get_primary_image_handle",
                unsafe extern "C" fn(*mut Ctx, *mut *mut Handle) -> HeifError
            );
            let handle_release = sym!(b"heif_image_handle_release", unsafe extern "C" fn(*const Handle));
            let handle_w = sym!(b"heif_image_handle_get_width", unsafe extern "C" fn(*const Handle) -> c_int);
            let handle_h = sym!(b"heif_image_handle_get_height", unsafe extern "C" fn(*const Handle) -> c_int);
            let thumb_count =
                sym!(b"heif_image_handle_get_number_of_thumbnails", unsafe extern "C" fn(*const Handle) -> c_int);
            let thumb_ids = sym!(
                b"heif_image_handle_get_list_of_thumbnail_IDs",
                unsafe extern "C" fn(*const Handle, *mut u32, c_int) -> c_int
            );
            let get_thumb = sym!(
                b"heif_image_handle_get_thumbnail",
                unsafe extern "C" fn(*const Handle, u32, *mut *mut Handle) -> HeifError
            );
            let decode_image = sym!(
                b"heif_decode_image",
                unsafe extern "C" fn(*const Handle, *mut *mut Image, c_int, c_int, *const c_void) -> HeifError
            );
            let image_release = sym!(b"heif_image_release", unsafe extern "C" fn(*const Image));
            let image_w = sym!(b"heif_image_get_width", unsafe extern "C" fn(*const Image, c_int) -> c_int);
            let image_h = sym!(b"heif_image_get_height", unsafe extern "C" fn(*const Image, c_int) -> c_int);
            let plane = sym!(
                b"heif_image_get_plane_readonly",
                unsafe extern "C" fn(*const Image, c_int, *mut c_int) -> *const u8
            );

            let fail = |e: HeifError| -> String {
                if e.message.is_null() {
                    format!("libheif error {}", e.code)
                } else {
                    std::ffi::CStr::from_ptr(e.message).to_string_lossy().into_owned()
                }
            };
            let c_path = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
            let ctx = ctx_alloc();
            if ctx.is_null() {
                return Err("libheif: out of memory".into());
            }
            let result = (|| {
                let e = read_file(ctx, c_path.as_ptr(), std::ptr::null());
                if e.code != 0 {
                    return Err(fail(e));
                }
                let mut handle: *mut Handle = std::ptr::null_mut();
                let e = primary(ctx, &mut handle);
                if e.code != 0 {
                    return Err(fail(e));
                }
                let (full_w, full_h) = (handle_w(handle), handle_h(handle));
                // A small preview: use the embedded thumbnail if it's big enough (much faster).
                let mut source = handle;
                if thumb_count(handle) > 0 {
                    let mut id = 0u32;
                    let mut th: *mut Handle = std::ptr::null_mut();
                    if thumb_ids(handle, &mut id, 1) == 1 && get_thumb(handle, id, &mut th).code == 0 {
                        let tw = handle_w(th).max(handle_h(th)) as u32;
                        if tw >= max.min(full_w.max(full_h) as u32) {
                            source = th;
                        } else {
                            handle_release(th);
                        }
                    }
                }
                let mut img: *mut Image = std::ptr::null_mut();
                let e = decode_image(source, &mut img, COLORSPACE_RGB, CHROMA_INTERLEAVED_RGBA, std::ptr::null());
                if source != handle {
                    handle_release(source);
                }
                handle_release(handle);
                if e.code != 0 {
                    return Err(fail(e));
                }
                let (w, h) = (image_w(img, CHANNEL_INTERLEAVED), image_h(img, CHANNEL_INTERLEAVED));
                let mut stride: c_int = 0;
                let data = plane(img, CHANNEL_INTERLEAVED, &mut stride);
                if data.is_null() || w <= 0 || h <= 0 || stride < w * 4 {
                    image_release(img);
                    return Err("libheif returned no pixels".into());
                }
                let mut px = Vec::with_capacity(w as usize * h as usize * 4);
                for y in 0..h as usize {
                    let row = std::slice::from_raw_parts(data.add(y * stride as usize), w as usize * 4);
                    px.extend_from_slice(row);
                }
                image_release(img);
                let buf = image::RgbaImage::from_raw(w as u32, h as u32, px).ok_or("bad image")?;
                let img = Rgba::from_image(image::DynamicImage::ImageRgba8(buf), max);
                Ok(Output { img, count: 1, info: vec![row("Dimensions", format!("{full_w} × {full_h}"))] })
            })();
            ctx_free(ctx);
            // Never unload libheif: its plugins may still hold threads. The process exits soon.
            std::mem::forget(lib);
            result
        }
    }
}
