//! Build `fixtures/gobos.tar.gz`, the gobo catalogue DMXpress renders beams
//! through.
//!
//! Sources:
//! - QLC+ `resources/gobos/<Maker>/gobo*.{png,svg}` (~1,000 images), and its
//!   `resources/fixtures/**/*.qxf` capabilities, which reference those images
//!   per DMX range (`Res1="Chauvet/gobo00079.png"`).
//! - OFL `resources/gobos/*.{png,svg}` + `.json` (names and keywords), and its
//!   fixture wheel slots that carry a `resource: "gobos/<key>"`.
//!
//! Both draw a gobo the same way: the metal is dark opaque ink, and the
//! apertures are either light ink or left transparent. So a mask pixel is
//! "open" to the degree that it is transparent or bright.
//!
//! Output, one tar.gz:
//! - `index.json`  — every gobo: key, display name, maker, keywords.
//! - `wheels.json` — per (manufacturer, model): channel name → DMX ranges → key,
//!   which the app joins onto patched fixtures by name and range.
//! - `<key>.png`   — the mask, `size`×`size` 8-bit grey, white = light passes.
//!
//! Usage:
//!   gobo-pack --qlc <qlcplus checkout> --ofl <ofl checkout> --out fixtures/gobos.tar.gz
//!             [--size 256] [--levels 8] [--sheet debug.png]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::FilterType;
use image::{GrayImage, ImageEncoder, RgbaImage};
use resvg::tiny_skia;
use serde::Serialize;

struct Source {
    key: String,
    path: PathBuf,
    manufacturer: String,
    /// From OFL metadata, when there is any.
    name: Option<String>,
    keywords: Option<String>,
}

#[derive(Serialize)]
struct IndexEntry {
    key: String,
    name: String,
    manufacturer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    keywords: Option<String>,
}

#[derive(Serialize)]
struct Index {
    version: u32,
    source: &'static str,
    gobos: Vec<IndexEntry>,
}

/// One DMX range on one wheel channel: [min, max, key, label].
type Range = (u8, u8, String, String);

#[derive(Serialize, Default)]
struct WheelFixture {
    manufacturer: String,
    model: String,
    source: &'static str,
    /// Channel name → ranges.
    wheels: BTreeMap<String, Vec<Range>>,
}

#[derive(Serialize)]
struct Wheels {
    version: u32,
    fixtures: Vec<WheelFixture>,
}

