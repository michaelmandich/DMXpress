//! Import a ShowBuddy Active DMX patch: fixture addresses, stage positions,
//! and per-channel definitions from the referenced .dmx library files.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_CONFIG: &str =
    "/Library/Application Support/db audioware/Show Buddy Active/Presets/DmxConfig.xml";

pub const PRESETS_DIR: &str =
    "/Library/Application Support/db audioware/Show Buddy Active/Presets";

/// Local copy of the last ShowBuddy patch DMXpress managed to read. ShowBuddy
/// lives at a fixed absolute macOS path outside this repository, so without a
/// cache a show built on top of it collapses to just the DMXpress-patched
/// fixtures on any other machine (a fresh clone, Windows, a different Mac).
pub const CACHE_FILE: &str = "showbuddy_cache.json";

/// Local copy of the ShowBuddy preset banks, including each .prt's parsed
/// contents. The banks themselves are just paths into ShowBuddy's Presets
/// folder, so without the parsed data a show has no presets at all away from
/// the machine ShowBuddy is installed on.
pub const PRESET_CACHE_FILE: &str = "showbuddy_presets.json";

/// What a channel most likely controls, inferred from its name/type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Dimmer,
    Red,
    Green,
    Blue,
    White,
    /// Color wheel / color-macro slider (non-RGB color control).
    Color,
    Strobe,
    Pan,
    PanFine,
    Tilt,
    TiltFine,
    Zoom,
    /// Movement / effect speed.
    Speed,
    Other,
}

impl Role {
    /// Short canonical tag for UI badges ("" for unclassified).
    pub fn tag(self) -> &'static str {
        match self {
            Role::Dimmer => "DIM",
            Role::Red => "RED",
            Role::Green => "GRN",
            Role::Blue => "BLU",
            Role::White => "WHT",
            Role::Color => "COL",
            Role::Strobe => "STRB",
            Role::Pan => "PAN",
            Role::PanFine => "PANf",
            Role::Tilt => "TILT",
            Role::TiltFine => "TILTf",
            Role::Zoom => "ZOOM",
            Role::Speed => "SPD",
            Role::Other => "",
        }
    }
}

/// One value range within a channel, e.g. `S,20,39,Red`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Band {
    /// D = dimmer, V = continuous value, S = switched/stepped range.
    pub kind: char,
    pub min: u8,
    pub max: u8,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub name: String,
    pub bands: Vec<Band>,
}

impl Channel {
    pub fn role(&self) -> Role {
        let n = self.name.to_lowercase();
        // "Pan"/"Pan fine" pairs sometimes share a name and mark fine via the
        // first band label (e.g. `V,0,255,Fine`); "Panf"/"Tiltf" also occur.
        let fine = n.contains("fine")
            || ((n.starts_with("pan") || n.starts_with("tilt")) && n.ends_with('f'))
            || self
                .bands
                .first()
                .is_some_and(|b| b.label.to_lowercase().contains("fine"));
        // Strobe before tilt: "Til strobe", "STRB", "Stbbm", "YSTB"...
        if n.contains("strobe") || n.contains("strb") || n.contains("stb") {
            return Role::Strobe;
        }
        // Speed before pan/tilt: "Tiltspd", "Movement speed", "ClrSpd"...
        if n.contains("speed") || n.contains("spd") {
            return Role::Speed;
        }
        if n.contains("pan") {
            return if fine { Role::PanFine } else { Role::Pan };
        }
        if n.contains("tilt") {
            return if fine { Role::TiltFine } else { Role::Tilt };
        }
        if n.contains("zoom") {
            return Role::Zoom;
        }
        if n.contains("red") {
            Role::Red
        } else if n.contains("grn") || n.contains("green") {
            Role::Green
        } else if n.contains("blu") {
            Role::Blue
        } else if n.contains("whit") || n.contains("wht") {
            Role::White
        } else if n.contains("color") || n.contains("colour") || n.contains("clr") {
            Role::Color
        } else if n.contains("dim")
            || n.contains("master")
            || self.bands.first().is_some_and(|b| b.kind == 'D')
        {
            Role::Dimmer
        } else {
            Role::Other
        }
    }

    /// Label of the band the value currently falls in, if any.
    pub fn band_label(&self, v: u8) -> Option<&str> {
        self.bands
            .iter()
            .find(|b| v >= b.min && v <= b.max && !b.label.is_empty())
            .map(|b| b.label.as_str())
    }

