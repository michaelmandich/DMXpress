//! The output compositor — DMXpress's renamed grandMA3 *mixer*.
//!
//! Every playback object (the live look, the chase overlay, and later Decks
//! and Phasers) hands the mixer a [`Layer`]. Each frame the mixer flattens its
//! layer stack bottom→top into the single [`Frame`] sent to Art-Net, so new
//! features become new layers instead of new special cases in the render loop.

use crate::net::Frame;

/// How a layer folds into everything beneath it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Blend {
    /// Crossfade toward this layer's value by its weight — the layer takes
    /// over the channel outright at full weight (LTP).
    #[default]
    Mix,
    /// Keep whichever is brighter (HTP), so two layers coexist instead of one
    /// erasing the other.
    Max,
    /// Sum onto what is beneath, clamped at full.
    Add,
}

/// One contribution to the final output frame: `frame`'s values blended onto
/// the channels in `weights` (0..1 each — a weight of 1 fully asserts the
/// channel, lower weights let lower layers show through, e.g. a stack fader).
///
/// Oscillator *motion* travels separately from the settled values, in
/// [`Layer::deltas`]: signed swings around the base that sum across layers and
/// clamp once at the very end. Folding motion into `frame` instead would clamp
/// it per layer, and a bipolar wave on a layer sitting at zero would lose its
/// whole negative half before the mixer ever saw it — a pan wobble stacked on
/// a pan sweep could then only ever push one way.
pub(crate) struct Layer {
    frame: Frame,
    weights: Vec<(usize, f32)>,
    blend: Blend,
    deltas: Vec<(usize, i16)>,
}

impl Layer {
    /// A layer that blends `frame` over everything beneath it on the given
    /// channels only.
    /// The channels this layer asserts, for tests that need to see which
    /// fixtures a pattern is actually covering.
    #[cfg(test)]
    pub fn weights(&self) -> &[(usize, f32)] {
        &self.weights
    }

    pub fn overlay(frame: Frame, weights: Vec<(usize, f32)>) -> Self {
        Self {
            frame,
            weights,
            blend: Blend::Mix,
            deltas: Vec::new(),
        }
    }

    /// Fold this layer in with something other than a straight crossfade.
    pub fn with_blend(mut self, blend: Blend) -> Self {
        self.blend = blend;
        self
    }

    /// Carry signed oscillator motion alongside the settled values.
    pub fn with_deltas(mut self, deltas: Vec<(usize, i16)>) -> Self {
        self.deltas = deltas;
        self
    }

    /// Merge this layer onto `out`, which already holds everything beneath it,
    /// and onto the running motion accumulator `acc`.
    ///
    /// A crossfade that asserts a channel takes its *motion* over too, in the
    /// same proportion: at full weight the layer owns the channel outright and
    /// whatever was swinging underneath is silenced, which is what makes a
    /// blackout layer above a running phaser actually black the light out
    /// rather than let it keep flashing around zero.
    fn merge_into(&self, out: &mut Frame, acc: &mut [i32]) {
        for &(i, w) in &self.weights {
            if i >= out.len() {
                continue;
            }
            match self.blend {
                Blend::Mix => {
                    out.blend_channel(i, self.frame[i], w);
                    let keep = 1.0 - w.clamp(0.0, 1.0);
                    acc[i] = (acc[i] as f32 * keep).round() as i32;
                }
                Blend::Max => {
                    let v = (self.frame[i] as f32 * w.clamp(0.0, 1.0)).round() as u8;
                    out[i] = out[i].max(v);
                }
                Blend::Add => {
                    let v = (self.frame[i] as f32 * w.clamp(0.0, 1.0)).round() as u8;
                    out[i] = out[i].saturating_add(v);
                }
            }
        }
        // The layer's own motion lands after the suppression pass, so a layer
        // never cancels the wave it is itself running.
        for &(i, d) in &self.deltas {
            if i < acc.len() {
                acc[i] += d as i32;
            }
        }
    }
}

/// The compositor: collects the current frame's layers and flattens them.
pub(crate) struct Mixer {
    stack: Vec<Layer>,
}

impl Mixer {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    /// Clear the stack to start assembling a new frame.
    pub fn begin(&mut self) {
        self.stack.clear();
    }

