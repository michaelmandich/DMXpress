//! The gobo catalogue: the masks beams are rendered through, their names,
//! and the wheel map that ties them to fixtures.
//!
//! Ships as `fixtures/gobos.tar.gz`, built by `tools/gobo-pack` from the
//! QLC+ and Open Fixture Library gobo art and seeded into the data dir the
//! same way the fixture library is. Inside: `index.json` (one entry per
//! gobo), `wheels.json` (per manufacturer and model, which gobo sits in which
//! DMX range of which channel) and one 8-bit PNG mask per gobo, white where
//! light passes.
//!
//! Loose PNGs dropped into `fixtures/gobos/` join the catalogue under
//! `user/…` keys, for gobos nobody has drawn yet — a photo of the projected
//! pattern on a dark wall works. Slot assignments made by hand in the Gobos
//! window live in `gobos_user.json`, keyed by fixture profile so every light
//! of that type picks them up.
//!
//! At patch time [`assign_patch`] fills `Band::gobo` on every gobo wheel of
//! every fixture; the stage's atlas (`stage::atlas`) uploads exactly those
//! masks to the GPU and the beam shader samples them.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::fixturedb::{self, Library};
use crate::showbuddy::{Fixture, Patch};

/// The bundled catalogue, as shipped.
pub const PACK_FILE: &str = "fixtures/gobos.tar.gz";
/// Loose user PNGs, scanned recursively.
pub const USER_DIR: &str = "fixtures/gobos";
/// Hand-made slot assignments.
pub const USER_FILE: &str = "gobos_user.json";
/// Side of every mask handed to the GPU; other sizes are resampled on load.
pub const MASK_SIZE: u32 = 256;

/// One gobo the catalogue knows.
pub struct Gobo {
    /// Stable key, e.g. `qlc/Chauvet/gobo00079`, `ofl/stars`, `user/my-logo`.
    pub key: String,
    pub name: String,
    pub manufacturer: String,
    pub keywords: String,
    /// Lowercased words for searching.
    haystack: String,
    /// The PNG as stored; decoded on demand.
    png: Vec<u8>,
}

/// A decoded mask: `size`×`size`, one byte per pixel, 255 = light passes.
pub struct Mask {
    pub size: u32,
    pub data: Vec<u8>,
}

#[derive(Deserialize, Default)]
struct IndexFile {
    #[serde(default)]
    gobos: Vec<IndexEntry>,
}

#[derive(Deserialize)]
struct IndexEntry {
    key: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    manufacturer: String,
    #[serde(default)]
    keywords: Option<String>,
}

/// One DMX range on a wheel channel: min, max, gobo key, slot label.
pub type WheelRange = (u8, u8, String, String);

/// Which gobo sits in which range of which channel, for one model.
#[derive(Debug, Clone, Deserialize)]
pub struct WheelMap {
    pub manufacturer: String,
    pub model: String,
    /// Channel name → ranges.
    pub wheels: BTreeMap<String, Vec<WheelRange>>,
}

#[derive(Deserialize, Default)]
struct WheelsFile {
    #[serde(default)]
    fixtures: Vec<WheelMap>,
}

#[derive(Default)]
pub struct Catalogue {
    pub gobos: Vec<Gobo>,
    by_key: HashMap<String, usize>,
    maps: Vec<WheelMap>,
    /// (squashed maker, squashed model) per map, for matching.
    map_keys: Vec<(String, String)>,
    /// Decoded masks; `None` remembers a PNG that failed to decode.
    masks: Mutex<HashMap<String, Option<Arc<Mask>>>>,
    /// Why the pack is missing, when it is. Loose user gobos still work.
    pub error: Option<String>,
}

/// Letters and digits only, lowercased: "Clay Paky" and "claypaky" agree.
pub fn squash(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

fn io_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
}

impl Catalogue {
    /// Reads the pack and the loose user PNGs off disk. Call once.
    pub fn load() -> Self {
        let mut cat = Self::default();
        match std::fs::File::open(PACK_FILE) {
            Ok(file) => {
                if let Err(e) = cat.read_pack(file) {
                    cat.error = Some(format!("{PACK_FILE}: {e}"));
                }
            }
            Err(e) => cat.error = Some(format!("{PACK_FILE}: {e}")),
        }
        cat.scan_user_dir(Path::new(USER_DIR), "user");
        cat.finish();
        cat
    }

