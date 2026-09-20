//! Basic 3D stage visualizer and light placement editor.
//!
//! Painter-based 3D geometry projected onto the egui canvas, with a custom
//! Metal/wgpu participating-media shader for smooth volumetric beams.
//!
//! Units are meters, Y is up. The stage is a raised box centered at origin.
//!
//! The module is split by concern:
//! - [`math`]: vectors, the camera, and small geometry helpers.
//! - [`settings`]: persisted stage / fixture defaults.
//! - [`fixture`]: archetype classification and live DMX-derived render state.
//! - [`layout`]: light transforms, scene instances, and floor-stand towers.
//! - [`gizmo`]: transform-gizmo handles and the drag-state machine.
//! - [`render`]: fixture meshes.
//! - [`atlas`]: the gobo masks the patch can show, as one GPU texture array.
//! - `volumetric`: GPU beam/pool bounds, resources and the beam shader.
//! - [`view`]: the `StageView` state plus persistence and selection helpers.
//! - [`geometry`]: snap-to-tower and gizmo picking math.
//! - [`input`]: pointer / keyboard handling (`StageView::ui`).
//! - [`draw`]: scene rendering (`StageView::draw_scene`).
//! - [`inspector`]: the light / tower / truss editor grids the Selection tab embeds.
//! - [`arrange`]: selection transform tools (align, distribute, mirror, arrange, aim, hang) as `StageView` methods.
//! - [`builder`]: element/composite builders, groups, outliner data, rig summary and setup listing.
//! - [`camera_views`]: quick views, framing, camera glides and saved cameras.
//! - [`place`]: placement patterns and mounting primitives for the Patch tab.

mod arrange;
mod atlas;
mod builder;
mod camera_views;
mod draw;
mod fixture;
#[cfg(test)]
pub(crate) mod headless;
mod geometry;
mod gizmo;
mod input;
mod inspector;
mod layout;
mod math;
mod place;
mod render;
mod settings;
mod transition_marker;
mod view;
mod volumetric;

const LAYOUT_FILE: &str = "stage_layout.json";
const SETTINGS_FILE: &str = "settings.json";
const SETUPS_DIR: &str = "setups";

// Public API used by the rest of the app.
pub use fixture::{fixture_level, fixture_swatch};
pub(crate) use layout::LayoutFile;
pub(crate) use layout::TrussKind;
pub(crate) use layout::{ElementRef, TOWER_SLOTS};
#[cfg(test)]
pub(crate) use layout::{Instance, LightTransform, Tower, Truss};
pub(crate) use math::{dir_from_angles, v3, V3};
pub(crate) use math::CameraSnapshot;
pub use settings::{DisplayToggles, RaidLook, Settings};
pub use view::StageView;
pub(crate) use view::{Sweep, SweepShape};
pub(crate) use volumetric::initialize as initialize_volumetric;
pub use camera_views::{load_cameras, save_cameras, CameraBookmark};
pub(crate) use fixture::{classify, Archetype};
// -- area B re-exports --
pub(crate) use camera_views::{assign_hotkey, bookmark_by_hotkey, unique_name, QuickView};
pub(crate) use settings::GridPitch;
// -- area C re-exports --
pub(crate) use builder::{
    ElementInfo, Placement, Recipe, RecipeKind, RigSummary, SetupInfo, ARC_PRESETS, F34_LENGTHS,
    HEIGHT_PRESETS, RADIUS_PRESETS, SPAN_PRESETS,
};
pub(crate) use layout::{CompositeKind, ElementGroup};
// -- area E re-exports --
pub(crate) use arrange::{
    aim_pan_tilt, AimTarget, AlignMode, ArrangeShape, Axis, DistributeMode, Facing, HangFill,
    MirrorPivot, MirrorPlane, OrderBy, PasteParts, SelectionSummary, TransformClip, NUDGE_STEPS,
    ROT_STEPS, SNAP_STEPS,
};
pub(crate) use inspector::{ElementAction, ElementSpot};
// -- area F re-exports --
pub(crate) use place::{fitted_truss, PlacePattern, PlaceSpec};
