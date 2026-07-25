//! Light placement transforms, scene instances, and floor-stand towers.

use eframe::egui::Color32;
use serde::{Deserialize, Serialize};

use super::math::{dir_from_angles, v3, V3};
use super::render::{add_box, Mesh};
use super::settings::Settings;
use crate::showbuddy::Fixture;

/// Editable placement of one light in the 3D scene.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LightTransform {
    pub pos: V3,
    /// Mounting orientation in degrees. Pan/tilt offsets are applied on top.
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    /// Spin of the fixture around its own forward (mounting) axis. For a
    /// base-mounted light (moving head) this is the only manual rotation —
    /// it spins the base, reorienting the pan/tilt sweep, with no gimbal
    /// tumbling. Pars ignore it (radially symmetric).
    #[serde(default)]
    pub roll_deg: f32,
    /// Per-fixture size multiplier on top of the global light scale.
    #[serde(default = "one")]
    pub scale: f32,
}

pub(crate) fn one() -> f32 {
    1.0
}

pub(crate) fn layout_key(f: &Fixture) -> String {
    format!("{}@{}", f.display, f.from)
}

pub(crate) fn default_transform(f: &Fixture, set: &Settings) -> LightTransform {
    LightTransform {
        // Spread initial placement using the ShowBuddy 2D layout.
        pos: v3((f.x - 0.5) * 12.0, set.default_height, (f.y - 0.5) * 10.0),
        yaw_deg: set.default_yaw,
        pitch_deg: set.default_pitch,
        roll_deg: 0.0,
        scale: 1.0,
    }
}

/// One visual light in the scene. Several instances may point at the same
/// patch fixture (physical duplicates wired to the same DMX addresses).
#[derive(Debug, Clone)]
pub(crate) struct Instance {
    /// Index into `patch.fixtures`.
    pub fixture: usize,
    pub t: LightTransform,
    /// Visualizer-only opacity for housing, lens, beam and surface pool.
    pub opacity: f32,
    /// Snapped onto a tower slot: (tower index, slot 0..8; 0..4 top, 4..8 bottom).
    pub mount: Option<(usize, usize)>,
}

pub(crate) const TOWER_SLOTS: usize = 8;
/// Vertical offset of a mounted light from the crossbar centre.
const TOWER_SLOT_OFFSET: f32 = 0.2;

/// Floor stand: pole topped with one crossbar. Lights clip on top (pointing
/// up) or underneath (pointing down) — 4 slots per face, 8 total.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Tower {
    /// Base center on the floor (y = 0).
    pub pos: V3,
    pub yaw_deg: f32,
    pub height: f32,
    pub width: f32,
}

impl Default for Tower {
    fn default() -> Self {
        Self {
            pos: v3(0.0, 0.0, 4.0),
            yaw_deg: 0.0,
            height: 3.2,
            width: 2.4,
        }
    }
}

impl Tower {
    /// Horizontal crossbar direction.
    pub fn bar_dir(&self) -> V3 {
        dir_from_angles(self.yaw_deg + 90.0, 0.0)
    }

    /// Slots 0..4 sit on top of the bar (point up); 4..8 hang underneath
    /// (point down).
    pub fn slot_points_up(slot: usize) -> bool {
        slot < 4
    }

    /// World position a light snapped into `slot` mounts at.
    pub fn slot_pos(&self, slot: usize) -> V3 {
        let k = (slot % 4) as f32;
        let off = if Self::slot_points_up(slot) {
            TOWER_SLOT_OFFSET
        } else {
            -TOWER_SLOT_OFFSET
        };
        self.pos
            + self.bar_dir() * ((k - 1.5) * self.width / 3.0)
            + v3(0.0, self.height + off, 0.0)
    }

    pub fn mesh(&self, selected: bool) -> Mesh {
        let mut m = Mesh::default();
        let col = if selected {
            Color32::from_gray(120)
        } else {
            Color32::from_gray(70)
        };
        let bar = self.bar_dir();
        let fwd = dir_from_angles(self.yaw_deg, 0.0);
        let up = v3(0.0, 1.0, 0.0);
        // Pole.
        add_box(
            &mut m,
            self.pos + up * (self.height * 0.5),
            bar * 0.045,
            up * (self.height * 0.5),
            fwd * 0.045,
            col,
            None,
        );
        // Single crossbar at the top.
        add_box(
            &mut m,
            self.pos + up * self.height,
            bar * (self.width * 0.5),
            up * 0.04,
            fwd * 0.04,
            col,
            None,
        );
        // Crossed feet.
        add_box(&mut m, self.pos + up * 0.03, bar * 0.5, up * 0.03, fwd * 0.06, col, None);
        add_box(&mut m, self.pos + up * 0.03, bar * 0.06, up * 0.03, fwd * 0.5, col, None);
        m
    }
}

/// On-disk layout: light instances + towers. Older builds stored a plain
/// `{ key -> transform }` map, which is still read as a fallback.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct LayoutFile {
    #[serde(default)]
    pub instances: Vec<SavedInstance>,
    #[serde(default)]
    pub towers: Vec<Tower>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SavedInstance {
    pub key: String,
    pub t: LightTransform,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub mount: Option<(usize, usize)>,
}
