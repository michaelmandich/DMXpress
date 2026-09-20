//! Programmer layers — stacked sections of a look, each with its own route.
//!
//! Until now the desk had exactly one programmer: a single [`Look`] with one
//! oscillator slot per DMX address. That made the second phaser on a channel
//! evict the first, so a blackout chop could not run over a colour pulse —
//! there was nowhere to park the effect it was overriding.
//!
//! A layer is that programmer, multiplied. Each one owns a `Look`, the set of
//! channels it asserts, the fixtures it targets and the order its effects fan
//! along. They stack bottom→top in the mixer: a higher layer takes the
//! channels it asserts and leaves the rest to show through, so the layer
//! beneath is a *fallback* rather than something that has been destroyed.
//! Oscillator motion is the exception — it sums across layers (see
//! [`crate::engine::Layer`]), which is what makes a wobble on top of a sweep
//! read as one combined move.
//!
//! The *selected* layer is the programmer: its live state lives in
//! `App::live` / `live_active` / `live_refs` so every existing writer, capture
//! path and pool button keeps working unchanged, and `App::select_layer`
//! swaps a layer in and out of those fields. The `look`/`active`/`refs` here
//! hold what a layer was left with while a different one is being programmed.
//!
//! What a layer is *made of* shows as [`LayerBox`]es — the preset, the
//! palettes and the phasers on it, each knowing the channels it claimed.
//! Dropping a box releases exactly those channels, which turns "remove this
//! and let what is underneath come back" into one set-difference rather than
//! a special case per effect type.
//!
//! Boxes are **derived every frame** by [`derive_boxes`], never recorded at
//! apply time. Recording meant a hook at every call site that could put
//! something into the programmer, and there are far more of those than they
//! look: four separate paths arm a phaser, two recall a preset, and undo
//! restores all of it behind everything's back. Miss one and the panel
//! quietly lies about what is on stage. Deriving from the state the desk
//! already keeps — `live_refs` for palettes, `active_phasers` for phasers,
//! the recalled preset — cannot miss a path, and it is live whether or not
//! the window has ever been opened.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::engine::Layer;
use crate::net::Frame;
use crate::oscillator::{Look, Osc};
use crate::palette::PaletteRef;
use crate::preset::SavedOsc;
use crate::scene::MergeMode;

/// What put a box on a layer. The variants carry the pool identity so the
/// panel can show a live name (and grey out a box whose source was deleted)
/// rather than a stale copy of one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoxKind {
    /// A native preset, by `UserPreset::id`.
    Preset(u32),
    /// A ShowBuddy bank preset, by (bank, index). Those are positions in a
    /// rig reloaded from ShowBuddy each run, so unlike the others this is
    /// not a stable id and never outlives the session.
    BankPreset(usize, usize),
    /// A palette, by `Palette::id`.
    Palette(u32),
    /// A running phaser, by name (the key `App::active_phasers` uses).
    Phaser(String),
}

impl BoxKind {
    /// Sort key for the panel: the most far-reaching thing a layer is doing
    /// reads first, so a layer's boxes tell their story top to bottom.
    pub fn rank(&self) -> u8 {
        match self {
            Self::Preset(_) | Self::BankPreset(..) => 0,
            Self::Palette(_) => 1,
            Self::Phaser(_) => 2,
        }
    }

    pub fn tag(&self) -> &'static str {
        match self {
            Self::Preset(_) | Self::BankPreset(..) => "PRESET",
            Self::Palette(_) => "PALETTE",
            Self::Phaser(_) => "PHASER",
        }
    }

    /// The tag as it fits on a tile's second line beside a channel count.
    /// The full word does not, and an elided "PALETTE .." says less than
    /// three letters and the number do.
    pub fn short_tag(&self) -> &'static str {
        match self {
            Self::Preset(_) | Self::BankPreset(..) => "PRE",
            Self::Palette(_) => "PAL",
            Self::Phaser(_) => "FX",
        }
    }
}

/// One contribution shown as a removable box in the Layers panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerBox {
    pub kind: BoxKind,
    /// Name as it read when applied — the fallback if the pool entry is gone.
    pub label: String,
    /// Channels this contribution claimed on its layer. Dropping the box
    /// releases exactly these, and whatever sits beneath shows through again.
    pub addrs: Vec<usize>,
    /// Shift-click multi-select in the panel. Carried across a refresh by
    /// [`derive_boxes`], never persisted: a reloaded show should not come
    /// back with a half-made selection.
    #[serde(skip)]
    pub selected: bool,
}

