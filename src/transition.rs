//! Preset transition options and the active transition renderer.
//!
//! A transition owns ticking copies of the outgoing and incoming oscillator
//! engines so animated presets can drift through the blend instead of jumping
//! to static snapshots.
//!
//! The console has one *global transition* time — the Transition window's top
//! slider, kept in [`TransitionConfig::duration`] — that every fading action
//! follows by default, so a whole show goes from hard cuts to slow dissolves
//! on one fader. Advanced mode breaks that out into a slot per action (see
//! [`TransitionTarget`]), each of which can follow the global, run its own
//! time, or cut outright. Every fade time in the console lives here: the pool
//! windows have no fade faders of their own.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::net::{self, Frame};
use crate::oscillator::Look;
use crate::showbuddy::Patch;
use crate::stage::{dir_from_angles, v3, V3};

/// Reusable transition selector placed beside any live control. This is the
/// common contract for knobs, buttons, palettes, phasers, and future effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) enum TransitionBinding {
    /// Follow the global transition slider.
    #[default]
    Master,
    /// Use this slot's own duration.
    Custom,
    /// Never fade — cut.
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

    pub fn hint(self) -> &'static str {
        match self {
            Self::Master => "Follows the global transition slider",
            Self::Custom => "Runs this row's own time",
            Self::None => "Cuts — never fades",
        }
    }
}

/// The panel a [`TransitionTarget`] belongs to — the headings advanced mode
/// groups its rows under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransitionGroup {
    Palettes,
    Phasers,
    Chases,
    Scenes,
    Cues,
    Presets,
    Attributes,
    Programmer,
    Masters,
    Layers,
}

impl TransitionGroup {
    pub const ALL: [Self; 10] = [
        Self::Palettes,
        Self::Phasers,
        Self::Chases,
        Self::Scenes,
        Self::Cues,
        Self::Presets,
        Self::Attributes,
        Self::Programmer,
        Self::Masters,
        Self::Layers,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Palettes => "Palettes",
            Self::Phasers => "Phasers",
            Self::Chases => "Chases",
            Self::Scenes => "Scenes",
            Self::Cues => "Cues",
            Self::Presets => "Presets",
            Self::Attributes => "Attributes",
            Self::Programmer => "Programmer",
            Self::Masters => "Masters",
            Self::Layers => "Layers",
        }
    }
}

/// Every action the console can fade instead of jump. Each owns a slot in
/// [`TransitionConfig`]; advanced mode shows one row per target.
///
/// Declaration order is both the order advanced mode lists them in and — via
/// [`TransitionTarget::idx`], which casts the variant — the order of the slot
/// array. `targets_are_in_slot_order` holds the two together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TransitionTarget {
    PaletteRecall,
    PaletteRelease,
    CycleIn,
    CycleOut,
    PhaserStart,
    PhaserStop,
    PhaserEdit,
    PhaserPose,
    PhaserHold,
    ChaseStart,
    ChaseStop,
    SceneStart,
    SceneStop,
    CueGo,
    CueRelease,
    PresetRecall,
    PresetRelease,
    GoboChange,
    ChannelJump,
    ProgrammerClear,
    EffectsClear,
    EncoderRelease,
    BlackoutIn,
    BlackoutOut,
    /// A programmer layer being brought up.
    LayerGo,
    /// A programmer layer being taken out.
    LayerRelease,
}

impl TransitionTarget {
    pub const COUNT: usize = 26;

    pub const ALL: [Self; Self::COUNT] = [
        Self::PaletteRecall,
        Self::PaletteRelease,
        Self::CycleIn,
        Self::CycleOut,
        Self::PhaserStart,
        Self::PhaserStop,
        Self::PhaserEdit,
        Self::PhaserPose,
        Self::PhaserHold,
        Self::ChaseStart,
        Self::ChaseStop,
        Self::SceneStart,
        Self::SceneStop,
        Self::CueGo,
        Self::CueRelease,
        Self::PresetRecall,
        Self::PresetRelease,
        Self::GoboChange,
        Self::ChannelJump,
        Self::ProgrammerClear,
        Self::EffectsClear,
        Self::EncoderRelease,
        Self::BlackoutIn,
        Self::BlackoutOut,
        Self::LayerGo,
        Self::LayerRelease,
    ];

    /// This target's index into the slot array.
    pub fn idx(self) -> usize {
        self as usize
    }