fn main() {
    let mut qlc: Option<PathBuf> = None;
    let mut ofl: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut sheet: Option<PathBuf> = None;
    let mut size: u32 = 256;
    let mut levels: u32 = 0;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--qlc" => qlc = args.next().map(PathBuf::from),
            "--ofl" => ofl = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--sheet" => sheet = args.next().map(PathBuf::from),
            "--size" => size = args.next().and_then(|s| s.parse().ok()).unwrap_or(256),
            // Grey levels to keep (0 = all 256). Fewer levels compress far
            // better and the GPU's bilinear filter smooths the steps back out.
            "--levels" => levels = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }

    let mut sources: Vec<Source> = Vec::new();
    let mut fixtures: Vec<WheelFixture> = Vec::new();
    // Capability labels seen per key, to name the QLC+ gobos.
    let mut labels: HashMap<String, HashMap<String, usize>> = HashMap::new();

    if let Some(qlc) = &qlc {
        collect_qlc_images(qlc, &mut sources);
        collect_qlc_wheels(qlc, &sources, &mut fixtures, &mut labels);
    }
    if let Some(ofl) = &ofl {
        collect_ofl_images(ofl, &mut sources);
        collect_ofl_wheels(ofl, &mut fixtures);
    }
    eprintln!("{} gobo images, {} fixtures with wheel maps", sources.len(), fixtures.len());

    if let Some(sheet) = &sheet {
        write_sheet(&sources, sheet);
    }
    let Some(out) = out else {
        return;
    };

    // Rasterize.
    let mut masks: Vec<(String, Vec<u8>)> = Vec::new();
    let mut failed = 0;
    for s in &sources {
        match rasterize(&s.path, size) {
            Some(rgba) => masks.push((s.key.clone(), encode_png(&to_mask(&rgba, levels)))),
            None => {
                failed += 1;
                eprintln!("  ! could not rasterize {}", s.path.display());
            }
        }
    }
    let rendered: HashSet<&str> = masks.iter().map(|(k, _)| k.as_str()).collect();

    // Names: OFL metadata first, then the most common capability label.
    let index = Index {
        version: 1,
        source: "QLC+ (Apache-2.0) resources/gobos and fixture definitions; \
                 Open Fixture Library (MIT) resources/gobos",
        gobos: sources
            .iter()
            .filter(|s| rendered.contains(s.key.as_str()))
            .map(|s| {
                let name = s
                    .name
                    .clone()
                    .or_else(|| {
                        labels.get(&s.key).and_then(|m| {
                            m.iter()
                                .max_by_key(|(l, n)| (**n, std::cmp::Reverse(l.len())))
                                .map(|(l, _)| l.clone())
                        })
                    })
                    .unwrap_or_else(|| stem(&s.path));
                IndexEntry {
                    key: s.key.clone(),
                    name,
                    manufacturer: s.manufacturer.clone(),
                    keywords: s.keywords.clone(),
                }
            })
            .collect(),
    };
    // Drop ranges whose image failed to render.
    for f in &mut fixtures {
        for ranges in f.wheels.values_mut() {
            ranges.retain(|(_, _, k, _)| rendered.contains(k.as_str()));
        }
        f.wheels.retain(|_, r| !r.is_empty());
    }
    fixtures.retain(|f| !f.wheels.is_empty());
    let wheels = Wheels { version: 1, fixtures };

    let file = fs::File::create(&out).expect("create output");
    let gz = flate2::write::GzEncoder::new(file, flate2::Compression::best());
    let mut tar = tar::Builder::new(gz);
    let mut total = 0usize;
    let mut add = |tar: &mut tar::Builder<_>, path: &str, data: &[u8]| {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_mtime(0);
        h.set_cksum();
        tar.append_data(&mut h, path, data).expect("append");
        total += data.len();
    };
    add(&mut tar, "index.json", serde_json::to_string(&index).unwrap().as_bytes());
    add(&mut tar, "wheels.json", serde_json::to_string(&wheels).unwrap().as_bytes());
    for (key, png) in &masks {
        add(&mut tar, &format!("{key}.png"), png);
    }
    tar.into_inner().expect("tar").finish().expect("gzip").flush().expect("flush");
    let packed = fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    eprintln!(
        "{} masks ({} failed), {} fixtures mapped -> {} ({:.1} MB packed, {:.1} MB raw)",
        masks.len(),
        failed,
        wheels.fixtures.len(),
        out.display(),
        packed as f64 / 1e6,
        total as f64 / 1e6
    );
}

fn stem(p: &Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or("gobo").to_string()
}

// ------------------------------------------------------------------ sources