    /// Usable dimming sub-range of a dimmer channel that also carries
    /// strobe/macro bands (e.g. the moving par's `8–134 dimming` next to
    /// `135–239 Strobes`). Logical 0–255 is compressed into this range at
    /// output so a dimmer sweep never wanders into the strobe section.
    pub fn dim_range(&self) -> Option<(u8, u8)> {
        if self.role() != Role::Dimmer {
            return None;
        }
        let b = self
            .bands
            .iter()
            .find(|b| b.label.to_lowercase().contains("dim"))?;
        if b.min == 0 && b.max == 255 {
            return None;
        }
        Some((b.min, b.max))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub display: String,
    pub file: PathBuf,
    /// 1-based absolute DMX start address (may exceed 512 for universe 2+).
    pub from: u16,
    pub to: u16,
    /// Normalized stage position from ShowBuddy (0..1).
    pub x: f32,
    pub y: f32,
    /// Movement geometry from the ShowBuddy <DmxHead> (degrees).
    pub pan_range: f32,
    pub tilt_range: f32,
    pub beam_width: f32,
    pub channels: Vec<Channel>,
}

impl Fixture {
    pub fn channel_count(&self) -> usize {
        (self.to.saturating_sub(self.from) + 1) as usize
    }
}

#[derive(Debug, Clone, Default)]
pub struct Patch {
    pub fixtures: Vec<Fixture>,
    pub warnings: Vec<String>,
}

pub fn load_default() -> Result<Patch> {
    load(Path::new(DEFAULT_CONFIG))
}

/// Fixtures from the last successful ShowBuddy import, if one was cached.
pub fn load_cache() -> Vec<Fixture> {
    std::fs::read_to_string(CACHE_FILE)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<Fixture>>(&text).ok())
        .unwrap_or_default()
}

/// Remember the ShowBuddy fixtures so the rig survives on a machine that
/// cannot reach ShowBuddy itself.
pub fn save_cache(fixtures: &[Fixture]) {
    if fixtures.is_empty() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(fixtures) {
        let _ = std::fs::write(CACHE_FILE, json);
    }
}

pub fn load(config: &Path) -> Result<Patch> {
    let xml = std::fs::read_to_string(config)
        .with_context(|| format!("reading {}", config.display()))?;
    let mut patch = Patch::default();

    for chunk in xml.split("<DmxFixture").skip(1) {
        let chunk = chunk.split("</DmxFixture>").next().unwrap_or(chunk);
        let (Some(name), Some(disp), Some(from), Some(to)) = (
            attr(chunk, "name"),
            attr(chunk, "disp"),
            attr(chunk, "from"),
            attr(chunk, "to"),
        ) else {
            patch.warnings.push("Skipped malformed <DmxFixture> entry".into());
            continue;
        };
        let (Ok(from), Ok(to)) = (from.parse::<u16>(), to.parse::<u16>()) else {
            patch
                .warnings
                .push(format!("Skipped fixture '{disp}': bad address range"));
            continue;
        };

        // Stage position from the nested <DmxHead>.
        let head = chunk.split("<DmxHead").nth(1).unwrap_or("");
        let x = attr(head, "xpos").and_then(|v| v.parse().ok()).unwrap_or(0.5);
        let y = attr(head, "ypos").and_then(|v| v.parse().ok()).unwrap_or(0.5);
        let pan_range = attr(head, "panRange")
            .and_then(|v| v.parse().ok())
            .unwrap_or(540.0);
        let tilt_range = attr(head, "tiltRange")
            .and_then(|v| v.parse().ok())
            .unwrap_or(170.0);
        let beam_width = attr(head, "beamWidth")
            .and_then(|v| v.parse().ok())
            .unwrap_or(30.0);

        // ShowBuddy writes paths with doubled slashes; harmless on macOS.
        let file = PathBuf::from(name);
        let span = (to.saturating_sub(from) + 1) as usize;
        let mut channels = match parse_fixture_file(&file) {
            Ok(ch) => ch,
            Err(e) => {
                patch
                    .warnings
                    .push(format!("'{disp}': {e:#}; using generic channels"));
                Vec::new()
            }
        };
        // Reconcile with the patched span.
        while channels.len() < span {
            channels.push(Channel {
                name: format!("Ch {}", channels.len() + 1),
                bands: Vec::new(),
            });
        }
        channels.truncate(span);

        patch.fixtures.push(Fixture {
            display: disp.to_string(),
            file,
            from,
            to,
            x,
            y,
            pan_range,
            tilt_range,
            beam_width,
            channels,
        });
    }

    Ok(patch)
}

