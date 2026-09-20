//! Inspector · Stage tab (area B): how the camera moves, the saved camera
//! bookmarks, what the stage draws and how big the stage box is.
//!
//! Nothing here is undoable. Camera moves are not show data at all (the
//! "previous camera" button is the escape hatch); the display switches live
//! in inspector.json per machine; only the stage size and the two look
//! sliders write `Settings`, which the Configuration undo already covers.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use eframe::egui::{self, Color32, Key};
use serde::{Deserialize, Serialize};

use super::STAGE_HELP;
use crate::app::App;
use crate::stage::{self, CameraSnapshot, DisplayToggles, GridPitch, QuickView, V3};
use crate::ui::{icons::Icon, theme};

/// Stage-tab preferences, persisted at `InspectorPrefs.stage`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct StagePrefs {
    pub animate_camera: bool,
    pub camera_tracks_selection: bool,
    pub quick_view_keeps_zoom: bool,
    pub show_camera_numbers: bool,
    pub restore_camera: bool,
    /// Written by App::on_exit, applied by App::new when restore_camera.
    pub last_camera: Option<CameraSnapshot>,
    pub tour_seconds: f32,
}

impl Default for StagePrefs {
    fn default() -> Self {
        Self {
            animate_camera: true,
            camera_tracks_selection: false,
            quick_view_keeps_zoom: false,
            show_camera_numbers: false,
            restore_camera: true,
            last_camera: None,
            tour_seconds: 8.0,
        }
    }
}

/// A camera tour in progress: which bookmark is next and when it is due.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tour {
    pub next: usize,
    pub at: f64,
}

/// Transient Stage-tab state (never persisted).
#[derive(Default)]
pub(crate) struct StageUi {
    /// The name typed into the Save row.
    pub camera_name: String,
    /// Which bookmark is being renamed inline, and its draft.
    pub rename: Option<(usize, String)>,
    /// The rename field still wants the keyboard.
    pub rename_focus: bool,
    /// Hash of the stage focus the tracking camera last moved for.
    pub last_track_key: u64,
    pub tour: Option<Tour>,
    /// A stage PNG was asked for and the reply has not arrived yet.
    pub snapshot_pending: bool,
}

/// One of the three named sets of display switches.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayPreset {
    All,
    Clean,
    Plan,
}

impl DisplayPreset {
    const ALL: [DisplayPreset; 3] = [DisplayPreset::All, DisplayPreset::Clean, DisplayPreset::Plan];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Clean => "Clean",
            Self::Plan => "Plan",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::All => "Everything on.",
            Self::Clean => {
                "What the audience sees: no grid, rings, gizmo, ticks, help, overlays or names."
            }
            Self::Plan => "Rig plan: names on every light, no beams or pools.",
        }
    }

    fn toggles(self) -> DisplayToggles {
        let d = DisplayToggles::default();
        match self {
            Self::All => d,
            Self::Clean => DisplayToggles {
                grid: false,
                slot_rings: false,
                gizmo: false,
                ticks: false,
                help: false,
                overlays: false,
                labels: false,
                label_all: false,
                ..d
            },
            Self::Plan => DisplayToggles {
                beams: false,
                pools: false,
                help: false,
                overlays: false,
                labels: true,
                label_all: true,
                grid: true,
                ..d
            },
        }
    }
}

/// What a bookmark row asked for this frame, applied after the list.
enum CamAction {
    Recall(usize),
    Update(usize),
    RenameStart(usize),
    RenameCommit(usize, String),
    RenameCancel,
    Hotkey(usize, Option<u8>),
    Color(usize, Option<[u8; 3]>),
    Move(usize, isize),
    Delete(usize),
}

/// What a framing button asked for this frame.
enum FrameAction {
    Selection,
    All,
    LookAt,
    Previous,
    Reset,
    Snapshot,
}

/// Where "Snapshot" drops its PNGs, beside the show.
const SNAPSHOTS_DIR: &str = "screenshots";

/// The camera keys, for the "Stage controls" fold.
const CAMERA_HELP: &[(&str, &str)] = &[
    ("F", "Frame the selection (or the rig)"),
    ("⇧F", "Look at the selection"),
    ("Home", "Frame the whole rig"),
    ("1–9", "Recall a saved camera"),
    ("⇧1–9", "Store the camera on that key"),
    ("⇧ quick view", "Turn only, keep the zoom"),
];

/// Standard stage footprints, in metres.
const QUICK_SIZES: [(f32, f32); 6] =
    [(6.0, 4.0), (8.0, 6.0), (10.0, 8.0), (12.0, 10.0), (16.0, 12.0), (20.0, 12.0)];

const DIGITS: [Key; 9] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

/// The glyph on a quick-view pad. (`QuickView` lives in `stage`, which
/// cannot name the `ui::icons` types, so the mapping lives here.)
fn quick_icon(q: QuickView) -> Icon {
    match q {
        QuickView::Iso => Icon::CamIso,
        QuickView::Front => Icon::CamFront,
        QuickView::Back => Icon::CamBack,
        QuickView::Left => Icon::CamLeft,
        QuickView::Right => Icon::CamRight,
        QuickView::Top => Icon::CamTop,
        QuickView::Foh => Icon::CamFoh,
    }
}

/// One control in a [`tool_rows`] strip.
struct Tool<'a> {
    icon: Icon,
    label: Option<&'a str>,
    hint: &'a str,
    /// `Some` draws a lit toggle instead of a plain button.
    on: Option<bool>,
    gate: Result<(), &'a str>,
}

impl<'a> Tool<'a> {
    fn button(icon: Icon, label: Option<&'a str>, hint: &'a str) -> Self {
        Self { icon, label, hint, on: None, gate: Ok(()) }
    }

    fn gated(mut self, gate: Result<(), &'a str>) -> Self {
        self.gate = gate;
        self
    }

    fn toggle(icon: Icon, label: Option<&'a str>, on: bool, hint: &'a str) -> Self {
        Self { icon, label, hint, on: Some(on), gate: Ok(()) }
    }
}

/// The width `theme::tool_button` / `toggle_icon` will ask for — the icon
/// button's own arithmetic, so a strip can be measured before it is drawn.
fn tool_width(ui: &egui::Ui, label: Option<&str>) -> f32 {
    let pad = ui.spacing().button_padding;
    let icon = ui.text_style_height(&egui::TextStyle::Button) * 1.05;
    match label {
        None => (pad.y + 2.0) * 2.0 + icon,
        Some(text) => {
            let galley = egui::WidgetText::from(text).into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Button,
            );
            pad.x * 2.0 + icon + 5.0 + galley.size().x
        }
    }
}

