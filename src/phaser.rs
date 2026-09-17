//! Phasers — DMXpress's renamed grandMA3 *phaser/MAtricks* effects.
//!
//! A phaser arms the existing per-channel oscillator across a whole selection
//! at once, fanning the phase along the fixtures (the *spread*, MA's MAtricks)
//! so a wave rolls down the rig. It targets a single feature (Dimmer, Color, …)
//! and oscillates each matching channel around whatever base value it holds, so
//! you set a level first and the phaser swings around it.

use serde::{Deserialize, Serialize};

use crate::oscillator::wave;
use crate::palette::{hsv, Feature};
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
    Amber,
    Uv,
    Cyan,
    Magenta,
    Yellow,
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
            ChannelFilter::Amber => "Amber",
            ChannelFilter::Uv => "UV",
            ChannelFilter::Cyan => "Cyan",
            ChannelFilter::Magenta => "Magenta",
            ChannelFilter::Yellow => "Yellow",
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
            ChannelFilter::Amber => role == Role::Amber,
            ChannelFilter::Uv => role == Role::Uv,
            ChannelFilter::Cyan => role == Role::Cyan,
            ChannelFilter::Magenta => role == Role::Magenta,
            ChannelFilter::Yellow => role == Role::Yellow,
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

/// What the stage needs to draw a phaser's figure on the floor: the (pan,
/// tilt) samples from [`Phaser::path_points`], which fixtures to draw it
/// for, and the colour to draw it in.
pub struct PhaserTrace {
    pub points: Vec<(f32, f32)>,
    pub fixtures: Vec<usize>,
    pub color: [u8; 3],
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
    /// The pan/tilt figure this phaser's components were generated from, if
    /// any — a label for the pool and the path preview, not a second source
    /// of truth: the components are what runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathShape>,
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

    /// Whether this phaser drives both pan and tilt. The engine offsets the
    /// tilt a quarter cycle when it does — so pan+tilt trace a circle rather
    /// than a diagonal — and the path preview has to apply the same rule or
    /// it would draw one figure while the rig ran another.
    pub fn both_axes(&self) -> bool {
        let has = |s: &str| {
            if self.components.is_empty() {
                self.targets.iter().any(|t| t.eq_ignore_ascii_case(s))
            } else {
                self.components.iter().any(|c| c.target.eq_ignore_ascii_case(s))
            }
        };
        if self.targets.is_empty() && self.components.is_empty() {
            self.feature == Feature::Position && self.filter == ChannelFilter::All
        } else {
            (has("PAN") || has("PANf")) && (has("TILT") || has("TILTf"))
        }
    }

