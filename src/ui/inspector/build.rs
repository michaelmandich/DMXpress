//! Inspector · Build tab (area C): the element palette, the parameter card
//! for the picked recipe, the outliner of everything built, saved setups and
//! the reset actions.
//!
//! The tab reads the rig once per frame (`stage.elements()`), draws from that
//! snapshot, and collects every intent into one deferred
//! [`OutlinerAction`] applied after the closures — element indices are only
//! valid for the frame they were read in.

use std::collections::HashSet;

use eframe::egui::{self, vec2, Align2, Color32, FontId, Key, Rect, Rounding, Sense, Stroke, Vec2};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::stage::{
    self, CompositeKind, ElementGroup, ElementInfo, ElementRef, Placement, Recipe, RecipeKind,
    RigSummary, SetupInfo, ARC_PRESETS, F34_LENGTHS, HEIGHT_PRESETS, RADIUS_PRESETS, SPAN_PRESETS,
};
use crate::ui::{
    icons::{self, Icon},
    theme::{self, PadFace, PadState, Tone},
};

use super::InspectorTab;

const HELP_ADD: &str = "Click a pad to set it up, Shift-click to add it straight away, \
                        right-click for standard sizes. Keys 1–8 add while the pointer is \
                        over this tab.";
const HELP_ELEMENTS: &str = "Click a row to pick it on the stage, double-click to rename it, \
                             right-click for the rest. While the pointer is over the list, \
                             ⌫ deletes, F2 renames and ↑/↓ step through it.";

/// Where a new element lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PlaceMode {
    #[default]
    Stagger,
    Upstage,
    Centre,
    Downstage,
    Behind,
    Camera,
    Custom,
}

impl PlaceMode {
    pub(crate) const ALL: [PlaceMode; 7] = [
        Self::Stagger,
        Self::Upstage,
        Self::Centre,
        Self::Downstage,
        Self::Behind,
        Self::Camera,
        Self::Custom,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Stagger => "Stagger",
            Self::Upstage => "Upstage",
            Self::Centre => "Centre",
            Self::Downstage => "Downstage",
            Self::Behind => "Behind",
            Self::Camera => "Camera",
            Self::Custom => "Custom",
        }
    }

    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Stagger => "The next free spot in the row the old Add buttons used.",
            Self::Upstage => "Just inside the back edge of the stage box.",
            Self::Centre => "The middle of the stage box.",
            Self::Downstage => "Just inside the front edge of the stage box.",
            Self::Behind => "Clear of the stage box at the back, for a backdrop rig.",
            Self::Camera => "Wherever the stage camera is looking right now.",
            Self::Custom => "The exact spot on the floor typed below.",
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Stagger => "stagger",
            Self::Upstage => "upstage",
            Self::Centre => "centre",
            Self::Downstage => "downstage",
            Self::Behind => "behind",
            Self::Camera => "camera",
            Self::Custom => "custom",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<PlaceMode> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }
}

/// A destructive action waiting for its inline card to be answered.
#[derive(Clone, PartialEq)]
pub(crate) enum Confirm {
    ClearRig,
    DeleteGroup(u32),
    OverwriteSetup(String),
    /// The name, and whether the rig-only flavour was asked for.
    OverwriteSetupRig(String),
    DeleteSetup(String),
}

/// What the open rename field is renaming.
#[derive(Clone, PartialEq)]
pub(crate) enum Renaming {
    Element(ElementRef),
    Group(u32),
    Setup(String),
}

/// Everything the outliner (and its keys) can ask for, applied after the
/// row closures so no stale index is used.
enum OutlinerAction {
    Select(ElementRef),
    SelectGroup(u32),
    Hover(ElementRef),
    ToggleGroup(u32),
    ToggleExpand(ElementRef),
    Rename(ElementRef),
    RenameGroup(u32),
    CommitRename,
    CancelRename,
    Hidden(ElementRef, bool),
    HideGroup(u32, bool),
    Delete(ElementRef),
    DeleteGroup(u32),
    Duplicate(ElementRef),
    Mirror(ElementRef),
    MirrorGroup(u32),
    Ungroup(u32),
    Move(ElementRef, i32),
    SelectMounted(ElementRef),
    Unhang(ElementRef),
    UnhangAll,
    SelectLoose,
    SelectLight(usize, bool),
    Camera(ElementRef),
    GroupEdit(Option<u32>),
    EditInSelection(ElementRef),
}

/// What a setup row asked for, applied after the fold's body.
enum SetupAction {
    Load(String),
    LoadRig(String),
    LoadLights(String),
    Merge(String),
    Save(String, bool),
    Overwrite(String, bool),
    Rename(String),
    Delete(String),
    Refresh,
}

/// Build-tab preferences, persisted at `InspectorPrefs.build`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct BuildPrefs {
    /// The palette pad that was last picked (a `RecipeKind` key).
    pub kind: Option<String>,
    /// The placement chip that was last picked (a `PlaceMode` key).
    pub place: Option<String>,
    /// Whether the outliner lists the lights hung on each element.
    pub show_lights: bool,
}

/// The Build tab's transient state: nothing here is saved with the show.
pub(crate) struct BuildUi {
    pub kind: RecipeKind,
    /// One remembered recipe per kind, so switching pads loses nothing.
    pub recipes: [Recipe; RecipeKind::COUNT],
    pub place: PlaceMode,
    pub custom_xz: (f32, f32),
    pub rename: Option<(Renaming, String)>,
    /// The rename field still needs focus.
    pub rename_focus: bool,
    /// Groups the outliner has folded away; groups start open.
    pub closed_groups: HashSet<u32>,
    /// Elements whose light rows show even with the Lights toggle off.
    pub expanded: HashSet<ElementRef>,
    pub show_lights: bool,
    pub group_edit: Option<u32>,
    /// A move/turn drag is in flight, so only the first frame pushes undo.
    pub group_gesture: bool,
    pub setups: Vec<SetupInfo>,
    pub setups_dirty: bool,
    pub confirm: Option<Confirm>,
    /// Last frame's tab and outliner rects, for the keyboard shortcuts.
    pub last_rect: Rect,
    pub well_rect: Rect,
    /// The prefs have been copied in once.
    pub seeded: bool,
}

impl Default for BuildUi {
    fn default() -> Self {
        let mut recipes = [Recipe::default_for(RecipeKind::Straight); RecipeKind::COUNT];
        for (i, k) in RecipeKind::ALL.into_iter().enumerate() {
            recipes[i] = Recipe::default_for(k);
        }
        Self {
            kind: RecipeKind::Straight,
            recipes,
            place: PlaceMode::Stagger,
            custom_xz: (0.0, 0.0),
            rename: None,
            rename_focus: false,
            closed_groups: HashSet::new(),
            expanded: HashSet::new(),
            show_lights: false,
            group_edit: None,
            group_gesture: false,
            setups: Vec::new(),
            setups_dirty: true,
            confirm: None,
            last_rect: Rect::NOTHING,
            well_rect: Rect::NOTHING,
            seeded: false,
        }
    }
}

impl BuildUi {
    /// The recipe the picked pad is carrying.
    pub(crate) fn recipe(&self) -> Recipe {
        self.recipes[self.kind as usize]
    }

    /// Where a build should land, in world coordinates.
    pub(crate) fn placement_world(
        &self,
        set: &stage::Settings,
        view: &stage::StageView,
    ) -> Placement {
        let back = set.stage_half_d;
        match self.place {
            PlaceMode::Stagger => Placement::Stagger,
            PlaceMode::Upstage => Placement::At(stage::v3(0.0, 0.0, -(back - 0.5))),
            PlaceMode::Centre => Placement::At(stage::v3(0.0, 0.0, 0.0)),
            PlaceMode::Downstage => Placement::At(stage::v3(0.0, 0.0, back - 0.5)),
            PlaceMode::Behind => Placement::At(stage::v3(0.0, 0.0, -(back + 1.5))),
            PlaceMode::Camera => Placement::At(stage::v3(
                view.cam.target.x.clamp(-20.0, 20.0),
                0.0,
                view.cam.target.z.clamp(-20.0, 20.0),
            )),
            PlaceMode::Custom => {
                Placement::At(stage::v3(self.custom_xz.0, 0.0, self.custom_xz.1))
            }
        }
    }
}

/// The same for a small text button, e.g. the setup rows' Load.
fn small_button_width(ui: &egui::Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let w = ui.fonts(|f| f.layout_no_wrap(text.to_owned(), font, theme::TEXT).size().x);
    w + ui.spacing().button_padding.x * 2.0 + ui.spacing().item_spacing.x + 2.0
}

/// The glyph a recipe kind wears on its pad and its rows.
fn kind_icon(k: RecipeKind) -> Icon {
    match k {
        RecipeKind::Tower => Icon::Tower,
        RecipeKind::Straight => Icon::TrussStraight,
        RecipeKind::Radius => Icon::TrussRadius,
        RecipeKind::Goalpost => Icon::Goalpost,
        RecipeKind::Box => Icon::BoxRig,
        RecipeKind::Ring => Icon::Ring,
        RecipeKind::TowerPair => Icon::TowerPair,
        RecipeKind::Arch => Icon::Arch,
    }
}

fn composite_icon(k: CompositeKind) -> Icon {
    kind_icon(k.recipe_kind())
}

/// "14 Sep" from a file's modified time (UTC) — enough to tell two saves
/// apart without pulling in a date crate.
fn short_date(t: std::time::SystemTime) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Howard Hinnant's civil_from_days, with the era shifted to 0000-03-01.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{day} {}", MONTHS[((month - 1).clamp(0, 11)) as usize])
}

/// What one setup row's second line says.
fn setup_caption(info: &SetupInfo) -> String {
    let mut parts: Vec<String> = Vec::new();
    if info.towers > 0 {
        parts.push(format!("{} tower{}", info.towers, if info.towers == 1 { "" } else { "s" }));
    }
    if info.trusses > 0 {
        parts.push(format!("{} truss{}", info.trusses, if info.trusses == 1 { "" } else { "es" }));
    }
    if info.lights > 0 {
        parts.push(format!("{} light{}", info.lights, if info.lights == 1 { "" } else { "s" }));
    }
    if parts.is_empty() {
        parts.push("empty".into());
    }
    if let Some(t) = info.modified {
        parts.push(short_date(t));
    }
    parts.join(" · ")
}

