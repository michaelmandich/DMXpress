//! The programmer encoder — the Stream Deck's sixth knob — and the staged
//! Clear that undoes it.
//!
//! Twist the knob to nudge one channel type (dimmer, gobo, pan…) across
//! every selected light; press it to step to the next channel type the
//! selection has. Values land in the *encoder layer*: a per-channel
//! override painted over the mixed output (programmer, stacks, scenes,
//! oscillators) but under the grand master, so a dimmer dialled in by hand
//! still fades with the rig. Storing a preset bakes the layer in.
//!
//! Clear works in three stages, each press taking the first that applies:
//! encoders, then oscillations/phasers, then a full blackout (every stack
//! released, the programmer emptied).

use std::collections::HashMap;

use crate::app::App;
use crate::net;

/// DMX steps per detent on a continuous channel: a full sweep in ~64 clicks.
const STEP: i32 = 4;

/// One channel type the encoder can drive: every matching channel across the
/// selected lights, plus the channel used for its name and band labels.
pub(crate) struct EncoderChannel {
    /// Stable identity, so the pick survives selection changes: `r:<role
    /// tag>` for classified channels, `n:<name>` for the rest.
    pub key: String,
    pub label: String,
    /// (fixture, channel) representative.
    pub rep: (usize, usize),
    /// 0-based DMX addresses, one per selected light that has this channel.
    /// The representative's own address comes first.
    pub addrs: Vec<usize>,
}

/// What the next Clear press removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClearStage {
    Encoders,
    Effects,
    Blackout,
}

impl App {
    /// The lights the encoder drives: the stage selection, else the single
    /// list-selected light — the same rule Channel control uses.
    pub(crate) fn encoder_targets(&self) -> Vec<usize> {
        let sf = self.stage.selected_fixtures();
        if sf.len() > 1 {
            sf
        } else if let Some(i) = self.sel_fixture {
            vec![i]
        } else {
            sf
        }
    }

    /// Channel types present across the targets, in the order the first
    /// light lists them (grouped by role, unclassified ones by name).
    pub(crate) fn encoder_channels(&self) -> Vec<EncoderChannel> {
        let mut out: Vec<EncoderChannel> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for fi in self.encoder_targets() {
            let Some(f) = self.patch.fixtures.get(fi) else {
                continue;
            };
            for (ci, ch) in f.channels.iter().enumerate() {
                let addr = f.from as usize + ci;
                if addr == 0 || addr > net::DMX_SLOTS {
                    continue;
                }
                let role = ch.role();
                let key = if role.tag().is_empty() {
                    format!("n:{}", ch.name.to_lowercase())
                } else {
                    format!("r:{}", role.tag())
                };
                match index.get(&key) {
                    Some(&p) => out[p].addrs.push(addr - 1),
                    None => {
                        index.insert(key.clone(), out.len());
                        out.push(EncoderChannel {
                            key,
                            label: ch.name.clone(),
                            rep: (fi, ci),
                            addrs: vec![addr - 1],
                        });
                    }
                }
            }
        }
        out
    }

    /// The channel type the knob is on: the remembered pick if the selection
    /// still has it, else the first one.
    pub(crate) fn encoder_current(&self) -> Option<EncoderChannel> {
        let mut chans = self.encoder_channels();
        if chans.is_empty() {
            return None;
        }
        let idx = self
            .encoder_key
            .as_ref()
            .and_then(|k| chans.iter().position(|c| &c.key == k))
            .unwrap_or(0);
        Some(chans.swap_remove(idx))
    }

    /// Knob press: step to the next channel type in the selection.
    pub(crate) fn encoder_next_channel(&mut self) {
        let chans = self.encoder_channels();
        if chans.is_empty() {
            self.log.push("Encoder: select some lights first".into());
            return;
        }
        let idx = self
            .encoder_key
            .as_ref()
            .and_then(|k| chans.iter().position(|c| &c.key == k));
        let next = match idx {
            Some(i) => (i + 1) % chans.len(),
            None => 0,
        };
        self.encoder_key = Some(chans[next].key.clone());
        self.log.push(format!("Encoder → {}", chans[next].label));
    }

    /// What `addr` is doing right now as far as the knob is concerned: the
    /// layer's own value once it holds the channel, else the mixed output
    /// (before masters), so the first click nudges from what you see.
    pub(crate) fn encoder_value(&self, addr: usize) -> u8 {
        self.encoder_layer
            .get(&addr)
            .copied()
            .unwrap_or_else(|| self.mixed.get(addr).copied().unwrap_or(0))
    }

