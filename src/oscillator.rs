//! Live oscillation engine: the animation model that drives ShowBuddy-style
//! per-channel modulation of the DMX buffer.

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::net::Frame;
use crate::showbuddy::PresetData;

const WAVEFORMS_FILE: &str = "waveforms.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SegmentKind {
    /// Direct point-to-point interpolation.
    #[default]
    Linear,
    /// Smooth eased interpolation with flat tangents at both points.
    Curved,
    /// Hold the first point until the next break.
    Square,
}

impl SegmentKind {
    pub const ALL: [SegmentKind; 3] = [Self::Linear, Self::Curved, Self::Square];

    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Point to point",
            Self::Curved => "Curved",
            Self::Square => "Square / hold",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WavePoint {
    pub x: f32,
    pub y: f32,
    /// Interpolation from this point to the next break.
    #[serde(default)]
    pub segment: SegmentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WaveTraversal {
    #[default]
    Forward,
    /// Traverse the drawn wave forward and then backward without a jump.
    Boomerang,
}

impl WaveTraversal {
    pub const ALL: [Self; 2] = [Self::Forward, Self::Boomerang];

    pub fn label(self) -> &'static str {
        match self {
            Self::Forward => "Forward",
            Self::Boomerang => "Boomerang ↔",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomWaveform {
    pub id: u32,
    pub name: String,
    pub points: Vec<WavePoint>,
    /// How the playhead traverses the drawn shape.
    #[serde(default)]
    pub traversal: WaveTraversal,
    /// Number of copies per oscillator cycle. Values above one create stripes.
    #[serde(default = "one_repeat")]
    pub repeats: u8,
}

fn one_repeat() -> u8 { 1 }

impl Default for CustomWaveform {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Custom wave".into(),
            points: vec![
                WavePoint { x: 0.0, y: 0.0, segment: SegmentKind::Linear },
                WavePoint { x: 1.0, y: 0.0, segment: SegmentKind::Linear },
            ],
            traversal: WaveTraversal::Forward,
            repeats: 1,
        }
    }
}

pub fn load_waveforms() -> Vec<CustomWaveform> {
    std::fs::read_to_string(WAVEFORMS_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_waveforms(waves: &[CustomWaveform]) {
    if let Ok(json) = serde_json::to_string_pretty(waves) {
        let _ = std::fs::write(WAVEFORMS_FILE, json);
    }
}

/// One channel's oscillator (ShowBuddy-style: Enabled/Invert/Amount/Offset/
/// Speed/Shape).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Osc {
    pub enabled: bool,
    pub invert: bool,
    /// Depth 0..1 of full range, around the base value.
    pub amount: f32,
    /// Phase offset 0..1.
    pub phase: f32,
    /// Beat-synced cycle length in beats; None = free-run from `speed`.
    pub subdiv: Option<f32>,
    /// Waveform morph 0..1 (triangle -> sine -> square).
    pub shape: f32,
    /// Follow the shared master beat clock. When false this oscillator keeps
    /// the tempo it had when armed and ignores subsequent taps.
    pub master_beat: bool,
    /// Private beat clock used while opted out of the master beat.
    pub local_beats: f32,
    pub local_tempo: f32,
    /// Embedded definition so rendering remains independent of pool edits.
    pub custom_wave: Option<CustomWaveform>,
}

impl Default for Osc {
    fn default() -> Self {
        Self {
            enabled: true,
            invert: false,
            amount: 0.4,
            phase: 0.0,
            subdiv: Some(4.0), // 1 bar
            shape: 0.5,
            master_beat: true,
            local_beats: 0.0,
            local_tempo: 120.0,
            custom_wave: None,
        }
    }
}

/// A renderable look: a base [`Frame`] plus the per-channel oscillators that
/// animate it. A *static* look simply has no oscillators, so the same type is
/// the universal currency for presets, transition endpoints and chase sources
/// — anything that can be rendered into a frame over time.
///
/// Phase is integrated incrementally (beats/cycles accumulators) so changing
/// tempo or speed bends the rate without making the lights jump.
#[derive(Clone)]
pub(crate) struct Look {
    pub base: Frame,
    /// Keyed by 0-based DMX buffer index. Empty = a static look.
    pub oscs: HashMap<usize, Osc>,
    pub master_speed: f32,
    /// Free-run speed 0..1.
    pub speed: f32,
    /// BPM for beat-synced channels.
    pub tempo: f32,
    /// Accumulated beats / free-run cycles.
    beats: f32,
    cycles: f32,
    /// Beat correction still to be eased into the shared clock after taps.
    beat_nudge: f32,
    last: Instant,
}

impl Look {
    /// A static blackout look.
    pub fn black() -> Self {
        Self::from_frame(Frame::black())
    }

    /// A static look that holds `base` unchanged.
    pub fn from_frame(base: Frame) -> Self {
        Self {
            base,
            oscs: HashMap::new(),
            master_speed: 1.0,
            speed: 0.357,
            tempo: 120.0,
            beats: 0.0,
            cycles: 0.0,
            beat_nudge: 0.0,
            last: Instant::now(),
        }
    }

    /// Build a look from a ShowBuddy preset — static if it has no oscillator
    /// assignments, animated otherwise.
    pub fn from_preset(data: &PresetData) -> Self {
        let oscs = data
            .mods
            .iter()
            .map(|m| {
                (
                    m.addr as usize - 1,
                    Osc {
                        enabled: true,
                        invert: m.invert,
                        amount: m.amount,
                        phase: m.phase,
                        subdiv: m.subdiv,
                        shape: data.shape,
                        master_beat: true,
                        local_beats: 0.0,
                        local_tempo: data.tempo,
                        custom_wave: None,
                    },
                )
            })
            .collect();
        Self {
            oscs,
            master_speed: data.master_speed,
            speed: data.speed,
            tempo: data.tempo,
            ..Self::from_frame(data.base_frame())
        }
    }

    pub fn is_animated(&self) -> bool {
        !self.oscs.is_empty()
    }

    /// Tap-sync without a visual snap: queue the shortest correction to the
    /// nearest beat/bar and let render ease it into the shared clock.
    pub fn drift_beats(&mut self, quantum: f32) {
        if quantum > 0.0 {
            let target = (self.beats / quantum).round() * quantum;
            self.beat_nudge = target - self.beats;
        }
    }

    /// Position inside a 4-beat bar (0..4) for beat indicators.
    pub fn beat_phase(&self) -> f32 {
        self.beats.rem_euclid(4.0)
    }

    /// Current shared beat position, used to launch or detach a local clock
    /// without changing oscillator phase.
    pub fn beat_clock(&self) -> f32 {
        self.beats
    }

    /// Rebase wall-clock sampling without changing either integrated phase.
    /// Used by the global transport after a freeze so the paused duration is
    /// never interpreted as animation time.
    pub fn resume_clock(&mut self) {
        self.last = Instant::now();
    }

    /// Advance the clocks and render the look into a frame. A static look just
    /// returns its base (and keeps its clock fresh so it never jumps when an
    /// oscillator is later armed).
    pub fn render(&mut self) -> Frame {
        if self.oscs.is_empty() {
            self.last = Instant::now();
            return self.base;
        }
        let dt = self.last.elapsed().as_secs_f32().min(0.25);
        self.last = Instant::now();
        self.beats += dt * self.tempo / 60.0 * self.master_speed;
        self.cycles += dt * (0.25 + self.speed * 3.75) * self.master_speed;

        // Settle most of a tap correction in roughly one second. Repeated
        // taps continuously refine the destination instead of jumping phase.
        if self.beat_nudge.abs() > 0.0001 {
            let step = self.beat_nudge * (1.0 - (-4.0 * dt).exp());
            self.beats += step;
            self.beat_nudge -= step;
        } else {
            self.beat_nudge = 0.0;
        }

        let mut buf = self.base;
        for (&idx, o) in &mut self.oscs {
            if !o.enabled || o.amount <= 0.0 {
                continue;
            }
            if !o.master_beat {
                o.local_beats += dt * o.local_tempo.max(1.0) / 60.0 * self.master_speed;
            }
            let x = match o.subdiv {
                Some(s) if s > 0.0 => {
                    (if o.master_beat { self.beats } else { o.local_beats }) / s
                }
                _ => self.cycles,
            } + o.phase;
            let mut w = o
                .custom_wave
                .as_ref()
                .map_or_else(|| wave(x, o.shape), |custom| custom_wave(x, custom));
            if o.invert {
                w = -w;
            }
            let v = self.base[idx] as f32 + o.amount * 255.0 * w;
            buf[idx] = v.clamp(0.0, 255.0) as u8;
        }
        buf
    }
}

/// Periodic waveform in -1..1. `shape` morphs triangle -> sine -> square.
fn wave(x: f32, shape: f32) -> f32 {
    let ph = x.rem_euclid(1.0);
    let sine = (ph * std::f32::consts::TAU).sin();
    if shape <= 0.5 {
        let tri = 1.0 - 4.0 * (ph - 0.5).abs(); // -1 at 0, +1 at 0.5
        let k = shape * 2.0;
        tri * (1.0 - k) + sine * k
    } else {
        let sq = if sine >= 0.0 { 1.0 } else { -1.0 };
        let k = (shape - 0.5) * 2.0;
        sine * (1.0 - k) + sq * k
    }
}

pub(crate) fn custom_wave(x: f32, waveform: &CustomWaveform) -> f32 {
    if waveform.points.len() < 2 {
        return 0.0;
    }
    let mut phase = (x * waveform.repeats.max(1) as f32).rem_euclid(1.0);
    if waveform.traversal == WaveTraversal::Boomerang {
        phase = if phase <= 0.5 {
            phase * 2.0
        } else {
            (1.0 - phase) * 2.0
        };
    }
    let points = &waveform.points;
    let i = points
        .windows(2)
        .position(|pair| phase >= pair[0].x && phase <= pair[1].x)
        .unwrap_or(points.len() - 2);
    let a = &points[i];
    let b = &points[i + 1];
    let span = (b.x - a.x).max(0.0001);
    let mut t = ((phase - a.x) / span).clamp(0.0, 1.0);
    match a.segment {
        SegmentKind::Square => a.y,
        SegmentKind::Linear => a.y + (b.y - a.y) * t,
        SegmentKind::Curved => {
            t = t * t * (3.0 - 2.0 * t);
            a.y + (b.y - a.y) * t
        }
    }
    .clamp(-1.0, 1.0)
}

/// Beat-sync choices for the oscillator Speed control.
pub(crate) const SPEED_CHOICES: [(&str, Option<f32>); 7] = [
    ("Free", None),
    ("4 bars", Some(16.0)),
    ("2 bars", Some(8.0)),
    ("1 bar", Some(4.0)),
    ("1/2", Some(2.0)),
    ("1/4", Some(1.0)),
    ("1/8", Some(0.5)),
];

pub(crate) fn subdiv_label(s: Option<f32>) -> String {
    for (name, v) in SPEED_CHOICES {
        if v == s {
            return name.into();
        }
    }
    match s {
        Some(b) => format!("{b} beats"),
        None => "Free".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp() -> CustomWaveform {
        CustomWaveform {
            points: vec![
                WavePoint { x: 0.0, y: -1.0, segment: SegmentKind::Linear },
                WavePoint { x: 1.0, y: 1.0, segment: SegmentKind::Linear },
            ],
            ..CustomWaveform::default()
        }
    }

    #[test]
    fn boomerang_retraces_without_endpoint_jump() {
        let mut wave = ramp();
        wave.traversal = WaveTraversal::Boomerang;
        assert!((custom_wave(0.25, &wave) - custom_wave(0.75, &wave)).abs() < 0.001);
        assert!((custom_wave(0.5, &wave) - 1.0).abs() < 0.001);
        assert!((custom_wave(0.0, &wave) + 1.0).abs() < 0.001);
    }

    #[test]
    fn repeats_create_identical_stripes() {
        let mut wave = ramp();
        wave.repeats = 4;
        assert!((custom_wave(0.05, &wave) - custom_wave(0.30, &wave)).abs() < 0.001);
    }
}