/// A strip of icon controls laid out in rows that actually fit, returning
/// the index clicked this frame.
///
/// `theme::toolbar` cannot do this: every kit control is a `ui.scope`, and
/// a scope is placed with `allocate_rect`, which `horizontal_wrapped` never
/// breaks — the row would run off the panel and drag its width with it. So
/// the strip is measured here and broken into plain horizontal rows.
fn tool_rows(ui: &mut egui::Ui, tools: &[Tool]) -> Option<usize> {
    let mut clicked = None;
    ui.scope(|ui| {
        let z = theme::zoom_of(ui);
        let gap = 4.0 * z;
        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
        ui.spacing_mut().button_padding = egui::vec2(6.0 * z, 4.0 * z);
        let avail = ui.available_width();
        let widths: Vec<f32> = tools.iter().map(|t| tool_width(ui, t.label)).collect();
        let mut i = 0;
        while i < tools.len() {
            let mut end = i;
            let mut used = 0.0;
            while end < tools.len() {
                let w = widths[end] + if end > i { gap } else { 0.0 };
                if end > i && used + w > avail {
                    break;
                }
                used += w;
                end += 1;
            }
            let row = i..end;
            ui.horizontal(|ui| {
                for k in row {
                    let t = &tools[k];
                    let resp = match t.on {
                        Some(on) => theme::gated(ui, t.gate, |ui| {
                            theme::toggle_icon(ui, t.icon, t.label, on, t.hint)
                        }),
                        None => theme::tool_button(ui, t.icon, t.label, t.hint, t.gate),
                    };
                    if resp.clicked() {
                        clicked = Some(k);
                    }
                }
            });
            i = end;
        }
    });
    clicked
}

/// Stage half extents covering every point with a one-metre margin, inside
/// the stage handle's own 0.5..=20 m range. `None` when there is nothing on
/// the stage to fit to.
fn fit_extents(points: &[V3]) -> Option<(f32, f32)> {
    if points.is_empty() {
        return None;
    }
    let mut hw = 0.0_f32;
    let mut hd = 0.0_f32;
    for p in points {
        hw = hw.max(p.x.abs());
        hd = hd.max(p.z.abs());
    }
    Some(((hw + 1.0).clamp(0.5, 20.0), (hd + 1.0).clamp(0.5, 20.0)))
}

/// A hash of what the stage has picked, so tracking only moves the camera
/// when the selection actually changes.
fn selection_key(stage: &stage::StageView) -> u64 {
    let mut sel: Vec<usize> = stage.selection.iter().copied().collect();
    sel.sort_unstable();
    let mut h = DefaultHasher::new();
    sel.hash(&mut h);
    stage.sel_tower.hash(&mut h);
    stage.sel_truss.hash(&mut h);
    stage.sel_stage.hash(&mut h);
    h.finish()
}

impl App {
    pub(crate) fn inspector_stage_tab(&mut self, ui: &mut egui::Ui) {
        // A workspace View or a recalled bookmark can change the mode
        // behind the tab's back.
        if self.insp.prefs.fly_mode != self.stage.fly_mode {
            self.insp.prefs.fly_mode = self.stage.fly_mode;
            self.insp.mark_dirty();
        }
        self.camera_section(ui);
        self.saved_cameras_section(ui);
        self.show_section(ui);
        self.stage_size_section(ui);
        self.insp_fold(ui, "stage.options", "Options", false, None, |app, ui| {
            app.stage_options(ui);
        });
        self.insp_fold(ui, "stage.controls", "Stage controls", false, None, |_app, ui| {
            theme::help_table(ui, "insp_stage_keys", STAGE_HELP);
            ui.add_space(4.0);
            theme::help_table(ui, "insp_stage_camkeys", CAMERA_HELP);
        });
    }

    /// Mode, quick views, framing, the nudge row and the numbers readout.
    fn camera_section(&mut self, ui: &mut egui::Ui) {
        theme::section_with(ui, "Camera", |ui| {
            theme::help(
                ui,
                "Right-drag orbits, middle-drag pans, scroll zooms. Quick views re-frame the whole rig; hold Shift to keep your zoom. Over the stage: F frames the selection, Home the rig, 1–9 recall saved cameras.",
            );
        });

        let segs = [
            theme::Segment {
                icon: Some(Icon::Orbit),
                label: "Orbit",
                hint: "Orbit around a focal point: right-drag turns, middle-drag pans, scroll zooms.",
                badge: None,
            },
            theme::Segment {
                icon: Some(Icon::Fly),
                label: "Fly",
                hint: "Free flight: hover the stage and use WASD, Space, Shift and the arrow keys.",
                badge: None,
            },
        ];
        if let Some(i) = theme::segmented(ui, "insp_stage_mode", &segs, self.stage.fly_mode as usize)
        {
            self.stage.fly_mode = i == 1;
            self.stage.cam_tween = None;
            self.insp.prefs.fly_mode = self.stage.fly_mode;
            self.insp.mark_dirty();
        }
        if self.stage.fly_mode {
            ui.horizontal(|ui| {
                theme::label_dim(ui, "Speed");
                ui.add(
                    egui::DragValue::new(&mut self.stage.fly_speed)
                        .speed(0.1)
                        .range(0.5..=30.0)
                        .suffix(" m/s"),
                )
                .on_hover_text(
                    "Metres per second the fly camera moves at. Scrolling over the stage changes it too.",
                );
            });
            theme::hint(ui, "WASD move · Space up · ⇧ down · arrows look");
        } else {
            theme::hint(ui, "Right-drag orbits · wheel zooms");
        }
        ui.add_space(6.0);

        self.quick_view_pads(ui);
        ui.add_space(6.0);
        self.frame_toolbar(ui);
        self.nudge_toolbar(ui);
        self.camera_numbers(ui);
        ui.add_space(8.0);
    }

