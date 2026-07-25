//! Preset transition options and the active transition renderer.
//!
//! A transition owns ticking copies of the outgoing and incoming oscillator
//! engines so animated presets can drift through the blend instead of jumping
//! to static snapshots.

use std::time::{Duration, Instant};

use crate::net::{self, Frame};
use crate::oscillator::Look;
use crate::showbuddy::Patch;
use crate::stage::{dir_from_angles, v3, V3};

/// Reusable transition selector placed beside any live control. This is the
/// common contract for knobs, buttons, palettes, phasers, and future effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransitionBinding {
    Master,
    Custom,
    None,
}

impl TransitionBinding {
    pub fn duration(self, master: f32, custom: f32) -> f32 {
        match self {
            Self::Master => master,
            Self::Custom => custom,
            Self::None => 0.0,
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Master => "M",
            Self::Custom => "C",
            Self::None => "—",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Master => Self::Custom,
            Self::Custom => Self::None,
            Self::None => Self::Master,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransitionMode {
    Simple,
    SphereScan,
    Radial,
}

impl TransitionMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Simple => "Simple",
            Self::SphereScan => "Sphere scan",
            Self::Radial => "Radial",
        }
    }

    pub fn uses_sphere(self) -> bool {
        !matches!(self, Self::Simple)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransitionCurve {
    Linear,
    Smooth,
}

impl TransitionCurve {
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Smooth => "Smooth",
        }
    }

