//! Inspector · Selection tab (area E): everything that moves, turns, lines
//! up, aims or hangs whatever is picked on the stage.
//!
//! The tab is one header card, an always-visible Select bar, the transform
//! grid and the nudge pad, then a five-page tool strip
//! (`Move · Arrange · Aim · Hang · Clip`). With a tower or truss picked
//! instead of lights it shows that element's editor and nothing else.
//!
//! Nothing here mutates the stage while it draws: every button pushes a
//! [`SelAction`] into a local list that [`App::apply_sel_action`] applies —
//! and logs — once the body is done. The maths lives in `stage::arrange`,
//! the editor's stage side in `stage::inspector`.

use eframe::egui::{self, Color32};
use serde::{Deserialize, Serialize};

use super::{focus_of, focus_text, Focus};
use crate::app::App;
use crate::net::Frame;
use crate::stage::{
    self, classify, v3, AimTarget, AlignMode, Archetype, ArrangeShape, Axis, DistributeMode,
    ElementAction, ElementRef, ElementSpot, Facing, HangFill, MirrorPivot,
    MirrorPlane, OrderBy, PasteParts, SelectionSummary, TransformClip, V3, NUDGE_STEPS, ROT_STEPS,
    SNAP_STEPS,
};
use crate::ui::{
    icons::{self, Icon},
    theme,
};

// ------------------------------------------------------------- prefs -------

/// The tool strip's five pages.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub(crate) enum SelTool {
    #[default]
    Move,
    Arrange,
    Aim,
    Hang,
    Clip,
}

impl SelTool {
    pub const ALL: [SelTool; 5] =
        [SelTool::Move, SelTool::Arrange, SelTool::Aim, SelTool::Hang, SelTool::Clip];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn label(self) -> &'static str {
        match self {
            SelTool::Move => "Move",
            SelTool::Arrange => "Arrange",
            SelTool::Aim => "Aim",
            SelTool::Hang => "Hang",
            SelTool::Clip => "Clip",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            SelTool::Move => Icon::AlignMid,
            SelTool::Arrange => Icon::ArrangeLine,
            SelTool::Aim => Icon::Target,
            SelTool::Hang => Icon::Hook,
            SelTool::Clip => Icon::Paste,
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            SelTool::Move => "Line the selection up, mirror it, turn it and set a common height.",
            SelTool::Arrange => "Re-lay the selection into a line, an arc, a circle or a grid.",
            SelTool::Aim => "Point the selection at the stage, at a point, or straight down.",
            SelTool::Hang => "Hang the selection on a tower or truss, or take it off again.",
            SelTool::Clip => "Copy a placement onto other lights, and make copies.",
        }
    }
}

/// Which shape the Arrange page lays out.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub(crate) enum ShapeKind {
    #[default]
    Line,
    Arc,
    Circle,
    Grid,
}

impl ShapeKind {
    pub const ALL: [ShapeKind; 4] =
        [ShapeKind::Line, ShapeKind::Arc, ShapeKind::Circle, ShapeKind::Grid];

    pub fn label(self) -> &'static str {
        match self {
            ShapeKind::Line => "Line",
            ShapeKind::Arc => "Arc",
            ShapeKind::Circle => "Circle",
            ShapeKind::Grid => "Grid",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            ShapeKind::Line => Icon::ArrangeLine,
            ShapeKind::Arc => Icon::ArrangeArc,
            ShapeKind::Circle => Icon::ArrangeCircle,
            ShapeKind::Grid => Icon::ArrangeGrid,
        }
    }
}

/// Which target the Aim page's pads last chose.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub(crate) enum AimKind {
    #[default]
    StageCentre,
    Centroid,
    Point,
    Transition,
    Chase,
}

/// Tool parameters the operator sets once per rig; cosmetic, saved in
/// inspector.json under "selection".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SelectionPrefs {
    pub tool: SelTool,
    pub nudge_step_i: usize,
    pub rot_step_i: usize,
    pub camera_axes: bool,
    pub gap: f32,
    pub shape: ShapeKind,
    pub line_heading: f32,
    pub line_spacing: f32,
    pub line_fit: bool,
    pub line_length: f32,
    pub arc_radius: f32,
    pub arc_heading: f32,
    pub arc_sweep: f32,
    pub circle_radius: f32,
    pub circle_start: f32,
    pub facing: Facing,
    pub grid_cols: usize,
    pub grid_dx: f32,
    pub grid_dz: f32,
    pub order: OrderBy,
    pub centre_on_stage: bool,
    pub mirror_copy: bool,
    pub mirror_pivot: MirrorPivot,
    pub aim_kind: AimKind,
    pub aim_point: V3,
    pub aim_heads: bool,
    pub fan_deg: f32,
    pub hang_fill: HangFill,
    pub hang_face: usize,
    pub snap_i: usize,
    pub stagger: f32,
    pub scatter_m: f32,
    pub paste: PasteParts,
    pub dup_count: usize,
    pub dup_offset: V3,
    pub every_n: usize,
}

impl Default for SelectionPrefs {
    fn default() -> Self {
        Self {
            tool: SelTool::Move,
            nudge_step_i: 2,
            rot_step_i: 2,
            camera_axes: false,
            gap: 1.0,
            shape: ShapeKind::Line,
            line_heading: 90.0,
            line_spacing: 1.0,
            line_fit: false,
            line_length: 4.0,
            arc_radius: 3.0,
            arc_heading: 0.0,
            arc_sweep: 120.0,
            circle_radius: 2.5,
            circle_start: 0.0,
            facing: Facing::Keep,
            grid_cols: 4,
            grid_dx: 1.0,
            grid_dz: 1.0,
            order: OrderBy::Address,
            centre_on_stage: false,
            mirror_copy: false,
            mirror_pivot: MirrorPivot::Selection,
            aim_kind: AimKind::StageCentre,
            aim_point: v3(0.0, 1.0, 0.0),
            aim_heads: false,
            fan_deg: 60.0,
            hang_fill: HangFill::Spread,
            hang_face: 1,
            snap_i: 2,
            stagger: 0.5,
            scatter_m: 0.3,
            paste: PasteParts::default(),
            dup_count: 1,
            dup_offset: v3(0.6, 0.0, 0.0),
            every_n: 2,
        }
    }
}

impl SelectionPrefs {
    pub fn nudge_step(&self) -> f32 {
        NUDGE_STEPS[self.nudge_step_i.min(NUDGE_STEPS.len() - 1)]
    }

    pub fn rot_step(&self) -> f32 {
        ROT_STEPS[self.rot_step_i.min(ROT_STEPS.len() - 1)]
    }

    pub fn snap_step(&self) -> f32 {
        SNAP_STEPS[self.snap_i.min(SNAP_STEPS.len() - 1)]
    }

    /// The shape the Arrange page would lay `n` lights out in.
    pub fn arrange_shape(&self, n: usize) -> ArrangeShape {
        match self.shape {
            ShapeKind::Line => ArrangeShape::Line {
                heading_deg: self.line_heading,
                spacing: if self.line_fit && n > 1 {
                    self.line_length / (n - 1) as f32
                } else {
                    self.line_spacing
                },
            },
            ShapeKind::Arc => ArrangeShape::Arc {
                radius: self.arc_radius,
                heading_deg: self.arc_heading,
                sweep_deg: self.arc_sweep,
                facing: self.facing,
            },
            ShapeKind::Circle => ArrangeShape::Circle {
                radius: self.circle_radius,
                start_deg: self.circle_start,
                facing: self.facing,
            },
            ShapeKind::Grid => ArrangeShape::Grid {
                cols: self.grid_cols.max(1),
                spacing_x: self.grid_dx,
                spacing_z: self.grid_dz,
            },
        }
    }

    /// The one-line preview under the Arrange rows.
    pub fn arrange_preview(&self, n: usize) -> String {
        match self.arrange_shape(n) {
            ArrangeShape::Line { spacing, .. } => {
                format!("{n} lights over {:.1} m", spacing * (n.max(1) - 1) as f32)
            }
            ArrangeShape::Arc { radius, sweep_deg, .. } => format!(
                "r {radius:.1} m · {:.0}° apart",
                if n > 1 { sweep_deg / (n - 1) as f32 } else { 0.0 }
            ),
            ArrangeShape::Circle { radius, .. } => {
                format!("r {radius:.1} m · {:.0}° apart", 360.0 / n.max(1) as f32)
            }
            ArrangeShape::Grid { cols, .. } => {
                format!("{} rows × {cols}", n.div_ceil(cols.max(1)))
            }
        }
    }
}

/// Per-launch scratch state of the tab.
#[derive(Default)]
pub(crate) struct SelectionUi {
    /// A DragValue drag / typing session is open; it undoes as one step.
    pub gesture_open: bool,
    pub clip: Option<TransformClip>,
    pub hang_element: Option<ElementRef>,
    pub height: f32,
    pub last_sel: Vec<usize>,
    pub rotate_deg: f32,
    pub every_off: usize,
    /// The lights that were selected before a tower or truss was picked.
    pub last_light_sel: Vec<usize>,
    pub last_instances_len: usize,
    pub last_aim: Option<AimTarget>,
    /// Text waiting to go onto the system clipboard next frame.
    pub pending_clipboard: Option<String>,
}

// ----------------------------------------------------------- intents -------

/// What a Select button asked for.
#[derive(Clone, Copy, PartialEq)]
enum SelectKind {
    All,
    None,
    Invert,
    SameType,
    SameKind,
    Copies,
    Unmounted,
    OnElement(ElementRef),
    Element(ElementRef),
    EveryNth,
    Kinds(&'static [Archetype], bool),
    Group(usize, bool),
    Anchor(usize),
    Drop(usize),
}

/// Deferred intents gathered inside closures and applied once after the
/// whole body, so nothing mutates `self` while it is borrowed for drawing.
enum SelAction {
    Select(SelectKind),
    Nudge(V3, f32),
    Align(Axis, AlignMode),
    Distribute(Axis, DistributeMode),
    CentreStage,
    Arrange,
    Mirror(MirrorPlane),
    Rotate(f32),
    Spread(f32),
    Aim(AimKind),
    AimDir(AimTarget),
    AimHeads,
    Fan,
    Cross,
    Height(f32),
    Snap,
    Stagger,
    Scatter,
    Hang,
    Detach,
    Reseat,
    Copy { os: bool },
    Paste,
    Match,
    Duplicate,
    RemoveCopy,
    Element(ElementAction),
    Place(ElementSpot),
    Reset(ResetWhat),
    SetPitch(f32),
}

/// Which value a transform row's reset button puts back to its default.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ResetWhat {
    Axis(Axis),
    Position,
    Yaw,
    Pitch,
    Roll,
    Scale,
    Opacity,
    Rotation,
}

impl ResetWhat {
    fn word(self) -> &'static str {
        match self {
            ResetWhat::Axis(a) => a.label(),
            ResetWhat::Position => "position",
            ResetWhat::Yaw => "yaw",
            ResetWhat::Pitch => "pitch",
            ResetWhat::Roll => "base spin",
            ResetWhat::Scale => "size",
            ResetWhat::Opacity => "opacity",
            ResetWhat::Rotation => "rotation",
        }
    }
}

const NEED_ONE: &str = "Select some lights first — click one on the stage or in Fixtures.";
const NEED_TWO: &str = "Select at least two lights.";
const NEED_THREE: &str = "Select at least three lights.";

/// `Ok` when at least `need` lights are picked, else the sentence to show.
fn gate(n: usize, need: usize) -> Result<(), &'static str> {
    if n >= need {
        Ok(())
    } else {
        Err(match need {
            0 | 1 => NEED_ONE,
            2 => NEED_TWO,
            _ => NEED_THREE,
        })
    }
}