    /// Builds a catalogue from an already-open pack, for tests.
    #[cfg(test)]
    fn from_pack(file: std::fs::File) -> std::io::Result<Self> {
        let mut cat = Self::default();
        cat.read_pack(file)?;
        cat.finish();
        Ok(cat)
    }

    fn read_pack(&mut self, file: std::fs::File) -> std::io::Result<()> {
        let gz = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
        let mut archive = tar::Archive::new(gz);
        let mut index = IndexFile::default();
        let mut wheels = WheelsFile::default();
        let mut pngs: HashMap<String, Vec<u8>> = HashMap::new();
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.to_string_lossy().replace('\\', "/");
            let mut data = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut data)?;
            match path.as_str() {
                "index.json" => index = serde_json::from_slice(&data).map_err(io_err)?,
                "wheels.json" => wheels = serde_json::from_slice(&data).map_err(io_err)?,
                p => {
                    if let Some(key) = p.strip_suffix(".png") {
                        pngs.insert(key.to_string(), data);
                    }
                }
            }
        }
        for e in index.gobos {
            let Some(png) = pngs.remove(&e.key) else { continue };
            let name = if e.name.is_empty() { last_segment(&e.key) } else { e.name };
            self.gobos.push(Gobo {
                key: e.key,
                name,
                manufacturer: e.manufacturer,
                keywords: e.keywords.unwrap_or_default(),
                haystack: String::new(),
                png,
            });
        }
        // Masks the index doesn't list still count, named after their file.
        for (key, png) in pngs {
            self.gobos.push(Gobo {
                name: last_segment(&key),
                manufacturer: key.split('/').nth(1).unwrap_or("").to_string(),
                key,
                keywords: String::new(),
                haystack: String::new(),
                png,
            });
        }
        self.maps = wheels.fixtures;
        Ok(())
    }

    /// Every `*.png` under `dir`, keyed `prefix/<relative path>` without the
    /// extension. Nothing is decoded here.
    fn scan_user_dir(&mut self, dir: &Path, prefix: &str) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if path.is_dir() {
                self.scan_user_dir(&path, &format!("{prefix}/{name}"));
                continue;
            }
            let Some(stem) = name.strip_suffix(".png").or_else(|| name.strip_suffix(".PNG")) else {
                continue;
            };
            let Ok(png) = std::fs::read(&path) else { continue };
            self.gobos.push(Gobo {
                key: format!("{prefix}/{stem}"),
                name: stem.replace(['-', '_'], " "),
                manufacturer: "Custom".into(),
                keywords: String::new(),
                haystack: String::new(),
                png,
            });
        }
    }

    fn finish(&mut self) {
        self.gobos.sort_by(|a, b| {
            (a.manufacturer.to_lowercase(), &a.key).cmp(&(b.manufacturer.to_lowercase(), &b.key))
        });
        self.gobos.dedup_by(|a, b| a.key == b.key);
        for g in &mut self.gobos {
            g.haystack = format!("{} {} {} {}", g.name, g.manufacturer, g.keywords, g.key).to_lowercase();
        }
        self.by_key = self.gobos.iter().enumerate().map(|(i, g)| (g.key.clone(), i)).collect();
        self.map_keys = self.maps.iter().map(|m| (squash(&m.manufacturer), squash(&m.model))).collect();
    }

    pub fn get(&self, key: &str) -> Option<&Gobo> {
        self.by_key.get(key).map(|&i| &self.gobos[i])
    }

    /// Distinct manufacturers, sorted.
    pub fn manufacturers(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self.gobos.iter().map(|g| g.manufacturer.as_str()).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Indices of gobos matching every whitespace-separated term of `query`,
    /// optionally pinned to one manufacturer, capped at `limit`.
    pub fn search(&self, query: &str, manufacturer: Option<&str>, limit: usize) -> Vec<usize> {
        let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
        let mut out = Vec::new();
        for (i, g) in self.gobos.iter().enumerate() {
            if manufacturer.is_some_and(|m| m != g.manufacturer) {
                continue;
            }
            if terms.iter().all(|t| g.haystack.contains(t.as_str())) {
                out.push(i);
                if out.len() >= limit {
                    break;
                }
            }
        }
        out
    }

    /// The decoded mask for `key`, resampled to [`MASK_SIZE`]. Cached; a
    /// PNG that won't decode is remembered as missing rather than retried.
    pub fn mask(&self, key: &str) -> Option<Arc<Mask>> {
        if let Some(hit) = self.masks.lock().get(key) {
            return hit.clone();
        }
        let decoded = self.get(key).and_then(|g| decode_mask(&g.png)).map(Arc::new);
        self.masks.lock().insert(key.to_string(), decoded.clone());
        decoded
    }

    /// The wheel map for a model: an exact maker+model match, else the same
    /// model under a compatible maker ("Chauvet" vs "Chauvet DJ").
    pub fn map_for(&self, manufacturer: &str, model: &str) -> Option<&WheelMap> {
        let (mk, md) = (squash(manufacturer), squash(model));
        if md.is_empty() {
            return None;
        }
        if let Some(i) = self.map_keys.iter().position(|(a, b)| *a == mk && *b == md) {
            return Some(&self.maps[i]);
        }
        self.map_keys
            .iter()
            .position(|(a, b)| {
                *b == md && (mk.is_empty() || a.is_empty() || a.contains(mk.as_str()) || mk.contains(a.as_str()))
            })
            .map(|i| &self.maps[i])
    }

    /// For fixtures known only by a free-text name (a built-in profile, a
    /// ShowBuddy import): the longest model name the text contains.
    pub fn map_for_hint(&self, hint: &str) -> Option<&WheelMap> {
        let h = squash(hint);
        self.map_keys
            .iter()
            .enumerate()
            .filter(|(_, (_, m))| m.len() >= 5 && h.contains(m.as_str()))
            .max_by_key(|(_, (_, m))| m.len())
            .map(|(i, _)| &self.maps[i])
    }

    /// Fill `Band::gobo` on the fixture's gobo wheels: the wheel map first,
    /// then the Gobos window's own assignments on top. A channel is paired
    /// with the map's wheel of the same name, else with the wheel in the
    /// same position; a band takes the range covering most of it. An
    /// assignment of `""` blanks a slot the map would otherwise fill.
    pub fn assign(
        &self,
        f: &mut Fixture,
        map: Option<&WheelMap>,
        user: Option<&BTreeMap<String, String>>,
    ) {
        let wheel_channels: Vec<usize> = f
            .channels
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_gobo_wheel())
            .map(|(i, _)| i)
            .collect();
        if let Some(map) = map {
            let map_wheels: Vec<(&String, &Vec<WheelRange>)> = map.wheels.iter().collect();
            for (wi, &ci) in wheel_channels.iter().enumerate() {
                let ch = &mut f.channels[ci];
                let wanted = squash(&ch.name);
                let ranges = map_wheels
                    .iter()
                    .find(|(n, _)| squash(n) == wanted)
                    .or_else(|| map_wheels.get(wi))
                    .map(|(_, r)| *r);
                let Some(ranges) = ranges else { continue };
                for band in &mut ch.bands {
                    if band.kind != 'S' {
                        continue;
                    }
                    if let Some(key) = best_range(ranges, band.min, band.max) {
                        band.gobo = Some(key.clone());
                    }
                }
            }
        }
        if let Some(user) = user {
            for (slot, key) in user {
                let Some((ci, bi)) = parse_slot(slot) else { continue };
                if let Some(band) = f.channels.get_mut(ci).and_then(|c| c.bands.get_mut(bi)) {
                    band.gobo = (!key.is_empty()).then(|| key.clone());
                }
            }
        }
    }
}