    fn ease(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Smooth => t * t * (3.0 - 2.0 * t),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TransitionSphere {
    pub pos: V3,
    pub yaw_deg: f32,
}

impl Default for TransitionSphere {
    fn default() -> Self {
        Self {
            pos: v3(0.0, 2.4, 0.0),
            yaw_deg: 0.0,
        }
    }
}

pub(crate) struct TransitionConfig {
    /// 0 = cut immediately; max UI value is 20 seconds.
    pub duration: f32,
    pub mode: TransitionMode,
    pub curve: TransitionCurve,
    /// Full editor controls whether the advanced sphere is visible/editable
    /// in the stage view.
    pub expanded: bool,
    pub sphere: TransitionSphere,
    /// Fraction of the full pattern time each fixture spends blending once the
    /// scan/blast reaches it.
    pub edge_width: f32,
    /// Radial mode: compact shockwave instead of a long centre-out dissolve.
    pub blast_mode: bool,
    /// Stage-only selection state for moving the advanced sphere.
    pub selected: bool,
    /// Live progress (0..1) of the running transition, for the stage preview.
    /// Set each frame by the app; `None` when idle.
    pub active_progress: Option<f32>,
}

impl Default for TransitionConfig {
    fn default() -> Self {
        Self {
            duration: 0.0,
            mode: TransitionMode::Simple,
            curve: TransitionCurve::Smooth,
            expanded: false,
            sphere: TransitionSphere::default(),
            edge_width: 0.18,
            blast_mode: true,
            selected: false,
            active_progress: None,
        }
    }
}

impl TransitionConfig {
    pub fn stage_visible(&self) -> bool {
        self.expanded && self.mode.uses_sphere()
    }
}

/// One queued blend: its incoming look (a [`Look`], static or animated) and
/// the per-channel sweep windows that schedule when each fixture crosses.
struct TransitionLayer {
    to: Look,
    started: Instant,
    duration: f32,
    curve: TransitionCurve,
    channel_windows: Vec<(f32, f32)>,
}

impl TransitionLayer {
    fn build(
        to: Look,
        config: &TransitionConfig,
        patch: &Patch,
        fixture_positions: &[(usize, V3)],
    ) -> Self {
        Self {
            to,
            started: Instant::now(),
            duration: config.duration.max(0.001),
            curve: config.curve,
            channel_windows: channel_windows(config, patch, fixture_positions),
        }
    }

    fn progress(&self) -> f32 {
        (self.started.elapsed().as_secs_f32() / self.duration).clamp(0.0, 1.0)
    }

    /// Blend this layer's incoming look over `buf` in place. The "from" of the
    /// blend is whatever the layers below already produced, so a newer layer
    /// naturally overwrites the older blend as its own edge sweeps past.
    fn apply(&mut self, buf: &mut Frame) {
        let raw = self.progress();
        let to = self.to.render();
        for i in 0..net::DMX_SLOTS {
            let (start, end) = self.channel_windows[i];
            let local = if end <= start {
                (raw >= end) as u8 as f32
            } else {
                ((raw - start) / (end - start)).clamp(0.0, 1.0)
            };
            let k = self.curve.ease(local);
            buf.blend_channel(i, to[i], k);
        }
    }
}

/// A stack of queued transition layers blended over a base look. Pressing a
/// new preset mid-transition pushes another layer that immediately starts
/// chasing the ones already running, so blends can be stacked without limit.
pub(crate) struct TransitionRun {
    base: Look,
    layers: Vec<TransitionLayer>,
}

impl TransitionRun {
    pub fn new(
        from: Look,
        to: Look,
        config: &TransitionConfig,
        patch: &Patch,
        fixture_positions: &[(usize, V3)],
    ) -> Self {
        Self {
            base: from,
            layers: vec![TransitionLayer::build(to, config, patch, fixture_positions)],
        }
    }

    /// Queue another blend on top of the running stack (the chasing effect).
    pub fn push(
        &mut self,
        to: Look,
        config: &TransitionConfig,
        patch: &Patch,
        fixture_positions: &[(usize, V3)],
    ) {
        self.layers
            .push(TransitionLayer::build(to, config, patch, fixture_positions));
    }

    /// Progress (0..1) of the newest queued layer — what the stage marker and
    /// the progress bar track.
    pub fn progress(&self) -> f32 {
        self.layers.last().map_or(1.0, |l| l.progress())
    }

    /// The look this run is ultimately heading to (the newest queued layer,
    /// else the settled base) — the effective "current look" while `live` is
    /// blacked out during the run.
    pub fn pending(&self) -> &Look {
        self.layers.last().map_or(&self.base, |l| &l.to)
    }

    /// Master-BPM override: force every look in the run to `bpm`.
    pub fn set_tempo(&mut self, bpm: f32) {
        self.base.tempo = bpm;
        for l in &mut self.layers {
            l.to.tempo = bpm;
        }
    }

    /// Ease every look's beat clock toward a tapped beat/bar.
    pub fn drift_beats(&mut self, quantum: f32) {
        self.base.drift_beats(quantum);
        for l in &mut self.layers {
            l.to.drift_beats(quantum);
        }
    }

    /// Move every wall-clock origin past a global pause. Integrated look
    /// clocks are merely rebased, so both transition progress and oscillator
    /// phase resume at the exact frozen sample.
    pub fn resume_after(&mut self, paused: Duration) {
        self.base.resume_clock();
        for layer in &mut self.layers {
            layer.started += paused;
            layer.to.resume_clock();
        }
    }

    pub fn render(&mut self) -> (Frame, bool) {
        // Start from the base look (its oscillators keep ticking), then blend
        // each queued layer over it from oldest to newest.
        let mut buf = self.base.render();
        for layer in &mut self.layers {
            layer.apply(&mut buf);
        }
        // Retire finished layers from the front, folding their incoming look
        // (and engine) into the base so it becomes the new floor for the rest.
        while self.layers.first().is_some_and(|l| l.progress() >= 1.0) {
            self.base = self.layers.remove(0).to;
        }
        (buf, self.layers.is_empty())
    }

    /// The settled base look once every layer has finished.
    pub fn finish(self) -> Look {
        self.base
    }
}

fn channel_windows(
    config: &TransitionConfig,
    patch: &Patch,
    fixture_positions: &[(usize, V3)],
) -> Vec<(f32, f32)> {
    let mut windows = vec![(0.0, 1.0); net::DMX_SLOTS];
    if config.duration <= 0.0 || matches!(config.mode, TransitionMode::Simple) {
        return windows;
    }

    let width = config.edge_width.clamp(0.02, 0.85);
    let mut fixture_windows = Vec::with_capacity(fixture_positions.len());
    match config.mode {
        TransitionMode::Simple => {}
        TransitionMode::SphereScan => {
            let front = dir_from_angles(config.sphere.yaw_deg, 0.0).norm();
            let side = dir_from_angles(config.sphere.yaw_deg + 90.0, 0.0).norm();
            for &(fi, pos) in fixture_positions {
                let d = pos - config.sphere.pos;
                let angle = d.dot(side).atan2(d.dot(front)).rem_euclid(std::f32::consts::TAU);
                let frac = angle / std::f32::consts::TAU;
                fixture_windows.push((fi, window_at(frac, width)));
            }
        }
        TransitionMode::Radial => {
            let max_dist = fixture_positions
                .iter()
                .map(|(_, pos)| (*pos - config.sphere.pos).len())
                .fold(0.0f32, f32::max)
                .max(0.001);
            for &(fi, pos) in fixture_positions {
                let frac = ((pos - config.sphere.pos).len() / max_dist).clamp(0.0, 1.0);
                let window = if config.blast_mode {
                    window_at(frac, width)
                } else {
                    let start = (frac * 0.65).clamp(0.0, 0.95);
                    (start, 1.0)
                };
                fixture_windows.push((fi, window));
            }
        }
    }

    for (fi, window) in fixture_windows {
        let Some(f) = patch.fixtures.get(fi) else { continue };
        for addr in f.from..=f.to {
            if (1..=net::DMX_SLOTS as u16).contains(&addr) {
                windows[addr as usize - 1] = window;
            }
        }
    }
    windows
}

fn window_at(frac: f32, width: f32) -> (f32, f32) {
    let start = (frac * (1.0 - width)).clamp(0.0, 1.0 - width);
    (start, (start + width).min(1.0))
}
