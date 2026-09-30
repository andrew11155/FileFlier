//! Photos and pictures: common raster formats, HEIC/AVIF, JPEG XL, SVG and camera RAW.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use image::metadata::Orientation;

use super::{Content, Kind, Loaded, Rgba, helper, row};

/// Largest side of a full preview (zooming in shows up to this much detail).
const PREVIEW_MAX: u32 = 4096;

pub fn load(path: &Path, kind: Kind) -> Loaded {
    match decode(path, kind, PREVIEW_MAX) {
        Ok((img, (w, h), mut info)) => {
            visible_rows(&mut info);
            if w > 0 {
                info.insert(0, row("Dimensions", format!("{w} × {h}")));
            }
            let mut l = Loaded::new(Content::Image(img)).with_info(info);
            l.kind = kind_label(path, kind);
            l
        }
        Err(e) => Loaded::new(Content::Error(e)),
    }
}

pub fn thumbnail(path: &Path, kind: Kind, max: u32) -> Option<Rgba> {
    decode(path, kind, max).ok().map(|(img, _, _)| img)
}

fn kind_label(path: &Path, kind: Kind) -> Option<String> {
    let ext = super::ext_of(path);
    Some(match kind {
        Kind::Heif if ext == "avif" => "AVIF image".into(),
        Kind::Heif => "HEIC image".into(),
        Kind::Jxl => "JPEG XL image".into(),
        Kind::Svg => "SVG image".into(),
        Kind::CameraRaw => format!("{} camera RAW", ext.to_uppercase()),
        _ => return None,
    })
}

type Decoded = (Rgba, (u32, u32), Vec<(String, String)>);

fn decode(path: &Path, kind: Kind, max: u32) -> Result<Decoded, String> {
    match kind {
        Kind::Raster => {
            let (img, dims) = raster(path, max)?;
            Ok((img, dims, exif_rows(path).0))
        }
        Kind::Heif => {
            let (img, dims) = match helper::run(helper::Op::Heif, path, max, 0) {
                Ok(o) => {
                    let dims = o
                        .info
                        .iter()
                        .find(|(k, _)| k == "Dimensions")
                        .and_then(|(_, v)| v.split_once(" × "))
                        .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
                        .unwrap_or((o.img.w, o.img.h));
                    (o.img, dims)
                }
                // No libheif (e.g. some native installs): ffmpeg can often do it, and macOS can.
                Err(e) => ffmpeg_image(path, max).or_else(|| sips_image(path, max)).ok_or(e)?,
            };
            Ok((img, dims, exif_rows(path).0))
        }
        Kind::Jxl => {
            let (img, dims) =
                ffmpeg_image(path, max).or_else(|| sips_image(path, max)).ok_or("JPEG XL previews need ffmpeg")?;
            Ok((img, dims, Vec::new()))
        }
        Kind::Svg => svg(path, max).map(|(img, dims)| (img, dims, Vec::new())),
        Kind::CameraRaw => {
            let (info, orientation) = exif_rows(path);
            let (mut img, _) = embedded_jpeg(path).ok_or("No preview image inside this RAW file")?;
            if let Some(o) = orientation {
                img.apply_orientation(o);
            }
            let dims = dims_from_rows(&info).unwrap_or((0, 0));
            Ok((Rgba::from_image(img, max), dims, info))
        }
        _ => Err("Not an image".into()),
    }
}

fn dims_from_rows(rows: &[(String, String)]) -> Option<(u32, u32)> {
    let w = rows.iter().find(|(k, _)| k == "_w")?.1.parse().ok()?;
    let h = rows.iter().find(|(k, _)| k == "_h")?.1.parse().ok()?;
    Some((w, h))
}

/// Decodes with the `image` crate, honoring EXIF orientation and memory limits.
fn raster(path: &Path, max: u32) -> Result<(Rgba, (u32, u32)), String> {
    use image::ImageDecoder;
    let mut reader =
        image::ImageReader::open(path).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(1 << 30);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let (w, h) = decoder.dimensions();
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    let dims = match orientation {
        Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH => {
            (h, w)
        }
        _ => (w, h),
    };
    Ok((Rgba::from_image(img, max), dims))
}

