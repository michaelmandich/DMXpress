//! The output compositor — DMXpress's renamed grandMA3 *mixer*.
//!
//! Every playback object (the live look, the chase overlay, and later Decks
//! and Phasers) hands the mixer a [`Layer`]. Each frame the mixer flattens its
//! layer stack bottom→top into the single [`Frame`] sent to Art-Net, so new
//! features become new layers instead of new special cases in the render loop.

use crate::net::Frame;

/// One contribution to the final output frame: `frame`'s values blended onto
/// the channels in `weights` (0..1 each — a weight of 1 fully asserts the
/// channel, lower weights let lower layers show through, e.g. a stack fader).
pub(crate) struct Layer {
    frame: Frame,
    weights: Vec<(usize, f32)>,
}

impl Layer {
    /// A layer that blends `frame` over everything beneath it on the given
    /// channels only.
    pub fn overlay(frame: Frame, weights: Vec<(usize, f32)>) -> Self {
        Self { frame, weights }
    }

    /// Merge this layer onto `out`, which already holds everything beneath it.
    fn merge_into(&self, out: &mut Frame) {
        for &(i, w) in &self.weights {
            out.blend_channel(i, self.frame[i], w);
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
