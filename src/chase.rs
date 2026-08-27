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
}

impl ChaseKind {
    pub const ALL: [ChaseKind; 6] = [
        ChaseKind::Sphere,
        ChaseKind::Linear,
        ChaseKind::Boomerang,
        ChaseKind::Stripes,
        ChaseKind::Glitter,
        ChaseKind::Pulse,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ChaseKind::Sphere => "Sphere",
            ChaseKind::Linear => "Linear",
            ChaseKind::Boomerang => "Boomerang",
            ChaseKind::Stripes => "Stripes",
            ChaseKind::Glitter => "Glitter",
            ChaseKind::Pulse => "Pulse",
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
    /// Number of simultaneous repeated bands in Stripe mode.
    pub stripe_count: u32,
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
            speed: 0.3,
            direction: 1.0,
            soft: true,
            expanded: false,
            selected: false,
            active_head: None,
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

        // Linear/pulse: normalise fixture projections across the rig extent.
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        if matches!(
            cfg.kind,
            ChaseKind::Linear | ChaseKind::Boomerang | ChaseKind::Stripes | ChaseKind::Pulse
        ) {
            for &(_, pos) in fixture_positions {
                let proj = (pos - cfg.sphere.pos).dot(travel);
                lo = lo.min(proj);
                hi = hi.max(proj);
            }
        }
        let span = (hi - lo).max(1e-3);
        // Band width as a fraction of the rig for linear kinds.
        let width = (cfg.band_deg.clamp(2.0, 180.0) / 180.0).clamp(0.02, 1.0);
        let head = self.head(cfg);

        let mut weights: Vec<(usize, f32)> = Vec::new();
        for &(fi, pos) in fixture_positions {
            let d = pos - cfg.sphere.pos;
            let w = match cfg.kind {
                ChaseKind::Sphere => {
                    let half =
                        (cfg.band_deg.clamp(2.0, 180.0) / 360.0 * 0.5).max(0.001);
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
                ChaseKind::Glitter => {
                    // Each fixture sparkles at random moments: `speed` flashes
                    // per second on average, band width = flash length.
                    let duty = (cfg.band_deg / 180.0).clamp(0.03, 0.95);
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

/// Cheap deterministic hash → 0..1 for glitter sparkle timing.
pub(crate) fn rand01(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(747796405).wrapping_add(2891336453);
    x = ((x >> ((x >> 28) + 4)) ^ x).wrapping_mul(277803737);
    x = (x >> 22) ^ x;
    (x & 0xffff) as f32 / 65535.0
}