/// The range covering at least half of `lo..=hi`, preferring the largest overlap.
fn best_range(ranges: &[WheelRange], lo: u8, hi: u8) -> Option<&String> {
    let width = hi as i32 - lo as i32 + 1;
    ranges
        .iter()
        .filter_map(|(rlo, rhi, key, _)| {
            let overlap = (hi.min(*rhi) as i32) - (lo.max(*rlo) as i32) + 1;
            (overlap * 2 >= width && overlap > 0).then_some((overlap, key))
        })
        .max_by_key(|(overlap, _)| *overlap)
        .map(|(_, key)| key)
}

fn last_segment(key: &str) -> String {
    key.rsplit('/').next().unwrap_or(key).to_string()
}

/// `"3/5"` → channel 3, band 5.
fn parse_slot(slot: &str) -> Option<(usize, usize)> {
    let (c, b) = slot.split_once('/')?;
    Some((c.parse().ok()?, b.parse().ok()?))
}

/// The slot key the user file stores an assignment under.
pub fn slot_key(channel: usize, band: usize) -> String {
    format!("{channel}/{band}")
}

/// PNG bytes → mask. A grey PNG is taken as is; anything else goes through
/// the same rule the pack was built with (dark opaque ink blocks light,
/// transparent or bright pixels pass it), so a photo of a projected gobo on
/// a dark wall works as well as a drawn one.
fn decode_mask(png: &[u8]) -> Option<Mask> {
    let img = image::load_from_memory(png).ok()?;
    let gray = match img {
        image::DynamicImage::ImageLuma8(g) => g,
        other => {
            let rgba = other.to_rgba8();
            image::GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| {
                let [r, g, b, a] = rgba.get_pixel(x, y).0;
                let luma = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0;
                let open = 1.0 - (a as f32 / 255.0) * (1.0 - luma);
                image::Luma([(open.clamp(0.0, 1.0) * 255.0) as u8])
            })
        }
    };
    let gray = if gray.width() == MASK_SIZE && gray.height() == MASK_SIZE {
        gray
    } else {
        image::imageops::resize(&gray, MASK_SIZE, MASK_SIZE, image::imageops::FilterType::Triangle)
    };
    Some(Mask { size: MASK_SIZE, data: gray.into_raw() })
}

