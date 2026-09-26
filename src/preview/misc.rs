//! Folders, archives, fonts, CSV and plain text.

use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, Instant};

use super::{CALCULATING, Content, Cx, Kind, ListItem, Loaded, Options, Rgba, row};

const MAX_LIST: usize = 5000;

// ------------------------------------------------------------------ folders

pub fn load_dir(path: &Path, opts: &Options, cx: &Cx) {
    let mut entries = match crate::fs_model::read_dir(path) {
        Ok(e) => e,
        Err(e) => return (cx.emit)(Loaded::new(Content::Error(e.to_string()))),
    };
    let hidden = entries.iter().filter(|e| e.is_hidden()).count();
    entries.retain(|x| !x.is_hidden());
    crate::fs_model::sort_entries(&mut entries, Default::default());
    let count = entries.len();
    entries.truncate(1000);
    let mut info = vec![row(
        "Contains",
        match hidden {
            0 => format!("{count} item{}", crate::app::plural(count)),
            h => format!("{count} item{} (+{h} hidden)", crate::app::plural(count)),
        },
    )];
    if !opts.remote {
        info.push(row("Size", CALCULATING));
    }
    let loaded = Loaded::new(Content::Dir(entries)).with_info(info.clone());
    (cx.emit)(loaded.clone());
    if opts.remote {
        return;
    }
    // Total size, walking in the background; stays on this filesystem and skips symlinks.
    let dev = std::fs::metadata(path).map(|m| m.dev()).ok();
    let start = Instant::now();
    let (mut bytes, mut files, mut complete) = (0u64, 0u64, true);
    let mut stack = vec![path.to_path_buf()];
    'walk: while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for item in rd.flatten() {
            if files % 256 == 0 && (cx.cancelled() || start.elapsed() > Duration::from_secs(20)) {
                complete = false;
                break 'walk;
            }
            let Ok(m) = item.metadata() else { continue };
            if m.is_dir() {
                if Some(m.dev()) == dev {
                    stack.push(item.path());
                }
            } else {
                bytes += m.blocks() * 512;
                files += 1;
            }
        }
    }
    if cx.cancelled() {
        return;
    }
    let size = crate::app::human_size(bytes);
    info[1].1 = if complete { size } else { format!("more than {size}") };
    (cx.emit)(Loaded { info, ..loaded });
}

// ------------------------------------------------------------------ archives

pub fn load_archive(path: &Path, kind: Kind) -> Loaded {
    let result = match kind {
        Kind::Zip => zip_listing(path),
        _ => tar_listing(path, kind == Kind::TarGz),
    };
    match result {
        Ok((mut items, info)) => {
            items.sort_by(|a, b| {
                b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            // Comic book archives show their cover.
            if super::ext_of(path) == "cbz"
                && let Some(cover) = comic_cover(path, 1600)
            {
                return Loaded::new(Content::Image(cover)).with_info(info);
            }
            Loaded::new(Content::Listing(items)).with_info(info)
        }
        Err(e) => Loaded::new(Content::Error(e)),
    }
}

type Listing = (Vec<ListItem>, Vec<(String, String)>);

fn summary(items: &[ListItem], total: u64, count: usize, packed: u64) -> Vec<(String, String)> {
    let files = items.iter().filter(|i| !i.is_dir).count().max(count.saturating_sub(items.len()));
    let mut info = vec![row("Contains", format!("{files} file{}", crate::app::plural(files)))];
    if total > 0 {
        info.push(row("Uncompressed", crate::app::human_size(total)));
        if packed > 0 && packed < total {
            info.push(row("Compression", format!("{:.0}% smaller", 100.0 - packed as f64 / total as f64 * 100.0)));
        }
    }
    info
}

fn zip_listing(path: &Path) -> Result<Listing, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|_| "This archive is damaged or unsupported".to_string())?;
    let mut items = Vec::new();
    let (mut total, mut packed) = (0u64, 0u64);
    let count = z.len();
    for i in 0..count {
        let Ok(e) = z.by_index_raw(i) else { continue };
        total += e.size();
        packed += e.compressed_size();
        if items.len() < MAX_LIST {
            items.push(ListItem {
                name: e.name().to_string(),
                size: (!e.is_dir()).then(|| e.size()),
                is_dir: e.is_dir(),
            });
        }
    }
    let info = summary(&items, total, count, packed);
    Ok((items, info))
}

fn tar_listing(path: &Path, gz: bool) -> Result<Listing, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let reader: Box<dyn Read> = if gz { Box::new(flate2::read::GzDecoder::new(f)) } else { Box::new(f) };
    let mut ar = tar::Archive::new(reader);
    let mut items = Vec::new();
    let mut total = 0u64;
    let start = Instant::now();
    for e in ar.entries().map_err(|_| "This archive is damaged".to_string())? {
        let Ok(e) = e else { break };
        let is_dir = e.header().entry_type().is_dir();
        let size = e.header().size().unwrap_or(0);
        total += size;
        let name = e.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        items.push(ListItem { name, size: (!is_dir).then_some(size), is_dir });
        if items.len() >= MAX_LIST || start.elapsed() > Duration::from_secs(10) {
            break;
        }
    }
    let info = summary(&items, total, items.len(), std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));
    Ok((items, info))
}

