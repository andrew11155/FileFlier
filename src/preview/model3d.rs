//! 3D models (STL, OBJ, PLY, 3MF) and 3D-printer G-code.
//!
//! Meshes are parsed here and drawn by a small software rasterizer (z-buffer,
//! flat shading, 2x supersampling), so previews need no GPU support and look the
//! same everywhere. The view can be turned by dragging; see `ui::preview_view`.
//! G-code files show the thumbnail the slicer embedded plus print details.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use super::{Content, Loaded, Rgba, row};

/// Files larger than this aren't parsed (a 400 MB STL is ~8M triangles).
const MAX_FILE: u64 = 400 << 20;
/// Triangle budget; bigger meshes are thinned out evenly for display.
const MAX_TRIS: usize = 6_000_000;

pub type V3 = [f32; 3];

pub struct Mesh {
    pub tris: Vec<[V3; 3]>,
    /// Axis-aligned bounds, in the file's units (usually millimetres).
    pub min: V3,
    pub max: V3,
    pub format: &'static str,
    /// Original triangle count (before any thinning).
    pub count: usize,
    pub objects: usize,
}

impl Mesh {
    fn new(mut tris: Vec<[V3; 3]>, format: &'static str, objects: usize) -> Result<Self, String> {
        tris.retain(|t| t.iter().all(|v| v.iter().all(|c| c.is_finite())));
        if tris.is_empty() {
            return Err("This model has no triangles".into());
        }
        let count = tris.len();
        if tris.len() > MAX_TRIS {
            let step = tris.len().div_ceil(MAX_TRIS);
            tris = tris.into_iter().step_by(step).collect();
        }
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for t in &tris {
            for v in t {
                for i in 0..3 {
                    min[i] = min[i].min(v[i]);
                    max[i] = max[i].max(v[i]);
                }
            }
        }
        Ok(Mesh { tris, min, max, format, count, objects })
    }

    /// Enclosed volume (divergence theorem); meaningful for closed meshes.
    pub fn volume(&self) -> f64 {
        let v: f64 = self
            .tris
            .iter()
            .map(|[a, b, c]| {
                let (a, b, c) = (a.map(f64::from), b.map(f64::from), c.map(f64::from));
                a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0])
            })
            .sum::<f64>()
            / 6.0;
        v.abs() * (self.count as f64 / self.tris.len() as f64)
    }
}

pub fn parse(path: &Path) -> Result<Mesh, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_FILE {
        return Err("This model is too large to preview".into());
    }
    let ext = super::ext_of(path);
    if ext == "3mf" {
        return parse_3mf(path);
    }
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    match ext.as_str() {
        "stl" => parse_stl(&data),
        "obj" => parse_obj(&String::from_utf8_lossy(&data)),
        "ply" => parse_ply(&data),
        _ => Err("Unknown model format".into()),
    }
}

// ------------------------------------------------------------------ STL

pub fn parse_stl(data: &[u8]) -> Result<Mesh, String> {
    let binary_len = |n: usize| 84 + n * 50;
    let n = data.get(80..84).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
    // Some binary STLs start with "solid" too; the exact size decides.
    let is_binary = n.is_some_and(|n| binary_len(n) == data.len()) || !data.trim_ascii_start().starts_with(b"solid");
    if is_binary {
        let n = n.ok_or("This STL file is truncated")?;
        let n = n.min((data.len().saturating_sub(84)) / 50);
        let f = |b: &[u8], i: usize| f32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let tris = (0..n)
            .map(|k| {
                let r = &data[84 + k * 50..84 + k * 50 + 50];
                let v = |o: usize| [f(r, o), f(r, o + 4), f(r, o + 8)];
                [v(12), v(24), v(36)]
            })
            .collect();
        return Mesh::new(tris, "STL (binary)", 1);
    }
    let text = String::from_utf8_lossy(data);
    let mut verts: Vec<V3> = Vec::new();
    let mut tris = Vec::new();
    let mut solids = 0;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("vertex") => {
                let mut c = it.filter_map(|s| s.parse::<f32>().ok());
                if let (Some(x), Some(y), Some(z)) = (c.next(), c.next(), c.next()) {
                    verts.push([x, y, z]);
                }
            }
            Some("endloop") => {
                if verts.len() >= 3 {
                    for i in 1..verts.len() - 1 {
                        tris.push([verts[0], verts[i], verts[i + 1]]);
                    }
                }
                verts.clear();
            }
            Some("solid") => solids += 1,
            _ => {}
        }
    }
    Mesh::new(tris, "STL (text)", solids.max(1))
}