    /// Add a layer on top of the current stack.
    pub fn push(&mut self, layer: Layer) {
        self.stack.push(layer);
    }

    /// Flatten the stack (bottom first) into the output frame.
    ///
    /// Two passes in one sweep: settled values fold layer by layer, while
    /// oscillator motion accumulates signed and untouched until the end, so
    /// stacked phasers sum instead of the topmost one winning.
    pub fn render(&self) -> Frame {
        let mut out = Frame::black();
        let mut acc = [0i32; crate::net::DMX_SLOTS];
        for layer in &self.stack {
            layer.merge_into(&mut out, &mut acc);
        }
        for (i, &d) in acc.iter().enumerate() {
            if d != 0 {
                out[i] = (out[i] as i32 + d).clamp(0, 255) as u8;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(addr: usize, v: u8) -> Frame {
        let mut f = Frame::black();
        f[addr] = v;
        f
    }

    /// The single-layer path has to keep rendering exactly as it did before
    /// motion travelled separately, or every existing show changes.
    #[test]
    fn one_layer_base_plus_motion_matches_the_old_sum() {
        let mut m = Mixer::new();
        m.push(Layer::overlay(lit(0, 100), vec![(0, 1.0)]).with_deltas(vec![(0, -40)]));
        assert_eq!(m.render()[0], 60);
    }

    /// Two phasers on one channel sum their swing instead of the upper one
    /// winning — the thing a single `Osc` slot per address made impossible.
    #[test]
    fn stacked_motion_sums_rather_than_replacing() {
        let mut m = Mixer::new();
        m.push(Layer::overlay(lit(0, 128), vec![(0, 1.0)]).with_deltas(vec![(0, 30)]));
        // A layer asserting nothing still contributes its motion.
        m.push(Layer::overlay(Frame::black(), Vec::new()).with_deltas(vec![(0, 40)]));
        assert_eq!(m.render()[0], 198);
    }

    /// A bipolar swing around a dark base keeps its negative half: the whole
    /// reason motion is not folded into the frame per layer.
    #[test]
    fn negative_motion_survives_a_dark_layer() {
        let mut m = Mixer::new();
        m.push(Layer::overlay(lit(0, 200), vec![(0, 1.0)]));
        m.push(Layer::overlay(Frame::black(), Vec::new()).with_deltas(vec![(0, -80)]));
        assert_eq!(m.render()[0], 120);
        // Clamping happens once, at the end, not per layer.
        let mut m = Mixer::new();
        m.push(Layer::overlay(lit(0, 10), vec![(0, 1.0)]).with_deltas(vec![(0, -50)]));
        m.push(Layer::overlay(Frame::black(), Vec::new()).with_deltas(vec![(0, 90)]));
        assert_eq!(m.render()[0], 50);
    }

    /// A layer that takes a channel outright takes its motion too, so a
    /// blackout over a running phaser actually blacks the light out.
    #[test]
    fn a_full_assert_silences_the_motion_beneath_it() {
        let mut m = Mixer::new();
        m.push(Layer::overlay(lit(0, 200), vec![(0, 1.0)]).with_deltas(vec![(0, 55)]));
        m.push(Layer::overlay(lit(0, 0), vec![(0, 1.0)]));
        assert_eq!(m.render()[0], 0);
    }

    /// Half-asserting leaves half the motion showing, so a layer fading in
    /// takes the one beneath it over smoothly rather than snapping.
    #[test]
    fn a_partial_assert_thins_the_motion_beneath_it() {
        let mut m = Mixer::new();
        m.push(Layer::overlay(lit(0, 100), vec![(0, 1.0)]).with_deltas(vec![(0, 80)]));
        m.push(Layer::overlay(lit(0, 0), vec![(0, 0.5)]));
        // Base crossfades 100 → 50, motion is halved to 40.
        assert_eq!(m.render()[0], 90);
    }

    /// Channels a layer does not assert fall through to the layer below.
    #[test]
    fn unasserted_channels_fall_through() {
        let mut m = Mixer::new();
        m.push(Layer::overlay(lit(5, 180), vec![(5, 1.0)]));
        m.push(Layer::overlay(lit(6, 90), vec![(6, 1.0)]));
        let out = m.render();
        assert_eq!((out[5], out[6]), (180, 90));
    }
}