impl LayerBox {
    pub fn new(kind: BoxKind, label: String, addrs: Vec<usize>) -> Self {
        Self {
            kind,
            label,
            addrs,
            selected: false,
        }
    }
}

/// One programmer layer.
#[derive(Clone, Serialize, Deserialize)]
pub struct ProgLayer {
    /// Stable identity, so a transition or a deck button can name a layer
    /// that has since moved in the stack.
    pub id: u32,
    pub name: String,
    /// The route effects applied here fan along, by `Order::id`;
    /// `None` = patch order. Per-layer, because running the same phaser down
    /// two different routes at once is the whole point of the feature.
    #[serde(default)]
    pub order: Option<u32>,
    /// Patch-fixture indices this layer works on, captured from the selection
    /// when something is applied to it. An order *sequences* these; it does
    /// not choose them.
    #[serde(default)]
    pub targets: Vec<usize>,
    /// Output level 0..1.
    #[serde(default = "one")]
    pub level: f32,
    #[serde(default)]
    pub merge: MergeMode,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// What is on this layer, most far-reaching first. Derived each frame
    /// (see [`derive_boxes`]), so it is never saved — what *produces* it is
    /// saved instead, in `ref_values` and `preset_id`.
    #[serde(skip)]
    pub boxes: Vec<LayerBox>,
    /// The preset recalled onto this layer, by `UserPreset::id`. Parked here
    /// while another layer is selected; the selected layer's is the live
    /// `App::active_user_preset`.
    #[serde(default)]
    pub preset_id: Option<u32>,
    /// A ShowBuddy bank preset recalled here. Not persisted: bank and index
    /// are positions in a rig that is reloaded from ShowBuddy each run.
    #[serde(skip)]
    pub preset_bank: Option<(usize, usize)>,
    /// Palette provenance, saved with the show so a reloaded layer still
    /// knows which palettes made it. `refs` is the runtime copy.
    #[serde(default)]
    pub ref_values: Vec<(usize, PaletteRef)>,

    /// Settled values, saved with the show (mirrors [`crate::scene::Scene`]).
    #[serde(default)]
    pub values: Vec<(usize, u8)>,
    /// Oscillators with the phase they held when saved.
    #[serde(default)]
    pub oscs: Vec<(usize, SavedOsc)>,
    #[serde(default = "one")]
    pub speed: f32,
    #[serde(default = "tempo")]
    pub tempo: f32,
    #[serde(default = "one")]
    pub master_speed: f32,

    /// Parked programmer state. Empty for whichever layer is selected — that
    /// one's state is live in `App::live` and friends.
    #[serde(skip, default = "Look::black")]
    pub(crate) look: Look,
    /// Parked record mask (the selected layer's is `App::live_active`).
    #[serde(skip)]
    pub active: HashSet<usize>,
    /// Parked palette provenance (the selected layer's is `App::live_refs`).
    #[serde(skip)]
    pub refs: HashMap<usize, PaletteRef>,
    /// A release in progress: when it began and how long it takes. The layer
    /// keeps rendering, thinning out, until it lands.
    #[serde(skip)]
    pub leaving: Option<(Instant, f32)>,
    /// A fade-in in progress, so a layer can be brought up under a time.
    #[serde(skip)]
    pub arriving: Option<(Instant, f32)>,
}

fn one() -> f32 {
    1.0
}
fn tempo() -> f32 {
    120.0
}
fn yes() -> bool {
    true
}

impl ProgLayer {
    /// A fresh, empty layer asserting nothing.
    pub fn new(id: u32, name: String) -> Self {
        Self {
            id,
            name,
            order: None,
            targets: Vec::new(),
            level: 1.0,
            merge: MergeMode::Override,
            enabled: true,
            boxes: Vec::new(),
            preset_id: None,
            preset_bank: None,
            ref_values: Vec::new(),
            values: Vec::new(),
            oscs: Vec::new(),
            speed: 0.0,
            tempo: 120.0,
            master_speed: 1.0,
            look: Look::black(),
            active: HashSet::new(),
            refs: HashMap::new(),
            leaving: None,
            arriving: None,
        }
    }