    /// The figure one fixture traces: `n` (pan, tilt) samples in -1..1 over a
    /// whole period, evaluated with the engine's own waveform and the same
    /// quarter-cycle tilt rule. Empty when nothing oscillates pan or tilt.
    pub fn path_points(&self, n: usize) -> Vec<(f32, f32)> {
        let axis = |target: &str| -> Option<char> {
            match target.to_ascii_uppercase().as_str() {
                "PAN" | "PANF" => Some('p'),
                "TILT" | "TILTF" => Some('t'),
                _ => None,
            }
        };
        // Legacy (component-less) phasers oscillate every matching channel
        // with the top-level settings; express that as two virtual components.
        let legacy: Vec<PhaserComponent>;
        let comps: &[PhaserComponent] = if self.components.is_empty() {
            if self.feature != Feature::Position || self.mode != PhaserMode::Wave {
                return Vec::new();
            }
            let mut v = Vec::new();
            for (t, ok) in [("PAN", matches!(self.filter, ChannelFilter::All | ChannelFilter::Pan)),
                            ("TILT", matches!(self.filter, ChannelFilter::All | ChannelFilter::Tilt))] {
                if ok {
                    v.push(PhaserComponent {
                        target: t.into(),
                        amount: self.amount,
                        shape: self.shape,
                        subdiv: self.subdiv,
                        phase: 0.0,
                        invert: self.invert,
                        ..PhaserComponent::for_target(t.into())
                    });
                }
            }
            legacy = v;
            &legacy
        } else {
            &self.components
        };
        let live: Vec<&PhaserComponent> = comps
            .iter()
            .filter(|c| c.mode != ComponentMode::Static && axis(&c.target).is_some())
            .collect();
        if live.is_empty() {
            return Vec::new();
        }
        let tilt_extra = if self.both_axes() { 0.25 } else { 0.0 };
        // One period: the smallest span every component completes whole
        // cycles in (free-run components count as one beat).
        let subdivs: Vec<f32> = live.iter().map(|c| c.subdiv.unwrap_or(1.0).max(0.01)).collect();
        let longest = subdivs.iter().cloned().fold(0.0f32, f32::max);
        let period = (1..=8)
            .map(|k| longest * k as f32)
            .find(|p| subdivs.iter().all(|s| ((p / s) - (p / s).round()).abs() < 1e-3))
            .unwrap_or(longest * 8.0);
        (0..n)
            .map(|i| {
                let beats = period * i as f32 / n as f32;
                let (mut pan, mut tilt) = (0.0, 0.0);
                for c in &live {
                    let is_tilt = axis(&c.target) == Some('t');
                    let x = beats / c.subdiv.unwrap_or(1.0).max(0.01)
                        + c.phase
                        + if is_tilt { tilt_extra } else { 0.0 };
                    let v = wave(x, c.shape) * c.amount * if c.invert { -1.0 } else { 1.0 };
                    if is_tilt { tilt += v } else { pan += v }
                }
                (pan.clamp(-1.0, 1.0), tilt.clamp(-1.0, 1.0))
            })
            .collect()
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

/// The pan/tilt figures other consoles ship as their position effects —
/// MagicQ's Position FX library (circles, squares, saws, lifts, zig-zags)
/// and QLC+'s EFX patterns (Circle, Eight, Line, Diamond, Square, Leaf,
/// Lissajous) cover the same ground — expressed here as pairs of oscillator
/// components on PAN and TILT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PathShape {
    Circle,
    /// Figure of eight, lobes side to side.
    Eight,
    /// An arch swept back and forth — a wash panning across a ceiling.
    Arc,
    /// Triangle waves a quarter apart: a diamond at constant speed.
    Diamond,
    /// The square's perimeter, corners softened so the beam keeps moving.
    Square,
    /// Corner to corner, no travel in between — the "choppy" square.
    SquareSnap,
    /// Pan only.
    Line,
    /// Tilt only.
    Lift,
    /// Pan and tilt in step: a diagonal line.
    Diagonal,
    /// Fast pan zig-zag riding a slow tilt.
    ZigZag,
    /// Three pan cycles to two tilt cycles: the pretzel.
    Lissajous,
}

impl PathShape {
    pub const ALL: [PathShape; 11] = [
        PathShape::Circle,
        PathShape::Eight,
        PathShape::Arc,
        PathShape::Diamond,
        PathShape::Square,
        PathShape::SquareSnap,
        PathShape::Line,
        PathShape::Lift,
        PathShape::Diagonal,
        PathShape::ZigZag,
        PathShape::Lissajous,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PathShape::Circle => "Circle",
            PathShape::Eight => "Eight",
            PathShape::Arc => "Arc",
            PathShape::Diamond => "Diamond",
            PathShape::Square => "Square",
            PathShape::SquareSnap => "Square (snap)",
            PathShape::Line => "Line",
            PathShape::Lift => "Lift",
            PathShape::Diagonal => "Diagonal",
            PathShape::ZigZag => "Zig-zag",
            PathShape::Lissajous => "Lissajous 3:2",
        }
    }

