//! Phasers — DMXpress's renamed grandMA3 *phaser/MAtricks* effects.
//!
//! A phaser arms the existing per-channel oscillator across a whole selection
//! at once, fanning the phase along the fixtures (the *spread*, MA's MAtricks)
//! so a wave rolls down the rig. It targets a single feature (Dimmer, Color, …)
//! and oscillates each matching channel around whatever base value it holds, so
//! you set a level first and the phaser swings around it.

use serde::{Deserialize, Serialize};

use crate::palette::Feature;
use crate::showbuddy::Role;

const PHASERS_FILE: &str = "phasers.json";

/// Narrows a phaser to specific channels *within* its feature, so e.g. a
/// Color phaser can pulse only the Red channels, or a Position phaser only
/// the Tilts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChannelFilter {
    #[default]
    All,
    Red,
    Green,
    Blue,
    White,
    Pan,
    Tilt,
    /// Channels named "Fan" (fogger output).
    Fan,
    /// Channels named "Heat" (fogger heater).
    Heat,
}

impl ChannelFilter {
    pub fn label(self) -> &'static str {
        match self {
            ChannelFilter::All => "All",
            ChannelFilter::Red => "Red",
            ChannelFilter::Green => "Green",
            ChannelFilter::Blue => "Blue",
            ChannelFilter::White => "White",
            ChannelFilter::Pan => "Pan",
            ChannelFilter::Tilt => "Tilt",
            ChannelFilter::Fan => "Fan",
            ChannelFilter::Heat => "Heat",
        }
    }

    /// Whether a channel (by role, with its name for role-less channels like
    /// a fogger's Fan/Heat) passes this filter.
    pub fn matches(self, role: Role, name: &str) -> bool {
        match self {
            ChannelFilter::All => true,
            ChannelFilter::Red => role == Role::Red,
            ChannelFilter::Green => role == Role::Green,
            ChannelFilter::Blue => role == Role::Blue,
            ChannelFilter::White => role == Role::White,
            ChannelFilter::Pan => matches!(role, Role::Pan | Role::PanFine),
            ChannelFilter::Tilt => matches!(role, Role::Tilt | Role::TiltFine),
            ChannelFilter::Fan => name.eq_ignore_ascii_case("fan"),
            ChannelFilter::Heat => name.eq_ignore_ascii_case("heat"),
        }
    }
}

/// What a phaser does to its channels while running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PhaserMode {
    /// Oscillate around each channel's base value.
    #[default]
    Wave,
    /// Add (or subtract, when inverted) a flat level until stopped.
    Add,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ComponentMode {
    /// Drive only a fixed channel value while the tile is active.
    Static,
    /// Oscillate around the channel's existing programmer value.
    #[default]
    Oscillation,
    /// First set the fixed value, then oscillate around it.
    StaticThenOscillation,
}

impl ComponentMode {
    pub const ALL: [Self; 3] = [Self::Oscillation, Self::StaticThenOscillation, Self::Static];

    pub fn label(self) -> &'static str {
        match self {
            Self::Static => "Static position",
            Self::Oscillation => "Oscillation",
            Self::StaticThenOscillation => "Static + oscillation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaserComponent {
    /// Role tag (PAN, TILT, DIM…) or exact unclassified channel name.
    pub target: String,
    #[serde(default)]
    pub mode: ComponentMode,
    #[serde(default = "default_mid_value")]
    pub static_value: u8,
    #[serde(default = "default_amount")]
    pub amount: f32,
    #[serde(default = "default_shape")]
    pub shape: f32,
    #[serde(default = "default_subdiv")]
    pub subdiv: Option<f32>,
    #[serde(default)]
    pub phase: f32,
    #[serde(default)]
    pub invert: bool,
    /// Reference into the global custom waveform pool; None uses Shape.
    #[serde(default)]
    pub waveform_id: Option<u32>,
}

impl PhaserComponent {
    pub fn for_target(target: String) -> Self {
        Self {
            target,
            mode: ComponentMode::Oscillation,
            static_value: default_mid_value(),
            amount: default_amount(),
            shape: default_shape(),
            subdiv: default_subdiv(),
            phase: 0.0,
            invert: false,
            waveform_id: None,
        }
    }