/// A row of chips that pick one value. Returns true when the pick changed.
///
/// `theme::chip` takes its id from its text, so a row is wrapped in ONE
/// `push_id` to keep two rows of "0.5 m" apart — never one scope per chip,
/// which would stop the row wrapping.
fn chips<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    salt: &str,
    current: &mut T,
    choices: &[(T, &str)],
    hint: &str,
) -> bool {
    let mut changed = false;
    ui.push_id(salt, |ui| {
        theme::toolbar(ui, |ui| {
            for (value, label) in choices {
                let hit = theme::chip(ui, label, theme::ACCENT_SOFT, *current == *value);
                if hit.on_hover_text(hint).clicked() && *current != *value {
                    *current = *value;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// A row of chips that fire an action rather than holding a state. One
/// `push_id` around the row, for the reason [`chips`] gives.
fn chip_buttons(ui: &mut egui::Ui, salt: &str, labels: &[&str], hint: &str) -> Option<usize> {
    let mut hit = None;
    ui.push_id(salt, |ui| {
        theme::toolbar(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                if theme::chip(ui, label, theme::ACCENT_SOFT, false).on_hover_text(hint).clicked() {
                    hit = Some(i);
                }
            }
        });
    });
    hit
}

/// A slider inside a `kv_grid` cell brings its own value box, so the track
/// has to be told what is left of the row or the grid pushes the panel open.
fn slider_room(ui: &mut egui::Ui) {
    let z = theme::zoom_of(ui);
    ui.spacing_mut().slider_width = (panel_width(ui) - 150.0 * z).max(40.0);
}

/// The panel's own width, stashed by the tab's prologue.
///
/// egui grows a `Ui`'s `max_rect` to cover anything that overflows it, so
/// the moment one widget is too wide every `available_width` after it reads
/// wider than the panel really is — and the next row believes it has room.
/// Measuring against the stashed width instead keeps a strip honest.
fn panel_width(ui: &egui::Ui) -> f32 {
    ui.data(|d| d.get_temp::<f32>(egui::Id::new("insp_sel_panel_w")))
        .unwrap_or_else(|| ui.available_width())
}

/// The inner width a ComboBox may take on the row it is on, allowing for
/// its own padding and arrow — a combo asked for the whole remaining width
/// paints wider than that and walks a content-sized panel open.
/// A combo's caption, dropped when the panel is too narrow to carry both it
/// and a readable combo — the combo's own selected text says the same thing.
fn combo_label(ui: &mut egui::Ui, text: &str) {
    if panel_width(ui) >= 150.0 * theme::zoom_of(ui) {
        theme::label_dim(ui, text);
    }
}

fn combo_width(ui: &egui::Ui) -> f32 {
    let left = ui.max_rect().min.x;
    let right = left + ui.max_rect().width().min(panel_width(ui));
    ((right - ui.cursor().min.x) - 34.0 * theme::zoom_of(ui)).max(60.0)
}

/// A DragValue kept to the kit's 64 px — or to whatever is left of the row
/// at a big zoom, so a two-column grid never pushes the panel open.
fn num(ui: &mut egui::Ui, dv: egui::DragValue<'_>, hint: &str) -> egui::Response {
    let w = field_width(ui);
    let h = ui.spacing().interact_size.y;
    ui.add_sized(egui::vec2(w, h), dv).on_hover_text(hint)
}

/// The width a value field may take here: the kit's 64 px scaled by the
/// panel zoom, but never more than a fair share of the panel. A grid cell's
/// own `available_width` is the whole rest of the row, so sizing off that
/// lets a two-column grid walk a content-sized panel open at zoom 2.
fn field_width(ui: &egui::Ui) -> f32 {
    (64.0 * theme::zoom_of(ui)).min((panel_width(ui) * 0.45).max(44.0))
}

/// The left cell of a transform row: a dim label plus a WARN "mixed" pill
/// when the selection does not agree on the value. At a big zoom the pill
/// shrinks to a ≠, which is all the room a 230 px panel leaves.
fn row_label(ui: &mut egui::Ui, text: &str, mixed: bool) {
    let tight = !has_room_for_resets(ui);
    ui.horizontal(|ui| {
        theme::label_dim(ui, text);
        if mixed {
            theme::pill(ui, if tight { "≠" } else { "mixed" }, theme::WARN)
                .on_hover_text("The selected lights differ here; typing a value sets them all.");
        }
    });
}

/// Is there room on a transform row for a label, a field AND a reset
/// button? At zoom 2 in a 230 px panel there is not, so the per-row resets
/// give way to the footer ones.
fn has_room_for_resets(ui: &egui::Ui) -> bool {
    panel_width(ui) >= 128.0 * theme::zoom_of(ui)
}

/// A 1 px upright rule inside a toolbar, to group buttons without a
/// `ui.separator`.
fn rule(ui: &mut egui::Ui) {
    let h = ui.spacing().interact_size.y;
    let (r, _) = ui.allocate_exact_size(egui::vec2(5.0, h), egui::Sense::hover());
    ui.painter().vline(r.center().x, r.y_range(), egui::Stroke::new(1.0, theme::EDGE));
}

/// Break a wrapping row before an item `w` wide when this row has no room
/// for it.
///
/// The kit's own controls claim their space before they draw, so a
/// `theme::toolbar` breaks before them by itself. A handful of things still
/// cannot: anything built inside a child `Ui` — a `ui.add_sized` value
/// field, a `theme::pill`'s frame, a stacked cluster of buttons — is placed
/// with `allocate_rect`, which never wraps. Those say how wide they are
/// first.
///
/// Only ever called inside a `theme::toolbar`: `end_row` means something
/// else inside a grid.
fn wrap_before(ui: &mut egui::Ui, w: f32) {
    // `Ui::available_width` reports the whole row inside a wrapping layout —
    // egui means "you may have this much, I will fold you onto the next
    // line" — so the room actually left has to be measured from the cursor.
    let left = ui.max_rect().min.x;
    let right = left + ui.max_rect().width().min(panel_width(ui));
    let cursor = ui.cursor().min.x;
    if cursor > left + 0.5 && right - cursor < w {
        ui.end_row();
    }
}

/// [`wrap_before`] for a `theme::pill` or a `theme::chip`, measured at the
/// 11 px face they set rather than the button style.
fn wrap_before_pill(ui: &mut egui::Ui, text: &str) {
    let z = theme::zoom_of(ui);
    let font = egui::FontId::new(11.0 * z, theme::medium());
    let w = ui.painter().layout_no_wrap(text.to_owned(), font, theme::TEXT).size().x + 13.0 * z;
    wrap_before(ui, w);
}

/// The label a `theme::tool_button` can actually show here, or `None` once
/// the words would not fit a whole row on their own — at zoom 2 in a 230 px
/// panel an icon with its tooltip beats a clipped caption.
fn fits_label<'a>(ui: &egui::Ui, label: &'a str) -> Option<&'a str> {
    (theme::tool_size(ui, Some(label)).x <= panel_width(ui)).then_some(label)
}

/// A type name cut to something a pill can hold at 230 px.
fn short(name: &str) -> String {
    let stem = super::type_stem(name);
    if stem.chars().count() <= 14 {
        return stem.to_owned();
    }
    let cut: String = stem.chars().take(13).collect();
    format!("{}\u{2026}", cut.trim_end())
}

/// The word an archetype goes by on a pill.
fn arch_word(a: Archetype) -> &'static str {
    match a {
        Archetype::Par => "Par",
        Archetype::Bar => "Bar",
        Archetype::MovingPar => "Moving head",
        Archetype::Beam => "Beam",
        Archetype::Specialty => "Other",
    }
}

const PARS: &[Archetype] = &[Archetype::Par, Archetype::Specialty];
const MOVERS: &[Archetype] = &[Archetype::MovingPar, Archetype::Beam];
const BARS: &[Archetype] = &[Archetype::Bar];

impl App {
    // ------------------------------------------------------------ body ----

    pub(crate) fn inspector_selection_tab(&mut self, ui: &mut egui::Ui) {
        let buf = self.sel_prologue(ui);
        let mut acts: Vec<SelAction> = Vec::new();
        let summary = self.stage.selection_summary(&self.patch);
        let n = summary.lights;

        self.sel_header(ui, &summary, &buf, &mut acts);
        ui.add_space(6.0);
        self.sel_select_bar(ui, n, &mut acts);
        ui.add_space(6.0);

        match summary.element {
            Some((r, mounted, slots)) if n == 0 => {
                self.sel_element_editor(ui, r, mounted, slots, &mut acts);
                ui.add_space(4.0);
                self.sel_nudge_card(ui, 0, &mut acts);
            }
            _ if n > 0 => {
                if n >= 2 {
                    self.sel_light_list(ui, &summary, &buf, &mut acts);
                }
                self.sel_transform_fold(ui, n, &mut acts);
                self.sel_nudge_card(ui, n, &mut acts);
                ui.add_space(6.0);
                self.sel_tool_strip(ui);
                ui.add_space(4.0);
                self.sel_tool_page(ui, &summary, &mut acts);
            }
            _ => {
                theme::hint(ui, "Pick a light, a tower or a truss to move it about.");
            }
        }
        ui.add_space(8.0);
        for a in acts {
            self.apply_sel_action(a);
        }
        if let Some(text) = self.insp.selection.pending_clipboard.take() {
            ui.output_mut(|o| o.copied_text = text);
        }
    }

    /// Everything the tab has to settle before it draws a single widget:
    /// stale indices, the hang target, the height seed, the nudge step the
    /// stage's arrow keys use, and the open DragValue gesture.
    fn sel_prologue(&mut self, ui: &egui::Ui) -> Frame {
        let len = self.stage.instances.len();
        self.stage.selection.retain(|&i| i < len);
        let (towers, trusses) = (self.stage.towers.len(), self.stage.trusses.len());
        let live = move |r: ElementRef| match r {
            ElementRef::Tower(i) => i < towers,
            ElementRef::Truss(i) => i < trusses,
        };
        let picked = self
            .stage
            .sel_tower
            .map(ElementRef::Tower)
            .or(self.stage.sel_truss.map(ElementRef::Truss));
        let first = (towers > 0)
            .then_some(ElementRef::Tower(0))
            .or((trusses > 0).then_some(ElementRef::Truss(0)));
        let mut now: Vec<usize> = self.stage.selection.iter().copied().collect();
        now.sort_unstable();
        let centroid = self.stage.selection_centroid();

        let s = &mut self.insp.selection;
        s.hang_element =
            s.hang_element.filter(|r| live(*r)).or(picked.filter(|r| live(*r))).or(first);
        if s.last_instances_len != len {
            s.last_instances_len = len;
            s.last_light_sel.clear();
        } else if !now.is_empty() {
            s.last_light_sel.clone_from(&now);
        }
        if s.last_sel != now {
            s.last_sel = now;
            if let Some(c) = centroid {
                s.height = c.y;
            }
        }
        // (`nudge_step` / `nudge_camera_relative` are stamped onto the stage
        // by `stamp_stage_prefs` every frame, not from here: the arrow keys
        // work over the stage canvas whichever tab is up.)
        // What the panel is really this wide, before anything overflows it.
        let w = ui.available_width();
        ui.data_mut(|d| d.insert_temp(egui::Id::new("insp_sel_panel_w"), w));
        let ctx = ui.ctx();
        if !ctx.is_using_pointer() && !ctx.wants_keyboard_input() {
            self.insp.selection.gesture_open = false;
        }
        *self.net.dmx.lock()
    }

    // ---------------------------------------------------------- header ----

    /// The card that says exactly what the tools are about to hit.
    fn sel_header(
        &mut self,
        ui: &mut egui::Ui,
        summary: &SelectionSummary,
        buf: &Frame,
        acts: &mut Vec<SelAction>,
    ) {
        let focus = focus_of(&self.stage, &self.patch);
        if matches!(focus, Focus::Nothing) {
            theme::empty_state(
                ui,
                Icon::Selection,
                "Nothing selected",
                "Click a light on the stage, or use Select below.",
            );
            return;
        }
        let (title, detail) = focus_text(&focus, &self.patch, &self.settings);
        let icon = match &focus {
            Focus::Tower { .. } => Icon::Tower,
            Focus::Truss { kind: stage::TrussKind::Radius, .. } => Icon::TrussRadius,
            Focus::Truss { .. } => Icon::TrussStraight,
            _ => Icon::Selection,
        };
        let anchor_colour = match &focus {
            Focus::Lights { first, .. } => {
                self.patch.fixtures.get(*first).map(|f| stage::fixture_swatch(f, buf))
            }
            _ => None,
        };
        let mount_text = summary.first_mount.map(|(r, slot)| {
            format!("on {} · slot {}", self.stage.element_name(r), slot + 1)
        });
        let mount_target = summary.first_mount.map(|(r, _)| r);
        let bounds = summary.bounds.map(|(lo, hi)| {
            format!("{:.1} × {:.1} × {:.1} m", hi.x - lo.x, hi.y - lo.y, hi.z - lo.z)
        });
        let multi = summary.lights > 1;

        theme::card(ui, |ui| {
            let z = theme::zoom_of(ui);
            ui.horizontal(|ui| {
                match anchor_colour {
                    Some(c) => {
                        theme::swatch(ui, c, 14.0).on_hover_text(
                            "The live colour of the light the tools take their anchor from.",
                        );
                    }
                    None => {
                        let (r, _) =
                            ui.allocate_exact_size(egui::Vec2::splat(14.0 * z), egui::Sense::hover());
                        icons::draw(ui.painter(), r, icon, theme::TEXT_DIM);
                    }
                }
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(title)
                            .family(theme::semibold())
                            .size(13.0)
                            .color(theme::TEXT),
                    );
                    theme::hint(ui, detail);
                });
            });
            ui.add_space(4.0);
            theme::toolbar(ui, |ui| {
                for a in &summary.archetypes {
                    wrap_before_pill(ui, arch_word(*a));
                    theme::pill(ui, arch_word(*a), theme::ACCENT_MUTED)
                        .on_hover_text("What kind of fixture the selection holds.");
                }
                if let Some((from, to)) = summary.dmx {
                    let text = format!("DMX {from}–{to}");
                    wrap_before_pill(ui, &text);
                    theme::pill(ui, &text, theme::TEXT_DIM)
                        .on_hover_text("The DMX channels this fixture answers on.");
                }
                if summary.copies_of_first > 1 {
                    wrap_before_pill(ui, &format!("×{} copies", summary.copies_of_first));
                    theme::pill(ui, &format!("×{} copies", summary.copies_of_first), theme::WARN)
                        .on_hover_text(
                            "Several lights on stage share these DMX channels — they all move \
                             together on the desk.",
                        );
                }
                if let Some(text) = &mount_text {
                    wrap_before_pill(ui, text);
                    let r = theme::pill(ui, text, theme::OK);
                    let hit = ui.interact(
                        r.rect,
                        ui.id().with("insp_sel_mount"),
                        egui::Sense::click(),
                    );
                    if hit
                        .on_hover_text("Select the tower or truss this light hangs on.")
                        .clicked()
                    {
                        if let Some(r) = mount_target {
                            acts.push(SelAction::Select(SelectKind::Element(r)));
                        }
                    }
                }
                if multi {
                    for (stem, count) in &summary.names {
                        let text = format!("{} ×{count}", short(stem));
                        wrap_before_pill(ui, &text);
                        theme::pill(ui, &text, theme::TEXT_DIM)
                            .on_hover_text(format!("{stem}: {count} of them are selected."));
                    }
                    if summary.mixed_rotation {
                        wrap_before_pill(ui, "mixed aim");
                        theme::pill(ui, "mixed aim", theme::WARN).on_hover_text(
                            "The selected lights do not all point the same way; an aim tool                              will bring them together.",
                        );
                    }
                    if summary.mounted > 0 {
                        wrap_before_pill(ui, &format!("{} hung", summary.mounted));
                        theme::pill(ui, &format!("{} hung", summary.mounted), theme::ACCENT_SOFT)
                            .on_hover_text("How many of the selected lights are hung on the rig.");
                    }
                }
                if let Some((_, mounted, slots)) = summary.element {
                    wrap_before_pill(ui, &format!("{mounted}/{slots} slots"));
                    theme::pill(ui, &format!("{mounted}/{slots} slots"), theme::ACCENT_SOFT)
                        .on_hover_text("How many of this element's mount slots hold a light.");
                }
            });
            // Its own row: the bounds never have to share, so the text
            // cannot be broken a character at a time.
            if let Some(b) = &bounds {
                ui.horizontal(|ui| {
                    theme::pill(ui, b, theme::TEXT_DIM).on_hover_text(
                        "How much space the selection covers, width × height × depth.",
                    );
                });
            }
        });
    }

    // ---------------------------------------------------------- select ----

    /// Build or narrow the selection without touching the stage.
    fn sel_select_bar(&mut self, ui: &mut egui::Ui, n: usize, acts: &mut Vec<SelAction>) {
        theme::section_with(ui, "Select", |ui| {
            theme::count_pill(ui, n, "light");
        });
        let all = self.stage.all_selected();
        let has_lights = !self.stage.instances.is_empty();
        let anchor = self.stage.last_selected.filter(|&fi| fi < self.patch.fixtures.len());
        let picked = self
            .stage
            .sel_tower
            .map(ElementRef::Tower)
            .or(self.stage.sel_truss.map(ElementRef::Truss))
            .or(self.insp.selection.hang_element);
        let mounted_target = self
            .stage
            .sel_tower
            .map(ElementRef::Tower)
            .or(self.stage.sel_truss.map(ElementRef::Truss))
            .or_else(|| {
                let s = self.stage.selection_summary(&self.patch);
                s.first_mount.map(|(r, _)| r)
            })
            .or(picked);
        let mut every_n = self.insp.prefs.selection.every_n;
        let off = self.insp.selection.every_off;
        let groups: Vec<(usize, String)> =
            self.groups.iter().enumerate().map(|(i, g)| (i, g.name.clone())).collect();
        let kinds_now = self.selected_archetypes();

        let some = if has_lights { Ok(()) } else { Err("Patch some fixtures first.") };
        theme::toolbar(ui, |ui| {
            if theme::tool_button(
                ui,
                Icon::SelectAll,
                None,
                "Select every light on the stage.",
                if all { Err("Everything is already selected.") } else { some },
            )
            .clicked()
            {
                acts.push(SelAction::Select(SelectKind::All));
            }
            if theme::tool_button(
                ui,
                Icon::SelectNone,
                None,
                "Clear the selection.",
                if n > 0 { Ok(()) } else { Err("Nothing is selected.") },
            )
            .clicked()
            {
                acts.push(SelAction::Select(SelectKind::None));
            }
            if theme::tool_button(
                ui,
                Icon::Invert,
                None,
                "Select every light that is not selected now.",
                some,
            )
            .clicked()
            {
                acts.push(SelAction::Select(SelectKind::Invert));
            }
            let need_anchor = if anchor.is_some() {
                Ok(())
            } else {
                Err("Pick a light first so there is a type to match.")
            };
            if theme::tool_button(
                ui,
                Icon::SelectType,
                None,
                "Select every light of the same type as the last one picked.",
                need_anchor,
            )
            .clicked()
            {
                acts.push(SelAction::Select(SelectKind::SameType));
            }
            if theme::tool_button(
                ui,
                Icon::SameKind,
                None,
                "Select every light of the same kind — par, bar or moving head — as the last one picked.",
                need_anchor,
            )
            .clicked()
            {
                acts.push(SelAction::Select(SelectKind::SameKind));
            }
            if theme::tool_button(
                ui,
                Icon::Copy,
                None,
                "Select every copy of the last picked fixture.",
                need_anchor,
            )
            .clicked()
            {
                acts.push(SelAction::Select(SelectKind::Copies));
            }
            if theme::tool_button(
                ui,
                Icon::Unmounted,
                None,
                "Select every light that is not hung on a tower or truss.",
                some,
            )
            .clicked()
            {
                acts.push(SelAction::Select(SelectKind::Unmounted));
            }
            if theme::tool_button(
                ui,
                Icon::Hook,
                None,
                "Select the lights hung on the picked tower or truss.",
                if mounted_target.is_some() {
                    Ok(())
                } else {
                    Err("Pick a tower or truss first, or a light that hangs on one.")
                },
            )
            .clicked()
            {
                if let Some(r) = mounted_target {
                    acts.push(SelAction::Select(SelectKind::OnElement(r)));
                }
            }
            wrap_before(ui, 5.0);
            rule(ui);
            wrap_before(ui, 64.0 * theme::zoom_of(ui));
            num(
                ui,
                egui::DragValue::new(&mut every_n).range(2..=8).prefix("every "),
                "Keep one light in this many.",
            );
            if theme::tool_button(
                ui,
                Icon::Selection,
                Some(&format!("nth +{off}")),
                "Keep every n-th light of the selection (or of the rig when nothing is selected), \
                 by address. Click again to shift which ones.",
                some,
            )
            .clicked()
            {
                acts.push(SelAction::Select(SelectKind::EveryNth));
            }
        });
        if every_n != self.insp.prefs.selection.every_n {
            self.insp.prefs.selection.every_n = every_n;
            self.insp.mark_dirty();
        }
        let shift = ui.input(|i| i.modifiers.shift);
        theme::toolbar(ui, |ui| {
            for (label, kinds, hint) in [
                ("Pars", PARS, "Select every par and plain fixture. ⇧-click adds them instead."),
                ("Movers", MOVERS, "Select every moving head and beam. ⇧-click adds them instead."),
                ("Bars", BARS, "Select every bar and batten. ⇧-click adds them instead."),
            ] {
                let on = !kinds_now.is_empty() && kinds_now.iter().all(|a| kinds.contains(a));
                if theme::chip(ui, label, theme::ACCENT_SOFT, on).on_hover_text(hint).clicked() {
                    acts.push(SelAction::Select(SelectKind::Kinds(kinds, shift)));
                }
            }
            wrap_before(ui, theme::tool_size(ui, Some("Groups")).x);
            ui.menu_button("Groups", |ui| {
                if groups.is_empty() {
                    theme::hint(ui, "No groups yet.");
                }
                for (i, name) in &groups {
                    if ui.button(name).clicked() {
                        acts.push(SelAction::Select(SelectKind::Group(*i, shift)));
                        ui.close_menu();
                    }
                }
            })
            .response
            .on_hover_text("Select a saved group's fixtures. ⇧-click adds them to the selection.");
        });
    }

    /// The distinct archetypes the current selection covers.
    fn selected_archetypes(&self) -> Vec<Archetype> {
        let mut out: Vec<Archetype> = Vec::new();
        for &i in &self.stage.selection {
            if let Some(a) = self
                .stage
                .instances
                .get(i)
                .and_then(|inst| self.patch.fixtures.get(inst.fixture))
                .map(classify)
            {
                if !out.contains(&a) {
                    out.push(a);
                }
            }
        }
        out
    }

    /// Every selected light as a row, so a wrong pick shows before a tool
    /// fires. Click makes a light the anchor; ⇧-click drops it.
    fn sel_light_list(
        &mut self,
        ui: &mut egui::Ui,
        summary: &SelectionSummary,
        buf: &Frame,
        acts: &mut Vec<SelAction>,
    ) {
        struct RowData {
            instance: usize,
            fixture: usize,
            colour: Color32,
            name: String,
            dmx: String,
            mount: Option<String>,
        }
        let sel = self.stage.sorted_selection(&self.patch, self.insp.prefs.selection.order);
        let total = sel.len();
        let anchor = self.stage.last_selected;
        let rows: Vec<RowData> = sel
            .iter()
            .take(64)
            .filter_map(|&i| {
                let fi = self.stage.instances[i].fixture;
                let f = self.patch.fixtures.get(fi)?;
                Some(RowData {
                    instance: i,
                    fixture: fi,
                    colour: stage::fixture_swatch(f, buf),
                    name: f.display.clone(),
                    dmx: format!("DMX {}–{}", f.from, f.to),
                    mount: self.stage.mount_label(fi),
                })
            })
            .collect();
        let more = total.saturating_sub(rows.len());
        let title = format!("{total} lights");
        let mounted_text = summary.mounted.to_string();
        let badge = (summary.mounted > 0).then_some((mounted_text.as_str(), theme::ACCENT_SOFT));
        self.insp_fold(ui, "selection.lights", &title, false, badge, move |_app, ui| {
            let shift = ui.input(|i| i.modifiers.shift);
            theme::well(ui, |ui| {
                for row in &rows {
                    let badges: Vec<(&str, Color32)> = row
                        .mount
                        .as_deref()
                        .map(|m| vec![(m, theme::TEXT_DIM)])
                        .unwrap_or_default();
                    let r = theme::list_row(
                        ui,
                        ("insp_sel_row", row.instance),
                        &theme::Row {
                            swatch: Some(row.colour),
                            icon: None,
                            title: &row.name,
                            subtitle: Some(&row.dmx),
                            badges: &badges,
                            selected: anchor == Some(row.fixture),
                            active: false,
                            indent: 0.0,
                        },
                    );
                    if r.on_hover_text(
                        "Click to make this light the anchor the tools measure from. \
                         ⇧-click drops it from the selection.",
                    )
                    .clicked()
                    {
                        acts.push(SelAction::Select(if shift {
                            SelectKind::Drop(row.instance)
                        } else {
                            SelectKind::Anchor(row.fixture)
                        }));
                    }
                }
                if more > 0 {
                    theme::hint(ui, format!("… and {more} more"));
                }
            });
        });
    }

    // ------------------------------------------------------- transform ----

    fn sel_transform_fold(&mut self, ui: &mut egui::Ui, n: usize, acts: &mut Vec<SelAction>) {
        let badge = n.to_string();
        self.insp_fold(
            ui,
            "selection.transform",
            "Transform",
            true,
            Some((badge.as_str(), theme::TEXT_DIM)),
            |app, ui| app.sel_light_grid(ui, acts),
        );
    }

    /// The absolute transform grid for 1..n lights: the centroid's position,
    /// the anchor's rotation, size and opacity, each row resettable. One
    /// drag or typing session is one undo step.
    fn sel_light_grid(&mut self, ui: &mut egui::Ui, acts: &mut Vec<SelAction>) {
        let Some(ed) = self.stage.light_editor(&self.patch) else { return };
        let before = ed.values;
        let mut now = before;
        let heads_only = ed.heads > 0 && ed.pars == 0;
        let resets = has_room_for_resets(ui);
        let show_angles = ed.pars > 0;
        let show_spin = ed.heads > 0;

        theme::kv_grid(ui, "insp_sel_xform", |ui| {
            ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
            for (label, mixed, value, axis, hint) in [
                (
                    "X",
                    ed.mixed.pos[0],
                    &mut now.pos.x,
                    Axis::X,
                    "Where the selection's centre sits across the stage, in metres.",
                ),
                (
                    "Y",
                    ed.mixed.pos[1],
                    &mut now.pos.y,
                    Axis::Y,
                    "How high the selection's centre hangs, in metres.",
                ),
                (
                    "Z",
                    ed.mixed.pos[2],
                    &mut now.pos.z,
                    Axis::Z,
                    "How far upstage or downstage the selection's centre sits, in metres.",
                ),
            ] {
                row_label(ui, label, mixed);
                ui.horizontal(|ui| {
                    num(
                        ui,
                        egui::DragValue::new(value).speed(0.01).fixed_decimals(2).suffix(" m"),
                        hint,
                    );
                    if resets
                        && theme::tool_button(
                            ui,
                            Icon::Reset,
                            None,
                            "Reset this value to its default.",
                            Ok(()),
                        )
                        .clicked()
                    {
                        acts.push(SelAction::Reset(ResetWhat::Axis(axis)));
                    }
                });
                ui.end_row();
            }
            if show_angles {
                row_label(ui, "Yaw", ed.mixed.yaw);
                ui.horizontal(|ui| {
                    num(
                        ui,
                        egui::DragValue::new(&mut now.yaw_deg)
                            .speed(0.5)
                            .fixed_decimals(1)
                            .range(-360.0..=360.0)
                            .suffix("°"),
                        "Which way the selected lights face, in degrees.",
                    );
                    if resets
                        && theme::tool_button(
                            ui,
                            Icon::Reset,
                            None,
                            "Reset this value to its default.",
                            Ok(()),
                        )
                        .clicked()
                    {
                        acts.push(SelAction::Reset(ResetWhat::Yaw));
                    }
                });
                ui.end_row();
                row_label(ui, "Pitch", ed.mixed.pitch);
                ui.horizontal(|ui| {
                    num(
                        ui,
                        egui::DragValue::new(&mut now.pitch_deg)
                            .speed(0.5)
                            .fixed_decimals(1)
                            .range(-360.0..=360.0)
                            .suffix("°"),
                        "How far the selected lights tilt: −90 is straight down, 0 is level.",
                    );
                    if resets
                        && theme::tool_button(
                            ui,
                            Icon::Reset,
                            None,
                            "Reset this value to its default.",
                            Ok(()),
                        )
                        .clicked()
                    {
                        acts.push(SelAction::Reset(ResetWhat::Pitch));
                    }
                });
                ui.end_row();
            }
            if show_spin {
                row_label(ui, "Spin", ed.mixed.roll);
                ui.horizontal(|ui| {
                    num(
                        ui,
                        egui::DragValue::new(&mut now.roll_deg)
                            .speed(1.0)
                            .fixed_decimals(1)
                            .range(0.0..=360.0)
                            .suffix("°"),
                        "Spin a moving head's base on its mount; the pan and tilt sweep follows.",
                    );
                    if resets
                        && theme::tool_button(
                            ui,
                            Icon::Reset,
                            None,
                            "Reset this value to its default.",
                            Ok(()),
                        )
                        .clicked()
                    {
                        acts.push(SelAction::Reset(ResetWhat::Roll));
                    }
                });
                ui.end_row();
            }
            row_label(ui, "Size", ed.mixed.scale);
            ui.horizontal(|ui| {
                num(
                    ui,
                    egui::DragValue::new(&mut now.scale)
                        .speed(0.02)
                        .fixed_decimals(2)
                        .range(0.05..=10.0)
                        .suffix("×"),
                    "Scale the selected fixture bodies up or down in the visualiser.",
                );
                if resets
                    && theme::tool_button(
                        ui,
                        Icon::Reset,
                        None,
                        "Reset this value to its default.",
                        Ok(()),
                    )
                    .clicked()
                {
                    acts.push(SelAction::Reset(ResetWhat::Scale));
                }
            });
            ui.end_row();
        });
        if show_angles {
            if let Some(i) = chip_buttons(
                ui,
                "insp_sel_pitch",
                &["↓ −90", "→ 0", "↑ 90"],
                "Point the selected lights straight down, out level, or straight up.",
            ) {
                acts.push(SelAction::SetPitch([-90.0, 0.0, 90.0][i]));
            }
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            row_label(ui, "Opacity", ed.mixed.opacity);
            if theme::tool_button(ui, Icon::Reset, None, "Bring these fixtures back to full.", Ok(()))
                .clicked()
            {
                acts.push(SelAction::Reset(ResetWhat::Opacity));
            }
            let _ = resets;
        });
        ui.scope(|ui| {
            // The slider's own value box sits beside the track, so the
            // track only gets what is left after it.
            let room = ui.available_width().min(panel_width(ui));
            ui.spacing_mut().slider_width = (room - field_width(ui) - 26.0).max(40.0);
            ui.add(egui::Slider::new(&mut now.opacity, 0.0..=1.0).fixed_decimals(2))
                .on_hover_text(
                    "Fade the whole visual contribution of these fixtures: housing, lens, beam \
                     haze and floor pool.",
                );
        });
        ui.add_space(4.0);
        theme::toolbar(ui, |ui| {
            let label = fits_label(ui, "Reset rotation");
            if theme::tool_button(
                ui,
                Icon::RotateCcw,
                label,
                "Put every selected light back to the mounting angles a new light gets.",
                Ok(()),
            )
            .clicked()
            {
                acts.push(SelAction::Reset(ResetWhat::Rotation));
            }
            let label = fits_label(ui, "Reset size");
            if theme::tool_button(
                ui,
                Icon::Height,
                label,
                "Put every selected light back to its normal size.",
                Ok(()),
            )
            .clicked()
            {
                acts.push(SelAction::Reset(ResetWhat::Scale));
            }
            let label = fits_label(ui, "Reset position");
            if theme::tool_button(
                ui,
                Icon::CentreStage,
                label,
                "Put the selection's centre back over the middle of the stage, at the default \
                 hanging height.",
                Ok(()),
            )
            .clicked()
            {
                acts.push(SelAction::Reset(ResetWhat::Position));
            }
            if heads_only {
                wrap_before_pill(ui, "heads");
                theme::pill(ui, "heads", theme::ACCENT_MUTED).on_hover_text(
                    "Moving heads only spin on their base here; aim their beams on the Aim page.",
                );
            }
        });

        if now != before {
            let opened = !self.insp.selection.gesture_open;
            let pre = self.stage.edit_pre();
            self.stage.undo_checkpoint(&mut self.insp.selection.gesture_open, pre);
            self.stage.apply_light_edit(&self.patch, before, now);
            if opened {
                let count = ed.count;
                self.log.push(format!(
                    "Edited {count} light{}",
                    if count == 1 { "" } else { "s" }
                ));
            }
        }
    }

    // ----------------------------------------------------------- nudge ----

    /// The step chips, the D-pad, the height pair and the turn pair — plus
    /// the arrow keys while the card is hovered.
    fn sel_nudge_card(&mut self, ui: &mut egui::Ui, n: usize, acts: &mut Vec<SelAction>) {
        let element = self.stage.sel_tower.is_some() || self.stage.sel_truss.is_some();
        let enabled = if n > 0 || element {
            Ok(())
        } else {
            Err("Select lights, or pick a tower or truss, to nudge.")
        };
        let step = self.insp.prefs.selection.nudge_step();
        let rot = self.insp.prefs.selection.rot_step();
        let mut step_i = self.insp.prefs.selection.nudge_step_i;
        let mut rot_i = self.insp.prefs.selection.rot_step_i;
        let mut camera = self.insp.prefs.selection.camera_axes;
        let mut changed = false;

        self.insp_fold(ui, "selection.nudge", "Nudge", true, None, |app, ui| {
            let top = ui.cursor().min;
            let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
            let mul = if shift {
                10.0
            } else if alt {
                0.1
            } else {
                1.0
            };
            let d = step * mul;
            let turn = rot * mul;
            let (right, fwd) = if camera {
                let yaw = app.stage.cam.yaw;
                (v3(yaw.cos(), 0.0, -yaw.sin()), v3(-yaw.sin(), 0.0, -yaw.cos()))
            } else {
                (v3(1.0, 0.0, 0.0), v3(0.0, 0.0, -1.0))
            };

            ui.push_id("insp_sel_step", |ui| {
                theme::toolbar(ui, |ui| {
                    for (i, step) in NUDGE_STEPS.iter().enumerate() {
                        let label = format!("{step} m");
                        let hit = theme::chip(ui, &label, theme::ACCENT_SOFT, step_i == i);
                        if hit.on_hover_text("How far one nudge moves the selection.").clicked() {
                            step_i = i;
                            changed = true;
                        }
                    }
                });
            });
            ui.push_id("insp_sel_rot", |ui| {
                theme::toolbar(ui, |ui| {
                    for (i, turn) in ROT_STEPS.iter().enumerate() {
                        let label = format!("{turn}°");
                        let hit = theme::chip(ui, &label, theme::ACCENT_SOFT, rot_i == i);
                        if hit.on_hover_text("How far one turn button spins the selection.").clicked() {
                            rot_i = i;
                            changed = true;
                        }
                    }
                });
            });
            ui.add_space(4.0);
            theme::toolbar(ui, |ui| {
                // Each cluster is one block to the parent layout, so the
                // strip is told how wide it is before it is drawn.
                let bw = theme::tool_size(ui, None).x;
                let gap = bw;
                wrap_before(ui, bw * 3.0 + ui.spacing().item_spacing.x * 2.0);
                // The floor D-pad, with the axes toggle in its middle.
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(gap);
                        if theme::tool_button(
                            ui,
                            Icon::ChevronUp,
                            None,
                            "Move upstage by the step. ⇧ ×10, ⌥ ÷10.",
                            enabled,
                        )
                        .clicked()
                        {
                            acts.push(SelAction::Nudge(fwd * d, 0.0));
                        }
                    });
                    ui.horizontal(|ui| {
                        if theme::tool_button(
                            ui,
                            Icon::ChevronLeft,
                            None,
                            "Move stage left by the step. ⇧ ×10, ⌥ ÷10.",
                            enabled,
                        )
                        .clicked()
                        {
                            acts.push(SelAction::Nudge(right * -d, 0.0));
                        }
                        if theme::toggle_icon(
                            ui,
                            if camera { Icon::Camera } else { Icon::Stage },
                            None,
                            camera,
                            "Nudge along the camera's right and forward instead of stage X and Z.",
                        )
                        .clicked()
                        {
                            camera = !camera;
                            changed = true;
                        }
                        if theme::tool_button(
                            ui,
                            Icon::ChevronRight,
                            None,
                            "Move stage right by the step. ⇧ ×10, ⌥ ÷10.",
                            enabled,
                        )
                        .clicked()
                        {
                            acts.push(SelAction::Nudge(right * d, 0.0));
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(gap);
                        if theme::tool_button(
                            ui,
                            Icon::ChevronDown,
                            None,
                            "Move downstage by the step. ⇧ ×10, ⌥ ÷10.",
                            enabled,
                        )
                        .clicked()
                        {
                            acts.push(SelAction::Nudge(fwd * -d, 0.0));
                        }
                    });
                });
                // Height.
                wrap_before(ui, bw);
                ui.vertical(|ui| {
                    if theme::tool_button(ui, Icon::ChevronUp, None, "Raise by the step. ⇧ ×10, ⌥ ÷10.", enabled)
                        .clicked()
                    {
                        acts.push(SelAction::Nudge(v3(0.0, d, 0.0), 0.0));
                    }
                    if theme::tool_button(ui, Icon::ChevronDown, None, "Lower by the step. ⇧ ×10, ⌥ ÷10.", enabled)
                        .clicked()
                    {
                        acts.push(SelAction::Nudge(v3(0.0, -d, 0.0), 0.0));
                    }
                    theme::label_dim(ui, "Y");
                });
                // Turn.
                wrap_before(ui, bw);
                ui.vertical(|ui| {
                    if theme::tool_button(
                        ui,
                        Icon::RotateCcw,
                        None,
                        "Turn anticlockwise by the angle step — the base spin on a moving head.",
                        enabled,
                    )
                    .clicked()
                    {
                        acts.push(SelAction::Nudge(V3::default(), -turn));
                    }
                    if theme::tool_button(
                        ui,
                        Icon::RotateCw,
                        None,
                        "Turn clockwise by the angle step — the base spin on a moving head.",
                        enabled,
                    )
                    .clicked()
                    {
                        acts.push(SelAction::Nudge(V3::default(), turn));
                    }
                    theme::label_dim(ui, "Turn");
                });
            });
            theme::hint(
                ui,
                "Arrow keys nudge while the stage or this card is hovered · ⌘↑↓ height · ⇧ ×10 · ⌥ ÷10.",
            );
            // Arrow keys while this card has the pointer, with the same
            // guards the stage uses.
            let bottom = ui.cursor().min.y;
            let card = egui::Rect::from_min_max(
                top,
                egui::pos2(top.x + ui.available_width(), bottom),
            );
            if ui.rect_contains_pointer(card)
                && !app.stage.fly_mode
                && !ui.ctx().wants_keyboard_input()
            {
                app.stage.nudge_keys(ui, &app.patch);
            }
        });

        if changed {
            let p = &mut self.insp.prefs.selection;
            p.nudge_step_i = step_i;
            p.rot_step_i = rot_i;
            p.camera_axes = camera;
            self.insp.mark_dirty();
        }
    }

    // ------------------------------------------------------ tool pages ----

    fn sel_tool_strip(&mut self, ui: &mut egui::Ui) {
        let segs: Vec<theme::Segment> = SelTool::ALL
            .iter()
            .map(|t| theme::Segment {
                icon: Some(t.icon()),
                label: t.label(),
                hint: t.hint(),
                badge: None,
            })
            .collect();
        let now = self.insp.prefs.selection.tool.index();
        if let Some(i) = theme::segmented(ui, "insp_sel_tools", &segs, now) {
            self.insp.prefs.selection.tool = SelTool::ALL[i];
            self.insp.mark_dirty();
        }
    }

    fn sel_tool_page(
        &mut self,
        ui: &mut egui::Ui,
        summary: &SelectionSummary,
        acts: &mut Vec<SelAction>,
    ) {
        match self.insp.prefs.selection.tool {
            SelTool::Move => self.sel_move_page(ui, summary, acts),
            SelTool::Arrange => self.sel_arrange_page(ui, summary, acts),
            SelTool::Aim => self.sel_aim_page(ui, summary, acts),
            SelTool::Hang => self.sel_hang_page(ui, summary, acts),
            SelTool::Clip => self.sel_clip_page(ui, summary, acts),
        }
    }

    fn sel_move_page(
        &mut self,
        ui: &mut egui::Ui,
        summary: &SelectionSummary,
        acts: &mut Vec<SelAction>,
    ) {
        let n = summary.lights;
        let (ge1, ge2, ge3) = (gate(n, 1), gate(n, 2), gate(n, 3));
        let mut prefs = self.insp.prefs.selection.clone();
        let mut height = self.insp.selection.height;
        let mut rotate_deg = self.insp.selection.rotate_deg;
        let stage_h = self.settings.stage_h;
        let default_h = self.settings.default_height;

        theme::section(ui, "Align & distribute");
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            let words = match axis {
                Axis::Y => ["lowest", "average", "highest"],
                _ => ["smallest", "average", "largest"],
            };
            theme::toolbar(ui, |ui| {
                wrap_before_pill(ui, axis.label());
                ui.monospace(egui::RichText::new(axis.label()).color(theme::TEXT_DIM));
                for (mode, icon, word) in [
                    (AlignMode::Min, Icon::AlignMin, words[0]),
                    (AlignMode::Mid, Icon::AlignMid, words[1]),
                    (AlignMode::Max, Icon::AlignMax, words[2]),
                ] {
                    let hint = format!(
                        "Move every selected light to the {word} {} of the selection.",
                        axis.label()
                    );
                    if theme::tool_button(ui, icon, None, &hint, ge2).clicked() {
                        acts.push(SelAction::Align(axis, mode));
                    }
                }
                wrap_before(ui, 5.0);
                rule(ui);
                let hint = format!(
                    "Space the selected lights evenly along {} between the two outermost ones.",
                    axis.label()
                );
                if theme::tool_button(ui, Icon::Distribute, None, &hint, ge3).clicked() {
                    acts.push(SelAction::Distribute(axis, DistributeMode::Even));
                }
            });
        }
        theme::toolbar(ui, |ui| {
            wrap_before(ui, 64.0 * theme::zoom_of(ui));
            num(
                ui,
                egui::DragValue::new(&mut prefs.gap).range(0.05..=10.0).speed(0.05).suffix(" m"),
                "The gap a by-gap spacing leaves between neighbours.",
            );
            let label = fits_label(ui, "by gap");
            if theme::tool_button(
                ui,
                Icon::Distribute,
                label,
                "Space the selected lights along whichever axis they spread most on, starting \
                 from the lowest, with this gap.",
                ge2,
            )
            .clicked()
            {
                acts.push(SelAction::Distribute(
                    self.stage.widest_axis(),
                    DistributeMode::Gap(prefs.gap),
                ));
            }
            let label = fits_label(ui, "Centre on stage");
            if theme::tool_button(
                ui,
                Icon::CentreStage,
                label,
                "Shift the selection so its centre sits at stage X = 0, Z = 0. Height is kept.",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::CentreStage);
            }
        });

        ui.add_space(4.0);
        theme::section(ui, "Mirror & rotate");
        theme::toolbar(ui, |ui| {
            let label = fits_label(ui, "Mirror X");
            if theme::tool_button(
                ui,
                Icon::MirrorX,
                label,
                "Flip the selection left ↔ right across the pivot. A moving head keeps its \
                 programmed pan — re-aim it afterwards.",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::Mirror(MirrorPlane::X));
            }
            let label = fits_label(ui, "Mirror Z");
            if theme::tool_button(
                ui,
                Icon::MirrorZ,
                label,
                "Flip the selection front ↔ back across the pivot.",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::Mirror(MirrorPlane::Z));
            }
            if theme::chip(ui, "as copy", theme::ACCENT_SOFT, prefs.mirror_copy)
                .on_hover_text("Leave the originals where they are and mirror a fresh set.")
                .clicked()
            {
                prefs.mirror_copy = !prefs.mirror_copy;
            }
        });
        chips(
            ui,
            "insp_sel_pivot",
            &mut prefs.mirror_pivot,
            &[(MirrorPivot::Selection, "about selection"), (MirrorPivot::Stage, "about stage")],
            "Whether a mirror reflects about the selection's own centre or the stage's.",
        );
        let step = prefs.rot_step();
        theme::toolbar(ui, |ui| {
            if theme::tool_button(
                ui,
                Icon::RotateCcw,
                Some(&format!("{step}°")),
                "Turn the whole group anticlockwise about its centre by the angle step; each \
                 light turns with it.",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::Rotate(-step));
            }
            if theme::tool_button(
                ui,
                Icon::RotateCw,
                Some(&format!("{step}°")),
                "Turn the whole group clockwise about its centre by the angle step.",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::Rotate(step));
            }
        });
        if let Some(i) = chip_buttons(
            ui,
            "insp_sel_turn",
            &["−90", "−45", "+45", "+90"],
            "Turn the whole group about its centre by this many degrees.",
        ) {
            acts.push(SelAction::Rotate([-90.0, -45.0, 45.0, 90.0][i]));
        }
        theme::toolbar(ui, |ui| {
            wrap_before(ui, 64.0 * theme::zoom_of(ui));
            num(
                ui,
                egui::DragValue::new(&mut rotate_deg).speed(1.0).range(-360.0..=360.0).suffix("°"),
                "A turn of your own choosing.",
            );
            let label = fits_label(ui, "Rotate");
            if theme::tool_button(ui, Icon::RotateCw, label, "Turn the group by this angle.", ge1)
                .clicked()
            {
                acts.push(SelAction::Rotate(rotate_deg));
            }
            let label = fits_label(ui, "Flip 180");
            if theme::tool_button(
                ui,
                Icon::MirrorZ,
                label,
                "Turn the group half a turn about its centre.",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::Rotate(180.0));
            }
        });
        theme::toolbar(ui, |ui| {
            let label = fits_label(ui, "×1.1");
            if theme::tool_button(
                ui,
                Icon::Spread,
                label,
                "Spread the selection out from its centre by 10 %. Height is kept.",
                ge2,
            )
            .clicked()
            {
                acts.push(SelAction::Spread(1.1));
            }
            let label = fits_label(ui, "÷1.1");
            if theme::tool_button(
                ui,
                Icon::Contract,
                label,
                "Pull the selection in toward its centre by 10 %. Height is kept.",
                ge2,
            )
            .clicked()
            {
                acts.push(SelAction::Spread(1.0 / 1.1));
            }
        });

        ui.add_space(4.0);
        theme::section(ui, "Height & grid");
        theme::toolbar(ui, |ui| {
            wrap_before(ui, 64.0 * theme::zoom_of(ui));
            num(
                ui,
                egui::DragValue::new(&mut height)
                    .range(0.0..=20.0)
                    .speed(0.02)
                    .fixed_decimals(2)
                    .suffix(" m"),
                "The height every selected light goes to.",
            );
            let label = fits_label(ui, "Set");
            if theme::tool_button(
                ui,
                Icon::Height,
                label,
                "Put every selected light at this height.",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::Height(height));
            }
        });
        if let Some(i) = chip_buttons(
            ui,
            "insp_sel_height",
            &["Floor", "Stage", "Default", "Tallest", "Lowest"],
            "Jump the whole selection to a common height.",
        ) {
            match i {
                0 => acts.push(SelAction::Height(0.3)),
                1 => acts.push(SelAction::Height(stage_h + 0.3)),
                2 => acts.push(SelAction::Height(default_h)),
                3 => acts.push(SelAction::Align(Axis::Y, AlignMode::Max)),
                _ => acts.push(SelAction::Align(Axis::Y, AlignMode::Min)),
            }
        }
        let snap_labels: Vec<String> = SNAP_STEPS.iter().map(|s| format!("{s} m")).collect();
        theme::toolbar(ui, |ui| {
            for (i, label) in snap_labels.iter().enumerate() {
                let hit = theme::chip(ui, label, theme::ACCENT_SOFT, prefs.snap_i == i);
                if hit.on_hover_text("The grid pitch positions round onto.").clicked() {
                    prefs.snap_i = i;
                }
            }
            let label = fits_label(ui, "Snap positions");
            if theme::tool_button(
                ui,
                Icon::Grid,
                label,
                "Round every selected light's X, Y and Z onto the nearest multiple of the step.",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::Snap);
            }
        });
        theme::toolbar(ui, |ui| {
            wrap_before(ui, 64.0 * theme::zoom_of(ui));
            num(
                ui,
                egui::DragValue::new(&mut prefs.stagger).range(-5.0..=5.0).speed(0.05).suffix(" m"),
                "How far every second light lifts.",
            );
            let label = fits_label(ui, "Stagger");
            if theme::tool_button(
                ui,
                Icon::Stagger,
                label,
                "Lift every second light by this much, for a two-row look.",
                ge2,
            )
            .clicked()
            {
                acts.push(SelAction::Stagger);
            }
            wrap_before(ui, 64.0 * theme::zoom_of(ui));
            num(
                ui,
                egui::DragValue::new(&mut prefs.scatter_m).range(0.0..=3.0).speed(0.05).suffix(" m"),
                "How far a scatter may throw a light.",
            );
            let label = fits_label(ui, "Scatter");
            if theme::tool_button(
                ui,
                Icon::Scatter,
                label,
                "Jitter the selection on the floor by up to this much, so the rig stops looking \
                 like a spreadsheet.",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::Scatter);
            }
        });

        self.stash_prefs(prefs);
        self.insp.selection.height = height;
        self.insp.selection.rotate_deg = rotate_deg;
    }

    fn sel_arrange_page(
        &mut self,
        ui: &mut egui::Ui,
        summary: &SelectionSummary,
        acts: &mut Vec<SelAction>,
    ) {
        let n = summary.lights;
        let mut prefs = self.insp.prefs.selection.clone();
        let z = theme::zoom_of(ui);
        let gap = 5.0 * z;
        let avail = ui.available_width().min(panel_width(ui));
        let w = ((avail - 3.0 * gap) / 4.0).clamp(30.0, 64.0 * z);
        theme::toolbar(ui, |ui| {
            for (i, kind) in ShapeKind::ALL.iter().enumerate() {
                let r = theme::pad_button(
                    ui,
                    ("insp_sel_shape", i),
                    egui::vec2(w, w.max(44.0 * z)),
                    theme::PadState { active: prefs.shape == *kind, ..Default::default() },
                    theme::PadFace::Icon(kind.icon()),
                    kind.label(),
                );
                if r.on_hover_text(match kind {
                    ShapeKind::Line => "Lay the selection out in a straight run.",
                    ShapeKind::Arc => "Lay the selection out along an arc.",
                    ShapeKind::Circle => "Lay the selection out round a full circle.",
                    ShapeKind::Grid => "Lay the selection out in rows and columns.",
                })
                .clicked()
                {
                    prefs.shape = *kind;
                }
            }
        });
        ui.add_space(4.0);
        theme::kv_grid(ui, "insp_sel_arrange", |ui| match prefs.shape {
            ShapeKind::Line => {
                theme::label_dim(ui, "Heading");
                num(
                    ui,
                    egui::DragValue::new(&mut prefs.line_heading).speed(1.0).range(-360.0..=360.0).suffix("°"),
                    "Which way the run points: 90 runs across the stage, 0 runs upstage.",
                );
                ui.end_row();
                if prefs.line_fit {
                    theme::label_dim(ui, "Length");
                    num(
                        ui,
                        egui::DragValue::new(&mut prefs.line_length).speed(0.1).range(0.5..=30.0).suffix(" m"),
                        "The total length the run is fitted into.",
                    );
                } else {
                    theme::label_dim(ui, "Spacing");
                    num(
                        ui,
                        egui::DragValue::new(&mut prefs.line_spacing).speed(0.05).range(0.1..=10.0).suffix(" m"),
                        "The gap between neighbours along the run.",
                    );
                }
                ui.end_row();
            }
            ShapeKind::Arc => {
                theme::label_dim(ui, "Radius");
                num(
                    ui,
                    egui::DragValue::new(&mut prefs.arc_radius).speed(0.05).range(0.2..=30.0).suffix(" m"),
                    "How far the lights sit from the arc's centre.",
                );
                ui.end_row();
                theme::label_dim(ui, "Heading");
                num(
                    ui,
                    egui::DragValue::new(&mut prefs.arc_heading).speed(1.0).range(-360.0..=360.0).suffix("°"),
                    "Which way the middle of the arc faces.",
                );
                ui.end_row();
                theme::label_dim(ui, "Sweep");
                num(
                    ui,
                    egui::DragValue::new(&mut prefs.arc_sweep).speed(1.0).range(10.0..=360.0).suffix("°"),
                    "How much of the circle the arc covers.",
                );
                ui.end_row();
            }
            ShapeKind::Circle => {
                theme::label_dim(ui, "Radius");
                num(
                    ui,
                    egui::DragValue::new(&mut prefs.circle_radius).speed(0.05).range(0.2..=30.0).suffix(" m"),
                    "How far the lights sit from the circle's centre.",
                );
                ui.end_row();
                theme::label_dim(ui, "Start");
                num(
                    ui,
                    egui::DragValue::new(&mut prefs.circle_start).speed(1.0).range(-360.0..=360.0).suffix("°"),
                    "Where round the circle the first light goes.",
                );
                ui.end_row();
            }
            ShapeKind::Grid => {
                theme::label_dim(ui, "Columns");
                num(
                    ui,
                    egui::DragValue::new(&mut prefs.grid_cols).range(1..=32),
                    "How many lights each row holds.",
                );
                ui.end_row();
                theme::label_dim(ui, "Across");
                num(
                    ui,
                    egui::DragValue::new(&mut prefs.grid_dx).speed(0.05).range(0.1..=10.0).suffix(" m"),
                    "The gap between columns.",
                );
                ui.end_row();
                theme::label_dim(ui, "Upstage");
                num(
                    ui,
                    egui::DragValue::new(&mut prefs.grid_dz).speed(0.05).range(0.1..=10.0).suffix(" m"),
                    "The gap between rows.",
                );
                ui.end_row();
            }
        });
        match prefs.shape {
            ShapeKind::Line => {
                theme::toolbar(ui, |ui| {
                    if theme::chip(ui, "fit length", theme::ACCENT_SOFT, prefs.line_fit)
                        .on_hover_text("Fit the run into a total length instead of a fixed spacing.")
                        .clicked()
                    {
                        prefs.line_fit = !prefs.line_fit;
                    }
                });
                if prefs.line_fit {
                    if let Some(i) = chip_buttons(
                        ui,
                        "insp_sel_len",
                        &["2 m", "4 m", "6 m", "8 m"],
                        "A ready-made total length for the run.",
                    ) {
                        prefs.line_length = [2.0, 4.0, 6.0, 8.0][i];
                    }
                } else if let Some(i) = chip_buttons(
                    ui,
                    "insp_sel_space",
                    &["0.5", "1", "1.5", "2"],
                    "A ready-made spacing, in metres.",
                ) {
                    prefs.line_spacing = [0.5, 1.0, 1.5, 2.0][i];
                }
                if let Some(i) = chip_buttons(
                    ui,
                    "insp_sel_head",
                    &["along X", "along Z"],
                    "Point the run across the stage, or up and down it.",
                ) {
                    prefs.line_heading = if i == 0 { 90.0 } else { 0.0 };
                }
            }
            ShapeKind::Arc => {
                if let Some(i) = chip_buttons(
                    ui,
                    "insp_sel_arcr",
                    &["r 2", "r 3", "r 5"],
                    "A ready-made radius, in metres.",
                ) {
                    prefs.arc_radius = [2.0, 3.0, 5.0][i];
                }
                if let Some(i) = chip_buttons(
                    ui,
                    "insp_sel_sweep",
                    &["90°", "120°", "180°", "270°"],
                    "A ready-made sweep for the arc.",
                ) {
                    prefs.arc_sweep = [90.0, 120.0, 180.0, 270.0][i];
                }
            }
            ShapeKind::Circle => {
                if let Some(i) = chip_buttons(
                    ui,
                    "insp_sel_circr",
                    &["r 1.5", "r 2.5", "r 4"],
                    "A ready-made radius, in metres.",
                ) {
                    prefs.circle_radius = [1.5, 2.5, 4.0][i];
                }
            }
            ShapeKind::Grid => {
                if let Some(i) = chip_buttons(
                    ui,
                    "insp_sel_cols",
                    &["2", "3", "4", "6"],
                    "A ready-made number of columns.",
                ) {
                    prefs.grid_cols = [2, 3, 4, 6][i];
                }
            }
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            combo_label(ui, "Order");
            let w = combo_width(ui);
            egui::ComboBox::from_id_salt("insp_sel_order")
                .selected_text(prefs.order.label())
                .width(w)
                .show_ui(ui, |ui| {
                    for o in OrderBy::ALL {
                        ui.selectable_value(&mut prefs.order, o, o.label());
                    }
                })
                .response
                .on_hover_text("Which light takes which point in the shape.");
        });
        if matches!(prefs.shape, ShapeKind::Arc | ShapeKind::Circle) {
            ui.horizontal(|ui| {
                combo_label(ui, "Facing");
                let w = combo_width(ui);
                egui::ComboBox::from_id_salt("insp_sel_facing")
                    .selected_text(prefs.facing.label())
                    .width(w)
                    .show_ui(ui, |ui| {
                        for f in Facing::ALL {
                            ui.selectable_value(&mut prefs.facing, f, f.label());
                        }
                    })
                    .response
                    .on_hover_text(
                        "Whether the arranged lights turn to face the shape's centre, away from \
                         it, or keep the way they point now.",
                    );
            });
        }
        chips(
            ui,
            "insp_sel_centre",
            &mut prefs.centre_on_stage,
            &[(false, "centre on selection"), (true, "centre on stage")],
            "Where the shape is built: around the selection as it stands, or around the stage.",
        );
        theme::hint(ui, prefs.arrange_preview(n.max(1)));
        ui.add_space(2.0);
        let icon = prefs.shape.icon();
        if theme::gated(ui, gate(n, 2), |ui| {
            theme::wide_button(ui, Some(icon), "Arrange", theme::Tone::Accent)
        })
        .on_hover_text("Lay the selected lights out in this shape.")
        .clicked()
        {
            acts.push(SelAction::Arrange);
        }
        self.stash_prefs(prefs);
    }

    fn sel_aim_page(
        &mut self,
        ui: &mut egui::Ui,
        summary: &SelectionSummary,
        acts: &mut Vec<SelAction>,
    ) {
        let n = summary.lights;
        let (ge1, ge2) = (gate(n, 1), gate(n, 2));
        let mut prefs = self.insp.prefs.selection.clone();
        let heads = summary.heads;
        let sphere_on = self.transition.mode.uses_sphere();
        let chase_on = self.chase.enabled;

        theme::section(ui, "Point at");
        theme::toolbar(ui, |ui| {
            let label = fits_label(ui, "Stage centre");
            if theme::tool_button(
                ui,
                Icon::Target,
                label,
                "Point every selected light at the middle of the stage floor.",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::Aim(AimKind::StageCentre));
            }
            let label = fits_label(ui, "Selection centre");
            if theme::tool_button(
                ui,
                Icon::CentreStage,
                label,
                "Point every selected light at the middle of the selection itself.",
                ge2,
            )
            .clicked()
            {
                acts.push(SelAction::Aim(AimKind::Centroid));
            }
            let label = fits_label(ui, "Down");
            if theme::tool_button(ui, Icon::AimDown, label, "Point every selected light straight down.", ge1)
                .clicked()
            {
                acts.push(SelAction::AimDir(AimTarget::StraightDown));
            }
            let label = fits_label(ui, "Up");
            if theme::tool_button(ui, Icon::AimUp, label, "Point every selected light straight up.", ge1)
                .clicked()
            {
                acts.push(SelAction::AimDir(AimTarget::StraightUp));
            }
            let label = fits_label(ui, "Level");
            if theme::tool_button(
                ui,
                Icon::AimLevel,
                label,
                "Level every selected light out, keeping the way it faces.",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::AimDir(AimTarget::Level));
            }
            let label = fits_label(ui, "Transition sphere");
            if theme::tool_button(
                ui,
                Icon::Sphere,
                label,
                "Point the selection at the transition sphere on the stage.",
                if sphere_on { ge1 } else { Err("Turn on a sphere transition first.") },
            )
            .clicked()
            {
                acts.push(SelAction::Aim(AimKind::Transition));
            }
            let label = fits_label(ui, "Chase sphere");
            if theme::tool_button(
                ui,
                Icon::Sphere,
                label,
                "Point the selection at the chase sphere on the stage.",
                if chase_on { ge1 } else { Err("Turn the chase on first.") },
            )
            .clicked()
            {
                acts.push(SelAction::Aim(AimKind::Chase));
            }
        });

        ui.add_space(4.0);
        theme::card(ui, |ui| {
            theme::card_title(ui, "Custom point", |ui| {
                theme::help(ui, "Any point in the room. X is across the stage, Y is height, Z is upstage and downstage.");
            });
            theme::kv_grid(ui, "insp_sel_aim_pt", |ui| {
                for (label, value, hint) in [
                    ("X", &mut prefs.aim_point.x, "How far across the stage the target sits."),
                    ("Y", &mut prefs.aim_point.y, "How high the target sits."),
                    ("Z", &mut prefs.aim_point.z, "How far upstage the target sits."),
                ] {
                    theme::label_dim(ui, label);
                    num(
                        ui,
                        egui::DragValue::new(value).speed(0.05).fixed_decimals(2).suffix(" m"),
                        hint,
                    );
                    ui.end_row();
                }
            });
            theme::toolbar(ui, |ui| {
                let label = fits_label(ui, "Use selection centre");
                if theme::tool_button(
                    ui,
                    Icon::CentreStage,
                    label,
                    "Copy the selection's own centre into the point above.",
                    ge1,
                )
                .clicked()
                {
                    if let Some(c) = self.stage.selection_centroid() {
                        prefs.aim_point = c;
                    }
                }
            });
            if theme::gated(ui, ge1, |ui| {
                theme::wide_button(ui, Some(Icon::Target), "Aim here", theme::Tone::Accent)
            })
            .on_hover_text("Point every selected light at the custom point.")
            .clicked()
            {
                acts.push(SelAction::Aim(AimKind::Point));
            }
        });

        ui.add_space(4.0);
        theme::section(ui, "Spread");
        theme::toolbar(ui, |ui| {
            wrap_before(ui, 64.0 * theme::zoom_of(ui));
            num(
                ui,
                egui::DragValue::new(&mut prefs.fan_deg).range(5.0..=180.0).speed(1.0).suffix("°"),
                "How wide the fan opens.",
            );
            let label = fits_label(ui, "Fan");
            if theme::tool_button(
                ui,
                Icon::Fan,
                label,
                "Spread the selected lights' yaw evenly across this angle around their average, \
                 ordered left to right.",
                ge2,
            )
            .clicked()
            {
                acts.push(SelAction::Fan);
            }
            let label = fits_label(ui, "Cross");
            if theme::tool_button(
                ui,
                Icon::CrossAim,
                label,
                "Aim each light at the mirror image of its own position, for crossed washes.",
                ge2,
            )
            .clicked()
            {
                acts.push(SelAction::Cross);
            }
        });

        ui.add_space(4.0);
        let mut include = prefs.aim_heads;
        ui.checkbox(&mut include, "Include moving heads").on_hover_text(
            "Moving heads normally keep their mounting frame and are aimed with pan and tilt. \
             Tick this to re-point the base instead.",
        );
        prefs.aim_heads = include;
        if heads > 0 {
            theme::toolbar(ui, |ui| {
                let label = fits_label(ui, "Pan/Tilt → target");
                if theme::tool_button(
                    ui,
                    Icon::AimMover,
                    label,
                    "Work out pan and tilt for each selected moving head so its beam lands on the \
                     last target you picked, and write them to the programmer.",
                    ge1,
                )
                .clicked()
                {
                    acts.push(SelAction::AimHeads);
                }
                wrap_before_pill(ui, &format!("{heads} moving heads"));
                theme::pill(ui, &format!("{heads} moving heads"), theme::WARN)
                    .on_hover_text("How many moving heads the selection holds.");
            });
        } else if !prefs.aim_heads {
            theme::hint(ui, "Pars and bars only — moving heads are left alone.");
        }
        self.stash_prefs(prefs);
    }

    fn sel_hang_page(
        &mut self,
        ui: &mut egui::Ui,
        summary: &SelectionSummary,
        acts: &mut Vec<SelAction>,
    ) {
        let n = summary.lights;
        let mut prefs = self.insp.prefs.selection.clone();
        let mut target = self.insp.selection.hang_element;
        let elements: Vec<(ElementRef, String)> = (0..self.stage.towers.len())
            .map(ElementRef::Tower)
            .chain((0..self.stage.trusses.len()).map(ElementRef::Truss))
            .map(|r| (r, self.stage.element_name(r)))
            .collect();
        if elements.is_empty() {
            theme::hint(ui, "Add a tower or truss on the Build tab first.");
            return;
        }
        let faces: &'static [&'static str] =
            target.map_or(&["Top", "Under"], |r| self.stage.element_faces(r));
        if prefs.hang_face >= faces.len() {
            prefs.hang_face = faces.len() - 1;
        }
        let (free, per_face) =
            target.map_or((0, 0), |r| self.stage.element_free(r, prefs.hang_face));

        theme::section(ui, "Hang on");
        let label = target
            .map(|r| self.stage.element_name(r))
            .unwrap_or_else(|| "Pick an element".into());
        let w = combo_width(ui);
        egui::ComboBox::from_id_salt("insp_sel_hang_el")
            .selected_text(label)
            .width(w)
            .show_ui(ui, |ui| {
                for (r, name) in &elements {
                    let (f, t) = self.stage.element_free(*r, prefs.hang_face.min(1));
                    ui.selectable_value(&mut target, Some(*r), format!("{name} · {f}/{t} free"));
                }
            })
            .response
            .on_hover_text("Which tower or truss the selection goes onto.");
        let face_choices: Vec<(usize, &str)> =
            faces.iter().enumerate().map(|(i, f)| (i, *f)).collect();
        chips(
            ui,
            "insp_sel_face",
            &mut prefs.hang_face,
            &face_choices,
            "Which face of the element the lights clip onto.",
        );
        let fills: Vec<(HangFill, &str)> = HangFill::ALL.iter().map(|f| (*f, f.label())).collect();
        chips(
            ui,
            "insp_sel_fill",
            &mut prefs.hang_fill,
            &fills,
            "How the lights are spent across the free slots.",
        );
        ui.horizontal(|ui| {
            combo_label(ui, "Order");
            let w = combo_width(ui);
            egui::ComboBox::from_id_salt("insp_sel_order_hang")
                .selected_text(prefs.order.label())
                .width(w)
                .show_ui(ui, |ui| {
                    for o in OrderBy::ALL {
                        ui.selectable_value(&mut prefs.order, o, o.label());
                    }
                })
                .response
                .on_hover_text("Which light takes which slot.");
        });
        theme::toolbar(ui, |ui| {
            wrap_before_pill(ui, &format!("{free} free of {per_face}"));
            theme::pill(
                ui,
                &format!("{free} free of {per_face}"),
                if free >= n && n > 0 { theme::OK } else { theme::WARN },
            )
            .on_hover_text("How many slots on this face are still empty.");
        });
        ui.add_space(2.0);
        let ready = if n == 0 {
            Err("Select lights first, then pick a tower or truss.")
        } else if target.is_none() {
            Err("Pick a tower or truss to hang them on.")
        } else {
            Ok(())
        };
        if theme::gated(ui, ready, |ui| {
            theme::wide_button(
                ui,
                Some(Icon::Hook),
                &format!("Hang {n} light{}", if n == 1 { "" } else { "s" }),
                theme::Tone::Accent,
            )
        })
        .on_hover_text("Fill this face's free slots with the selection, in order.")
        .clicked()
        {
            acts.push(SelAction::Hang);
        }
        ui.add_space(4.0);
        let hung = if self.stage.selection_has_mounted() {
            Ok(())
        } else {
            Err("None of the selected lights are hung on anything.")
        };
        theme::toolbar(ui, |ui| {
            let label = fits_label(ui, "Detach");
            if theme::tool_button(
                ui,
                Icon::Detach,
                label,
                "Unhook the selected lights; they stay exactly where they hang.",
                hung,
            )
            .clicked()
            {
                acts.push(SelAction::Detach);
            }
            let label = fits_label(ui, "Re-seat");
            if theme::tool_button(
                ui,
                Icon::Reseat,
                label,
                "Re-glue every hung light to its slot, after a truss has been turned or moved by hand.",
                hung,
            )
            .clicked()
            {
                acts.push(SelAction::Reseat);
            }
        });
        self.insp.selection.hang_element = target;
        self.stash_prefs(prefs);
    }

    fn sel_clip_page(
        &mut self,
        ui: &mut egui::Ui,
        summary: &SelectionSummary,
        acts: &mut Vec<SelAction>,
    ) {
        let n = summary.lights;
        let (ge1, ge2) = (gate(n, 1), gate(n, 2));
        let mut prefs = self.insp.prefs.selection.clone();
        let clip_text = self
            .insp
            .selection
            .clip
            .as_ref()
            .map_or_else(|| "Clipboard empty".to_string(), |c| c.summary());
        let has_clip = self.insp.selection.clip.is_some();
        let alt = ui.input(|i| i.modifiers.alt);
        let copies = {
            let st = &self.stage;
            st.selected_fixtures()
                .iter()
                .any(|&fi| st.instances.iter().filter(|i| i.fixture == fi).count() > 1)
        };

        theme::section(ui, "Clipboard");
        theme::toolbar(ui, |ui| {
            let label = fits_label(ui, "Copy");
            if theme::tool_button(
                ui,
                Icon::Copy,
                label,
                "Copy the selected lights' position, rotation, size and opacity. ⌥-click also \
                 puts the JSON on the system clipboard.",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::Copy { os: alt });
            }
            let label = fits_label(ui, "Paste");
            if theme::tool_button(
                ui,
                Icon::Paste,
                label,
                "Apply the copied placement: one copied light goes onto every selected light; a \
                 copied group is re-laid around the current centre.",
                if has_clip { ge1 } else { Err("Copy a placement first.") },
            )
            .clicked()
            {
                acts.push(SelAction::Paste);
            }
            let label = fits_label(ui, "Match anchor");
            if theme::tool_button(
                ui,
                Icon::Match,
                label,
                "Give every selected light the rotation and size of the last one picked.",
                ge2,
            )
            .clicked()
            {
                acts.push(SelAction::Match);
            }
        });
        theme::toolbar(ui, |ui| {
            for (i, (label, on, hint)) in [
                ("position", &mut prefs.paste.position, "Paste where the copied lights sat."),
                ("rotation", &mut prefs.paste.rotation, "Paste the way the copied lights faced."),
                ("scale", &mut prefs.paste.scale, "Paste the copied lights' size."),
                ("opacity", &mut prefs.paste.opacity, "Paste the copied lights' housing opacity."),
            ]
            .into_iter()
            .enumerate()
            {
                let _ = i;
                let hit = theme::chip(ui, label, theme::ACCENT_SOFT, *on);
                if hit.on_hover_text(hint).clicked() {
                    *on = !*on;
                }
            }
        });
        theme::hint(ui, clip_text);

        ui.add_space(4.0);
        theme::section(ui, "Copies");
        theme::toolbar(ui, |ui| {
            let label = fits_label(ui, "Duplicate");
            if theme::tool_button(
                ui,
                Icon::Duplicate,
                label,
                "Copy the selected lights. Copies follow the same DMX channels — for fixtures \
                 wired to several physical units. (⌘D)",
                ge1,
            )
            .clicked()
            {
                acts.push(SelAction::Duplicate);
            }
            wrap_before(ui, 64.0 * theme::zoom_of(ui));
            num(
                ui,
                egui::DragValue::new(&mut prefs.dup_count).range(1..=16).prefix("× "),
                "How many copies each selected light makes.",
            );
        });
        theme::toolbar(ui, |ui| {
            wrap_before(ui, theme::tool_size(ui, Some("Step")).x);
            theme::label_dim(ui, "Step");
            wrap_before(ui, 64.0 * theme::zoom_of(ui));
            num(
                ui,
                egui::DragValue::new(&mut prefs.dup_offset.x).speed(0.05).suffix(" m"),
                "How far across the stage each copy steps.",
            );
            wrap_before(ui, 64.0 * theme::zoom_of(ui));
            num(
                ui,
                egui::DragValue::new(&mut prefs.dup_offset.z).speed(0.05).suffix(" m"),
                "How far upstage each copy steps.",
            );
        });
        theme::toolbar(ui, |ui| {
            let label = fits_label(ui, "Remove copy");
            if theme::tool_button(
                ui,
                Icon::Trash,
                label,
                "Delete the selected copies. The last copy of each fixture always stays. (⌫)",
                if copies {
                    Ok(())
                } else {
                    Err("These are the only copies — patch more fixtures instead.")
                },
            )
            .clicked()
            {
                acts.push(SelAction::RemoveCopy);
            }
        });
        self.stash_prefs(prefs);
    }

    // -------------------------------------------------- element editor ----

    /// The picked tower / truss: its grid, quick chips and footer.
    fn sel_element_editor(
        &mut self,
        ui: &mut egui::Ui,
        r: ElementRef,
        mounted: usize,
        slots: usize,
        acts: &mut Vec<SelAction>,
    ) {
        let title = self.stage.element_kind_label(r);
        let slot_text = format!("{mounted}/{slots} slots");
        let mut action = ElementAction::None;
        let mut place: Option<ElementSpot> = None;
        theme::card(ui, |ui| {
            theme::card_title(ui, title, |ui| {
                theme::pill(ui, &slot_text, theme::ACCENT_SOFT)
                    .on_hover_text("How many of this element's mount slots hold a light.");
            });
            match r {
                ElementRef::Tower(i) => self.sel_tower_grid(ui, i),
                ElementRef::Truss(i) => self.sel_truss_grid(ui, i),
            }
            let spots = ElementSpot::ALL.map(|s| s.label());
            if let Some(i) = chip_buttons(
                ui,
                "insp_sel_place",
                &spots,
                "Move this element onto a spot on the stage. Its height is kept.",
            ) {
                place = Some(ElementSpot::ALL[i]);
            }
            ui.add_space(2.0);
            let last = self.insp.selection.last_light_sel.len();
            theme::toolbar(ui, |ui| {
                let label = fits_label(ui, "Select mounted lights");
                if theme::tool_button(
                    ui,
                    Icon::Hook,
                    label,
                    "Select every light hung on this element.",
                    if mounted > 0 { Ok(()) } else { Err("Nothing is hung on it yet.") },
                )
                .clicked()
                {
                    action = ElementAction::SelectMounted;
                }
                if theme::tool_button(
                    ui,
                    Icon::Hook,
                    Some(&format!("Hang last {last} lights")),
                    "Hang the lights you had selected before picking this element on it.",
                    if last > 0 {
                        Ok(())
                    } else {
                        Err("Select some lights, then pick a tower or truss.")
                    },
                )
                .clicked()
                {
                    action = ElementAction::HangLast;
                }
            });
            theme::toolbar(ui, |ui| {
                let label = fits_label(ui, "Mirror X");
                if theme::tool_button(
                    ui,
                    Icon::MirrorX,
                    label,
                    "Reflect this element across the stage's centre line, left ↔ right.",
                    Ok(()),
                )
                .clicked()
                {
                    action = ElementAction::MirrorX;
                }
                let label = fits_label(ui, "Mirror Z");
                if theme::tool_button(
                    ui,
                    Icon::MirrorZ,
                    label,
                    "Reflect this element across the stage's centre line, front ↔ back.",
                    Ok(()),
                )
                .clicked()
                {
                    action = ElementAction::MirrorZ;
                }
                wrap_before(ui, theme::tool_size(ui, Some("Delete")).x);
                if theme::danger_button(ui, "Delete")
                    .on_hover_text("Remove this element. Its lights stay where they hang.")
                    .clicked()
                {
                    action = ElementAction::Delete;
                }
            });
        });
        if let Some(spot) = place {
            acts.push(SelAction::Place(spot));
        }
        if action != ElementAction::None {
            acts.push(SelAction::Element(action));
        }
    }

    fn sel_tower_grid(&mut self, ui: &mut egui::Ui, ti: usize) {
        let Some(tw) = self.stage.towers.get(ti) else { return };
        let (mut x, mut z, mut yaw, mut height, mut width) =
            (tw.pos.x, tw.pos.z, tw.yaw_deg, tw.height, tw.width);
        let (x0, z0, yaw0, h0, w0) = (x, z, yaw, height, width);
        slider_room(ui);
        theme::kv_grid(ui, "insp_sel_tower", |ui| {
            theme::label_dim(ui, "X");
            num(
                ui,
                egui::DragValue::new(&mut x).speed(0.05).fixed_decimals(2).suffix(" m"),
                "Where the tower stands across the stage.",
            );
            ui.end_row();
            theme::label_dim(ui, "Z");
            num(
                ui,
                egui::DragValue::new(&mut z).speed(0.05).fixed_decimals(2).suffix(" m"),
                "How far upstage the tower stands.",
            );
            ui.end_row();
            theme::label_dim(ui, "Yaw");
            num(
                ui,
                egui::DragValue::new(&mut yaw).speed(0.5).fixed_decimals(1).suffix("°"),
                "Which way the crossbar points; the hung lights turn with it.",
            );
            ui.end_row();
            theme::label_dim(ui, "Height");
            ui.add(egui::Slider::new(&mut height, 1.2..=6.0).fixed_decimals(2))
                .on_hover_text("How high the crossbar sits, in metres.");
            ui.end_row();
            theme::label_dim(ui, "Width");
            ui.add(egui::Slider::new(&mut width, 0.8..=4.0).fixed_decimals(2))
                .on_hover_text("How wide the crossbar is, in metres.");
            ui.end_row();
        });
        if let Some(i) = chip_buttons(
            ui,
            "insp_sel_tw_h",
            &["2 m", "3 m", "4 m"],
            "A standard crossbar height.",
        ) {
            height = [2.0, 3.0, 4.0][i];
        }
        if let Some(i) =
            chip_buttons(ui, "insp_sel_tw_y", &["face 0°", "face 90°"], "A square-on heading.")
        {
            yaw = [0.0, 90.0][i];
        }
        if (x, z, yaw, height, width) == (x0, z0, yaw0, h0, w0) {
            return;
        }
        let pre = self.stage.edit_pre();
        self.stage.undo_checkpoint(&mut self.insp.selection.gesture_open, pre);
        if let Some(tw) = self.stage.towers.get_mut(ti) {
            tw.pos.x = x;
            tw.pos.z = z;
            tw.yaw_deg = yaw;
            tw.height = height;
            tw.width = width;
        }
        self.stage.apply_tower_edit(&self.patch, ti, yaw - yaw0);
    }

    fn sel_truss_grid(&mut self, ui: &mut egui::Ui, ti: usize) {
        let Some(tr) = self.stage.trusses.get(ti) else { return };
        let kind = tr.kind;
        let straight = kind == stage::TrussKind::Straight;
        let (mut x, mut z, mut y, mut yaw) = (tr.pos.x, tr.pos.z, tr.pos.y, tr.yaw_deg);
        let (mut pitch, mut roll) = (tr.pitch_deg, tr.roll_deg);
        let (mut length, mut radius, mut arc, mut grounded) =
            (tr.length, tr.radius, tr.arc_deg, tr.grounded);
        let start = (x, z, y, yaw, pitch, roll, length, radius, arc, grounded);
        let prev_per_face = self.stage.truss_per_face(ti);
        slider_room(ui);
        theme::kv_grid(ui, "insp_sel_truss", |ui| {
            theme::label_dim(ui, "X");
            num(
                ui,
                egui::DragValue::new(&mut x).speed(0.05).fixed_decimals(2).suffix(" m"),
                "Where the run sits across the stage.",
            );
            ui.end_row();
            theme::label_dim(ui, "Z");
            num(
                ui,
                egui::DragValue::new(&mut z).speed(0.05).fixed_decimals(2).suffix(" m"),
                "How far upstage the run sits.",
            );
            ui.end_row();
            theme::label_dim(ui, "Height");
            ui.add(egui::Slider::new(&mut y, 0.5..=8.0).fixed_decimals(2))
                .on_hover_text("How high the run hangs, in metres.");
            ui.end_row();
            theme::label_dim(ui, if straight { "Heading" } else { "Start" });
            num(
                ui,
                egui::DragValue::new(&mut yaw).speed(0.5).fixed_decimals(1).suffix("°"),
                if straight {
                    "Which way the run points; the hung lights turn with it."
                } else {
                    "Where round the circle the arc begins."
                },
            );
            ui.end_row();
            theme::label_dim(ui, "Pitch");
            num(
                ui,
                egui::DragValue::new(&mut pitch)
                    .speed(0.5)
                    .fixed_decimals(1)
                    .range(-180.0..=180.0)
                    .suffix("°"),
                "Tilt the whole run out of the horizontal: 0 is flat overhead, 90 stands it on \
                 edge against a wall.",
            );
            ui.end_row();
            theme::label_dim(ui, "Roll");
            num(
                ui,
                egui::DragValue::new(&mut roll)
                    .speed(0.5)
                    .fixed_decimals(1)
                    .range(-180.0..=180.0)
                    .suffix("°"),
                "Which way a pitched run leans.",
            );
            ui.end_row();
            if straight {
                theme::label_dim(ui, "Length");
                ui.add(egui::Slider::new(&mut length, 0.5..=6.0).fixed_decimals(2))
                    .on_hover_text("How long the run is, in metres. Shortening it can drop lights off the end.");
                ui.end_row();
            } else {
                theme::label_dim(ui, "Radius");
                ui.add(egui::Slider::new(&mut radius, 0.5..=8.0).fixed_decimals(2))
                    .on_hover_text("How far the arc sits from its centre, in metres.");
                ui.end_row();
                theme::label_dim(ui, "Arc");
                ui.add(egui::Slider::new(&mut arc, 15.0..=360.0).fixed_decimals(0))
                    .on_hover_text("How much of the circle the run covers.");
                ui.end_row();
            }
            theme::label_dim(ui, "Grounded");
            ui.checkbox(&mut grounded, "")
                .on_hover_text("Draw support legs down to the floor at both ends.");
            ui.end_row();
        });
        if straight {
            if let Some(i) = chip_buttons(
                ui,
                "insp_sel_tr_len",
                &["0.5 m", "1 m", "1.5 m", "2 m", "3 m"],
                "A standard F34 run length.",
            ) {
                length = [0.5, 1.0, 1.5, 2.0, 3.0][i];
            }
        } else {
            if let Some(i) = chip_buttons(
                ui,
                "insp_sel_tr_arc",
                &["90°", "180°", "270°", "360°"],
                "A standard arc.",
            ) {
                arc = [90.0, 180.0, 270.0, 360.0][i];
            }
            if let Some(i) = chip_buttons(
                ui,
                "insp_sel_tr_rad",
                &["r 1", "r 2", "r 3", "r 4"],
                "A standard radius, in metres.",
            ) {
                radius = [1.0, 2.0, 3.0, 4.0][i];
            }
        }
        if let Some(i) =
            chip_buttons(ui, "insp_sel_tr_yaw", &["face 0°", "face 90°"], "A square-on heading.")
        {
            yaw = [0.0, 90.0][i];
        }
        if (x, z, y, yaw, pitch, roll, length, radius, arc, grounded) == start {
            return;
        }
        let pre = self.stage.edit_pre();
        self.stage.undo_checkpoint(&mut self.insp.selection.gesture_open, pre);
        if let Some(tr) = self.stage.trusses.get_mut(ti) {
            tr.pos = v3(x, y, z);
            tr.yaw_deg = yaw;
            tr.pitch_deg = pitch;
            tr.roll_deg = roll;
            tr.length = length;
            tr.radius = radius;
            tr.arc_deg = arc;
            tr.grounded = grounded;
        }
        let fell = self.stage.apply_truss_edit(&self.patch, ti, yaw - start.3, prev_per_face);
        if fell > 0 {
            let name = self.stage.element_name(ElementRef::Truss(ti));
            self.log.push(format!(
                "{fell} light{} fell off {name} — it got shorter",
                if fell == 1 { "" } else { "s" }
            ));
        }
    }

    /// Store the page's edited prefs, marking the file dirty only on a real
    /// change.
    fn stash_prefs(&mut self, prefs: SelectionPrefs) {
        if self.insp.prefs.selection != prefs {
            self.insp.prefs.selection = prefs;
            self.insp.mark_dirty();
        }
    }

    // ----------------------------------------------------------- apply ----

    /// Resolve a page's aim choice into a world target.
    fn resolve_aim(&self, kind: AimKind) -> AimTarget {
        match kind {
            AimKind::StageCentre => AimTarget::StageCentre,
            AimKind::Centroid => AimTarget::Centroid,
            AimKind::Point => AimTarget::Point(self.insp.prefs.selection.aim_point),
            AimKind::Transition => AimTarget::Point(self.transition.sphere.pos),
            AimKind::Chase => AimTarget::Point(self.chase.sphere.pos),
        }
    }

    /// The one place this tab mutates the stage or the programmer. Every arm
    /// logs one human sentence.
    fn apply_sel_action(&mut self, a: SelAction) {
        let prefs = self.insp.prefs.selection.clone();
        match a {
            SelAction::Select(kind) => self.apply_select(kind),
            SelAction::Nudge(delta, dyaw) => {
                if self.stage.selection.is_empty() {
                    if self.stage.nudge_element(&self.patch, delta, dyaw) {
                        let r = self
                            .stage
                            .sel_tower
                            .map(ElementRef::Tower)
                            .or(self.stage.sel_truss.map(ElementRef::Truss));
                        if let Some(r) = r {
                            let name = self.stage.element_name(r);
                            self.log.push(format!("Nudged {name}"));
                        }
                    }
                } else {
                    let out = self.stage.nudge_selection(&self.patch, delta, dyaw, 0.0);
                    if out.changed > 0 {
                        self.log.push(format!("Nudged {} lights", out.changed));
                    }
                }
            }
            SelAction::Align(axis, mode) => {
                let out = self.stage.align_selection(&self.patch, axis, mode);
                if out.changed > 0 {
                    self.log.push(format!(
                        "Aligned {} lights on {} ({})",
                        out.changed,
                        axis.label(),
                        mode.label()
                    ));
                }
            }
            SelAction::Distribute(axis, mode) => {
                let out = self.stage.distribute_selection(&self.patch, axis, mode);
                if out.changed > 0 {
                    self.log
                        .push(format!("Distributed {} lights along {}", out.changed, axis.label()));
                } else {
                    self.log.push("Already even".into());
                }
            }
            SelAction::CentreStage => {
                let out = self.stage.centre_selection_on_stage(&self.patch);
                if out.changed > 0 {
                    self.log.push(format!("Centred {} lights on the stage", out.changed));
                }
            }
            SelAction::Arrange => {
                let n = self.stage.selection.len();
                let shape = prefs.arrange_shape(n);
                let centre = match self.stage.selection_centroid() {
                    Some(c) if prefs.centre_on_stage => v3(0.0, c.y, 0.0),
                    Some(c) => c,
                    None => return,
                };
                let out = self.stage.arrange_selection(&self.patch, shape, prefs.order, centre);
                if out.changed > 0 {
                    self.log.push(format!(
                        "Arranged {} lights in a {}",
                        out.changed,
                        shape.name()
                    ));
                }
            }
            SelAction::Mirror(plane) => {
                let out = self.stage.mirror_selection(
                    &self.patch,
                    plane,
                    prefs.mirror_pivot,
                    prefs.mirror_copy,
                );
                if out.changed > 0 {
                    self.log.push(format!(
                        "Mirrored {} lights across {}{}",
                        out.changed,
                        plane.label(),
                        if prefs.mirror_copy { " as copies" } else { "" }
                    ));
                    self.sync_selection_units();
                    self.sel_fixture = self.stage.last_selected;
                }
            }
            SelAction::Rotate(deg) => {
                let out = self.stage.rotate_selection(&self.patch, deg);
                if out.changed > 0 {
                    self.log.push(format!("Rotated {} lights by {deg}°", out.changed));
                }
            }
            SelAction::Spread(k) => {
                let out = self.stage.scale_spread(&self.patch, k);
                if out.changed > 0 {
                    self.log.push(format!("Spread {} lights ×{k:.2}", out.changed));
                }
            }
            SelAction::Aim(kind) => {
                let target = self.resolve_aim(kind);
                self.insp.prefs.selection.aim_kind = kind;
                self.insp.selection.last_aim = Some(target);
                self.insp.mark_dirty();
                self.apply_aim(target, prefs.aim_heads);
            }
            SelAction::AimDir(target) => {
                self.insp.selection.last_aim = Some(target);
                self.apply_aim(target, prefs.aim_heads);
            }
            SelAction::AimHeads => self.apply_aim_heads(),
            SelAction::Fan => {
                let out = self.stage.fan_selection(&self.patch, prefs.fan_deg);
                if out.changed > 0 {
                    self.log
                        .push(format!("Fanned {} lights across {}°", out.changed, prefs.fan_deg));
                }
            }
            SelAction::Cross => {
                let set = self.settings.clone();
                let out = self.stage.cross_aim_selection(&self.patch, &set);
                if out.changed > 0 {
                    self.log.push(format!("Crossed {} lights", out.changed));
                }
            }
            SelAction::Height(y) => {
                let out = self.stage.set_selection_height(&self.patch, y);
                if out.changed > 0 {
                    self.insp.selection.height = y;
                    self.log.push(format!("Set height {y:.2} m on {} lights", out.changed));
                }
            }
            SelAction::Snap => {
                let step = prefs.snap_step();
                let out = self.stage.snap_selection_to_grid(&self.patch, step);
                if out.changed > 0 {
                    self.log.push(format!("Snapped {} lights to a {step} m grid", out.changed));
                }
            }
            SelAction::Stagger => {
                let out = self.stage.stagger_selection(
                    &self.patch,
                    Axis::Y,
                    prefs.stagger,
                    prefs.order,
                );
                if out.changed > 0 {
                    self.log.push(format!("Staggered {} lights", out.changed));
                }
            }
            SelAction::Scatter => {
                let seed = self.stage.instances.len() as u32 ^ 0x9E37;
                let out = self.stage.scatter_selection(&self.patch, prefs.scatter_m, seed);
                if out.changed > 0 {
                    self.log.push(format!("Scattered {} lights", out.changed));
                }
            }
            SelAction::Hang => {
                let Some(target) = self.insp.selection.hang_element else { return };
                let faces = self.stage.element_faces(target).len();
                let face = prefs.hang_face.min(faces.saturating_sub(1));
                let n = self.stage.selection.len();
                let out =
                    self.stage.hang_selection(&self.patch, target, face, prefs.hang_fill, prefs.order);
                let name = self.stage.element_name(target);
                let face_name = self.stage.element_faces(target)[face];
                if out.changed > 0 {
                    let mut line = format!(
                        "Hung {} of {n} lights on {name} ({})",
                        out.changed,
                        face_name.to_lowercase()
                    );
                    if out.skipped > 0 {
                        line.push_str(&format!(", {} left over — no free slots", out.skipped));
                    }
                    self.log.push(line);
                } else {
                    self.log.push(format!("No free slots on {name}"));
                }
            }
            SelAction::Detach => {
                let out = self.stage.detach_selection(&self.patch);
                if out.changed > 0 {
                    self.log.push(format!("Detached {} lights", out.changed));
                }
            }
            SelAction::Reseat => {
                let out = self.stage.reseat_selection(&self.patch);
                if out.changed > 0 {
                    self.log.push(format!("Re-seated {} lights", out.changed));
                }
            }
            SelAction::Copy { os } => {
                let clip = self.stage.copy_transform(&self.patch);
                if let Some(clip) = clip {
                    let n = clip.items.len();
                    if os {
                        if let Ok(json) = serde_json::to_string(&clip) {
                            self.log.push("Copied the placement to the clipboard".into());
                            let text = json;
                            self.insp.selection.clip = Some(clip);
                            self.insp.selection.pending_clipboard = Some(text);
                            return;
                        }
                    }
                    self.insp.selection.clip = Some(clip);
                    self.log.push(format!("Copied the placement of {n} lights"));
                }
            }
            SelAction::Paste => {
                let Some(clip) = self.insp.selection.clip.clone() else { return };
                let out = self.stage.paste_transform(&self.patch, &clip, prefs.paste);
                if out.changed > 0 {
                    self.log
                        .push(format!("Pasted {} onto {} lights", prefs.paste.words(), out.changed));
                }
            }
            SelAction::Match => {
                let out = self.stage.match_anchor(&self.patch);
                if out.changed > 0 {
                    self.log.push(format!("Matched {} lights to the anchor", out.changed));
                }
            }
            SelAction::Duplicate => {
                let out = self.stage.duplicate_selection_n(
                    &self.patch,
                    prefs.dup_count.max(1),
                    prefs.dup_offset,
                );
                if out.changed > 0 {
                    self.log.push(format!("Duplicated lights ×{}", prefs.dup_count.max(1)));
                    self.sync_selection_units();
                    self.sel_fixture = self.stage.last_selected;
                }
            }
            SelAction::RemoveCopy => {
                self.stage.delete_selection(&self.patch);
                self.log.push("Removed the selected copies".into());
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
            }
            SelAction::Place(spot) => {
                let Some(r) = self.picked_element() else { return };
                let set = self.settings.clone();
                if self.stage.place_element(&self.patch, r, spot, &set) {
                    let name = self.stage.element_name(r);
                    self.log.push(format!("Moved {name} to {}", spot.label().to_lowercase()));
                }
            }
            SelAction::Element(action) => self.apply_element_action(action),
            SelAction::SetPitch(v) => {
                let out = self.stage.set_selection_rotation(&self.patch, None, Some(v), None);
                if out.changed > 0 {
                    self.log.push(format!("Set pitch {v:.0} degrees on {} lights", out.changed));
                }
            }
            SelAction::Reset(what) => {
                let set = self.settings.clone();
                let out = match what {
                    ResetWhat::Axis(axis) => match self.stage.selection_centroid() {
                        Some(mut c) => {
                            let v = if axis == Axis::Y { set.default_height } else { 0.0 };
                            axis.set(&mut c, v);
                            self.stage.set_selection_position(&self.patch, c)
                        }
                        None => return,
                    },
                    ResetWhat::Position => self
                        .stage
                        .set_selection_position(&self.patch, v3(0.0, set.default_height, 0.0)),
                    ResetWhat::Yaw => self.stage.set_selection_rotation(
                        &self.patch,
                        Some(set.default_yaw),
                        None,
                        None,
                    ),
                    ResetWhat::Pitch => self.stage.set_selection_rotation(
                        &self.patch,
                        None,
                        Some(set.default_pitch),
                        None,
                    ),
                    ResetWhat::Roll => {
                        self.stage.set_selection_rotation(&self.patch, None, None, Some(0.0))
                    }
                    ResetWhat::Rotation => self.stage.reset_selection_rotation(&self.patch, &set),
                    ResetWhat::Scale => self.stage.set_selection_scale(&self.patch, 1.0),
                    ResetWhat::Opacity => self.stage.set_selection_opacity(&self.patch, 1.0),
                };
                if out.changed > 0 {
                    self.log.push(format!("Reset {} on {} lights", what.word(), out.changed));
                }
            }
        }
    }

    /// The tower or truss the stage has picked, if it is still there.
    fn picked_element(&self) -> Option<ElementRef> {
        self.stage
            .sel_tower
            .filter(|&i| i < self.stage.towers.len())
            .map(ElementRef::Tower)
            .or(self.stage.sel_truss.filter(|&i| i < self.stage.trusses.len()).map(ElementRef::Truss))
    }

    fn apply_aim(&mut self, target: AimTarget, include_heads: bool) {
        let set = self.settings.clone();
        let out = self.stage.aim_selection(&self.patch, &set, target, include_heads);
        if out.changed == 0 && out.skipped == 0 {
            return;
        }
        let mut line = format!("Aimed {} lights at {}", out.changed, target.name());
        if out.skipped > 0 {
            line.push_str(&format!(" ({} moving heads skipped)", out.skipped));
        }
        self.log.push(line);
    }

    /// Solve pan and tilt for every selected moving head so its beam lands
    /// on the last chosen target, and write them to the programmer.
    fn apply_aim_heads(&mut self) {
        use crate::showbuddy::Role;
        let target = self
            .insp
            .selection
            .last_aim
            .unwrap_or_else(|| self.resolve_aim(self.insp.prefs.selection.aim_kind));
        let point = match target {
            AimTarget::Point(p) => p,
            AimTarget::StageCentre => v3(0.0, self.settings.stage_h, 0.0),
            AimTarget::Centroid => match self.stage.selection_centroid() {
                Some(c) => c,
                None => return,
            },
            _ => {
                self.log.push("Pick a point to aim the heads at first".into());
                return;
            }
        };
        let orig = *self.net.dmx.lock();
        let mut buf = orig;
        let mut done = 0;
        let mut misses: Vec<String> = Vec::new();
        let sel = self.stage.sorted_selection(&self.patch, OrderBy::Selection);
        for i in sel {
            let inst = &self.stage.instances[i];
            let Some(f) = self.patch.fixtures.get(inst.fixture) else { continue };
            if !matches!(classify(f), Archetype::MovingPar | Archetype::Beam) {
                continue;
            }
            let Some((pan, tilt)) =
                stage::aim_pan_tilt(&inst.t, f.pan_range, f.tilt_range, point)
            else {
                misses.push(f.display.clone());
                continue;
            };
            let base = f.from as usize - 1;
            let mut write = |coarse: Role, fine: Role, v: f32| {
                let raw = (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
                let has_fine = f.channels.iter().any(|c| c.role() == fine);
                for (k, ch) in f.channels.iter().enumerate() {
                    let slot = base + k;
                    if slot >= crate::net::DMX_SLOTS {
                        continue;
                    }
                    if ch.role() == coarse {
                        buf[slot] = if has_fine {
                            (raw >> 8) as u8
                        } else {
                            (v.clamp(0.0, 1.0) * 255.0).round() as u8
                        };
                    } else if ch.role() == fine {
                        buf[slot] = (raw & 0xFF) as u8;
                    }
                }
            };
            write(Role::Pan, Role::PanFine, pan);
            write(Role::Tilt, Role::TiltFine, tilt);
            done += 1;
        }
        if done > 0 {
            let fade = self.transition.fade(crate::transition::TransitionTarget::ChannelJump);
            self.commit_channel_edit(buf, &orig, fade);
            self.undo.note_input();
            self.log.push(format!("Aimed {done} moving heads at {} (pan/tilt)", target.name()));
        }
        for display in misses {
            self.log.push(format!("{display}: target out of pan/tilt range"));
        }
    }

    fn apply_element_action(&mut self, action: ElementAction) {
        let Some(r) = self.picked_element() else { return };
        let name = self.stage.element_name(r);
        match action {
            ElementAction::None => {}
            ElementAction::Delete => {
                match r {
                    ElementRef::Tower(i) => self.stage.delete_tower(&self.patch, i),
                    ElementRef::Truss(i) => self.stage.delete_truss(&self.patch, i),
                }
                self.log.push(format!("Deleted {name}"));
            }
            ElementAction::SelectMounted => {
                let n = self.stage.select_mounted(r);
                if n > 0 {
                    self.sync_selection_units();
                    self.sel_fixture = self.stage.last_selected;
                    self.log.push(format!("Selected {n} lights on {name}"));
                }
            }
            ElementAction::HangLast => {
                let last: Vec<usize> = self
                    .insp
                    .selection
                    .last_light_sel
                    .iter()
                    .copied()
                    .filter(|&i| i < self.stage.instances.len())
                    .collect();
                if last.is_empty() {
                    return;
                }
                let n = last.len();
                self.stage.selection = last.into_iter().collect();
                self.stage.sel_tower = None;
                self.stage.sel_truss = None;
                let prefs = &self.insp.prefs.selection;
                let faces = self.stage.element_faces(r).len();
                let face = prefs.hang_face.min(faces.saturating_sub(1));
                let (fill, order) = (prefs.hang_fill, prefs.order);
                let out = self.stage.hang_selection(&self.patch, r, face, fill, order);
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
                let mut line = format!("Hung {} of {n} lights on {name}", out.changed);
                if out.skipped > 0 {
                    line.push_str(&format!(", {} left over — no free slots", out.skipped));
                }
                self.log.push(line);
            }
            ElementAction::MirrorX | ElementAction::MirrorZ => {
                let plane = if action == ElementAction::MirrorX {
                    MirrorPlane::X
                } else {
                    MirrorPlane::Z
                };
                if self.stage.mirror_element(&self.patch, r, plane) {
                    self.log.push(format!("Mirrored {name} across {}", plane.label()));
                }
            }
        }
    }

    /// Every Select button lands here, and every path ends the way
    /// `central.rs` does.
    fn apply_select(&mut self, kind: SelectKind) {
        let mut line: Option<String> = None;
        match kind {
            SelectKind::All => {
                if !self.stage.all_selected() && self.stage.select_all_fixtures() {
                    line = Some(format!("Selected all {} lights", self.stage.selection.len()));
                }
            }
            SelectKind::None => self.stage.clear_selection(),
            SelectKind::Invert => self.stage.invert_selection(),
            SelectKind::SameType => {
                if let Some(fi) = self.stage.last_selected.filter(|&fi| fi < self.patch.fixtures.len())
                {
                    self.stage.select_same_type(&self.patch, fi);
                }
            }
            SelectKind::SameKind => self.stage.select_same_archetype(&self.patch),
            SelectKind::Copies => {
                if let Some(fi) = self.stage.last_selected.filter(|&fi| fi < self.patch.fixtures.len())
                {
                    self.stage.select_fixture(fi, false);
                }
            }
            SelectKind::Unmounted => self.stage.select_unmounted(),
            SelectKind::OnElement(r) => {
                self.stage.select_mounted(r);
            }
            SelectKind::Element(r) => self.stage.select_element(r),
            SelectKind::EveryNth => {
                let n = self.insp.prefs.selection.every_n.max(2);
                let off = self.insp.selection.every_off;
                self.stage.select_every_nth(&self.patch, n, off);
                self.insp.selection.every_off = (off + 1) % n;
            }
            SelectKind::Kinds(kinds, additive) => {
                self.stage.select_by_archetype(&self.patch, kinds, additive)
            }
            SelectKind::Group(gi, additive) => {
                if let Some(g) = self.groups.get(gi) {
                    let name = g.name.clone();
                    let members: Vec<usize> = g.fixtures.clone();
                    if !additive {
                        self.stage.clear_selection();
                    }
                    for fi in members {
                        if fi < self.patch.fixtures.len() {
                            self.stage.select_fixture(fi, true);
                        }
                    }
                    line = Some(format!("Selected group '{name}'"));
                }
            }
            SelectKind::Anchor(fi) => {
                self.stage.last_selected = Some(fi);
            }
            SelectKind::Drop(i) => {
                self.stage.selection.remove(&i);
            }
        }
        self.sync_selection_units();
        self.sel_fixture = self.stage.last_selected;
        if let Some(line) = line {
            self.log.push(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::DMX_SLOTS;
    use crate::stage::headless::{render_frames, save};
    use crate::ui::inspector::InspectorTab;

    fn lit(pixels: &[u8]) -> usize {
        pixels.chunks(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120).count()
    }

    #[test]
    fn prefs_round_trip_and_default() {
        let d: SelectionPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(d, SelectionPrefs::default());
        assert_eq!(d.nudge_step(), 0.1);
        assert_eq!(d.rot_step(), 15.0);
        assert_eq!(d.snap_step(), 0.5);
        let mut p = SelectionPrefs::default();
        p.tool = SelTool::Hang;
        p.shape = ShapeKind::Grid;
        p.aim_point = v3(1.0, 2.0, 3.0);
        let back: SelectionPrefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        // A stale index never panics the chip rows.
        let mut wild = SelectionPrefs { nudge_step_i: 99, rot_step_i: 99, snap_i: 99, ..p };
        assert_eq!(wild.nudge_step(), 1.0);
        assert_eq!(wild.rot_step(), 90.0);
        assert_eq!(wild.snap_step(), 1.0);
        wild.line_fit = true;
        wild.line_length = 4.0;
        match wild.arrange_shape(5) {
            ArrangeShape::Grid { cols, .. } => assert_eq!(cols, 4),
            other => panic!("expected a grid, got {other:?}"),
        }
        wild.shape = ShapeKind::Line;
        match wild.arrange_shape(5) {
            ArrangeShape::Line { spacing, .. } => assert!((spacing - 1.0).abs() < 1e-6),
            other => panic!("expected a line, got {other:?}"),
        }
        // One light cannot be fitted to a length; it keeps the spacing.
        match wild.arrange_shape(1) {
            ArrangeShape::Line { spacing, .. } => assert!((spacing - 1.0).abs() < 1e-6),
            other => panic!("expected a line, got {other:?}"),
        }
    }

    #[test]
    fn tool_metadata_is_unique() {
        for (i, a) in SelTool::ALL.iter().enumerate() {
            assert_eq!(a.index(), i);
            for b in &SelTool::ALL[i + 1..] {
                assert_ne!(a.label(), b.label());
                assert!(a.icon() != b.icon());
                assert_ne!(a.hint(), b.hint());
            }
            assert!(a.hint().ends_with('.'));
        }
        for (i, a) in ShapeKind::ALL.iter().enumerate() {
            for b in &ShapeKind::ALL[i + 1..] {
                assert_ne!(a.label(), b.label());
                assert!(a.icon() != b.icon());
            }
        }
        assert_eq!(gate(0, 1), Err(NEED_ONE));
        assert_eq!(gate(1, 2), Err(NEED_TWO));
        assert_eq!(gate(2, 3), Err(NEED_THREE));
        assert_eq!(gate(3, 3), Ok(()));
    }

    /// The whole tab, on the real rig, at the panel's 230 px minimum: one
    /// light, several lights, each tool page, a picked truss, nothing
    /// selected, and zoom 2. Written to `target/selection_*.png`.
    #[test]
    fn selection_tab_renders_headless() {
        let mut app = crate::app::App::new();
        if app.patch.fixtures.is_empty() {
            eprintln!("no patch — skipping");
            return;
        }
        // Nothing this test does may touch the project's stage_layout.json.
        let temp = std::env::temp_dir().join("dmxpress_selection_tab_test.json");
        app.stage.layout_path = temp.clone();
        *app.net.dmx.lock() = Frame([170u8; DMX_SLOTS]);
        app.insp.prefs.tab = InspectorTab::Selection;
        app.insp.prefs.width = Some(230.0);
        app.collapsed.remove("inspector");
        app.collapsed.insert("channels");
        app.collapsed.insert("fixtures");
        app.show_log = false;
        app.show_osc = false;

        let size = [1000, 900];
        let shot = |app: &mut crate::app::App, name: &str| -> bool {
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

        // (d) nothing selected.
        app.stage.clear_selection();
        if !shot(&mut app, "selection_empty") {
            let _ = std::fs::remove_file(&temp);
            return;
        }
        // (a) one light.
        app.stage.select_fixture(0, false);
        shot(&mut app, "selection_one");
        // (b) several lights, every tool page.
        for i in 0..6.min(app.patch.fixtures.len()) {
            app.stage.select_fixture(i, true);
        }
        for tool in SelTool::ALL {
            app.insp.prefs.selection.tool = tool;
            shot(&mut app, &format!("selection_{}", tool.label().to_lowercase()));
        }
        // The Arrange page at zoom 2, after an arc has been laid out.
        app.stage.arrange_selection(
            &app.patch,
            ArrangeShape::Arc {
                radius: 3.0,
                heading_deg: 0.0,
                sweep_deg: 180.0,
                facing: Facing::Inward,
            },
            OrderBy::Address,
            v3(0.0, 4.0, 0.0),
        );
        app.insp.prefs.selection.tool = SelTool::Arrange;
        app.zoom.inspector = 2.0;
        shot(&mut app, "selection_zoom2");
        app.zoom.inspector = 1.0;
        // (c) a truss hung with the selection, then picked on its own.
        // `add_truss` is gone; the Build tab's planner is the one way to make an element.
        app.stage.build(
            &app.patch,
            stage::Recipe::default_for(stage::RecipeKind::Straight),
            stage::Placement::Stagger,
        );
        for i in 0..6.min(app.patch.fixtures.len()) {
            app.stage.select_fixture(i, true);
        }
        let out = app.stage.hang_selection(
            &app.patch,
            ElementRef::Truss(0),
            0,
            HangFill::Spread,
            OrderBy::Address,
        );
        assert!(out.changed >= 2, "nothing hung: {out:?}");
        app.insp.selection.hang_element = Some(ElementRef::Truss(0));
        app.insp.prefs.selection.tool = SelTool::Hang;
        shot(&mut app, "selection_hung");
        // Distinct slots on face 0, and the mounts really are truss mounts.
        let mut slots: Vec<usize> = app
            .stage
            .instances
            .iter()
            .filter_map(|inst| inst.truss_mount.map(|(t, s)| {
                assert_eq!(t, 0);
                s
            }))
            .collect();
        slots.sort_unstable();
        let before = slots.len();
        slots.dedup();
        assert_eq!(slots.len(), before, "two lights share a slot");
        assert!(slots.iter().all(|&s| s < app.stage.trusses[0].slot_count()), "face 0 only");

        app.stage.clear_selection();
        app.stage.sel_truss = Some(0);
        shot(&mut app, "selection_truss");
        app.stage.sel_truss = None;
        app.stage.towers.push(crate::stage::Tower::default());
        app.stage.sel_tower = Some(0);
        shot(&mut app, "selection_tower");

        assert!(app.stage.undo_stack.len() >= 2, "the tools pushed no undo steps");
        assert!(!app.insp.dirty, "the headless scenes must not mark prefs dirty");
        let _ = std::fs::remove_file(&temp);
    }
}

#[cfg(test)]
mod fit_tests {
    use super::*;
    use crate::net::DMX_SLOTS;
    use crate::stage::headless::{render_frames, save};

    /// The tab drawn in a panel pinned to the Inspector's 230 px minimum, at
    /// zoom 1 and zoom 2 — the width egui would never let the app show it at
    /// once its content asks for more. Anything that clips here clips for
    /// real. Written to `target/selection_fit_*.png`.
    #[test]
    fn selection_tab_fits_the_panel_minimum_headless() {
        let mut app = crate::app::App::new();
        if app.patch.fixtures.is_empty() {
            eprintln!("no patch — skipping");
            return;
        }
        let temp = std::env::temp_dir().join("dmxpress_selection_fit_test.json");
        app.stage.layout_path = temp.clone();
        *app.net.dmx.lock() = Frame([170u8; DMX_SLOTS]);
        for i in 0..6.min(app.patch.fixtures.len()) {
            app.stage.select_fixture(i, true);
        }
        app.stage.trusses.push(crate::stage::Truss::straight());
        app.insp.selection.hang_element = Some(ElementRef::Truss(0));
        let size = [300, 1400];
        // What the tab actually asked for inside the panel, measured rather
        // than eyeballed: a strip that does not wrap shows up here as a
        // number wider than the panel long before it shows up in a PNG.
        let used = std::cell::Cell::new(0.0f32);
        for (zoom, tag) in [(1.0f32, "1"), (2.0, "2")] {
            for tool in SelTool::ALL {
                app.insp.prefs.selection.tool = tool;
                used.set(0.0);
                let Some(pixels) = render_frames(4, size, |ctx, frame| {
                    if frame == 0 {
                        crate::ui::install_theme(ctx);
                        return;
                    }
                    egui::SidePanel::right("insp_fit")
                        .exact_width(230.0)
                        .frame(theme::panel_frame(&ctx.style()))
                        .show(ctx, |ui| {
                            crate::ui::apply_zoom(ui, zoom);
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    app.inspector_selection_tab(ui);
                                    used.set(used.get().max(ui.min_rect().width()));
                                });
                        });
                }) else {
                    eprintln!("no GPU adapter — skipping");
                    return;
                };
                save(&pixels, size, &format!("selection_fit_{tag}_{}", tool.label().to_lowercase()));
                eprintln!("{} at zoom {zoom}: {} px of 214", tool.label(), used.get());
                if zoom == 1.0 {
                    assert!(
                        used.get() <= 214.0,
                        "the {} page wants {} px inside a 214 px panel — something is not wrapping",
                        tool.label(),
                        used.get()
                    );
                }
            }
        }
        // …and the element editor at the minimum, too.
        app.stage.clear_selection();
        app.stage.sel_truss = Some(0);
        if render_frames(4, size, |ctx, frame| {
            if frame == 0 {
                crate::ui::install_theme(ctx);
                return;
            }
            egui::SidePanel::right("insp_fit")
                .exact_width(230.0)
                .frame(theme::panel_frame(&ctx.style()))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            app.inspector_selection_tab(ui);
                            used.set(used.get().max(ui.min_rect().width()));
                        });
                });
        })
        .map(|p| save(&p, size, "selection_fit_1_truss"))
        .is_none()
        {
            eprintln!("no GPU adapter — skipping");
        }
        assert!(used.get() <= 214.0, "the element editor wants {} px", used.get());
        let _ = std::fs::remove_file(&temp);
    }
}
