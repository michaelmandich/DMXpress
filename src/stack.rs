//! Stacks — DMXpress's renamed grandMA3 *cue lists*.
//!
//! A stack is an ordered list of cues. Each cue stores only the channels the
//! programmer was actively holding when it was recorded (tracking: untouched
//! channels keep whatever the previous cue left them at). Values may be hard
//! numbers or *references* to a palette, so a cue can remember "Color Palette
//! 3". Playing a stack fades between the tracked output of consecutive cues and
//! contributes one [`Layer`] to the mixer, beneath the programmer.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::engine::Layer;
use crate::net::{Frame, DMX_SLOTS};
use crate::palette::PaletteRef;

const STACKS_FILE: &str = "stacks.json";

/// A stored channel value: a hard number, or a palette reference with the value
/// it resolved to when recorded (used as a fallback if the palette is gone).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CueVal {
    Absolute(u8),
    Palette { reference: PaletteRef, value: u8 },
}

/// One recorded step of a stack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cue {
    pub number: f32,
    pub name: String,
    /// Fade time in seconds used when this cue is fired.
    pub fade: f32,
    /// 0-based DMX index → stored value. Only the channels this cue changes.
    pub values: Vec<(usize, CueVal)>,
}

/// A timed crossfade between two output frames.
#[derive(Clone)]
struct CueFade {
    from: Frame,
    to: Frame,
    started: Instant,
    dur: f32,
}

impl CueFade {
    /// Current blended frame and whether the fade has finished.
    fn frame(&self) -> (Frame, bool) {
        let t = if self.dur <= 0.0 {
            1.0
        } else {
            (self.started.elapsed().as_secs_f32() / self.dur).clamp(0.0, 1.0)
        };
        let mut f = self.from;
        for i in 0..DMX_SLOTS {
            f.blend_channel(i, self.to.0[i], t);
        }
        (f, t >= 1.0)
    }
}

fn full_level() -> f32 {
    1.0
}

/// A cue list plus its live playback state (the runtime fields are not saved).
#[derive(Clone, Serialize, Deserialize)]
pub struct Stack {
    pub name: String,
    pub cues: Vec<Cue>,
    /// Index of the cue currently standing on stage (None = stack not started).
    #[serde(skip)]
    pub current: Option<usize>,
    #[serde(skip)]
    run: Option<CueFade>,
    /// Settled output of the current cue (held when no fade is running).
    #[serde(skip)]
    settled: Frame,
    /// Output master 0..1 (the executor fader, driven by a Deck in Phase 5).
    #[serde(skip, default = "full_level")]
    pub level: f32,
}

impl Stack {
    pub fn new(name: String) -> Self {
        Self {
            name,
            cues: Vec::new(),
            current: None,
            run: None,
            settled: Frame::black(),
            level: 1.0,
        }
    }

    /// Jump to cue `idx`, fading its `frame` in over `fade` seconds.
    pub fn fire(&mut self, idx: usize, frame: Frame, fade: f32) {
        self.current = Some(idx);
        if fade <= 0.0 {
            self.settled = frame;
            self.run = None;
        } else {
            self.run = Some(CueFade {
                from: self.settled,
                to: frame,
                started: Instant::now(),
                dur: fade,
            });
        }
    }

    /// Whether the stack is currently fading (needs fast repaints).
    pub fn is_fading(&self) -> bool {
        self.run.is_some()
    }

    /// Keep an in-flight cue fade at the exact sample held during a global
    /// transport freeze.
    pub fn resume_after(&mut self, paused: Duration) {
        if let Some(run) = &mut self.run {
            run.started += paused;
        }
    }

    /// Stop playing: release the stack so it asserts nothing.
    pub fn release(&mut self) {
        self.current = None;
        self.run = None;
    }

    /// Channels the stack asserts: everything its cues up to `current` touch.
    fn covered(&self) -> Vec<usize> {
        let upto = self.current.map(|c| c + 1).unwrap_or(0);
        let mut set = std::collections::BTreeSet::new();
        for c in self.cues.iter().take(upto) {
            for &(a, _) in &c.values {
                set.insert(a);
            }
        }
        set.into_iter().collect()
    }

    /// This stack's contribution to the output frame, or `None` when it is not
    /// playing. The covered channels are blended at the stack's master level so
    /// faders work and untouched channels fall through to other layers.
    pub fn render_layer(&mut self) -> Option<Layer> {
        if self.current.is_none() {
            return None;
        }
        let frame = if let Some(run) = &self.run {
            let (f, done) = run.frame();
            if done {
                self.settled = f;
                self.run = None;
            }
            f
        } else {
            self.settled
        };
        let level = self.level.clamp(0.0, 1.0);
        let weights: Vec<(usize, f32)> = self.covered().into_iter().map(|a| (a, level)).collect();
        Some(Layer::overlay(frame, weights))
    }
}

/// Load saved stacks (empty if the file is missing or unreadable).
pub fn load_stacks() -> Vec<Stack> {
    std::fs::read_to_string(STACKS_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist stacks to disk (runtime playback state is skipped).
pub fn save_stacks(stacks: &[Stack]) {
    if let Ok(json) = serde_json::to_string_pretty(stacks) {
        let _ = std::fs::write(STACKS_FILE, json);
    }
}