// ------------------------------------------------------------------ OBJ

pub fn parse_obj(text: &str) -> Result<Mesh, String> {
    let mut verts: Vec<V3> = Vec::new();
    let mut tris = Vec::new();
    let mut objects = 0;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let mut c = it.filter_map(|s| s.parse::<f32>().ok());
                verts.push([c.next().unwrap_or(0.0), c.next().unwrap_or(0.0), c.next().unwrap_or(0.0)]);
            }
            Some("f") => {
                let idx: Vec<usize> = it
                    .filter_map(|tok| {
                        let i: i64 = tok.split('/').next()?.parse().ok()?;
                        let n = verts.len() as i64;
                        let i = if i < 0 { n + i } else { i - 1 };
                        (0..n).contains(&i).then_some(i as usize)
                    })
                    .collect();
                for k in 1..idx.len().saturating_sub(1) {
                    tris.push([verts[idx[0]], verts[idx[k]], verts[idx[k + 1]]]);
                }
            }
            Some("o") | Some("g") => objects += 1,
            _ => {}
        }
    }
    // OBJ is conventionally Y-up; turn it Z-up like printer formats.
    for t in &mut tris {
        for v in t.iter_mut() {
            *v = [v[0], -v[2], v[1]];
        }
    }
    Mesh::new(tris, "Wavefront OBJ", objects.max(1))
}

// ------------------------------------------------------------------ PLY