    /// Four-character tag for a deck key.
    pub fn tag(self) -> &'static str {
        match self {
            PathShape::Circle => "CIRC",
            PathShape::Eight => "EIGH",
            PathShape::Arc => "ARC",
            PathShape::Diamond => "DIAM",
            PathShape::Square => "SQR",
            PathShape::SquareSnap => "SNAP",
            PathShape::Line => "LINE",
            PathShape::Lift => "LIFT",
            PathShape::Diagonal => "DIAG",
            PathShape::ZigZag => "ZIG",
            PathShape::Lissajous => "LISS",
        }
    }

    /// The PAN/TILT components that trace this figure, at `subdiv` beats per
    /// pan cycle and depth `amount`. Phases here are written against the
    /// engine's rule that tilt already gets +0.25 when both axes are present
    /// — so a circle needs none, and a diagonal has to cancel it.
    pub fn components(self, subdiv: f32, amount: f32) -> Vec<PhaserComponent> {
        let comp = |target: &str, shape: f32, div: f32, phase: f32, amt: f32| PhaserComponent {
            shape,
            subdiv: Some(div),
            phase,
            amount: amt,
            ..PhaserComponent::for_target(target.into())
        };
        const TRI: f32 = 0.0;
        const SINE: f32 = 0.5;
        const SOFT_SQUARE: f32 = 0.8;
        const SQUARE: f32 = 1.0;
        match self {
            PathShape::Circle => vec![comp("PAN", SINE, subdiv, 0.0, amount), comp("TILT", SINE, subdiv, 0.0, amount)],
            PathShape::Eight => vec![comp("PAN", SINE, subdiv, 0.0, amount), comp("TILT", SINE, subdiv * 0.5, -0.25, amount * 0.7)],
            PathShape::Arc => vec![comp("PAN", SINE, subdiv, 0.0, amount), comp("TILT", SINE, subdiv * 0.5, 0.0, amount * 0.45)],
            PathShape::Diamond => vec![comp("PAN", TRI, subdiv, 0.0, amount), comp("TILT", TRI, subdiv, 0.0, amount)],
            PathShape::Square => vec![comp("PAN", SOFT_SQUARE, subdiv, 0.0, amount), comp("TILT", SOFT_SQUARE, subdiv, 0.0, amount)],
            PathShape::SquareSnap => vec![comp("PAN", SQUARE, subdiv, 0.0, amount), comp("TILT", SQUARE, subdiv, 0.0, amount)],
            PathShape::Line => vec![comp("PAN", SINE, subdiv, 0.0, amount)],
            PathShape::Lift => vec![comp("TILT", SINE, subdiv, 0.0, amount)],
            PathShape::Diagonal => vec![comp("PAN", SINE, subdiv, 0.0, amount), comp("TILT", SINE, subdiv, -0.25, amount)],
            PathShape::ZigZag => vec![comp("PAN", TRI, subdiv * 0.25, 0.0, amount), comp("TILT", TRI, subdiv, -0.25, amount)],
            PathShape::Lissajous => vec![comp("PAN", SINE, subdiv / 3.0, 0.0, amount), comp("TILT", SINE, subdiv / 2.0, 0.0, amount)],
        }
    }
}

/// One ready-made phaser per path shape, all fixtures moving in unison at
/// eight beats a cycle, coloured across the spectrum in shape order.
pub fn path_shape_phasers() -> Vec<Phaser> {
    let n = PathShape::ALL.len() as f32;
    PathShape::ALL
        .into_iter()
        .enumerate()
        .map(|(i, shape)| Phaser {
            name: shape.label().to_string(),
            feature: Feature::Position,
            components: shape.components(8.0, 0.3),
            path: Some(shape),
            color: hsv(i as f32 / n * 330.0, 0.8, 0.95),
            master_beat: true,
            spread: 0.0,
            ..Default::default()
        })
        .collect()
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
            path: None,
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
///
/// `spread` is one knob from order to chaos: 0 keeps every light on the
/// same phase, 1 fans exactly one cycle along the selection (a wave), and
/// past 1 the wave breaks up — see [`scatter_offset`] — until at 2 every
/// light sits on its own phase with no pattern left to see.
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
    scatter_offset(k, pos, spread)
}