/// Parse a ShowBuddy .dmx fixture library file: alternating channel-name
/// lines and one or more `KIND,min,max[,label]` band lines.
fn parse_fixture_file(path: &Path) -> Result<Vec<Channel>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let mut channels: Vec<Channel> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(band) = parse_band(line) {
            if let Some(ch) = channels.last_mut() {
                ch.bands.push(band);
            }
        } else {
            channels.push(Channel {
                name: line.to_string(),
                bands: Vec::new(),
            });
        }
    }
    Ok(channels)
}

fn parse_band(line: &str) -> Option<Band> {
    let mut parts = line.splitn(4, ',');
    let kind = parts.next()?.trim();
    if !matches!(kind, "D" | "V" | "S") {
        return None;
    }
    let min = leading_u8(parts.next()?)?;
    let max = leading_u8(parts.next()?)?;
    let label = parts.next().unwrap_or("").trim().to_string();
    Some(Band {
        kind: kind.chars().next().unwrap(),
        min,
        max,
        label,
    })
}

/// Parse leading digits, tolerating trailing junk like `255 ?`.
fn leading_u8(s: &str) -> Option<u8> {
    let digits: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Extract `name="value"` from a tag snippet. Matches ` name="` to avoid
/// suffix collisions between attribute names.
fn attr<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!(" {name}=\"");
    let i = s.find(&pat)? + pat.len();
    let rest = &s[i..];
    Some(&rest[..rest.find('"')?])
}

// ---------------------------------------------------------------- presets

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetRef {
    pub name: String,
    pub path: PathBuf,
    /// The parsed .prt, kept inline so the preset still recalls when the file
    /// itself is out of reach. Filled in by [`hydrate_presets`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<PresetData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetBank {
    pub name: String,
    pub order: i32,
    pub presets: Vec<PresetRef>,
}

/// Scan the ShowBuddy Presets folder: each subdirectory is a bank of .prt
/// files, ordered by its bank.xml where present.
pub fn load_preset_banks() -> Result<Vec<PresetBank>> {
    let root = Path::new(PRESETS_DIR);
    let mut banks = Vec::new();
    for entry in
        std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?
    {
        let Ok(entry) = entry else { continue };
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let bank_xml = std::fs::read_to_string(dir.join("bank.xml")).unwrap_or_default();
        let ordered: Vec<String> = bank_xml
            .split("<Preset")
            .skip(1)
            .filter_map(|c| attr(c, "name").map(str::to_string))
            .collect();
        let order = bank_xml
            .split("<BankOrder")
            .nth(1)
            .and_then(|c| attr(c, "val"))
            .and_then(|v| v.parse::<f32>().ok())
            .map(|v| v as i32)
            .unwrap_or(i32::MAX);

        let mut presets: Vec<PresetRef> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                (p.extension().is_some_and(|x| x == "prt")).then(|| PresetRef {
                    name: p.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
                    path: p,
                    data: None,
                })
            })
            .collect();
        presets.sort_by(|a, b| {
            let ia = ordered.iter().position(|n| *n == a.name).unwrap_or(usize::MAX);
            let ib = ordered.iter().position(|n| *n == b.name).unwrap_or(usize::MAX);
            ia.cmp(&ib).then_with(|| a.name.cmp(&b.name))
        });
        if !presets.is_empty() {
            banks.push(PresetBank {
                name: entry.file_name().to_string_lossy().trim().to_string(),
                order,
                presets,
            });
        }
    }
    banks.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(&b.name)));
    Ok(banks)
}

/// Parse every .prt in `banks` into its [`PresetRef::data`], so the banks no
/// longer depend on the files staying where ShowBuddy put them. Returns how
/// many presets were read.
pub fn hydrate_presets(banks: &mut [PresetBank]) -> usize {
    let mut n = 0;
    for bank in banks.iter_mut() {
        for p in &mut bank.presets {
            if p.data.is_some() {
                continue;
            }
            if let Ok(data) = load_preset(&p.path) {
                p.data = Some(data);
                n += 1;
            }
        }
    }
    n
}

/// Preset banks from the last successful ShowBuddy import, if cached.
pub fn load_preset_cache() -> Vec<PresetBank> {
    std::fs::read_to_string(PRESET_CACHE_FILE)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<PresetBank>>(&text).ok())
        .unwrap_or_default()
}