// ------------------------------------------------------------ assignments

/// Slot assignments made in the Gobos window: fixture profile → slot
/// (`channel/band`) → gobo key, or `""` for "no gobo here".
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct UserGobos {
    #[serde(default)]
    pub by_profile: BTreeMap<String, BTreeMap<String, String>>,
}

impl UserGobos {
    pub fn load() -> Self {
        std::fs::read_to_string(USER_FILE)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(USER_FILE, text);
        }
    }

    /// Pin `key` to a slot (`Some("")` blanks it); `None` returns the slot
    /// to whatever the catalogue says.
    pub fn set(&mut self, profile: &str, channel: usize, band: usize, key: Option<&str>) {
        let slot = slot_key(channel, band);
        match key {
            Some(k) => {
                self.by_profile.entry(profile.to_string()).or_default().insert(slot, k.to_string());
            }
            None => {
                if let Some(slots) = self.by_profile.get_mut(profile) {
                    slots.remove(&slot);
                    if slots.is_empty() {
                        self.by_profile.remove(profile);
                    }
                }
            }
        }
    }

    /// Whether a slot has been pinned by hand (to a gobo or to none).
    pub fn is_pinned(&self, profile: &str, channel: usize, band: usize) -> bool {
        self.by_profile.get(profile).is_some_and(|slots| slots.contains_key(&slot_key(channel, band)))
    }
}

/// A slot the Gobos window is choosing a picture for.
#[derive(Debug, Clone)]
pub struct GoboPick {
    pub profile: String,
    pub channel: usize,
    pub band: usize,
    /// "Intimidator Spot 110 · Gobo · Gobo 3", for the picker's heading.
    pub title: String,
}

/// The key a fixture's assignments and wheel map are looked up by: its
/// profile (`lib:…`, `builtin:…`, or a ShowBuddy file).
pub fn profile_of(f: &Fixture) -> String {
    f.file.to_string_lossy().replace('\\', "/")
}

