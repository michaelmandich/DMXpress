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
//! - `volumetric`: GPU beam bounds, resources and participating-media shader.
//! - [`view`]: the `StageView` state plus persistence and selection helpers.
//! - [`geometry`]: snap-to-tower and gizmo picking math.
//! - [`input`]: pointer / keyboard handling (`StageView::ui`).
//! - [`draw`]: scene rendering (`StageView::draw_scene`).
//! - [`inspector`]: the right-hand transform editor.

mod draw;
mod fixture;
mod geometry;
mod gizmo;
mod input;
mod inspector;
mod layout;
mod math;
mod render;
mod settings;
mod transition_marker;
mod view;
mod volumetric;

const LAYOUT_FILE: &str = "stage_layout.json";
const SETTINGS_FILE: &str = "settings.json";
const SETUPS_DIR: &str = "setups";

// Public API used by the rest of the app.
pub use fixture::fixture_swatch;
pub(crate) use layout::LayoutFile;
pub(crate) use math::{dir_from_angles, v3, V3};
pub(crate) use math::CameraSnapshot;
pub use settings::Settings;
pub use view::StageView;
pub(crate) use volumetric::initialize as initialize_volumetric;