pub fn parse_ply(data: &[u8]) -> Result<Mesh, String> {
    let end = data.windows(10).position(|w| w == b"end_header").ok_or("Not a PLY file")?;
    let header = String::from_utf8_lossy(&data[..end]);
    let mut body = end + 10;
    while body < data.len() && (data[body] == b'\r' || data[body] == b'\n') {
        body += 1;
        if data[body - 1] == b'\n' {
            break;
        }
    }
    #[derive(PartialEq, Clone, Copy)]
    enum Fmt {
        Ascii,
        Le,
        Be,
    }
    struct Elem {
        name: String,
        count: usize,
        // (name, type, list count type)
        props: Vec<(String, String, Option<String>)>,
    }
    let mut fmt = Fmt::Ascii;
    let mut elems: Vec<Elem> = Vec::new();
    for line in header.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            ["format", f, ..] => {
                fmt = match *f {
                    "binary_little_endian" => Fmt::Le,
                    "binary_big_endian" => Fmt::Be,
                    _ => Fmt::Ascii,
                }
            }
            ["element", name, n] => {
                elems.push(Elem { name: name.to_string(), count: n.parse().unwrap_or(0), props: Vec::new() })
            }
            ["property", "list", ct, t, name] => {
                if let Some(e) = elems.last_mut() {
                    e.props.push((name.to_string(), t.to_string(), Some(ct.to_string())));
                }
            }
            ["property", t, name] => {
                if let Some(e) = elems.last_mut() {
                    e.props.push((name.to_string(), t.to_string(), None));
                }
            }
            _ => {}
        }
    }
    fn size(t: &str) -> usize {
        match t {
            "char" | "uchar" | "int8" | "uint8" => 1,
            "short" | "ushort" | "int16" | "uint16" => 2,
            "double" | "float64" => 8,
            _ => 4,
        }
    }
    let mut pos = body;
    let mut ascii = String::from_utf8_lossy(&data[body.min(data.len())..]).into_owned();
    let mut words = ascii.split_ascii_whitespace();
    let mut read = |t: &str| -> Option<f64> {
        if fmt == Fmt::Ascii {
            return words.next()?.parse().ok();
        }
        let n = size(t);
        let b = data.get(pos..pos + n)?;
        pos += n;
        let mut a = [0u8; 8];
        a[..n].copy_from_slice(b);
        if fmt == Fmt::Be {
            a[..n].reverse();
        }
        Some(match t {
            "char" | "int8" => a[0] as i8 as f64,
            "uchar" | "uint8" => a[0] as f64,
            "short" | "int16" => i16::from_le_bytes([a[0], a[1]]) as f64,
            "ushort" | "uint16" => u16::from_le_bytes([a[0], a[1]]) as f64,
            "int" | "int32" => i32::from_le_bytes(a[..4].try_into().ok()?) as f64,
            "uint" | "uint32" => u32::from_le_bytes(a[..4].try_into().ok()?) as f64,
            "double" | "float64" => f64::from_le_bytes(a),
            _ => f32::from_le_bytes(a[..4].try_into().ok()?) as f64,
        })
    };
    let mut verts: Vec<V3> = Vec::new();
    let mut tris = Vec::new();
    'elems: for e in &elems {
        for _ in 0..e.count {
            let mut p = [0f32; 3];
            let mut face: Vec<usize> = Vec::new();
            for (name, t, list) in &e.props {
                match list {
                    Some(ct) => {
                        let Some(n) = read(ct) else { break 'elems };
                        for _ in 0..n as usize {
                            let Some(v) = read(t) else { break 'elems };
                            face.push(v as usize);
                        }
                    }
                    None => {
                        let Some(v) = read(t) else { break 'elems };
                        match name.as_str() {
                            "x" => p[0] = v as f32,
                            "y" => p[1] = v as f32,
                            "z" => p[2] = v as f32,
                            _ => {}
                        }
                    }
                }
            }
            if e.name == "vertex" {
                verts.push(p);
            } else if e.name == "face" && face.len() >= 3 && face.iter().all(|&i| i < verts.len()) {
                for k in 1..face.len() - 1 {
                    tris.push([verts[face[0]], verts[face[k]], verts[face[k + 1]]]);
                }
            }
        }
    }
    ascii.clear();
    Mesh::new(tris, "PLY", 1)
}

// ------------------------------------------------------------------ 3MF

/// Row-major 3x4 affine transform as 3MF writes it (`m00 m01 m02 m10 ... m32`).
type Xf = [f32; 12];
const IDENTITY: Xf = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

fn parse_xf(s: &str) -> Xf {
    let v: Vec<f32> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    v.try_into().unwrap_or(IDENTITY)
}

fn apply(m: &Xf, p: V3) -> V3 {
    [
        p[0] * m[0] + p[1] * m[3] + p[2] * m[6] + m[9],
        p[0] * m[1] + p[1] * m[4] + p[2] * m[7] + m[10],
        p[0] * m[2] + p[1] * m[5] + p[2] * m[8] + m[11],
    ]
}

/// `a` then `b`.
fn compose(a: &Xf, b: &Xf) -> Xf {
    let mut o = [0.0; 12];
    for r in 0..4 {
        for c in 0..3 {
            let (x, y, z) = (a[r * 3], a[r * 3 + 1], a[r * 3 + 2]);
            o[r * 3 + c] = x * b[c] + y * b[3 + c] + z * b[6 + c] + if r == 3 { b[9 + c] } else { 0.0 };
        }
    }
    o
}

#[derive(Default)]
struct Obj {
    verts: Vec<V3>,
    tris: Vec<[u32; 3]>,
    /// (model file, object id, transform)
    components: Vec<(Option<String>, String, Xf)>,
}

#[derive(Default)]
struct ModelFile {
    objects: HashMap<String, Obj>,
    build: Vec<(String, Xf)>,
    unit_mm: f32,
}

fn parse_model_xml(xml: &[u8]) -> ModelFile {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_reader(xml);
    let mut out = ModelFile { unit_mm: 1.0, ..Default::default() };
    let mut cur: Option<(String, Obj)> = None;
    let mut buf = Vec::new();
    loop {
        let ev = match r.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(e) => e,
        };
        let (e, is_end) = match &ev {
            Event::Start(e) | Event::Empty(e) => (e.clone(), false),
            Event::End(e) => {
                if e.local_name().as_ref() == "object"
                    && let Some((id, o)) = cur.take()
                {
                    out.objects.insert(id, o);
                }
                buf.clear();
                continue;
            }
            _ => {
                buf.clear();
                continue;
            }
        };
        let _ = is_end;
        let attr = |name: &str| -> Option<String> {
            e.attributes().flatten().find(|a| a.key.local_name().as_ref() == name).map(|a| a.value.to_string())
        };
        match e.local_name().as_ref() {
            "model" => {
                out.unit_mm = match attr("unit").as_deref() {
                    Some("micron") => 0.001,
                    Some("centimeter") => 10.0,
                    Some("inch") => 25.4,
                    Some("foot") => 304.8,
                    Some("meter") => 1000.0,
                    _ => 1.0,
                }
            }
            "object" => {
                let obj = (attr("id").unwrap_or_default(), Obj::default());
                if matches!(ev, Event::Empty(_)) {
                    out.objects.insert(obj.0, obj.1);
                } else {
                    cur = Some(obj);
                }
            }
            "vertex" => {
                if let Some((_, o)) = &mut cur {
                    let f = |n: &str| attr(n).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                    o.verts.push([f("x"), f("y"), f("z")]);
                }
            }
            "triangle" => {
                if let Some((_, o)) = &mut cur {
                    let f = |n: &str| attr(n).and_then(|s| s.parse::<u32>().ok());
                    if let (Some(a), Some(b), Some(c)) = (f("v1"), f("v2"), f("v3")) {
                        o.tris.push([a, b, c]);
                    }
                }
            }
            "component" => {
                if let Some((_, o)) = &mut cur {
                    let xf = attr("transform").map(|s| parse_xf(&s)).unwrap_or(IDENTITY);
                    o.components.push((attr("path"), attr("objectid").unwrap_or_default(), xf));
                }
            }
            "item" => {
                let xf = attr("transform").map(|s| parse_xf(&s)).unwrap_or(IDENTITY);
                out.build.push((attr("objectid").unwrap_or_default(), xf));
            }
            _ => {}
        }
        buf.clear();
    }
    out
}

