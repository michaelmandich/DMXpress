//! Views — saved workspace layouts (which windows/bars are open).
//!
//! grandMA3 calls these *Views*; here a `View` is just a named snapshot of the
//! panel-visibility flags, so you can flip between "Programming", "Playback",
//! and "Patch" workspaces in one click.

use std::fs;

use serde::{Deserialize, Serialize};

use crate::stage::CameraSnapshot;

const VIEWS_FILE: &str = "views.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct View {
    pub name: String,
    /// Stage camera pose. Older workspace views simply leave this empty.
    #[serde(default)]
    pub(crate) camera: Option<CameraSnapshot>,
    #[serde(default)]
    pub(crate) fly_mode: bool,
    pub artnet: bool,
    pub transition: bool,
    pub chases: bool,
    pub groups: bool,
    #[serde(default)]
    pub orders: bool,
    /// Absent in views saved before programmer layers existed.
    #[serde(default)]
    pub layers: bool,
    #[serde(default)]
    pub scenes: bool,
    #[serde(default)]
    pub audio: bool,
    pub palettes: bool,
    pub phasers: bool,
    pub stacks: bool,
    pub decks: bool,
    pub command: bool,
    pub log: bool,
    pub osc: bool,
    /// Docked panels that are open rather than folded away. Views saved
    /// before folding existed leave every panel open.
    #[serde(default = "yes")]
    pub fixtures: bool,
    #[serde(default = "yes")]
    pub inspector: bool,
    #[serde(default = "yes")]
    pub channels: bool,
    #[serde(default)]
    pub phaser_board: bool,
    /// The Inspector tab the view was saved on. Older views leave it unset.
    #[serde(default)]
    pub inspector_tab: Option<crate::ui::inspector::InspectorTab>,
    /// The Presets board window. Older views leave it closed.
    #[serde(default)]
    pub preset_board: bool,
}

fn yes() -> bool {
    true
}

pub fn load_views() -> Vec<View> {
    match fs::read_to_string(crate::paths::data_path(VIEWS_FILE)) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn save_views(views: &[View]) {
    if let Ok(text) = serde_json::to_string_pretty(views) {
        let _ = fs::write(crate::paths::data_path(VIEWS_FILE), text);
    }
}