    /// Knob twist: nudge the current channel type on every target by `delta`
    /// detents. Continuous channels move `STEP` per detent; stepped ones
    /// (gobo and colour wheels, macros) jump a whole band per detent,
    /// landing mid-band so the wheel sits squarely on the slot.
    pub(crate) fn encoder_nudge(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        let Some(ch) = self.encoder_current() else {
            self.log.push("Encoder: select some lights first".into());
            return;
        };
        let bands: Vec<(u8, u8)> = if self.channel_is_stepped(ch.addrs[0]) {
            self.patch
                .fixtures
                .get(ch.rep.0)
                .and_then(|f| f.channels.get(ch.rep.1))
                .map(|c| {
                    let mut b: Vec<(u8, u8)> = c.bands.iter().map(|b| (b.min, b.max)).collect();
                    b.sort_unstable();
                    b
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        for &a in &ch.addrs {
            let cur = self.encoder_value(a);
            let new = if bands.len() > 1 {
                let i = bands
                    .iter()
                    .position(|&(lo, hi)| cur >= lo && cur <= hi)
                    .unwrap_or(0) as i32;
                let j = (i + delta).clamp(0, bands.len() as i32 - 1) as usize;
                let (lo, hi) = bands[j];
                ((lo as u16 + hi as u16) / 2) as u8
            } else {
                (cur as i32 + delta * STEP).clamp(0, 255) as u8
            };
            self.encoder_layer.insert(a, new);
        }
    }

    /// Readout for the strip and the channel-control header: the channel
    /// name, its value on the representative light (the band's name where
    /// the channel is stepped, e.g. a gobo), and how many lights it drives.
    pub(crate) fn encoder_readout(&self) -> Option<(String, String, usize)> {
        let ch = self.encoder_current()?;
        let v = self.encoder_value(ch.addrs[0]);
        let band = if self.channel_is_stepped(ch.addrs[0]) {
            self.patch
                .fixtures
                .get(ch.rep.0)
                .and_then(|f| f.channels.get(ch.rep.1))
                .and_then(|c| c.band_label(v))
                .map(str::to_string)
        } else {
            None
        };
        Some((ch.label, band.unwrap_or_else(|| v.to_string()), ch.addrs.len()))
    }

    /// Stage 1: drop the encoder layer. Returns whether there was one.
    pub(crate) fn clear_encoders(&mut self) -> bool {
        if self.encoder_layer.is_empty() {
            return false;
        }
        let n = self.encoder_layer.len();
        self.encoder_layer.clear();
        self.log.push(format!("Cleared encoders ({n} ch)"));
        true
    }

    /// Whether any oscillator or phaser is contributing to the output.
    pub(crate) fn effects_running(&self) -> bool {
        !self.live.oscs.is_empty()
            || !self.active_phasers.is_empty()
            || !self.hold_overrides.is_empty()
            || !self.add_overrides.is_empty()
    }

    /// Stage 2: stop every oscillator and phaser, reverting to base values
    /// (the Phasers window's Clear FX).
    pub(crate) fn clear_effects(&mut self) {
        self.live.oscs.clear();
        self.active_phasers.clear();
        self.hold_overrides.clear();
        self.add_overrides.clear();
        self.osc_ramps.clear();
        self.add_ramps.clear();
        self.log.push("Cleared effects (reverted to base)".into());
    }

    /// Stage 3: full blackout — encoders and effects dropped, every stack
    /// and scene released, the programmer emptied (the command line's
    /// `black`). The show goes dark and stays dark until you build again.
    pub(crate) fn blackout_all(&mut self) {
        self.encoder_layer.clear();
        self.live.oscs.clear();
        self.active_phasers.clear();
        self.hold_overrides.clear();
        self.add_overrides.clear();
        self.osc_ramps.clear();
        self.add_ramps.clear();
        for st in &mut self.stacks {
            st.release();
        }
        for sc in &mut self.scenes {
            sc.stop();
        }
        self.cycle_on = false;
        self.cycle_fade = None;
        self.cycle_seq = None;
        self.effect_lanes.clear();
        self.clear_programmer();
        self.log.push("Blackout — everything released".into());
    }

    /// Which stage the next Clear press performs.
    pub(crate) fn clear_stage_next(&self) -> ClearStage {
        if !self.encoder_layer.is_empty() {
            ClearStage::Encoders
        } else if self.effects_running() {
            ClearStage::Effects
        } else {
            ClearStage::Blackout
        }
    }

    /// One press of Clear: encoders first, then effects, then blackout.
    pub(crate) fn clear_stage(&mut self) {
        match self.clear_stage_next() {
            ClearStage::Encoders => {
                self.clear_encoders();
            }
            ClearStage::Effects => self.clear_effects(),
            ClearStage::Blackout => self.blackout_all(),
        }
    }
}