/// First image in a zip (comic books).
pub fn comic_cover(path: &Path, max: u32) -> Option<Rgba> {
    if super::ext_of(path) != "cbz" {
        return None;
    }
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let mut names: Vec<String> = z
        .file_names()
        .filter(|n| {
            let l = n.to_lowercase();
            [".jpg", ".jpeg", ".png", ".webp", ".gif"].iter().any(|e| l.ends_with(e))
        })
        .map(str::to_string)
        .collect();
    names.sort();
    let mut buf = Vec::new();
    z.by_name(names.first()?).ok()?.take(64 << 20).read_to_end(&mut buf).ok()?;
    Some(Rgba::from_image(image::load_from_memory(&buf).ok()?, max))
}

// ------------------------------------------------------------------ fonts

pub fn load_font(path: &Path) -> Loaded {
    let data = match super::read_head(path, 64 << 20) {
        Ok(d) => d,
        Err(e) => return Loaded::new(Content::Error(e.to_string())),
    };
    let Ok(face) = ttf_parser::Face::parse(&data, 0) else {
        return Loaded::new(Content::Error("This font couldn't be read".into()));
    };
    let name =
        |id: u16| face.names().into_iter().filter(|n| n.name_id == id && n.is_unicode()).find_map(|n| n.to_string());
    let family = name(ttf_parser::name_id::TYPOGRAPHIC_FAMILY).or_else(|| name(ttf_parser::name_id::FAMILY));
    let style = name(ttf_parser::name_id::TYPOGRAPHIC_SUBFAMILY).or_else(|| name(ttf_parser::name_id::SUBFAMILY));
    let mut info = Vec::new();
    if let Some(f) = &family {
        info.push(row("Family", f.clone()));
    }
    if let Some(s) = &style {
        info.push(row("Style", s.clone()));
    }
    info.push(row("Glyphs", face.number_of_glyphs().to_string()));
    if let Some(v) = name(ttf_parser::name_id::VERSION) {
        info.push(row("Version", v));
    }
    let title = [family.clone().unwrap_or_default(), style.unwrap_or_default()].join(" ").trim().to_string();
    let content = match font_sample(&data, &title) {
        Some(img) => Content::Image(img),
        None => Content::Icon(None),
    };
    let mut l = Loaded::new(content).with_info(info);
    l.kind = Some("Font".into());
    l
}

/// Renders sample lines in the font: dark text on a white card.
fn font_sample(data: &[u8], title: &str) -> Option<Rgba> {
    use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
    let font = FontRef::try_from_slice_and_index(data, 0).ok()?;
    let lines: [(&str, f32); 5] = [
        (if title.is_empty() { "Aa Bb Cc" } else { title }, 64.0),
        ("ABCDEFGHIJKLMNOPQRSTUVWXYZ", 40.0),
        ("abcdefghijklmnopqrstuvwxyz", 40.0),
        ("0123456789 !?&@#%()[]", 40.0),
        ("The quick brown fox jumps over the lazy dog.", 30.0),
    ];
    let (w, pad) = (1400u32, 48.0f32);
    let height: f32 = lines.iter().map(|(_, s)| s * 1.45).sum::<f32>() + pad * 2.0;
    let h = height.ceil() as u32;
    let mut px = vec![255u8; (w * h * 4) as usize];
    let mut y = pad;
    for (text, size) in lines {
        let sf = font.as_scaled(PxScale::from(size));
        y += sf.ascent();
        let mut x = pad;
        let mut prev = None;
        for ch in text.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            let g = id.with_scale_and_position(PxScale::from(size), point(x, y));
            x += sf.h_advance(id);
            prev = Some(id);
            if x > w as f32 - pad {
                break;
            }
            if let Some(o) = font.outline_glyph(g) {
                let b = o.px_bounds();
                o.draw(|gx, gy, cov| {
                    let (px_x, px_y) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    if px_x >= 0 && px_y >= 0 && (px_x as u32) < w && (px_y as u32) < h {
                        let i = ((px_y as u32 * w + px_x as u32) * 4) as usize;
                        let v = (255.0 * (1.0 - cov.clamp(0.0, 1.0)) + 20.0 * cov.clamp(0.0, 1.0)) as u8;
                        let v = v.min(px[i]);
                        px[i..i + 3].copy_from_slice(&[v, v, v]);
                    }
                });
            }
        }
        y += size * 1.45 - sf.ascent();
    }
    Some(Rgba { w, h, px })
}

// ------------------------------------------------------------------ CSV and text