/// First frame through ffmpeg (JPEG XL, AVIF/HEIC fallback).
fn ffmpeg_image(path: &Path, max: u32) -> Option<(Rgba, (u32, u32))> {
    if !super::have("ffmpeg") {
        return None;
    }
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-i"]).arg(path).args([
        "-frames:v",
        "1",
        "-f",
        "image2pipe",
        "-c:v",
        "png",
        "-",
    ]);
    let png = super::run_cmd(cmd, Duration::from_secs(20), None)?;
    let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok()?;
    let dims = (img.width(), img.height());
    Some((Rgba::from_image(img, max), dims))
}

/// macOS's built-in image converter (HEIC, AVIF, JPEG XL and more), as a fallback.
fn sips_image(path: &Path, max: u32) -> Option<(Rgba, (u32, u32))> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let out = std::env::temp_dir().join(format!("file-flier-sips-{}-{}.png", std::process::id(), rand_suffix()));
    let mut cmd = Command::new("sips");
    cmd.args(["-s", "format", "png", "-Z", &max.to_string()]).arg(path).arg("--out").arg(&out);
    super::run_cmd(cmd, Duration::from_secs(20), None);
    let img = image::open(&out).ok();
    let _ = std::fs::remove_file(&out);
    let img = img?;
    let dims = (img.width(), img.height());
    Some((Rgba::from_image(img, max), dims))
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

pub fn fontdb() -> Arc<resvg::usvg::fontdb::Database> {
    static DB: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

fn svg(path: &Path, max: u32) -> Result<(Rgba, (u32, u32)), String> {
    use resvg::{tiny_skia, usvg};
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let opt =
        usvg::Options { resources_dir: path.parent().map(Path::to_path_buf), fontdb: fontdb(), ..Default::default() };
    let tree = usvg::Tree::from_data(&data, &opt).map_err(|e| format!("This SVG couldn't be read ({e})"))?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    // Render crisply: fit the longest side to `max`, even for tiny icons.
    let scale = max as f32 / w.max(h).max(1.0);
    let (pw, ph) = (((w * scale).ceil() as u32).max(1), ((h * scale).ceil() as u32).max(1));
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or("SVG too large")?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let px = pixmap.pixels().iter().flat_map(|c| {
        let c = c.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    });
    Ok((Rgba { w: pw, h: ph, px: px.collect() }, (w.round() as u32, h.round() as u32)))
}

/// A small icon (PNG, SVG, XPM-less) scaled to fit `size`.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn icon(path: &Path, size: u32) -> Option<Rgba> {
    if super::ext_of(path) == "svg" {
        return svg(path, size).ok().map(|(i, _)| i);
    }
    raster(path, size).ok().map(|(i, _)| i)
}

/// Camera RAW files carry a full-size JPEG preview; find the largest one.
pub fn embedded_jpeg(path: &Path) -> Option<(image::DynamicImage, (u32, u32))> {
    let data = super::read_head(path, 200 << 20).ok()?;
    let mut best: Option<(usize, u64)> = None;
    let mut i = 0;
    let mut tried = 0;
    while let Some(off) = data[i..].windows(3).position(|w| w == [0xFF, 0xD8, 0xFF]) {
        let start = i + off;
        i = start + 3;
        tried += 1;
        if tried > 64 {
            break;
        }
        let reader = image::ImageReader::with_format(std::io::Cursor::new(&data[start..]), image::ImageFormat::Jpeg);
        if let Ok((w, h)) = reader.into_dimensions() {
            let area = w as u64 * h as u64;
            if area > best.map_or(0, |b| b.1) {
                best = Some((start, area));
            }
        }
    }
    let (start, _) = best?;
    let img = image::load_from_memory_with_format(&data[start..], image::ImageFormat::Jpeg).ok()?;
    let dims = (img.width(), img.height());
    Some((img, dims))
}