    pub fn group(self) -> TransitionGroup {
        match self {
            Self::PaletteRecall | Self::PaletteRelease | Self::CycleIn | Self::CycleOut => {
                TransitionGroup::Palettes
            }
            Self::PhaserStart
            | Self::PhaserStop
            | Self::PhaserEdit
            | Self::PhaserPose
            | Self::PhaserHold => TransitionGroup::Phasers,
            Self::ChaseStart | Self::ChaseStop => TransitionGroup::Chases,
            Self::SceneStart | Self::SceneStop => TransitionGroup::Scenes,
            Self::CueGo | Self::CueRelease => TransitionGroup::Cues,
            Self::PresetRecall | Self::PresetRelease => TransitionGroup::Presets,
            Self::GoboChange | Self::ChannelJump => TransitionGroup::Attributes,
            Self::ProgrammerClear | Self::EffectsClear | Self::EncoderRelease => {
                TransitionGroup::Programmer
            }
            Self::BlackoutIn | Self::BlackoutOut => TransitionGroup::Masters,
            Self::LayerGo | Self::LayerRelease => TransitionGroup::Layers,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PaletteRecall => "Recall",
            Self::PaletteRelease => "Release",
            Self::CycleIn => "Cycle in",
            Self::CycleOut => "Cycle out",
            Self::PhaserStart => "Start",
            Self::PhaserStop => "Stop",
            Self::PhaserEdit => "Live edit",
            Self::PhaserPose => "Pose / FX",
            Self::PhaserHold => "Hold",
            Self::ChaseStart => "Start",
            Self::ChaseStop => "Stop",
            Self::SceneStart => "Go",
            Self::SceneStop => "Release",
            Self::CueGo => "Go",
            Self::CueRelease => "Release",
            Self::PresetRecall => "Recall",
            Self::PresetRelease => "Release",
            Self::GoboChange => "Gobo / wheel slot",
            Self::ChannelJump => "Channel jump",
            Self::ProgrammerClear => "Clear programmer",
            Self::EffectsClear => "Clear effects",
            Self::EncoderRelease => "Encoder release",
            Self::BlackoutIn => "Blackout on",
            Self::BlackoutOut => "Blackout off",
            Self::LayerGo => "Layer in",
            Self::LayerRelease => "Layer out",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::PaletteRecall => "Dropping a palette into the programmer",
            Self::PaletteRelease => "Taking a palette back out of the programmer",
            Self::CycleIn => "Bringing the palette cycle up",
            Self::CycleOut => "Taking the palette cycle away",
            Self::PhaserStart => "Applying a wave or flat-add phaser",
            Self::PhaserStop => "Stopping one — its depth eases back to the base",
            Self::PhaserEdit => "Re-applying while the editor is live; keep this short",
            Self::PhaserPose => "Recalling a static pose or FX state",
            Self::PhaserHold => "Forced holds (smoke, strobe on) coming and going",
            Self::ChaseStart => "Injecting the chase over the programmer",
            Self::ChaseStop => "Pulling the chase back off",
            Self::SceneStart => "A captured scene arriving on stage",
            Self::SceneStop => "Releasing a scene",
            Self::CueGo => "Go on a cue stack",
            Self::CueRelease => "Releasing a stack back to the programmer",
            Self::PresetRecall => "Firing a preset from a pad or the pool",
            Self::PresetRelease => "Dropping the active preset",
            Self::GoboChange => "Gobo and colour-wheel slots; stepped wheels snap midway",
            Self::ChannelJump => "Buttons and typed channel values; hand drags stay instant",
            Self::ProgrammerClear => "Emptying the programmer",
            Self::EffectsClear => "The Clear button's effects stage",
            Self::EncoderRelease => "Dropping the encoder layer",
            Self::BlackoutIn => "Going to black",
            Self::BlackoutOut => "Coming back up",
            Self::LayerGo => "Bringing a new programmer layer up",
            Self::LayerRelease => "Taking a layer out — it keeps playing, thinning away",
        }
    }