/// Remember the preset banks so they still recall on a machine that cannot
/// reach ShowBuddy.
pub fn save_preset_cache(banks: &[PresetBank]) {
    if banks.is_empty() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(banks) {
        let _ = std::fs::write(PRESET_CACHE_FILE, json);
    }
}

/// Parse a .prt preset into (1-based DMX address, value) pairs. Numeric
/// param names are channel numbers; named UI params (Bank, Preset...) are
/// skipped. Values are normalized 0..1 in the file.
pub fn load_preset(path: &Path) -> Result<PresetData> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let xml = String::from_utf8_lossy(&bytes);
    let xml: &str = &xml;
    let mut data = PresetData::default();
    let named_f32 = |name: &str, default: f32| -> f32 {
        xml.split("<Param")
            .skip(1)
            .find(|c| attr(c, "nm") == Some(name))
            .and_then(|c| attr(c, "v"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    data.master_speed = named_f32("Master Speed", 1.0);
    data.speed = named_f32("Speed", 0.357);
    data.shape = named_f32("Shape", 0.5);
    data.tempo = named_f32("MasterTempo", 120.0);
    let global_amount = named_f32("Amount", 0.0);

    for chunk in xml.split("<Param").skip(1) {
        let Some(addr) = attr(chunk, "nm").and_then(|n| n.parse::<u16>().ok()) else {
            continue;
        };
        let Some(v) = attr(chunk, "v").and_then(|v| v.parse::<f32>().ok()) else {
            continue;
        };
        if (1..=1024).contains(&addr) {
            data.values.push((addr, (v.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
    }

    // Oscillator assignments: <c n="ch0" t="type" a="amount" p="phase" s="subdiv"/>
    for chunk in xml.split("<c ").skip(1) {
        let chunk = chunk.split("/>").next().unwrap_or(chunk);
        let Some(n) = attr_here(chunk, "n").and_then(|v| v.parse::<u16>().ok()) else {
            continue;
        };
        if n >= 1024 {
            continue;
        }
        let amount = attr_here(chunk, "a")
            .and_then(|v| v.parse().ok())
            .unwrap_or(global_amount);
        if amount <= 0.0 {
            continue;
        }
        data.mods.push(PresetMod {
            addr: n + 1,
            amount,
            phase: attr_here(chunk, "p").and_then(|v| v.parse().ok()).unwrap_or(0.0),
            subdiv: attr_here(chunk, "s").and_then(|v| v.parse().ok()),
            invert: attr_here(chunk, "t").and_then(|v| v.parse::<i32>().ok()) == Some(2),
        });
    }
    Ok(data)
}

/// A loaded .prt preset: static channel snapshot plus oscillator assignments.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresetData {
    /// 1-based DMX address -> base value.
    pub values: Vec<(u16, u8)>,
    pub mods: Vec<PresetMod>,
    pub master_speed: f32,
    /// Global FX speed 0..1.
    pub speed: f32,
    /// Global waveform morph 0..1 (sine -> square).
    pub shape: f32,
    /// BPM for beat-synced channels.
    pub tempo: f32,
}

impl PresetData {
    /// Build the static base frame: blackout with each stored channel value
    /// written at its 1-based address.
    pub fn base_frame(&self) -> crate::net::Frame {
        let mut f = crate::net::Frame::black();
        for &(addr, v) in &self.values {
            if (1..=crate::net::DMX_SLOTS as u16).contains(&addr) {
                f[addr as usize - 1] = v;
            }
        }
        f
    }
}

/// One channel's oscillation within a preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetMod {
    /// 1-based DMX address.
    pub addr: u16,
    /// Modulation depth 0..1 (of full range, around the base value).
    pub amount: f32,
    /// Phase offset 0..1.
    pub phase: f32,
    /// Beat subdivision (`s` attr): cycle takes `s` beats at the preset tempo.
    pub subdiv: Option<f32>,
    pub invert: bool,
}

/// Like `attr` but for chunks that start inside the tag (no leading space
/// guaranteed before the first attribute).
fn attr_here<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    attr(s, name).or_else(|| {
        let pat = format!("{name}=\"");
        if s.starts_with(&pat) {
            let rest = &s[pat.len()..];
            Some(&rest[..rest.find('"')?])
        } else {
            None
        }
    })
}