    pub fn matches(&self, role: Role, name: &str) -> bool {
        let tag = role.tag();
        if tag.is_empty() {
            self.target.eq_ignore_ascii_case(name)
        } else {
            self.target.eq_ignore_ascii_case(tag)
        }
    }
}

fn default_mid_value() -> u8 { 128 }
fn default_amount() -> f32 { 0.5 }
fn default_shape() -> f32 { 0.5 }
fn default_subdiv() -> Option<f32> { Some(4.0) }

/// A reusable, named effect definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Phaser {
    pub name: String,
    pub feature: Feature,
    /// Only touch channels matching this filter within the feature.
    #[serde(default)]
    pub filter: ChannelFilter,
    /// Direct channel-type targets: role tags ("RED", "TILT") or, for
    /// unclassified channels, names ("Fan") — the same grouping as the
    /// collective channel list. When non-empty this overrides
    /// `feature`/`filter`: a channel is touched if *any* target matches.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Composable channel layers. When non-empty these replace the legacy
    /// single target/config while preserving old saved phasers unchanged.
    #[serde(default)]
    pub components: Vec<PhaserComponent>,
    /// What the phaser does: oscillate, or add a flat level until stopped.
    #[serde(default)]
    pub mode: PhaserMode,
    /// Fixtures this phaser falls back to when nothing is selected
    /// (`display@from` keys). Recorded from the selection when the phaser is
    /// stored; Re-bind in the tile menu re-binds it. A live selection always wins.
    #[serde(default)]
    pub fixtures: Vec<String>,
    /// Pool-tile colour (bright when running, darkened when idle).
    #[serde(default = "default_color")]
    pub color: [u8; 3],
    /// Oscillation depth 0..1 (of full range, around each base value).
    pub amount: f32,
    /// Waveform morph 0..1 (triangle → sine → square).
    pub shape: f32,
    /// Beat subdivision; `None` = free-run from `speed`.
    pub subdiv: Option<f32>,
    /// Whether beat-synced oscillators follow taps and the master BPM.
    #[serde(default = "default_true")]
    pub master_beat: bool,
    /// Free-run speed 0..1 (used when `subdiv` is `None`).
    pub speed: f32,
    pub invert: bool,
    /// Total phase fanned across the selection, in cycles (1.0 = one full wrap).
    pub spread: f32,
    /// Mirror the spread into this many symmetric wings (1 = no wings).
    pub wings: u32,
    /// Static pose: per-fixture channel values (fixture key → (channel index,
    /// value)). When non-empty this phaser *stops* movement and recalls the
    /// pose instead of oscillating.
    #[serde(default)]
    pub static_pos: Vec<(String, Vec<(usize, u8)>)>,
    /// Hold: per-fixture channel values forced onto the output every frame
    /// while active — they override presets, blackouts and the grand master
    /// until the tile is clicked off (e.g. keep a smoke machine running).
    #[serde(default)]
    pub hold: Vec<(String, Vec<(usize, u8)>)>,
}

impl Phaser {
    /// Whether this phaser touches a channel: the channel-type targets when
    /// any are set, else the legacy feature + filter pair. Targets match the
    /// way the collective channel list groups: by role tag, or by name for
    /// unclassified channels.
    pub fn matches_channel(&self, role: Role, name: &str) -> bool {
        if self.targets.is_empty() {
            return Feature::of(role) == self.feature && self.filter.matches(role, name);
        }
        let tag = role.tag();
        self.targets.iter().any(|t| {
            if tag.is_empty() {
                t.eq_ignore_ascii_case(name)
            } else {
                t.eq_ignore_ascii_case(tag)
            }
        })
    }

    /// Whether the phaser is movement-flavoured (for pool column placement).
    pub fn is_movement(&self) -> bool {
        if !self.components.is_empty() {
            self.components.iter().all(|component| {
                matches!(
                    component.target.to_ascii_uppercase().as_str(),
                    "PAN" | "PANF" | "TILT" | "TILTF"
                )
            })
        } else if self.targets.is_empty() {
            self.feature == Feature::Position
        } else {
            self.targets.iter().all(|t| {
                matches!(
                    t.to_ascii_uppercase().as_str(),
                    "PAN" | "PANF" | "TILT" | "TILTF"
                )
            })
        }
    }
}

fn default_color() -> [u8; 3] {
    [110, 120, 150]
}

fn default_true() -> bool {
    true
}

impl Default for Phaser {
    fn default() -> Self {
        Self {
            name: "Phaser".into(),
            feature: Feature::Dimmer,
            filter: ChannelFilter::All,
            targets: Vec::new(),
            components: Vec::new(),
            mode: PhaserMode::Wave,
            fixtures: Vec::new(),
            color: default_color(),
            amount: 0.5,
            shape: 0.5,
            subdiv: Some(4.0),
            master_beat: true,
            speed: 0.357,
            invert: false,
            spread: 1.0,
            wings: 1,
            static_pos: Vec::new(),
            hold: Vec::new(),
        }
    }
}

