//! Palettes — DMXpress's renamed grandMA3 *referenced presets*.
//!
//! Where a ShowBuddy `.prt` is a whole-rig snapshot, a palette stores just one
//! *feature* (Color, Position, …) for the fixtures you had selected. Recalling
//! a palette drops those values into the programmer and records a *reference*
//! back to the palette, so a cue can remember "Color Palette 3" instead of raw
//! RGB — edit the palette and everything that points at it follows.

use serde::{Deserialize, Serialize};

use crate::showbuddy::Role;

const PALETTES_FILE: &str = "palettes.json";

/// The attribute family a palette (or channel) belongs to. Channels are sorted
/// into exactly one feature so palettes stay focused (a Color palette never
/// disturbs Position, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Feature {
    Dimmer,
    Position,
    Color,
    Beam,
    Focus,
    Control,
}

impl Feature {
    /// Pool order shown in the UI.
    pub const ALL: [Feature; 6] = [
        Feature::Dimmer,
        Feature::Position,
        Feature::Color,
        Feature::Beam,
        Feature::Focus,
        Feature::Control,
    ];

    /// Which feature a channel role contributes to.
    pub fn of(role: Role) -> Feature {
        match role {
            Role::Dimmer => Feature::Dimmer,
            Role::Red | Role::Green | Role::Blue | Role::White | Role::Color => Feature::Color,
            Role::Pan | Role::PanFine | Role::Tilt | Role::TiltFine => Feature::Position,
            Role::Zoom => Feature::Focus,
            Role::Strobe => Feature::Beam,
            Role::Speed | Role::Other => Feature::Control,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Feature::Dimmer => "Dimmer",
            Feature::Position => "Position",
            Feature::Color => "Color",
            Feature::Beam => "Beam",
            Feature::Focus => "Focus",
            Feature::Control => "Control",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Feature::Dimmer => "🔆",
            Feature::Position => "✛",
            Feature::Color => "🎨",
            Feature::Beam => "🔦",
            Feature::Focus => "🔍",
            Feature::Control => "⚙",
        }
    }
}

/// A stable pointer to a palette: the channels a cue or the programmer link to.
/// Carries the feature so the resolver can fall back gracefully if the palette
/// was deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteRef {
    pub feature: Feature,
    pub id: u32,
}

/// A named, feature-scoped set of channel values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Palette {
    /// Stable id (never reused) so references survive reordering/deletion.
    pub id: u32,
    pub feature: Feature,
    pub name: String,
    /// 0-based DMX index → value. Only channels belonging to `feature`.
    pub values: Vec<(usize, u8)>,
}

impl Palette {
    pub fn reference(&self) -> PaletteRef {
        PaletteRef {
            feature: self.feature,
            id: self.id,
        }
    }
}

/// Load saved palettes (empty if the file is missing or unreadable).
pub fn load_palettes() -> Vec<Palette> {
    std::fs::read_to_string(PALETTES_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist palettes to disk (best-effort).
pub fn save_palettes(palettes: &[Palette]) {
    if let Ok(json) = serde_json::to_string_pretty(palettes) {
        let _ = std::fs::write(PALETTES_FILE, json);
    }
}

const SEQUENCES_FILE: &str = "sequences.json";

/// How a palette sequence disperses the colour change across the rig.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SeqPattern {
    /// The change rolls across the fixtures in patch order.
    #[default]
    Wave,
    /// The change sweeps from both ends toward the middle.
    Wings,
    /// Each fixture gets a random offset (shimmering dispersal).
    Random,
}

impl SeqPattern {
    pub const ALL: [SeqPattern; 3] = [SeqPattern::Wave, SeqPattern::Wings, SeqPattern::Random];

    pub fn label(self) -> &'static str {
        match self {
            SeqPattern::Wave => "Wave",
            SeqPattern::Wings => "Wings",
            SeqPattern::Random => "Random",
        }
    }
}

/// A saved palette sequence: the colours plus how the cycle moves — speed,
/// spacing across the rig, dispersal pattern, and the shape of each change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaletteSeq {
    pub name: String,
    /// Folder name; empty = root of the pool.
    #[serde(default)]
    pub folder: String,
    /// Palette ids, in cycle order.
    pub ids: Vec<u32>,
    /// Relative width/dwell of each id. Missing entries default to 1.0 for
    /// backward compatibility. Repeated ids are intentional separate bands.
    #[serde(default)]
    pub weights: Vec<f32>,
    /// Beats per colour step.
    pub beats_per: f32,
    /// Follow taps and the master BPM; off keeps the launch-time tempo.
    #[serde(default = "default_true")]
    pub master_beat: bool,
    /// Phase fanned across the fixtures (0 = all together, 1 = the whole
    /// cycle spread over the rig).
    #[serde(default)]
    pub spread: f32,
    #[serde(default)]
    pub pattern: SeqPattern,
    /// Shape of each change: 0 = smooth crossfade, 1 = hard snap.
    #[serde(default)]
    pub shape: f32,
}

fn default_true() -> bool {
    true
}

/// Saved sequences plus their folder list.
#[derive(Default, Serialize, Deserialize)]
pub struct SeqStore {
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub seqs: Vec<PaletteSeq>,
}

/// Load saved palette sequences (empty if the file is missing/unreadable).
pub fn load_seqs() -> SeqStore {
    std::fs::read_to_string(SEQUENCES_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist palette sequences to disk (best-effort).
pub fn save_seqs(folders: &[String], seqs: &[PaletteSeq]) {
    let store = SeqStore {
        folders: folders.to_vec(),
        seqs: seqs.to_vec(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&store) {
        let _ = std::fs::write(SEQUENCES_FILE, json);
    }
}