pub fn parse_3mf(path: &Path) -> Result<Mesh, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|_| "This 3MF file is damaged".to_string())?;
    let names: Vec<String> = z.file_names().map(str::to_string).collect();
    let mut files: HashMap<String, ModelFile> = HashMap::new();
    for n in names.iter().filter(|n| n.to_lowercase().ends_with(".model")) {
        let mut buf = Vec::new();
        if let Ok(e) = z.by_name(n) {
            e.take(MAX_FILE).read_to_end(&mut buf).map_err(|e| e.to_string())?;
            files.insert(format!("/{}", n.trim_start_matches('/')), parse_model_xml(&buf));
        }
    }
    let root = files
        .keys()
        .find(|k| k.eq_ignore_ascii_case("/3D/3dmodel.model"))
        .or_else(|| files.iter().find(|(_, m)| !m.build.is_empty()).map(|(k, _)| k))
        .cloned()
        .ok_or("This 3MF file has no model")?;
    let unit = files[&root].unit_mm;
    let mut tris = Vec::new();
    fn emit(files: &HashMap<String, ModelFile>, file: &str, id: &str, xf: &Xf, depth: usize, out: &mut Vec<[V3; 3]>) {
        if depth > 8 || out.len() > MAX_TRIS * 2 {
            return;
        }
        let Some(o) = files.get(file).and_then(|m| m.objects.get(id)) else { return };
        for t in &o.tris {
            let v = |i: u32| o.verts.get(i as usize).map(|p| apply(xf, *p));
            if let (Some(a), Some(b), Some(c)) = (v(t[0]), v(t[1]), v(t[2])) {
                out.push([a, b, c]);
            }
        }
        for (p, cid, cxf) in &o.components {
            let target = p.clone().map(|p| format!("/{}", p.trim_start_matches('/'))).unwrap_or(file.to_string());
            emit(files, &target, cid, &compose(cxf, xf), depth + 1, out);
        }
    }
    let build = &files[&root].build;
    for (id, xf) in build {
        emit(&files, &root, id, xf, 0, &mut tris);
    }
    if unit != 1.0 {
        for t in &mut tris {
            for v in t.iter_mut() {
                *v = v.map(|c| c * unit);
            }
        }
    }
    Mesh::new(tris, "3MF", build.len().max(1))
}

// ------------------------------------------------------------------ rendering

/// Camera: `yaw` turns around the vertical (Z) axis, `pitch` looks down from above.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct View {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