// ------------------------------------------------------------------ EXIF

/// Human-readable photo details, plus the EXIF orientation.
/// Also returns hidden `_w`/`_h` rows with the pixel size when known.
pub fn exif_rows(path: &Path) -> (Vec<(String, String)>, Option<Orientation>) {
    use exif::{In, Tag, Value};
    let mut rows = Vec::new();
    let Ok(file) = std::fs::File::open(path) else { return (rows, None) };
    let Ok(ex) = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)) else {
        return (rows, None);
    };
    let text = |tag: Tag| -> Option<String> {
        let f = ex.get_field(tag, In::PRIMARY)?;
        let s = match &f.value {
            Value::Ascii(v) => v.first().map(|b| String::from_utf8_lossy(b).trim().to_string())?,
            _ => f.display_value().with_unit(&ex).to_string(),
        };
        (!s.is_empty()).then_some(s)
    };
    let uint = |tag: Tag| ex.get_field(tag, In::PRIMARY).and_then(|f| f.value.get_uint(0));

    let make = text(Tag::Make).unwrap_or_default();
    let model = text(Tag::Model).unwrap_or_default();
    let camera = if model.to_lowercase().starts_with(&make.to_lowercase()) || make.is_empty() {
        model
    } else {
        format!("{make} {model}")
    };
    if !camera.is_empty() {
        rows.push(row("Camera", camera));
    }
    if let Some(v) = text(Tag::LensModel) {
        rows.push(row("Lens", v));
    }
    let exposure: Vec<String> = [Tag::FocalLength, Tag::FNumber, Tag::ExposureTime]
        .into_iter()
        .filter_map(text)
        .chain(uint(Tag::PhotographicSensitivity).map(|iso| format!("ISO {iso}")))
        .collect();
    if !exposure.is_empty() {
        rows.push(row("Exposure", exposure.join("  ·  ")));
    }
    if let Some(v) = text(Tag::DateTimeOriginal) {
        // "2024:05:01 12:30:00" -> "2024-05-01 12:30"
        let v = match v.split_once(' ') {
            Some((d, t)) => format!("{} {}", d.replace(':', "-"), t.get(..5).unwrap_or(t)),
            None => v,
        };
        rows.push(row("Taken", v));
    }
    if let Some(loc) = gps(&ex) {
        rows.push(row("Location", loc));
    }
    if let Some(v) = text(Tag::Software) {
        rows.push(row("Software", v));
    }
    if let (Some(w), Some(h)) = (uint(Tag::PixelXDimension), uint(Tag::PixelYDimension)) {
        rows.push(row("_w", w.to_string()));
        rows.push(row("_h", h.to_string()));
    }
    let orientation = uint(Tag::Orientation).and_then(|o| Orientation::from_exif(o as u8));
    (rows, orientation)
}

fn gps(ex: &exif::Exif) -> Option<String> {
    use exif::{In, Tag, Value};
    let coord = |tag: Tag, ref_tag: Tag| -> Option<(f64, String)> {
        let Value::Rational(v) = &ex.get_field(tag, In::PRIMARY)?.value else { return None };
        if v.len() < 3 {
            return None;
        }
        let deg = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
        let r = match &ex.get_field(ref_tag, In::PRIMARY)?.value {
            Value::Ascii(a) => String::from_utf8_lossy(a.first()?).to_string(),
            _ => return None,
        };
        Some((deg, r))
    };
    let (lat, lat_ref) = coord(Tag::GPSLatitude, Tag::GPSLatitudeRef)?;
    let (lon, lon_ref) = coord(Tag::GPSLongitude, Tag::GPSLongitudeRef)?;
    Some(format!("{lat:.4}° {lat_ref}, {lon:.4}° {lon_ref}"))
}

/// Strips helper rows (`_w`, `_h`) before display.
pub fn visible_rows(rows: &mut Vec<(String, String)>) {
    rows.retain(|(k, _)| !k.starts_with('_'));
}
