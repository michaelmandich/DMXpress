//! Native DMXpress presets: a saved snapshot of the programmer — asserted
//! base values plus any running oscillators — recallable like a ShowBuddy
//! preset but stored in `presets.json`, independent of ShowBuddy.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::net::Frame;
use crate::oscillator::{CustomWaveform, Osc};
use crate::palette::SeqPattern;

pub const PRESETS_FILE: &str = "presets.json";

/// Serialized oscillator parameters for one channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedOsc {
    pub invert: bool,
    pub amount: f32,
    pub phase: f32,
    pub subdiv: Option<f32>,
    pub shape: f32,
    #[serde(default)]
    pub custom_wave: Option<CustomWaveform>,
    #[serde(default = "default_true")]
    pub master_beat: bool,
    #[serde(default)]
    pub local_beats: f32,
    #[serde(default = "default_tempo")]
    pub local_tempo: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedCycle {
    pub ids: Vec<u32>,
    #[serde(default)]
    pub weights: Vec<f32>,
    pub beats_per: f32,
    pub spread: f32,
    pub pattern: SeqPattern,
    pub shape: f32,
    pub master_beat: bool,
    pub tempo: f32,
}

fn default_true() -> bool { true }
fn default_tempo() -> f32 { 120.0 }
fn default_master_speed() -> f32 { 1.0 }

/// One stored look: what the programmer held when it was saved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreset {
    pub name: String,
    /// Folder this preset lives in (empty = top level).
    #[serde(default)]
    pub folder: String,
    /// Asserted base values (0-based address → value).
    pub values: Vec<(usize, u8)>,
    /// Oscillators (0-based address → parameters).
    #[serde(default)]
    pub oscs: Vec<(usize, SavedOsc)>,
    pub speed: f32,
    pub tempo: f32,
    #[serde(default = "default_master_speed")]
    pub master_speed: f32,
    /// Runtime sources captured with the look. Recall restores these only
    /// where no currently-running live effect already owns the same layer.
    #[serde(default)]
    pub active_phasers: Vec<(String, Vec<usize>)>,
    #[serde(default)]
    pub add_overrides: Vec<(usize, i16)>,
    #[serde(default)]
    pub hold_overrides: Vec<(usize, u8)>,
    #[serde(default)]
    pub cycle: Option<SavedCycle>,
}

impl UserPreset {
    /// Rebuild the base frame (black everywhere the preset says nothing).
    pub(crate) fn base_frame(&self) -> Frame {
        let mut f = Frame::black();
        for &(a, v) in &self.values {
            if a < f.len() {
                f[a] = v;
            }
        }
        f
    }

    /// Rebuild the oscillator map.
    pub(crate) fn osc_map(&self) -> HashMap<usize, Osc> {
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
}

/// Everything in `presets.json`: folders (which may be empty) and presets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresetStore {
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub presets: Vec<UserPreset>,
}

pub fn load_presets() -> PresetStore {
    let Ok(s) = std::fs::read_to_string(PRESETS_FILE) else {
        return PresetStore::default();
    };
    if let Ok(store) = serde_json::from_str::<PresetStore>(&s) {
        store
    } else if let Ok(presets) = serde_json::from_str::<Vec<UserPreset>>(&s) {
        // Older format: a bare preset list without folders.
        PresetStore { folders: Vec::new(), presets }
    } else {
        PresetStore::default()
    }
}

pub fn save_presets(folders: &[String], presets: &[UserPreset]) {
    let store = PresetStore { folders: folders.to_vec(), presets: presets.to_vec() };
    if let Ok(json) = serde_json::to_string_pretty(&store) {
        let _ = std::fs::write(PRESETS_FILE, json);
    }
}