impl Default for View {
    /// Front-right, from slightly above: how slicers show a model.
    fn default() -> Self {
        View { yaw: -0.6, pitch: 0.45, zoom: 1.0 }
    }
}

/// Renders the mesh into a `w`x`h` image with a transparent background.
pub fn render(mesh: &Mesh, view: View, w: u32, h: u32) -> Rgba {
    const SS: usize = 2; // supersampling for smooth edges
    let (sw, sh) = (w as usize * SS, h as usize * SS);
    let center = [0, 1, 2].map(|i| (mesh.min[i] + mesh.max[i]) / 2.0);
    // Tightest sphere around the center: the model fills the frame at any angle
    // without its size jumping while it's turned.
    let radius = mesh
        .tris
        .iter()
        .flatten()
        .map(|v| (0..3).map(|i| (v[i] - center[i]).powi(2)).sum::<f32>())
        .fold(0.0f32, f32::max)
        .sqrt()
        .max(1e-6);
    let (sy, cy) = view.yaw.sin_cos();
    let (sp, cp) = view.pitch.sin_cos();
    // World (Z up) -> camera (right, up, depth away from the viewer).
    let cam = |p: V3| -> V3 {
        let (x, y, z) = (p[0] - center[0], p[1] - center[1], p[2] - center[2]);
        let x1 = x * cy - y * sy;
        let y1 = x * sy + y * cy;
        [x1, y1 * sp + z * cp, y1 * cp - z * sp]
    };
    let scale = (sw.min(sh) as f32 / 2.0) * 0.97 * view.zoom / radius;
    let (ox, oy) = (sw as f32 / 2.0, sh as f32 / 2.0);

    let mut depth = vec![f32::INFINITY; sw * sh];
    let mut color = vec![[0u8; 3]; sw * sh];
    let base = [78.0f32, 156.0, 240.0];
    let norm = |v: V3| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
        v.map(|c| c / l)
    };
    let key = norm([-0.45, 0.7, -0.55]);
    let fill = norm([0.7, -0.1, -0.7]);
    let half = norm([key[0], key[1], key[2] - 1.0]);
    for t in &mesh.tris {
        let [a, b, c] = t.map(cam);
        let n = {
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
        };
        let mut n = norm(n);
        if n[2] > 0.0 {
            n = n.map(|c| -c); // light both sides: STL winding is often unreliable
        }
        let dot = |l: V3| (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0);
        let light = 0.34 + 0.6 * dot(key) + 0.26 * dot(fill);
        let spec = dot(half).powf(28.0) * 0.28;
        let shade = base.map(|c| (c * light + 255.0 * spec).clamp(0.0, 255.0) as u8);
        // Screen space (y down).
        let s = |p: V3| [ox + p[0] * scale, oy - p[1] * scale, p[2]];
        let (p0, p1, p2) = (s(a), s(b), s(c));
        let area = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p1[1] - p0[1]) * (p2[0] - p0[0]);
        if area.abs() < 1e-9 {
            continue;
        }
        let x0 = p0[0].min(p1[0]).min(p2[0]).floor().max(0.0) as usize;
        let x1 = (p0[0].max(p1[0]).max(p2[0]).ceil() as usize).min(sw);
        let y0 = p0[1].min(p1[1]).min(p2[1]).floor().max(0.0) as usize;
        let y1 = (p0[1].max(p1[1]).max(p2[1]).ceil() as usize).min(sh);
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let inv = 1.0 / area;
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let w0 = ((p1[0] - px) * (p2[1] - py) - (p1[1] - py) * (p2[0] - px)) * inv;
                let w1 = ((p2[0] - px) * (p0[1] - py) - (p2[1] - py) * (p0[0] - px)) * inv;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * p0[2] + w1 * p1[2] + w2 * p2[2];
                let i = y * sw + x;
                if z < depth[i] {
                    depth[i] = z;
                    color[i] = shade;
                }
            }
        }
    }
    // Downsample: average the covered subsamples, alpha = coverage.
    let mut px = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for dy in 0..SS {
                for dx in 0..SS {
                    let i = (y * SS + dy) * sw + x * SS + dx;
                    if depth[i].is_finite() {
                        r += color[i][0] as u32;
                        g += color[i][1] as u32;
                        b += color[i][2] as u32;
                        n += 1;
                    }
                }
            }
            if let (Some(r), Some(g), Some(b)) = (r.checked_div(n), g.checked_div(n), b.checked_div(n)) {
                let o = (y * w as usize + x) * 4;
                px[o..o + 4].copy_from_slice(&[r as u8, g as u8, b as u8, (n * 255 / (SS * SS) as u32) as u8]);
            }
        }
    }
    Rgba { w, h, px }
}

fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn mm(v: f32) -> String {
    if v >= 100.0 { format!("{v:.0}") } else { format!("{v:.1}") }
}

pub fn load(path: &Path) -> Loaded {
    let mesh = match parse(path) {
        Ok(m) => m,
        Err(e) => {
            // Sliced 3MF (e.g. Bambu Studio's .gcode.3mf) has plate pictures instead of a mesh.
            return match threemf_picture(path) {
                Some(img) => Loaded::new(Content::Image(img)),
                None => Loaded::error(e),
            };
        }
    };
    let img = render(&mesh, View::default(), 900, 900);
    let d = [0, 1, 2].map(|i| mesh.max[i] - mesh.min[i]);
    let mut info = vec![
        row("Size", format!("{} × {} × {} mm", mm(d[0]), mm(d[1]), mm(d[2]))),
        row("Triangles", group(mesh.count)),
    ];
    let vol = mesh.volume();
    if vol > 0.0 {
        info.push(row("Volume", format!("{:.1} cm³", vol / 1000.0)));
    }
    if mesh.objects > 1 {
        info.push(row("Objects", mesh.objects.to_string()));
    }
    info.push(row("Format", mesh.format));
    Loaded::new(Content::Model { mesh: std::sync::Arc::new(mesh), img }).with_info(info)
}

/// Plate or project picture stored in a 3MF by slicers.
fn threemf_picture(path: &Path) -> Option<Rgba> {
    if super::ext_of(path) != "3mf" {
        return None;
    }
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    ["Metadata/plate_1.png", "Metadata/thumbnail.png", "Metadata/top_1.png", "Thumbnails/thumbnail.png"]
        .iter()
        .find_map(|n| {
            let mut buf = Vec::new();
            z.by_name(n).ok()?.take(32 << 20).read_to_end(&mut buf).ok()?;
            Some(Rgba::from_image(image::load_from_memory(&buf).ok()?, 1200))
        })
}

pub fn thumbnail(path: &Path, max: u32) -> Option<Rgba> {
    match parse(path) {
        Ok(mesh) => Some(render(&mesh, View::default(), max, max)),
        Err(_) => threemf_picture(path),
    }
}

// ------------------------------------------------------------------ G-code

