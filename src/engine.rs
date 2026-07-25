//! The output compositor — DMXpress's renamed grandMA3 *mixer*.
//!
//! Every playback object (the live look, the chase overlay, and later Decks
//! and Phasers) hands the mixer a [`Layer`]. Each frame the mixer flattens its
//! layer stack bottom→top into the single [`Frame`] sent to Art-Net, so new
//! features become new layers instead of new special cases in the render loop.

use crate::net::Frame;

/// How a layer folds into everything beneath it.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
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
pub(crate) struct Layer {
    frame: Frame,
    weights: Vec<(usize, f32)>,
    blend: Blend,
}

impl Layer {
    /// A layer that blends `frame` over everything beneath it on the given
    /// channels only.
    pub fn overlay(frame: Frame, weights: Vec<(usize, f32)>) -> Self {
        Self {
            frame,
            weights,
            blend: Blend::Mix,
        }
    }

    /// Fold this layer in with something other than a straight crossfade.
    pub fn with_blend(mut self, blend: Blend) -> Self {
        self.blend = blend;
        self
    }

    /// Merge this layer onto `out`, which already holds everything beneath it.
    fn merge_into(&self, out: &mut Frame) {
        for &(i, w) in &self.weights {
            if i >= out.len() {
                continue;
            }
            match self.blend {
                Blend::Mix => out.blend_channel(i, self.frame[i], w),
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
    pub fn render(&self) -> Frame {
        let mut out = Frame::black();
        for layer in &self.stack {
            layer.merge_into(&mut out);
        }
        out
    }
}
