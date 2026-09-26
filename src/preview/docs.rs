//! Office documents: Word/ODF/RTF/.doc text, spreadsheets as tables, slides,
//! EPUB covers, embedded thumbnails, and real page previews through LibreOffice.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use quick_xml::events::Event;

use super::{Block, Content, Cx, Kind, Loaded, Rgba, row};

const MAX_BLOCKS: usize = 3000;
const MAX_ROWS: usize = 400;
const MAX_COLS: usize = 40;
const XML_MAX: u64 = 64 << 20;

pub fn load(path: &Path, kind: Kind) -> Loaded {
    let ext = super::ext_of(path);
    let odf = ext.starts_with("od") || ext.starts_with("ot");
    let result = match kind {
        Kind::Word if odf => odf_doc(path, false),
        Kind::Sheet if odf => odf_doc(path, true),
        Kind::Slides | Kind::OdfDrawing if odf => odf_doc(path, false),
        Kind::Word => docx(path),
        Kind::Sheet => xlsx(path),
        Kind::Slides => pptx(path),
        Kind::Rtf => rtf(path),
        Kind::LegacyWord => legacy_doc(path),
        Kind::Epub => epub(path),
        Kind::IWork => iwork(path),
        _ => Ok(Loaded::new(Content::Icon(None))),
    };
    result.unwrap_or_else(|e| Loaded::new(Content::Error(e)))
}

// ------------------------------------------------------------------ zip helpers

type Zip = zip::ZipArchive<std::fs::File>;

fn open_zip(path: &Path) -> Result<Zip, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    zip::ZipArchive::new(f).map_err(|_| "This file is damaged or not a valid document".to_string())
}

