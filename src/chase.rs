//! Chases: looping, non-destructive overlays that inject a preset as a
//! moving pattern — a band sweeping around a sphere, a plane sweeping across
//! the rig, random glitter sparkles, or a single one-shot pulse. Unlike a
//! transition a chase never changes the underlying look — fixtures revert to
//! the base as the pattern passes by.

use std::time::{Duration, Instant};

use crate::engine::Layer;
use crate::net;
use crate::oscillator::Look;
use crate::showbuddy::Patch;
use crate::stage::{dir_from_angles, V3};
use crate::transition::TransitionSphere;

/// What the chase band injects: a ShowBuddy bank preset or a native preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChaseSource {
    /// ShowBuddy preset (bank, index).
    Bank(usize, usize),
    /// Native DMXpress preset index.
    User(usize),
}

/// The movement pattern of a chase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChaseKind {
    /// A band orbiting the sphere's axis (tiltable).
    Sphere,
    /// A flat wave travelling across the rig in one direction.
    Linear,
    /// A flat wave that reflects at each end and retraces its path.
    Boomerang,
    /// Multiple evenly-spaced moving bands across the rig.
    Stripes,
    /// Random per-fixture sparkles.
    Glitter,
    /// One single linear sweep, then it stops by itself.
    Pulse,
    /// Rings expanding out from the chase origin, like a stone in water.
    Ripple,
    /// A bright head dragging a decaying tail behind it.
    Comet,
    /// A spiral arm sweeping round, angle and radius together.
    Spiral,
    /// A wave falling down the rig's height, whatever way it is aimed.
    Cascade,
    /// A mirrored pair of bands opening out from the middle of the rig.
    Chevron,
    /// The rig split into banks that flip on and off against each other.
    Checker,
}

impl ChaseKind {
    pub const ALL: [ChaseKind; 12] = [
        ChaseKind::Sphere,
        ChaseKind::Linear,
        ChaseKind::Boomerang,
        ChaseKind::Stripes,
        ChaseKind::Glitter,
        ChaseKind::Pulse,
        ChaseKind::Ripple,
        ChaseKind::Comet,
        ChaseKind::Spiral,
        ChaseKind::Cascade,
        ChaseKind::Chevron,
        ChaseKind::Checker,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ChaseKind::Sphere => "Sphere",
            ChaseKind::Linear => "Linear",
            ChaseKind::Boomerang => "Boomerang",
            ChaseKind::Stripes => "Stripes",
            ChaseKind::Glitter => "Glitter",
            ChaseKind::Pulse => "Pulse",
            ChaseKind::Ripple => "Ripple",
            ChaseKind::Comet => "Comet",
            ChaseKind::Spiral => "Spiral",
            ChaseKind::Cascade => "Cascade",
            ChaseKind::Chevron => "Chevron",
            ChaseKind::Checker => "Checker",
        }
    }

    /// Four-character tag for the Stream Deck's Chases page.
    pub fn tag(self) -> &'static str {
        match self {
            ChaseKind::Sphere => "SPHR",
            ChaseKind::Linear => "LINE",
            ChaseKind::Boomerang => "BOOM",
            ChaseKind::Stripes => "STRP",
            ChaseKind::Glitter => "GLTR",
            ChaseKind::Pulse => "PULS",
            ChaseKind::Ripple => "RIPL",
            ChaseKind::Comet => "COMT",
            ChaseKind::Spiral => "SPRL",
            ChaseKind::Cascade => "CASC",
            ChaseKind::Chevron => "CHEV",
            ChaseKind::Checker => "CHEK",
        }
    }

    /// Whether `stripe_count` means something for this shape, and what to
    /// call it in the UI.
    pub fn repeat_label(self) -> Option<&'static str> {
        match self {
            ChaseKind::Stripes => Some("Stripe count"),
            ChaseKind::Ripple => Some("Ring count"),
            ChaseKind::Spiral => Some("Spiral arms"),
            ChaseKind::Checker => Some("Bank count"),
            _ => None,
        }
    }
}

