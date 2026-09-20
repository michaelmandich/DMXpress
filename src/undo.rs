//! Global undo — Ctrl+Shift+Z takes back the last thing you did, anywhere.
//!
//! Rather than teaching every mutation site in the console to record an
//! inverse, the app snapshots its whole undoable state — the show
//! configuration, the programmer, the running effects, the deck's pads —
//! and notices when it changed. A change only becomes a step once the input
//! has gone quiet for a moment, so a slider drag or a typed name is one
//! step, not a hundred. The last [`DEPTH`] steps are kept, and every undo
//! is itself redoable with Ctrl+Shift+Y.
//!
//! The snapshot is JSON: it costs a few milliseconds a few times a second,
//! and buys a single restore path (`apply_snapshot`) that the backups and
//! show import reuse, so there is one way to put a show back.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::net::{Frame, DMX_SLOTS};

/// How many steps back Ctrl+Shift+Z can go.
pub const DEPTH: usize = 10;
/// How long the input has to be quiet before a change counts as a step.
pub const SETTLE: Duration = Duration::from_millis(350);
/// How often the state is compared while the input is quiet.
pub const CHECK_EVERY: Duration = Duration::from_millis(250);

/// Everything Ctrl+Shift+Z can put back. Maps are stored as sorted lists so
/// the same state always serializes to the same bytes.
#[derive(Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub show: crate::config::Configuration,
    pub live_base: Vec<u8>,
    pub live_oscs: Vec<(usize, crate::oscillator::Osc)>,
    pub live_master_speed: f32,
    pub live_speed: f32,
    pub live_tempo: f32,
    pub live_active: Vec<usize>,
    pub live_refs: Vec<(usize, crate::palette::PaletteRef)>,
    pub active_phasers: Vec<(String, Vec<usize>)>,
    pub effect_lanes: Vec<(String, Vec<u32>)>,
    pub hold_overrides: Vec<(usize, u8)>,
    pub encoder_layer: Vec<(usize, u8)>,
    pub active_preset: Option<(usize, usize)>,
    pub phaser_deck: Vec<Option<crate::streamdeck::PhaserSlot>>,
    /// The Presets board's pads. Absent in steps saved before the board existed.
    #[serde(default)]
    pub preset_deck: Vec<Option<crate::preset_deck::PresetSlot>>,
}

/// One recorded state: the serialized snapshot and when it was replaced.
pub struct Step {
    pub json: Vec<u8>,
    pub taken: Instant,
}

/// The undo/redo stacks and the change detector that feeds them.
pub struct History {
    /// The state as last seen: its fingerprint and the bytes to restore it.
    committed: Option<(u64, Vec<u8>)>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    last_check: Instant,
    last_input: Instant,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    pub fn new() -> Self {
        Self {
            committed: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_check: Instant::now(),
            last_input: Instant::now(),
        }
    }

    /// The person is still doing something: hold off committing.
    pub fn note_input(&mut self) {
        self.last_input = Instant::now();
    }

    pub fn quiet(&self) -> bool {
        self.last_input.elapsed() >= SETTLE
    }

    pub fn due(&self) -> bool {
        self.last_check.elapsed() >= CHECK_EVERY
    }

    /// Fingerprint of the state as last seen, if any.
    pub fn hash(&self) -> Option<u64> {
        self.committed.as_ref().map(|(h, _)| *h)
    }

    /// Compare a fresh snapshot with the last one seen. The first call just
    /// remembers; afterwards a difference records the previous state as a
    /// step and empties the redo side. Returns whether a step was recorded.
    pub fn observe(&mut self, json: Vec<u8>) -> bool {
        self.last_check = Instant::now();
        let hash = fingerprint(&json);
        match self.committed.take() {
            Some((prev_hash, prev)) if prev_hash == hash => {
                self.committed = Some((prev_hash, prev));
                false
            }
            Some((_, prev)) => {
                self.undo.push(Step { json: prev, taken: Instant::now() });
                if self.undo.len() > DEPTH {
                    self.undo.remove(0);
                }
                self.redo.clear();
                self.committed = Some((hash, json));
                true
            }
            None => {
                self.committed = Some((hash, json));
                false
            }
        }
    }

    /// Take one step back: the snapshot to restore. The state being left
    /// goes to the redo side.
    pub fn undo(&mut self) -> Option<Step> {
        let step = self.undo.pop()?;
        if let Some((_, cur)) = self.committed.take() {
            self.redo.push(Step { json: cur, taken: Instant::now() });
        }
        Some(step)
    }