fn zip_bytes(z: &mut Zip, name: &str, max: u64) -> Option<Vec<u8>> {
    let f = z.by_name(name).ok()?;
    let mut buf = Vec::new();
    f.take(max).read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn zip_text(z: &mut Zip, name: &str) -> Option<String> {
    zip_bytes(z, name, XML_MAX).map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn zip_image(z: &mut Zip, names: &[&str], max: u32) -> Option<Rgba> {
    names.iter().find_map(|n| {
        let data = zip_bytes(z, n, 32 << 20)?;
        let img = image::load_from_memory(&data).ok()?;
        Some(Rgba::from_image(img, max))
    })
}

const ODF_THUMB: &[&str] = &["Thumbnails/thumbnail.png"];
const OOXML_THUMB: &[&str] = &["docProps/thumbnail.jpeg", "docProps/thumbnail.jpg", "docProps/thumbnail.png"];
const IWORK_THUMB: &[&str] =
    &["preview.jpg", "QuickLook/Thumbnail.jpg", "QuickLook/Preview.jpg", "preview-web.jpg", "preview-micro.jpg"];

/// Picture stored inside the document itself (for grid view).
pub fn embedded_thumbnail(path: &Path, kind: Kind, max: u32) -> Option<Rgba> {
    let mut z = open_zip(path).ok()?;
    match kind {
        Kind::Word | Kind::Sheet | Kind::Slides | Kind::OdfDrawing => {
            zip_image(&mut z, ODF_THUMB, max).or_else(|| zip_image(&mut z, OOXML_THUMB, max))
        }
        Kind::IWork => zip_image(&mut z, IWORK_THUMB, max),
        Kind::Epub => epub_cover(&mut z, max),
        _ => None,
    }
}

// ------------------------------------------------------------------ XML walking

enum Ev<'a> {
    Start(&'a str, &'a [(String, String)]),
    End(&'a str),
    Text(&'a str),
}

/// Streams XML as start/end/text events with qualified names (`w:p`, `text:h`).
/// Self-closing elements produce a Start followed by an End.
fn walk(xml: &str, mut f: impl FnMut(Ev)) {
    let mut r = quick_xml::Reader::from_str(xml);
    r.config_mut().check_end_names = false;
    let mut attrs: Vec<(String, String)> = Vec::new();
    loop {
        match r.read_event() {
            Ok(ev @ (Event::Start(_) | Event::Empty(_))) => {
                let (e, empty) = match ev {
                    Event::Start(e) => (e, false),
                    Event::Empty(e) => (e, true),
                    _ => unreachable!(),
                };
                let name = e.name().as_ref().to_string();
                attrs.clear();
                for a in e.attributes().flatten() {
                    let k = a.key.as_ref().to_string();
                    let v = a
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map(|v| v.into_owned())
                        .unwrap_or_default();
                    attrs.push((k, v));
                }
                f(Ev::Start(&name, &attrs));
                if empty {
                    f(Ev::End(&name));
                }
            }
            Ok(Event::End(e)) => f(Ev::End(e.name().as_ref())),
            Ok(Event::Text(t)) => f(Ev::Text(&t.xml10_content())),
            Ok(Event::CData(t)) => f(Ev::Text(&t.xml10_content())),
            Ok(Event::GeneralRef(r)) => {
                let s = match r.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => match r.as_ref() {
                        "amp" => "&".into(),
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        _ => String::new(),
                    },
                };
                f(Ev::Text(&s));
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// Collects paragraphs, headings, bullets and table rows.
#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    /// Open paragraphs: (text, heading level, is bullet).
    paras: Vec<(String, Option<u8>, bool)>,
    table_depth: usize,
    row: Vec<String>,
    cell: Option<String>,
    /// ODF `number-columns-repeated` of the open cell.
    cell_repeat: usize,
    rows: Vec<Vec<String>>,
    list_depth: usize,
    skip_depth: usize,
    full: bool,
}

impl Builder {
    fn text(&mut self, t: &str) {
        if self.skip_depth == 0
            && let Some(p) = self.paras.last_mut()
        {
            p.0.push_str(t);
        }
    }

    fn open_para(&mut self, level: Option<u8>) {
        let bullet = self.list_depth > 0;
        self.paras.push((String::new(), level, bullet));
    }

    fn close_para(&mut self) {
        let Some((text, level, bullet)) = self.paras.pop() else { return };
        let text = text.trim_end().to_string();
        if let Some(cell) = self.cell.as_mut() {
            if !text.is_empty() {
                if !cell.is_empty() {
                    cell.push('\n');
                }
                cell.push_str(&text);
            }
            return;
        }
        if text.trim().is_empty() || self.blocks.len() >= MAX_BLOCKS {
            self.full |= self.blocks.len() >= MAX_BLOCKS;
            return;
        }
        self.blocks.push(match (level, bullet) {
            (Some(l), _) => Block::Heading(l, text),
            (None, true) => Block::Bullet(text),
            _ => Block::Para(text),
        });
    }

    fn open_row(&mut self) {
        self.row.clear();
    }

    fn open_cell(&mut self) {
        self.cell = Some(String::new());
    }

    fn close_cell(&mut self, repeat: usize) {
        if let Some(c) = self.cell.take() {
            for _ in 0..repeat.clamp(1, MAX_COLS) {
                if self.row.len() < MAX_COLS {
                    self.row.push(c.clone());
                }
            }
        }
    }

    fn close_row(&mut self, repeat: usize) {
        while self.row.last().is_some_and(|c| c.is_empty()) {
            self.row.pop();
        }
        if self.row.is_empty() {
            return;
        }
        for _ in 0..repeat.clamp(1, 5) {
            if self.rows.len() < MAX_ROWS {
                self.rows.push(self.row.clone());
            } else {
                self.full = true;
            }
            if self.table_depth <= 1 && self.blocks.len() < MAX_BLOCKS {
                self.blocks.push(Block::Row(self.row.clone()));
            }
        }
    }
}

// ------------------------------------------------------------------ OpenDocument

fn odf_doc(path: &Path, sheet: bool) -> Result<Loaded, String> {
    let mut z = open_zip(path)?;
    let content = zip_text(&mut z, "content.xml").ok_or("This document has no content")?;
    let mut b = Builder::default();
    let mut slide = 0;
    let mut sheets: Vec<String> = Vec::new();
    let mut first_table_done = false;
    let skip = ["text:tracked-changes", "office:annotation", "text:note", "svg:desc", "svg:title", "draw:frame-alt"];
    walk(&content, |ev| match ev {
        Ev::Start(n, a) => {
            if b.skip_depth > 0 || skip.contains(&n) {
                b.skip_depth += 1;
                return;
            }
            match n {
                "text:p" => b.open_para(None),
                "text:h" => b.open_para(Some(attr(a, "text:outline-level").and_then(|l| l.parse().ok()).unwrap_or(1))),
                "text:list-item" => b.list_depth += 1,
                "text:s" => b.text(&" ".repeat(attr(a, "text:c").and_then(|c| c.parse().ok()).unwrap_or(1).min(80))),
                "text:tab" => b.text("\t"),
                "text:line-break" => b.text("\n"),
                "draw:page" => {
                    slide += 1;
                    b.blocks.push(Block::Slide(slide));
                }
                "table:table" => {
                    b.table_depth += 1;
                    if let Some(name) = attr(a, "table:name") {
                        sheets.push(name.to_string());
                    }
                }
                "table:table-row" => b.open_row(),
                "table:table-cell" | "table:covered-table-cell" => {
                    b.open_cell();
                    b.cell_repeat = attr(a, "table:number-columns-repeated").and_then(|r| r.parse().ok()).unwrap_or(1);
                }
                _ => {}
            }
        }
        Ev::End(n) => {
            if b.skip_depth > 0 {
                b.skip_depth -= 1;
                return;
            }
            match n {
                "text:p" | "text:h" => b.close_para(),
                "text:list-item" => b.list_depth = b.list_depth.saturating_sub(1),
                "table:table" => {
                    b.table_depth = b.table_depth.saturating_sub(1);
                    if b.table_depth == 0 {
                        first_table_done = true;
                    }
                }
                "table:table-row" if !(sheet && first_table_done) => b.close_row(1),
                "table:table-cell" | "table:covered-table-cell" => b.close_cell(b.cell_repeat),
                _ => {}
            }
        }
        Ev::Text(t) => b.text(t),
    });

    let mut info = Vec::new();
    if let Some(meta) = zip_text(&mut z, "meta.xml") {
        info = odf_meta(&meta);
    }
    if sheet && sheets.len() > 1 {
        info.push(row("Sheets", sheets.join(", ")));
    }
    let thumb = zip_image(&mut z, ODF_THUMB, 512);
    let content = if sheet {
        Content::Table { rows: b.rows, truncated: b.full }
    } else if b.blocks.iter().all(|x| matches!(x, Block::Slide(_))) {
        match thumb {
            Some(t) => Content::Image(t),
            None => Content::Icon(Some("Empty document".into())),
        }
    } else {
        Content::Document { thumb: if slide > 0 { thumb } else { None }, blocks: b.blocks }
    };
    Ok(Loaded::new(content).with_info(info))
}

fn odf_meta(xml: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut cur = String::new();
    let mut text = String::new();
    walk(xml, |ev| match ev {
        Ev::Start(n, a) => {
            cur = n.to_string();
            text.clear();
            if n == "meta:document-statistic" {
                for (k, label) in [("meta:page-count", "Pages"), ("meta:word-count", "Words")] {
                    if let Some(v) = attr(a, k) {
                        rows.push(row(label, v));
                    }
                }
            }
        }
        Ev::Text(t) => text.push_str(t),
        Ev::End(n) if n == cur => {
            let v = text.trim();
            if !v.is_empty() {
                match n {
                    "dc:title" => rows.insert(0, row("Title", v)),
                    "meta:initial-creator" => rows.push(row("Author", v)),
                    "dc:creator" => rows.push(row("Last edited by", v)),
                    "meta:creation-date" => rows.push(row("Written", iso_date(v))),
                    "meta:generator" => rows.push(row("Created with", app_name(v))),
                    _ => {}
                }
            }
            cur.clear();
        }
        _ => {}
    });
    // Only show "Last edited by" when it differs from the author.
    let author = rows.iter().find(|(k, _)| k == "Author").map(|(_, v)| v.clone());
    rows.retain(|(k, v)| k != "Last edited by" || Some(v) != author.as_ref());
    rows
}

fn iso_date(v: &str) -> String {
    v.get(..16).unwrap_or(v).replace('T', " ")
}

// ------------------------------------------------------------------ Office Open XML

fn ooxml_meta(z: &mut Zip) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut pick = |xml: &str, fields: &[(&str, &str)]| {
        let mut cur = String::new();
        let mut text = String::new();
        walk(xml, |ev| match ev {
            Ev::Start(n, _) => {
                cur = n.to_string();
                text.clear();
            }
            Ev::Text(t) => text.push_str(t),
            Ev::End(n) if n == cur => {
                if let Some((_, label)) = fields.iter().find(|(tag, _)| *tag == n)
                    && !text.trim().is_empty()
                {
                    let v = if n.starts_with("dcterms:") { iso_date(text.trim()) } else { text.trim().to_string() };
                    rows.push(row(label, v));
                }
                cur.clear();
            }
            _ => {}
        });
    };
    if let Some(core) = zip_text(z, "docProps/core.xml") {
        pick(
            &core,
            &[
                ("dc:title", "Title"),
                ("dc:creator", "Author"),
                ("cp:lastModifiedBy", "Last edited by"),
                ("dcterms:created", "Written"),
            ],
        );
    }
    if let Some(app) = zip_text(z, "docProps/app.xml") {
        pick(&app, &[("Pages", "Pages"), ("Words", "Words"), ("Slides", "Slides"), ("Application", "Created with")]);
    }
    for (k, v) in rows.iter_mut() {
        if k == "Created with" {
            *v = app_name(v);
        }
    }
    let author = rows.iter().find(|(k, _)| k == "Author").map(|(_, v)| v.clone());
    rows.retain(|(k, v)| (k != "Last edited by" || Some(v) != author.as_ref()) && v != "0");
    rows
}

/// "LibreOffice/24.2.7.2$Linux_X86_64 LibreOffice_project/..." -> "LibreOffice 24.2.7.2"
fn app_name(v: &str) -> String {
    v.split('$').next().unwrap_or(v).replace('/', " ").trim().to_string()
}

fn docx(path: &Path) -> Result<Loaded, String> {
    let mut z = open_zip(path)?;
    let xml = zip_text(&mut z, "word/document.xml").ok_or("This document has no content")?;
    let mut b = Builder::default();
    let mut in_t = false;
    let mut in_ppr = false;
    walk(&xml, |ev| match ev {
        Ev::Start(n, a) => {
            if b.skip_depth > 0 || n == "mc:Fallback" || n == "w:tabs" {
                b.skip_depth += 1;
                return;
            }
            match n {
                "w:p" => b.open_para(None),
                "w:pPr" => in_ppr = true,
                "w:pStyle" if in_ppr => {
                    if let (Some(p), Some(style)) = (b.paras.last_mut(), attr(a, "w:val")) {
                        let s = style.to_lowercase();
                        p.1 = if s == "title" {
                            Some(1)
                        } else if s == "subtitle" {
                            Some(2)
                        } else {
                            s.strip_prefix("heading").and_then(|d| d.parse().ok())
                        };
                    }
                }
                "w:numPr" if in_ppr => {
                    if let Some(p) = b.paras.last_mut() {
                        p.2 = true;
                    }
                }
                "w:t" => in_t = true,
                "w:tab" if !in_ppr => b.text("\t"),
                "w:br" | "w:cr" => b.text("\n"),
                "w:tbl" => b.table_depth += 1,
                "w:tr" => b.open_row(),
                "w:tc" => b.open_cell(),
                _ => {}
            }
        }
        Ev::End(n) => {
            if b.skip_depth > 0 {
                b.skip_depth -= 1;
                return;
            }
            match n {
                "w:p" => b.close_para(),
                "w:pPr" => in_ppr = false,
                "w:t" => in_t = false,
                "w:tbl" => b.table_depth = b.table_depth.saturating_sub(1),
                "w:tr" => b.close_row(1),
                "w:tc" => b.close_cell(1),
                _ => {}
            }
        }
        Ev::Text(t) if in_t => b.text(t),
        _ => {}
    });
    let info = ooxml_meta(&mut z);
    Ok(Loaded::new(Content::Document { thumb: None, blocks: b.blocks }).with_info(info))
}

/// Sorts `ppt/slides/slide10.xml` after `slide9.xml`.
fn numbered(names: impl Iterator<Item = String>, prefix: &str) -> Vec<String> {
    let mut v: Vec<(u32, String)> = names
        .filter_map(|n| {
            let num = n.strip_prefix(prefix)?.strip_suffix(".xml")?.parse().ok()?;
            Some((num, n))
        })
        .collect();
    v.sort();
    v.into_iter().map(|(_, n)| n).collect()
}

fn pptx(path: &Path) -> Result<Loaded, String> {
    let mut z = open_zip(path)?;
    let slides = numbered(z.file_names().map(str::to_string).collect::<Vec<_>>().into_iter(), "ppt/slides/slide");
    let mut b = Builder::default();
    for (i, name) in slides.iter().enumerate() {
        let Some(xml) = zip_text(&mut z, name) else { continue };
        b.blocks.push(Block::Slide(i + 1));
        let mut in_t = false;
        walk(&xml, |ev| match ev {
            Ev::Start("a:p", _) => b.open_para(None),
            Ev::End("a:p") => b.close_para(),
            Ev::Start("a:t", _) => in_t = true,
            Ev::End("a:t") => in_t = false,
            Ev::Start("a:br", _) => b.text("\n"),
            Ev::Text(t) if in_t => b.text(t),
            _ => {}
        });
        if b.blocks.len() >= MAX_BLOCKS {
            break;
        }
    }
    let mut info = ooxml_meta(&mut z);
    if !info.iter().any(|(k, _)| k == "Slides") {
        info.push(row("Slides", slides.len().to_string()));
    }
    let thumb = zip_image(&mut z, OOXML_THUMB, 1024);
    Ok(Loaded::new(Content::Document { thumb, blocks: b.blocks }).with_info(info))
}

/// "AB12" -> column index 27.
fn col_index(cell_ref: &str) -> usize {
    cell_ref
        .bytes()
        .take_while(u8::is_ascii_alphabetic)
        .fold(0usize, |acc, c| acc * 26 + (c.to_ascii_uppercase() - b'A') as usize + 1)
        .saturating_sub(1)
}

fn xlsx(path: &Path) -> Result<Loaded, String> {
    let mut z = open_zip(path)?;
    // Shared strings.
    let mut shared: Vec<String> = Vec::new();
    if let Some(xml) = zip_text(&mut z, "xl/sharedStrings.xml") {
        let (mut cur, mut in_t, mut skip) = (String::new(), false, 0usize);
        walk(&xml, |ev| match ev {
            Ev::Start("rPh", _) => skip += 1,
            Ev::End("rPh") => skip = skip.saturating_sub(1),
            Ev::Start("si", _) => cur.clear(),
            Ev::End("si") => shared.push(std::mem::take(&mut cur)),
            Ev::Start("t", _) => in_t = true,
            Ev::End("t") => in_t = false,
            Ev::Text(t) if in_t && skip == 0 => cur.push_str(t),
            _ => {}
        });
    }
    // Sheet names, in workbook order.
    let mut sheets = Vec::new();
    if let Some(xml) = zip_text(&mut z, "xl/workbook.xml") {
        walk(&xml, |ev| {
            if let Ev::Start("sheet", a) = ev
                && let Some(n) = attr(a, "name")
            {
                sheets.push(n.to_string());
            }
        });
    }
    let sheet_files =
        numbered(z.file_names().map(str::to_string).collect::<Vec<_>>().into_iter(), "xl/worksheets/sheet");
    let first = sheet_files.first().ok_or("This workbook has no sheets")?.clone();
    let xml = zip_text(&mut z, &first).ok_or("This workbook has no sheets")?;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    let (mut row_cells, mut col, mut ty, mut val, mut in_v) =
        (Vec::<String>::new(), 0usize, String::new(), String::new(), false);
    walk(&xml, |ev| match ev {
        Ev::Start("row", _) => row_cells.clear(),
        Ev::End("row") => {
            while row_cells.last().is_some_and(String::is_empty) {
                row_cells.pop();
            }
            if rows.len() < MAX_ROWS {
                rows.push(row_cells.clone());
            } else {
                truncated = true;
            }
        }
        Ev::Start("c", a) => {
            col = attr(a, "r").map(col_index).unwrap_or(row_cells.len());
            ty = attr(a, "t").unwrap_or("").to_string();
            val.clear();
        }
        Ev::End("c") => {
            let text = match ty.as_str() {
                "s" => val.trim().parse::<usize>().ok().and_then(|i| shared.get(i).cloned()).unwrap_or_default(),
                "b" => if val.trim() == "1" { "TRUE" } else { "FALSE" }.to_string(),
                "str" | "inlineStr" | "e" => val.clone(),
                _ => val.trim().parse::<f64>().map(|f| format!("{f}")).unwrap_or_else(|_| val.clone()),
            };
            if col < MAX_COLS {
                if row_cells.len() <= col {
                    row_cells.resize(col + 1, String::new());
                }
                row_cells[col] = text;
            }
        }
        Ev::Start("v", _) | Ev::Start("t", _) => in_v = true,
        Ev::End("v") | Ev::End("t") => in_v = false,
        Ev::Text(t) if in_v => val.push_str(t),
        _ => {}
    });
    // Drop blank rows at the end.
    while rows.last().is_some_and(Vec::is_empty) {
        rows.pop();
    }
    let mut info = ooxml_meta(&mut z);
    if sheets.len() > 1 {
        info.push(row("Sheets", sheets.join(", ")));
    }
    Ok(Loaded::new(Content::Table { rows, truncated }).with_info(info))
}

// ------------------------------------------------------------------ RTF

fn rtf(path: &Path) -> Result<Loaded, String> {
    let data = super::read_head(path, 16 << 20).map_err(|e| e.to_string())?;
    let text = rtf_text(&data);
    let blocks = text
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.trim().is_empty())
        .take(MAX_BLOCKS)
        .map(|l| Block::Para(l.to_string()))
        .collect();
    Ok(Loaded::new(Content::Document { thumb: None, blocks }))
}

fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘',
        '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    if (0x80..0xA0).contains(&b) { HIGH[(b - 0x80) as usize] } else { b as char }
}

/// Plain text of an RTF document.
pub fn rtf_text(data: &[u8]) -> String {
    const SKIP: &[&str] = &[
        "fonttbl",
        "colortbl",
        "stylesheet",
        "info",
        "pict",
        "header",
        "footer",
        "headerl",
        "headerr",
        "headerf",
        "footerl",
        "footerr",
        "footerf",
        "object",
        "themedata",
        "colorschememapping",
        "latentstyles",
        "datastore",
        "xmlnstbl",
        "listtable",
        "listoverridetable",
        "rsidtbl",
        "generator",
        "mmathPr",
        "pgdsctbl",
        "fldinst",
        "bkmkstart",
        "bkmkend",
        "filetbl",
        "revtbl",
        "fonttable",
        "operator",
        "author",
        "title",
        "company",
        "wgrffmtfilter",
        "xmlopen",
        "listtext",
        "pntext",
        "pntxtb",
        "pntxta",
        "defchp",
        "defpap",
        "ftnsep",
        "ftnsepc",
        "aftnsep",
    ];
    let mut out = String::new();
    // Per group: (skipping, unicode fallback count)
    let mut stack: Vec<(bool, usize)> = vec![(false, 1)];
    let mut skip_chars = 0usize;
    let mut i = 0;
    let mut group_start = false;
    while i < data.len() {
        let c = data[i];
        let (skipping, uc) = *stack.last().unwrap();
        match c {
            b'{' => {
                stack.push((skipping, uc));
                group_start = true;
                i += 1;
                continue;
            }
            b'}' => {
                if stack.len() > 1 {
                    stack.pop();
                }
                i += 1;
            }
            b'\\' => {
                i += 1;
                let Some(&n) = data.get(i) else { break };
                if n.is_ascii_alphabetic() {
                    let s = i;
                    while i < data.len() && data[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word = std::str::from_utf8(&data[s..i]).unwrap_or("");
                    let ps = i;
                    if i < data.len() && (data[i] == b'-' || data[i].is_ascii_digit()) {
                        i += 1;
                        while i < data.len() && data[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                    let param: Option<i64> = std::str::from_utf8(&data[ps..i]).ok().and_then(|p| p.parse().ok());
                    if i < data.len() && data[i] == b' ' {
                        i += 1;
                    }
                    if group_start && SKIP.contains(&word) {
                        stack.last_mut().unwrap().0 = true;
                    }
                    if !skipping {
                        match word {
                            "par" | "line" | "sect" | "page" | "row" => out.push('\n'),
                            "tab" | "cell" => out.push('\t'),
                            "emdash" => out.push('—'),
                            "endash" => out.push('–'),
                            "bullet" => out.push('•'),
                            "lquote" => out.push('‘'),
                            "rquote" => out.push('’'),
                            "ldblquote" => out.push('“'),
                            "rdblquote" => out.push('”'),
                            "uc" => stack.last_mut().unwrap().1 = param.unwrap_or(1).max(0) as usize,
                            "u" => {
                                if let Some(p) = param {
                                    let cp = if p < 0 { p + 65536 } else { p } as u32;
                                    out.push(char::from_u32(cp).unwrap_or('�'));
                                    skip_chars = uc;
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    i += 1;
                    match n {
                        b'*' => stack.last_mut().unwrap().0 = true,
                        b'\'' => {
                            let hex = data.get(i..i + 2).and_then(|h| std::str::from_utf8(h).ok());
                            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                                i += 2;
                                if !skipping {
                                    if skip_chars > 0 {
                                        skip_chars -= 1;
                                    } else {
                                        out.push(cp1252(b));
                                    }
                                }
                            }
                        }
                        b'~' if !skipping => out.push('\u{a0}'),
                        b'_' if !skipping => out.push('‑'),
                        b'{' | b'}' | b'\\' if !skipping => out.push(n as char),
                        b'\n' | b'\r' if !skipping => out.push('\n'),
                        _ => {}
                    }
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                if !skipping {
                    if skip_chars > 0 {
                        skip_chars -= 1;
                    } else {
                        out.push(cp1252(c));
                    }
                }
                i += 1;
            }
        }
        group_start = false;
    }
    out
}

// ------------------------------------------------------------------ Word 97-2003 (.doc)

fn legacy_doc(path: &Path) -> Result<Loaded, String> {
    let text = doc_text(path)?;
    let blocks = text
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.trim().is_empty())
        .take(MAX_BLOCKS)
        .map(|l| Block::Para(l.to_string()))
        .collect();
    Ok(Loaded::new(Content::Document { thumb: None, blocks }))
}

/// Main text of a Word 97-2003 document, read through its piece table.
pub fn doc_text(path: &Path) -> Result<String, String> {
    let bad = || "This Word document couldn't be read".to_string();
    let mut comp = cfb::open(path).map_err(|_| bad())?;
    let mut wd = Vec::new();
    comp.open_stream("/WordDocument").map_err(|_| bad())?.take(64 << 20).read_to_end(&mut wd).map_err(|_| bad())?;
    let u16_at = |b: &[u8], o: usize| b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u32_at = |b: &[u8], o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    let n_fib = u16_at(&wd, 2).ok_or_else(bad)?;
    if n_fib < 0xC1 {
        return Err("This is an older Word format (Word 95 or earlier)".into());
    }
    let flags = u16_at(&wd, 0x0A).ok_or_else(bad)?;
    if flags & 0x0100 != 0 {
        return Err("This document is password-protected".into());
    }
    let table_name = if flags & 0x0200 != 0 { "/1Table" } else { "/0Table" };
    let ccp_text = u32_at(&wd, 0x4C).ok_or_else(bad)? as usize;
    let fc_clx = u32_at(&wd, 0x1A2).ok_or_else(bad)? as usize;
    let lcb_clx = u32_at(&wd, 0x1A6).ok_or_else(bad)? as usize;
    let mut table = Vec::new();
    comp.open_stream(table_name).map_err(|_| bad())?.take(64 << 20).read_to_end(&mut table).map_err(|_| bad())?;
    let clx = table.get(fc_clx..fc_clx + lcb_clx).ok_or_else(bad)?;
    let mut i = 0;
    while clx.get(i) == Some(&0x01) {
        i += 3 + u16_at(clx, i + 1).ok_or_else(bad)? as usize;
    }
    if clx.get(i) != Some(&0x02) {
        return Err(bad());
    }
    let lcb = u32_at(clx, i + 1).ok_or_else(bad)? as usize;
    let plc = clx.get(i + 5..i + 5 + lcb).ok_or_else(bad)?;
    let n = plc.len().saturating_sub(4) / 12;
    let mut raw = String::new();
    for k in 0..n {
        let (Some(cp0), Some(cp1)) = (u32_at(plc, k * 4), u32_at(plc, k * 4 + 4)) else { break };
        let (cp0, cp1) = (cp0 as usize, (cp1 as usize).min(ccp_text));
        if cp0 >= cp1 {
            continue;
        }
        let Some(fc) = u32_at(plc, 4 * (n + 1) + k * 8 + 2) else { break };
        let len = cp1 - cp0;
        if fc & 0x4000_0000 != 0 {
            let off = (fc & 0x3FFF_FFFF) as usize / 2;
            if let Some(bytes) = wd.get(off..off + len) {
                raw.extend(bytes.iter().map(|&b| cp1252(b)));
            }
        } else {
            let off = fc as usize;
            if let Some(bytes) = wd.get(off..off + len * 2) {
                let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
                raw.push_str(&String::from_utf16_lossy(&units));
            }
        }
        if raw.len() > 8 << 20 {
            break;
        }
    }
    // Word's special characters: paragraph marks, cell marks, fields.
    let mut out = String::with_capacity(raw.len());
    let mut field_depth = 0usize;
    let mut in_instr = false;
    for c in raw.chars() {
        match c {
            '\u{13}' => {
                field_depth += 1;
                in_instr = true;
            }
            '\u{14}' => in_instr = false,
            '\u{15}' => {
                field_depth = field_depth.saturating_sub(1);
                in_instr = false;
            }
            _ if in_instr && field_depth > 0 => {}
            '\r' | '\u{0b}' | '\u{0c}' => out.push('\n'),
            // A cell mark right after another one ends the table row.
            '\u{07}' if out.ends_with('\t') => {
                out.pop();
                out.push('\n');
            }
            '\u{07}' => out.push('\t'),
            '\u{1e}' => out.push('-'),
            '\u{01}' | '\u{08}' | '\u{1f}' | '\u{02}' | '\u{05}' => {}
            c => out.push(c),
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ EPUB / iWork

fn epub_opf(z: &mut Zip) -> Option<(String, String)> {
    let container = zip_text(z, "META-INF/container.xml")?;
    let mut opf_path = None;
    walk(&container, |ev| {
        if let Ev::Start("rootfile", a) = ev
            && opf_path.is_none()
        {
            opf_path = attr(a, "full-path").map(str::to_string);
        }
    });
    let opf_path = opf_path?;
    let opf = zip_text(z, &opf_path)?;
    Some((opf_path, opf))
}

fn epub_cover(z: &mut Zip, max: u32) -> Option<Rgba> {
    let (opf_path, opf) = epub_opf(z)?;
    let mut cover_id = None;
    let mut items: Vec<(String, String, String)> = Vec::new(); // id, href, properties
    walk(&opf, |ev| {
        if let Ev::Start(n, a) = ev {
            let local = n.rsplit(':').next().unwrap_or(n);
            if local == "meta" && attr(a, "name") == Some("cover") {
                cover_id = attr(a, "content").map(str::to_string);
            } else if local == "item" {
                items.push((
                    attr(a, "id").unwrap_or("").into(),
                    attr(a, "href").unwrap_or("").into(),
                    attr(a, "properties").unwrap_or("").into(),
                ));
            }
        }
    });
    let href = items
        .iter()
        .find(|(_, _, p)| p.split_whitespace().any(|p| p == "cover-image"))
        .or_else(|| items.iter().find(|(id, _, _)| Some(id) == cover_id.as_ref()))
        .map(|(_, h, _)| h.clone())?;
    let base = opf_path.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
    let full = format!("{base}{}", crate::app::percent_decode(&href).to_string_lossy());
    zip_image(z, &[full.as_str()], max)
}

fn epub(path: &Path) -> Result<Loaded, String> {
    let mut z = open_zip(path)?;
    let mut info = Vec::new();
    if let Some((_, opf)) = epub_opf(&mut z) {
        let mut cur = String::new();
        let mut text = String::new();
        walk(&opf, |ev| match ev {
            Ev::Start(n, _) => {
                cur = n.to_string();
                text.clear();
            }
            Ev::Text(t) => text.push_str(t),
            Ev::End(n) if n == cur => {
                let label = match n {
                    "dc:title" => Some("Title"),
                    "dc:creator" => Some("Author"),
                    "dc:publisher" => Some("Publisher"),
                    "dc:language" => Some("Language"),
                    "dc:date" => Some("Published"),
                    _ => None,
                };
                if let Some(l) = label
                    && !text.trim().is_empty()
                    && !info.iter().any(|(k, _): &(String, String)| k == l)
                {
                    let v = text.trim();
                    info.push(row(l, if l == "Published" { v.get(..10).unwrap_or(v) } else { v }));
                }
            }
            _ => {}
        });
    }
    let content = match epub_cover(&mut z, 1600) {
        Some(c) => Content::Image(c),
        None => Content::Icon(None),
    };
    let mut l = Loaded::new(content).with_info(info);
    l.kind = Some("EPUB e-book".into());
    Ok(l)
}

fn iwork(path: &Path) -> Result<Loaded, String> {
    let mut z = open_zip(path)?;
    let content = match zip_image(&mut z, IWORK_THUMB, 2048) {
        Some(img) => Content::Image(img),
        None => Content::Icon(Some("No preview stored in this file".into())),
    };
    Ok(Loaded::new(content))
}

// ------------------------------------------------------------------ LibreOffice pages

static LIBREOFFICE: OnceLock<Option<Vec<String>>> = OnceLock::new();

pub fn libreoffice_probed() -> Option<bool> {
    LIBREOFFICE.get().map(Option::is_some)
}

/// How to run LibreOffice, if it's installed (cached).
pub fn libreoffice() -> Option<&'static [String]> {
    LIBREOFFICE
        .get_or_init(|| {
            let flatpak = crate::ops::in_flatpak();
            if flatpak && !crate::ops::flatpak_can_spawn_on_host() {
                return None;
            }
            let host = |args: &[&str]| -> Option<Vec<u8>> {
                let mut cmd = if flatpak {
                    let mut c = Command::new("flatpak-spawn");
                    c.arg("--host").args(args);
                    c
                } else {
                    let mut c = Command::new(args[0]);
                    c.args(&args[1..]);
                    c
                };
                cmd.stderr(std::process::Stdio::null());
                super::run_cmd(cmd, Duration::from_secs(10), None)
            };
            let prefix: Vec<String> = if flatpak { vec!["flatpak-spawn".into(), "--host".into()] } else { vec![] };
            for bin in ["soffice", "libreoffice"] {
                let found = if flatpak {
                    host(&["sh", "-c", &format!("command -v {bin}")]).is_some()
                } else {
                    super::have(if bin == "soffice" { "soffice" } else { "libreoffice" })
                };
                if found {
                    return Some([prefix.clone(), vec![bin.to_string()]].concat());
                }
            }
            // LibreOffice installed from Flathub (common on Bazzite / Fedora Atomic).
            if (flatpak || super::have("flatpak"))
                && host(&["flatpak", "info", "org.libreoffice.LibreOffice"]).is_some()
            {
                return Some(
                    [prefix, vec!["flatpak".into(), "run".into(), "org.libreoffice.LibreOffice".into()]].concat(),
                );
            }
            None
        })
        .as_deref()
}

fn doc_key(path: &Path) -> Option<String> {
    use md5::Digest;
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let key = format!("{}\0{mtime}\0{}", path.display(), meta.len());
    Some(super::hex(&md5::Md5::digest(key.as_bytes())))
}

fn docs_cache() -> Option<PathBuf> {
    Some(super::cache_dir()?.join("documents"))
}

/// A PDF of `path` converted earlier by LibreOffice, if it's still current.
pub fn converted_pdf(path: &Path) -> Option<PathBuf> {
    let p = docs_cache()?.join(format!("{}.pdf", doc_key(path)?));
    p.is_file().then_some(p)
}

pub fn convert_with_libreoffice(path: &Path, cx: &Cx) -> Option<PathBuf> {
    if let Some(p) = converted_pdf(path) {
        return Some(p);
    }
    let lo = libreoffice()?;
    let dir = docs_cache()?;
    let key = doc_key(path)?;
    let work = dir.join(format!("tmp-{key}"));
    std::fs::create_dir_all(&work).ok()?;
    let profile = super::cache_dir()?.join("libreoffice-profile");
    let profile_uri = format!("file://{}", super::thumbs::uri_encode(&profile.to_string_lossy()));
    let mut cmd = Command::new(&lo[0]);
    cmd.args(&lo[1..])
        .arg(format!("-env:UserInstallation={profile_uri}"))
        .args(["--headless", "--norestore", "--nologo", "--nolockcheck", "--convert-to", "pdf", "--outdir"])
        .arg(&work)
        .arg(path);
    let cancel = || cx.cancelled();
    let ok = super::run_cmd(cmd, Duration::from_secs(90), Some(&cancel)).is_some();
    let produced =
        std::fs::read_dir(&work).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "pdf"));
    let target = dir.join(format!("{key}.pdf"));
    let result = match (ok, produced) {
        (true, Some(pdf)) => std::fs::rename(pdf, &target).ok().map(|_| target),
        _ => None,
    };
    let _ = std::fs::remove_dir_all(&work);
    prune_cache(&dir);
    result
}

/// Keeps the converted-document cache under ~200 MB.
fn prune_cache(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, p) in files {
        if total < 200 << 20 {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtf_plain_text() {
        let rtf =
            br"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\*\generator Foo;}\f0 Hello \b World\b0\par Caf\'e9 \u8364?5\par}";
        assert_eq!(rtf_text(rtf), "Hello World\nCafé €5\n");
    }

    #[test]
    fn spreadsheet_columns() {
        assert_eq!(col_index("A1"), 0);
        assert_eq!(col_index("Z9"), 25);
        assert_eq!(col_index("AB12"), 27);
    }

    #[test]
    fn walks_xml_with_entities() {
        let mut out = String::new();
        walk("<a:p><a:t>Fish &amp; chips &#8364;</a:t></a:p>", |ev| {
            if let Ev::Text(t) = ev {
                out.push_str(t)
            }
        });
        assert_eq!(out, "Fish & chips €");
    }
}