pub(crate) struct ChaseConfig {
    pub enabled: bool,
    /// Preset whose values the moving band injects.
    pub source: Option<ChaseSource>,
    /// The movement pattern.
    pub kind: ChaseKind,
    pub sphere: TransitionSphere,
    /// Tilt of the sweep plane in degrees (0 = flat orbit / horizontal
    /// travel, 90 = the pattern climbs vertically).
    pub pitch_deg: f32,
    /// Angular width of the pulse band in degrees (up to a quarter sphere).
    /// Linear/pulse map it to a stage fraction; glitter maps it to density.
    pub band_deg: f32,
    /// Repeat count: stripes, ripple rings, spiral arms or checker banks,
    /// depending on the shape (see [`ChaseKind::repeat_label`]).
    pub stripe_count: u32,
    /// Size the band to the rig instead of to a fixed angle: the same chase
    /// then reads the same on four lights and on forty, because the band is
    /// derived from how many fixtures it should cover rather than from how
    /// much room the rig happens to take up.
    pub auto_width: bool,
    /// How many fixtures the band should cover when `auto_width` is on.
    pub target_lights: u32,
    /// Revolutions (sweeps, sparkles) per second.
    pub speed: f32,
    /// +1 / -1 travel direction.
    pub direction: f32,
    /// Soft cosine edges (a pulse) instead of a hard wedge.
    pub soft: bool,
    /// Show / edit the chase sphere on the stage.
    pub expanded: bool,
    pub selected: bool,
    /// Live band-centre fraction (0..1) for the stage marker; set each frame.
    pub active_head: Option<f32>,
    /// Live sweep count, un-wrapped — the marker needs the whole number to
    /// work out which checker bank is up or which fixtures are sparkling,
    /// and `active_head` has already thrown the integer part away.
    pub active_step: Option<f32>,
}

impl Default for ChaseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            source: None,
            kind: ChaseKind::Sphere,
            sphere: TransitionSphere::default(),
            pitch_deg: 0.0,
            band_deg: 60.0,
            stripe_count: 4,
            auto_width: false,
            target_lights: 3,
            speed: 0.3,
            direction: 1.0,
            soft: true,
            expanded: false,
            selected: false,
            active_head: None,
            active_step: None,
        }
    }
}

impl ChaseConfig {
    pub fn stage_visible(&self) -> bool {
        self.expanded
    }
}

/// The live injected look (a [`Look`], static or animated) plus the chase clock.
pub(crate) struct ChaseRun {
    inject: Look,
    started: Instant,
}

impl ChaseRun {
    pub fn new(inject: Look) -> Self {
        Self {
            inject,
            started: Instant::now(),
        }
    }

    /// Master-BPM override for the injected look.
    pub fn set_tempo(&mut self, bpm: f32) {
        self.inject.tempo = bpm;
    }

    /// Tap-sync the injected look's beat clock.
    pub fn drift_beats(&mut self, quantum: f32) {
        self.inject.drift_beats(quantum);
    }

    /// Resume from a global freeze without advancing the chase head or the
    /// injected look's oscillator phase.
    pub fn resume_after(&mut self, paused: Duration) {
        self.started += paused;
        self.inject.resume_clock();
    }

    /// Push the chase clock backwards so tests can step through a sweep
    /// without sleeping through it.
    #[cfg(test)]
    pub fn rewind(&mut self, d: Duration) {
        self.started -= d;
    }

    /// Sweeps completed since the chase started, fractional part included.
    pub fn raw_progress(&self, cfg: &ChaseConfig) -> f32 {
        self.started.elapsed().as_secs_f32() * cfg.speed.max(0.001)
    }

    /// Band-centre position as a 0..1 fraction around the circle.
    pub fn head(&self, cfg: &ChaseConfig) -> f32 {
        (self.started.elapsed().as_secs_f32() * cfg.speed * cfg.direction).rem_euclid(1.0)
    }

