//! User-configurable stage / fixture defaults, persisted to settings.json.

use serde::{Deserialize, Serialize};

use super::SETTINGS_FILE;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Stage half-width (m).
    pub stage_half_w: f32,
    /// Stage half-depth (m).
    pub stage_half_d: f32,
    /// Stage height (m).
    pub stage_h: f32,
    /// Multiplier on fixture body size.
    pub light_scale: f32,
    /// Multiplier on beam "air-catching" opacity (1.0 = default).
    #[serde(default = "super::layout::one")]
    pub beam_opacity: f32,
    /// Defaults for newly placed lights.
    pub default_height: f32,
    pub default_yaw: f32,
    pub default_pitch: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stage_half_w: 4.0,
            stage_half_d: 3.0,
            stage_h: 1.0,
            light_scale: 1.0,
            beam_opacity: 1.0,
            default_height: 4.0,
            default_yaw: 0.0,
            default_pitch: -90.0,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read_to_string(SETTINGS_FILE)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(SETTINGS_FILE, json);
        }
    }
}