    /// Bring the layer up, over `fade` seconds.
    pub fn start(&mut self, fade: f32) {
        self.enabled = true;
        self.leaving = None;
        self.arriving = (fade > 0.0).then(|| (Instant::now(), fade));
    }

    /// Take the layer out — at once, or thinning out over `fade` seconds.
    /// Returns whether the caller should drop it immediately.
    pub fn stop(&mut self, fade: f32) -> bool {
        if fade <= 0.0 {
            self.leaving = None;
            return true;
        }
        if self.leaving.is_none() {
            self.leaving = Some((Instant::now(), fade));
        }
        false
    }

    /// How much of the layer is still on stage through a release, 0..1.
    pub fn presence(&self) -> f32 {
        match self.leaving {
            None => 1.0,
            Some((started, dur)) => {
                1.0 - (started.elapsed().as_secs_f32() / dur.max(0.001)).clamp(0.0, 1.0)
            }
        }
    }

    /// Current output weight: level, eased in and thinned out again.
    pub fn gain(&self) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        let mut g = self.level.clamp(0.0, 1.0) * self.presence();
        if let Some((started, dur)) = self.arriving {
            g *= (started.elapsed().as_secs_f32() / dur.max(0.001)).clamp(0.0, 1.0);
        }
        g
    }

    /// Whether a fade is still moving, so the UI keeps repainting.
    pub fn is_fading(&self) -> bool {
        if self.leaving.is_some() {
            return true;
        }
        match self.arriving {
            Some((started, dur)) => started.elapsed().as_secs_f32() < dur,
            None => false,
        }
    }

    /// A finished release, ready to be dropped from the stack.
    pub fn released(&self) -> bool {
        self.leaving.is_some() && self.presence() <= 0.0
    }

    /// Let a landed fade-in go, so `is_fading` stops asking for repaints.
    pub fn settle(&mut self) {
        if let Some((started, dur)) = self.arriving {
            if started.elapsed().as_secs_f32() >= dur {
                self.arriving = None;
            }
        }
    }

    /// Advance this layer's parked clock and hand the mixer its contribution.
    /// Only for layers that are *not* selected — the selected layer's frame
    /// comes from the live programmer (and may be mid-transition).
    pub(crate) fn layer(&mut self) -> Option<Layer> {
        let gain = self.gain();
        let (frame, deltas) = self.look.render_parts();
        if gain <= 0.0 || self.active.is_empty() {
            // Still rendered above, so the clock keeps running and the layer
            // does not jump when its level comes back up.
            return None;
        }
        Some(Self::compose(
            frame,
            deltas,
            &self.active,
            gain,
            self.merge,
        ))
    }

    /// Build a mixer contribution from a rendered look and a record mask.
    /// Shared with the selected layer, whose frame the caller supplies.
    pub(crate) fn compose(
        frame: Frame,
        deltas: Vec<(usize, i16)>,
        active: &HashSet<usize>,
        gain: f32,
        merge: MergeMode,
    ) -> Layer {
        let weights: Vec<(usize, f32)> = active.iter().map(|&a| (a, gain)).collect();
        // Motion is scaled by the same gain, so fading a layer out takes its
        // swing down with it instead of leaving a wave over a dark base.
        let deltas: Vec<(usize, i16)> = deltas
            .into_iter()
            .filter(|(a, _)| active.contains(a))
            .map(|(a, d)| (a, (d as f32 * gain).round() as i16))
            .collect();
        Layer::overlay(frame, weights)
            .with_blend(merge.blend())
            .with_deltas(deltas)
    }

    /// The layer as it goes into a show file: its structure, plus the look
    /// it is holding distilled into the serializable fields.
    ///
    /// Takes the runtime by reference and builds a fresh record rather than
    /// cloning the layer, because everything `#[serde(skip)]` — the `Look`
    /// with its oscillators, the record mask, the palette map, the derived
    /// boxes — would be deep-copied only to be dropped. This is on the undo
    /// path, which captures four times a second.
    pub(crate) fn saved(
        &self,
        look: &Look,
        active: &HashSet<usize>,
        refs: &HashMap<usize, PaletteRef>,
        order: Option<u32>,
    ) -> Self {
        let mut values: Vec<(usize, u8)> =
            active.iter().map(|&a| (a, look.base[a])).collect();
        values.sort_unstable();
        let mut oscs: Vec<(usize, SavedOsc)> = look
            .oscs
            .iter()
            .map(|(&a, o)| {
                (
                    a,
                    SavedOsc {
                        invert: o.invert,
                        amount: o.amount,
                        phase: o.phase,
                        subdiv: o.subdiv,
                        shape: o.shape,
                        custom_wave: o.custom_wave.clone(),
                        master_beat: o.master_beat,
                        local_beats: o.local_beats,
                        local_tempo: o.local_tempo,
                    },
                )
            })
            .collect();
        oscs.sort_unstable_by_key(|(a, _)| *a);
        let mut ref_values: Vec<(usize, PaletteRef)> =
            refs.iter().map(|(&a, &r)| (a, r)).collect();
        ref_values.sort_unstable_by_key(|(a, _)| *a);
        Self {
            id: self.id,
            name: self.name.clone(),
            order,
            targets: self.targets.clone(),
            level: self.level,
            merge: self.merge,
            enabled: self.enabled,
            boxes: Vec::new(),
            preset_id: self.preset_id,
            preset_bank: self.preset_bank,
            ref_values,
            values,
            oscs,
            speed: look.speed,
            tempo: look.tempo,
            master_speed: look.master_speed,
            look: Look::black(),
            active: HashSet::new(),
            refs: HashMap::new(),
            leaving: None,
            arriving: None,
        }
    }

    /// Rebuild the parked runtime from the snapshot after a show loads.
    pub fn restore(&mut self) {
        let mut f = Frame::black();
        for &(a, v) in &self.values {
            if a < f.len() {
                f[a] = v;
            }
        }
        let mut look = Look::from_frame(f);
        look.oscs = self
            .oscs
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
            .collect();
        look.speed = self.speed;
        look.tempo = self.tempo;
        look.master_speed = self.master_speed;
        self.active = self.values.iter().map(|&(a, _)| a).collect();
        self.look = look;
        self.refs = self.ref_values.iter().copied().collect();
    }

    /// Rebuild this layer's boxes from what is actually on it.
    ///
    /// `preset` is the recalled preset as (which kind, its name, the
    /// channels it carries), `palettes` resolves a palette id to its name,
    /// and `phasers` is the desk's running-phaser map. `active` is the layer's own record mask:
    /// every box is intersected with it, so a phaser is attributed to the
    /// layer whose channels it is actually holding rather than to all of
    /// them at once.
    pub fn derive_boxes(
        &mut self,
        preset: Option<(BoxKind, String, &[usize])>,
        palettes: &HashMap<u32, String>,
        phasers: &[(String, Vec<usize>)],
    ) {
        let active = &self.active;
        let keep = |addrs: &[usize]| -> Vec<usize> {
            let mut v: Vec<usize> = addrs.iter().copied().filter(|a| active.contains(a)).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let mut out: Vec<LayerBox> = Vec::new();
        if let Some((kind, name, addrs)) = preset {
            // A preset that has since been overwritten channel by channel is
            // no longer on the layer in any meaningful sense.
            let mine = keep(addrs);
            if !mine.is_empty() {
                out.push(LayerBox::new(kind, name, mine));
            }
        }
        // Palettes come from channel provenance, so a palette half painted
        // over by something else shows only the channels it still owns.
        let mut by_palette: HashMap<u32, Vec<usize>> = HashMap::new();
        for (&a, r) in &self.refs {
            if active.contains(&a) {
                by_palette.entry(r.id).or_default().push(a);
            }
        }
        let mut pal: Vec<(u32, Vec<usize>)> = by_palette.into_iter().collect();
        pal.sort_unstable_by_key(|(id, _)| *id);
        for (id, mut addrs) in pal {
            addrs.sort_unstable();
            let name = palettes
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("palette {id}"));
            out.push(LayerBox::new(BoxKind::Palette(id), name, addrs));
        }
        for (name, addrs) in phasers {
            let mine = keep(addrs);
            if !mine.is_empty() {
                out.push(LayerBox::new(BoxKind::Phaser(name.clone()), name.clone(), mine));
            }
        }
        out.sort_by_key(|b| b.kind.rank());
        // A mark the operator made survives the rebuild it happens to land in.
        for b in &mut out {
            if let Some(old) = self.boxes.iter().find(|o| o.kind == b.kind) {
                b.selected = old.selected;
            }
        }
        self.boxes = out;
    }

    /// Channels claimed by a box and by no other box on this layer — what
    /// dropping it should actually release.
    pub fn exclusive_addrs(&self, idx: usize) -> Vec<usize> {
        let Some(target) = self.boxes.get(idx) else {
            return Vec::new();
        };
        let others: HashSet<usize> = self
            .boxes
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .flat_map(|(_, b)| b.addrs.iter().copied())
            .collect();
        target
            .addrs
            .iter()
            .copied()
            .filter(|a| !others.contains(a))
            .collect()
    }
}