/// Turn a light's place in the order (`pos`, 0..1) into its phase offset for
/// a spacing of `spread`. Up to 1 the offset is simply `pos * spread`: the
/// pattern fans out. Past 1 the wave cross-fades into a golden-ratio
/// scatter, so neighbours drift apart and, at 2, the order is gone entirely
/// — a scramble that never clumps the way real random numbers do. (It is a
/// cross-fade, not a sum: wave plus scatter makes a near-rational step that
/// lands some lights on top of each other.) Existing shows with spacings up
/// to 1 look the same.
pub fn scatter_offset(k: usize, pos: f32, spread: f32) -> f32 {
    let spread = spread.max(0.0);
    let fan = spread.min(1.0);
    let chaos = (spread - 1.0).clamp(0.0, 1.0);
    if chaos <= 0.0 {
        return pos * fan;
    }
    (pos * fan * (1.0 - chaos) + chaos * scatter(k)).rem_euclid(1.0)
}

/// Where light `k` lands when the order is thrown away.
pub fn scatter(k: usize) -> f32 {
    (k as f32 * 0.618_034).fract()
}

/// The Phasers window's library filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibFilter {
    #[default]
    All,
    /// Dimmer, colour, strobe: everything that is not movement.
    Light,
    Movement,
    /// Poses, holds, locks and FX tiles: stored snapshots, not waves.
    Snapshots,
    Running,
}

impl LibFilter {
    pub const ALL: [Self; 5] = [Self::All, Self::Light, Self::Movement, Self::Snapshots, Self::Running];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Light => "Light",
            Self::Movement => "Movement",
            Self::Snapshots => "Snapshots",
            Self::Running => "Running",
        }
    }
}

impl Phaser {
    /// A stored pose, hold, lock or FX tile rather than a wave.
    pub fn is_snapshot(&self) -> bool {
        !self.static_pos.is_empty() || !self.hold.is_empty()
    }
}

/// A handful of ready-to-use phasers, seeded on first run.
pub fn default_phasers() -> Vec<Phaser> {
    let mut v = vec![
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
    ];
    // Every pan/tilt figure ships as a default too — Circle, Eight, Square
    // and the rest — so a new show has them without hunting for a button.
    v.extend(path_shape_phasers());
    v
}

/// Fold the path-shape defaults into a pool that predates them. A pool with
/// no figure phasers at all hasn't seen this set, so it gets it — upgrading
/// a same-named legacy entry (the old spread-based "Circle") in place rather
/// than duplicating it, and keeping whatever the user had tuned on it.
/// Pools that already have any figure are left alone, so deleting one later
/// sticks. Returns how many entries were added or upgraded.
pub fn merge_path_defaults(phasers: &mut Vec<Phaser>) -> usize {
    if phasers.iter().any(|p| p.path.is_some()) {
        return 0;
    }
    let mut changed = 0;
    for preset in path_shape_phasers() {
        match phasers.iter_mut().find(|p| p.name == preset.name) {
            Some(existing) => {
                if existing.components.is_empty() {
                    existing.components = preset.components.clone();
                }
                existing.path = preset.path;
                changed += 1;
            }
            None => {
                phasers.push(preset);
                changed += 1;
            }
        }
    }
    changed
}

