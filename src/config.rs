//! Whole-show configurations: every persisted setting bundled into one named
//! file so a complete rig (patch additions, stage layout, stage settings, and
//! all pools) can be saved and restored — e.g. switching between two entirely
//! different lighting setups.
//!
//! Saved under `configs/<name>.json` next to the other state files.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::group::Group;
use crate::layer::ProgLayer;
use crate::order::Order;
use crate::palette::Palette;
use crate::phaser::Phaser;
use crate::preset::UserPreset;
use crate::profiles::UserFixture;
use crate::scene::Scene;
use crate::showbuddy::Fixture;
use crate::stack::Stack;
use crate::stage::{CameraBookmark, LayoutFile, Settings};
use crate::transition::TransitionFile;
use crate::view::View;

pub const CONFIGS_DIR: &str = "configs";

/// Everything a show setup consists of.
#[derive(Serialize, Deserialize)]
pub struct Configuration {
    pub settings: Settings,
    /// 3D stage arrangement (light placements, duplicates, towers).
    pub layout: LayoutFile,
    /// Fixtures patched in DMXpress on top of the ShowBuddy patch.
    #[serde(default)]
    pub user_fixtures: Vec<UserFixture>,
    /// Whether the ShowBuddy patch is merged in at all.
    #[serde(default = "yes")]
    pub include_showbuddy: bool,
    /// The ShowBuddy-derived fixtures as they stood when this show was saved.
    ///
    /// The layout only stores `display@address` keys, and ShowBuddy itself
    /// lives at a fixed absolute macOS path outside this repository, so
    /// without this snapshot a cloned show loses every ShowBuddy light and
    /// keeps only the DMXpress-patched ones. Absent in configurations written
    /// before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub showbuddy_patch: Option<Vec<Fixture>>,
    /// Individual ShowBuddy fixtures hidden from the rig.
    #[serde(default)]
    pub excluded_fixtures: Vec<String>,
    #[serde(default)]
    pub groups: Vec<Group>,
    /// Custom effect routes. Absent in configurations written before orders
    /// existed, in which case effects simply fan out in patch order.
    #[serde(default)]
    pub orders: Vec<Order>,
    #[serde(default)]
    pub palettes: Vec<Palette>,
    #[serde(default)]
    pub phasers: Vec<Phaser>,
    #[serde(default)]
    pub user_presets: Vec<UserPreset>,
    #[serde(default)]
    pub preset_folders: Vec<String>,
    #[serde(default)]
    pub stacks: Vec<Stack>,
    /// Captured layerable effect states. Absent in configurations written
    /// before scenes existed.
    #[serde(default)]
    pub scenes: Vec<Scene>,
    /// Programmer layers, bottom→top. Absent in configurations written
    /// before layers existed, which restore as a single base layer.
    #[serde(default)]
    pub layers: Vec<ProgLayer>,
    #[serde(default)]
    pub views: Vec<View>,
    /// Saved stage-camera bookmarks (Inspector → Stage → Saved cameras).
    /// Absent in configurations written before them.
    #[serde(default)]
    pub cameras: Vec<CameraBookmark>,
    #[serde(default)]
    pub universe: u16,
    #[serde(default = "one")]
    pub grand_master: f32,
    /// Every fade time in the console: the global transition, advanced
    /// mode, and the per-action slots. Absent in configurations written
    /// before the transition desk, which start from the defaults.
    #[serde(default)]
    pub transition: TransitionFile,
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

fn path(name: &str) -> PathBuf {
    let safe: String = name
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_().".contains(c) { c } else { '_' })
        .collect();
    crate::paths::data_path(CONFIGS_DIR).join(format!("{safe}.json"))
}

pub fn list() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(crate::paths::data_path(CONFIGS_DIR)) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(stem.to_string());
                }
            }
        }
    }
    out.sort();
    out
}

pub fn save(name: &str, cfg: &Configuration) -> bool {
    if name.trim().is_empty() {
        return false;
    }
    let _ = std::fs::create_dir_all(crate::paths::data_path(CONFIGS_DIR));
    serde_json::to_string_pretty(cfg)
        .ok()
        .and_then(|json| std::fs::write(path(name), json).ok())
        .is_some()
}

pub fn load(name: &str) -> Option<Configuration> {
    let text = std::fs::read_to_string(path(name)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn delete(name: &str) {
    let _ = std::fs::remove_file(path(name));
}