    /// Reapply the last undone step.
    pub fn redo(&mut self) -> Option<Step> {
        let step = self.redo.pop()?;
        if let Some((_, cur)) = self.committed.take() {
            self.undo.push(Step { json: cur, taken: Instant::now() });
        }
        Some(step)
    }

    /// After a restore: this is the state now, and it is not a new step.
    pub fn settle(&mut self, json: Vec<u8>) {
        self.committed = Some((fingerprint(&json), json));
        self.last_check = Instant::now();
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }
}

/// A stable 64-bit digest of a snapshot's bytes.
pub fn fingerprint(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// "12 s", "3 min", "2 h" — how long ago a step was taken.
pub fn ago(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h", s / 3600)
    }
}

impl App {
    /// Everything the undo history and the backups care about.
    pub(crate) fn capture_snapshot(&self) -> Snapshot {
        let mut live_oscs: Vec<(usize, crate::oscillator::Osc)> =
            self.live.oscs.iter().map(|(k, v)| (*k, v.clone())).collect();
        live_oscs.sort_by_key(|(k, _)| *k);
        let mut live_active: Vec<usize> = self.live_active.iter().copied().collect();
        live_active.sort_unstable();
        let mut live_refs: Vec<(usize, crate::palette::PaletteRef)> =
            self.live_refs.iter().map(|(k, v)| (*k, *v)).collect();
        live_refs.sort_by_key(|(k, _)| *k);
        let mut active_phasers: Vec<(String, Vec<usize>)> = self
            .active_phasers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        active_phasers.sort_by(|a, b| a.0.cmp(&b.0));
        let mut hold_overrides: Vec<(usize, u8)> =
            self.hold_overrides.iter().map(|(k, v)| (*k, *v)).collect();
        hold_overrides.sort_unstable();
        let mut encoder_layer: Vec<(usize, u8)> =
            self.encoder_layer.iter().map(|(k, v)| (*k, *v)).collect();
        encoder_layer.sort_unstable();
        Snapshot {
            show: self.snapshot_configuration(),
            live_base: self.live.base.0.to_vec(),
            live_oscs,
            live_master_speed: self.live.master_speed,
            live_speed: self.live.speed,
            live_tempo: self.live.tempo,
            live_active,
            live_refs,
            active_phasers,
            effect_lanes: self
                .effect_lanes
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            hold_overrides,
            encoder_layer,
            active_preset: self.active_preset,
            phaser_deck: self.phaser_deck.clone(),
            preset_deck: self.preset_deck.clone(),
        }
    }

    /// The snapshot as bytes, the form the history keeps.
    pub(crate) fn snapshot_json(&self) -> Vec<u8> {
        serde_json::to_vec(&self.capture_snapshot()).unwrap_or_default()
    }

    /// Put a snapshot back: the show through the same path a configuration
    /// loads by, then the programmer and effects it wiped.
    pub(crate) fn apply_snapshot(&mut self, snap: Snapshot) {
        self.apply_configuration(snap.show);
        if snap.live_base.len() == DMX_SLOTS {
            let mut base = Frame::black();
            base.0.copy_from_slice(&snap.live_base);
            self.live.base = base;
        }
        self.live.oscs = snap.live_oscs.into_iter().collect();
        self.live.master_speed = snap.live_master_speed;
        self.live.speed = snap.live_speed;
        self.live.tempo = snap.live_tempo;
        self.live_active = snap.live_active.into_iter().collect();
        self.live_refs = snap.live_refs.into_iter().collect();
        self.active_phasers = snap.active_phasers.into_iter().collect();
        self.effect_lanes = snap.effect_lanes.into_iter().collect();
        self.hold_overrides = snap.hold_overrides.into_iter().collect();
        self.encoder_layer = snap.encoder_layer.into_iter().collect();
        self.active_preset = snap.active_preset;
        self.phaser_deck = snap.phaser_deck;
        crate::streamdeck::save_phaser_deck(&self.phaser_deck);
        self.preset_deck = snap.preset_deck;
        crate::preset_deck::pad_preset_deck(&mut self.preset_deck);
        crate::preset_deck::save_preset_deck(&self.preset_deck);
    }

    /// Ctrl+Shift+Z / Ctrl+Shift+Y. Runs first in the frame so a focused
    /// text field never sees the chord.
    pub(crate) fn undo_keys(&mut self, ctx: &egui::Context) {
        let chord = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        let undo = ctx.input_mut(|i| i.consume_key(chord, egui::Key::Z));
        let redo = ctx.input_mut(|i| i.consume_key(chord, egui::Key::Y));
        if undo {
            self.undo();
        } else if redo {
            self.redo();
        }
    }