/// Pan ("back and forth") and Tilt ("up and down") at 3 speeds x 3
/// amplitudes each — 18 in total, in a fixed order (axis, then speed, then
/// amplitude) that the Stream Deck page-fill relies on to lay them into the
/// grid. Each row runs the full spectrum left to right, with Tilt's hues
/// nudged half a step so the two rows interleave rather than repeat.
pub fn movement_variety_phasers() -> Vec<Phaser> {
    const SPEEDS: [(&str, f32); 3] = [("Slow", 16.0), ("Med", 8.0), ("Fast", 4.0)];
    const AMPS: [(&str, f32); 3] = [("Small", 0.25), ("Mid", 0.5), ("Big", 0.9)];
    let axes: [(&str, ChannelFilter); 2] = [("Pan", ChannelFilter::Pan), ("Tilt", ChannelFilter::Tilt)];
    let per_row = (SPEEDS.len() * AMPS.len()) as f32;

    let mut out = Vec::with_capacity(18);
    for (axis_idx, (axis_name, filter)) in axes.into_iter().enumerate() {
        let mut col = 0usize;
        for (speed_name, subdiv) in SPEEDS {
            for (amp_name, amount) in AMPS {
                let hue = col as f32 / per_row * 330.0 + axis_idx as f32 * (330.0 / per_row * 0.5);
                out.push(Phaser {
                    name: format!("{axis_name} {speed_name} {amp_name}"),
                    feature: Feature::Position,
                    filter,
                    color: hsv(hue, 0.85, 0.95),
                    amount,
                    shape: 0.5,
                    subdiv: Some(subdiv),
                    master_beat: true,
                    spread: 0.5,
                    ..Default::default()
                });
                col += 1;
            }
        }
    }
    out
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
    if merge_path_defaults(&mut phasers) > 0 {
        save_phasers(&phasers);
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
    use super::*;
    use super::spread_phase;

    /// Circles have to come out round and diagonals straight. Both depend on
    /// the plotter applying the engine's quarter-cycle tilt rule exactly —
    /// get it wrong and every "circle" is a diagonal, or vice versa.
    #[test]
    fn path_shapes_trace_their_figures() {
        let trace = |shape: PathShape| {
            let mut ph = Phaser { components: shape.components(8.0, 1.0), ..Default::default() };
            ph.feature = Feature::Position;
            ph.path_points(256)
        };
        for shape in PathShape::ALL {
            let pts = trace(shape);
            assert!(!pts.is_empty(), "{shape:?} traced nothing");
            assert!(pts.iter().all(|(x, y)| x.abs() <= 1.0 && y.abs() <= 1.0), "{shape:?} out of range");
        }
        // Circle: constant radius.
        let radii: Vec<f32> = trace(PathShape::Circle).iter().map(|(x, y)| (x * x + y * y).sqrt()).collect();
        let (lo, hi) = radii.iter().fold((f32::MAX, f32::MIN), |(l, h), r| (l.min(*r), h.max(*r)));
        assert!(hi - lo < 0.05, "circle radius wobbles {lo}..{hi}");
        // Diagonal: pan tracks tilt.
        assert!(trace(PathShape::Diagonal).iter().all(|(x, y)| (x - y).abs() < 0.02), "diagonal isn't");
        // Line and Lift stay on one axis.
        assert!(trace(PathShape::Line).iter().all(|(_, y)| y.abs() < 1e-4));
        assert!(trace(PathShape::Lift).iter().all(|(x, _)| x.abs() < 1e-4));
        // Snap-square only ever sits at the four corners.
        assert!(trace(PathShape::SquareSnap).iter().all(|(x, y)| x.abs() > 0.99 && y.abs() > 0.99));
    }

    /// The figures are defaults, and an old pool picks them up on load —
    /// upgrading its legacy "Circle" rather than growing a second one.
    #[test]
    fn path_shapes_are_defaults_and_upgrade_old_pools() {
        let defaults = default_phasers();
        for shape in PathShape::ALL {
            assert!(defaults.iter().any(|p| p.path == Some(shape)), "{shape:?} missing from defaults");
        }
        assert_eq!(defaults.iter().filter(|p| p.name == "Circle").count(), 1);

        let mut old = vec![Phaser {
            name: "Circle".into(),
            feature: Feature::Position,
            spread: 0.25,
            ..Default::default()
        }];
        assert_eq!(merge_path_defaults(&mut old), PathShape::ALL.len());
        assert_eq!(old.len(), PathShape::ALL.len(), "Circle should upgrade in place");
        let circle = old.iter().find(|p| p.name == "Circle").unwrap();
        assert_eq!(circle.path, Some(PathShape::Circle));
        assert!(!circle.components.is_empty(), "legacy circle gained components");
        assert!((circle.spread - 0.25).abs() < 1e-6, "user's spread was kept");
        assert_eq!(merge_path_defaults(&mut old), 0, "second pass must be a no-op");
    }

    /// Draws every figure to a sheet so the shapes can be checked by eye.
    #[test]
    fn dump_path_shape_sheet() {
        const TILE: u32 = 120;
        let n = PathShape::ALL.len() as u32;
        let mut sheet = image::RgbImage::from_pixel(n * TILE, TILE, image::Rgb([22, 24, 28]));
        for (i, shape) in PathShape::ALL.into_iter().enumerate() {
            let mut ph = Phaser { components: shape.components(8.0, 1.0), ..Default::default() };
            ph.feature = Feature::Position;
            let pts = ph.path_points(400);
            let ox = i as u32 * TILE;
            let c = (ox as f32 + TILE as f32 * 0.5, TILE as f32 * 0.5);
            for k in 0..pts.len() {
                let (x0, y0) = pts[k];
                let (x1, y1) = pts[(k + 1) % pts.len()];
                let a = (c.0 + x0 * 48.0, c.1 - y0 * 48.0);
                let b = (c.0 + x1 * 48.0, c.1 - y1 * 48.0);
                let steps = ((a.0 - b.0).abs().max((a.1 - b.1).abs()) as usize).max(1);
                for s in 0..=steps {
                    let t = s as f32 / steps as f32;
                    let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                    if x >= 0.0 && y >= 0.0 && (x as u32) < n * TILE && (y as u32) < TILE {
                        sheet.put_pixel(x as u32, y as u32, image::Rgb([120, 220, 255]));
                    }
                }
            }
        }
        let path = std::env::temp_dir().join("dmxpress_paths.png");
        sheet.save(&path).expect("write path sheet");
        println!("path sheet: {}", path.display());
    }

    #[test]
    fn spread_assigns_one_phase_per_effect_unit() {
        assert_eq!(spread_phase(0, 3, 1.0, 1), 0.0);
        assert!((spread_phase(1, 3, 1.0, 1) - 1.0 / 3.0).abs() < 0.001);
        assert!((spread_phase(2, 3, 1.0, 1) - 2.0 / 3.0).abs() < 0.001);
    }

    /// Past one wave the spacing scatters: at the top every light is on its
    /// own phase and no two neighbours sit close, so a rig of any size
    /// flips about instead of rolling.
    #[test]
    fn spread_past_one_wave_scatters_the_rig() {
        let n = 40;
        // At one wave neighbours are a fortieth of a cycle apart: a roll.
        let wave: Vec<f32> = (0..n).map(|k| spread_phase(k, n, 1.0, 1)).collect();
        for w in wave.windows(2) {
            assert!((w[1] - w[0]).abs() < 0.03, "wave neighbours are close");
        }
        // Fully scattered: neighbours are far apart and no phase repeats.
        let chaos: Vec<f32> = (0..n).map(|k| spread_phase(k, n, 2.0, 1)).collect();
        for w in chaos.windows(2) {
            let d = (w[1] - w[0]).rem_euclid(1.0);
            let d = d.min(1.0 - d);
            assert!(d > 0.2, "scattered neighbours differ: {} vs {}", w[0], w[1]);
        }
        for (i, a) in chaos.iter().enumerate() {
            for b in &chaos[i + 1..] {
                assert!((a - b).abs() > 0.005, "every light has its own phase");
            }
        }
        // Halfway there is still a wave, just a broken one.
        let mid: Vec<f32> = (0..n).map(|k| spread_phase(k, n, 1.5, 1)).collect();
        assert!(mid.iter().zip(&wave).any(|(m, w)| (m - w).abs() > 0.1));
        assert!(mid.iter().zip(&chaos).any(|(m, c)| (m - c).abs() > 0.1));
        // Every offset stays a phase.
        for v in chaos.iter().chain(&mid) {
            assert!((0.0..1.0).contains(v));
        }
    }
}