/// A warm inline question with a confirm and a cancel. `Some(true)` =
/// go ahead, `Some(false)` = leave it alone.
fn confirm_card(ui: &mut egui::Ui, question: &str, verb: &str) -> Option<bool> {
    let z = theme::zoom_of(ui);
    let mut out = None;
    egui::Frame::none()
        .fill(theme::WARN.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, theme::WARN.gamma_multiply(0.5)))
        .rounding(Rounding::same(6.0))
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(question).size(11.0 * z).color(theme::TEXT),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            theme::toolbar(ui, |ui| {
                if theme::danger_button(ui, verb)
                    .on_hover_text("Go ahead — this cannot be undone with ⌘Z.")
                    .clicked()
                {
                    out = Some(true);
                }
                if ui
                    .button("Cancel")
                    .on_hover_text("Leave everything as it is.")
                    .clicked()
                {
                    out = Some(false);
                }
            });
        });
    out
}

/// One parameter: a label / DragValue row with its standard sizes beneath.
/// Returns true when the value changed this frame.
fn param_row(
    ui: &mut egui::Ui,
    row: usize,
    label: &str,
    hint: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f64,
    suffix: &str,
    presets: &[f32],
) -> bool {
    let mut changed = theme::kv_grid(ui, ("insp_build_params", row), |ui| {
        theme::label_dim(ui, label);
        let r = ui.add(
            egui::DragValue::new(value)
                .speed(speed)
                .range(range)
                .suffix(suffix)
                .max_decimals(1),
        );
        ui.end_row();
        r.on_hover_text(hint).changed()
    });
    if !presets.is_empty() {
        // One scope for the whole row: `chip` keys its id off its text, and
        // a scope per chip would stop the row wrapping.
        ui.push_id(("insp_build_chips", row), |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
            for &v in presets {
                let text = if (v - v.round()).abs() < 1e-3 {
                    format!("{v:.0}")
                } else {
                    format!("{v:.1}")
                };
                let on = (*value - v).abs() < 1e-3;
                let r = theme::chip(ui, &text, theme::ACCENT_SOFT, on);
                if r.on_hover_text(format!("Set {label} to {text}{suffix}.")).clicked() {
                    *value = v;
                    changed = true;
                }
            }
        });
        });
    }
    changed
}

