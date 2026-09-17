//! Native DMXpress presets: a saved snapshot of the programmer — asserted
//! base values plus any running oscillators — recallable like a ShowBuddy
//! preset but stored in `presets.json`, independent of ShowBuddy.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::net::Frame;
use crate::oscillator::{CustomWaveform, Osc};
use crate::palette::SeqPattern;
use crate::streamdeck::KeyIcon;

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
    /// Stable id, 1-based and never reused. 0 = written before ids existed;
    /// `assign_ids` fixes that up on load.
    #[serde(default)]
    pub id: u32,
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
    /// Stacked effect lanes (island name → palette ids): one id holds that
    /// slot, several step through them on the cycle clock.
    #[serde(default)]
    pub lanes: Vec<(String, Vec<u32>)>,
    /// Pad and swatch colour picked by hand; `None` = derived from the values.
    #[serde(default)]
    pub color: Option<[u8; 3]>,
    /// Up to four characters shown large on a pad / key; empty = from the name.
    #[serde(default)]
    pub symbol: String,
    /// Artwork behind the symbol on a deck key.
    #[serde(default)]
    pub icon: KeyIcon,
    /// Recall fade in seconds; `None` = the tab's recall mode / the Transition window.
    #[serde(default)]
    pub fade: Option<f32>,
    /// Shown in the Pinned strip at the top of the Presets tab.
    #[serde(default)]
    pub pinned: bool,
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

/// The next free id: one past the highest in use (1 when there are none).
pub fn next_id(presets: &[UserPreset]) -> u32 {
    presets.iter().map(|p| p.id).max().map_or(1, |m| m + 1)
}

/// Give every preset with no id (0) or a duplicate id a fresh one. Returns the next free id.
pub fn assign_ids(presets: &mut [UserPreset]) -> u32 {
    let mut next = next_id(presets);
    let mut seen = std::collections::HashSet::new();
    for p in presets.iter_mut() {
        if p.id == 0 || !seen.insert(p.id) {
            p.id = next;
            seen.insert(p.id);
            next += 1;
        }
    }
    next
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
    let mut store = if let Ok(store) = serde_json::from_str::<PresetStore>(&s) {
        store
    } else if let Ok(presets) = serde_json::from_str::<Vec<UserPreset>>(&s) {
        // Older format: a bare preset list without folders.
        PresetStore { folders: Vec::new(), presets }
    } else {
        PresetStore::default()
    };
    assign_ids(&mut store.presets);
    store
}

pub fn save_presets(folders: &[String], presets: &[UserPreset]) {
    let store = PresetStore { folders: folders.to_vec(), presets: presets.to_vec() };
    if let Ok(json) = serde_json::to_string_pretty(&store) {
        let _ = std::fs::write(PRESETS_FILE, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A presets.json written before ids existed gets 1..n on load; zero and
    /// duplicate ids are replaced, unique ones are left alone.
    #[test]
    fn old_presets_json_gets_ids() {
        let json = r#"{"folders":[],"presets":[
            {"name":"A","values":[[0,255]],"speed":1.0,"tempo":120.0},
            {"name":"B","values":[[1,128]],"speed":1.0,"tempo":120.0}
        ]}"#;
        let mut store: PresetStore = serde_json::from_str(json).unwrap();
        assert!(store.presets.iter().all(|p| p.id == 0));
        assert_eq!(assign_ids(&mut store.presets), 3);
        let ids: Vec<u32> = store.presets.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![1, 2]);

        let mut dup = store.presets.clone();
        dup.extend(store.presets.iter().cloned());
        for (p, id) in dup.iter_mut().zip([0u32, 5, 5, 0]) {
            p.id = id;
        }
        assert_eq!(assign_ids(&mut dup), 9);
        let ids: Vec<u32> = dup.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![6, 5, 7, 8]);

        // Already unique: untouched.
        let before: Vec<u32> = dup.iter().map(|p| p.id).collect();
        assert_eq!(assign_ids(&mut dup), 9);
        let after: Vec<u32> = dup.iter().map(|p| p.id).collect();
        assert_eq!(before, after);
    }
}