    /// Stable key for the show file, so reordering or adding targets later
    /// leaves saved desks readable.
    pub fn key(self) -> &'static str {
        match self {
            Self::PaletteRecall => "palette_recall",
            Self::PaletteRelease => "palette_release",
            Self::CycleIn => "cycle_in",
            Self::CycleOut => "cycle_out",
            Self::PhaserStart => "phaser_start",
            Self::PhaserStop => "phaser_stop",
            Self::PhaserEdit => "phaser_edit",
            Self::PhaserPose => "phaser_pose",
            Self::PhaserHold => "phaser_hold",
            Self::ChaseStart => "chase_start",
            Self::ChaseStop => "chase_stop",
            Self::SceneStart => "scene_start",
            Self::SceneStop => "scene_stop",
            Self::CueGo => "cue_go",
            Self::CueRelease => "cue_release",
            Self::PresetRecall => "preset_recall",
            Self::PresetRelease => "preset_release",
            Self::GoboChange => "gobo_change",
            Self::ChannelJump => "channel_jump",
            Self::ProgrammerClear => "programmer_clear",
            Self::EffectsClear => "effects_clear",
            Self::EncoderRelease => "encoder_release",
            Self::BlackoutIn => "blackout_in",
            Self::BlackoutOut => "blackout_out",
            Self::LayerGo => "layer_go",
            Self::LayerRelease => "layer_release",
        }
    }

    /// What this slot starts as. Nearly everything follows the global, so the
    /// one slider means something the moment it is touched. The exceptions are
    /// the controls an operator grabs when something is wrong — forced holds
    /// and blackout — which cut until they are deliberately bound.
    fn default_slot(self) -> TransitionSlot {
        let (binding, custom) = match self {
            Self::PhaserHold | Self::BlackoutIn | Self::BlackoutOut => {
                (TransitionBinding::None, 0.0)
            }
            Self::PhaserEdit => (TransitionBinding::Master, 0.3),
            Self::GoboChange => (TransitionBinding::Master, 0.4),
            Self::ChannelJump => (TransitionBinding::Master, 0.5),
            Self::CycleIn | Self::CycleOut => (TransitionBinding::Master, 2.0),
            Self::SceneStart | Self::SceneStop | Self::CueGo | Self::CueRelease => {
                (TransitionBinding::Master, 3.0)
            }
            // A layer coming or going is a scene-sized gesture, not a nudge.
            Self::LayerGo | Self::LayerRelease => (TransitionBinding::Master, 2.0),
            _ => (TransitionBinding::Master, 1.5),
        };
        TransitionSlot { binding, custom }
    }
}

/// One action's transition setting: which clock it follows, and the time it
/// runs when it follows itself.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct TransitionSlot {
    pub binding: TransitionBinding,
    pub custom: f32,
}

