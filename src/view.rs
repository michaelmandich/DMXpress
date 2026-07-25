//! Views — saved workspace layouts (which windows/bars are open).
//!
//! grandMA3 calls these *Views*; here a `View` is just a named snapshot of the
//! panel-visibility flags, so you can flip between "Programming", "Playback",
//! and "Patch" workspaces in one click.

use std::fs;
use std::path::Path;

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
    #[serde(default)]
    pub scenes: bool,
    pub palettes: bool,
    pub phasers: bool,
    pub stacks: bool,
    pub decks: bool,
    pub command: bool,
    pub log: bool,
    pub osc: bool,
}

pub fn load_views() -> Vec<View> {
    match fs::read_to_string(Path::new(VIEWS_FILE)) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn save_views(views: &[View]) {
    if let Ok(text) = serde_json::to_string_pretty(views) {
        let _ = fs::write(Path::new(VIEWS_FILE), text);
    }
}