/// Assemble the programmer layers into this frame's mixer contributions,
/// bottom→top, and say whether any of them still needs repainting.
///
/// `selected` is the layer being programmed: its picture comes from the live
/// programmer (`prog_frame`/`prog_deltas`, which may be mid-transition) and
/// its record mask from `live_active`, because that state lives on `App`
/// rather than on the layer while it is selected. Every other layer renders
/// its own parked look — always, even at zero gain, so its clock keeps
/// running and it does not jump when it is brought back up.
pub(crate) fn stack_layers(
    layers: &mut [ProgLayer],
    selected: usize,
    prog_frame: Frame,
    prog_deltas: Vec<(usize, i16)>,
    live_active: &HashSet<usize>,
) -> (Vec<Layer>, bool) {
    let mut prog = Some((prog_frame, prog_deltas));
    let mut out: Vec<Layer> = Vec::with_capacity(layers.len());
    let mut busy = false;
    for (i, l) in layers.iter_mut().enumerate() {
        l.settle();
        if l.is_fading() {
            busy = true;
        }
        if i == selected {
            let gain = l.gain();
            if let Some((frame, deltas)) = prog.take() {
                if gain > 0.0 && !live_active.is_empty() {
                    out.push(ProgLayer::compose(frame, deltas, live_active, gain, l.merge));
                }
            }
        } else {
            if l.look.is_animated() {
                busy = true;
            }
            if let Some(layer) = l.layer() {
                out.push(layer);
            }
        }
    }
    (out, busy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Mixer;

    /// The worked example this whole feature exists for: a blue pulse with a
    /// dimmer phaser on the bottom layer, and a chop on top that blacks out
    /// everything *except* the movement channels. The pan must keep moving
    /// while the dimmer goes dark — a single shared programmer could not do
    /// this, because the second phaser evicted the first.
    #[test]
    fn a_chop_layer_blacks_the_light_but_leaves_the_movement_swinging() {
        const DIM: usize = 10;
        const PAN: usize = 11;

        let mut base_frame = Frame::black();
        base_frame[DIM] = 180;
        base_frame[PAN] = 128;
        let base_active: HashSet<usize> = [DIM, PAN].into_iter().collect();
        // A dimmer flash and a pan sweep, both running on the base layer.
        let base_motion = vec![(DIM, -60i16), (PAN, 40i16)];

        // The chop asserts zero on the dimmer only — the movement channel is
        // excluded, so the layer never claims it.
        let mut chop_frame = Frame::black();
        chop_frame[DIM] = 0;
        let chop_active: HashSet<usize> = [DIM].into_iter().collect();

        let mut m = Mixer::new();
        m.push(ProgLayer::compose(
            base_frame,
            base_motion,
            &base_active,
            1.0,
            MergeMode::Override,
        ));
        m.push(ProgLayer::compose(
            chop_frame,
            Vec::new(),
            &chop_active,
            1.0,
            MergeMode::Override,
        ));
        let out = m.render();
        assert_eq!(out[DIM], 0, "the chop owns the dimmer outright");
        assert_eq!(out[PAN], 168, "the pan sweep is untouched by the chop");
    }

    /// Half way through its release the chop is only half asserting, so the
    /// light comes back up smoothly rather than snapping on.
    #[test]
    fn a_releasing_layer_hands_the_channel_back_gradually() {
        const DIM: usize = 3;
        let mut base = Frame::black();
        base[DIM] = 200;
        let active: HashSet<usize> = [DIM].into_iter().collect();
        let mut m = Mixer::new();
        m.push(ProgLayer::compose(base, Vec::new(), &active, 1.0, MergeMode::Override));
        m.push(ProgLayer::compose(
            Frame::black(),
            Vec::new(),
            &active,
            0.5,
            MergeMode::Override,
        ));
        assert_eq!(m.render()[DIM], 100);
    }

    /// A layer only asserts what it claims; everything else falls through,
    /// which is what makes the layer below a fallback rather than gone.
    #[test]
    fn an_unclaimed_channel_falls_through_to_the_layer_below() {
        const A: usize = 4;
        const B: usize = 5;
        let mut base = Frame::black();
        base[A] = 90;
        base[B] = 90;
        let mut top = Frame::black();
        top[A] = 255;
        let mut m = Mixer::new();
        m.push(ProgLayer::compose(
            base,
            Vec::new(),
            &[A, B].into_iter().collect(),
            1.0,
            MergeMode::Override,
        ));
        m.push(ProgLayer::compose(
            top,
            Vec::new(),
            &[A].into_iter().collect(),
            1.0,
            MergeMode::Override,
        ));
        let out = m.render();
        assert_eq!((out[A], out[B]), (255, 90));
    }

    /// Dropping a box gives back only the channels no other box on the layer
    /// is still holding, so removing a palette cannot blank a preset.
    #[test]
    fn dropping_a_box_spares_channels_another_box_shares() {
        let mut l = ProgLayer::new(1, "L".into());
        l.active = [1, 2, 3, 4].into_iter().collect();
        l.preset_id = Some(1);
        l.refs.insert(3, PaletteRef { feature: crate::palette::Feature::Color, id: 2 });
        l.refs.insert(4, PaletteRef { feature: crate::palette::Feature::Color, id: 2 });
        let names: HashMap<u32, String> = [(2, "Red".to_string())].into_iter().collect();
        l.derive_boxes(Some((BoxKind::Preset(1), "Blue 50".into(), &[1, 2, 3])), &names, &[]);
        // Box 0 is the preset (rank 0), box 1 the palette.
        assert_eq!(l.exclusive_addrs(0), vec![1, 2], "3 is shared with the palette");
        assert_eq!(l.exclusive_addrs(1), vec![4], "3 is shared with the preset");
    }

    /// Everything on the layer shows, whichever path put it there — the
    /// panel reads the desk rather than a log of what it saw happen.
    #[test]
    fn boxes_are_derived_from_live_state_and_sorted_by_impact() {
        let mut l = ProgLayer::new(1, "L".into());
        l.active = [1, 5, 7, 8].into_iter().collect();
        l.preset_id = Some(9);
        l.refs.insert(5, PaletteRef { feature: crate::palette::Feature::Color, id: 3 });
        let names: HashMap<u32, String> = [(3, "Cyan".to_string())].into_iter().collect();
        let phasers = vec![("Chop".to_string(), vec![7, 8])];
        l.derive_boxes(Some((BoxKind::Preset(9), "Blue".into(), &[1])), &names, &phasers);
        let kinds: Vec<&BoxKind> = l.boxes.iter().map(|b| &b.kind).collect();
        assert_eq!(
            kinds,
            vec![&BoxKind::Preset(9), &BoxKind::Palette(3), &BoxKind::Phaser("Chop".into())]
        );
        assert_eq!(l.boxes[2].addrs, vec![7, 8]);
    }

    /// The bug this derivation replaced: a preset recalled onto the base
    /// layer recorded no box, because the record hook was gated on *not*
    /// being the rig-wide layer — which the base layer always is. Nothing
    /// is gated now, so a rig-wide recall shows like any other.
    #[test]
    fn a_rig_wide_preset_on_the_base_layer_still_shows() {
        let mut base = ProgLayer::new(1, "Base".into());
        // A rig takeover asserts every channel.
        base.active = (0..crate::net::DMX_SLOTS).collect();
        base.preset_id = Some(4);
        base.derive_boxes(
            Some((BoxKind::Preset(4), "Blue 50".into(), &[10, 11, 12])),
            &HashMap::new(),
            &[],
        );
        assert_eq!(base.boxes.len(), 1);
        assert_eq!(base.boxes[0].label, "Blue 50");
        // The box carries the preset's own channels, not all 1024 of them.
        assert_eq!(base.boxes[0].addrs, vec![10, 11, 12]);
    }

    /// A ShowBuddy bank preset is a preset too, and reads as one.
    #[test]
    fn a_bank_preset_shows_like_a_native_one() {
        let mut l = ProgLayer::new(1, "L".into());
        l.active = [3, 4].into_iter().collect();
        l.preset_bank = Some((0, 2));
        l.derive_boxes(
            Some((BoxKind::BankPreset(0, 2), "Warm wash".into(), &[3, 4])),
            &HashMap::new(),
            &[],
        );
        assert_eq!(l.boxes.len(), 1);
        assert_eq!(l.boxes[0].kind.short_tag(), "PRE");
    }

    /// A phaser is attributed to the layer actually holding its channels,
    /// not to every layer at once — `active_phasers` is desk-wide.
    #[test]
    fn a_phaser_only_shows_on_the_layer_holding_its_channels() {
        let phasers = vec![("Sweep".to_string(), vec![10, 11, 12])];
        let names = HashMap::new();
        let mut mine = ProgLayer::new(1, "mine".into());
        mine.active = [10, 11].into_iter().collect();
        mine.derive_boxes(None, &names, &phasers);
        assert_eq!(mine.boxes.len(), 1);
        assert_eq!(mine.boxes[0].addrs, vec![10, 11], "only the channels it holds here");

        let mut other = ProgLayer::new(2, "other".into());
        other.active = [50].into_iter().collect();
        other.derive_boxes(None, &names, &phasers);
        assert!(other.boxes.is_empty(), "a layer holding none of it shows nothing");
    }

    /// A hold phaser must not read as part of a layer. `hold_overrides` is
    /// stamped after the mixer and after the grand master, so nothing in the
    /// stack can override it; a box inside a layer would say the opposite.
    /// The split happens before derivation, so the layer simply never sees
    /// the forced channels.
    #[test]
    fn a_forced_hold_is_not_a_layer_box() {
        // What `refresh_layer_boxes` hands down once the forced channels
        // have been taken out: the hold owned only 290 and 291, so nothing
        // of it is left for the layer.
        let mut l = ProgLayer::new(1, "Base".into());
        l.active = (0..512).collect();
        l.derive_boxes(None, &HashMap::new(), &[]);
        assert!(l.boxes.is_empty(), "the fogger hold belongs above the stack");

        // A wave phaser on the same layer still reads normally.
        l.derive_boxes(None, &HashMap::new(), &[("Sweep".to_string(), vec![10, 11])]);
        assert_eq!(l.boxes.len(), 1);
    }

    /// A mark the operator made survives the refresh that lands on top of it.
    #[test]
    fn a_marked_box_stays_marked_across_a_refresh() {
        let mut l = ProgLayer::new(1, "L".into());
        l.active = [7].into_iter().collect();
        let phasers = vec![("Chop".to_string(), vec![7])];
        l.derive_boxes(None, &HashMap::new(), &phasers);
        l.boxes[0].selected = true;
        l.derive_boxes(None, &HashMap::new(), &phasers);
        assert!(l.boxes[0].selected);
    }

    /// Provenance is what gets saved, so a reloaded layer still knows which
    /// palette made it and can rebuild its boxes.
    #[test]
    fn palette_provenance_survives_a_round_trip() {
        let mut l = ProgLayer::new(2, "Movement".into());
        l.look.base[9] = 77;
        l.active = [9].into_iter().collect();
        l.refs.insert(9, PaletteRef { feature: crate::palette::Feature::Color, id: 4 });
        let out = l.saved(&l.look, &l.active, &l.refs, None);
        let json = serde_json::to_string(&out).expect("serialize");
        let mut back: ProgLayer = serde_json::from_str(&json).expect("deserialize");
        back.restore();
        assert_eq!(back.refs.get(&9).map(|r| r.id), Some(4));
        let names: HashMap<u32, String> = [(4, "Amber".to_string())].into_iter().collect();
        back.derive_boxes(None, &names, &[]);
        assert_eq!(back.boxes.len(), 1);
        assert_eq!(back.boxes[0].label, "Amber");
    }

    /// A layer's snapshot survives a save/load round trip.
    #[test]
    fn a_layer_round_trips_through_its_snapshot() {
        let mut l = ProgLayer::new(2, "Movement".into());
        l.look.base[9] = 77;
        l.active = [9].into_iter().collect();
        let out = l.saved(&l.look, &l.active, &l.refs, None);
        let json = serde_json::to_string(&out).expect("serialize");
        let mut back: ProgLayer = serde_json::from_str(&json).expect("deserialize");
        back.restore();
        assert_eq!(back.name, "Movement");
        assert_eq!(back.look.base[9], 77);
        assert!(back.active.contains(&9));
    }
}