/// Which wheel map a patched fixture gets: the library entry's maker and
/// model for library fixtures, otherwise whatever model name the profile or
/// display name happens to contain.
pub fn map_for_fixture<'a>(cat: &'a Catalogue, library: &Library, f: &Fixture) -> Option<&'a WheelMap> {
    let profile = profile_of(f);
    match fixturedb::library_id(&profile) {
        Some(id) => library.find(id).and_then(|e| {
            cat.map_for(&e.manufacturer, &e.model)
                .or_else(|| cat.map_for_hint(&format!("{} {}", e.manufacturer, e.model)))
        }),
        None => cat.map_for_hint(&profile).or_else(|| cat.map_for_hint(&f.display)),
    }
}

/// Fill in the gobo slots of every fixture in the patch.
pub fn assign_patch(patch: &mut Patch, library: &Library, cat: &Catalogue, user: &UserGobos) {
    for f in &mut patch.fixtures {
        let map = map_for_fixture(cat, library, f).cloned();
        let profile = profile_of(f);
        cat.assign(f, map.as_ref(), user.by_profile.get(&profile));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::showbuddy::{Band, Channel, Role};

    fn wheel(name: &str, slots: &[(u8, u8, &str)]) -> Channel {
        Channel {
            name: name.into(),
            bands: slots
                .iter()
                .map(|(lo, hi, l)| Band { kind: 'S', min: *lo, max: *hi, label: (*l).into(), gobo: None })
                .collect(),
            role: Some(Role::Gobo),
        }
    }

    fn fixture(channels: Vec<Channel>) -> Fixture {
        Fixture {
            display: "Spot".into(),
            file: "lib:test/spot/8ch".into(),
            from: 1,
            to: channels.len() as u16,
            x: 0.5,
            y: 0.5,
            pan_range: 540.0,
            tilt_range: 270.0,
            beam_width: 20.0,
            channels,
        }
    }

    /// A wheel's ranges as `(min, max, gobo key)`.
    type Slots<'a> = &'a [(u8, u8, &'a str)];

    fn map(model: &str, wheels: &[(&str, Slots<'_>)]) -> WheelMap {
        WheelMap {
            manufacturer: "Chauvet".into(),
            model: model.into(),
            wheels: wheels
                .iter()
                .map(|(n, r)| {
                    (
                        n.to_string(),
                        r.iter().map(|(lo, hi, k)| (*lo, *hi, k.to_string(), String::new())).collect(),
                    )
                })
                .collect(),
        }
    }

    fn catalogue(maps: Vec<WheelMap>) -> Catalogue {
        let mut cat = Catalogue { maps, ..Default::default() };
        cat.finish();
        cat
    }

    /// Wheels and rotation channels are told apart by their bands first and
    /// their names second, so "Gobo wheel (rotating)" is still a wheel.
    #[test]
    fn wheels_and_rotation_channels_classify() {
        let w = wheel("Gobo wheel (rotating)", &[(0, 7, "Open"), (8, 15, "Gobo 1"), (16, 23, "Gobo 2")]);
        assert!(w.is_gobo_wheel() && !w.is_gobo_rotation());
        let rot = Channel { name: "Gobo rotation".into(), bands: Vec::new(), role: Some(Role::Gobo) };
        assert!(rot.is_gobo_rotation() && !rot.is_gobo_wheel());
        let ofl_rot = wheel("Gobo 1 Rot", &[(0, 127, "WheelRotation"), (128, 255, "WheelRotation")]);
        assert!(ofl_rot.is_gobo_rotation() && !ofl_rot.is_gobo_wheel());
        let color = Channel { name: "Color".into(), bands: Vec::new(), role: Some(Role::Color) };
        assert!(!color.is_gobo_wheel() && !color.is_gobo_rotation());
    }

    /// Bands take the map range that covers most of them, matched by channel
    /// name, and a user assignment overrides or blanks the result.
    #[test]
    fn assignment_joins_by_name_and_range() {
        let cat = catalogue(vec![map(
            "Intimidator Spot 110",
            &[("Gobo", &[(8, 15, "qlc/Chauvet/gobo00079"), (16, 23, "qlc/GLP/gobo00016")])],
        )]);
        let mut f = fixture(vec![
            Channel { name: "Pan".into(), bands: Vec::new(), role: Some(Role::Pan) },
            wheel("Gobo", &[(0, 7, "Open"), (8, 15, "Laser"), (16, 23, "Star"), (24, 31, "Shake 1")]),
        ]);
        let map = cat.map_for("Chauvet DJ", "Intimidator Spot 110").expect("maker-tolerant match");
        cat.assign(&mut f, Some(map), None);
        let bands = &f.channels[1].bands;
        assert_eq!(bands[0].gobo, None);
        assert_eq!(bands[1].gobo.as_deref(), Some("qlc/Chauvet/gobo00079"));
        assert_eq!(bands[2].gobo.as_deref(), Some("qlc/GLP/gobo00016"));
        assert_eq!(bands[3].gobo, None);

        let mut user = BTreeMap::new();
        user.insert(slot_key(1, 3), "user/mine".to_string());
        user.insert(slot_key(1, 1), String::new());
        cat.assign(&mut f, Some(map), Some(&user));
        let bands = &f.channels[1].bands;
        assert_eq!(bands[1].gobo, None, "a blank assignment wins over the map");
        assert_eq!(bands[3].gobo.as_deref(), Some("user/mine"));
    }

    /// A map whose channel names don't match pairs wheels by position.
    #[test]
    fn assignment_falls_back_to_wheel_order() {
        let cat = catalogue(vec![map(
            "Spot 250",
            &[("Gobo Wheel", &[(10, 19, "a")]), ("Rotating gobos", &[(10, 19, "b")])],
        )]);
        let mut f = fixture(vec![
            wheel("Gobo wheel (static)", &[(0, 9, "Open"), (10, 19, "Gobo 1")]),
            wheel("Gobo wheel (rotating)", &[(0, 9, "Open"), (10, 19, "Gobo 1")]),
        ]);
        let map = cat.map_for_hint("builtin:Spot 250 (16ch)").expect("hint match");
        cat.assign(&mut f, Some(map), None);
        // BTreeMap order: "Gobo Wheel" then "Rotating gobos".
        assert_eq!(f.channels[0].bands[1].gobo.as_deref(), Some("a"));
        assert_eq!(f.channels[1].bands[1].gobo.as_deref(), Some("b"));
    }

    /// Model matching never lets a short name like "Spot" match everything.
    #[test]
    fn hint_matching_needs_a_real_model_name() {
        let cat = catalogue(vec![map("Spot", &[("Gobo", &[(0, 9, "a")])])]);
        assert!(cat.map_for_hint("builtin:Some Spot (8ch)").is_none());
        assert!(cat.map_for("", "Spot").is_some(), "an exact model still matches");
    }

    /// The bundled pack has to be there, unpack, carry masks for the wheel
    /// map's every key, and decode to the size the GPU expects.
    #[test]
    fn bundled_pack_loads() {
        let Ok(file) = std::fs::File::open(PACK_FILE) else {
            eprintln!("no {PACK_FILE} here — skipping");
            return;
        };
        let cat = Catalogue::from_pack(file).expect("pack parses");
        assert!(cat.gobos.len() > 900, "only {} gobos", cat.gobos.len());
        assert!(cat.maps.len() > 150, "only {} wheel maps", cat.maps.len());
        for m in &cat.maps {
            for ranges in m.wheels.values() {
                for (_, _, key, _) in ranges {
                    assert!(cat.get(key).is_some(), "{}/{}: unknown gobo {key}", m.manufacturer, m.model);
                }
            }
        }
        let first = &cat.gobos[0];
        let mask = cat.mask(&first.key).expect("decodes");
        assert_eq!(mask.size, MASK_SIZE);
        assert_eq!(mask.data.len(), (MASK_SIZE * MASK_SIZE) as usize);
        assert!(mask.data.iter().any(|&v| v > 200) && mask.data.iter().any(|&v| v < 50));
        assert!(!cat.search("star", None, 10).is_empty());
    }
}