    /// The seven canonical angles as a pad grid.
    fn quick_view_pads(&mut self, ui: &mut egui::Ui) {
        let z = theme::zoom_of(ui);
        let avail = ui.available_width();
        let gap = 5.0 * z;
        let cols = theme::pad_columns(avail, 60.0 * z, gap);
        let w = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).max(24.0);
        let h = (w * 0.74).clamp(34.0 * z, 92.0 * z);
        let mut picked: Option<(QuickView, bool)> = None;
        // Explicit rows: a pad is a `ui.scope`, which `horizontal_wrapped`
        // cannot break (see `tool_rows`).
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
            for chunk in QuickView::ALL.chunks(cols) {
                ui.horizontal(|ui| {
                    for &q in chunk {
                        let current = self.stage.camera_is_quick(q);
                        let resp = theme::pad_button(
                            ui,
                            ("insp_stage_qv", q.label()),
                            egui::vec2(w, h),
                            theme::PadState {
                                fill: current.then_some(theme::ACCENT),
                                ..Default::default()
                            },
                            theme::PadFace::Icon(quick_icon(q)),
                            q.label(),
                        )
                        .on_hover_text(q.hint());
                        if resp.clicked() {
                            picked = Some((q, ui.input(|i| i.modifiers.shift)));
                        }
                    }
                });
            }
        });
        if let Some((q, shift)) = picked {
            let keep = shift != self.insp.prefs.stage.quick_view_keeps_zoom;
            let animate = self.insp.prefs.stage.animate_camera;
            self.stage.quick_view(q, &self.settings, keep, animate);
            self.log.push(format!("Camera → {}", q.label()));
        }
    }

    /// Frame · All · Look at · Previous · Reset · Snapshot.
    fn frame_toolbar(&mut self, ui: &mut egui::Ui) {
        let has_sel = !self.stage.selection.is_empty()
            || self.stage.sel_tower.is_some()
            || self.stage.sel_truss.is_some()
            || self.stage.sel_stage;
        let sel_gate = if has_sel {
            Ok(())
        } else {
            Err("Select lights, a tower, a truss or the stage box first.")
        };
        let back_gate = if self.stage.cam_history.is_empty() {
            Err("No previous camera yet — move the camera first.")
        } else {
            Ok(())
        };
        let tracking = self.insp.prefs.stage.camera_tracks_selection;
        let tools = [
            Tool::button(
                Icon::FrameSel,
                Some("Frame"),
                "Fit the selected lights, tower or truss in view (F over the stage).",
            )
            .gated(sel_gate),
            Tool::button(
                Icon::FrameAll,
                Some("All"),
                "Fit the whole rig and the stage box in view (Home over the stage).",
            ),
            Tool::toggle(
                Icon::Target,
                None,
                tracking,
                "Look at the selection without changing the zoom (⇧F over the stage). Lit while the camera tracks the selection.",
            )
            .gated(sel_gate),
            Tool::button(Icon::Undo, None, "Go back to the previous camera position.")
                .gated(back_gate),
            Tool::button(Icon::Reset, Some("Reset"), "Return to the default camera."),
            Tool::button(
                Icon::Camera,
                Some("Snapshot"),
                "Save a PNG of the stage view into the screenshots folder.",
            ),
        ];
        let act = tool_rows(ui, &tools).map(|i| match i {
            0 => FrameAction::Selection,
            1 => FrameAction::All,
            2 => FrameAction::LookAt,
            3 => FrameAction::Previous,
            4 => FrameAction::Reset,
            _ => FrameAction::Snapshot,
        });
        let animate = self.insp.prefs.stage.animate_camera;
        match act {
            Some(FrameAction::Selection) => self.frame_or_rig(animate),
            Some(FrameAction::All) => {
                self.stage.frame_all(&self.settings, animate);
                self.log.push("Framed the whole rig".into());
            }
            Some(FrameAction::LookAt) => {
                if self.stage.look_at_selection(animate) {
                    self.log.push("Camera → the selection".into());
                }
            }
            Some(FrameAction::Previous) => {
                if self.stage.previous_camera(animate) {
                    self.log.push("Camera → previous".into());
                }
            }
            Some(FrameAction::Reset) => {
                self.stage.reset_camera(animate);
                self.log.push("Camera reset".into());
            }
            Some(FrameAction::Snapshot) => {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot);
                self.insp.stage.snapshot_pending = true;
            }
            None => {}
        }
    }

    /// Fit the selection, or the whole rig when nothing is picked.
    fn frame_or_rig(&mut self, animate: bool) {
        let msg = self.framed_log();
        if self.stage.frame_selection(&self.settings, animate) {
            self.log.push(msg);
        } else {
            self.stage.frame_all(&self.settings, animate);
            self.log.push("Framed the whole rig".into());
        }
    }

    /// The sentence "Frame" writes to the log for what is picked now.
    fn framed_log(&self) -> String {
        if !self.stage.selection.is_empty() {
            let n = self.stage.selected_fixtures().len();
            format!("Framed {n} light{}", if n == 1 { "" } else { "s" })
        } else if let Some(i) = self.stage.sel_tower {
            format!("Framed tower {}", i + 1)
        } else if let Some(i) = self.stage.sel_truss {
            format!("Framed truss {}", i + 1)
        } else {
            "Framed the stage box".into()
        }
    }

    /// Turn and zoom by the button, for people who would rather not drag.
    fn nudge_toolbar(&mut self, ui: &mut egui::Ui) {
        let tools = [
            Tool::button(Icon::ChevronLeft, None, "Orbit 15° to the left."),
            Tool::button(Icon::ChevronRight, None, "Orbit 15° to the right."),
            Tool::button(Icon::ChevronUp, None, "Tilt the camera 10° higher."),
            Tool::button(Icon::ChevronDown, None, "Tilt the camera 10° lower."),
            Tool::button(Icon::ZoomOut, None, "Zoom out (distance × 1.25)."),
            Tool::button(Icon::ZoomIn, None, "Zoom in (distance × 0.8)."),
        ];
        let turn = 15.0_f32.to_radians();
        let tilt = 10.0_f32.to_radians();
        match tool_rows(ui, &tools) {
            Some(0) => self.stage.orbit_by(-turn, 0.0),
            Some(1) => self.stage.orbit_by(turn, 0.0),
            Some(2) => self.stage.orbit_by(0.0, tilt),
            Some(3) => self.stage.orbit_by(0.0, -tilt),
            Some(4) => self.stage.zoom_by(1.25),
            Some(5) => self.stage.zoom_by(0.8),
            _ => {}
        }
    }

    /// The pose as a chip, and the exact numbers behind it.
    fn camera_numbers(&mut self, ui: &mut egui::Ui) {
        let open = self.insp.prefs.stage.show_camera_numbers;
        let txt = format!(
            "yaw {:.0}° · pitch {:.0}° · {:.1} m",
            self.stage.cam.yaw.to_degrees().rem_euclid(360.0),
            self.stage.cam.pitch.to_degrees(),
            self.stage.cam.dist,
        );
        let mut toggled = false;
        ui.horizontal_wrapped(|ui| {
            if theme::chip(ui, &txt, theme::TEXT_DIM, open)
                .on_hover_text("Where the camera is. Click for exact numbers you can type.")
                .clicked()
            {
                toggled = true;
            }
        });
        if toggled {
            self.insp.prefs.stage.show_camera_numbers = !open;
            self.insp.mark_dirty();
        }
        if !open {
            return;
        }
        let mut yaw = self.stage.cam.yaw.to_degrees().rem_euclid(360.0);
        let mut pitch = self.stage.cam.pitch.to_degrees();
        let mut dist = self.stage.cam.dist;
        let mut fov = self.stage.cam.fov_y.to_degrees();
        let mut target = self.stage.cam.target;
        let mut changed = false;
        let mut began = false;
        theme::kv_grid(ui, "insp_stage_cam", |ui| {
            let mut row = |ui: &mut egui::Ui, label: &str, drag: egui::DragValue, hint: &str| {
                theme::label_dim(ui, label);
                let r = ui.add(drag).on_hover_text(hint);
                changed |= r.changed();
                began |= r.drag_started() || r.gained_focus();
                ui.end_row();
            };
            row(
                ui,
                "Yaw °",
                egui::DragValue::new(&mut yaw).speed(0.5).range(0.0..=360.0).max_decimals(1),
                "Which way the camera looks around the stage, in degrees.",
            );
            row(
                ui,
                "Pitch °",
                egui::DragValue::new(&mut pitch).speed(0.5).range(-8.5..=85.9).max_decimals(1),
                "How far above the horizon the camera sits, in degrees.",
            );
            row(
                ui,
                "Distance",
                egui::DragValue::new(&mut dist).speed(0.1).range(3.0..=80.0).max_decimals(1).suffix(" m"),
                "How far the camera is from what it is looking at.",
            );
            row(
                ui,
                "FOV °",
                egui::DragValue::new(&mut fov).speed(0.5).range(25.0..=100.0).max_decimals(1),
                "Lens angle: small is a long lens, large is wide.",
            );
            row(
                ui,
                "Target X",
                egui::DragValue::new(&mut target.x).speed(0.05).max_decimals(2).suffix(" m"),
                "Where the camera is aimed, across the stage.",
            );
            row(
                ui,
                "Target Y",
                egui::DragValue::new(&mut target.y).speed(0.05).max_decimals(2).suffix(" m"),
                "Where the camera is aimed, above the floor.",
            );
            row(
                ui,
                "Target Z",
                egui::DragValue::new(&mut target.z).speed(0.05).max_decimals(2).suffix(" m"),
                "Where the camera is aimed, front to back.",
            );
        });
        if changed {
            if began {
                self.stage.remember_camera();
            }
            self.stage.cam.yaw = yaw.to_radians();
            self.stage.cam.pitch = pitch.to_radians();
            self.stage.cam.dist = dist;
            self.stage.cam.fov_y = fov.to_radians();
            self.stage.cam.target = target;
            self.stage.cam_tween = None;
        }
    }

    /// The saved-camera list: save row, rows, context menu, tour toggle.
    fn saved_cameras_section(&mut self, ui: &mut egui::Ui) {
        let n = self.cameras.len();
        let touring = self.insp.stage.tour.is_some();
        theme::section_with(ui, "Saved cameras", |ui| {
            theme::count_pill(ui, n, "camera");
        });

        // Save row.
        let z = theme::zoom_of(ui);
        let mut save_now = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().button_padding = egui::vec2(6.0 * z, 4.0 * z);
            // Right to left, so the field asks for exactly what the button
            // leaves and the panel is never dragged wider.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::tool_button(
                    ui,
                    Icon::Bookmark,
                    Some("Save"),
                    "Save the current camera under this name (blank = numbered).",
                    Ok(()),
                )
                .clicked()
                {
                    save_now = true;
                }
                let w = (ui.available_width() - 6.0 * z).max(40.0);
                let field = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.insp.stage.camera_name)
                            .hint_text("Camera name…")
                            .desired_width(w),
                    )
                    .on_hover_text("A name for the camera you are about to save. Enter saves it.");
                if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    save_now = true;
                }
            });
        });
        if save_now {
            let name = std::mem::take(&mut self.insp.stage.camera_name);
            self.save_camera_bookmark(Some(name), None);
        }
        let tour_gate = if n >= 2 { Ok(()) } else { Err("Save at least two cameras first.") };
        let tools = [Tool::toggle(
            Icon::Tour,
            Some("Tour"),
            touring,
            "Cycle through the saved cameras every few seconds. Any camera drag stops it.",
        )
        .gated(tour_gate)];
        if tool_rows(ui, &tools).is_some() {
            if touring {
                self.stop_tour();
            } else {
                let now = ui.input(|i| i.time);
                self.start_tour(now);
            }
        }

        if self.cameras.is_empty() {
            theme::empty_state(
                ui,
                Icon::Camera,
                "No saved cameras",
                "Frame a view, then press Save.",
            );
            ui.add_space(8.0);
            return;
        }

        // Everything a row shows, read before anything is drawn.
        let current = self.cameras.iter().position(|b| self.stage.camera_is(&b.camera));
        let tour_next = self.insp.stage.tour.map(|t| t.next);
        let rows: Vec<(String, String, Option<Color32>, Option<[u8; 3]>, Option<u8>)> = self
            .cameras
            .iter()
            .map(|b| {
                let mut parts: Vec<String> = Vec::new();
                if let Some(k) = b.hotkey {
                    parts.push(format!("key {k}"));
                }
                if b.fly_mode {
                    parts.push("fly".into());
                }
                if parts.is_empty() {
                    parts.push(format!(
                        "yaw {:.0}° · {:.1} m",
                        b.camera.yaw.to_degrees().rem_euclid(360.0),
                        b.camera.dist
                    ));
                }
                (
                    b.name.clone(),
                    parts.join(" · "),
                    b.color.map(|[r, g, bl]| Color32::from_rgb(r, g, bl)),
                    b.color,
                    b.hotkey,
                )
            })
            .collect();

        let mut rename = self.insp.stage.rename.take();
        let mut want_focus = self.insp.stage.rename_focus;
        let renaming = rename.as_ref().map(|(i, _)| *i);
        let mut act: Option<CamAction> = None;
        let n = rows.len();
        for (i, (name, sub, swatch, rgb, hotkey)) in rows.iter().enumerate() {
            if renaming == Some(i) {
                let draft = rename.as_mut().map(|(_, d)| d).expect("a draft while renaming");
                let resp = ui.push_id(("insp_stage_rn", i), |ui| {
                    ui.add(
                        egui::TextEdit::singleline(draft)
                            .desired_width(f32::INFINITY)
                            .hint_text("New name…"),
                    )
                    .on_hover_text("Type a new name, then press Enter. Escape keeps the old one.")
                })
                .inner;
                if want_focus {
                    resp.request_focus();
                    want_focus = false;
                }
                let (enter, escape) =
                    ui.input(|inp| (inp.key_pressed(Key::Enter), inp.key_pressed(Key::Escape)));
                if escape {
                    act = Some(CamAction::RenameCancel);
                } else if resp.lost_focus() {
                    act = if enter {
                        Some(CamAction::RenameCommit(i, draft.clone()))
                    } else {
                        Some(CamAction::RenameCancel)
                    };
                }
                continue;
            }
            let row = theme::list_row(
                ui,
                ("insp", "stage", i),
                &theme::Row {
                    swatch: *swatch,
                    icon: swatch.is_none().then_some(Icon::Camera),
                    title: name,
                    subtitle: Some(sub),
                    badges: &[],
                    selected: current == Some(i),
                    active: tour_next == Some(i),
                    indent: 0.0,
                },
            )
            .on_hover_text(
                "Click to recall this camera. Right-click to update, rename, give it a key or a colour, reorder or delete.",
            );
            if row.clicked() {
                act = Some(CamAction::Recall(i));
            }
            row.context_menu(|ui| {
                if ui
                    .button("Update to current camera")
                    .on_hover_text("Point this bookmark at wherever the camera is now.")
                    .clicked()
                {
                    act = Some(CamAction::Update(i));
                    ui.close_menu();
                }
                if ui
                    .button("Rename")
                    .on_hover_text("Give this camera a different name.")
                    .clicked()
                {
                    act = Some(CamAction::RenameStart(i));
                    ui.close_menu();
                }
                ui.menu_button("Hotkey", |ui| {
                    if ui
                        .selectable_label(hotkey.is_none(), "None")
                        .on_hover_text("No digit key recalls this camera.")
                        .clicked()
                    {
                        act = Some(CamAction::Hotkey(i, None));
                        ui.close_menu();
                    }
                    for k in 1..=9u8 {
                        if ui
                            .selectable_label(*hotkey == Some(k), k.to_string())
                            .on_hover_text("Press this digit over the stage to recall the camera.")
                            .clicked()
                        {
                            act = Some(CamAction::Hotkey(i, Some(k)));
                            ui.close_menu();
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Colour");
                    let mut picked = rgb.unwrap_or([0x82, 0xDB, 0xD8]);
                    if ui
                        .color_edit_button_srgb(&mut picked)
                        .on_hover_text("Tint this row so it stands out in the list.")
                        .changed()
                    {
                        act = Some(CamAction::Color(i, Some(picked)));
                    }
                    if ui
                        .small_button("Clear")
                        .on_hover_text("Go back to the plain camera icon.")
                        .clicked()
                    {
                        act = Some(CamAction::Color(i, None));
                        ui.close_menu();
                    }
                });
                ui.separator();
                if ui
                    .add_enabled(i > 0, egui::Button::new("Move up"))
                    .on_hover_text("Swap this camera with the one above it.")
                    .clicked()
                {
                    act = Some(CamAction::Move(i, -1));
                    ui.close_menu();
                }
                if ui
                    .add_enabled(i + 1 < n, egui::Button::new("Move down"))
                    .on_hover_text("Swap this camera with the one below it.")
                    .clicked()
                {
                    act = Some(CamAction::Move(i, 1));
                    ui.close_menu();
                }
                ui.separator();
                if theme::danger_button(ui, "Delete")
                    .on_hover_text("Forget this saved camera. The show file records it.")
                    .clicked()
                {
                    act = Some(CamAction::Delete(i));
                    ui.close_menu();
                }
            });
        }
        self.insp.stage.rename = rename;
        self.insp.stage.rename_focus = want_focus;
        if let Some(act) = act {
            self.apply_cam_action(act);
        }
        ui.add_space(8.0);
    }

    /// Apply one deferred bookmark action, bounds-checked against the list
    /// as it is now.
    fn apply_cam_action(&mut self, act: CamAction) {
        let n = self.cameras.len();
        match act {
            CamAction::Recall(i) => {
                self.recall_camera(i);
            }
            CamAction::Update(i) => {
                if i < n {
                    let camera = self.stage.cam.snapshot();
                    let fly_mode = self.stage.fly_mode;
                    let b = &mut self.cameras[i];
                    b.camera = camera;
                    b.fly_mode = fly_mode;
                    let name = b.name.clone();
                    stage::save_cameras(&self.cameras);
                    self.log.push(format!("Camera \"{name}\" updated"));
                }
            }
            CamAction::RenameStart(i) => {
                if i < n {
                    self.insp.stage.rename = Some((i, self.cameras[i].name.clone()));
                    self.insp.stage.rename_focus = true;
                }
            }
            CamAction::RenameCommit(i, text) => {
                self.insp.stage.rename = None;
                self.insp.stage.rename_focus = false;
                let text = text.trim().to_owned();
                if i < n && !text.is_empty() && text != self.cameras[i].name {
                    let old = std::mem::replace(&mut self.cameras[i].name, text.clone());
                    stage::save_cameras(&self.cameras);
                    self.log.push(format!("Camera \"{old}\" renamed \"{text}\""));
                }
            }
            CamAction::RenameCancel => {
                self.insp.stage.rename = None;
                self.insp.stage.rename_focus = false;
            }
            CamAction::Hotkey(i, key) => {
                if i < n {
                    stage::assign_hotkey(&mut self.cameras, i, key);
                    stage::save_cameras(&self.cameras);
                    let name = self.cameras[i].name.clone();
                    match key {
                        Some(k) => self.log.push(format!("Camera \"{name}\" → key {k}")),
                        None => self.log.push(format!("Camera \"{name}\" lost its key")),
                    }
                }
            }
            CamAction::Color(i, color) => {
                if i < n {
                    self.cameras[i].color = color;
                    stage::save_cameras(&self.cameras);
                }
            }
            CamAction::Move(i, delta) => {
                let j = i as isize + delta;
                if i < n && j >= 0 && (j as usize) < n {
                    let j = j as usize;
                    self.cameras.swap(i, j);
                    if let Some((r, _)) = self.insp.stage.rename.as_mut() {
                        if *r == i {
                            *r = j;
                        } else if *r == j {
                            *r = i;
                        }
                    }
                    stage::save_cameras(&self.cameras);
                }
            }
            CamAction::Delete(i) => {
                if i < n {
                    let gone = self.cameras.remove(i);
                    if self.insp.stage.rename.as_ref().is_some_and(|(r, _)| *r >= i) {
                        self.insp.stage.rename = None;
                        self.insp.stage.rename_focus = false;
                    }
                    if self.cameras.len() < 2 {
                        self.stop_tour();
                    }
                    stage::save_cameras(&self.cameras);
                    self.log.push(format!("Deleted camera \"{}\"", gone.name));
                }
            }
        }
    }

    /// Go to bookmark `i`, keeping `prefs.fly_mode` in step with the mode
    /// the bookmark was saved in. False when the index no longer exists.
    fn goto_bookmark(&mut self, i: usize, animate: bool) -> bool {
        let Some(b) = self.cameras.get(i).cloned() else {
            return false;
        };
        self.stage.recall_bookmark(&b, animate);
        if self.insp.prefs.fly_mode != self.stage.fly_mode {
            self.insp.prefs.fly_mode = self.stage.fly_mode;
            self.insp.mark_dirty();
        }
        true
    }

    /// Recall bookmark `i` and say so. False when it no longer exists.
    pub(crate) fn recall_camera(&mut self, i: usize) -> bool {
        let animate = self.insp.prefs.stage.animate_camera;
        let name = match self.cameras.get(i) {
            Some(b) => b.name.clone(),
            None => return false,
        };
        if !self.goto_bookmark(i, animate) {
            return false;
        }
        self.log.push(format!("Camera → \"{name}\""));
        true
    }

    /// Save the current camera as a new bookmark and return its index.
    /// A blank name is numbered, a duplicate name gets a counter.
    pub(crate) fn save_camera_bookmark(
        &mut self,
        name: Option<String>,
        hotkey: Option<u8>,
    ) -> usize {
        let base = name.unwrap_or_default();
        let name = stage::unique_name(&self.cameras, &base);
        let mut b = self.stage.bookmark(name.clone());
        b.hotkey = hotkey;
        self.cameras.push(b);
        let i = self.cameras.len() - 1;
        if let Some(k) = hotkey {
            stage::assign_hotkey(&mut self.cameras, i, Some(k));
        }
        stage::save_cameras(&self.cameras);
        match hotkey {
            Some(k) => self.log.push(format!("Saved camera \"{name}\" (key {k})")),
            None => self.log.push(format!("Saved camera \"{name}\"")),
        }
        i
    }

    /// The display switches, their three presets and the two look settings.
    fn show_section(&mut self, ui: &mut egui::Ui) {
        theme::section_with(ui, "Show", |ui| {
            theme::help(
                ui,
                "Switches are this machine only — never saved with the show or undone. Beam haze and body size are show settings.",
            );
        });
        let mut d = self.insp.prefs.display;
        let mut preset: Option<DisplayPreset> = None;
        ui.horizontal_wrapped(|ui| {
            for p in DisplayPreset::ALL {
                if theme::chip(ui, p.label(), theme::ACCENT_SOFT, d == p.toggles())
                    .on_hover_text(p.hint())
                    .clicked()
                {
                    preset = Some(p);
                }
            }
        });
        ui.add_space(4.0);
        let shift = ui.input(|i| i.modifiers.shift);
        let tools = [
            Tool::toggle(Icon::Grid, None, d.grid, "Show the floor grid."),
            Tool::toggle(Icon::Stage, None, d.stage_box, "Show the stage riser box."),
            Tool::toggle(Icon::Beam, None, d.beams, "Draw the beam haze (GPU)."),
            Tool::toggle(
                Icon::Pool,
                None,
                d.pools,
                "Draw the floor and stage light pools (GPU).",
            ),
            Tool::toggle(
                Icon::Label,
                d.label_all.then_some("all"),
                d.labels,
                "Name tags: off → selected lights → every light. Shift-click cycles backwards.",
            ),
            Tool::toggle(Icon::Tower, None, d.towers, "Show the towers."),
            Tool::toggle(Icon::TrussStraight, None, d.trusses, "Show the trusses."),
            Tool::toggle(
                Icon::SlotRing,
                None,
                d.slot_rings,
                "Show the blue slot rings on a picked tower or truss and while dragging a light.",
            ),
            Tool::toggle(Icon::Gizmo, None, d.gizmo, "Show the transform gizmo on the selection."),
            Tool::toggle(Icon::Tick, None, d.ticks, "Show the aim ticks on dark lights."),
            Tool::toggle(
                Icon::Overlay,
                None,
                d.overlays,
                "Show the transition sphere, chase band and phaser traces.",
            ),
            Tool::toggle(Icon::Help, None, d.help, "Show the shortcut help in the stage corner."),
        ];
        match tool_rows(ui, &tools) {
            Some(0) => d.grid = !d.grid,
            Some(1) => d.stage_box = !d.stage_box,
            Some(2) => d.beams = !d.beams,
            Some(3) => d.pools = !d.pools,
            Some(4) => {
                // Off → selected → all, and back again with Shift.
                let (labels, all) = match (d.labels, d.label_all, shift) {
                    (false, _, false) => (true, false),
                    (true, false, false) => (true, true),
                    (true, true, false) => (false, false),
                    (false, _, true) => (true, true),
                    (true, true, true) => (true, false),
                    (true, false, true) => (false, false),
                };
                d.labels = labels;
                d.label_all = all;
            }
            Some(5) => d.towers = !d.towers,
            Some(6) => d.trusses = !d.trusses,
            Some(7) => d.slot_rings = !d.slot_rings,
            Some(8) => d.gizmo = !d.gizmo,
            Some(9) => d.ticks = !d.ticks,
            Some(10) => d.overlays = !d.overlays,
            Some(_) => d.help = !d.help,
            None => {}
        }
        ui.add_space(4.0);

        let mut settings_changed = false;
        theme::kv_grid(ui, "insp_stage_look", |ui| {
            theme::label_dim(ui, "Grid pitch");
            egui::ComboBox::from_id_salt("insp_stage_grid_pitch")
                .width(ui.available_width().min(74.0))
                .selected_text(d.grid_pitch.label())
                .show_ui(ui, |ui| {
                    for g in GridPitch::ALL {
                        ui.selectable_value(&mut d.grid_pitch, g, g.label());
                    }
                })
                .response
                .on_hover_text("Spacing of the floor grid lines — a ruler for eyeballing distances.");
            ui.end_row();

            theme::label_dim(ui, "Body size");
            settings_changed |= ui
                .add(
                    egui::DragValue::new(&mut self.settings.light_scale)
                        .speed(0.02)
                        .range(0.2..=5.0),
                )
                .on_hover_text("How large the fixture bodies are drawn. A show setting.")
                .changed();
            ui.end_row();

            theme::label_dim(ui, "Beam haze");
            settings_changed |= ui
                .add(
                    egui::DragValue::new(&mut self.settings.beam_opacity)
                        .speed(0.02)
                        .range(0.0..=3.0)
                        .fixed_decimals(2),
                )
                .on_hover_text("How much air the beams appear to catch. A show setting.")
                .changed();
            ui.end_row();
        });
        // The rail gets a full-width row of its own: a Slider with its value
        // box beside it is wider than a grid cell can hold at 230 px.
        settings_changed |= ui
            .add_sized(
                [ui.available_width(), 18.0],
                egui::Slider::new(&mut self.settings.beam_opacity, 0.0..=3.0).show_value(false),
            )
            .on_hover_text("Drag to change how much air the beams appear to catch.")
            .changed();
        theme::hint(ui, "Haze and body size are show settings (undoable).");

        if let Some(p) = preset {
            d = p.toggles();
            self.log.push(format!("Stage view: {}", p.label()));
        }
        if d != self.insp.prefs.display {
            self.set_display(d);
        }
        if settings_changed {
            self.settings.save();
        }
        ui.add_space(8.0);
    }

    /// Write the display switches through to the stage the same frame.
    fn set_display(&mut self, d: DisplayToggles) {
        if d != self.insp.prefs.display {
            self.insp.prefs.display = d;
            self.stage.show = d;
            self.insp.mark_dirty();
        }
    }

    /// Stage footprint: numbers, standard sizes, fit-to-rig and select.
    fn stage_size_section(&mut self, ui: &mut egui::Ui) {
        let sel_stage = self.stage.sel_stage;
        theme::section_with(ui, "Stage", |ui| {
            if sel_stage {
                theme::pill(ui, "handles shown", theme::ACCENT_SOFT);
            }
        });
        let mut w = self.settings.stage_half_w * 2.0;
        let mut depth = self.settings.stage_half_d * 2.0;
        let mut high = self.settings.stage_h;
        let mut changed = false;
        theme::kv_grid(ui, "insp_stage_size", |ui| {
            theme::label_dim(ui, "Width");
            changed |= ui
                .add(egui::DragValue::new(&mut w).speed(0.1).range(1.0..=40.0).suffix(" m"))
                .on_hover_text("How wide the stage box is, left to right.")
                .changed();
            ui.end_row();
            theme::label_dim(ui, "Depth");
            changed |= ui
                .add(egui::DragValue::new(&mut depth).speed(0.1).range(1.0..=40.0).suffix(" m"))
                .on_hover_text("How deep the stage box is, front to back.")
                .changed();
            ui.end_row();
            theme::label_dim(ui, "Height");
            changed |= ui
                .add(egui::DragValue::new(&mut high).speed(0.05).range(0.0..=5.0).suffix(" m"))
                .on_hover_text("How high the stage riser stands off the floor.")
                .changed();
            ui.end_row();
        });
        if changed {
            self.settings.stage_half_w = w * 0.5;
            self.settings.stage_half_d = depth * 0.5;
            self.settings.stage_h = high;
            self.settings.save();
        }

        let mut pick: Option<(f32, f32)> = None;
        ui.horizontal_wrapped(|ui| {
            for (sw, sd) in QUICK_SIZES {
                let active = (self.settings.stage_half_w - sw * 0.5).abs() < 0.01
                    && (self.settings.stage_half_d - sd * 0.5).abs() < 0.01;
                if theme::chip(ui, &format!("{sw:.0}×{sd:.0}"), theme::TEXT_DIM, active)
                    .on_hover_text(format!(
                        "Set the stage to {sw:.0} by {sd:.0} metres (height unchanged)."
                    ))
                    .clicked()
                {
                    pick = Some((sw, sd));
                }
            }
        });
        if let Some((sw, sd)) = pick {
            self.set_stage_size(sw, sd);
        }
        ui.add_space(4.0);

        let fit_gate = if self.stage.rig_extent_points().is_empty() {
            Err("Nothing on the stage to fit to.")
        } else {
            Ok(())
        };
        let tools = [
            Tool::button(
                Icon::Ruler,
                Some("Fit rig"),
                "Size the stage to the lights, towers and trusses with a one-metre margin.",
            )
            .gated(fit_gate),
            Tool::button(
                Icon::Stage,
                Some("Select"),
                "Select the stage box so its resize arrows appear on the stage.",
            ),
        ];
        match tool_rows(ui, &tools) {
            Some(0) => self.fit_stage_to_rig(),
            Some(_) => {
                self.stage.clear_selection();
                self.stage.sel_tower = None;
                self.stage.sel_truss = None;
                self.stage.sel_stage = true;
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
            }
            None => {}
        }
        theme::hint(ui, "Select the stage box and drag its gold arrows to resize by eye.");
        ui.add_space(8.0);
    }

    /// Set the stage footprint in metres (height untouched).
    fn set_stage_size(&mut self, w: f32, d: f32) {
        self.settings.stage_half_w = (w * 0.5).clamp(0.5, 20.0);
        self.settings.stage_half_d = (d * 0.5).clamp(0.5, 20.0);
        self.settings.save();
        self.log.push(format!("Stage {w:.0} × {d:.0} m"));
    }

    /// Grow or shrink the stage box to whatever is standing on it.
    fn fit_stage_to_rig(&mut self) {
        let Some((hw, hd)) = fit_extents(&self.stage.rig_extent_points()) else {
            return;
        };
        self.settings.stage_half_w = hw;
        self.settings.stage_half_d = hd;
        self.settings.save();
        self.log.push(format!(
            "Stage fitted to the rig: {:.1} × {:.1} m",
            hw * 2.0,
            hd * 2.0
        ));
    }

    /// The Options fold: how the camera behaves.
    fn stage_options(&mut self, ui: &mut egui::Ui) {
        let mut p = self.insp.prefs.stage.clone();
        ui.checkbox(&mut p.animate_camera, "Animate camera moves")
            .on_hover_text("Glide over a third of a second instead of cutting.");
        ui.checkbox(&mut p.camera_tracks_selection, "Camera tracks the selection")
            .on_hover_text("Re-centre the orbit on whatever you select.");
        ui.checkbox(&mut p.quick_view_keeps_zoom, "Quick views keep zoom")
            .on_hover_text("Quick views only turn the camera; Shift-click re-frames instead.");
        ui.checkbox(&mut p.restore_camera, "Restore the camera on launch")
            .on_hover_text("Open the app looking where you left off.");
        ui.horizontal(|ui| {
            theme::label_dim(ui, "Tour every");
            ui.add(
                egui::DragValue::new(&mut p.tour_seconds)
                    .speed(0.5)
                    .range(2.0..=60.0)
                    .suffix(" s"),
            )
            .on_hover_text("How long the camera tour rests on each saved camera.");
        });
        if p != self.insp.prefs.stage {
            self.insp.prefs.stage = p;
            self.insp.mark_dirty();
        }
    }

    fn start_tour(&mut self, now: f64) {
        if self.cameras.len() < 2 {
            return;
        }
        self.insp.stage.tour = Some(Tour { next: 0, at: now });
        self.log.push(format!(
            "Camera tour started ({} cameras, every {:.0} s)",
            self.cameras.len(),
            self.insp.prefs.stage.tour_seconds
        ));
    }

    fn stop_tour(&mut self) {
        if self.insp.stage.tour.take().is_some() {
            self.log.push("Camera tour stopped".into());
        }
    }

    /// Run after `central_panel`, so `stage.hovered` is this frame's: the
    /// camera keyboard, selection tracking, the tour and the PNG drain.
    pub(crate) fn stage_camera_tick(&mut self, ctx: &egui::Context) {
        let animate = self.insp.prefs.stage.animate_camera;
        let hot = self.stage.hovered && !ctx.wants_keyboard_input();
        let mut moved = false;

        if hot {
            let (frame_key, shift, home, digit) = ctx.input(|i| {
                (
                    i.key_pressed(Key::F) && !i.modifiers.command,
                    i.modifiers.shift,
                    i.key_pressed(Key::Home),
                    DIGITS.iter().position(|k| i.key_pressed(*k)).map(|p| p as u8 + 1),
                )
            });
            if frame_key {
                if shift {
                    if self.stage.look_at_selection(animate) {
                        self.log.push("Camera → the selection".into());
                    }
                } else {
                    self.frame_or_rig(animate);
                }
                moved = true;
            }
            if home {
                self.stage.frame_all(&self.settings, animate);
                self.log.push("Framed the whole rig".into());
                moved = true;
            }
            if let Some(n) = digit {
                if shift {
                    match stage::bookmark_by_hotkey(&self.cameras, n) {
                        Some(i) => {
                            let camera = self.stage.cam.snapshot();
                            let fly_mode = self.stage.fly_mode;
                            let b = &mut self.cameras[i];
                            b.camera = camera;
                            b.fly_mode = fly_mode;
                            let name = b.name.clone();
                            stage::save_cameras(&self.cameras);
                            self.log.push(format!("Camera \"{name}\" updated (key {n})"));
                        }
                        None => {
                            self.save_camera_bookmark(Some(format!("Cam {n}")), Some(n));
                        }
                    }
                } else if let Some(i) = stage::bookmark_by_hotkey(&self.cameras, n) {
                    moved |= self.recall_camera(i);
                }
            }
        }

        // Tracking: re-centre on each new selection, never mid-drag.
        if self.insp.prefs.stage.camera_tracks_selection && !self.stage.is_dragging() {
            let key = selection_key(&self.stage);
            if key != self.insp.stage.last_track_key {
                self.insp.stage.last_track_key = key;
                moved |= self.stage.look_at_selection(animate);
            }
        }

        // The tour: one bookmark every `tour_seconds`, quietly.
        if let Some(mut t) = self.insp.stage.tour {
            if self.cameras.len() < 2 {
                self.stop_tour();
            } else {
                let now = ctx.input(|i| i.time);
                if now >= t.at {
                    let i = t.next.min(self.cameras.len() - 1);
                    self.goto_bookmark(i, animate);
                    t.next = (i + 1) % self.cameras.len();
                    t.at = now + self.insp.prefs.stage.tour_seconds.max(2.0) as f64;
                    self.insp.stage.tour = Some(t);
                    moved = true;
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }

        if self.insp.stage.snapshot_pending {
            self.drain_stage_snapshot(ctx);
        }

        // Any camera gesture of the operator's own ends the tour.
        if std::mem::take(&mut self.stage.user_moved) {
            self.stop_tour();
        }
        if moved {
            ctx.request_repaint();
        }
    }

    /// Catch the screenshot reply and write the stage's slice of it.
    fn drain_stage_snapshot(&mut self, ctx: &egui::Context) {
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = shot else {
            return;
        };
        self.insp.stage.snapshot_pending = false;
        let ppp = ctx.pixels_per_point().max(0.1);
        let full = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(image.width() as f32 / ppp, image.height() as f32 / ppp),
        );
        let crop = self.stage.last_rect.intersect(full);
        if crop.width() < 2.0 || crop.height() < 2.0 {
            self.log.push("Stage snapshot failed: the stage was not on screen".into());
            return;
        }
        let region = image.region(&crop, Some(ppp));
        let raw: Vec<u8> = region.pixels.iter().flat_map(|c| c.to_array()).collect();
        let (w, h) = (region.width() as u32, region.height() as u32);
        let Some(img) = image::RgbaImage::from_raw(w, h, raw) else {
            self.log.push("Stage snapshot failed: the pixels did not fit".into());
            return;
        };
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let dir = crate::paths::data_path(SNAPSHOTS_DIR);
        let path = dir.join(format!("stage-{secs}.png"));
        let _ = std::fs::create_dir_all(&dir);
        match img.save(&path) {
            Ok(()) => self.log.push(format!("Stage snapshot saved to {}", path.display())),
            Err(e) => self.log.push(format!("Stage snapshot failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{Frame, DMX_SLOTS};
    use crate::stage::headless::{render_frames, save};
    use crate::ui::inspector::{InspectorPrefs, InspectorTab};

    fn lit(pixels: &[u8]) -> usize {
        pixels.chunks(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120).count()
    }

    #[test]
    fn display_presets_do_what_the_labels_say() {
        assert_eq!(DisplayPreset::All.toggles(), DisplayToggles::default());
        let clean = DisplayPreset::Clean.toggles();
        assert!(!clean.grid && !clean.slot_rings && !clean.gizmo);
        assert!(!clean.ticks && !clean.help && !clean.overlays && !clean.labels);
        assert!(clean.beams && clean.pools && clean.towers && clean.trusses && clean.stage_box);
        let plan = DisplayPreset::Plan.toggles();
        assert!(!plan.beams && !plan.pools && !plan.help && !plan.overlays);
        assert!(plan.labels && plan.label_all && plan.grid);
        // Three distinct looks, so the chips never light together.
        assert_ne!(clean, plan);
        assert_ne!(clean, DisplayPreset::All.toggles());
        assert_ne!(plan, DisplayPreset::All.toggles());
        for p in DisplayPreset::ALL {
            assert!(p.hint().ends_with('.'), "{} has no full-sentence hint", p.label());
        }
    }

    #[test]
    fn quick_stage_sizes_are_within_the_handle_clamps() {
        for (w, d) in QUICK_SIZES {
            assert!((0.5..=20.0).contains(&(w * 0.5)), "{w} m is outside the handle range");
            assert!((0.5..=20.0).contains(&(d * 0.5)), "{d} m is outside the handle range");
        }
    }

    #[test]
    fn fit_extents_covers_every_light_with_a_margin() {
        let pts = [
            crate::stage::v3(-3.0, 2.0, -1.0),
            crate::stage::v3(0.0, 2.0, 2.0),
            crate::stage::v3(5.0, 2.0, 2.0),
        ];
        assert_eq!(fit_extents(&pts), Some((6.0, 3.0)));
        assert_eq!(fit_extents(&[]), None);
        let far = [crate::stage::v3(40.0, 0.0, 0.0)];
        assert_eq!(fit_extents(&far), Some((20.0, 1.0)));
    }

    #[test]
    fn stage_prefs_default_and_old_files_load() {
        let d: StagePrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(d, StagePrefs::default());
        assert!(d.animate_camera && d.restore_camera);
        assert!(!d.camera_tracks_selection && !d.quick_view_keeps_zoom);
        assert_eq!(d.tour_seconds, 8.0);
        assert!(d.last_camera.is_none());
        let prefs: InspectorPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs.stage, StagePrefs::default());
        let partial: StagePrefs = serde_json::from_str(r#"{"tour_seconds":20.0}"#).unwrap();
        assert_eq!(partial.tour_seconds, 20.0);
        assert!(partial.animate_camera);
    }

    #[test]
    fn grid_pitch_round_trips_and_defaults() {
        let d: DisplayToggles = serde_json::from_str("{}").unwrap();
        assert_eq!(d.grid_pitch, GridPitch::Two);
        assert!(!d.label_all);
        assert!(d.slot_rings && d.ticks && d.overlays);
        let half: DisplayToggles = serde_json::from_str(r#"{"grid_pitch":"Half"}"#).unwrap();
        assert_eq!(half.grid_pitch, GridPitch::Half);
        assert_eq!(half.grid_pitch.metres(), 0.5);
        for g in GridPitch::ALL {
            assert!(g.metres() > 0.0);
            assert!(g.label().ends_with('m'));
        }
    }

    #[test]
    fn selection_key_tracks_what_is_picked() {
        let mut v = crate::stage::StageView::new();
        v.layout_path = "does-not-exist.json".into();
        let empty = selection_key(&v);
        v.sel_stage = true;
        assert_ne!(selection_key(&v), empty);
        v.sel_stage = false;
        assert_eq!(selection_key(&v), empty);
        v.selection.insert(3);
        let one = selection_key(&v);
        v.selection.insert(7);
        assert_ne!(selection_key(&v), one);
        v.selection.remove(&7);
        assert_eq!(selection_key(&v), one);
    }

    /// The Stage tab at the panel's 230 px minimum, with saved cameras and a
    /// non-default display state, plus a zoom-2 pass. Read-only on the
    /// working directory: prefs and bookmarks are poked in memory only
    /// (never `set_tab`, `save_cameras` or `Settings::save`), so `dirty`
    /// stays false and nothing on disk moves.
    #[test]
    fn inspector_stage_tab_renders_headless() {
        let mut app = crate::app::App::new();
        app.stage.layout_path = std::env::temp_dir().join("dmxpress-stage-tab-test.json");
        *app.net.dmx.lock() = Frame([170u8; DMX_SLOTS]);
        app.collapsed.insert("channels");
        app.collapsed.remove("inspector");
        app.collapsed.remove("fixtures");
        app.show_log = false;
        app.show_osc = false;
        app.insp.prefs.tab = InspectorTab::Stage;
        app.insp.prefs.width = Some(230.0);
        app.insp.prefs.stage.animate_camera = false;
        app.cameras.clear();
        let mut front = app.stage.bookmark("Front of house".into());
        front.hotkey = Some(1);
        front.color = Some([240, 120, 40]);
        app.cameras.push(front);
        let mut top = app.stage.bookmark("Top plan".into());
        top.fly_mode = true;
        top.hotkey = Some(2);
        app.cameras.push(top);

        let size = [1000, 760];
        let shot = |app: &mut crate::app::App, name: &str, size: [u32; 2]| -> bool {
            let Some(pixels) = render_frames(5, size, |ctx, frame| {
                if frame == 0 {
                    crate::ui::install_theme(ctx);
                } else {
                    app.draw_ui(ctx);
                }
            }) else {
                eprintln!("no GPU adapter — skipping");
                return false;
            };
            save(&pixels, size, name);
            assert!(lit(&pixels) > 1000, "{name} came out black");
            true
        };

        // 1: orbit mode, a few lights selected and framed.
        for i in 0..3.min(app.patch.fixtures.len()) {
            app.stage.select_fixture(i, true);
        }
        app.stage.frame_selection(&app.settings, false);
        if !shot(&mut app, "inspector_stage_headless_1", size) {
            return;
        }

        // 2: fly mode, the Clean look, the stage box picked, a top view.
        app.stage.fly_mode = true;
        app.insp.prefs.fly_mode = true;
        app.insp.prefs.display = DisplayPreset::Clean.toggles();
        app.stage.quick_view(QuickView::Top, &app.settings, false, false);
        app.stage.clear_selection();
        app.stage.sel_stage = true;
        app.insp.prefs.stage.show_camera_numbers = true;
        shot(&mut app, "inspector_stage_headless_2", size);

        // 3: everything at A+ zoom.
        app.zoom.inspector = 2.0;
        app.insp.stage.rename = Some((0, "Wide FOH".into()));
        shot(&mut app, "inspector_stage_headless_3", [1000, 1500]);

        // 4: the whole tab at 230 px with a row being renamed and both
        // folds open.
        app.zoom.inspector = 1.0;
        app.stage.fly_mode = false;
        app.insp.prefs.fly_mode = false;
        app.insp.prefs.display = DisplayPreset::Plan.toggles();
        app.insp.prefs.folded.insert("stage.options".into());
        app.insp.prefs.folded.insert("stage.controls".into());
        shot(&mut app, "inspector_stage_headless_4", [1000, 1600]);

        // 5: nothing saved, so the empty state and the disabled tour show.
        app.insp.stage.rename = None;
        app.cameras.clear();
        app.insp.prefs.display = DisplayToggles::default();
        app.insp.prefs.stage.show_camera_numbers = false;
        shot(&mut app, "inspector_stage_headless_5", [1000, 1200]);

        assert!(!app.insp.dirty, "the headless scenes must not mark prefs dirty");
        let _ = std::fs::remove_file(&app.stage.layout_path);
    }
}