    /// Whether a one-shot pulse has fully swept past the rig.
    pub fn pulse_done(&self, cfg: &ChaseConfig) -> bool {
        cfg.kind == ChaseKind::Pulse
            && self.started.elapsed().as_secs_f32() * cfg.speed.max(0.001) >= 1.0
    }

    /// Build this frame's chase contribution as a mixer [`Layer`]: the injected
    /// look, weighted only on the fixtures the moving pattern currently covers
    /// (so it blends over the base and reverts behind it).
    pub fn layer(
        &mut self,
        cfg: &ChaseConfig,
        patch: &Patch,
        fixture_positions: &[(usize, V3)],
    ) -> Layer {
        let inject = self.inject.render();
        let t = self.started.elapsed().as_secs_f32();
        let up = V3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };
        let front = dir_from_angles(cfg.sphere.yaw_deg, 0.0).norm();
        let p = cfg.pitch_deg.to_radians();
        // The sweep plane's second axis, tilted up out of the horizontal.
        let side = (dir_from_angles(cfg.sphere.yaw_deg + 90.0, 0.0) * p.cos() + up * p.sin())
            .norm();
        // Linear travel direction (tilted the same way).
        let travel = (front * p.cos() + up * p.sin()).norm();

        // Rig extent along the travel axis, the vertical, and out from the
        // origin. Every shape works off these rather than raw metres, which
        // is what lets one chase read the same on a 4-light bar and on a
        // 40-light rig.
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let (mut y_lo, mut y_hi) = (f32::MAX, f32::MIN);
        let mut max_radius = 0.0f32;
        for &(_, pos) in fixture_positions {
            let d = pos - cfg.sphere.pos;
            let proj = d.dot(travel);
            lo = lo.min(proj);
            hi = hi.max(proj);
            y_lo = y_lo.min(pos.y);
            y_hi = y_hi.max(pos.y);
            max_radius = max_radius.max((d.dot(front).powi(2) + d.dot(side).powi(2)).sqrt());
        }
        let span = (hi - lo).max(1e-3);
        let y_span = y_hi - y_lo;
        let max_radius = max_radius.max(1e-3);

        // Auto width sizes the band by fixture count instead of by angle, so
        // it always covers roughly `target_lights` lights.
        let count = fixture_positions.len().max(1) as f32;
        let band_deg = if cfg.auto_width {
            (180.0 * cfg.target_lights.max(1) as f32 / count).clamp(6.0, 180.0)
        } else {
            cfg.band_deg
        };
        // Band width as a fraction of the rig for linear kinds.
        let width = (band_deg.clamp(2.0, 180.0) / 180.0).clamp(0.02, 1.0);
        let head = self.head(cfg);