/// Phase offset for fixture `k` of `n`, fanning `spread` across the selection
/// and mirroring it into `wings` symmetric groups.
pub fn spread_phase(k: usize, n: usize, spread: f32, wings: usize) -> f32 {
    if n <= 1 {
        return 0.0;
    }
    let w = wings.max(1);
    let per = n.div_ceil(w).max(1);
    let wi = k / per;
    let mut pos = (k % per) as f32 / per as f32;
    if wi % 2 == 1 {
        pos = 1.0 - pos; // mirror alternate wings
    }
    pos * spread
}

/// A handful of ready-to-use phasers, seeded on first run.
pub fn default_phasers() -> Vec<Phaser> {
    vec![
        Phaser {
            name: "Dimmer Chase".into(),
            feature: Feature::Dimmer,
            filter: ChannelFilter::All,
            color: [235, 200, 90],
            amount: 0.5,
            shape: 0.4,
            subdiv: Some(4.0),
            ..Default::default()
        },
        Phaser {
            name: "Dimmer Wings".into(),
            feature: Feature::Dimmer,
            filter: ChannelFilter::All,
            color: [235, 160, 70],
            amount: 0.5,
            shape: 0.4,
            subdiv: Some(4.0),
            wings: 2,
            ..Default::default()
        },
        Phaser {
            name: "Color Rainbow".into(),
            feature: Feature::Color,
            filter: ChannelFilter::All,
            color: [170, 90, 220],
            amount: 0.5,
            shape: 0.5,
            subdiv: Some(8.0),
            speed: 0.2,
            ..Default::default()
        },
        Phaser {
            name: "Red Pulse".into(),
            feature: Feature::Color,
            filter: ChannelFilter::Red,
            color: [225, 55, 55],
            amount: 1.0,
            shape: 1.0,
            subdiv: Some(2.0),
            spread: 0.0,
            ..Default::default()
        },
        Phaser {
            name: "Blue Pulse".into(),
            feature: Feature::Color,
            filter: ChannelFilter::Blue,
            color: [60, 100, 235],
            amount: 1.0,
            shape: 1.0,
            subdiv: Some(2.0),
            spread: 0.0,
            ..Default::default()
        },
        Phaser {
            name: "Pan Sweep".into(),
            feature: Feature::Position,
            filter: ChannelFilter::Pan,
            color: [70, 190, 190],
            amount: 0.3,
            shape: 0.5,
            subdiv: Some(8.0),
            speed: 0.2,
            spread: 0.5,
            ..Default::default()
        },
        Phaser {
            name: "Tilt Wave".into(),
            feature: Feature::Position,
            filter: ChannelFilter::Tilt,
            color: [80, 200, 120],
            amount: 0.3,
            shape: 0.5,
            subdiv: Some(8.0),
            speed: 0.2,
            spread: 0.5,
            ..Default::default()
        },
        Phaser {
            name: "Circle".into(),
            feature: Feature::Position,
            filter: ChannelFilter::All,
            color: [210, 130, 220],
            amount: 0.25,
            shape: 0.5,
            subdiv: Some(8.0),
            speed: 0.2,
            spread: 0.25,
            ..Default::default()
        },
    ]
}

/// Load saved phasers, seeding the defaults the first time (no file yet).
pub fn load_phasers() -> Vec<Phaser> {
    let mut phasers: Vec<Phaser> = match std::fs::read_to_string(PHASERS_FILE) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|_| default_phasers()),
        Err(_) => default_phasers(),
    };
    // Heal tiles saved before the editor stopped leaking pose/hold snapshots
    // into stored wave/add phasers: chip-built tiles (non-empty targets)
    // never legitimately carry one, and real poses/FX are only ever stored
    // as Position or Beam.
    for p in &mut phasers {
        if !p.targets.is_empty() {
            p.static_pos.clear();
            p.hold.clear();
        } else if !matches!(p.feature, Feature::Position | Feature::Beam) {
            p.static_pos.clear();
        }
    }
    phasers
}

/// Persist phasers to disk (best-effort).
pub fn save_phasers(phasers: &[Phaser]) {
    if let Ok(json) = serde_json::to_string_pretty(phasers) {
        let _ = std::fs::write(PHASERS_FILE, json);
    }
}

#[cfg(test)]
mod tests {
    use super::spread_phase;

    #[test]
    fn spread_assigns_one_phase_per_effect_unit() {
        assert_eq!(spread_phase(0, 3, 1.0, 1), 0.0);
        assert!((spread_phase(1, 3, 1.0, 1) - 1.0 / 3.0).abs() < 0.001);
        assert!((spread_phase(2, 3, 1.0, 1) - 2.0 / 3.0).abs() < 0.001);
    }
}
