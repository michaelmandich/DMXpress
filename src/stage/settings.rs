//! User-configurable stage / fixture defaults, persisted to settings.json.

use serde::{Deserialize, Serialize};

use super::SETTINGS_FILE;

/// The material the Fixtures panel's raid-grid tiles are drawn in. Every
/// look shows the same thing — live colour, name, DMX address — but as a
/// different kind of object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RaidLook {
    /// A colour tile lit from above, matching the console's raised controls.
    #[default]
    Lit,
    /// An amplifier pilot-light jewel: a faceted lens in a chrome bezel on
    /// a dark faceplate, glowing with the fixture's colour.
    Jewel,
    /// A chamfered black slab with a glowing colour bar and corner brackets.
    Future,
    /// A pane of tinted glass with a specular streak.
    Glass,
    /// A domed lens set into a brushed-steel frame.
    Chrome,
    /// A soft convex button, deep-rounded and shadowed.
    Pillow,
}

impl RaidLook {
    pub const ALL: [RaidLook; 6] = [
        RaidLook::Lit,
        RaidLook::Jewel,
        RaidLook::Future,
        RaidLook::Glass,
        RaidLook::Chrome,
        RaidLook::Pillow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RaidLook::Lit => "Lit",
            RaidLook::Jewel => "Amp jewel",
            RaidLook::Future => "Future",
            RaidLook::Glass => "Glass",
            RaidLook::Chrome => "Chrome",
            RaidLook::Pillow => "Pillow",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            RaidLook::Lit => "Colour tiles lit from above, like the rest of the console",
            RaidLook::Jewel => "Faceted pilot-light jewels in chrome bezels on a dark faceplate",
            RaidLook::Future => "Chamfered black slabs with a glowing colour bar",
            RaidLook::Glass => "Tinted glass panes with a specular streak",
            RaidLook::Chrome => "Domed lenses in brushed-steel frames",
            RaidLook::Pillow => "Soft convex buttons with deep shadows",
        }
    }
}

/// Floor-grid line spacing (an enum so `DisplayToggles` stays `Eq`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GridPitch {
    Half,
    One,
    #[default]
    Two,
    Five,
}

impl GridPitch {
    pub const ALL: [GridPitch; 4] =
        [GridPitch::Half, GridPitch::One, GridPitch::Two, GridPitch::Five];

    pub fn label(self) -> &'static str {
        match self {
            Self::Half => "0.5 m",
            Self::One => "1 m",
            Self::Two => "2 m",
            Self::Five => "5 m",
        }
    }

    pub fn metres(self) -> f32 {
        match self {
            Self::Half => 0.5,
            Self::One => 1.0,
            Self::Two => 2.0,
            Self::Five => 5.0,
        }
    }
}

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
    /// How the Fixtures panel draws its raid grid.
    #[serde(default)]
    pub raid_look: RaidLook,
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
            raid_look: RaidLook::Lit,
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

/// What the stage draws. Cosmetic — persisted by the Inspector in
/// inspector.json, copied onto `StageView.show` every frame; deliberately
/// not in `Settings` (which is in Configuration → undo churn).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayToggles {
    pub grid: bool,
    pub stage_box: bool,
    pub towers: bool,
    pub trusses: bool,
    pub beams: bool,
    pub pools: bool,
    pub labels: bool,
    pub gizmo: bool,
    pub help: bool,
    /// Blue slot rings on a picked tower / truss and while dragging a light.
    pub slot_rings: bool,
    /// Aim ticks on dark lights.
    pub ticks: bool,
    /// Transition sphere, chase band and phaser traces.
    pub overlays: bool,
    /// With `labels`: name every light, not only the selected ones.
    pub label_all: bool,
    pub grid_pitch: GridPitch,
}

impl Default for DisplayToggles {
    fn default() -> Self {
        Self {
            grid: true,
            stage_box: true,
            towers: true,
            trusses: true,
            beams: true,
            pools: true,
            labels: true,
            gizmo: true,
            help: true,
            slot_rings: true,
            ticks: true,
            overlays: true,
            label_all: false,
            grid_pitch: GridPitch::Two,
        }
    }
}