        let mut weights: Vec<(usize, f32)> = Vec::new();
        for &(fi, pos) in fixture_positions {
            let d = pos - cfg.sphere.pos;
            let w = match cfg.kind {
                ChaseKind::Sphere => {
                    let half = (band_deg.clamp(2.0, 180.0) / 360.0 * 0.5).max(0.001);
                    let angle = d
                        .dot(side)
                        .atan2(d.dot(front))
                        .rem_euclid(std::f32::consts::TAU);
                    let frac = angle / std::f32::consts::TAU;
                    // Shortest distance around the loop from the band centre.
                    let mut dist = (frac - head).abs();
                    if dist > 0.5 {
                        dist = 1.0 - dist;
                    }
                    if dist > half {
                        continue;
                    }
                    if cfg.soft {
                        (std::f32::consts::FRAC_PI_2 * (1.0 - dist / half)).sin()
                    } else {
                        1.0
                    }
                }
                ChaseKind::Linear | ChaseKind::Boomerang | ChaseKind::Stripes | ChaseKind::Pulse => {
                    let half = width * 0.5;
                    let tn = ((pos - cfg.sphere.pos).dot(travel) - lo) / span;
                    if cfg.kind == ChaseKind::Stripes {
                        let count = cfg.stripe_count.max(1) as f32;
                        let phase =
                            (tn * count - t * cfg.speed * cfg.direction).rem_euclid(1.0);
                        let dist = phase.min(1.0 - phase);
                        if dist > half {
                            continue;
                        }
                        if cfg.soft {
                            (std::f32::consts::FRAC_PI_2 * (1.0 - dist / half)).sin()
                        } else {
                            1.0
                        }
                    } else {
                    // Travel across the rig plus one band width, so the wave
                    // fully enters and exits.
                    let progress = if cfg.kind == ChaseKind::Pulse {
                        (t * cfg.speed.max(0.001)).min(1.0)
                    } else if cfg.kind == ChaseKind::Boomerang {
                        let p = (t * cfg.speed).rem_euclid(2.0);
                        if p <= 1.0 { p } else { 2.0 - p }
                    } else {
                        (t * cfg.speed).rem_euclid(1.0)
                    };
                    let mut centre = if cfg.kind == ChaseKind::Boomerang {
                        progress
                    } else {
                        progress * (1.0 + width) - half
                    };
                    if cfg.direction < 0.0 {
                        centre = 1.0 - centre;
                    }
                    let dist = (tn - centre).abs();
                    if dist > half {
                        continue;
                    }
                    if cfg.soft {
                        (std::f32::consts::FRAC_PI_2 * (1.0 - dist / half)).sin()
                    } else {
                        1.0
                    }
                    }
                }
                ChaseKind::Ripple => {
                    // Rings expanding out from the origin. Radius is measured
                    // in the sweep plane and normalised, so a tight cluster
                    // and a room-wide rig both get full rings.
                    let radius =
                        (d.dot(front).powi(2) + d.dot(side).powi(2)).sqrt() / max_radius;
                    let rings = cfg.stripe_count.max(1) as f32;
                    let phase = (radius * rings - t * cfg.speed * cfg.direction).rem_euclid(1.0);
                    let dist = phase.min(1.0 - phase);
                    let half = width * 0.5;
                    if dist > half {
                        continue;
                    }
                    soft_edge(cfg.soft, dist, half)
                }
                ChaseKind::Comet => {
                    // A head with a tail trailing *behind* it — the falloff is
                    // one-sided, which is what separates this from Linear.
                    let tn = (d.dot(travel) - lo) / span;
                    let centre = (t * cfg.speed * cfg.direction).rem_euclid(1.0);
                    let behind = (centre - tn).rem_euclid(1.0);
                    let tail = (width * 3.0).clamp(0.05, 1.0);
                    if behind > tail {
                        continue;
                    }
                    // Squared falloff: a bright head fading fast, then a long
                    // dim streak, rather than a flat band.
                    let k = 1.0 - behind / tail;
                    k * k
                }
                ChaseKind::Spiral => {
                    // Angle and radius together: an arm that sweeps round and
                    // reaches outward at the same time.
                    let angle = d
                        .dot(side)
                        .atan2(d.dot(front))
                        .rem_euclid(std::f32::consts::TAU)
                        / std::f32::consts::TAU;
                    let radius =
                        (d.dot(front).powi(2) + d.dot(side).powi(2)).sqrt() / max_radius;
                    let arms = cfg.stripe_count.max(1) as f32;
                    let phase = ((angle + radius) * arms - t * cfg.speed * cfg.direction)
                        .rem_euclid(1.0);
                    let dist = phase.min(1.0 - phase);
                    let half = width * 0.5;
                    if dist > half {
                        continue;
                    }
                    soft_edge(cfg.soft, dist, half)
                }
                ChaseKind::Cascade => {
                    // Falls down the rig's height, ignoring the sweep yaw. A
                    // rig hung all at one height has no vertical extent to
                    // fall through, so it borrows the travel axis instead.
                    let tn = if y_span > 0.25 {
                        1.0 - (pos.y - y_lo) / y_span
                    } else {
                        (d.dot(travel) - lo) / span
                    };
                    let phase = (tn - t * cfg.speed * cfg.direction).rem_euclid(1.0);
                    let dist = phase.min(1.0 - phase);
                    let half = width * 0.5;
                    if dist > half {
                        continue;
                    }
                    soft_edge(cfg.soft, dist, half)
                }
                ChaseKind::Chevron => {
                    // Distance out from the middle of the rig, so both halves
                    // light as mirror images and the pair opens outward.
                    let tn = (d.dot(travel) - lo) / span;
                    let from_centre = (tn - 0.5).abs() * 2.0;
                    let mut centre = (t * cfg.speed).rem_euclid(1.0);
                    if cfg.direction < 0.0 {
                        centre = 1.0 - centre;
                    }
                    let dist = (from_centre - centre).abs();
                    let half = width * 0.5;
                    if dist > half {
                        continue;
                    }
                    soft_edge(cfg.soft, dist, half)
                }
                ChaseKind::Checker => {
                    // The rig cut into banks that alternate on each step —
                    // on two lights this is a straight ping-pong, on forty
                    // it's a blinder pattern.
                    let tn = (d.dot(travel) - lo) / span;
                    let banks = cfg.stripe_count.max(2) as f32;
                    let bank = (tn * banks).floor() as i64;
                    let step = t * cfg.speed.max(0.01);
                    if (bank + step.floor() as i64).rem_euclid(2) != 0 {
                        continue;
                    }
                    if cfg.soft {
                        // Fade each step in and out instead of snapping.
                        (std::f32::consts::PI * step.fract()).sin().max(0.0)
                    } else {
                        1.0
                    }
                }
                ChaseKind::Glitter => {
                    // Each fixture sparkles at random moments: `speed` flashes
                    // per second on average, band width = flash length.
                    let duty = (band_deg / 180.0).clamp(0.03, 0.95);
                    let u = t * cfg.speed.max(0.01) + rand01(fi as u32 * 7919 + 13);
                    let k = u.floor();
                    let frac = u - k;
                    let start = rand01(
                        (fi as u32).wrapping_mul(2654435761)
                            ^ (k as i64 as u32).wrapping_mul(40503),
                    ) * (1.0 - duty);
                    if frac < start || frac >= start + duty {
                        continue;
                    }
                    if cfg.soft {
                        (std::f32::consts::PI * (frac - start) / duty).sin()
                    } else {
                        1.0
                    }
                }
            };
            let Some(f) = patch.fixtures.get(fi) else {
                continue;
            };
            for addr in f.from..=f.to {
                let i = addr as usize;
                if (1..=net::DMX_SLOTS).contains(&i) {
                    weights.push((i - 1, w));
                }
            }
        }
        Layer::overlay(inject, weights)
    }
}