    /// Runs last in the frame: notes input, and — once it has been quiet —
    /// checks whether anything changed and records the step.
    pub(crate) fn undo_tick(&mut self, ctx: &egui::Context) {
        let busy = ctx.input(|i| {
            !i.events.is_empty() || i.pointer.any_down() || !i.keys_down.is_empty()
        });
        if busy {
            self.undo.note_input();
        }
        if self.undo.due() && self.undo.quiet() {
            let json = self.snapshot_json();
            self.undo.observe(json);
        }
        self.autosave_tick();
    }

    pub(crate) fn undo(&mut self) {
        let Some(step) = self.undo.undo() else {
            self.log.push("Nothing to undo".into());
            return;
        };
        let age = ago(step.taken.elapsed());
        self.restore_step(step);
        let left = self.undo.undo_len();
        self.log.push(format!("Undo — back to how it was {age} ago ({left} more)"));
    }

    pub(crate) fn redo(&mut self) {
        let Some(step) = self.undo.redo() else {
            self.log.push("Nothing to redo".into());
            return;
        };
        self.restore_step(step);
        self.log.push(format!("Redo ({} more)", self.undo.redo_len()));
    }

    fn restore_step(&mut self, step: Step) {
        match serde_json::from_slice::<Snapshot>(&step.json) {
            Ok(snap) => self.apply_snapshot(snap),
            Err(e) => self.log.push(format!("Undo failed to read the saved state: {e}")),
        }
        // Whatever the restore normalised, this is the state now.
        let json = self.snapshot_json();
        self.undo.settle(json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(n: u32) -> Vec<u8> {
        format!("state-{n}").into_bytes()
    }

    /// The first look remembers, a repeat is nothing, a change is a step,
    /// undo walks back through the steps and redo walks forward again.
    #[test]
    fn history_records_changes_and_walks_both_ways() {
        let mut h = History::new();
        assert!(!h.observe(bytes(0)));
        assert!(!h.observe(bytes(0)));
        assert!(h.observe(bytes(1)));
        assert!(h.observe(bytes(2)));
        assert_eq!(h.undo_len(), 2);
        assert_eq!(h.undo().unwrap().json, bytes(1));
        assert_eq!(h.redo_len(), 1);
        // The restore settles on what was undone to.
        h.settle(bytes(1));
        assert_eq!(h.undo().unwrap().json, bytes(0));
        h.settle(bytes(0));
        assert!(h.undo().is_none());
        assert_eq!(h.redo().unwrap().json, bytes(1));
        h.settle(bytes(1));
        assert_eq!(h.redo().unwrap().json, bytes(2));
        h.settle(bytes(2));
        assert!(h.redo().is_none());
    }

    /// Only the last ten steps are kept, and a new change after an undo
    /// throws the redo side away.
    #[test]
    fn history_keeps_ten_steps_and_drops_redo_on_a_new_change() {
        let mut h = History::new();
        for n in 0..=25 {
            h.observe(bytes(n));
        }
        assert_eq!(h.undo_len(), DEPTH);
        assert_eq!(h.undo().unwrap().json, bytes(24));
        h.settle(bytes(24));
        assert_eq!(h.redo_len(), 1);
        assert!(h.observe(bytes(99)));
        assert_eq!(h.redo_len(), 0);
    }

    /// A real console: change a couple of things, undo, and they are back —
    /// and the snapshot is cheap enough to take a few times a second.
    #[test]
    fn app_round_trips_through_a_snapshot() {
        let mut app = crate::app::App::new();
        let t = Instant::now();
        let json = app.snapshot_json();
        let took = t.elapsed();
        eprintln!("snapshot: {} KB in {:?}", json.len() / 1024, took);
        assert!(took < Duration::from_millis(200), "snapshot too slow: {took:?}");
        app.undo.observe(json);

        let gm = app.grand_master;
        let fade = app.transition.duration;
        app.grand_master = (gm * 0.5 + 0.1).min(1.0);
        app.transition.duration = fade + 4.5;
        app.live.base.0[3] = 200;
        app.live_active.insert(3);
        assert!(app.undo.observe(app.snapshot_json()), "the change was noticed");

        app.undo();
        assert_eq!(app.grand_master, gm);
        assert_eq!(app.transition.duration, fade);
        assert_eq!(app.live.base.0[3], 0);
        assert!(!app.live_active.contains(&3));
        assert_eq!(app.undo.undo_len(), 0);
        assert_eq!(app.undo.redo_len(), 1);

        app.redo();
        assert_eq!(app.transition.duration, fade + 4.5);
        assert_eq!(app.live.base.0[3], 200);
    }
}
