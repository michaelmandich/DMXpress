//! Transform-gizmo handle identity and the drag-state machine.

use eframe::egui::{Color32, Pos2};

use super::math::{v3, V3};
use super::settings::Settings;

/// One handle of the Fusion-style transform gizmo.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GizmoPart {
    TransX,
    TransY,
    TransZ,
    RotX,
    RotY,
    RotZ,
}

impl GizmoPart {
    pub fn axis(self) -> V3 {
        match self {
            GizmoPart::TransX | GizmoPart::RotX => v3(1.0, 0.0, 0.0),
            GizmoPart::TransY | GizmoPart::RotY => v3(0.0, 1.0, 0.0),
            GizmoPart::TransZ | GizmoPart::RotZ => v3(0.0, 0.0, 1.0),
        }
    }
    pub fn is_rot(self) -> bool {
        matches!(self, GizmoPart::RotX | GizmoPart::RotY | GizmoPart::RotZ)
    }
    pub fn color(self) -> Color32 {
        match self {
            GizmoPart::TransX | GizmoPart::RotX => Color32::from_rgb(232, 86, 86),
            GizmoPart::TransY | GizmoPart::RotY => Color32::from_rgb(96, 206, 96),
            GizmoPart::TransZ | GizmoPart::RotZ => Color32::from_rgb(92, 152, 236),
        }
    }
}

pub(crate) enum Drag {
    None,
    Move,
    MoveTower,
    MoveTransitionSphere,
    MoveChaseSphere,
    PanCam,
    Marquee(Pos2),
    /// Dragging a transform-gizmo handle.
    Gizmo(GizmoPart),
    /// Dragging one of the stage-box resize arrows.
    StageEdge(StageHandle),
}

/// One of the stage-box resize arrows: the four top-edge midpoints resize
/// width/depth (symmetric about the origin), the centre arrow sets height.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StageHandle {
    East,
    West,
    South,
    North,
    Top,
}

impl StageHandle {
    pub const ALL: [StageHandle; 5] = [
        StageHandle::East,
        StageHandle::West,
        StageHandle::South,
        StageHandle::North,
        StageHandle::Top,
    ];

    /// Outward direction the arrow points (and drags) along.
    pub fn axis(self) -> V3 {
        match self {
            StageHandle::East => v3(1.0, 0.0, 0.0),
            StageHandle::West => v3(-1.0, 0.0, 0.0),
            StageHandle::South => v3(0.0, 0.0, 1.0),
            StageHandle::North => v3(0.0, 0.0, -1.0),
            StageHandle::Top => v3(0.0, 1.0, 0.0),
        }
    }

    /// Where the arrow starts: edge midpoints on the stage top surface.
    pub fn anchor(self, set: &Settings) -> V3 {
        let (hw, h, hd) = (set.stage_half_w, set.stage_h, set.stage_half_d);
        match self {
            StageHandle::East => v3(hw, h, 0.0),
            StageHandle::West => v3(-hw, h, 0.0),
            StageHandle::South => v3(0.0, h, hd),
            StageHandle::North => v3(0.0, h, -hd),
            StageHandle::Top => v3(0.0, h, 0.0),
        }
    }

    /// Apply a drag of `world` units along the arrow's outward axis.
    pub fn apply(self, set: &mut Settings, world: f32) {
        match self {
            StageHandle::East | StageHandle::West => {
                set.stage_half_w = (set.stage_half_w + world).clamp(0.5, 20.0);
            }
            StageHandle::South | StageHandle::North => {
                set.stage_half_d = (set.stage_half_d + world).clamp(0.5, 20.0);
            }
            StageHandle::Top => {
                set.stage_h = (set.stage_h + world).clamp(0.0, 5.0);
            }
        }
    }
}