/// Band envelope: a cosine-ish pulse that peaks at the band centre and falls
/// to nothing at `half`, or a flat top when the edge is set to hard.
fn soft_edge(soft: bool, dist: f32, half: f32) -> f32 {
    if soft {
        (std::f32::consts::FRAC_PI_2 * (1.0 - dist / half.max(1e-4))).sin()
    } else {
        1.0
    }
}

/// Cheap deterministic hash → 0..1 for glitter sparkle timing.
pub(crate) fn rand01(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(747796405).wrapping_add(2891336453);
    x = ((x >> ((x >> 28) + 4)) ^ x).wrapping_mul(277803737);
    x = (x >> 22) ^ x;
    (x & 0xffff) as f32 / 65535.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::showbuddy::{Channel, Fixture};

    /// A rig of `n` single-channel fixtures in a line, optionally stacked at
    /// different heights so the vertical shapes have something to fall down.
    fn rig(n: usize, tiered: bool) -> (Patch, Vec<(usize, V3)>) {
        let mut fixtures = Vec::new();
        let mut positions = Vec::new();
        for i in 0..n {
            let addr = i as u16 + 1;
            fixtures.push(Fixture {
                display: format!("L{i}"),
                file: "test".into(),
                from: addr,
                to: addr,
                x: 0.5,
                y: 0.5,
                pan_range: 540.0,
                tilt_range: 270.0,
                beam_width: 25.0,
                channels: vec![Channel {
                    name: "Dimmer".into(),
                    bands: Vec::new(),
                    role: None,
                }],
            });
            let t = i as f32 / (n.max(2) - 1) as f32;
            positions.push((
                i,
                V3 {
                    x: (t - 0.5) * 10.0,
                    y: if tiered { t * 4.0 } else { 3.0 },
                    z: (t - 0.5) * 2.0,
                },
            ));
        }
        (Patch { fixtures, warnings: Vec::new() }, positions)
    }

    /// How many fixtures a chase is lighting at one instant.
    fn lit(run: &mut ChaseRun, cfg: &ChaseConfig, patch: &Patch, pos: &[(usize, V3)]) -> usize {
        run.layer(cfg, patch, pos)
            .weights()
            .iter()
            .filter(|(_, w)| *w > 0.05)
            .count()
    }

    /// Every shape has to actually do something, on a four-light bar and on a
    /// forty-light rig alike — and it has to leave some of the rig dark at
    /// least some of the time, or it isn't a chase, it's a wash.
    #[test]
    fn every_shape_moves_on_any_rig_size() {
        for (n, tiered) in [(4usize, false), (40, true)] {
            let (patch, pos) = rig(n, tiered);
            for kind in ChaseKind::ALL {
                let cfg = ChaseConfig { kind, speed: 0.4, ..Default::default() };
                let mut run = ChaseRun::new(Look::black());
                let (mut ever_lit, mut ever_dark) = (false, false);
                for _ in 0..32 {
                    run.rewind(Duration::from_millis(90));
                    let count = lit(&mut run, &cfg, &patch, &pos);
                    ever_lit |= count > 0;
                    ever_dark |= count < n;
                }
                assert!(ever_lit, "{kind:?} never lit anything on {n} lights");
                assert!(ever_dark, "{kind:?} lit the whole rig at every step on {n} lights");
            }
        }
    }

    /// The point of auto width: the band covers about the same number of
    /// lights whatever the rig size, instead of scaling with the room.
    #[test]
    fn auto_width_covers_the_same_few_lights_on_any_rig() {
        let target = 3;
        for n in [4usize, 12, 40] {
            let (patch, pos) = rig(n, false);
            let cfg = ChaseConfig {
                kind: ChaseKind::Linear,
                auto_width: true,
                target_lights: target,
                speed: 0.4,
                soft: false,
                ..Default::default()
            };
            let mut run = ChaseRun::new(Look::black());
            let mut peak = 0;
            for _ in 0..32 {
                run.rewind(Duration::from_millis(90));
                peak = peak.max(lit(&mut run, &cfg, &patch, &pos));
            }
            assert!(
                (1..=target as usize * 2).contains(&peak),
                "{n}-light rig lit {peak} at once, expected around {target}"
            );
        }
    }

    /// Cascade reads off the rig's height when there is one. A rig hung all
    /// at a single height has none, so it has to fall back to sweeping
    /// across rather than lighting everything at once.
    #[test]
    fn cascade_handles_a_flat_rig() {
        for tiered in [true, false] {
            let (patch, pos) = rig(12, tiered);
            let cfg = ChaseConfig { kind: ChaseKind::Cascade, speed: 0.4, ..Default::default() };
            let mut run = ChaseRun::new(Look::black());
            let (mut ever_lit, mut ever_dark) = (false, false);
            for _ in 0..32 {
                run.rewind(Duration::from_millis(90));
                let count = lit(&mut run, &cfg, &patch, &pos);
                ever_lit |= count > 0;
                ever_dark |= count < 12;
            }
            assert!(ever_lit && ever_dark, "cascade stalled on tiered={tiered}");
        }
    }
}