fn collect_qlc_images(qlc: &Path, out: &mut Vec<Source>) {
    let root = qlc.join("resources").join("gobos");
    let Ok(makers) = fs::read_dir(&root) else {
        eprintln!("no {}", root.display());
        return;
    };
    let mut dirs: Vec<PathBuf> = makers.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for dir in dirs {
        let maker = dir.file_name().unwrap().to_string_lossy().to_string();
        let mut files: Vec<PathBuf> = fs::read_dir(&dir)
            .map(|d| d.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        for path in files {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            if ext != "png" && ext != "svg" {
                continue;
            }
            let key = format!("qlc/{maker}/{}", stem(&path));
            // A .png and .svg of the same gobo: keep the vector one.
            if let Some(existing) = out.iter_mut().find(|s| s.key == key) {
                if ext == "svg" {
                    existing.path = path;
                }
                continue;
            }
            out.push(Source { key, path, manufacturer: maker.clone(), name: None, keywords: None });
        }
    }
}

/// `Chauvet/gobo00079.png` → `qlc/Chauvet/gobo00079`, if that image exists.
fn qlc_key(res: &str, sources: &[Source]) -> Option<String> {
    let res = res.trim().replace('\\', "/");
    let (maker, file) = res.rsplit_once('/')?;
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let key = format!("qlc/{maker}/{stem}");
    sources.iter().any(|s| s.key == key).then_some(key)
}

fn collect_qlc_wheels(
    qlc: &Path,
    sources: &[Source],
    out: &mut Vec<WheelFixture>,
    labels: &mut HashMap<String, HashMap<String, usize>>,
) {
    let root = qlc.join("resources").join("fixtures");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    for path in files {
        if path.extension().and_then(|e| e.to_str()) != Some("qxf") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        // Every .qxf opens with `<!DOCTYPE FixtureDefinition>`, which roxmltree
        // refuses unless told DTDs are fine.
        let options = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
        let Ok(doc) = roxmltree::Document::parse_with_options(&text, options) else {
            eprintln!("  ! bad xml {}", path.display());
            continue;
        };
        let text_of = |tag: &str| {
            doc.descendants()
                .find(|n| n.has_tag_name(tag))
                .and_then(|n| n.text())
                .map(|t| t.trim().to_string())
                .unwrap_or_default()
        };
        let manufacturer = text_of("Manufacturer");
        let model = text_of("Model");
        if manufacturer.is_empty() || model.is_empty() {
            continue;
        }
        let mut fixture = WheelFixture { manufacturer, model, source: "qlc", wheels: BTreeMap::new() };
        for ch in doc
            .descendants()
            .filter(|n| n.has_tag_name("Channel") && n.attribute("Name").is_some())
        {
            // Only definition channels carry capabilities; mode channel refs don't.
            let name = ch.attribute("Name").unwrap().trim().to_string();
            let mut ranges: Vec<Range> = Vec::new();
            for cap in ch.children().filter(|n| n.has_tag_name("Capability")) {
                let res = cap.attribute("Res1").or_else(|| cap.attribute("Res"));
                let Some(res) = res else { continue };
                let Some(key) = qlc_key(res, sources) else { continue };
                let lo: u32 = cap.attribute("Min").and_then(|v| v.parse().ok()).unwrap_or(0);
                let hi: u32 = cap.attribute("Max").and_then(|v| v.parse().ok()).unwrap_or(255);
                let label = cap.text().unwrap_or("").trim().to_string();
                if !label.is_empty() {
                    *labels.entry(key.clone()).or_default().entry(label.clone()).or_default() += 1;
                }
                ranges.push((lo.min(255) as u8, hi.min(255) as u8, key, label));
            }
            if !ranges.is_empty() {
                fixture.wheels.insert(name, ranges);
            }
        }
        if !fixture.wheels.is_empty() {
            out.push(fixture);
        }
    }
}

fn collect_ofl_images(ofl: &Path, out: &mut Vec<Source>) {
    let root = ofl.join("resources").join("gobos");
    let Ok(rd) = fs::read_dir(&root) else {
        eprintln!("no {}", root.display());
        return;
    };
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    files.sort();
    for path in files {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if ext != "png" && ext != "svg" {
            continue;
        }
        let key = format!("ofl/{}", stem(&path));
        if let Some(existing) = out.iter_mut().find(|s| s.key == key) {
            if ext == "svg" {
                existing.path = path;
            }
            continue;
        }
        let meta: Option<serde_json::Value> = fs::read_to_string(path.with_extension("json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok());
        let field = |k: &str| {
            meta.as_ref()
                .and_then(|m| m.get(k))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        out.push(Source {
            key,
            path,
            manufacturer: "Open Fixture Library".into(),
            name: field("name"),
            keywords: field("keywords"),
        });
    }
}

fn collect_ofl_wheels(ofl: &Path, out: &mut Vec<WheelFixture>) {
    let root = ofl.join("fixtures");
    let makers: HashMap<String, String> = fs::read_to_string(root.join("manufacturers.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| {
                    v.get("name").and_then(|n| n.as_str()).map(|n| (k.clone(), n.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    for path in files {
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        if !text.contains("\"resource\"") {
            continue;
        }
        let Ok(fx) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let Some(wheels) = fx.get("wheels").and_then(|w| w.as_object()) else { continue };
        let maker_key = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let manufacturer = makers.get(maker_key).cloned().unwrap_or_else(|| maker_key.to_string());
        let model = fx.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        if model.is_empty() {
            continue;
        }
        let mut fixture = WheelFixture { manufacturer, model, source: "ofl", wheels: BTreeMap::new() };
        let Some(channels) = fx.get("availableChannels").and_then(|c| c.as_object()) else { continue };
        for (ch_name, ch) in channels {
            let caps: Vec<&serde_json::Value> = match (ch.get("capabilities"), ch.get("capability")) {
                (Some(serde_json::Value::Array(a)), _) => a.iter().collect(),
                (_, Some(c)) => vec![c],
                _ => Vec::new(),
            };
            let mut ranges: Vec<Range> = Vec::new();
            for cap in caps {
                let t = cap.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if !matches!(t, "WheelSlot" | "WheelShake" | "WheelSlotRotation") {
                    continue;
                }
                let wheel_name = match cap.get("wheel") {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(serde_json::Value::Array(a)) => {
                        a.first().and_then(|v| v.as_str()).unwrap_or(ch_name).to_string()
                    }
                    _ => ch_name.clone(),
                };
                let Some(slots) = wheels
                    .get(&wheel_name)
                    .and_then(|w| w.get("slots"))
                    .and_then(|s| s.as_array())
                else {
                    continue;
                };
                let Some(n) = cap.get("slotNumber").and_then(|n| n.as_f64()) else { continue };
                let idx = n as usize;
                if idx == 0 || idx > slots.len() || n.fract() != 0.0 {
                    continue;
                }
                let slot = &slots[idx - 1];
                let Some(res) = slot.get("resource").and_then(|r| r.as_str()) else { continue };
                let Some(gobo) = res.strip_prefix("gobos/") else { continue };
                if gobo.starts_with("aliases/") {
                    continue;
                }
                let key = format!("ofl/{gobo}");
                let range = cap.get("dmxRange").and_then(|r| r.as_array());
                let (mut lo, mut hi) = match range {
                    Some(r) if r.len() == 2 => {
                        (r[0].as_u64().unwrap_or(0), r[1].as_u64().unwrap_or(255))
                    }
                    _ => (0, 255),
                };
                while lo.max(hi) > 255 {
                    lo >>= 8;
                    hi >>= 8;
                }
                let label = slot.get("name").and_then(|s| s.as_str()).unwrap_or("").to_string();
                ranges.push((lo as u8, hi as u8, key, label));
            }
            if !ranges.is_empty() {
                fixture.wheels.insert(ch_name.clone(), ranges);
            }
        }
        if !fixture.wheels.is_empty() {
            out.push(fixture);
        }
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

// ------------------------------------------------------------ rasterizing

/// The image as straight-alpha RGBA, scaled to fit and centred in a
/// `size`×`size` transparent square.
fn rasterize(path: &Path, size: u32) -> Option<RgbaImage> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let data = fs::read(path).ok()?;
    if ext == "svg" {
        let opt = resvg::usvg::Options::default();
        let tree = resvg::usvg::Tree::from_data(&data, &opt).ok()?;
        let (w, h) = (tree.size().width(), tree.size().height());
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let scale = size as f32 / w.max(h);
        let tx = (size as f32 - w * scale) * 0.5;
        let ty = (size as f32 - h * scale) * 0.5;
        let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
        let transform = tiny_skia::Transform::from_translate(tx, ty).pre_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        let mut out = RgbaImage::new(size, size);
        for (i, px) in pixmap.pixels().iter().enumerate() {
            let c = px.demultiply();
            let (x, y) = ((i as u32) % size, (i as u32) / size);
            out.put_pixel(x, y, image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]));
        }
        Some(out)
    } else {
        let img = image::load_from_memory(&data).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            return None;
        }
        let scale = size as f32 / w.max(h) as f32;
        let nw = ((w as f32 * scale).round() as u32).max(1);
        let nh = ((h as f32 * scale).round() as u32).max(1);
        let scaled = image::imageops::resize(&img, nw, nh, FilterType::Lanczos3);
        let mut out = RgbaImage::new(size, size);
        image::imageops::overlay(&mut out, &scaled, ((size - nw) / 2) as i64, ((size - nh) / 2) as i64);
        Some(out)
    }
}

/// Mask: 255 where light passes. Dark opaque ink is the metal; anything
/// transparent or bright is an aperture (a coloured glass gobo comes out as
/// partially open, in proportion to how bright the colour is). Everything
/// outside the inscribed circle is the gobo holder, so it is blocked.
fn to_mask(rgba: &RgbaImage, levels: u32) -> GrayImage {
    let (w, h) = rgba.dimensions();
    let mut out = GrayImage::new(w, h);
    let cx = (w as f32 - 1.0) * 0.5;
    let cy = (h as f32 - 1.0) * 0.5;
    let radius = w.min(h) as f32 * 0.5;
    for (x, y, p) in rgba.enumerate_pixels() {
        let [r, g, b, a] = p.0;
        let luma = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0;
        let a = a as f32 / 255.0;
        let dist = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
        let holder = (radius - dist + 0.5).clamp(0.0, 1.0);
        let mut open = (1.0 - a * (1.0 - luma)) * holder;
        if levels > 1 {
            let steps = (levels - 1) as f32;
            open = (open * steps).round() / steps;
        }
        out.put_pixel(x, y, image::Luma([(open.clamp(0.0, 1.0) * 255.0).round() as u8]));
    }
    out
}

fn encode_png(img: &GrayImage) -> Vec<u8> {
    let mut buf = Vec::new();
    let enc = PngEncoder::new_with_quality(&mut buf, CompressionType::Best, PngFilter::Adaptive);
    enc.write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::L8)
        .expect("png encode");
    buf
}

/// Debug contact sheet: raw renders over mid grey next to their masks.
fn write_sheet(sources: &[Source], out: &Path) {
    const CELL: u32 = 96;
    const COLS: u32 = 12;
    let step = (sources.len() / 48).max(1);
    let picks: Vec<&Source> = sources.iter().step_by(step).take(48).collect();
    let rows = (picks.len() as u32).div_ceil(COLS / 2);
    let mut sheet =
        RgbaImage::from_pixel(COLS * CELL, rows * CELL, image::Rgba([128, 128, 128, 255]));
    for (i, s) in picks.iter().enumerate() {
        let Some(img) = rasterize(&s.path, CELL - 8) else { continue };
        let col = (i as u32 % (COLS / 2)) * 2;
        let row = i as u32 / (COLS / 2);
        image::imageops::overlay(&mut sheet, &img, (col * CELL + 4) as i64, (row * CELL + 4) as i64);
        let mask = to_mask(&img, 0);
        let mask_rgba = RgbaImage::from_fn(mask.width(), mask.height(), |x, y| {
            let v = mask.get_pixel(x, y).0[0];
            image::Rgba([v, v, v, 255])
        });
        image::imageops::overlay(
            &mut sheet,
            &mask_rgba,
            ((col + 1) * CELL + 4) as i64,
            (row * CELL + 4) as i64,
        );
    }
    sheet.save(out).expect("sheet");
    for (i, s) in picks.iter().enumerate() {
        eprintln!("sheet {:2}: {}", i, s.key);
    }
}