pub fn load_csv(path: &Path) -> Loaded {
    let data = match super::read_head(path, 4 << 20) {
        Ok(d) => d,
        Err(e) => return Loaded::new(Content::Error(e.to_string())),
    };
    let first_line = data.split(|&b| b == b'\n').next().unwrap_or(&[]);
    let delim = if super::ext_of(path) == "tsv" {
        b'\t'
    } else {
        *b",;\t|".iter().max_by_key(|d| first_line.iter().filter(|b| b == d).count()).unwrap()
    };
    let mut r = csv::ReaderBuilder::new().has_headers(false).flexible(true).delimiter(delim).from_reader(&data[..]);
    let mut rows = Vec::new();
    let mut truncated = false;
    for rec in r.records() {
        let Ok(rec) = rec else { break };
        if rows.len() >= 400 {
            truncated = true;
            break;
        }
        rows.push(rec.iter().take(40).map(str::to_string).collect::<Vec<_>>());
    }
    let lines = data.iter().filter(|&&b| b == b'\n').count();
    let mut l = Loaded::new(Content::Table { rows, truncated }).with_info(vec![row("Rows", lines.to_string())]);
    l.kind = Some(if delim == b'\t' { "Tab-separated values".into() } else { "CSV spreadsheet".into() });
    l
}

const TEXT_BYTES: usize = 512 * 1024;
const TEXT_LINES: usize = 5000;

pub fn load_text_or_binary(path: &Path) -> Loaded {
    let buf = match super::read_head(path, TEXT_BYTES) {
        Ok(b) => b,
        Err(e) => return Loaded::new(Content::Error(e.to_string())),
    };
    let Some((text, encoding)) = decode_text(&buf) else {
        return Loaded::new(Content::Icon(None));
    };
    let full_file = std::fs::metadata(path).map(|m| m.len() as usize <= buf.len()).unwrap_or(true);
    let total_lines = text.lines().count();
    let truncated = !full_file || total_lines > TEXT_LINES;
    let text: String =
        if total_lines > TEXT_LINES { text.lines().take(TEXT_LINES).collect::<Vec<_>>().join("\n") } else { text };
    let lines = if full_file { total_lines.to_string() } else { format!("{}+", total_lines) };
    let info = vec![row("Lines", lines), row("Encoding", encoding)];
    let syntax = syntax_for(path);
    Loaded::new(Content::Text { text, syntax, truncated }).with_info(info)
}

/// UTF-8 / UTF-16 text, or None for binary data.
fn decode_text(buf: &[u8]) -> Option<(String, &'static str)> {
    if let Some(rest) = buf.strip_prefix(&[0xFF, 0xFE]) {
        let u: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        return Some((String::from_utf16_lossy(&u), "UTF-16"));
    }
    if let Some(rest) = buf.strip_prefix(&[0xFE, 0xFF]) {
        let u: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        return Some((String::from_utf16_lossy(&u), "UTF-16"));
    }
    let buf = buf.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(buf);
    if buf.contains(&0) {
        return None;
    }
    match std::str::from_utf8(buf) {
        Ok(t) => Some((t.to_string(), if t.is_ascii() { "ASCII" } else { "UTF-8" })),
        // Tolerate a multi-byte character cut off at the end of the buffer.
        Err(e) if e.error_len().is_none() => {
            Some((String::from_utf8_lossy(&buf[..e.valid_up_to()]).into_owned(), "UTF-8"))
        }
        Err(_) => {
            // Mostly-printable single-byte text (Latin-1 / Windows-1252).
            let printable = buf.iter().filter(|&&b| b >= 0x20 || b"\t\r\n".contains(&b)).count();
            (printable * 100 >= buf.len() * 97).then(|| (buf.iter().map(|&b| b as char).collect(), "Latin-1"))
        }
    }
}

/// Language name for syntax highlighting.
pub fn syntax_for(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    match name.as_str() {
        "makefile" | "gnumakefile" => return "makefile".into(),
        "dockerfile" | "containerfile" => return "dockerfile".into(),
        "cmakelists.txt" => return "cmake".into(),
        ".bashrc" | ".bash_profile" | ".profile" | ".zshrc" => return "sh".into(),
        _ => {}
    }
    match super::ext_of(path).as_str() {
        "" | "txt" | "log" => String::new(),
        "yml" => "yaml".into(),
        "h" | "hpp" | "hh" | "cc" | "cxx" => "cpp".into(),
        "desktop" | "service" | "conf" | "cfg" | "ini" | "toml" => "ini".into(),
        "svelte" | "vue" => "html".into(),
        e => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_text_and_binary() {
        assert_eq!(decode_text(b"hello\n").map(|t| t.1), Some("ASCII"));
        assert_eq!(decode_text("héllo".as_bytes()).map(|t| t.1), Some("UTF-8"));
        assert!(decode_text(b"\x7fELF\x02\x01\x01\0\0\0").is_none());
        assert_eq!(decode_text(&[0xFF, 0xFE, b'h', 0, b'i', 0]).map(|t| t.0), Some("hi".into()));
    }
}