/// The whole desk as it goes into a show file. Slots are keyed by
/// [`TransitionTarget::key`] so a file written by an older build simply leaves
/// the newer targets at their defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransitionFile {
    #[serde(default)]
    pub global: f32,
    #[serde(default)]
    pub advanced: bool,
    #[serde(default)]
    pub mode: TransitionMode,
    #[serde(default)]
    pub curve: TransitionCurve,
    #[serde(default)]
    pub slots: BTreeMap<String, TransitionSlot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) enum TransitionMode {
    #[default]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) enum TransitionCurve {
    Linear,
    #[default]
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
    /// The global transition: the one time every action follows unless its
    /// own slot says otherwise. 0 = cut immediately; max UI value is 20 s.
    pub duration: f32,
    /// Advanced mode: show (and use) a slot per action instead of running the
    /// whole console off the global slider.
    pub advanced: bool,
    /// Per-action settings, indexed by [`TransitionTarget::idx`].
    pub slots: [TransitionSlot; TransitionTarget::COUNT],
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
            advanced: false,
            slots: default_slots(),
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

fn default_slots() -> [TransitionSlot; TransitionTarget::COUNT] {
    let mut slots = [TransitionSlot {
        binding: TransitionBinding::Master,
        custom: 1.5,
    }; TransitionTarget::COUNT];
    for t in TransitionTarget::ALL {
        slots[t.idx()] = t.default_slot();
    }
    slots
}

impl TransitionConfig {
    pub fn stage_visible(&self) -> bool {
        self.expanded && self.mode.uses_sphere()
    }

    /// How long `target` should take. Simple mode runs the whole console off
    /// the global slider; advanced mode lets each slot answer for itself.
    pub fn fade(&self, target: TransitionTarget) -> f32 {
        if !self.advanced {
            return self.duration.max(0.0);
        }
        let slot = self.slots[target.idx()];
        slot.binding.duration(self.duration, slot.custom).max(0.0)
    }

    pub fn slot_mut(&mut self, target: TransitionTarget) -> &mut TransitionSlot {
        &mut self.slots[target.idx()]
    }

    /// Put every slot on one binding — the advanced header's quick actions.
    pub fn bind_all(&mut self, binding: TransitionBinding) {
        for slot in &mut self.slots {
            slot.binding = binding;
        }
    }

    pub fn reset_slots(&mut self) {
        self.slots = default_slots();
    }

    /// The desk as it goes into a show file.
    pub fn to_file(&self) -> TransitionFile {
        TransitionFile {
            global: self.duration,
            advanced: self.advanced,
            mode: self.mode,
            curve: self.curve,
            slots: TransitionTarget::ALL
                .iter()
                .map(|t| (t.key().to_owned(), self.slots[t.idx()]))
                .collect(),
        }
    }

    /// Take a desk back out of a show file. Targets the file does not mention
    /// — because it was written before they existed — keep their defaults.
    pub fn apply_file(&mut self, file: TransitionFile) {
        self.duration = file.global.clamp(0.0, 20.0);
        self.advanced = file.advanced;
        self.mode = file.mode;
        self.curve = file.curve;
        self.slots = default_slots();
        for t in TransitionTarget::ALL {
            if let Some(slot) = file.slots.get(t.key()) {
                self.slots[t.idx()] = TransitionSlot {
                    binding: slot.binding,
                    custom: slot.custom.clamp(0.0, 20.0),
                };
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `idx` casts the variant, so `ALL` must list the targets in declaration
    /// order or every slot would answer for the wrong action.
    #[test]
    fn targets_are_in_slot_order() {
        for (i, t) in TransitionTarget::ALL.iter().enumerate() {
            assert_eq!(t.idx(), i, "{t:?} is out of order in ALL");
        }
    }

    #[test]
    fn target_keys_are_unique() {
        let mut keys: Vec<&str> = TransitionTarget::ALL.iter().map(|t| t.key()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "two targets share a show-file key");
    }

    /// Simple mode is the whole point of the global slider: every action
    /// follows it, whatever the advanced slots happen to say.
    #[test]
    fn simple_mode_runs_everything_off_the_global() {
        let mut cfg = TransitionConfig {
            duration: 4.0,
            ..Default::default()
        };
        cfg.slot_mut(TransitionTarget::PaletteRecall).binding = TransitionBinding::None;
        cfg.slot_mut(TransitionTarget::BlackoutIn).binding = TransitionBinding::None;
        for t in TransitionTarget::ALL {
            assert_eq!(cfg.fade(t), 4.0, "{} ignored the global", t.label());
        }
    }

    #[test]
    fn advanced_mode_lets_each_slot_answer() {
        let mut cfg = TransitionConfig {
            duration: 4.0,
            advanced: true,
            ..Default::default()
        };
        assert_eq!(cfg.fade(TransitionTarget::PaletteRecall), 4.0, "defaults to master");
        assert_eq!(cfg.fade(TransitionTarget::BlackoutIn), 0.0, "blackout cuts by default");

        let slot = cfg.slot_mut(TransitionTarget::PaletteRecall);
        slot.binding = TransitionBinding::Custom;
        slot.custom = 0.75;
        assert_eq!(cfg.fade(TransitionTarget::PaletteRecall), 0.75);
        assert_eq!(cfg.fade(TransitionTarget::PhaserStart), 4.0, "its neighbour is untouched");

        cfg.slot_mut(TransitionTarget::PaletteRecall).binding = TransitionBinding::None;
        assert_eq!(cfg.fade(TransitionTarget::PaletteRecall), 0.0);
    }

    /// A desk saved into a show comes back the same, and a file written
    /// before a target existed leaves that target on its default.
    #[test]
    fn desk_round_trips_through_a_show_file() {
        let mut cfg = TransitionConfig {
            duration: 6.5,
            advanced: true,
            mode: TransitionMode::Radial,
            curve: TransitionCurve::Linear,
            ..Default::default()
        };
        let slot = cfg.slot_mut(TransitionTarget::CueGo);
        slot.binding = TransitionBinding::Custom;
        slot.custom = 12.0;

        let mut file = cfg.to_file();
        file.slots.remove(TransitionTarget::PhaserStop.key());

        let mut back = TransitionConfig::default();
        back.apply_file(file);
        assert_eq!(back.duration, 6.5);
        assert!(back.advanced);
        assert_eq!(back.mode, TransitionMode::Radial);
        assert_eq!(back.curve, TransitionCurve::Linear);
        assert_eq!(back.fade(TransitionTarget::CueGo), 12.0);
        assert_eq!(
            back.slots[TransitionTarget::PhaserStop.idx()].binding,
            TransitionBinding::Master,
            "a target the file predates keeps its default"
        );
    }

    #[test]
    fn bind_all_sweeps_every_slot() {
        let mut cfg = TransitionConfig {
            duration: 2.0,
            advanced: true,
            ..Default::default()
        };
        cfg.bind_all(TransitionBinding::None);
        for t in TransitionTarget::ALL {
            assert_eq!(cfg.fade(t), 0.0, "{} still fades", t.label());
        }
        cfg.bind_all(TransitionBinding::Master);
        for t in TransitionTarget::ALL {
            assert_eq!(cfg.fade(t), 2.0, "{} did not pick the global up", t.label());
        }
    }
}