/// A heading-in-degrees row with two named shortcuts.
fn heading_row(ui: &mut egui::Ui, row: usize, value: &mut f32) -> bool {
    let mut changed = theme::kv_grid(ui, ("insp_build_params", row), |ui| {
        theme::label_dim(ui, "Heading");
        let r = ui.add(
            egui::DragValue::new(value).speed(0.5).range(0.0..=360.0).suffix("°").max_decimals(0),
        );
        ui.end_row();
        r.on_hover_text("Which way this element faces, in degrees clockwise from upstage.")
            .changed()
    });
    ui.push_id(("insp_build_chips", row), |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
            for (text, deg) in [("across", 0.0f32), ("along", 90.0)] {
                let on = (*value - deg).abs() < 1e-3;
                let r = theme::chip(ui, text, theme::ACCENT_SOFT, on);
                if r.on_hover_text(format!("Face {deg:.0}° — {text} the stage.")).clicked() {
                    *value = deg;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// One outliner entry: a composite's header with its members, or a single
/// row.
enum Node {
    Group { g: ElementGroup, name: String, members: Vec<usize>, hidden: bool },
    Item(usize),
}

/// One light hanging on an element, ready to draw without touching the
/// stage again.
struct LightRow {
    inst: usize,
    title: String,
    subtitle: String,
    color: Color32,
    selected: bool,
}

impl App {
    pub(crate) fn inspector_build_tab(&mut self, ui: &mut egui::Ui) {
        self.build_prologue(ui);
        let mut build_now = self.build_pads(ui);
        ui.add_space(6.0);
        if self.build_card(ui) {
            build_now = Some(self.insp.build.kind);
        }
        // Keys 1–8 fire the pads while the pointer is over the tab.
        if build_now.is_none() && !ui.ctx().wants_keyboard_input() {
            let tab = self.insp.build.last_rect;
            if tab.is_positive() && ui.rect_contains_pointer(tab) {
                const KEYS: [Key; 8] = [
                    Key::Num1,
                    Key::Num2,
                    Key::Num3,
                    Key::Num4,
                    Key::Num5,
                    Key::Num6,
                    Key::Num7,
                    Key::Num8,
                ];
                for (n, key) in KEYS.into_iter().enumerate() {
                    if ui.input(|i| i.key_pressed(key)) {
                        build_now = Some(RecipeKind::ALL[n]);
                    }
                }
            }
        }
        if let Some(k) = build_now {
            self.build_kind(k);
        }
        ui.add_space(8.0);
        self.build_outliner(ui);
        ui.add_space(8.0);
        let n = self.insp.build.setups.len();
        let n_text = n.to_string();
        let badge = (n > 0).then_some((n_text.as_str(), theme::TEXT_DIM));
        self.insp_fold(ui, "build.setups", "Setups", true, badge, |app, ui| {
            app.build_setups(ui);
        });
        ui.add_space(8.0);
        self.build_reset(ui);
        self.insp.build.last_rect = ui.min_rect();
    }

    /// No widgets: drop stale references, refresh the setups cache and let
    /// Escape back out of a confirm or a group edit.
    fn build_prologue(&mut self, ui: &mut egui::Ui) {
        let (nt, nf) = (self.stage.towers.len(), self.stage.trusses.len());
        let live = |r: &ElementRef| match r {
            ElementRef::Tower(i) => *i < nt,
            ElementRef::Truss(i) => *i < nf,
        };
        let gids: HashSet<u32> = self
            .stage
            .towers
            .iter()
            .filter_map(|t| t.group.map(|g| g.id))
            .chain(self.stage.trusses.iter().filter_map(|t| t.group.map(|g| g.id)))
            .collect();
        let b = &mut self.insp.build;
        b.expanded.retain(live);
        b.closed_groups.retain(|g| gids.contains(g));
        if b.group_edit.is_some_and(|g| !gids.contains(&g)) {
            b.group_edit = None;
            b.group_gesture = false;
        }
        match &b.rename {
            Some((Renaming::Element(r), _)) if !live(r) => b.rename = None,
            Some((Renaming::Group(g), _)) if !gids.contains(g) => b.rename = None,
            _ => {}
        }
        if let Some(Confirm::DeleteGroup(g)) = &b.confirm {
            if !gids.contains(g) {
                b.confirm = None;
            }
        }
        // Seed the remembered pad, placement and Lights toggle once.
        if !b.seeded {
            b.seeded = true;
            let prefs = self.insp.prefs.build.clone();
            let b = &mut self.insp.build;
            if let Some(k) = prefs.kind.as_deref().and_then(RecipeKind::from_key) {
                b.kind = k;
            }
            if let Some(p) = prefs.place.as_deref().and_then(PlaceMode::from_key) {
                b.place = p;
            }
            b.show_lights = prefs.show_lights;
        }
        if self.insp.build.setups_dirty {
            self.insp.build.setups_dirty = false;
            self.insp.build.setups = stage::StageView::setup_infos();
        }
        if !ui.ctx().wants_keyboard_input() && ui.input(|i| i.key_pressed(Key::Escape)) {
            let tab = self.insp.build.last_rect;
            if tab.is_positive() && ui.rect_contains_pointer(tab) {
                self.insp.build.confirm = None;
                self.insp.build.group_edit = None;
                self.insp.build.group_gesture = false;
            }
        }
    }

    /// The eight palette pads. Returns the kind to build right now, if any.
    fn build_pads(&mut self, ui: &mut egui::Ui) -> Option<RecipeKind> {
        let z = theme::zoom_of(ui);
        let selected = self.insp.build.kind;
        let recipes = self.insp.build.recipes;
        let shift = ui.input(|i| i.modifiers.shift);
        let mut build_now: Option<RecipeKind> = None;
        let mut pick: Option<RecipeKind> = None;
        let mut variant: Option<(usize, Recipe)> = None;

        theme::section_with(ui, "Add element", |ui| {
            theme::help(ui, HELP_ADD);
        });
        let avail = ui.available_width();
        let gap = 5.0 * z;
        let cols = theme::pad_columns(avail, 60.0 * z, gap);
        let w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
        let h = (w * 0.74).clamp(34.0 * z, 92.0 * z);
        // Rows are laid out by hand: `pad_button` scopes its own id, which
        // stops `horizontal_wrapped` from wrapping it.
        for (row, chunk) in RecipeKind::ALL.chunks(cols).enumerate() {
            if row > 0 {
                ui.add_space(gap);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(gap, gap);
                for (col, &k) in chunk.iter().enumerate() {
                    let n = row * cols + col;
                    let resp = theme::pad_button(
                        ui,
                        ("insp_build_pad", n as u8),
                        vec2(w, h),
                        PadState::default(),
                        PadFace::Icon(kind_icon(k)),
                        k.label(),
                    );
                    let rect = resp.rect;
                    if ui.is_rect_visible(rect) {
                        let p = ui.painter();
                        if k == selected {
                            // The tab-strip look: a muted ring and a lit
                            // bar. The bar rides the top edge, because the
                            // bottom of a pad carries its name.
                            p.rect_stroke(
                                rect,
                                Rounding::same(theme::R_PAD),
                                Stroke::new(1.5, theme::ACCENT_MUTED),
                            );
                            let bar = Rect::from_min_max(
                                egui::pos2(rect.left() + 8.0 * z, rect.top() + 1.0),
                                egui::pos2(rect.right() - 8.0 * z, rect.top() + 1.0 + 2.0 * z),
                            );
                            p.rect_filled(bar, Rounding::same(1.0), theme::ACCENT_SOFT);
                        }
                        p.text(
                            rect.left_top() + vec2(5.0 * z, 5.0 * z),
                            Align2::LEFT_TOP,
                            (n + 1).to_string(),
                            FontId::new(9.0 * z, theme::medium()),
                            theme::TEXT_DIM,
                        );
                    }
                    let resp = resp.on_hover_text(k.hover());
                    if resp.double_clicked() || (resp.clicked() && shift) {
                        build_now = Some(k);
                    } else if resp.clicked() {
                        pick = Some(k);
                    }
                    resp.context_menu(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{} — standard sizes", k.label()))
                                .size(11.0)
                                .color(theme::TEXT_DIM),
                        );
                        for (label, r) in recipes[n].variants() {
                            if ui.button(label).clicked() {
                                variant = Some((n, r));
                                build_now = Some(k);
                                ui.close_menu();
                            }
                        }
                    });
                }
            });
        }
        if let Some((n, r)) = variant {
            self.insp.build.recipes[n] = r;
        }
        if let Some(k) = pick {
            if self.insp.build.kind != k {
                self.insp.build.kind = k;
                self.insp.prefs.build.kind = Some(k.key().to_owned());
                self.insp.mark_dirty();
            }
        }
        build_now
    }

    /// The picked recipe's parameters, its placement and the one Add
    /// button. Returns true when Add was pressed.
    fn build_card(&mut self, ui: &mut egui::Ui) -> bool {
        let z = theme::zoom_of(ui);
        let k = self.insp.build.kind;
        let mut recipe = self.insp.build.recipe();
        let mut place = self.insp.build.place;
        let mut custom = self.insp.build.custom_xz;
        let mut add = false;
        theme::card(ui, |ui| {
            theme::card_title(ui, k.label(), |ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0 * z), Sense::hover());
                icons::draw(ui.painter(), r, kind_icon(k), theme::ACCENT_SOFT);
            });
            theme::hint(ui, k.caption());
            match &mut recipe {
                Recipe::Tower { height, width, yaw } => {
                    param_row(ui, 0, "Height", "How tall the stand is, floor to crossbar.", height, 1.2..=6.0, 0.05, " m", &HEIGHT_PRESETS);
                    param_row(ui, 1, "Width", "How wide the crossbar is; the four slots spread across it.", width, 0.8..=4.0, 0.05, " m", &[]);
                    heading_row(ui, 2, yaw);
                }
                Recipe::Straight { length, height, yaw, grounded } => {
                    param_row(ui, 0, "Length", "How long the run is; a slot every half metre on each face.", length, 0.5..=6.0, 0.05, " m", &F34_LENGTHS);
                    param_row(ui, 1, "Height", "Trim height of the run above the floor.", height, 0.5..=8.0, 0.05, " m", &HEIGHT_PRESETS);
                    heading_row(ui, 2, yaw);
                    ui.checkbox(grounded, "Legs")
                        .on_hover_text("Draw support legs down to the floor at both ends.");
                }
                Recipe::Radius { radius, arc, height, yaw, grounded } => {
                    param_row(ui, 0, "Radius", "How far the run sits from the centre of its circle.", radius, 0.5..=8.0, 0.05, " m", &RADIUS_PRESETS);
                    param_row(ui, 1, "Arc", "How far round the circle the run sweeps.", arc, 15.0..=360.0, 0.5, "°", &ARC_PRESETS);
                    param_row(ui, 2, "Height", "Trim height of the run above the floor.", height, 0.5..=8.0, 0.05, " m", &HEIGHT_PRESETS);
                    heading_row(ui, 3, yaw);
                    ui.checkbox(grounded, "Legs")
                        .on_hover_text("Draw support legs down to the floor at both ends of the arc.");
                }
                Recipe::Goalpost { span, height, yaw } => {
                    param_row(ui, 0, "Span", "How far apart the two legs stand.", span, 2.0..=10.0, 0.05, " m", &SPAN_PRESETS);
                    param_row(ui, 1, "Height", "How tall the legs are; the beam sits on their tops.", height, 1.2..=6.0, 0.05, " m", &HEIGHT_PRESETS);
                    heading_row(ui, 2, yaw);
                }
                Recipe::Box { width, depth, height, yaw, grounded } => {
                    param_row(ui, 0, "Width", "How wide the rectangle is, left to right.", width, 2.0..=12.0, 0.05, " m", &[3.0, 4.0, 6.0, 8.0]);
                    param_row(ui, 1, "Depth", "How deep the rectangle is, front to back.", depth, 2.0..=12.0, 0.05, " m", &[3.0, 4.0, 6.0]);
                    param_row(ui, 2, "Height", "Trim height of all four runs.", height, 0.5..=8.0, 0.05, " m", &HEIGHT_PRESETS);
                    heading_row(ui, 3, yaw);
                    ui.checkbox(grounded, "Legs")
                        .on_hover_text("Draw support legs down to the floor at the corners.");
                }
                Recipe::Ring { radius, height } => {
                    param_row(ui, 0, "Radius", "How wide the circle is, centre to truss.", radius, 0.5..=8.0, 0.05, " m", &RADIUS_PRESETS);
                    param_row(ui, 1, "Height", "Trim height of the ring above the floor.", height, 0.5..=8.0, 0.05, " m", &HEIGHT_PRESETS);
                }
                Recipe::TowerPair { spacing, height, width, yaw } => {
                    param_row(ui, 0, "Spacing", "How far apart the two stands are.", spacing, 1.0..=10.0, 0.05, " m", &SPAN_PRESETS);
                    param_row(ui, 1, "Height", "How tall both stands are.", height, 1.2..=6.0, 0.05, " m", &HEIGHT_PRESETS);
                    param_row(ui, 2, "Width", "How wide each crossbar is.", width, 0.8..=4.0, 0.05, " m", &[]);
                    heading_row(ui, 3, yaw);
                }
                Recipe::Arch { span, rise, yaw } => {
                    param_row(ui, 0, "Span", "How far apart the two legs stand; the arch is half that across.", span, 2.0..=10.0, 0.05, " m", &[3.0, 4.0, 6.0]);
                    param_row(ui, 1, "Rise", "How tall the legs are; the arch springs from their tops.", rise, 1.2..=6.0, 0.05, " m", &HEIGHT_PRESETS);
                    heading_row(ui, 2, yaw);
                }
            }
            ui.add_space(4.0);
            theme::label_dim(ui, "Place");
            ui.push_id("insp_build_place", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
                    for mode in PlaceMode::ALL {
                        let r = theme::chip(ui, mode.label(), theme::ACCENT_SOFT, mode == place);
                        if r.on_hover_text(mode.hint()).clicked() {
                            place = mode;
                        }
                    }
                });
            });
            if place == PlaceMode::Custom {
                theme::kv_grid(ui, "insp_build_place_xz", |ui| {
                    theme::label_dim(ui, "X");
                    ui.add(egui::DragValue::new(&mut custom.0).speed(0.05).range(-30.0..=30.0).suffix(" m"))
                        .on_hover_text("Metres right of the stage centre.");
                    ui.end_row();
                    theme::label_dim(ui, "Z");
                    ui.add(egui::DragValue::new(&mut custom.1).speed(0.05).range(-30.0..=30.0).suffix(" m"))
                        .on_hover_text("Metres downstage of the stage centre.");
                    ui.end_row();
                });
            }
            ui.add_space(6.0);
            if theme::wide_button(ui, Some(kind_icon(k)), &format!("Add {}", k.label()), Tone::Accent)
                .on_hover_text("Add this to the stage with these settings. It becomes the selected element.")
                .clicked()
            {
                add = true;
            }
            theme::hint(ui, "Shift-click a pad adds it straight away.");
        });
        self.insp.build.recipes[k as usize] = recipe;
        self.insp.build.custom_xz = custom;
        if place != self.insp.build.place {
            self.insp.build.place = place;
            self.insp.prefs.build.place = Some(place.key().to_owned());
            self.insp.mark_dirty();
        }
        add
    }

    /// Build one element from its remembered parameters.
    fn build_kind(&mut self, k: RecipeKind) {
        let recipe = self.insp.build.recipes[k as usize];
        let placement = self.insp.build.placement_world(&self.settings, &self.stage);
        let res = self.stage.build(&self.patch, recipe, placement);
        self.log.push(format!("Added {}", res.summary));
        if let Some(gid) = res.group {
            // A freshly built composite starts open, whatever was folded.
            self.insp.build.closed_groups.remove(&gid);
        }
        self.insp.build.rename = None;
        self.sync_selection_units();
        self.sel_fixture = self.stage.last_selected;
    }

    /// The list of everything built, composites nested under their headers.
    fn build_outliner(&mut self, ui: &mut egui::Ui) {
        let z = theme::zoom_of(ui);
        let rows = self.stage.elements();
        let summary: RigSummary = self.stage.rig_summary();
        let sel = self.stage.selected_element();
        let undo_ok = !self.stage.undo_stack.is_empty();
        let show_lights = self.insp.build.show_lights;
        let closed = self.insp.build.closed_groups.clone();
        let expanded = self.insp.build.expanded.clone();
        let group_edit = self.insp.build.group_edit;
        let mut rename = self.insp.build.rename.clone();
        let mut rename_focus = self.insp.build.rename_focus;
        let confirm = self.insp.build.confirm.clone();
        let mut act: Option<OutlinerAction> = None;
        let mut lights_toggle = show_lights;
        let mut undo_now = false;
        let mut group_delta: Option<(u32, stage::V3, f32)> = None;
        let mut delete_group: Option<u32> = None;
        let mut cancel_confirm = false;

        // The composite headers and their members, worked out from the row
        // order `elements()` already put them in.
        let mut nodes: Vec<Node> = Vec::new();
        let mut i = 0;
        while i < rows.len() {
            match rows[i].group {
                Some(g) => {
                    let mut members = vec![i];
                    let mut j = i + 1;
                    while j < rows.len() && rows[j].group.map(|x| x.id) == Some(g.id) {
                        members.push(j);
                        j += 1;
                    }
                    if members.len() == 1 {
                        nodes.push(Node::Item(i));
                    } else {
                        let hidden = members.iter().all(|&k| rows[k].hidden);
                        let name = self.stage.group_name(g.id);
                        nodes.push(Node::Group { g, name, members, hidden });
                    }
                    i = j;
                }
                None => {
                    nodes.push(Node::Item(i));
                    i += 1;
                }
            }
        }

        // Every light hanging on every element, described once.
        let buf = *self.net.dmx.lock();
        let lights: Vec<Vec<LightRow>> = rows
            .iter()
            .map(|info| {
                if !show_lights && !expanded.contains(&info.r) {
                    return Vec::new();
                }
                self.stage
                    .mounted_on(info.r)
                    .into_iter()
                    .filter_map(|(inst, slot)| {
                        let fi = self.stage.instances.get(inst)?.fixture;
                        let f = self.patch.fixtures.get(fi)?;
                        Some(LightRow {
                            inst,
                            title: f.display.clone(),
                            subtitle: format!(
                                "@{} · {}",
                                f.from,
                                self.stage.mount_face_label(info.r, slot)
                            ),
                            color: stage::fixture_swatch(f, &buf),
                            selected: self.stage.selection.contains(&inst),
                        })
                    })
                    .collect()
            })
            .collect();

        theme::section_with(ui, "Elements", |ui| {
            if theme::tool_button(
                ui,
                Icon::Undo,
                None,
                "Undo the last stage edit (⌘Z over the stage).",
                if undo_ok { Ok(()) } else { Err("Nothing to undo on the stage.") },
            )
            .clicked()
            {
                undo_now = true;
            }
            if theme::toggle_icon(
                ui,
                Icon::Light,
                None,
                show_lights,
                "Show the lights hanging on each element as rows under it.",
            )
            .clicked()
            {
                lights_toggle = !show_lights;
            }
            theme::count_pill(ui, summary.towers + summary.trusses, "element")
                .on_hover_text(HELP_ELEMENTS);
        });

        let well = theme::well(ui, |ui| {
            if rows.is_empty() {
                theme::empty_state(
                    ui,
                    Icon::Build,
                    "No towers or trusses yet",
                    "Pick an element above.",
                );
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("insp_build_outliner")
                .max_height(260.0 * z)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for node in &nodes {
                        match node {
                            Node::Group { g, name, members, hidden } => {
                                let open = !closed.contains(&g.id);
                                let sel_inside = members
                                    .iter()
                                    .any(|&k| sel == Some(rows[k].r));
                                let parts = format!("{} parts", members.len());
                                let mut badges: Vec<(&str, Color32)> =
                                    vec![(parts.as_str(), theme::TEXT_DIM)];
                                if *hidden {
                                    badges.push(("hidden", theme::WARN));
                                }
                                ui.horizontal(|ui| {
                                    if theme::tool_button(
                                        ui,
                                        if open { Icon::ChevronDown } else { Icon::ChevronRight },
                                        None,
                                        "Fold this composite's parts away, or open them again.",
                                        Ok(()),
                                    )
                                    .clicked()
                                    {
                                        act = Some(OutlinerAction::ToggleGroup(g.id));
                                    }
                                    let renaming = rename
                                        .as_ref()
                                        .is_some_and(|(r, _)| *r == Renaming::Group(g.id));
                                    if renaming {
                                        if let Some((_, buf)) = rename.as_mut() {
                                            let r = ui.add(
                                                egui::TextEdit::singleline(buf)
                                                    .desired_width(f32::INFINITY)
                                                    .id_salt(("insp_build_rename_g", g.id)),
                                            )
                                            .on_hover_text(
                                                "Type a new name for the composite; every                                                  part keeps its own role. Enter saves,                                                  Escape cancels.",
                                            );
                                            if rename_focus {
                                                r.request_focus();
                                                rename_focus = false;
                                            }
                                            if r.lost_focus() {
                                                act = Some(OutlinerAction::CommitRename);
                                            }
                                            if ui.input(|i| i.key_pressed(Key::Escape)) {
                                                act = Some(OutlinerAction::CancelRename);
                                            }
                                        }
                                    } else {
                                        let row = theme::list_row(
                                            ui,
                                            ("insp_build_grow", g.id),
                                            &theme::Row {
                                                swatch: None,
                                                icon: Some(composite_icon(g.kind)),
                                                title: name,
                                                subtitle: None,
                                                badges: &badges,
                                                selected: sel_inside && !open,
                                                active: false,
                                                indent: 0.0,
                                            },
                                        );
                                        if row.hovered() {
                                            // Light the first part, so the
                                            // header points at the rig.
                                            act = Some(OutlinerAction::Hover(
                                                rows[members[0]].r,
                                            ));
                                        }
                                        let row = row.on_hover_text(
                                            "One composite. Click to pick it on the stage; \
                                             right-click to move, hide, ungroup or delete it.",
                                        );
                                        if row.clicked() {
                                            act = Some(OutlinerAction::SelectGroup(g.id));
                                        }
                                        if row.double_clicked() {
                                            act = Some(OutlinerAction::RenameGroup(g.id));
                                        }
                                        row.context_menu(|ui| {
                                            if ui.button("Rename group…").clicked() {
                                                act = Some(OutlinerAction::RenameGroup(g.id));
                                                ui.close_menu();
                                            }
                                            if ui.button("Move / turn…").clicked() {
                                                act = Some(OutlinerAction::GroupEdit(Some(g.id)));
                                                ui.close_menu();
                                            }
                                            if ui.button("Mirror group across centre").clicked() {
                                                act = Some(OutlinerAction::MirrorGroup(g.id));
                                                ui.close_menu();
                                            }
                                            let verb =
                                                if *hidden { "Show group" } else { "Hide group" };
                                            if ui.button(verb).clicked() {
                                                act = Some(OutlinerAction::HideGroup(
                                                    g.id, !*hidden,
                                                ));
                                                ui.close_menu();
                                            }
                                            if ui.button("Ungroup").clicked() {
                                                act = Some(OutlinerAction::Ungroup(g.id));
                                                ui.close_menu();
                                            }
                                            ui.separator();
                                            if ui.button("Delete group…").clicked() {
                                                delete_group = Some(g.id);
                                                ui.close_menu();
                                            }
                                        });
                                    }
                                });
                                if group_edit == Some(g.id) {
                                    let mut d = [0.0f32; 4];
                                    theme::kv_grid(ui, ("insp_build_gxf", g.id), |ui| {
                                        for (n, (label, hint, speed)) in [
                                            ("ΔX", "Slide the whole composite left or right, in metres.", 0.05),
                                            ("ΔZ", "Slide the whole composite up or downstage, in metres.", 0.05),
                                            ("ΔHeight", "Raise or lower it; its legs grow with it.", 0.05),
                                            ("ΔTurn", "Turn the whole composite about its own centre.", 0.5),
                                        ]
                                        .into_iter()
                                        .enumerate()
                                        {
                                            theme::label_dim(ui, label);
                                            ui.add(
                                                egui::DragValue::new(&mut d[n])
                                                    .speed(speed)
                                                    .max_decimals(2),
                                            )
                                            .on_hover_text(hint);
                                            ui.end_row();
                                        }
                                    });
                                    if ui
                                        .small_button("Done")
                                        .on_hover_text("Close the move and turn controls.")
                                        .clicked()
                                    {
                                        act = Some(OutlinerAction::GroupEdit(None));
                                    }
                                    if d.iter().any(|v| *v != 0.0) {
                                        group_delta = Some((
                                            g.id,
                                            stage::v3(d[0], d[2], d[1]),
                                            d[3],
                                        ));
                                    }
                                }
                                if open {
                                    for &k in members {
                                        element_row(
                                            ui,
                                            &rows[k],
                                            &lights[k],
                                            sel,
                                            true,
                                            &mut rename,
                                            &mut rename_focus,
                                            &mut act,
                                        );
                                    }
                                }
                            }
                            Node::Item(k) => element_row(
                                ui,
                                &rows[*k],
                                &lights[*k],
                                sel,
                                false,
                                &mut rename,
                                &mut rename_focus,
                                &mut act,
                            ),
                        }
                    }
                });
        });
        self.insp.build.well_rect = well.response.rect;

        if let Some(gid) = delete_group {
            self.insp.build.confirm = Some(Confirm::DeleteGroup(gid));
        }
        if let Some(Confirm::DeleteGroup(gid)) = confirm {
            let name = self.stage.group_name(gid);
            let parts = self.stage.group_members(gid).len();
            let hung: usize = self
                .stage
                .group_members(gid)
                .iter()
                .map(|&r| self.stage.mounted_on(r).len())
                .sum();
            let q = format!(
                "Delete '{name}' and its {parts} parts? {hung} lights will be unhung."
            );
            match confirm_card(ui, &q, "Delete") {
                Some(true) => act = Some(OutlinerAction::DeleteGroup(gid)),
                Some(false) => cancel_confirm = true,
                None => {}
            }
        }

        // Footer: unhang everything, and a way to find the loose lights.
        theme::toolbar(ui, |ui| {
            if theme::tool_button(
                ui,
                Icon::Close,
                Some("Unhang all"),
                "Let go of every light on every tower and truss, leaving them where they are.",
                if summary.slots_used > 0 { Ok(()) } else { Err("No lights are hung.") },
            )
            .clicked()
            {
                act = Some(OutlinerAction::UnhangAll);
            }
            theme::pill(
                ui,
                &if summary.slots_used > 0 {
                    format!("{} hung", summary.slots_used)
                } else {
                    "none hung".to_owned()
                },
                if summary.slots_used > 0 { theme::ACCENT_SOFT } else { theme::TEXT_DIM },
            );
        });
        if summary.lights_loose > 0 {
            ui.horizontal(|ui| {
                theme::hint(ui, format!("{} lights not hung", summary.lights_loose));
                if ui
                    .small_button("Select")
                    .on_hover_text("Select every light that is not hung on anything.")
                    .clicked()
                {
                    act = Some(OutlinerAction::SelectLoose);
                }
            });
        }

        // Keys, while the pointer is over the list.
        let well_rect = self.insp.build.well_rect;
        if act.is_none()
            && !ui.ctx().wants_keyboard_input()
            && well_rect.is_positive()
            && ui.rect_contains_pointer(well_rect)
        {
            if let Some(r) = sel {
                if ui.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)) {
                    act = Some(OutlinerAction::Delete(r));
                } else if ui.input(|i| i.key_pressed(Key::F2)) {
                    act = Some(OutlinerAction::Rename(r));
                }
            }
            if act.is_none() {
                let at = rows.iter().position(|info| Some(info.r) == sel);
                let step = if ui.input(|i| i.key_pressed(Key::ArrowDown)) {
                    1i32
                } else if ui.input(|i| i.key_pressed(Key::ArrowUp)) {
                    -1
                } else {
                    0
                };
                if step != 0 && !rows.is_empty() {
                    let next = match at {
                        Some(k) => (k as i32 + step).clamp(0, rows.len() as i32 - 1) as usize,
                        None => {
                            if step > 0 {
                                0
                            } else {
                                rows.len() - 1
                            }
                        }
                    };
                    act = Some(OutlinerAction::Select(rows[next].r));
                }
            }
        }

        self.insp.build.rename = rename;
        self.insp.build.rename_focus = rename_focus;
        if cancel_confirm {
            self.insp.build.confirm = None;
        }
        if lights_toggle != show_lights {
            self.insp.build.show_lights = lights_toggle;
            self.insp.prefs.build.show_lights = lights_toggle;
            self.insp.mark_dirty();
        }
        if undo_now && self.stage.undo(&self.patch) {
            self.log.push("Undid the last stage edit".into());
            self.sync_selection_units();
            self.sel_fixture = self.stage.last_selected;
        }
        if let Some((gid, mv, turn)) = group_delta {
            if !self.insp.build.group_gesture {
                self.stage.push_undo();
                self.insp.build.group_gesture = true;
            }
            if mv != stage::v3(0.0, 0.0, 0.0) {
                self.stage.move_group(&self.patch, gid, mv);
            }
            if turn != 0.0 {
                self.stage.rotate_group(&self.patch, gid, turn);
            }
        } else if self.insp.build.group_gesture && !ui.input(|i| i.pointer.any_down()) {
            self.insp.build.group_gesture = false;
            if let Some(gid) = self.insp.build.group_edit {
                let name = self.stage.group_name(gid);
                self.log.push(format!("Moved '{name}'"));
            }
        }
        if let Some(a) = act {
            self.apply_outliner_action(a);
        }
    }

    fn apply_outliner_action(&mut self, act: OutlinerAction) {
        match act {
            OutlinerAction::Hover(r) => self.stage.hover_element = Some(r),
            OutlinerAction::Select(r) => {
                self.stage.select_element(r);
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
            }
            OutlinerAction::SelectGroup(gid) => {
                if let Some(&r) = self.stage.group_members(gid).first() {
                    self.stage.select_element(r);
                    self.sync_selection_units();
                    self.sel_fixture = self.stage.last_selected;
                }
                self.insp.build.closed_groups.remove(&gid);
            }
            OutlinerAction::ToggleGroup(gid) => {
                if !self.insp.build.closed_groups.remove(&gid) {
                    self.insp.build.closed_groups.insert(gid);
                }
            }
            OutlinerAction::ToggleExpand(r) => {
                if !self.insp.build.expanded.remove(&r) {
                    self.insp.build.expanded.insert(r);
                }
            }
            OutlinerAction::Rename(r) => {
                let name = self.stage.element_name(r);
                self.insp.build.rename = Some((Renaming::Element(r), name));
                self.insp.build.rename_focus = true;
            }
            OutlinerAction::RenameGroup(gid) => {
                let name = self.stage.group_name(gid);
                self.insp.build.rename = Some((Renaming::Group(gid), name));
                self.insp.build.rename_focus = true;
            }
            OutlinerAction::CancelRename => {
                self.insp.build.rename = None;
                self.insp.build.rename_focus = false;
            }
            OutlinerAction::CommitRename => {
                let Some((what, text)) = self.insp.build.rename.take() else { return };
                self.insp.build.rename_focus = false;
                match what {
                    Renaming::Element(r) => {
                        let was = self.stage.element_name(r);
                        self.stage.rename_element(&self.patch, r, &text);
                        let now = self.stage.element_name(r);
                        if now != was {
                            self.log.push(format!("Renamed '{was}' → '{now}'"));
                        }
                    }
                    Renaming::Group(gid) => {
                        let was = self.stage.group_name(gid);
                        self.stage.rename_group(&self.patch, gid, &text);
                        let now = self.stage.group_name(gid);
                        if now != was {
                            self.log.push(format!("Renamed '{was}' → '{now}'"));
                        }
                    }
                    Renaming::Setup(old) => {
                        let new = text.trim().to_owned();
                        if new.is_empty() || new == old {
                            return;
                        }
                        if stage::StageView::rename_setup(&old, &new) {
                            self.log.push(format!("Setup '{old}' renamed to '{new}'"));
                        } else {
                            self.log.push(format!("A setup called '{new}' already exists"));
                        }
                        self.insp.build.setups_dirty = true;
                    }
                }
            }
            OutlinerAction::Hidden(r, hidden) => {
                let name = self.stage.element_name(r);
                self.stage.set_hidden(&self.patch, r, hidden);
                self.log.push(format!(
                    "{} '{name}'",
                    if hidden { "Hidden" } else { "Shown" }
                ));
            }
            OutlinerAction::HideGroup(gid, hidden) => {
                let name = self.stage.group_name(gid);
                self.stage.set_group_hidden(&self.patch, gid, hidden);
                self.log.push(format!(
                    "{} '{name}'",
                    if hidden { "Hidden" } else { "Shown" }
                ));
            }
            OutlinerAction::Delete(r) => {
                let name = self.stage.element_name(r);
                let lights = self.stage.delete_element(&self.patch, r);
                self.log.push(format!("Deleted '{name}' ({lights} lights unhung)"));
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
            }
            OutlinerAction::DeleteGroup(gid) => {
                let name = self.stage.group_name(gid);
                let (parts, lights) = self.stage.delete_group(&self.patch, gid);
                self.insp.build.confirm = None;
                self.insp.build.group_edit = None;
                self.log
                    .push(format!("Deleted '{name}' ({parts} parts, {lights} lights unhung)"));
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
            }
            OutlinerAction::Duplicate(r) => {
                let was = self.stage.element_name(r);
                if let Some(made) = self.stage.duplicate_element(&self.patch, r) {
                    let now = self.stage.element_name(made);
                    self.log.push(format!("Duplicated '{was}' → '{now}'"));
                    self.sync_selection_units();
                    self.sel_fixture = self.stage.last_selected;
                }
            }
            OutlinerAction::Mirror(r) => {
                let was = self.stage.element_name(r);
                if let Some(made) = self.stage.mirror_element_copy(&self.patch, r) {
                    let now = self.stage.element_name(made);
                    self.log.push(format!("Mirrored '{was}' → '{now}'"));
                    self.sync_selection_units();
                    self.sel_fixture = self.stage.last_selected;
                }
            }
            OutlinerAction::MirrorGroup(gid) => {
                let was = self.stage.group_name(gid);
                if let Some(made) = self.stage.mirror_group(&self.patch, gid) {
                    let now = self.stage.group_name(made);
                    self.log.push(format!("Mirrored '{was}' → '{now}'"));
                    self.sync_selection_units();
                    self.sel_fixture = self.stage.last_selected;
                }
            }
            OutlinerAction::Ungroup(gid) => {
                let name = self.stage.group_name(gid);
                self.stage.ungroup(&self.patch, gid);
                self.insp.build.group_edit = None;
                self.log.push(format!("Ungrouped '{name}'"));
            }
            OutlinerAction::Move(r, dir) => {
                let name = self.stage.element_name(r);
                let moved = self.stage.move_element(&self.patch, r, dir);
                if moved != r {
                    self.log.push(format!(
                        "Moved '{name}' {}",
                        if dir < 0 { "up" } else { "down" }
                    ));
                }
            }
            OutlinerAction::SelectMounted(r) => {
                let name = self.stage.element_name(r);
                let n = self.stage.select_mounted(r);
                if n > 0 {
                    self.sync_selection_units();
                    self.sel_fixture = self.stage.last_selected;
                    self.log.push(format!("Selected {n} lights on '{name}'"));
                }
            }
            OutlinerAction::Unhang(r) => {
                let name = self.stage.element_name(r);
                let n = self.stage.unhang_element(&self.patch, r);
                if n > 0 {
                    self.log.push(format!("Unhung {n} lights from '{name}'"));
                }
            }
            OutlinerAction::UnhangAll => {
                let n = self.stage.unhang_all(&self.patch);
                if n > 0 {
                    self.log.push(format!("Unhung {n} lights"));
                }
            }
            OutlinerAction::SelectLoose => {
                let loose: Vec<usize> = self
                    .stage
                    .instances
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| i.mount.is_none() && i.truss_mount.is_none())
                    .map(|(k, _)| k)
                    .collect();
                if loose.is_empty() {
                    return;
                }
                self.stage.clear_selection();
                self.stage.sel_stage = false;
                self.stage.last_selected =
                    loose.first().map(|&k| self.stage.instances[k].fixture);
                let n = loose.len();
                self.stage.selection.extend(loose);
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
                self.log.push(format!("Selected {n} lights that are not hung"));
            }
            OutlinerAction::SelectLight(inst, additive) => {
                if inst >= self.stage.instances.len() {
                    return;
                }
                if !additive {
                    self.stage.clear_selection();
                }
                self.stage.sel_stage = false;
                self.stage.sel_tower = None;
                self.stage.sel_truss = None;
                self.stage.selection.insert(inst);
                self.stage.last_selected = Some(self.stage.instances[inst].fixture);
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
            }
            OutlinerAction::Camera(r) => {
                if let Some(c) = self.stage.element_centre(r) {
                    self.stage.cam.target = c;
                    // A glide still in flight stamps its own pose over the
                    // camera later this frame, so it has to be called off.
                    self.stage.cam_tween = None;
                }
            }
            OutlinerAction::GroupEdit(gid) => {
                self.insp.build.group_edit = gid;
                self.insp.build.group_gesture = false;
            }
            OutlinerAction::EditInSelection(r) => {
                self.stage.select_element(r);
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
                self.insp.set_tab(InspectorTab::Selection);
            }
        }
    }

    /// The saved-setups section, inside its fold.
    fn build_setups(&mut self, ui: &mut egui::Ui) {
        let setups = self.insp.build.setups.clone();
        let confirm = self.insp.build.confirm.clone();
        let mut rename = self.insp.build.rename.clone();
        let mut rename_focus = self.insp.build.rename_focus;
        let mut act: Option<SetupAction> = None;

        let typed = self.setup_name.trim().to_owned();
        ui.add(
            egui::TextEdit::singleline(&mut self.setup_name)
                .hint_text("name this rig")
                .desired_width(f32::INFINITY),
        )
        .on_hover_text("What to call this arrangement when it is saved.");
        let stem = stage::StageView::setup_path(&typed)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();
        if !typed.is_empty() && stem != typed {
            theme::hint(ui, format!("saved as '{stem}'"));
        }
        let gate = if typed.is_empty() { Err("Type a name first.") } else { Ok(()) };
        let exists = setups.iter().any(|s| s.name == stem);
        theme::toolbar(ui, |ui| {
            if theme::tool_button(
                ui,
                Icon::Save,
                Some("Save"),
                "Save every light position, copy, tower and truss under this name.",
                gate,
            )
            .clicked()
            {
                act = Some(if exists {
                    SetupAction::Overwrite(typed.clone(), false)
                } else {
                    SetupAction::Save(typed.clone(), false)
                });
            }
            if theme::tool_button(
                ui,
                Icon::Save,
                Some("Rig only"),
                "Save just the towers and trusses as a template — no light positions.",
                gate,
            )
            .clicked()
            {
                act = Some(if exists {
                    SetupAction::Overwrite(typed.clone(), true)
                } else {
                    SetupAction::Save(typed.clone(), true)
                });
            }
            if theme::tool_button(ui, Icon::Refresh, None, "Re-read the setups folder.", Ok(()))
                .clicked()
            {
                act = Some(SetupAction::Refresh);
            }
        });
        ui.add_space(4.0);

        if setups.is_empty() {
            theme::empty_state(
                ui,
                Icon::Folder,
                "No setups saved",
                "Name the rig above and press Save.",
            );
        }
        for info in &setups {
            let renaming =
                rename.as_ref().is_some_and(|(r, _)| *r == Renaming::Setup(info.name.clone()));
            ui.push_id(("insp_build_setup", &info.name), |ui| {
                ui.horizontal(|ui| {
                    if renaming {
                        if let Some((_, buf)) = rename.as_mut() {
                            let r = ui.add(
                                egui::TextEdit::singleline(buf)
                                    .desired_width(f32::INFINITY)
                                    .id_salt("insp_build_rename_setup"),
                            )
                            .on_hover_text(
                                "Type a new name for this setup. Enter saves it; an                                  existing name is refused.",
                            );
                            if rename_focus {
                                r.request_focus();
                                rename_focus = false;
                            }
                            if r.lost_focus() {
                                act = Some(SetupAction::Rename(info.name.clone()));
                            }
                        }
                        return;
                    }
                    let caption = setup_caption(info);
                    let rig_only = info.lights == 0 && info.towers + info.trusses > 0;
                    let badges: &[(&str, Color32)] =
                        if rig_only { &[("rig", theme::ACCENT_SOFT)] } else { &[] };
                    let w = (ui.available_width() - small_button_width(ui, "Load")).max(40.0);
                    let row = ui
                        .allocate_ui(vec2(w, 0.0), |ui| {
                            theme::list_row(
                                ui,
                                ("insp_build_srow", &info.name),
                                &theme::Row {
                                    swatch: None,
                                    icon: Some(Icon::Folder),
                                    title: &info.name,
                                    subtitle: Some(&caption),
                                    badges,
                                    selected: false,
                                    active: false,
                                    indent: 0.0,
                                },
                            )
                        })
                        .inner
                        .on_hover_text(
                            "A saved arrangement. Right-click for the partial loads, \
                             overwrite, rename and delete.",
                        );
                    if row.clicked() {
                        act = Some(SetupAction::Load(info.name.clone()));
                    }
                    row.context_menu(|ui| {
                        if ui.button("Load").clicked() {
                            act = Some(SetupAction::Load(info.name.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Load rig only").clicked() {
                            act = Some(SetupAction::LoadRig(info.name.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Load lights only").clicked() {
                            act = Some(SetupAction::LoadLights(info.name.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Add to current rig").clicked() {
                            act = Some(SetupAction::Merge(info.name.clone()));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Overwrite with current").clicked() {
                            act = Some(SetupAction::Overwrite(info.name.clone(), false));
                            ui.close_menu();
                        }
                        if ui.button("Rename…").clicked() {
                            act = Some(SetupAction::Rename(info.name.clone()));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Delete…").clicked() {
                            act = Some(SetupAction::Delete(info.name.clone()));
                            ui.close_menu();
                        }
                    });
                    if ui
                        .small_button("Load")
                        .on_hover_text(
                            "Replace the current arrangement with this setup. ⌘Z cannot \
                             undo a load; Ctrl+Shift+Z can.",
                        )
                        .clicked()
                    {
                        act = Some(SetupAction::Load(info.name.clone()));
                    }
                });
            });
            match &confirm {
                Some(Confirm::OverwriteSetup(n)) if *n == info.name => {
                    let q = format!("Overwrite '{n}' with the current rig?");
                    match confirm_card(ui, &q, "Overwrite") {
                        Some(true) => act = Some(SetupAction::Save(n.clone(), false)),
                        Some(false) => self.insp.build.confirm = None,
                        None => {}
                    }
                }
                Some(Confirm::OverwriteSetupRig(n)) if *n == info.name => {
                    let q = format!("Overwrite '{n}' with the current rig only?");
                    match confirm_card(ui, &q, "Overwrite") {
                        Some(true) => act = Some(SetupAction::Save(n.clone(), true)),
                        Some(false) => self.insp.build.confirm = None,
                        None => {}
                    }
                }
                Some(Confirm::DeleteSetup(n)) if *n == info.name => {
                    let q = format!("Delete '{n}'? This cannot be undone.");
                    match confirm_card(ui, &q, "Delete") {
                        Some(true) => {
                            stage::StageView::delete_setup(n);
                            self.log.push(format!("Setup '{n}' deleted"));
                            self.insp.build.confirm = None;
                            self.insp.build.setups_dirty = true;
                        }
                        Some(false) => self.insp.build.confirm = None,
                        None => {}
                    }
                }
                _ => {}
            }
        }

        self.insp.build.rename = rename;
        self.insp.build.rename_focus = rename_focus;
        let Some(act) = act else { return };
        match act {
            SetupAction::Refresh => {
                self.insp.build.setups_dirty = true;
                self.log.push("Setups folder re-read".into());
            }
            SetupAction::Save(name, rig_only) => {
                let ok = if rig_only {
                    self.stage.save_setup_rig(&self.patch, &name)
                } else {
                    self.stage.save_setup(&self.patch, &name)
                };
                let s = self.stage.rig_summary();
                if ok {
                    self.log.push(if rig_only {
                        format!(
                            "Rig '{name}' saved ({} elements)",
                            s.towers + s.trusses
                        )
                    } else {
                        format!(
                            "Setup '{name}' saved ({} elements, {} lights)",
                            s.towers + s.trusses,
                            self.stage.instances.len()
                        )
                    });
                    self.setup_name.clear();
                } else {
                    self.log.push(format!("Setup '{name}' could not be saved"));
                }
                self.insp.build.confirm = None;
                self.insp.build.setups_dirty = true;
            }
            SetupAction::Overwrite(name, rig_only) => {
                let stem = stage::StageView::setup_path(&name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&name)
                    .to_owned();
                self.insp.build.confirm = Some(if rig_only {
                    Confirm::OverwriteSetupRig(stem)
                } else {
                    Confirm::OverwriteSetup(stem)
                });
            }
            SetupAction::Delete(name) => {
                self.insp.build.confirm = Some(Confirm::DeleteSetup(name));
            }
            SetupAction::Rename(name) => {
                match self.insp.build.rename.clone() {
                    Some((Renaming::Setup(old), _)) if old == name => {
                        self.apply_outliner_action(OutlinerAction::CommitRename);
                    }
                    _ => {
                        self.insp.build.rename = Some((Renaming::Setup(name.clone()), name));
                        self.insp.build.rename_focus = true;
                    }
                }
            }
            SetupAction::Load(name) => {
                if self.stage.load_setup(&self.patch, &self.settings, &name) {
                    self.log.push(format!("Setup '{name}' loaded"));
                } else {
                    self.log.push(format!("Setup '{name}' failed to load"));
                }
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
            }
            SetupAction::LoadRig(name) => match stage::StageView::read_setup(&name) {
                Some(lf) => {
                    self.stage.import_rig(&self.patch, lf);
                    self.log
                        .push(format!("Rig from '{name}' loaded — lights left where they were"));
                }
                None => self.log.push(format!("Setup '{name}' failed to load")),
            },
            SetupAction::LoadLights(name) => {
                if self.stage.load_setup_lights(&self.patch, &self.settings, &name) {
                    self.log
                        .push(format!("Light positions from '{name}' loaded — rig kept"));
                } else {
                    self.log.push(format!("Setup '{name}' failed to load"));
                }
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
            }
            SetupAction::Merge(name) => match stage::StageView::read_setup(&name) {
                Some(lf) => {
                    let n = self.stage.merge_rig(&self.patch, lf);
                    self.log.push(format!("Added {n} elements from '{name}'"));
                }
                None => self.log.push(format!("Setup '{name}' failed to load")),
            },
        }
    }

    /// Clearing the rig and the existing full reset.
    fn build_reset(&mut self, ui: &mut egui::Ui) {
        let summary = self.stage.rig_summary();
        let elements = summary.towers + summary.trusses;
        let mut clear_ask = false;
        let mut reset = false;
        theme::section(ui, "Reset");
        theme::toolbar(ui, |ui| {
            // `gated` cannot wrap inside a toolbar, so both of these carry
            // their own reason instead; the WARN card below is the guard.
            let gate = if elements == 0 { Err("Nothing to clear.") } else { Ok(()) };
            if theme::tool_button(
                ui,
                Icon::Trash,
                Some("Clear rig…"),
                "Remove every tower and truss. Lights stay where they hang, unhung. Asks first.",
                gate,
            )
            .clicked()
            {
                clear_ask = true;
            }
            if theme::tool_button(
                ui,
                Icon::Reset,
                Some("Reset lights…"),
                "Move every light back to its default position and remove copies, towers and trusses. Asks first.",
                Ok(()),
            )
            .clicked()
            {
                reset = true;
            }
        });
        if clear_ask {
            self.insp.build.confirm = Some(Confirm::ClearRig);
        }
        if reset {
            self.confirm_reset = true;
        }
        if self.insp.build.confirm == Some(Confirm::ClearRig) {
            let q = format!(
                "Clear {elements} elements? {} lights will be unhung and stay where they are.",
                summary.slots_used
            );
            match confirm_card(ui, &q, "Clear") {
                Some(true) => {
                    let (n, lights) = self.stage.clear_rig(&self.patch);
                    self.log.push(format!(
                        "Rig cleared: {n} elements removed, {lights} lights unhung"
                    ));
                    self.insp.build.confirm = None;
                    self.sync_selection_units();
                    self.sel_fixture = self.stage.last_selected;
                }
                Some(false) => self.insp.build.confirm = None,
                None => {}
            }
        }
        theme::hint(ui, "Save a setup first if you want to come back to this arrangement.");
    }
}

/// One element row, plus its hung lights when they are showing.
#[allow(clippy::too_many_arguments)]
fn element_row(
    ui: &mut egui::Ui,
    info: &ElementInfo,
    lights: &[LightRow],
    sel: Option<ElementRef>,
    in_group: bool,
    rename: &mut Option<(Renaming, String)>,
    rename_focus: &mut bool,
    act: &mut Option<OutlinerAction>,
) {
    let z = theme::zoom_of(ui);
    let renaming = rename.as_ref().is_some_and(|(r, _)| *r == Renaming::Element(info.r));
    // The slot fill already reads in the caption, so the row keeps only the
    // badge that changes what the stage does — and says "hidden" in the
    // caption too, since a 230 px row has no room for badges.
    let badges: &[(&str, Color32)] =
        if info.hidden { &[("hidden", theme::WARN)] } else { &[] };
    let caption = if info.hidden {
        format!("hidden · {}", info.caption)
    } else {
        info.caption.clone()
    };
    ui.push_id(("insp_build_row", info.r), |ui| {
        ui.horizontal(|ui| {
            // Keep the Eye button's exact width clear, so the row can
            // never push the panel wider than it is.
            let eye = theme::tool_size(ui, None).x + ui.spacing().item_spacing.x + 1.0;
            let w = (ui.available_width() - eye).max(40.0);
            ui.allocate_ui(vec2(w, 0.0), |ui| {
                if renaming {
                    if let Some((_, buf)) = rename.as_mut() {
                        let r = ui.add(
                            egui::TextEdit::singleline(buf)
                                .desired_width(f32::INFINITY)
                                .id_salt(("insp_build_rename", info.r)),
                        )
                        .on_hover_text(
                            "Type a name for this element. Enter saves, Escape cancels,                              and an empty name goes back to the default.",
                        );
                        if *rename_focus {
                            r.request_focus();
                            *rename_focus = false;
                        }
                        if r.lost_focus() {
                            *act = Some(OutlinerAction::CommitRename);
                        }
                        if ui.input(|i| i.key_pressed(Key::Escape)) {
                            *act = Some(OutlinerAction::CancelRename);
                        }
                    }
                    return;
                }
                // Under a group header the name is already on the header,
                // so the row shows only its role ("beam", "left leg").
                let title = match (in_group, info.label.split_once(" · ")) {
                    (true, Some((_, role))) if !role.trim().is_empty() => role.trim(),
                    _ => info.label.as_str(),
                };
                let row = theme::list_row(
                    ui,
                    ("insp_build_lrow", info.r),
                    &theme::Row {
                        swatch: None,
                        icon: Some(kind_icon(info.glyph)),
                        title,
                        subtitle: Some(&caption),
                        badges,
                        selected: sel == Some(info.r),
                        active: false,
                        indent: if in_group { 14.0 * z } else { 0.0 },
                    },
                );
                if row.hovered() {
                    *act = Some(OutlinerAction::Hover(info.r));
                }
                let row = row.on_hover_text(
                    "Click to pick it on the stage, double-click to rename it, right-click \
                     for duplicate, mirror, hide and delete.",
                );
                if row.clicked() {
                    *act = Some(OutlinerAction::Select(info.r));
                }
                if row.double_clicked() {
                    *act = Some(OutlinerAction::Rename(info.r));
                }
                row.context_menu(|ui| {
                    if ui.button("Rename (F2)").clicked() {
                        *act = Some(OutlinerAction::Rename(info.r));
                        ui.close_menu();
                    }
                    if ui.button("Duplicate").clicked() {
                        *act = Some(OutlinerAction::Duplicate(info.r));
                        ui.close_menu();
                    }
                    if ui
                        .button("Mirror across centre")
                        .on_hover_text("Make a reflected copy across the stage's centre line. Lights are not copied.")
                        .clicked()
                    {
                        *act = Some(OutlinerAction::Mirror(info.r));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Move up").clicked() {
                        *act = Some(OutlinerAction::Move(info.r, -1));
                        ui.close_menu();
                    }
                    if ui.button("Move down").clicked() {
                        *act = Some(OutlinerAction::Move(info.r, 1));
                        ui.close_menu();
                    }
                    ui.separator();
                    let hung = info.mounted > 0;
                    if ui
                        .add_enabled(
                            hung,
                            egui::Button::new(format!("Select hung lights ({})", info.mounted)),
                        )
                        .clicked()
                    {
                        *act = Some(OutlinerAction::SelectMounted(info.r));
                        ui.close_menu();
                    }
                    if ui.add_enabled(hung, egui::Button::new("Unhang lights")).clicked() {
                        *act = Some(OutlinerAction::Unhang(info.r));
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(hung, egui::Button::new("List its lights"))
                        .on_hover_text("Show the lights hung on this one element, with the Lights toggle off.")
                        .clicked()
                    {
                        *act = Some(OutlinerAction::ToggleExpand(info.r));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(if info.hidden { "Show" } else { "Hide" }).clicked() {
                        *act = Some(OutlinerAction::Hidden(info.r, !info.hidden));
                        ui.close_menu();
                    }
                    if ui.button("Point camera here").clicked() {
                        *act = Some(OutlinerAction::Camera(info.r));
                        ui.close_menu();
                    }
                    if ui.button("Edit in Selection tab").clicked() {
                        *act = Some(OutlinerAction::EditInSelection(info.r));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Delete (⌫)").clicked() {
                        *act = Some(OutlinerAction::Delete(info.r));
                        ui.close_menu();
                    }
                });
            });
            if !renaming
                && theme::tool_button(
                    ui,
                    if info.hidden { Icon::EyeOff } else { Icon::Eye },
                    None,
                    if info.hidden {
                        "Show this element on the stage again."
                    } else {
                        "Hide this element on the stage. Its lights stay hung and follow it; \
                         hidden elements cannot be clicked or snapped to."
                    },
                    Ok(()),
                )
                .clicked()
            {
                *act = Some(OutlinerAction::Hidden(info.r, !info.hidden));
            }
        });
        for light in lights {
            let indent = if in_group { 42.0 * z } else { 28.0 * z };
            let row = ui
                .push_id(("insp_build_light", light.inst), |ui| {
                    theme::list_row(
                        ui,
                        ("insp_build_lightrow", light.inst),
                        &theme::Row {
                            swatch: Some(light.color),
                            icon: None,
                            title: &light.title,
                            subtitle: Some(&light.subtitle),
                            badges: &[],
                            selected: light.selected,
                            active: false,
                            indent,
                        },
                    )
                })
                .inner
                .on_hover_text("One light hung on this element. Click to select it; ⇧-click to add it.");
            if row.clicked() {
                let additive = ui.input(|i| i.modifiers.shift);
                *act = Some(OutlinerAction::SelectLight(light.inst, additive));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{Frame, DMX_SLOTS};
    use crate::stage::headless::{render_frames, save};
    use crate::stage::v3;

    /// How wide the docked Inspector ended up. The panel is content-sized,
    /// so anything in the tab that will not wrap shows up here.
    fn panel_width(ctx: &egui::Context) -> f32 {
        egui::panel::PanelState::load(ctx, egui::Id::new("inspector"))
            .map_or(0.0, |p| p.rect.width())
    }

    fn lit(pixels: &[u8]) -> usize {
        pixels.chunks(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120).count()
    }

    /// "Point camera here" writes `cam.target` straight onto the camera, and
    /// `step_cam_tween` overwrites the whole camera from a glide's own
    /// snapshot later in the same frame — so during a bookmark recall or a
    /// camera tour the menu item used to do nothing at all.
    #[test]
    fn point_camera_here_calls_off_a_glide_in_flight() {
        let mut app = App::new();
        app.stage.layout_path = std::env::temp_dir().join("dmxpress_build_point_cam.json");
        app.stage.towers.clear();
        app.stage.trusses.clear();
        app.stage.build(
            &app.patch,
            Recipe::default_for(RecipeKind::Tower),
            Placement::At(v3(2.0, 0.0, -1.0)),
        );
        let r = ElementRef::Tower(0);
        let centre = app.stage.element_centre(r).expect("the tower has a centre");
        let mut away = app.stage.cam.snapshot();
        away.target = v3(9.0, 9.0, 9.0);
        app.stage.go_to(away, true);
        assert!(app.stage.cam_tween.is_some(), "no glide to interrupt");
        app.apply_outliner_action(OutlinerAction::Camera(r));
        assert!(app.stage.cam_tween.is_none(), "the glide would stamp over the new target");
        assert_eq!(app.stage.cam.target, centre);
        let _ = std::fs::remove_file(&app.stage.layout_path);
    }

    #[test]
    fn place_modes_round_trip_and_read_as_sentences() {
        for (i, a) in PlaceMode::ALL.iter().enumerate() {
            assert_eq!(PlaceMode::from_key(a.key()), Some(*a));
            assert!(a.hint().ends_with('.'), "{}", a.label());
            for b in &PlaceMode::ALL[i + 1..] {
                assert_ne!(a.key(), b.key());
                assert_ne!(a.label(), b.label());
            }
        }
        assert_eq!(PlaceMode::from_key("nope"), None);
    }

    #[test]
    fn placement_world_follows_the_stage_box() {
        let set = crate::stage::Settings::default();
        let mut view = crate::stage::StageView::new();
        view.layout_path = std::env::temp_dir().join("dmxpress_build_placement.json");
        let mut ui = BuildUi::default();
        assert_eq!(ui.placement_world(&set, &view), Placement::Stagger);
        ui.place = PlaceMode::Centre;
        assert_eq!(ui.placement_world(&set, &view), Placement::At(v3(0.0, 0.0, 0.0)));
        ui.place = PlaceMode::Upstage;
        match ui.placement_world(&set, &view) {
            Placement::At(p) => assert!(p.z < 0.0 && p.z > -set.stage_half_d),
            _ => panic!("a spot was expected"),
        }
        ui.place = PlaceMode::Behind;
        match ui.placement_world(&set, &view) {
            Placement::At(p) => assert!(p.z < -set.stage_half_d),
            _ => panic!("a spot was expected"),
        }
        ui.place = PlaceMode::Camera;
        view.cam.target = v3(99.0, 2.0, -3.0);
        assert_eq!(ui.placement_world(&set, &view), Placement::At(v3(20.0, 0.0, -3.0)));
        ui.place = PlaceMode::Custom;
        ui.custom_xz = (1.5, -2.5);
        assert_eq!(ui.placement_world(&set, &view), Placement::At(v3(1.5, 0.0, -2.5)));
        // Every pad keeps its own parameters.
        assert_eq!(ui.recipes[RecipeKind::Tower as usize], Recipe::default_for(RecipeKind::Tower));
        assert_eq!(ui.recipe(), Recipe::default_for(RecipeKind::Straight));
    }

    #[test]
    fn short_date_reads_as_a_day_and_a_month() {
        let d = |secs: u64| {
            short_date(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
        };
        assert_eq!(d(0), "1 Jan");
        // 2024-09-14T00:00:00Z.
        assert_eq!(d(1_726_272_000), "14 Sep");
        // 2000-02-29T12:00:00Z — a leap day.
        assert_eq!(d(951_825_600), "29 Feb");
    }

    #[test]
    fn setup_captions_summarise_the_file() {
        let info = SetupInfo {
            name: "Club A".into(),
            towers: 2,
            trusses: 3,
            lights: 12,
            modified: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_726_272_000)),
        };
        assert_eq!(setup_caption(&info), "2 towers · 3 trusses · 12 lights · 14 Sep");
        let rig = SetupInfo { lights: 0, trusses: 1, modified: None, ..info.clone() };
        assert_eq!(setup_caption(&rig), "2 towers · 1 truss");
        let empty = SetupInfo { towers: 0, trusses: 0, lights: 0, ..rig };
        assert_eq!(setup_caption(&empty), "empty");
    }

    #[test]
    fn build_prefs_round_trip() {
        let d: BuildPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(d, BuildPrefs::default());
        let p = BuildPrefs {
            kind: Some(RecipeKind::Arch.key().to_owned()),
            place: Some(PlaceMode::Centre.key().to_owned()),
            show_lights: true,
        };
        let back: BuildPrefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        assert_eq!(RecipeKind::from_key(&p.kind.unwrap()), Some(RecipeKind::Arch));
    }

    /// The whole tab at the panel's 230 px minimum, then at zoom 2.0, then
    /// empty — written to `target/inspector_build*.png`. Read-only on the
    /// working directory: the layout goes to a temp file, `prefs.tab` is
    /// poked directly and no setup file is ever touched.
    #[test]
    fn build_tab_renders_headless() {
        let mut app = crate::app::App::new();
        app.stage.layout_path = std::env::temp_dir().join("dmxpress_build_tab_test_layout.json");
        let _ = std::fs::remove_file(&app.stage.layout_path);
        *app.net.dmx.lock() = Frame([170u8; DMX_SLOTS]);
        app.collapsed.insert("channels");
        app.collapsed.insert("fixtures");
        app.collapsed.remove("inspector");
        app.show_log = false;
        app.show_osc = false;
        app.insp.prefs.width = Some(230.0);
        app.insp.prefs.tab = InspectorTab::Build;
        // A rig with a composite, a couple of towers, a straight and a
        // radius run, so every kind of row is on screen.
        app.stage.clear_rig(&app.patch);
        app.stage.build(&app.patch, Recipe::default_for(RecipeKind::Goalpost), Placement::At(v3(0.0, 0.0, -3.0)));
        app.stage.build(&app.patch, Recipe::default_for(RecipeKind::Tower), Placement::Stagger);
        app.stage.build(&app.patch, Recipe::default_for(RecipeKind::Tower), Placement::Stagger);
        app.stage.build(
            &app.patch,
            Recipe::Straight { length: 2.0, height: 3.2, yaw: 90.0, grounded: true },
            Placement::Stagger,
        );
        app.stage.build(&app.patch, Recipe::default_for(RecipeKind::Radius), Placement::Stagger);
        app.stage.rename_element(&app.patch, ElementRef::Tower(2), "SL stand");
        app.stage.set_hidden(&app.patch, ElementRef::Truss(2), true);
        for (k, inst) in app.stage.instances.iter_mut().take(2).enumerate() {
            inst.truss_mount = Some((0, k));
        }
        app.stage.select_element(ElementRef::Truss(0));
        app.insp.build.kind = RecipeKind::Radius;
        app.insp.build.show_lights = true;
        app.insp.build.seeded = true;
        app.insp.build.setups = vec![
            SetupInfo { name: "Club A".into(), towers: 2, trusses: 3, lights: 12, modified: None },
            SetupInfo { name: "Festival main".into(), towers: 4, trusses: 0, lights: 0, modified: None },
        ];
        app.insp.build.setups_dirty = false;

        // Structural facts first, so they hold on a machine with no GPU.
        assert_eq!(app.stage.towers.len(), 4);
        assert_eq!(app.stage.trusses.len(), 3);
        assert_eq!(app.stage.elements().len(), 7);
        assert_eq!(app.stage.element_name(ElementRef::Tower(2)), "SL stand");
        assert_eq!(app.stage.element_name(ElementRef::Tower(0)), "Goalpost 1 · left leg");
        assert_eq!(app.stage.group_name(1), "Goalpost 1");
        assert_eq!(app.stage.group_members(1).len(), 3);
        assert_eq!(app.stage.rig_summary().slots_used, 2);

        let size = [1000, 1500];
        let mut widths: Vec<f32> = Vec::new();
        let Some(pixels) = render_frames(5, size, |ctx, frame| {
            if frame == 0 {
                crate::ui::install_theme(ctx);
            } else {
                app.draw_ui(ctx);
                widths.push(panel_width(ctx));
            }
        }) else {
            eprintln!("no GPU adapter — skipping");
            let _ = std::fs::remove_file(&app.stage.layout_path);
            return;
        };
        save(&pixels, size, "inspector_build_headless");
        assert!(lit(&pixels) > 1000, "the Build tab came out black");
        // The panel is content-sized: anything that will not wrap pushes it
        // wider than the 230 px minimum, and keeps pushing every frame.
        let last = *widths.last().expect("a frame was drawn");
        assert!(
            last <= 231.0,
            "the Build tab widened the Inspector to {last} px (frames: {widths:?})"
        );

        // Scene 2: the confirm card, a rename field and the group Δ grid,
        // at double zoom.
        app.zoom.inspector = 2.0;
        app.insp.build.confirm = Some(Confirm::ClearRig);
        app.insp.build.rename =
            Some((Renaming::Element(ElementRef::Tower(0)), "US left leg".into()));
        app.insp.build.group_edit = Some(1);
        // The shortest card and a folded Setups section, so the outliner,
        // the group grid and the confirm card all fit on one tall canvas.
        app.insp.build.kind = RecipeKind::Tower;
        app.insp.prefs.folded.insert("build.setups".into());
        let tall = [900, 2900];
        let mut zoom_widths: Vec<f32> = Vec::new();
        let Some(pixels) = render_frames(5, tall, |ctx, frame| {
            if frame == 0 {
                crate::ui::install_theme(ctx);
            } else {
                app.draw_ui(ctx);
                zoom_widths.push(panel_width(ctx));
            }
        }) else {
            let _ = std::fs::remove_file(&app.stage.layout_path);
            return;
        };
        save(&pixels, tall, "inspector_build_headless_zoom");
        assert!(lit(&pixels) > 1000, "the zoomed Build tab came out black");
        let last = *zoom_widths.last().expect("a frame was drawn");
        assert!(last <= 470.0, "the zoomed Build tab widened the Inspector to {last} px");

        // Scene 3: nothing built and nothing saved — both empty states.
        app.zoom.inspector = 1.0;
        app.insp.prefs.folded.remove("build.setups");
        app.stage.clear_rig(&app.patch);
        app.insp.build.confirm = None;
        app.insp.build.rename = None;
        app.insp.build.group_edit = None;
        app.insp.build.setups.clear();
        app.insp.build.kind = RecipeKind::Box;
        app.insp.build.place = PlaceMode::Custom;
        let mut empty_widths: Vec<f32> = Vec::new();
        let Some(pixels) = render_frames(5, size, |ctx, frame| {
            if frame == 0 {
                crate::ui::install_theme(ctx);
            } else {
                app.draw_ui(ctx);
                empty_widths.push(panel_width(ctx));
            }
        }) else {
            let _ = std::fs::remove_file(&app.stage.layout_path);
            return;
        };
        save(&pixels, size, "inspector_build_headless_empty");
        assert!(lit(&pixels) > 1000, "the empty Build tab came out black");
        let last = *empty_widths.last().expect("a frame was drawn");
        assert!(last <= 231.0, "the empty Build tab widened the Inspector to {last} px");
        assert!(!app.insp.dirty, "a headless render must not dirty the prefs");
        let _ = std::fs::remove_file(&app.stage.layout_path);
    }
}
