//! Scenes — captured effect states that layer instead of replacing.
//!
//! A preset recalls *into* the programmer, so two of them fight: the
//! programmer holds one oscillator per DMX address, and the second recall
//! evicts the first. A scene stores the same snapshot but renders it through
//! its own [`Look`] into its own mixer layer, so several can run at once with
//! a settable priority and level.
//!
//! The phase of every oscillator is baked in at capture time, which is what
//! makes the whole thing work: a wave rolling left-to-right and one rolling
//! right-to-left keep their own fan and genuinely coexist, rather than
//! arguing over one global spread.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::engine::{Blend, Layer};
use crate::net::Frame;
use crate::oscillator::{Look, Osc};
use crate::preset::SavedOsc;

const SCENES_FILE: &str = "scenes.json";

/// How a running scene folds into the output beneath it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MergeMode {
    /// Take the channel over entirely at full level (LTP).
    #[default]
    Override,
    /// Keep whichever is brighter, so scenes coexist instead of erasing.
    Highest,
    /// Sum onto what is already there.
    Add,
}

impl MergeMode {
    pub const ALL: [Self; 3] = [Self::Override, Self::Highest, Self::Add];

    pub fn label(self) -> &'static str {
        match self {
            Self::Override => "Override",
            Self::Highest => "Highest",
            Self::Add => "Add",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Override => "Takes the channel over from every scene below it",
            Self::Highest => "Keeps whichever is brighter, so both scenes show",
            Self::Add => "Sums onto the scenes below, clamped at full",
        }
    }

    fn blend(self) -> Blend {
        match self {
            Self::Override => Blend::Mix,
            Self::Highest => Blend::Max,
            Self::Add => Blend::Add,
        }
    }
}

/// Live playback state, rebuilt on every Go and never saved.
#[derive(Clone)]
pub(crate) struct SceneRun {
    look: Look,
    started: Instant,
}

/// One captured effect state.
#[derive(Clone, Serialize, Deserialize)]
pub struct Scene {
    pub name: String,
    #[serde(default = "default_color")]
    pub color: [u8; 3],
    /// Asserted base values (0-based address → value).
    pub values: Vec<(usize, u8)>,
    /// Oscillators, with the phase they held when captured.
    #[serde(default)]
    pub oscs: Vec<(usize, SavedOsc)>,
    /// Channels this scene asserts. Everything else passes straight through.
    pub active: Vec<usize>,
    pub speed: f32,
    pub tempo: f32,
    #[serde(default = "default_master_speed")]
    pub master_speed: f32,
    #[serde(default)]
    pub merge: MergeMode,
    /// Output level 0..1.
    #[serde(default = "default_master_speed")]
    pub level: f32,
    /// Seconds to fade in when started.
    #[serde(default)]
    pub fade: f32,
    /// Seconds on stage before the chain hands to the next scene;
    /// 0 = stay until stopped.
    #[serde(default)]
    pub hold: f32,
    /// Name of the effect route active when this was captured. Informational
    /// — the oscillator phases already carry the fan — but handy for getting
    /// back to the route that produced it.
    #[serde(default)]
    pub order: Option<String>,
    #[serde(skip)]
    pub(crate) run: Option<SceneRun>,
}

fn default_color() -> [u8; 3] {
    [90, 150, 155]
}

fn default_master_speed() -> f32 {
    1.0
}

impl Scene {
    /// Rebuild the base frame (black everywhere the scene says nothing).
    fn base_frame(&self) -> Frame {
        let mut f = Frame::black();
        for &(a, v) in &self.values {
            if a < f.len() {
                f[a] = v;
            }
        }
        f
    }

    fn osc_map(&self) -> HashMap<usize, Osc> {
        self.oscs
            .iter()
            .map(|(a, o)| {
                (
                    *a,
                    Osc {
                        enabled: true,
                        invert: o.invert,
                        amount: o.amount,
                        phase: o.phase,
                        subdiv: o.subdiv,
                        shape: o.shape,
                        master_beat: o.master_beat,
                        local_beats: o.local_beats,
                        local_tempo: o.local_tempo,
                        custom_wave: o.custom_wave.clone(),
                    },
                )
            })
            .collect()
    }

    pub fn is_running(&self) -> bool {
        self.run.is_some()
    }

    pub fn is_animated(&self) -> bool {
        !self.oscs.is_empty()
    }

    /// Start playing, from the phase the scene was captured at.
    pub fn start(&mut self) {
        let mut look = Look::from_frame(self.base_frame());
        look.oscs = self.osc_map();
        look.speed = self.speed;
        look.tempo = self.tempo;
        look.master_speed = self.master_speed;
        self.run = Some(SceneRun {
            look,
            started: Instant::now(),
        });
    }

    pub fn stop(&mut self) {
        self.run = None;
    }

    /// Seconds this scene has been on stage.
    pub fn elapsed(&self) -> f32 {
        self.run
            .as_ref()
            .map_or(0.0, |r| r.started.elapsed().as_secs_f32())
    }

    /// Whether the hold time has run out, so a chain should move on.
    pub fn expired(&self) -> bool {
        self.hold > 0.0 && self.is_running() && self.elapsed() >= self.hold
    }

    /// Current output weight: the level, eased in over the fade time.
    pub fn gain(&self) -> f32 {
        let level = self.level.clamp(0.0, 1.0);
        if self.fade <= 0.0 {
            return level;
        }
        level * (self.elapsed() / self.fade).clamp(0.0, 1.0)
    }

    /// Whether the fade-in is still moving (so the UI keeps repainting).
    pub fn is_fading(&self) -> bool {
        self.is_running() && self.fade > 0.0 && self.elapsed() < self.fade
    }

    /// Advance the scene's clock and hand the mixer its contribution.
    pub(crate) fn layer(&mut self) -> Option<Layer> {
        let gain = self.gain();
        let blend = self.merge.blend();
        let run = self.run.as_mut()?;
        if gain <= 0.0 {
            // Still render so the clock keeps running and the scene does not
            // jump when the level comes back up.
            run.look.render();
            return None;
        }
        let frame = run.look.render();
        let weights: Vec<(usize, f32)> = self.active.iter().map(|&a| (a, gain)).collect();
        if weights.is_empty() {
            return None;
        }
        Some(Layer::overlay(frame, weights).with_blend(blend))
    }

    /// Move the wall-clock origins past a transport freeze.
    pub fn resume_after(&mut self, paused: Duration) {
        if let Some(run) = &mut self.run {
            run.look.resume_clock();
            run.started += paused;
        }
    }
}

pub fn load_scenes() -> Vec<Scene> {
    std::fs::read_to_string(SCENES_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_scenes(scenes: &[Scene]) {
    if let Ok(json) = serde_json::to_string_pretty(scenes) {
        let _ = std::fs::write(SCENES_FILE, json);
    }
}