fn base64(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// The largest thumbnail a slicer embedded (`; thumbnail begin WxH len` + base64 lines).
pub fn gcode_thumbnail(text: &str) -> Option<image::DynamicImage> {
    let mut best: Option<(u32, String)> = None;
    let mut cur: Option<(u32, String)> = None;
    for line in text.lines() {
        let l = line.trim_start_matches(';').trim();
        if let Some(rest) = l
            .strip_prefix("thumbnail begin")
            .or_else(|| l.strip_prefix("thumbnail_PNG begin"))
            .or_else(|| l.strip_prefix("thumbnail_JPG begin"))
        {
            let area = rest
                .split_whitespace()
                .next()
                .and_then(|d| d.split_once('x'))
                .and_then(|(w, h)| Some(w.parse::<u32>().ok()? * h.parse::<u32>().ok()?))
                .unwrap_or(0);
            cur = Some((area, String::new()));
        } else if l.starts_with("thumbnail end")
            || l.starts_with("thumbnail_PNG end")
            || l.starts_with("thumbnail_JPG end")
        {
            if let Some(c) = cur.take()
                && best.as_ref().is_none_or(|b| c.0 > b.0)
            {
                best = Some(c);
            }
        } else if let Some((_, data)) = &mut cur {
            data.push_str(l);
        }
    }
    image::load_from_memory(&base64(&best?.1)).ok()
}

/// Print details from slicer comments (PrusaSlicer, OrcaSlicer, Bambu Studio, Cura).
pub fn gcode_info(text: &str) -> Vec<(String, String)> {
    let mut info = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |k: &str, v: String| {
        if !v.is_empty() && seen.insert(k.to_string()) {
            info.push(row(k, v));
        }
    };
    let cura_time = |s: &str| -> Option<String> {
        let t: u64 = s.trim().parse::<f64>().ok()? as u64;
        Some(if t >= 3600 { format!("{}h {}m", t / 3600, t % 3600 / 60) } else { format!("{}m {}s", t / 60, t % 60) })
    };
    for line in text.lines().filter(|l| l.starts_with(';')) {
        let l = line.trim_start_matches(';').trim();
        let (k, v) = match l.split_once('=').or_else(|| l.split_once(':')) {
            Some((k, v)) => (k.trim().to_lowercase(), v.trim()),
            None => continue,
        };
        match k.as_str() {
            "estimated printing time (normal mode)" | "total estimated time" | "model printing time" => {
                push("Print time", v.split(';').next().unwrap_or(v).trim().to_string())
            }
            "time" => {
                if let Some(t) = cura_time(v) {
                    push("Print time", t)
                }
            }
            "filament used [g]" | "total filament weight [g]" => push("Filament", format!("{v} g")),
            "filament used" => push("Filament", v.to_string()),
            "filament used [mm]" => {
                if let Ok(mm) = v.split(',').next().unwrap_or(v).trim().parse::<f64>() {
                    push("Filament length", format!("{:.2} m", mm / 1000.0))
                }
            }
            "filament_type" | "filament type" => push("Material", v.replace(';', ", ")),
            "layer_height" | "layer height" => push("Layer height", format!("{v} mm")),
            "nozzle_diameter" => push("Nozzle", format!("{} mm", v.split(',').next().unwrap_or(v))),
            "printer_model" | "printer_settings_id" => push("Printer", v.trim_matches('"').to_string()),
            "generated by" => push("Sliced with", v.to_string()),
            _ => {}
        }
    }
    // PrusaSlicer's header line: "; generated by PrusaSlicer 2.7.1 on ..."
    if let Some(g) = text.lines().take(5).find_map(|l| l.strip_prefix("; generated by ")) {
        push("Sliced with", g.split(" on ").next().unwrap_or(g).to_string());
    }
    info
}

pub fn load_gcode(path: &Path) -> Loaded {
    let text = match read_gcode(path) {
        Some(t) => t,
        None => return Loaded::error("This G-code file couldn't be read"),
    };
    let info = gcode_info(&text);
    match gcode_thumbnail(&text) {
        Some(img) => Loaded::new(Content::Image(Rgba::from_image(img, 1200))).with_info(info),
        None => {
            let mut l = super::misc::load_text_or_binary(path);
            l.info.splice(0..0, info);
            l
        }
    }
}

/// Slicers write thumbnails and settings at the start and end; skip the middle of big files.
fn read_gcode(path: &Path) -> Option<String> {
    use std::io::{Seek, SeekFrom};
    const PART: u64 = 2 << 20;
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut buf = Vec::new();
    (&mut f).take(PART).read_to_end(&mut buf).ok()?;
    if len > PART * 2 {
        f.seek(SeekFrom::End(-(PART as i64))).ok()?;
        buf.push(b'\n');
        f.take(PART).read_to_end(&mut buf).ok()?;
    } else {
        f.read_to_end(&mut buf).ok()?;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

pub fn gcode_thumb(path: &Path, max: u32) -> Option<Rgba> {
    let text = read_gcode(path)?;
    Some(Rgba::from_image(gcode_thumbnail(&text)?, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube_stl_binary() -> Vec<u8> {
        // 12 triangles of a 10 mm cube.
        let v = |x: f32, y: f32, z: f32| [x * 10.0, y * 10.0, z * 10.0];
        let quads = [
            [v(0., 0., 0.), v(1., 0., 0.), v(1., 1., 0.), v(0., 1., 0.)],
            [v(0., 0., 1.), v(0., 1., 1.), v(1., 1., 1.), v(1., 0., 1.)],
            [v(0., 0., 0.), v(0., 0., 1.), v(1., 0., 1.), v(1., 0., 0.)],
            [v(0., 1., 0.), v(1., 1., 0.), v(1., 1., 1.), v(0., 1., 1.)],
            [v(0., 0., 0.), v(0., 1., 0.), v(0., 1., 1.), v(0., 0., 1.)],
            [v(1., 0., 0.), v(1., 0., 1.), v(1., 1., 1.), v(1., 1., 0.)],
        ];
        let mut out = vec![0u8; 80];
        out.extend_from_slice(&12u32.to_le_bytes());
        for q in quads {
            for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                out.extend_from_slice(&[0u8; 12]);
                for p in t {
                    for c in p {
                        out.extend_from_slice(&c.to_le_bytes());
                    }
                }
                out.extend_from_slice(&[0, 0]);
            }
        }
        out
    }

    #[test]
    fn stl_binary_and_volume() {
        let m = parse_stl(&cube_stl_binary()).unwrap();
        assert_eq!(m.count, 12);
        assert_eq!(m.max, [10.0, 10.0, 10.0]);
        assert!((m.volume() - 1000.0).abs() < 0.01, "{}", m.volume());
        let img = render(&m, View::default(), 64, 64);
        let covered = img.px.chunks(4).filter(|p| p[3] > 0).count();
        assert!(covered > 64 * 64 / 4, "cube should fill a good part of the frame: {covered}");
    }

    #[test]
    fn stl_text_obj_ply() {
        let stl = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        assert_eq!(parse_stl(stl.as_bytes()).unwrap().count, 1);
        let obj = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1/1 2/2 3/3 4/4\n";
        assert_eq!(parse_obj(obj).unwrap().count, 2);
        let ply = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
        assert_eq!(parse_ply(ply.as_bytes()).unwrap().count, 1);
        let mut bin = b"ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n".to_vec();
        for c in [0f32, 0., 0., 2., 0., 0., 0., 2., 0.] {
            bin.extend_from_slice(&c.to_le_bytes());
        }
        bin.push(3);
        for i in [0i32, 1, 2] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        let m = parse_ply(&bin).unwrap();
        assert_eq!((m.count, m.max[0]), (1, 2.0));
    }

    #[test]
    fn threemf_with_components_and_transforms() {
        let dir = std::env::temp_dir().join(format!("ff-3mf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.3mf");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("3D/3dmodel.model", o).unwrap();
        std::io::Write::write_all(&mut z, br#"<?xml version="1.0"?>
<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06">
 <resources>
  <object id="2" type="model"><components><component p:path="/3D/Objects/o.model" objectid="1" transform="1 0 0 0 1 0 0 0 1 5 0 0"/></components></object>
 </resources>
 <build><item objectid="2" transform="2 0 0 0 2 0 0 0 2 0 0 0"/></build>
</model>"#).unwrap();
        z.start_file("3D/Objects/o.model", o).unwrap();
        std::io::Write::write_all(
            &mut z,
            br#"<model unit="millimeter"><resources><object id="1"><mesh>
<vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices>
<triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources></model>"#,
        )
        .unwrap();
        z.finish().unwrap();
        let m = parse_3mf(&path).unwrap();
        assert_eq!(m.count, 1);
        // Component moves x by 5, then the build item scales by 2: x spans 10..12.
        assert_eq!((m.min[0], m.max[0], m.max[1]), (10.0, 12.0, 2.0));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gcode_details_and_thumbnail() {
        // A 1x1 PNG, base64, split across lines like slicers do.
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
        let text = format!(
            "; generated by PrusaSlicer 2.7.1+linux on 2026-01-01\n;\n; thumbnail begin 1x1 {}\n; {}\n; {}\n; thumbnail end\nG28\n; filament used [g] = 12.34\n; estimated printing time (normal mode) = 1h 2m 3s\n; filament_type = PLA\n; layer_height = 0.2\n",
            png.len(),
            &png[..40],
            &png[40..]
        );
        let img = gcode_thumbnail(&text).expect("thumbnail");
        assert_eq!((img.width(), img.height()), (1, 1));
        let info = gcode_info(&text);
        let get = |k: &str| info.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
        assert_eq!(get("Print time"), Some("1h 2m 3s"));
        assert_eq!(get("Filament"), Some("12.34 g"));
        assert_eq!(get("Material"), Some("PLA"));
        assert_eq!(get("Layer height"), Some("0.2 mm"));
        assert_eq!(get("Sliced with"), Some("PrusaSlicer 2.7.1+linux"));
    }
}
