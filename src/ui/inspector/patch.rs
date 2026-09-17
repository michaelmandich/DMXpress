//! Inspector · Patch tab (area F): a quick-patch wizard — recents, the
//! fixture library, the built-in profiles, addressing, naming and where the
//! batch lands — plus the rig summary and a two-step unpatch.
//!
//! The wizard reads top to bottom: pick a model, check the preview, set
//! count / address / name, choose a shape or an element to hang on, press
//! Patch. One press patches, places (or hangs) and selects, so a rig goes
//! up in one pass instead of a light at a time.
//!
//! Patch state lives on `App` (`patch_search`, `patch_profile`,
//! `patch_library_sel`, `patch_name`, `patch_addr`, `patch_count`), shared
//! with the legacy Patch window so the two never disagree. Everything the
//! wizard adds is in [`PatchUi`] (per session) and [`PatchPrefs`] (saved).

use std::collections::HashSet;
use std::time::Instant;

use eframe::egui::{self, Color32};
use serde::{Deserialize, Serialize};

use super::addressing::{self, PlanStop};
use super::{type_stem, InspectorTab};
use crate::app::App;
use crate::fixturedb::{self, Library};
use crate::net::DMX_SLOTS;
use crate::profiles::{self, UserFixture, PROFILES};
use crate::showbuddy::{Patch, Role};
use crate::stage::{
    self, v3, Archetype, ElementRef, PlacePattern, PlaceSpec, TrussKind, TOWER_SLOTS,
};
use crate::ui::icons::Icon;
use crate::ui::patchcfg::PatchSelection;
use crate::ui::theme::{self, PadFace, PadState, Tone};

/// How long a two-step confirm stays armed.
const CONFIRM_SECS: f32 = 4.0;
/// Slot pitch on a truss face — a run this long per light holds exactly one.
const SLOT_PITCH: f32 = 0.5;

/// Which fixture source the picker is showing.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PatchSource {
    #[default]
    Builtin,
    Library,
}

/// The coarse kind a library category maps onto, for the filter chips.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ArchClass {
    Moving,
    Par,
    Bar,
    Strobe,
    Other,
}

impl ArchClass {
    const ALL: [ArchClass; 5] =
        [Self::Moving, Self::Par, Self::Bar, Self::Strobe, Self::Other];

    fn label(self) -> &'static str {
        match self {
            Self::Moving => "Moving",
            Self::Par => "Par",
            Self::Bar => "Bar",
            Self::Strobe => "Strobe",
            Self::Other => "Other",
        }
    }
}

/// What the start address follows while auto-follow is on.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AddrMode {
    /// One past the last patched light.
    #[default]
    Append,
    /// The first hole big enough for the batch.
    FirstGap,
}

/// What a hung batch hangs on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ElementChoice {
    NewTruss(TrussKind),
    NewTower,
    Existing(ElementRef),
}

/// What a pending confirm would unpatch.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum UnpatchScope {
    /// Everything selected on the stage.
    Selection,
    /// Every light of one type (its `Fixture.file`).
    Type(String),
    /// One light, by `display@from`.
    One(String),
}

/// One library search hit, owned so the row loop can change the pick.
#[derive(Clone)]
pub(crate) struct Hit {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub class: ArchClass,
}

/// One batch to patch. The only way fixtures enter the rig from this tab.
pub(crate) struct PatchRequest {
    pub profile: String,
    pub label: String,
    pub span: u16,
    pub base_name: String,
    pub first_number: u16,
    pub count: u16,
    pub start: u16,
    pub skip_taken: bool,
    pub same_universe: bool,
}

/// What a batch left behind: the layout keys of the new lights (the only
/// handle that survives `rebuild_patch`), whether it ran out of slots, and
/// where the next batch would start.
#[derive(Default)]
pub(crate) struct PatchOutcome {
    pub keys: Vec<String>,
    pub stopped: bool,
    pub next_addr: u16,
}

/// One row of the rig summary: every light sharing a fixture definition.
pub(crate) struct RigGroup {
    pub file: String,
    pub label: String,
    pub count: usize,
    pub channels: usize,
    pub from_min: u16,
    pub to_max: u16,
    pub first_fi: usize,
    pub fis: Vec<usize>,
    /// DMXpress patched it, so it can be added to and unpatched here.
    pub is_user: bool,
    pub profile: Option<String>,
}

/// A rig row's deferred action, applied after the list is drawn.
enum RigAction {
    SelectAll(usize),
    AddOne(usize),
    UnpatchType(usize),
    Toggle(String),
    SelectOne(usize, bool),
    UnpatchOne(String),
    Rename(usize),
    RenameDone(String, String),
}

/// Patch-tab preferences, persisted at `InspectorPrefs.patch`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PatchPrefs {
    pub pattern: PlacePattern,
    pub spacing: f32,
    pub skip_taken: bool,
    pub same_universe: bool,
}

impl Default for PatchPrefs {
    fn default() -> Self {
        Self {
            pattern: PlacePattern::LineX,
            spacing: 1.0,
            skip_taken: true,
            same_universe: true,
        }
    }
}

/// Everything the wizard remembers for this session.
pub(crate) struct PatchUi {
    pub source: PatchSource,
    pub arch_filter: Option<ArchClass>,
    pub hits: Vec<Hit>,
    pub hits_key: Option<(String, Option<String>, Option<ArchClass>)>,
    pub hit_cursor: Option<usize>,
    /// Filled once by `ensure_library`, so the maker list costs nothing.
    pub makers: Vec<String>,
    pub preview_key: Option<String>,
    pub preview_arch: Option<Archetype>,
    pub preview_roles: Vec<Role>,
    pub first_number: u16,
    pub number_auto: bool,
    pub addr_auto: bool,
    /// The address auto-follow last wrote; a different value means the
    /// legacy Patch window edited the field, so auto-follow steps aside.
    pub addr_seen: u16,
    pub addr_mode: AddrMode,
    pub height: f32,
    pub height_seeded: bool,
    pub fit_to_stage: bool,
    pub row: f32,
    pub columns: u16,
    pub radius: f32,
    pub arc_deg: f32,
    pub face_centre: bool,
    pub element: ElementChoice,
    pub face: usize,
    pub fit_length: bool,
    pub new_length: f32,
    pub select_after: bool,
    pub confirm_unpatch: Option<(UnpatchScope, Instant)>,
    pub sel_fingerprint: Vec<usize>,
    pub open_groups: HashSet<String>,
    /// (fixture file, draft prefix) while a type is being renamed.
    pub rename: Option<(String, String)>,
}

impl Default for PatchUi {
    fn default() -> Self {
        Self {
            source: PatchSource::Builtin,
            arch_filter: None,
            hits: Vec::new(),
            hits_key: None,
            hit_cursor: None,
            makers: Vec::new(),
            preview_key: None,
            preview_arch: None,
            preview_roles: Vec::new(),
            first_number: 1,
            number_auto: true,
            addr_auto: true,
            addr_seen: 0,
            addr_mode: AddrMode::Append,
            height: 4.0,
            height_seeded: false,
            fit_to_stage: true,
            row: 0.0,
            columns: 0,
            radius: 3.0,
            arc_deg: 120.0,
            face_centre: true,
            element: ElementChoice::NewTruss(TrussKind::Straight),
            face: 1,
            fit_length: true,
            new_length: 3.0,
            select_after: true,
            confirm_unpatch: None,
            sel_fingerprint: Vec::new(),
            open_groups: HashSet::new(),
            rename: None,
        }
    }
}

/// The batch as it stands this frame — recomputed before any widget, so
/// the pills, the button label and the commit all agree.
struct Plan {
    span: u16,
    starts: Vec<u16>,
    stop: PlanStop,
    names: Vec<String>,
    /// Display names of the lights the batch would land on top of.
    clashes: Vec<String>,
    /// A `display@from` key the batch would duplicate.
    collision: bool,
    /// A hole big enough for the whole batch, when the plan is not using it.
    gap: Option<u16>,
}

impl Plan {
    fn first(&self) -> u16 {
        self.starts.first().copied().unwrap_or(0)
    }

    fn last(&self) -> u16 {
        self.starts.last().map_or(0, |a| a + self.span.max(1) - 1)
    }
}

/// Which coarse kind a library category string reads as.
pub(crate) fn category_class(category: &str) -> ArchClass {
    let c = category.to_lowercase();
    let has = |k: &str| c.contains(k);
    if has("moving") || has("scanner") || has("head") {
        ArchClass::Moving
    } else if has("par") || has("wash") || has("color changer") || has("flood") {
        ArchClass::Par
    } else if has("bar") || has("batten") || has("strip") || has("pixel") {
        ArchClass::Bar
    } else if has("strobe") || has("blinder") {
        ArchClass::Strobe
    } else {
        ArchClass::Other
    }
}

/// The rig grouped by fixture definition, in address order.
pub(crate) fn rig_groups(patch: &Patch, library: &Library) -> Vec<RigGroup> {
    let mut out: Vec<RigGroup> = Vec::new();
    for (fi, f) in patch.fixtures.iter().enumerate() {
        let file = f.file.to_string_lossy().to_string();
        if let Some(g) = out.iter_mut().find(|g| g.file == file) {
            g.count += 1;
            g.fis.push(fi);
            g.from_min = g.from_min.min(f.from);
            g.to_max = g.to_max.max(f.to);
            continue;
        }
        let (label, profile) = if let Some(name) = file.strip_prefix("builtin:") {
            (addressing::short_name(name), Some(name.to_string()))
        } else if let Some(id) = fixturedb::library_id(&file) {
            let label = match library.find(id) {
                Some(lf) => format!("{} {}", lf.model, lf.mode).trim().to_string(),
                None => {
                    let mut tail: Vec<&str> = id.rsplit('/').take(2).collect();
                    tail.reverse();
                    tail.join(" ")
                }
            };
            (label, Some(file.clone()))
        } else {
            let stem = std::path::Path::new(&file)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| file.clone());
            (stem, None)
        };
        out.push(RigGroup {
            is_user: profile.is_some(),
            file,
            label,
            count: 1,
            channels: f.channel_count(),
            from_min: f.from,
            to_max: f.to,
            first_fi: fi,
            fis: vec![fi],
            profile,
        });
    }
    out.sort_by_key(|g| g.from_min);
    out
}

/// Was this light patched by DMXpress (rather than imported from ShowBuddy)?
fn is_user_fixture(file: &str) -> bool {
    file.starts_with("builtin:") || file.starts_with(fixturedb::LIB_PREFIX)
}

/// The archetype mark on the preview card: what the light physically is,
/// at a glance, before anything is patched.
fn arch_glyph(painter: &egui::Painter, rect: egui::Rect, arch: Option<Archetype>, color: Color32) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5;
    match arch {
        Some(Archetype::Par) => {
            painter.circle_filled(c, r * 0.78, color);
        }
        Some(Archetype::Bar) => {
            painter.rect_filled(
                egui::Rect::from_center_size(c, egui::vec2(r * 0.7, r * 1.7)),
                r * 0.25,
                color,
            );
        }
        Some(Archetype::MovingPar) => {
            painter.circle_stroke(c, r * 0.8, egui::Stroke::new(r * 0.22, color));
            painter.circle_filled(c, r * 0.3, color);
        }
        Some(Archetype::Beam) => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + egui::vec2(0.0, -r),
                    c + egui::vec2(r * 0.62, r * 0.85),
                    c + egui::vec2(-r * 0.62, r * 0.85),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        _ => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, egui::Vec2::splat(r * 1.3)),
                r * 0.2,
                egui::Stroke::new(1.5, color),
            );
        }
    }
}


/// How wide a full-row ComboBox may ask to be: the arrow and its padding
/// are added on top of the width it is given.
fn combo_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() - 28.0 * theme::zoom_of(ui)).max(40.0)
}

/// `text` cut to `max` pixels in `font`, ending in an ellipsis.
fn elide(ui: &egui::Ui, text: &str, font: egui::FontId, max: f32) -> String {
    let width = |s: &str| {
        ui.painter().layout_no_wrap(s.to_owned(), font.clone(), theme::TEXT).size().x
    };
    if width(text) <= max {
        return text.to_owned();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut candidate = out.clone();
        candidate.push(ch);
        candidate.push('\u{2026}');
        if width(&candidate) > max.max(12.0) {
            break;
        }
        out.push(ch);
    }
    out.push('\u{2026}');
    out
}

/// What a full-row ComboBox may show: its `width` is a floor, not a cap —
/// the button grows to fit whatever text is picked, and inside a narrow
/// grid cell that widens the whole panel.
fn combo_text(ui: &egui::Ui, text: &str) -> String {
    let font = egui::FontId::new(13.0 * theme::zoom_of(ui), egui::FontFamily::Proportional);
    elide(ui, text, font, (combo_width(ui) - 12.0 * theme::zoom_of(ui)).max(24.0))
}

/// A card title cut to leave `reserve` pixels for the pills beside it —
/// `card_title` lays its title out first, so a long one would push them
/// off the card and widen the whole panel.
fn clip_title(ui: &egui::Ui, text: &str, reserve: f32) -> String {
    let font = egui::FontId::new(13.0 * theme::zoom_of(ui), theme::semibold());
    elide(ui, text, font, (ui.available_width() - reserve).max(24.0))
}

/// How a picked pad is lit: the accent fill the hero button wears.
fn pad_state(picked: bool) -> PadState {
    PadState {
        fill: picked.then(|| theme::ACCENT.lerp_to_gamma(theme::SURFACE, 0.2)),
        ..Default::default()
    }
}

/// One word for what the light is.
fn arch_word(arch: Option<Archetype>) -> &'static str {
    match arch {
        Some(Archetype::Par) => "Par",
        Some(Archetype::Bar) => "Bar",
        Some(Archetype::MovingPar) => "Moving wash",
        Some(Archetype::Beam) => "Beam",
        _ => "Other",
    }
}

/// Lay `n` pads of at least `min_w` out in rows that fit the panel. The
/// height keeps every pad at least 46 px, the point below which
/// `theme::paint_pad` drops the name under the face.
fn pad_grid(
    ui: &mut egui::Ui,
    n: usize,
    min_w: f32,
    mut pad: impl FnMut(&mut egui::Ui, usize, egui::Vec2),
) {
    if n == 0 {
        return;
    }
    let z = theme::zoom_of(ui);
    let gap = 5.0 * z;
    let avail = ui.available_width();
    let cols = theme::pad_columns(avail, min_w * z, gap).min(n).max(1);
    // Floored, so a row of pads is never a fraction wider than the panel —
    // a resizable side panel grows to whatever its content asks for.
    let w = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor().max(24.0);
    let size = egui::vec2(w, (w * 0.5).clamp(46.0 * z, 92.0 * z));
    let mut i = 0;
    while i < n {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for k in i..(i + cols).min(n) {
                pad(ui, k, size);
            }
        });
        ui.add_space(gap);
        i += cols;
    }
}

/// A square icon button that turns `DANGER` while it is armed, so a second
/// click reads as the dangerous one.
fn armed_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: Option<&str>,
    hint: &str,
    armed: bool,
    enabled: Result<(), &str>,
) -> egui::Response {
    // The visuals are swapped in place and put back, never scoped: a child
    // `Ui` is placed with `allocate_rect`, which a wrapping row cannot
    // break before (see `theme::toggle_icon`).
    let saved = armed.then(|| ui.visuals().widgets.clone());
    if armed {
        let v = &mut ui.visuals_mut().widgets;
        v.inactive.weak_bg_fill = theme::DANGER;
        v.inactive.bg_stroke = egui::Stroke::new(1.0, theme::DANGER);
        v.hovered.weak_bg_fill = theme::DANGER;
        v.hovered.bg_stroke = egui::Stroke::new(1.0, theme::TEXT);
    }
    let hint = if armed {
        "Click again to confirm — this cannot be clicked back."
    } else {
        hint
    };
    let r = theme::tool_button(ui, icon, label, hint, enabled);
    if let Some(widgets) = saved {
        ui.visuals_mut().widgets = widgets;
    }
    r
}

/// A thin capacity bar: a recessed trough with a lit fill.
fn usage_bar(ui: &mut egui::Ui, used: u16, color: Color32) {
    let z = theme::zoom_of(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0 * z), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let rounding = egui::Rounding::same(3.0 * z);
    let p = ui.painter();
    p.rect(rect, rounding, theme::WELL, egui::Stroke::new(1.0, theme::EDGE));
    let frac = (used as f32 / 512.0).clamp(0.0, 1.0);
    if frac > 0.0 {
        let fill = egui::Rect::from_min_size(
            rect.min,
            egui::vec2((rect.width() * frac).max(2.0), rect.height()),
        );
        p.add(egui::Shape::mesh(theme::vgradient_mesh(
            fill,
            rounding,
            theme::lighten(color, 0.15),
            theme::darken(color, 0.1),
        )));
    }
}

impl App {
    // ---- the tab ----

    pub(crate) fn inspector_patch_tab(&mut self, ui: &mut egui::Ui) {
        // --- frame prologue: state only, no widgets ---
        if self.insp.patch.source == PatchSource::Library || self.patch_library_sel.is_some() {
            self.ensure_library();
        }
        if !self.insp.patch.height_seeded {
            self.insp.patch.height = self.settings.default_height;
            self.insp.patch.height_seeded = true;
        }
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        self.patch_confirm_tick(escape);
        self.patch_count = self.patch_count.clamp(1, 64);

        let sel = self.patch_selection();
        self.patch_preview_sync(&sel);
        let plan = self.patch_plan(&sel);

        // --- the wizard ---
        let mut commit = self.patch_picker(ui, &sel);
        ui.add_space(6.0);
        self.patch_preview_card(ui, &sel);
        ui.add_space(8.0);
        commit |= self.patch_address_section(ui, &sel, &plan);
        ui.add_space(8.0);
        self.patch_place_section(ui, plan.starts.len());
        ui.add_space(8.0);
        commit |= self.patch_button(ui, &sel, &plan);
        if commit {
            self.quick_patch_commit();
        }

        ui.add_space(10.0);
        let rig = format!("{}", self.patch.fixtures.len());
        self.insp_fold(ui, "patch.rig", "Rig", true, Some((rig.as_str(), theme::TEXT_DIM)), |app, ui| {
            app.patch_rig_body(ui)
        });

        ui.add_space(8.0);
        self.patch_selection_section(ui);

        ui.add_space(8.0);
        let sb = if self.include_showbuddy { "on" } else { "off" };
        let tint = if self.include_showbuddy { theme::OK } else { theme::TEXT_DIM };
        self.insp_fold(ui, "patch.showbuddy", "ShowBuddy", false, Some((sb, tint)), |app, ui| {
            app.patch_showbuddy_body(ui)
        });

        ui.add_space(8.0);
        theme::toolbar(ui, |ui| {
            if theme::tool_button(
                ui,
                Icon::Patch,
                Some("Full patch window"),
                "Open the old Patch window, with the raw fixture lists and the ShowBuddy exclusions.",
                Ok(()),
            )
            .clicked()
            {
                self.show_patch = true;
            }
        });
    }

    // ---- prologue helpers ----

    /// Load the 14 MB fixture library, once, and cache its maker list.
    fn ensure_library(&mut self) {
        if self.library.is_loaded() {
            return;
        }
        self.library = fixturedb::Library::load();
        if let Some(err) = self.library.error.clone() {
            self.log.push(format!("Fixture library unavailable — {err}"));
        } else {
            self.log.push(format!(
                "Fixture library: {} modes from {} manufacturers",
                self.library.fixtures.len(),
                self.library.manufacturers().len()
            ));
        }
        self.insp.patch.makers =
            self.library.manufacturers().iter().map(|s| (*s).to_string()).collect();
    }

    /// Expire a pending confirm: four seconds, Escape, or a new selection.
    fn patch_confirm_tick(&mut self, escape: bool) {
        let fingerprint = self.stage.selected_fixtures();
        let moved = fingerprint != self.insp.patch.sel_fingerprint;
        if moved {
            self.insp.patch.sel_fingerprint = fingerprint;
        }
        let drop = match &self.insp.patch.confirm_unpatch {
            None => false,
            Some((scope, at)) => {
                escape
                    || at.elapsed().as_secs_f32() > CONFIRM_SECS
                    || (moved && *scope == UnpatchScope::Selection)
            }
        };
        if drop {
            self.insp.patch.confirm_unpatch = None;
        }
    }

    /// Rebuild the preview's archetype and role badges when the pick moves.
    fn patch_preview_sync(&mut self, sel: &PatchSelection) {
        if self.insp.patch.preview_key.as_deref() == Some(sel.profile.as_str()) {
            return;
        }
        let fixture = match fixturedb::library_id(&sel.profile) {
            Some(id) => self.library.find(id).map(|f| f.to_fixture(sel.label.clone(), 1)),
            None => profiles::find(&sel.profile).map(|p| p.to_fixture(sel.label.clone(), 1)),
        };
        self.insp.patch.preview_arch = fixture.as_ref().map(stage::classify);
        self.insp.patch.preview_roles = fixture
            .as_ref()
            .map(|f| {
                f.channels
                    .iter()
                    .map(|c| c.role())
                    .filter(|r| !r.tag().is_empty())
                    .take(11)
                    .collect()
            })
            .unwrap_or_default();
        self.insp.patch.preview_key = Some(sel.profile.clone());
    }

    /// The batch as it stands: addresses, names, clashes and the first hole
    /// that would fit it. Also runs the address and numbering auto-follow,
    /// which must happen before their fields are drawn.
    fn patch_plan(&mut self, sel: &PatchSelection) -> Plan {
        let span = sel.channels as u16;
        let ranges = addressing::ranges(&self.patch.fixtures);
        let prefs = self.insp.prefs.patch.clone();

        if self.insp.patch.addr_auto {
            if self.insp.patch.addr_seen != 0 && self.patch_addr != self.insp.patch.addr_seen {
                // The legacy Patch window moved the field: stop fighting it.
                self.insp.patch.addr_auto = false;
            } else {
                let want = self.follow_addr(&ranges, span);
                self.patch_addr = want;
                self.insp.patch.addr_seen = want;
            }
        }

        let base = self.patch_base_name(sel);
        if self.insp.patch.number_auto {
            let displays: Vec<&str> =
                self.patch.fixtures.iter().map(|f| f.display.as_str()).collect();
            self.insp.patch.first_number = addressing::next_number(&displays, &base);
        }

        let (starts, stop) = addressing::plan_addresses(
            &ranges,
            span,
            self.patch_count,
            self.patch_addr,
            prefs.skip_taken,
            prefs.same_universe,
        );
        let names = addressing::display_names(&base, starts.len(), self.insp.patch.first_number);
        let mut clashes: Vec<String> = Vec::new();
        for &from in &starts {
            for i in addressing::overlaps(&ranges, from, span) {
                if let Some(f) = self.patch.fixtures.get(i) {
                    let line = format!("{} ({}–{})", f.display, f.from, f.to);
                    if !clashes.contains(&line) {
                        clashes.push(line);
                    }
                }
            }
        }
        let collision = starts.iter().zip(&names).any(|(&from, display)| {
            let key = profiles::fixture_key(display, from);
            self.patch.fixtures.iter().any(|f| profiles::fixture_key(&f.display, f.from) == key)
        });
        let whole = span.saturating_mul(self.patch_count.max(1));
        let gap = addressing::first_free(&ranges, whole, 1, prefs.same_universe)
            .filter(|&a| a != self.patch_addr);
        Plan { span, starts, stop, names, clashes, collision, gap }
    }

    /// Where the address auto-follow puts a `span`-channel batch. Shared
    /// with [`App::quick_patch_commit`], which re-derives it against the
    /// pick that is really being patched.
    fn follow_addr(&self, ranges: &[(u16, u16)], span: u16) -> u16 {
        match self.insp.patch.addr_mode {
            AddrMode::Append => addressing::next_free(ranges),
            AddrMode::FirstGap => {
                addressing::first_free(ranges, span, 1, self.insp.prefs.patch.same_universe)
                    .unwrap_or_else(|| addressing::next_free(ranges))
            }
        }
    }

    /// The name prefix the batch uses: what was typed, else the model.
    fn patch_base_name(&self, sel: &PatchSelection) -> String {
        if self.patch_name.trim().is_empty() {
            addressing::short_name(&sel.label)
        } else {
            self.patch_name.trim().to_string()
        }
    }

    // ---- 1. the picker ----

    /// Recents, the source segments, and either the library search or the
    /// built-in profiles. Returns true when a recent was double-clicked.
    fn patch_picker(&mut self, ui: &mut egui::Ui, sel: &PatchSelection) -> bool {
        let mut commit = false;
        theme::section_with(ui, "Quick patch", |ui| {
            theme::help(
                ui,
                "Pick a model, set how many and where they start, choose a shape or an element \
                 to hang them on, then press Patch. The lights are placed and selected for you.",
            );
        });

        // Recents: the fastest path back to a light you just patched.
        let recents: Vec<profiles::RecentProfile> = self.recent_profiles.clone();
        ui.horizontal(|ui| {
            let z = theme::zoom_of(ui);
            let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(11.0 * z), egui::Sense::hover());
            crate::ui::icons::draw(ui.painter(), r, Icon::Recent, theme::TEXT_DIM);
            theme::hint(ui, "Recent");
        });
        if recents.is_empty() {
            theme::hint(ui, "Patch something and it shows up here.");
        } else {
            let mut pick: Option<(usize, bool)> = None;
            pad_grid(ui, recents.len(), 96.0, |ui, i, size| {
                let r = &recents[i];
                let picked = sel.profile == r.profile;
                let resp = theme::pad_button(
                    ui,
                    ("insp_patch_recent", i),
                    size,
                    pad_state(picked),
                    PadFace::Symbol(&format!("{}ch", r.channels)),
                    &r.label,
                );
                let resp = resp.on_hover_text(format!(
                    "Patch {} again ({} channels). Double-click to patch it now.",
                    r.label, r.channels
                ));
                if resp.double_clicked() {
                    pick = Some((i, true));
                } else if resp.clicked() {
                    pick = Some((i, false));
                }
            });
            if let Some((i, now)) = pick {
                commit |= self.pick_recent(i, now);
            }
        }

        ui.add_space(4.0);
        let segs = [
            theme::Segment {
                icon: Some(Icon::ParCan),
                label: "Built-in",
                hint: "DMXpress's own profiles — generic pars, bars, movers and effects.",
                badge: None,
            },
            theme::Segment {
                icon: Some(Icon::LibraryBook),
                label: "Library",
                hint: "Search the fixture library: thousands of real models and their modes.",
                badge: None,
            },
        ];
        let current = (self.insp.patch.source == PatchSource::Library) as usize;
        if let Some(i) = theme::segmented(ui, "insp_patch_src", &segs, current) {
            self.insp.patch.source =
                if i == 1 { PatchSource::Library } else { PatchSource::Builtin };
        }
        ui.add_space(4.0);
        match self.insp.patch.source {
            PatchSource::Library => commit |= self.patch_library_picker(ui),
            PatchSource::Builtin => self.patch_builtin_picker(ui),
        }
        commit
    }

    /// Put a recent profile back in the picker; `now` also patches it.
    fn pick_recent(&mut self, i: usize, now: bool) -> bool {
        let Some(r) = self.recent_profiles.get(i).cloned() else { return false };
        if let Some(id) = fixturedb::library_id(&r.profile) {
            self.patch_library_sel = Some(id.to_string());
            self.insp.patch.source = PatchSource::Library;
        } else if let Some(k) = PROFILES.iter().position(|p| p.name == r.profile) {
            self.patch_profile = k;
            self.patch_library_sel = None;
            self.insp.patch.source = PatchSource::Builtin;
        } else {
            self.log.push(format!("Recent profile '{}' is not available any more", r.label));
            self.recent_profiles.remove(i);
            return false;
        }
        now
    }

    /// Search box, maker combo, class chips and the hit list.
    fn patch_library_picker(&mut self, ui: &mut egui::Ui) -> bool {
        let mut commit = false;
        let z = theme::zoom_of(ui);
        let mut enter = false;
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(12.0 * z), egui::Sense::hover());
            crate::ui::icons::draw(ui.painter(), r, Icon::Search, theme::TEXT_DIM);
            // The glyph, the clear button and three lots of spacing all
            // share this row; asking for more than is left widens the panel.
            let w = (ui.available_width() - 50.0 * z).max(40.0);
            let resp = ui
                .add(
                    egui::TextEdit::singleline(&mut self.patch_search)
                        .hint_text("maker model…")
                        .desired_width(w),
                )
                .on_hover_text(
                    "Every word has to match, so \"chauvet spot 110\" narrows the way you expect. \
                     Enter patches the highlighted fixture.",
                );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                enter = true;
            }
            if resp.has_focus() {
                let (down, up) = ui.input(|i| {
                    (i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::ArrowUp))
                });
                let n = self.insp.patch.hits.len();
                if n > 0 && (down || up) {
                    let cur = self.insp.patch.hit_cursor.unwrap_or(0);
                    self.insp.patch.hit_cursor =
                        Some(if down { (cur + 1) % n } else { (cur + n - 1) % n });
                }
            }
            if theme::tool_button(
                ui,
                Icon::Close,
                None,
                "Clear the search, the maker and the class filter.",
                Ok(()),
            )
            .clicked()
            {
                self.patch_search.clear();
                self.patch_manufacturer = None;
                self.insp.patch.arch_filter = None;
            }
        });

        // Makers, cached at load — never a per-frame scan of the library.
        let makers = self.insp.patch.makers.clone();
        let selected = self.patch_manufacturer.clone();
        // A ComboBox is its width plus the arrow and its padding, so the
        // full available width would widen the panel a little more every
        // frame.
        egui::ComboBox::from_id_salt("insp_patch_maker")
            .width(combo_width(ui))
            .selected_text(combo_text(ui, selected.as_deref().unwrap_or("All makers")))
            .show_ui(ui, |ui| {
                if ui.selectable_label(selected.is_none(), "All makers").clicked() {
                    self.patch_manufacturer = None;
                }
                for m in &makers {
                    if ui.selectable_label(selected.as_deref() == Some(m.as_str()), m).clicked() {
                        self.patch_manufacturer = Some(m.clone());
                    }
                }
            })
            .response
            .on_hover_text("Narrow the search to one manufacturer.");

        ui.add_space(4.0);
        ui.push_id("insp_patch_class", |ui| {
            ui.horizontal_wrapped(|ui| {
                let filter = self.insp.patch.arch_filter;
                if theme::chip(ui, "All", theme::TEXT_DIM, filter.is_none())
                    .on_hover_text("Show every kind of fixture.")
                    .clicked()
                {
                    self.insp.patch.arch_filter = None;
                }
                for class in ArchClass::ALL {
                    let on = filter == Some(class);
                    let tint = if on { theme::ACCENT_SOFT } else { theme::TEXT_DIM };
                    if theme::chip(ui, class.label(), tint, on)
                        .on_hover_text(format!("Only show {} fixtures.", class.label().to_lowercase()))
                        .clicked()
                    {
                        self.insp.patch.arch_filter = if on { None } else { Some(class) };
                    }
                }
            });
        });

        self.patch_sync_hits();

        ui.add_space(4.0);
        if let Some(err) = self.library.error.clone() {
            theme::pill(ui, "library unavailable", theme::WARN).on_hover_text(err);
            return commit;
        }
        let hits = self.insp.patch.hits.clone();
        let cursor = self.insp.patch.hit_cursor;
        let picked = self.patch_library_sel.clone();
        let mut chosen: Option<String> = None;
        theme::well(ui, |ui| {
            if hits.is_empty() {
                theme::hint(ui, "No fixtures match that search.");
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("insp_patch_hits")
                .max_height(180.0 * z)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, hit) in hits.iter().enumerate() {
                        let row = theme::Row {
                            swatch: None,
                            icon: None,
                            title: &hit.title,
                            subtitle: Some(&hit.subtitle),
                            badges: &[],
                            selected: picked.as_deref() == Some(hit.id.as_str()),
                            active: false,
                            indent: 0.0,
                        };
                        let resp = theme::list_row(ui, ("insp_patch_hit", i), &row);
                        if cursor == Some(i) {
                            ui.painter().rect_stroke(
                                resp.rect,
                                egui::Rounding::same(5.0),
                                egui::Stroke::new(1.0, theme::ACCENT_SOFT),
                            );
                            resp.scroll_to_me(None);
                        }
                        if resp.on_hover_text("Patch this fixture. Its mode sets the channel count.").clicked() {
                            chosen = Some(hit.id.clone());
                        }
                    }
                });
        });
        if enter {
            if let Some(hit) = hits.get(cursor.unwrap_or(0)) {
                chosen = Some(hit.id.clone());
                commit = true;
            }
        }
        if let Some(id) = chosen {
            self.patch_library_sel = Some(id);
        }
        theme::hint(
            ui,
            format!("{} shown · {} in the library", hits.len(), self.library.fixtures.len()),
        );
        commit
    }

    /// Recompute the hit list only when the search, maker or class changed.
    fn patch_sync_hits(&mut self) {
        let key = (
            self.patch_search.clone(),
            self.patch_manufacturer.clone(),
            self.insp.patch.arch_filter,
        );
        if self.insp.patch.hits_key.as_ref() == Some(&key) {
            return;
        }
        let limit = if key.2.is_some() { 600 } else { 150 };
        let hits: Vec<Hit> = self
            .library
            .search(&key.0, key.1.as_deref(), limit)
            .into_iter()
            .filter_map(|i| self.library.fixtures.get(i))
            .map(|f| Hit {
                id: f.id.clone(),
                title: f.title(),
                subtitle: f.subtitle(),
                class: category_class(&f.category),
            })
            .filter(|h| key.2.is_none_or(|c| h.class == c))
            .take(150)
            .collect();
        self.insp.patch.hits = hits;
        self.insp.patch.hits_key = Some(key);
        self.insp.patch.hit_cursor = None;
    }

    /// The built-in profiles as pads.
    fn patch_builtin_picker(&mut self, ui: &mut egui::Ui) {
        let current = self.patch_library_sel.is_none().then_some(self.patch_profile);
        let mut pick: Option<usize> = None;
        // Capped, so the twenty-odd built-ins never push the address and
        // place sections off the bottom of the panel.
        egui::ScrollArea::vertical()
            .id_salt("insp_patch_builtins")
            .max_height(232.0 * theme::zoom_of(ui))
            .auto_shrink([false, true])
            .show(ui, |ui| {
        pad_grid(ui, PROFILES.len(), 96.0, |ui, i, size| {
            let p = &PROFILES[i];
            let on = current == Some(i);
            let resp = theme::pad_button(
                ui,
                ("insp_patch_builtin", i),
                size,
                pad_state(on),
                PadFace::Symbol(&format!("{}ch", p.channel_count())),
                &addressing::short_name(p.name),
            );
            if resp.on_hover_text(format!("Patch DMXpress's {} profile.", p.name)).clicked() {
                pick = Some(i);
            }
        });
            });
        if let Some(i) = pick {
            self.patch_profile = i;
            self.patch_library_sel = None;
        }
    }

    // ---- 2. the preview ----

    /// Exactly what the button will patch: model, mode, channel count,
    /// movement and the first channel roles.
    fn patch_preview_card(&mut self, ui: &mut egui::Ui, sel: &PatchSelection) {
        let arch = self.insp.patch.preview_arch;
        let roles = self.insp.patch.preview_roles.clone();
        theme::card(ui, |ui| {
            let title = clip_title(ui, &sel.label, 26.0);
            theme::card_title(ui, &title, |ui| {
                let z = theme::zoom_of(ui);
                let (r, _) =
                    ui.allocate_exact_size(egui::Vec2::splat(15.0 * z), egui::Sense::hover());
                arch_glyph(ui.painter(), r, arch, theme::ACCENT_SOFT);
            });
            ui.horizontal_wrapped(|ui| {
                if sel.channels == 0 {
                    theme::tag(ui, "no channels", theme::DANGER)
                        .on_hover_text("This mode has no channels, so there is nothing to patch.");
                } else {
                    theme::tag(ui, &format!("{} ch", sel.channels), theme::ACCENT_SOFT)
                        .on_hover_text("How many DMX slots each copy takes.");
                }
                theme::tag(ui, arch_word(arch), theme::OK).on_hover_text("How the stage will draw it.");
                if sel.pan > 0.0 {
                    theme::tag(ui, &format!("pan {}°", sel.pan as i32), theme::TEXT_DIM)
                        .on_hover_text("How far it can pan.");
                }
                if sel.tilt > 0.0 {
                    theme::tag(ui, &format!("tilt {}°", sel.tilt as i32), theme::TEXT_DIM)
                        .on_hover_text("How far it can tilt.");
                }
            });
            if !roles.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
                    for r in roles.iter().take(10) {
                        theme::tag(ui, r.tag(), crate::ui::role_color(*r))
                            .on_hover_text(format!("A {:?} channel.", r));
                    }
                    if roles.len() > 10 {
                        theme::tag(ui, "+", theme::TEXT_DIM)
                            .on_hover_text("More channels than fit here.");
                    }
                });
            }
        });
    }

    // ---- 3. addressing and naming ----

    /// Count, start address, universe, the two hop rules, the name prefix
    /// and the live plan pills. Returns true when Enter in the name field
    /// asked to patch.
    fn patch_address_section(
        &mut self,
        ui: &mut egui::Ui,
        sel: &PatchSelection,
        plan: &Plan,
    ) -> bool {
        let mut commit = false;
        theme::section(ui, "Address & names");
        let ranges = addressing::ranges(&self.patch.fixtures);
        theme::kv_grid(ui, "insp_patch_addr", |ui| {
            theme::label_dim(ui, "Count");
            ui.horizontal_wrapped(|ui| {
                let room = addressing::fits(self.patch_addr, plan.span, 64);
                ui.add(egui::DragValue::new(&mut self.patch_count).range(1..=64).speed(0.2))
                    .on_hover_text(format!(
                        "How many copies to patch in one press — {room} of them fit from \
                         address {} on.",
                        self.patch_addr
                    ));
                ui.push_id("insp_patch_counts", |ui| {
                    for n in [4u16, 6, 8, 12] {
                        if theme::chip(ui, &n.to_string(), theme::TEXT_DIM, self.patch_count == n)
                            .on_hover_text(format!("Patch {n} of them."))
                            .clicked()
                        {
                            self.patch_count = n;
                        }
                    }
                });
            });
            ui.end_row();

            theme::label_dim(ui, "Start");
            ui.horizontal_wrapped(|ui| {
                // Addresses read as monospace; set on the row itself rather
                // than in a `scope`, which would stop the row wrapping.
                ui.style_mut().override_font_id =
                    Some(egui::FontId::monospace(12.0 * theme::zoom_of(ui)));
                let resp =
                    ui.add(egui::DragValue::new(&mut self.patch_addr).range(1..=DMX_SLOTS as u16));
                ui.style_mut().override_font_id = None;
                if resp
                    .on_hover_text(
                        "The first light's 1-based DMX address; 513 and up is universe 2.",
                    )
                    .changed()
                {
                    self.insp.patch.addr_auto = false;
                }
                let append = self.insp.patch.addr_auto && self.insp.patch.addr_mode == AddrMode::Append;
                if theme::toggle_icon(
                    ui,
                    Icon::NextFree,
                    None,
                    append,
                    "Follow the first address after the last patched light.",
                )
                .clicked()
                {
                    self.insp.patch.addr_mode = AddrMode::Append;
                    self.insp.patch.addr_auto = true;
                    self.insp.patch.addr_seen = 0;
                }
                let gap = self.insp.patch.addr_auto && self.insp.patch.addr_mode == AddrMode::FirstGap;
                if theme::toggle_icon(
                    ui,
                    Icon::Gap,
                    None,
                    gap,
                    "Follow the first hole in the patch big enough for this fixture.",
                )
                .clicked()
                {
                    self.insp.patch.addr_mode = AddrMode::FirstGap;
                    self.insp.patch.addr_auto = true;
                    self.insp.patch.addr_seen = 0;
                }
            });
            ui.end_row();

            theme::label_dim(ui, "Universe");
            ui.horizontal_wrapped(|ui| {
                // Which universe the batch is in, and one click to move it.
                let here = addressing::universe(self.patch_addr);
                for (u, label, base) in [(1u8, "U1", 1u16), (2, "U2", 513)] {
                    ui.push_id(("insp_patch_uni", u), |ui| {
                        if theme::chip(ui, label, theme::ACCENT_SOFT, here == u)
                            .on_hover_text(format!(
                                "Universe {u}: addresses {}–{}. Click to start at its first free \
                                 address.",
                                base,
                                base + 511
                            ))
                            .clicked()
                        {
                            self.patch_addr = addressing::first_free_in(
                                &ranges,
                                plan.span,
                                u,
                                self.insp.prefs.patch.same_universe,
                            );
                            self.insp.patch.addr_auto = false;
                        }
                    });
                }
            });
            ui.end_row();
        });

        ui.horizontal_wrapped(|ui| {
            let mut skip = self.insp.prefs.patch.skip_taken;
            if ui
                .checkbox(&mut skip, "Skip taken")
                .on_hover_text(
                    "Each copy takes the next free hole instead of running straight on from \
                     the previous one.",
                )
                .changed()
            {
                self.insp.prefs.patch.skip_taken = skip;
                self.insp.mark_dirty();
            }
            let mut one = self.insp.prefs.patch.same_universe;
            if ui
                .checkbox(&mut one, "One universe")
                .on_hover_text("Never let a light straddle address 512/513.")
                .changed()
            {
                self.insp.prefs.patch.same_universe = one;
                self.insp.mark_dirty();
            }
        });

        ui.add_space(4.0);
        theme::kv_grid(ui, "insp_patch_names", |ui| {
            theme::label_dim(ui, "Name");
            let hint = addressing::short_name(&sel.label);
            let resp = ui
                .add(
                    egui::TextEdit::singleline(&mut self.patch_name)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY),
                )
                .on_hover_text("What the lights are called. Copies are numbered after it. Enter patches.");
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                commit = true;
            }
            ui.end_row();

            theme::label_dim(ui, "Number");
            ui.horizontal(|ui| {
                if ui
                    .add(
                        // `clamp_existing_to_range` defaults to true, which
                        // would rewrite a number the auto-follow produced
                        // above the range — and report that rewrite as a
                        // change, turning "auto" off by itself.
                        egui::DragValue::new(&mut self.insp.patch.first_number)
                            .range(1..=9999)
                            .clamp_existing_to_range(false)
                            .speed(0.2),
                    )
                    .on_hover_text("The number the first copy gets.")
                    .changed()
                {
                    self.insp.patch.number_auto = false;
                }
                if theme::chip(ui, "auto", theme::ACCENT_SOFT, self.insp.patch.number_auto)
                    .on_hover_text("Carry on from the highest number already in the rig.")
                    .clicked()
                {
                    self.insp.patch.number_auto = true;
                }
            });
            ui.end_row();
        });

        // What the batch will actually do, before it does it.
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if plan.starts.is_empty() {
                theme::tag(ui, "nothing fits", theme::DANGER)
                    .on_hover_text("There is no room left before slot 1024. Free some addresses.");
            } else {
                let text = format!(
                    "{} × {} ch → {}–{} · U{}",
                    plan.starts.len(),
                    plan.span,
                    plan.first(),
                    plan.last(),
                    addressing::universe(plan.first())
                );
                theme::tag(ui, &text, theme::OK).on_hover_text(format!(
                    "The batch takes {} slots, from {} to {}.",
                    plan.starts.len() * plan.span.max(1) as usize,
                    plan.first(),
                    plan.last()
                ));
            }
            if let PlanStop::Overflow(n) = plan.stop {
                theme::tag(ui, &format!("only {n} of {} fit", self.patch_count), theme::DANGER)
                    .on_hover_text("The rest would run past slot 1024, the last address DMXpress sends.");
            }
            if !plan.clashes.is_empty() {
                let first = &plan.clashes[0];
                let more = plan.clashes.len() - 1;
                let text = if more > 0 {
                    format!("overlaps {first} +{more} more")
                } else {
                    format!("overlaps {first}")
                };
                theme::tag(ui, &text, theme::WARN)
                    .on_hover_text(format!("Sharing addresses:\n{}", plan.clashes.join("\n")));
            }
            if plan.collision {
                theme::tag(ui, "name already in rig", theme::WARN).on_hover_text(
                    "Two lights with the same name and address share one saved position. \
                     Change the prefix or the number.",
                );
            }
            if let Some(a) = plan.gap {
                if theme::chip(ui, &format!("gap at {a}"), theme::ACCENT_SOFT, false)
                    .on_hover_text("The first hole the whole batch fits in. Click to start there.")
                    .clicked()
                {
                    self.patch_addr = a;
                    self.insp.patch.addr_auto = false;
                }
            }
        });
        if let (Some(first), Some(last)) = (plan.names.first(), plan.names.last()) {
            let text = if plan.names.len() > 1 {
                format!("→ {first} … {last}")
            } else {
                format!("→ {first}")
            };
            theme::hint(ui, text);
        }
        commit
    }

    // ---- 4. placement ----

    /// The pattern pads and the parameters the chosen pattern needs.
    fn patch_place_section(&mut self, ui: &mut egui::Ui, count: usize) {
        theme::section_with(ui, "Place", |ui| {
            theme::help(
                ui,
                "Where the new lights land. Without this every freshly patched light spawns on \
                 the same spot in the middle of the stage.",
            );
        });
        let pattern = self.insp.prefs.patch.pattern;
        let mut chosen: Option<PlacePattern> = None;
        pad_grid(ui, PlacePattern::ALL.len(), 62.0, |ui, i, size| {
            let p = PlacePattern::ALL[i];
            let on = p == pattern;
            let icon = match p {
                PlacePattern::Keep => Icon::Reset,
                PlacePattern::LineX => Icon::ArrangeLine,
                PlacePattern::LineZ => Icon::ArrangeColumn,
                PlacePattern::Grid => Icon::ArrangeGrid,
                PlacePattern::Arc => Icon::ArrangeArc,
                PlacePattern::Circle => Icon::ArrangeCircle,
                PlacePattern::Floor => Icon::Floor,
                PlacePattern::Truss => Icon::Hook,
                PlacePattern::Tower => Icon::Tower,
            };
            let hint = match p {
                PlacePattern::Keep => "Leave the new lights where the patch puts them.",
                PlacePattern::LineX => "A line across the stage, left to right.",
                PlacePattern::LineZ => "A line up and down the stage, front to back.",
                PlacePattern::Grid => "A block, filled row by row.",
                PlacePattern::Arc => "An arc bent around the middle of the stage.",
                PlacePattern::Circle => "A full ring around the middle of the stage.",
                PlacePattern::Floor => "Standing on the deck, angled up into the rig.",
                PlacePattern::Truss => "Hung on a truss — an existing one or a new one sized to fit.",
                PlacePattern::Tower => "Clipped onto a floor stand's crossbar.",
            };
            let resp = theme::pad_button(
                ui,
                ("insp_patch_pattern", i),
                size,
                pad_state(on),
                PadFace::Icon(icon),
                p.label(),
            );
            if resp.on_hover_text(hint).clicked() {
                chosen = Some(p);
            }
        });
        if let Some(p) = chosen {
            self.insp.prefs.patch.pattern = p;
            self.insp.mark_dirty();
        }
        let pattern = self.insp.prefs.patch.pattern;

        if pattern.is_mount() {
            self.patch_mount_rows(ui, count, pattern);
        } else if pattern != PlacePattern::Keep {
            self.patch_shape_rows(ui, pattern);
        }

        ui.add_space(4.0);
        let mut select = self.insp.patch.select_after;
        if ui
            .checkbox(&mut select, "Select the new lights")
            .on_hover_text("Leave the batch selected so the Selection tab can aim or nudge it.")
            .changed()
        {
            self.insp.patch.select_after = select;
        }
        theme::hint(ui, "⌘Z undoes the placement · Ctrl+Shift+Z undoes the patch.");
    }

    /// Spacing, height and the shape's own parameters.
    fn patch_shape_rows(&mut self, ui: &mut egui::Ui, pattern: PlacePattern) {
        use PlacePattern as P;
        theme::kv_grid(ui, "insp_patch_place", |ui| {
            if matches!(pattern, P::LineX | P::LineZ | P::Grid | P::Floor) {
                theme::label_dim(ui, "Spacing");
                ui.horizontal(|ui| {
                    let mut s = self.insp.prefs.patch.spacing;
                    if ui
                        .add(
                            egui::DragValue::new(&mut s)
                                .range(0.2..=5.0)
                                .speed(0.05)
                                .suffix(" m"),
                        )
                        .on_hover_text("Metres between neighbouring lights.")
                        .changed()
                    {
                        self.insp.prefs.patch.spacing = s;
                        self.insp.mark_dirty();
                    }
                    let mut fit = self.insp.patch.fit_to_stage;
                    if ui
                        .checkbox(&mut fit, "Fit")
                        .on_hover_text("Shrink the spacing so the whole run stays on the stage.")
                        .changed()
                    {
                        self.insp.patch.fit_to_stage = fit;
                    }
                });
                ui.end_row();
            }
            if pattern != P::Floor {
                theme::label_dim(ui, "Height");
                ui.add(
                    egui::DragValue::new(&mut self.insp.patch.height)
                        .range(0.0..=12.0)
                        .speed(0.1)
                        .suffix(" m"),
                )
                .on_hover_text("How high off the floor the lights hang.");
                ui.end_row();
            }
            if matches!(pattern, P::LineX | P::Grid | P::Floor | P::LineZ) {
                theme::label_dim(ui, if pattern == P::LineZ { "Column X" } else { "Row Z" });
                ui.add(
                    egui::DragValue::new(&mut self.insp.patch.row)
                        .range(-20.0..=20.0)
                        .speed(0.1)
                        .suffix(" m"),
                )
                .on_hover_text(
                    "How far upstage (or across) the run sits; 0 is the centre of the stage.",
                );
                ui.end_row();
            }
            if pattern == P::Grid {
                theme::label_dim(ui, "Columns");
                ui.add(egui::DragValue::new(&mut self.insp.patch.columns).range(0..=32))
                    .on_hover_text("How many across; 0 makes the block as square as it can.");
                ui.end_row();
            }
            if matches!(pattern, P::Arc | P::Circle) {
                theme::label_dim(ui, "Radius");
                ui.add(
                    egui::DragValue::new(&mut self.insp.patch.radius)
                        .range(0.5..=15.0)
                        .speed(0.1)
                        .suffix(" m"),
                )
                .on_hover_text("How far the lights sit from the centre of the shape.");
                ui.end_row();
                if pattern == P::Arc {
                    theme::label_dim(ui, "Arc");
                    ui.add(
                        egui::DragValue::new(&mut self.insp.patch.arc_deg)
                            .range(15.0..=360.0)
                            .speed(1.0)
                            .suffix("°"),
                    )
                    .on_hover_text("How far round the arc bends.");
                    ui.end_row();
                }
                theme::label_dim(ui, "Facing");
                let mut face = self.insp.patch.face_centre;
                if ui
                    .checkbox(&mut face, "Point at the centre")
                    .on_hover_text("Turn every light toward the middle of the shape.")
                    .changed()
                {
                    self.insp.patch.face_centre = face;
                }
                ui.end_row();
            }
        });
    }

    /// Which element the batch hangs on, which face, and how long a new
    /// truss should be.
    fn patch_mount_rows(&mut self, ui: &mut egui::Ui, count: usize, pattern: PlacePattern) {
        let truss = pattern == PlacePattern::Truss;
        // The element list is rebuilt every frame: another tab may have
        // deleted the one that was picked.
        let mut options: Vec<(ElementChoice, String)> = Vec::new();
        // A hidden element is not drawn and drag-snap refuses it, so it is
        // not on offer here either.
        let existing: Vec<ElementRef> = if truss {
            options.push((ElementChoice::NewTruss(TrussKind::Straight), "+ New F34 truss".into()));
            options.push((ElementChoice::NewTruss(TrussKind::Radius), "+ New radius truss".into()));
            (0..self.stage.trusses.len())
                .filter(|&i| !self.stage.trusses[i].hidden)
                .map(ElementRef::Truss)
                .collect()
        } else {
            options.push((ElementChoice::NewTower, "+ New tower".into()));
            (0..self.stage.towers.len())
                .filter(|&i| !self.stage.towers[i].hidden)
                .map(ElementRef::Tower)
                .collect()
        };
        for r in existing {
            // The numbers come off the face the batch would really hang on:
            // a straight run has four, so clamping to 1 reports the bottom
            // face's occupancy for a batch bound for Left or Right.
            let faces = self.stage.element_faces(r).len();
            let (free, total) = self.stage.element_free(r, self.insp.patch.face.min(faces - 1));
            options.push((
                ElementChoice::Existing(r),
                format!("{} · {free}/{total} free", self.stage.element_name(r)),
            ));
        }
        // Fall back to "new" when the pick no longer exists or is the wrong kind.
        if !options.iter().any(|(c, _)| *c == self.insp.patch.element) {
            self.insp.patch.element = if truss {
                ElementChoice::NewTruss(TrussKind::Straight)
            } else {
                ElementChoice::NewTower
            };
        }
        let current = options
            .iter()
            .find(|(c, _)| *c == self.insp.patch.element)
            .map(|(_, l)| l.clone())
            .unwrap_or_default();
        let faces: Vec<&'static str> = match self.insp.patch.element {
            ElementChoice::Existing(r) => self.stage.element_faces(r).to_vec(),
            ElementChoice::NewTruss(TrussKind::Straight) => {
                vec!["Top", "Bottom", "Left", "Right"]
            }
            _ => vec!["Top", "Under"],
        };
        self.insp.patch.face = self.insp.patch.face.min(faces.len() - 1);

        theme::kv_grid(ui, "insp_patch_place", |ui| {
            theme::label_dim(ui, "On");
            egui::ComboBox::from_id_salt("insp_patch_element")
                .width(combo_width(ui))
                .selected_text(combo_text(ui, &current))
                .show_ui(ui, |ui| {
                    for (choice, label) in &options {
                        if ui
                            .selectable_label(*choice == self.insp.patch.element, label.as_str())
                            .clicked()
                        {
                            self.insp.patch.element = *choice;
                        }
                    }
                })
                .response
                .on_hover_text(
                    "Which element the batch hangs on — an existing one, or a new one built to \
                     size before the lights are patched.",
                );
            ui.end_row();

            if matches!(self.insp.patch.element, ElementChoice::NewTruss(TrussKind::Straight)) {
                theme::label_dim(ui, "Length");
                ui.horizontal(|ui| {
                    let mut fit = self.insp.patch.fit_length;
                    if ui
                        .checkbox(&mut fit, "To fit")
                        .on_hover_text("Build the run exactly long enough for this batch.")
                        .changed()
                    {
                        self.insp.patch.fit_length = fit;
                    }
                    if !fit {
                        ui.add(
                            egui::DragValue::new(&mut self.insp.patch.new_length)
                                .range(0.5..=12.0)
                                .speed(0.1)
                                .suffix(" m"),
                        )
                        .on_hover_text("How long the new run is.");
                    } else {
                        theme::hint(ui, format!("{:.1} m", fit_length(count)));
                    }
                });
                ui.end_row();
            }
        });

        // The faces are a segmented strip of their own: a wrapping row of
        // buttons inside a Grid cell cannot measure itself properly, and an
        // over-wide cell drags the whole panel with it.
        let hints: Vec<String> = faces
            .iter()
            .map(|f| format!("Clip the lights onto the {} face of the element.", f.to_lowercase()))
            .collect();
        let segs: Vec<theme::Segment> = faces
            .iter()
            .zip(&hints)
            .map(|(label, hint)| theme::Segment {
                icon: None,
                label,
                hint: hint.as_str(),
                badge: None,
            })
            .collect();
        if let Some(i) = theme::segmented(ui, "insp_patch_face", &segs, self.insp.patch.face) {
            self.insp.patch.face = i;
        }

        // Does the batch actually fit on that face? A new element is sized
        // here exactly as `resolve_element` will size it — a tower always
        // has four slots per face and a radius run six, so "room for all"
        // was a promise only a fitted straight run could keep.
        let face = self.insp.patch.face;
        let (free, total) = match self.insp.patch.element {
            ElementChoice::Existing(r) => self.stage.element_free(r, face),
            ElementChoice::NewTower => (TOWER_SLOTS / 2, TOWER_SLOTS / 2),
            ElementChoice::NewTruss(kind) => {
                let n = stage::fitted_truss(kind, self.new_truss_length(kind, count)).slot_count();
                (n, n)
            }
        };
        if count > free {
            theme::pill(ui, &format!("only {free} of {count} fit"), theme::WARN)
                .on_hover_text("The rest are parked in a line just beneath the element.");
        } else if matches!(self.insp.patch.element, ElementChoice::Existing(_)) {
            theme::hint(ui, format!("{count} of {free} free slots ({total} on this face)"));
        } else {
            theme::hint(ui, format!("A new element, with room for all {count} on one face."));
        }
    }

    /// The length `resolve_element` would build a new run with: `None` for
    /// a radius run, whose arc is what sizes it.
    fn new_truss_length(&self, kind: TrussKind, count: usize) -> Option<f32> {
        (kind == TrussKind::Straight).then(|| {
            if self.insp.patch.fit_length {
                fit_length(count)
            } else {
                self.insp.patch.new_length
            }
        })
    }

    // ---- 5. the button ----

    /// The one hero action of the tab.
    fn patch_button(&mut self, ui: &mut egui::Ui, sel: &PatchSelection, plan: &Plan) -> bool {
        let n = plan.starts.len();
        let reason = if sel.channels == 0 {
            Err("This mode has no channels to patch.")
        } else if n == 0 {
            Err("Nothing fits before slot 1024 — free some addresses first.")
        } else {
            Ok(())
        };
        let long = format!("Patch {n} × {} → {}–{}", sel.label, plan.first(), plan.last());
        let short = if plan.clashes.is_empty() {
            format!("Patch {n} → {}–{}", plan.first(), plan.last())
        } else {
            format!("Patch {n} anyway → {}–{}", plan.first(), plan.last())
        };
        let label = if ui.available_width() >= 300.0 && plan.clashes.is_empty() { long } else { short };
        let resp = theme::gated(ui, reason, |ui| {
            theme::wide_button(ui, Some(Icon::QuickPatch), &label, Tone::Accent)
        });
        let hover = if plan.clashes.is_empty() {
            format!(
                "Patch {n} × {} at {}–{}, place them and select them.",
                sel.label,
                plan.first(),
                plan.last()
            )
        } else {
            format!(
                "Patch {n} × {} at {}–{} even though that overlaps {}.",
                sel.label,
                plan.first(),
                plan.last(),
                plan.clashes.join(", ")
            )
        };
        let clicked = resp.on_hover_text(hover).clicked();
        theme::hint(ui, "Enter patches from the name or search box.");
        clicked
    }

    // ---- 6. the rig ----

    /// Capacity, one card per fixture type, and the rig sheet.
    fn patch_rig_body(&mut self, ui: &mut egui::Ui) {
        let ranges = addressing::ranges(&self.patch.fixtures);
        let usage = addressing::universe_usage(&ranges);
        let tint = |used: u16| {
            if used >= 512 {
                theme::DANGER
            } else if used as f32 >= 512.0 * 0.8 {
                theme::WARN
            } else {
                theme::OK
            }
        };
        let mut toggle_sb = false;
        ui.horizontal_wrapped(|ui| {
            theme::tag(ui, &format!("{} lights", self.patch.fixtures.len()), theme::ACCENT_SOFT)
                .on_hover_text("Every light in the rig, ShowBuddy's included.");
            for (u, used) in usage.iter().enumerate() {
                theme::tag(ui, &format!("U{} {used}/512", u + 1), tint(*used)).on_hover_text(
                    format!("Slots used in universe {} of the two DMXpress sends.", u + 1),
                );
            }
            let on = self.include_showbuddy;
            if theme::chip(
                ui,
                if on { "ShowBuddy on" } else { "ShowBuddy off" },
                if on { theme::ACCENT_SOFT } else { theme::TEXT_DIM },
                on,
            )
            .on_hover_text("Whether ShowBuddy's own patch is merged into the rig. Click to flip it.")
            .clicked()
            {
                toggle_sb = true;
            }
        });
        for (u, used) in usage.iter().enumerate() {
            ui.push_id(("insp_patch_bar", u), |ui| usage_bar(ui, *used, tint(*used)));
        }
        if toggle_sb {
            self.toggle_showbuddy();
        }

        ui.add_space(6.0);
        let groups = rig_groups(&self.patch, &self.library);
        if groups.is_empty() {
            theme::empty_state(
                ui,
                Icon::QuickPatch,
                "Nothing patched yet",
                "Pick a model above and press Patch.",
            );
            return;
        }
        let buf = *self.net.dmx.lock();
        let mut act: Option<RigAction> = None;
        egui::ScrollArea::vertical()
            .id_salt("insp_patch_rig")
            .max_height(260.0 * theme::zoom_of(ui))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (gi, g) in groups.iter().enumerate() {
                    let open = self.insp.patch.open_groups.contains(&g.file);
                    let renaming = self
                        .insp
                        .patch
                        .rename
                        .as_ref()
                        .is_some_and(|(f, _)| *f == g.file);
                    theme::card(ui, |ui| {
                        if renaming {
                            let mut draft = self
                                .insp
                                .patch
                                .rename
                                .as_ref()
                                .map(|(_, d)| d.clone())
                                .unwrap_or_default();
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut draft)
                                    .hint_text("new prefix")
                                    .desired_width(f32::INFINITY),
                            );
                            // Focus it on the frame it appears, but never
                            // steal focus back, or Enter could not commit.
                            if !resp.has_focus() && ui.memory(|m| m.focused().is_none()) {
                                resp.request_focus();
                            }
                            self.insp.patch.rename = Some((g.file.clone(), draft.clone()));
                            if resp.lost_focus() {
                                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    act = Some(RigAction::RenameDone(g.file.clone(), draft));
                                } else {
                                    self.insp.patch.rename = None;
                                }
                            }
                        } else {
                            let title = clip_title(ui, &g.label, 62.0);
                            theme::card_title(ui, &title, |ui| {
                                theme::tag(ui, &format!("×{}", g.count), theme::ACCENT_SOFT)
                                    .on_hover_text("How many of this type are in the rig.");
                                if let Some(f) = self.patch.fixtures.get(g.first_fi) {
                                    theme::swatch(ui, stage::fixture_swatch(f, &buf), 12.0)
                                        .on_hover_text("What this type is putting out right now.");
                                }
                            });
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.monospace(
                                egui::RichText::new(format!("{}–{}", g.from_min, g.to_max))
                                    .size(11.0),
                            );
                            theme::tag(ui, &format!("{} ch", g.channels), theme::TEXT_DIM)
                                .on_hover_text("Channels each one of these takes.");
                            if !g.is_user {
                                theme::tag(ui, "ShowBuddy", theme::TEXT_DIM)
                                    .on_hover_text("Imported from ShowBuddy's own patch.");
                            }
                        });
                        theme::toolbar(ui, |ui| {
                            let chev = if open { Icon::ChevronDown } else { Icon::ChevronRight };
                            if theme::tool_button(ui, chev, None, "Show the individual lights of this type.", Ok(()))
                                .clicked()
                            {
                                act = Some(RigAction::Toggle(g.file.clone()));
                            }
                            let user = if g.is_user {
                                Ok(())
                            } else {
                                Err("ShowBuddy fixtures are patched in ShowBuddy.")
                            };
                            if theme::tool_button(
                                ui,
                                Icon::AddOne,
                                None,
                                "Patch one more of these at the next free address, numbered on from the last.",
                                user,
                            )
                            .clicked()
                            {
                                act = Some(RigAction::AddOne(gi));
                            }
                            if theme::tool_button(
                                ui,
                                Icon::SelectType,
                                None,
                                "Select every light of this type on the stage.",
                                Ok(()),
                            )
                            .clicked()
                            {
                                act = Some(RigAction::SelectAll(gi));
                            }
                            if theme::tool_button(
                                ui,
                                Icon::Label,
                                None,
                                "Rename every light of this type, keeping their places on the stage.",
                                user,
                            )
                            .clicked()
                            {
                                act = Some(RigAction::Rename(gi));
                            }
                            let armed = self.patch_armed(&UnpatchScope::Type(g.file.clone()));
                            if armed_button(
                                ui,
                                Icon::Unplug,
                                None,
                                "Unpatch all of these — click again to confirm.",
                                armed,
                                Ok(()),
                            )
                            .clicked()
                            {
                                act = Some(RigAction::UnpatchType(gi));
                            }
                        });
                        if open {
                            ui.add_space(2.0);
                            for &fi in g.fis.iter().take(40) {
                                let Some(f) = self.patch.fixtures.get(fi) else { continue };
                                let key = profiles::fixture_key(&f.display, f.from);
                                let mount = self.stage.mount_label(fi);
                                let sub = match &mount {
                                    Some(m) => format!("{}–{} · {m}", f.from, f.to),
                                    None => format!("{}–{}", f.from, f.to),
                                };
                                let title = f.display.clone();
                                let selected = self.stage.fixture_selected(fi);
                                let armed = self.patch_armed(&UnpatchScope::One(key.clone()));
                                // Right-to-left: the × takes its own width
                                // first and the row fills what is left, so
                                // neither can push the panel wider.
                                ui.horizontal(|ui| {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if armed_button(
                                                ui,
                                                Icon::Close,
                                                None,
                                                "Unpatch this one light — click again to confirm.",
                                                armed,
                                                Ok(()),
                                            )
                                            .clicked()
                                            {
                                                act = Some(RigAction::UnpatchOne(key.clone()));
                                            }
                                            ui.with_layout(
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    let resp = theme::list_row(
                                                        ui,
                                                        ("insp_patch_member", fi),
                                                        &theme::Row {
                                                            swatch: None,
                                                            icon: None,
                                                            title: &title,
                                                            subtitle: Some(&sub),
                                                            badges: &[],
                                                            selected,
                                                            active: false,
                                                            indent: 2.0,
                                                        },
                                                    );
                                                    if resp
                                                        .on_hover_text(
                                                            "Select this light on the stage.",
                                                        )
                                                        .clicked()
                                                    {
                                                        let shift =
                                                            ui.input(|i| i.modifiers.shift);
                                                        act = Some(RigAction::SelectOne(fi, shift));
                                                    }
                                                },
                                            );
                                        },
                                    );
                                });
                            }
                            if g.fis.len() > 40 {
                                theme::hint(ui, format!("+{} more", g.fis.len() - 40));
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
            });

        ui.add_space(4.0);
        theme::toolbar(ui, |ui| {
            if theme::tool_button(
                ui,
                Icon::Copy,
                Some("Copy rig sheet"),
                "Copy a plain-text address sheet of the whole rig to the clipboard.",
                Ok(()),
            )
            .clicked()
            {
                let sheet = self.rig_sheet_text(&groups);
                let lines = sheet.lines().count();
                ui.output_mut(|o| o.copied_text = sheet);
                self.log.push(format!("Rig sheet copied ({lines} lines)"));
            }
        });

        if let Some(a) = act {
            self.patch_rig_action(&groups, a);
        }
    }

    /// Apply a rig row's deferred action.
    fn patch_rig_action(&mut self, groups: &[RigGroup], act: RigAction) {
        match act {
            RigAction::Toggle(file) => {
                if !self.insp.patch.open_groups.remove(&file) {
                    self.insp.patch.open_groups.insert(file);
                }
            }
            RigAction::SelectAll(gi) => {
                let Some(g) = groups.get(gi) else { return };
                self.stage.clear_selection();
                self.stage.select_same_type(&self.patch, g.first_fi);
                self.stage.sel_stage = false;
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
                self.log.push(format!("Selected all {} × {}", g.count, g.label));
            }
            RigAction::SelectOne(fi, additive) => {
                self.stage.select_fixture(fi, additive);
                self.stage.sel_stage = false;
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
            }
            RigAction::AddOne(gi) => self.patch_one_more(groups, gi),
            RigAction::UnpatchType(gi) => {
                let Some(g) = groups.get(gi) else { return };
                let scope = UnpatchScope::Type(g.file.clone());
                self.patch_confirm(scope);
            }
            RigAction::UnpatchOne(key) => self.patch_confirm(UnpatchScope::One(key)),
            RigAction::Rename(gi) => {
                let Some(g) = groups.get(gi) else { return };
                let stem = self
                    .patch
                    .fixtures
                    .get(g.first_fi)
                    .map(|f| type_stem(&f.display).to_string())
                    .unwrap_or_else(|| g.label.clone());
                self.insp.patch.rename = Some((g.file.clone(), stem));
            }
            RigAction::RenameDone(file, draft) => {
                self.insp.patch.rename = None;
                if !draft.trim().is_empty() {
                    self.rename_type_prefix(&file, draft.trim());
                }
            }
        }
    }

    /// Arm a two-step confirm, or fire it when it is already armed.
    fn patch_confirm(&mut self, scope: UnpatchScope) {
        if self.patch_armed(&scope) {
            self.insp.patch.confirm_unpatch = None;
            self.unpatch(&scope);
        } else {
            self.insp.patch.confirm_unpatch = Some((scope, Instant::now()));
        }
    }

    fn patch_armed(&self, scope: &UnpatchScope) -> bool {
        self.insp.patch.confirm_unpatch.as_ref().is_some_and(|(s, _)| s == scope)
    }

    // ---- 7. the selection ----

    /// What is selected on the stage, and the two things this tab can do
    /// with it: send it to the Selection tab, or take it out of the patch.
    fn patch_selection_section(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Selection");
        let fis = self.stage.selected_fixtures();
        let types = {
            let mut files: Vec<&std::path::Path> = fis
                .iter()
                .filter_map(|&fi| self.patch.fixtures.get(fi).map(|f| f.file.as_path()))
                .collect();
            files.sort_unstable();
            files.dedup();
            files.len()
        };
        let (user, sb) = fis.iter().fold((0usize, 0usize), |(u, s), &fi| {
            match self.patch.fixtures.get(fi) {
                Some(f) if is_user_fixture(&f.file.to_string_lossy()) => (u + 1, s),
                Some(_) => (u, s + 1),
                None => (u, s),
            }
        });
        ui.horizontal_wrapped(|ui| {
            if fis.is_empty() {
                theme::tag(ui, "nothing selected", theme::TEXT_DIM)
                    .on_hover_text("Click a light on the stage or in the Fixtures panel.");
            } else {
                let plural = if types == 1 { "type" } else { "types" };
                theme::tag(
                    ui,
                    &format!("{} lights · {types} {plural}", fis.len()),
                    theme::ACCENT_SOFT,
                )
                .on_hover_text("What the buttons below would act on.");
            }
        });
        let gate = if fis.is_empty() {
            Err("Select lights on the stage or in the Fixtures panel first.")
        } else {
            Ok(())
        };
        let armed = self.patch_armed(&UnpatchScope::Selection);
        if armed {
            if theme::wide_button(
                ui,
                Some(Icon::Unplug),
                &format!("Confirm — unpatch {}", fis.len()),
                Tone::Danger,
            )
            .on_hover_text(format!(
                "Removes {user} DMXpress lights from the patch and hides {sb} ShowBuddy ones. \
                 Ctrl+Shift+Z brings them back."
            ))
            .clicked()
            {
                self.patch_confirm(UnpatchScope::Selection);
            }
            ui.horizontal_wrapped(|ui| {
                theme::tag(ui, &format!("removes {user} · hides {sb}"), theme::WARN)
                .on_hover_text("ShowBuddy lights are only hidden — restore them under ShowBuddy.");
                if theme::tool_button(ui, Icon::Close, Some("Keep"), "Leave the patch alone.", Ok(()))
                    .clicked()
                {
                    self.insp.patch.confirm_unpatch = None;
                }
            });
        } else {
            theme::toolbar(ui, |ui| {
                if theme::tool_button(
                    ui,
                    Icon::Jump,
                    Some("Arrange in Selection"),
                    "Open the Selection tab to arrange, aim or hang the selected lights.",
                    gate,
                )
                .clicked()
                {
                    self.insp.set_tab(InspectorTab::Selection);
                }
                if theme::tool_button(
                    ui,
                    Icon::Unplug,
                    Some("Unpatch…"),
                    "Take the selected lights out of the rig — asks once more first.",
                    gate,
                )
                .clicked()
                {
                    self.patch_confirm(UnpatchScope::Selection);
                }
            });
        }
    }

    // ---- 8. ShowBuddy ----

    fn patch_showbuddy_body(&mut self, ui: &mut egui::Ui) {
        let mut include = self.include_showbuddy;
        if ui
            .checkbox(&mut include, "Include ShowBuddy patch")
            .on_hover_text(
                "Untick to start fresh: only DMXpress-patched fixtures stay in the rig, and \
                 ShowBuddy's preset banks are hidden too. Everything comes back when re-ticked.",
            )
            .changed()
        {
            self.include_showbuddy = include;
            self.toggle_showbuddy_saved();
        }
        if self.include_showbuddy && !std::path::Path::new(crate::showbuddy::DEFAULT_CONFIG).exists()
        {
            let detail = if self.showbuddy_patch.is_empty() {
                "ShowBuddy is not reachable on this machine and no saved copy of its fixtures is \
                 available, so only DMXpress-patched fixtures are in the rig."
                    .to_string()
            } else {
                format!(
                    "ShowBuddy is not reachable on this machine; using the {} fixture(s) saved \
                     with this show.",
                    self.showbuddy_patch.len()
                )
            };
            theme::pill(ui, "ShowBuddy not reachable", theme::WARN).on_hover_text(detail);
        }
        if self.excluded_fixtures.is_empty() {
            theme::hint(ui, "No ShowBuddy lights are hidden.");
            return;
        }
        ui.add_space(4.0);
        theme::hint(ui, "Removed from the rig:");
        let removed = self.excluded_fixtures.clone();
        let mut restore: Option<usize> = None;
        let mut all = false;
        theme::well(ui, |ui| {
            for (i, key) in removed.iter().enumerate() {
                ui.horizontal(|ui| {
                    if theme::tool_button(
                        ui,
                        Icon::Reset,
                        None,
                        "Put this ShowBuddy light back in the rig.",
                        Ok(()),
                    )
                    .clicked()
                    {
                        restore = Some(i);
                    }
                    theme::hint(ui, key.as_str());
                });
            }
        });
        theme::toolbar(ui, |ui| {
            if theme::tool_button(
                ui,
                Icon::Reset,
                Some("Restore all"),
                "Put every hidden ShowBuddy light back in the rig.",
                Ok(()),
            )
            .clicked()
            {
                all = true;
            }
        });
        if all {
            self.excluded_fixtures.clear();
            self.log.push("Restored all ShowBuddy fixtures".into());
            self.save_user_patch();
            self.rebuild_patch();
        } else if let Some(i) = restore {
            let key = self.excluded_fixtures.remove(i);
            self.log.push(format!("Restored ShowBuddy fixture '{key}'"));
            self.save_user_patch();
            self.rebuild_patch();
        }
    }

    /// Flip the ShowBuddy merge from the capacity chip.
    fn toggle_showbuddy(&mut self) {
        self.include_showbuddy = !self.include_showbuddy;
        self.toggle_showbuddy_saved();
    }

    fn toggle_showbuddy_saved(&mut self) {
        self.log.push(format!(
            "ShowBuddy patch {}",
            if self.include_showbuddy { "included" } else { "hidden" }
        ));
        self.save_user_patch();
        self.rebuild_patch();
    }

    // ---- the pipeline ----

    /// Turn a request into fixtures: plan the addresses, name the copies,
    /// push them onto `user_fixtures`, persist and rebuild. Returns the
    /// layout keys of the new lights — the only handle that survives
    /// `rebuild_patch`, which invalidates every index.
    pub(crate) fn patch_run(&mut self, req: &PatchRequest) -> PatchOutcome {
        if req.span == 0 || req.count == 0 {
            self.log.push("Nothing to patch: that mode has no channels".into());
            return PatchOutcome::default();
        }
        let ranges = addressing::ranges(&self.patch.fixtures);
        let (starts, stop) = addressing::plan_addresses(
            &ranges,
            req.span,
            req.count.clamp(1, 64),
            req.start,
            req.skip_taken,
            req.same_universe,
        );
        if starts.is_empty() {
            self.log.push(if req.skip_taken {
                format!(
                    "Patch stopped: no free run of {}ch at or after address {}",
                    req.span, req.start
                )
            } else {
                format!(
                    "Patch stopped: address {} + {}ch exceeds slot {DMX_SLOTS}",
                    req.start, req.span
                )
            });
            return PatchOutcome { keys: Vec::new(), stopped: true, next_addr: req.start };
        }
        let names = addressing::display_names(&req.base_name, starts.len(), req.first_number);
        let mut keys = Vec::new();
        for (from, display) in starts.iter().copied().zip(names) {
            self.log.push(format!("Patched '{display}' at {from}-{}", from + req.span - 1));
            keys.push(profiles::fixture_key(&display, from));
            self.user_fixtures.push(UserFixture {
                profile: req.profile.clone(),
                display,
                from,
            });
        }
        profiles::note_recent(
            &mut self.recent_profiles,
            profiles::RecentProfile {
                profile: req.profile.clone(),
                label: req.label.clone(),
                channels: req.span,
            },
        );
        let last = starts[starts.len() - 1];
        let next_addr = (last as usize + req.span as usize).min(DMX_SLOTS) as u16;
        self.patch_addr = next_addr;
        self.insp.patch.addr_seen = next_addr;
        self.save_user_patch();
        self.rebuild_patch();
        self.log.push(format!(
            "Quick patch: {} × {} at {}–{}",
            keys.len(),
            req.label,
            starts[0],
            last + req.span - 1
        ));
        PatchOutcome { keys, stopped: matches!(stop, PlanStop::Overflow(_)), next_addr }
    }

    /// The batch the commit would run, or `None` when there is nothing to
    /// patch or nowhere to put it. Writes nothing: the validation the whole
    /// commit hangs on, so that a batch which cannot land raises no truss.
    ///
    /// Both auto-follows are re-derived here rather than trusted from the
    /// frame prologue. The picker can move the pick in the same frame it
    /// asks for the commit (a recent double-clicked, Enter in the search
    /// box), and the numbering and start address the prologue wrote then
    /// belong to the model before this one.
    fn patch_request(&mut self) -> Option<PatchRequest> {
        // A library pick resolves to nothing until the library is in
        // memory, and `patch_selection` would quietly fall back to the
        // built-in profile — so load it before asking what is picked.
        if self.patch_library_sel.is_some() {
            self.ensure_library();
        }
        let sel = self.patch_selection();
        let span = sel.channels as u16;
        if span == 0 {
            self.log.push("Nothing to patch: that mode has no channels".into());
            return None;
        }
        let prefs = self.insp.prefs.patch.clone();
        let base_name = self.patch_base_name(&sel);
        let ranges = addressing::ranges(&self.patch.fixtures);
        if self.insp.patch.addr_auto {
            self.patch_addr = self.follow_addr(&ranges, span);
            self.insp.patch.addr_seen = self.patch_addr;
        }
        if self.insp.patch.number_auto {
            let displays: Vec<&str> =
                self.patch.fixtures.iter().map(|f| f.display.as_str()).collect();
            self.insp.patch.first_number = addressing::next_number(&displays, &base_name);
        }
        let count = self.patch_count.clamp(1, 64);
        // Enter in the name or search box commits without the Patch
        // button's own gate, so the plan is checked here too — before
        // anything is built or saved.
        let (starts, _) = addressing::plan_addresses(
            &ranges,
            span,
            count,
            self.patch_addr,
            prefs.skip_taken,
            prefs.same_universe,
        );
        if starts.is_empty() {
            self.log.push(format!(
                "Patch stopped: no free run of {span}ch at or after address {}",
                self.patch_addr
            ));
            return None;
        }
        Some(PatchRequest {
            profile: sel.profile.clone(),
            label: sel.label.clone(),
            span,
            base_name,
            first_number: self.insp.patch.first_number,
            count,
            start: self.patch_addr,
            skip_taken: prefs.skip_taken,
            same_universe: prefs.same_universe,
        })
    }

    /// Everything the Patch button does: patch, create an element if the
    /// pattern needs one, then place or hang the new lights and select
    /// them.
    fn quick_patch_commit(&mut self) {
        let Some(req) = self.patch_request() else { return };
        let count = req.count;
        let pattern = self.insp.prefs.patch.pattern;
        // Flush unsaved layout edits: the rebuild re-reads the file.
        self.stage.save(&self.patch);
        let out = self.patch_run(&req);
        self.insp.patch.addr_seen = out.next_addr;
        if out.stopped && out.keys.is_empty() {
            return;
        }
        let insts = self.stage.instances_for_keys(&self.patch, &out.keys);
        if insts.is_empty() {
            return;
        }
        // Only now, with lights on the stage that need hanging, is a new
        // element raised — and raised after the rebuild, so its undo step
        // is not wiped by `apply_layout`.
        let target = if pattern.is_mount() { self.resolve_element(count as usize) } else { None };
        match target {
            Some(r) => {
                let faces = self.stage.element_faces(r);
                let face = self.insp.patch.face.min(faces.len().saturating_sub(1));
                let (hung, left) = self.stage.mount_instances(&self.patch, &insts, r, face, None);
                let name = self.stage.element_name(r);
                self.log.push(format!(
                    "Hung {hung} of {} lights on {name} ({})",
                    insts.len(),
                    faces[face]
                ));
                if !left.is_empty() {
                    self.log.push(format!("… {} placed in a line beneath it", left.len()));
                }
            }
            None => {
                let spec = self.place_spec();
                self.stage.place_instances(&self.patch, &insts, &spec);
                if pattern != PlacePattern::Keep {
                    self.log.push(format!(
                        "Placed {} lights: {} ({:.1} m apart)",
                        insts.len(),
                        pattern.label(),
                        self.insp.prefs.patch.spacing
                    ));
                }
            }
        }
        if self.insp.patch.select_after {
            self.select_keys(&out.keys);
        }
        if !self.insp.patch.number_auto {
            self.insp.patch.first_number =
                self.insp.patch.first_number.saturating_add(out.keys.len() as u16);
        }
    }

    /// Select exactly the lights those layout keys name.
    fn select_keys(&mut self, keys: &[String]) {
        let fis: Vec<usize> = keys
            .iter()
            .filter_map(|k| {
                self.patch
                    .fixtures
                    .iter()
                    .position(|f| profiles::fixture_key(&f.display, f.from) == *k)
            })
            .collect();
        if fis.is_empty() {
            return;
        }
        self.stage.clear_selection();
        self.stage.sel_stage = false;
        for fi in fis {
            self.stage.select_fixture(fi, true);
        }
        self.sync_selection_units();
        self.sel_fixture = self.stage.last_selected;
    }

    /// The placement the Place section describes.
    fn place_spec(&self) -> PlaceSpec {
        PlaceSpec {
            pattern: self.insp.prefs.patch.pattern,
            origin: v3(0.0, self.insp.patch.height, 0.0),
            spacing: self.insp.prefs.patch.spacing,
            max_extent: self
                .insp
                .patch
                .fit_to_stage
                .then(|| self.settings.stage_half_w * 2.0),
            row: self.insp.patch.row,
            columns: self.insp.patch.columns as usize,
            radius: self.insp.patch.radius,
            arc_deg: self.insp.patch.arc_deg,
            face_centre: self.insp.patch.face_centre,
            yaw: self.settings.default_yaw,
            pitch: self.settings.default_pitch,
        }
    }

    /// The element the batch will hang on, creating it when asked. Runs
    /// after the patch, so nothing is raised for a batch that never
    /// landed; a new element saves itself, so the rebuild has already
    /// re-read the layout by the time it exists.
    fn resolve_element(&mut self, count: usize) -> Option<ElementRef> {
        match self.insp.patch.element {
            ElementChoice::NewTruss(kind) => {
                let length = self.new_truss_length(kind, count);
                let ti = self.stage.add_truss_fitted(&self.patch, kind, length);
                let r = ElementRef::Truss(ti);
                self.log.push(format!("Added {}", self.stage.element_name(r)));
                Some(r)
            }
            ElementChoice::NewTower => {
                let ti = self.stage.add_tower_fitted(&self.patch);
                let r = ElementRef::Tower(ti);
                self.log.push(format!("Added {}", self.stage.element_name(r)));
                Some(r)
            }
            ElementChoice::Existing(r) => {
                let ok = match r {
                    ElementRef::Tower(i) => i < self.stage.towers.len(),
                    ElementRef::Truss(i) => i < self.stage.trusses.len(),
                };
                ok.then_some(r)
            }
        }
    }

    /// One more of a type, at the next free address, numbered on from the
    /// last and placed beside (or hung next to) it.
    fn patch_one_more(&mut self, groups: &[RigGroup], gi: usize) {
        let Some(g) = groups.get(gi) else { return };
        let Some(profile) = g.profile.clone() else { return };
        let span = g.channels as u16;
        let stem = self
            .patch
            .fixtures
            .get(g.first_fi)
            .map(|f| type_stem(&f.display).to_string())
            .unwrap_or_else(|| g.label.clone());
        let first = {
            let displays: Vec<&str> =
                self.patch.fixtures.iter().map(|f| f.display.as_str()).collect();
            addressing::next_number(&displays, &stem)
        };
        let ranges = addressing::ranges(&self.patch.fixtures);
        let same_universe = self.insp.prefs.patch.same_universe;
        let start = addressing::first_free(&ranges, span, 1, same_universe)
            .unwrap_or_else(|| addressing::next_free(&ranges));
        // The last member's placement, captured as values before the rebuild.
        let last_fi = g
            .fis
            .iter()
            .copied()
            .max_by_key(|&fi| self.patch.fixtures.get(fi).map_or(0, |f| f.from));
        let anchor = last_fi.and_then(|fi| {
            self.stage
                .instances
                .iter()
                .find(|inst| inst.fixture == fi)
                .map(|inst| (inst.t.pos, inst.mount, inst.truss_mount))
        });
        self.stage.save(&self.patch);
        let out = self.patch_run(&PatchRequest {
            profile,
            label: g.label.clone(),
            span,
            base_name: stem,
            first_number: first,
            count: 1,
            start,
            skip_taken: false,
            same_universe,
        });
        let insts = self.stage.instances_for_keys(&self.patch, &out.keys);
        if insts.is_empty() {
            return;
        }
        match anchor {
            Some((_, _, Some((ti, slot)))) if ti < self.stage.trusses.len() => {
                let per_face = self.stage.trusses[ti].slot_count().max(1);
                let face = slot / per_face;
                self.stage.mount_instances(
                    &self.patch,
                    &insts,
                    ElementRef::Truss(ti),
                    face,
                    Some(slot),
                );
            }
            Some((_, Some((ti, slot)), _)) if ti < self.stage.towers.len() => {
                let face = usize::from(slot >= 4);
                self.stage.mount_instances(
                    &self.patch,
                    &insts,
                    ElementRef::Tower(ti),
                    face,
                    Some(slot),
                );
            }
            Some((pos, _, _)) => {
                let spacing = self.insp.prefs.patch.spacing.max(0.2);
                let spec = PlaceSpec {
                    pattern: PlacePattern::LineX,
                    origin: v3(pos.x + spacing, pos.y, pos.z),
                    spacing,
                    max_extent: None,
                    row: pos.z,
                    columns: 0,
                    radius: 1.0,
                    arc_deg: 0.0,
                    face_centre: false,
                    yaw: self.settings.default_yaw,
                    pitch: self.settings.default_pitch,
                };
                self.stage.place_instances(&self.patch, &insts, &spec);
            }
            None => {
                let spec = self.place_spec();
                self.stage.place_instances(&self.patch, &insts, &spec);
            }
        }
        self.select_keys(&out.keys);
    }

    /// Take lights out of the rig: DMXpress ones are deleted, ShowBuddy
    /// ones are hidden (and restorable under ShowBuddy).
    fn unpatch(&mut self, scope: &UnpatchScope) {
        let pick: Vec<usize> = match scope {
            UnpatchScope::Selection => self.stage.selected_fixtures(),
            UnpatchScope::Type(file) => self
                .patch
                .fixtures
                .iter()
                .enumerate()
                .filter(|(_, f)| f.file.to_string_lossy() == *file.as_str())
                .map(|(i, _)| i)
                .collect(),
            UnpatchScope::One(key) => self
                .patch
                .fixtures
                .iter()
                .enumerate()
                .filter(|(_, f)| profiles::fixture_key(&f.display, f.from) == *key)
                .map(|(i, _)| i)
                .collect(),
        };
        let victims: Vec<(String, u16, bool)> = pick
            .iter()
            .filter_map(|&fi| self.patch.fixtures.get(fi))
            .map(|f| (f.display.clone(), f.from, is_user_fixture(&f.file.to_string_lossy())))
            .collect();
        if victims.is_empty() {
            return;
        }
        let (mut removed, mut hidden) = (0usize, 0usize);
        self.user_fixtures.retain(|uf| {
            let doomed = victims
                .iter()
                .any(|(d, a, user)| *user && uf.display == *d && uf.from == *a);
            if doomed {
                removed += 1;
            }
            !doomed
        });
        for (display, from, user) in &victims {
            if *user {
                continue;
            }
            let key = profiles::fixture_key(display, *from);
            if !self.excluded_fixtures.contains(&key) {
                self.excluded_fixtures.push(key);
                hidden += 1;
            }
        }
        let names: Vec<&str> = victims.iter().take(3).map(|(d, _, _)| d.as_str()).collect();
        let tail = if victims.len() > 3 { "…" } else { "" };
        self.log.push(format!(
            "Unpatched {} lights ({removed} removed, {hidden} ShowBuddy hidden): {}{tail}",
            victims.len(),
            names.join(", ")
        ));
        self.save_user_patch();
        self.rebuild_patch();
    }

    /// Rename every light of one type, keeping each one's place on the
    /// stage (the layout is keyed by `display@from`, so it has to move with
    /// the names).
    fn rename_type_prefix(&mut self, file: &str, new_prefix: &str) {
        let mut members: Vec<(String, u16)> = self
            .patch
            .fixtures
            .iter()
            .filter(|f| f.file.to_string_lossy() == *file)
            .map(|f| (f.display.clone(), f.from))
            .collect();
        // Only DMXpress's own fixtures can be renamed: a ShowBuddy light's
        // name comes back from ShowBuddy on the next rebuild, and rewriting
        // its layout key would orphan its position.
        members.retain(|(display, from)| {
            self.user_fixtures.iter().any(|uf| uf.display == *display && uf.from == *from)
        });
        if members.is_empty() {
            self.log.push("Rename ShowBuddy fixtures in ShowBuddy".into());
            return;
        }
        members.sort_by_key(|(_, from)| *from);
        let old_prefix = type_stem(&members[0].0).to_string();
        let names = addressing::display_names(new_prefix, members.len(), 1);
        let mut lf = self.stage.export_layout(&self.patch);
        for ((old, from), new) in members.iter().zip(&names) {
            let old_key = profiles::fixture_key(old, *from);
            let new_key = profiles::fixture_key(new, *from);
            for si in lf.instances.iter_mut() {
                if si.key == old_key {
                    si.key = new_key.clone();
                }
            }
            for uf in self.user_fixtures.iter_mut() {
                if uf.display == *old && uf.from == *from {
                    uf.display = new.clone();
                }
            }
        }
        self.save_user_patch();
        self.rebuild_patch();
        let settings = self.settings.clone();
        self.stage.import_layout(&self.patch, &settings, lf);
        self.log.push(format!(
            "Renamed {} × '{old_prefix}' → '{new_prefix}'",
            members.len()
        ));
    }

    /// The rig as a plain-text address sheet for the desk label.
    fn rig_sheet_text(&self, groups: &[RigGroup]) -> String {
        let label_of = |f: &crate::showbuddy::Fixture| {
            let file = f.file.to_string_lossy().to_string();
            groups.iter().find(|g| g.file == file).map(|g| g.label.clone()).unwrap_or(file)
        };
        addressing::rig_sheet(&self.patch.fixtures, &label_of)
    }
}

/// How long a straight run has to be to hold `n` lights on one face.
fn fit_length(n: usize) -> f32 {
    (n as f32 * SLOT_PITCH).clamp(1.0, 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Frame;
    use crate::stage::headless::{render_frames, save};

    fn lit(pixels: &[u8]) -> usize {
        pixels.chunks(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120).count()
    }

    #[test]
    fn category_class_keywords() {
        assert_eq!(category_class("Moving Head"), ArchClass::Moving);
        assert_eq!(category_class("Color Changer"), ArchClass::Par);
        assert_eq!(category_class("Pixel Bar"), ArchClass::Bar);
        assert_eq!(category_class("Strobe"), ArchClass::Strobe);
        assert_eq!(category_class(""), ArchClass::Other);
        assert_eq!(category_class("Fog machine"), ArchClass::Other);
    }

    #[test]
    fn rig_groups_groups_by_file_sorted_by_first_address() {
        let par = profiles::find("Generic RGBW Par (4ch)").expect("the generic par profile");
        let fog = profiles::find("Fogger (2ch)").expect("the fogger profile");
        let patch = Patch {
            fixtures: vec![
                par.to_fixture("Par 1".into(), 1),
                par.to_fixture("Par 2".into(), 5),
                fog.to_fixture("Fog".into(), 9),
            ],
            warnings: Vec::new(),
        };
        let groups = rig_groups(&patch, &Library::default());
        assert_eq!(groups.len(), 2);
        let g = &groups[0];
        assert_eq!((g.count, g.channels, g.from_min, g.to_max), (2, 4, 1, 8));
        assert!(g.is_user);
        assert_eq!(g.profile.as_deref(), Some("Generic RGBW Par (4ch)"));
        assert_eq!(g.fis, vec![0, 1]);
        assert_eq!(groups[1].label, "Fogger");
        assert_eq!(groups[1].count, 1);
    }

    #[test]
    fn fit_length_gives_one_slot_per_light() {
        assert_eq!(fit_length(6), 3.0);
        assert_eq!(fit_length(1), 1.0);
        assert_eq!(fit_length(40), 12.0);
    }

    /// The capacity the "On" row promises for an element that does not
    /// exist yet has to be the capacity the element will really have.
    #[test]
    fn a_new_element_promises_only_the_slots_it_will_have() {
        let mut app = crate::app::App::new();
        let slots = |app: &App, kind, count| {
            stage::fitted_truss(kind, app.new_truss_length(kind, count)).slot_count()
        };
        app.insp.patch.fit_length = true;
        assert_eq!(app.new_truss_length(TrussKind::Straight, 6), Some(3.0));
        assert_eq!(slots(&app, TrussKind::Straight, 6), 6, "a fitted run takes the lot");
        assert_eq!(slots(&app, TrussKind::Straight, 40), 24, "but only to twelve metres");
        app.insp.patch.fit_length = false;
        app.insp.patch.new_length = 1.0;
        assert_eq!(app.new_truss_length(TrussKind::Straight, 6), Some(1.0));
        assert_eq!(slots(&app, TrussKind::Straight, 6), 2, "a 1 m run holds two, not six");
        // A radius run is sized by its arc, so the batch never sizes it.
        assert_eq!(app.new_truss_length(TrussKind::Radius, 8), None);
        assert_eq!(slots(&app, TrussKind::Radius, 8), 6);
    }

    /// A rig of `n` 4-channel pars, in memory only.
    fn par_rig(n: u16) -> Patch {
        let par = profiles::find("Generic RGBW Par (4ch)").expect("the generic par profile");
        Patch {
            fixtures: (0..n)
                .map(|i| par.to_fixture(format!("Generic RGBW Par {}", i + 1), 1 + i * 4))
                .collect(),
            warnings: Vec::new(),
        }
    }

    /// An App with a rig of its own and both auto-follows off. Nothing
    /// here calls `patch_run` or `unpatch`, so nothing on disk changes.
    fn commit_app(rig: Patch) -> crate::app::App {
        let mut app = crate::app::App::new();
        app.patch = rig;
        app.patch_library_sel = None;
        app.patch_name.clear();
        app.patch_count = 1;
        app.insp.patch.addr_auto = false;
        app.insp.patch.number_auto = true;
        app.insp.prefs.patch.skip_taken = true;
        app.insp.prefs.patch.same_universe = true;
        app
    }

    fn profile_at(name: &str) -> usize {
        PROFILES.iter().position(|p| p.name == name).unwrap_or_else(|| panic!("{name} is gone"))
    }

    /// The picker can move the pick in the same frame it asks for the
    /// commit, so the request must be built from the model being patched,
    /// not from the one the frame prologue planned for.
    #[test]
    fn a_commit_renumbers_for_the_model_it_is_really_patching() {
        let mut app = commit_app(par_rig(5));
        app.patch_addr = 21;
        // The prologue numbered the batch for the par that was picked…
        app.patch_profile = profile_at("Generic RGBW Par (4ch)");
        app.insp.patch.first_number = 6;
        // …and a recent double-click switched to the fogger in that frame.
        app.patch_profile = profile_at("Fogger (2ch)");
        let req = app.patch_request().expect("there is room at address 21");
        assert_eq!(req.base_name, "Fogger");
        assert_eq!(req.first_number, 1, "the first Fogger is not 'Fogger 6'");
        assert_eq!(
            addressing::display_names(&req.base_name, 1, req.first_number),
            vec!["Fogger"]
        );
    }

    /// In FirstGap mode the stale start address was a hole that fitted the
    /// *previous* model's span — patching the new one straight over the
    /// lights that follow it.
    #[test]
    fn a_commit_re_finds_the_first_gap_for_the_new_span() {
        let par = profiles::find("Generic RGBW Par (4ch)").expect("the generic par profile");
        let mut app = commit_app(Patch {
            fixtures: vec![
                par.to_fixture("Par 1".into(), 1),
                par.to_fixture("Par 2".into(), 7),
            ],
            warnings: Vec::new(),
        });
        app.insp.patch.addr_auto = true;
        app.insp.patch.addr_mode = AddrMode::FirstGap;
        app.insp.patch.addr_seen = 0;
        // 5–6 is the hole a 2-channel fogger found last frame.
        app.patch_addr = 5;
        app.patch_profile = profile_at("Generic RGBW Par (4ch)");
        let req = app.patch_request().expect("there is room past the second par");
        assert_eq!(req.span, 4);
        assert_eq!(req.start, 11, "a 4-channel hole, not the fogger's two");
    }

    /// Enter in the name or the search box commits without the Patch
    /// button's gate. The request has to refuse the batch on its own, or a
    /// new truss is raised and saved for lights that never arrive.
    #[test]
    fn a_commit_refuses_a_batch_with_nowhere_to_go() {
        let mut app = commit_app(par_rig(5));
        app.insp.prefs.patch.pattern = PlacePattern::Truss;
        app.insp.patch.element = ElementChoice::NewTruss(TrussKind::Straight);
        let trusses = app.stage.trusses.len();
        let patched = app.user_fixtures.len();
        app.patch_addr = 1024;
        app.patch_profile = profile_at("Fogger (2ch)");
        assert!(app.patch_request().is_none(), "1024 + 2ch runs past slot 1024");
        assert!(app.log.iter().any(|l| l.contains("Patch stopped")), "{:?}", app.log.last());
        assert_eq!(app.stage.trusses.len(), trusses, "no orphan truss");
        assert_eq!(app.user_fixtures.len(), patched, "and nothing patched");
        // One slot lower it fits, and the request comes back.
        app.patch_addr = 1023;
        assert!(app.patch_request().is_some());
    }

    /// `DragValue::range` clamps the value it is given and reports that
    /// clamp as a change, which used to rewrite a number the auto-follow
    /// had produced *and* turn "auto" off by itself — so two lights of the
    /// series ended up sharing a name. Real egui, no GPU.
    #[test]
    fn auto_numbering_past_the_number_field_is_left_alone() {
        let par = profiles::find("Generic RGBW Par (4ch)").expect("the generic par profile");
        let mut app = crate::app::App::new();
        app.collapsed.insert("channels");
        app.collapsed.insert("fixtures");
        app.collapsed.remove("inspector");
        app.show_log = false;
        app.insp.prefs.width = Some(300.0);
        app.insp.prefs.tab = InspectorTab::Patch;
        app.patch = Patch {
            fixtures: vec![par.to_fixture("Spot 1000".into(), 1)],
            warnings: Vec::new(),
        };
        app.patch_name = "Spot".into();
        app.insp.patch.number_auto = true;
        let ctx = egui::Context::default();
        crate::ui::install_theme(&ctx);
        for _ in 0..3 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.draw_ui(ctx),
            );
        }
        assert_eq!(app.insp.patch.first_number, 1001, "the series carries on past 999");
        assert!(app.insp.patch.number_auto, "drawing the field must not cancel auto");
    }

    /// The Patch tab at the panel's 230 px minimum in four states, written
    /// to `target/ui_inspector_patch*.png`. State only: this never calls
    /// `patch_run`, `unpatch`, `patch_one_more` or the ShowBuddy toggle, so
    /// nothing on disk changes.
    #[test]
    fn inspector_patch_tab_renders_headless() {
        let mut app = crate::app::App::new();
        *app.net.dmx.lock() = Frame([170u8; DMX_SLOTS]);
        app.collapsed.insert("channels");
        app.collapsed.insert("fixtures");
        app.collapsed.remove("inspector");
        app.show_log = false;
        app.show_osc = false;
        app.insp.prefs.width = Some(230.0);
        app.insp.prefs.tab = InspectorTab::Patch;
        let patched = app.user_fixtures.len();
        let size = [1000, 1150];
        let shot_at = |app: &mut crate::app::App, name: &str, size: [u32; 2]| -> bool {
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
        let shot = |app: &mut crate::app::App, name: &str| shot_at(app, name, size);

        // 1. Fresh: no recents, the built-in profiles, a line across stage.
        app.recent_profiles.clear();
        app.insp.patch.source = PatchSource::Builtin;
        app.patch_library_sel = None;
        app.patch_profile = 2;
        app.patch_count = 6;
        app.patch_name = "Spot".into();
        if !shot(&mut app, "ui_inspector_patch") {
            return;
        }

        // 2. Library search, a fixture picked, hung on a new truss.
        app.recent_profiles = vec![
            profiles::RecentProfile {
                profile: "Maverick MK2 Spot (32ch)".into(),
                label: "Maverick MK2 Spot".into(),
                channels: 32,
            },
            profiles::RecentProfile {
                profile: "Generic RGBW Par (4ch)".into(),
                label: "Generic RGBW Par".into(),
                channels: 4,
            },
        ];
        app.insp.patch.source = PatchSource::Library;
        app.patch_search = "chauvet spot".into();
        app.insp.prefs.patch.pattern = crate::stage::PlacePattern::Truss;
        app.insp.patch.element = ElementChoice::NewTruss(TrussKind::Straight);
        for i in 0..3.min(app.patch.fixtures.len()) {
            app.stage.select_fixture(i, true);
        }
        shot(&mut app, "ui_inspector_patch_library");
        if app.library.is_loaded() && app.library.error.is_none() {
            assert!(app.insp.patch.hits_key.is_some(), "the hit list was never built");
        }

        // 3. The rig summary, with one type open and an armed unpatch.
        app.insp.patch.source = PatchSource::Builtin;
        app.patch_library_sel = None;
        app.insp.prefs.patch.pattern = crate::stage::PlacePattern::Grid;
        app.insp.patch.addr_auto = false;
        app.patch_addr = 500;
        app.patch_count = 8;
        app.insp.prefs.patch.skip_taken = false;
        app.insp.patch.confirm_unpatch = Some((UnpatchScope::Selection, Instant::now()));
        let files: Vec<String> = app
            .patch
            .fixtures
            .iter()
            .map(|f| f.file.to_string_lossy().to_string())
            .collect();
        for file in files.into_iter().take(2) {
            app.insp.patch.open_groups.insert(file);
        }
        shot(&mut app, "ui_inspector_patch_rig");

        // 4. Zoom 2: every row has to wrap, not overflow.
        app.zoom.inspector = 2.0;
        app.insp.patch.confirm_unpatch = None;
        shot(&mut app, "ui_inspector_patch_zoom2");
        app.zoom.inspector = 1.0;

        // 5. Tall enough that the whole wizard is on screen at once: the
        //    place pads, the Patch button, the rig cards and the footer.
        app.insp.prefs.patch.pattern = crate::stage::PlacePattern::Truss;
        app.insp.patch.face = 1;
        app.patch_count = 6;
        app.insp.patch.open_groups.clear();
        if let Some(g) = rig_groups(&app.patch, &app.library).first() {
            app.insp.patch.open_groups.insert(g.file.clone());
        }
        shot_at(&mut app, "ui_inspector_patch_tall", [820, 1900]);

        // 6. The foot of the tab with the rig folded away: the Selection
        //    section armed for an unpatch, ShowBuddy open, the footer link.
        app.insp.prefs.folded.insert("patch.rig".into());
        app.insp.prefs.folded.insert("patch.showbuddy".into());
        app.insp.patch.confirm_unpatch = Some((UnpatchScope::Selection, Instant::now()));
        shot_at(&mut app, "ui_inspector_patch_foot", [820, 2000]);
        app.insp.prefs.folded.clear();
        app.insp.patch.confirm_unpatch = None;

        // 7. A new tower has four slots a face whatever the batch asks
        //    for, so the capacity line has to warn instead of promise.
        app.insp.prefs.patch.pattern = crate::stage::PlacePattern::Tower;
        app.insp.patch.element = ElementChoice::NewTower;
        app.insp.patch.face = 0;
        app.patch_count = 6;
        shot_at(&mut app, "ui_inspector_patch_tower", [820, 1900]);

        assert!(!app.insp.dirty, "the headless scenes must not mark prefs dirty");
        assert_eq!(app.user_fixtures.len(), patched, "the scenes must not patch anything");
    }
}




#[cfg(test)]
mod width_tests {
    use super::*;

    /// The panel is resizable and grows to whatever its content asks for,
    /// so a row that measures itself wrong does not overflow — it quietly
    /// drags the whole Inspector wider, frame after frame. This walks the
    /// tab's states with the real `draw_ui` (no GPU needed) and reads the
    /// panel's own width back afterwards.
    #[test]
    fn patch_tab_holds_the_narrow_panel() {
        let mut app = crate::app::App::new();
        app.collapsed.insert("channels");
        app.collapsed.insert("fixtures");
        app.collapsed.remove("inspector");
        app.show_log = false;
        app.insp.prefs.width = Some(230.0);
        app.insp.prefs.tab = InspectorTab::Patch;
        let ctx = egui::Context::default();
        crate::ui::install_theme(&ctx);
        let width = |ctx: &egui::Context| {
            ctx.data(|d| {
                d.get_temp::<egui::containers::panel::PanelState>(egui::Id::new("inspector"))
                    .map_or(0.0, |p| p.rect.width())
            })
        };
        let run = |app: &mut crate::app::App, what: &str, limit: f32| {
            let mut last = 0.0;
            for _ in 0..6 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.draw_ui(ctx),
                );
                last = width(&ctx);
            }
            assert!(last < limit, "{what}: the panel grew to {last:.0} px");
        };

        // The built-in picker, a free-standing line.
        run(&mut app, "built-in", 245.0);

        // The rig summary with two types open and a confirm armed.
        app.insp.prefs.patch.pattern = PlacePattern::Grid;
        let files: Vec<String> =
            app.patch.fixtures.iter().map(|f| f.file.to_string_lossy().to_string()).collect();
        for file in files.into_iter().take(2) {
            app.insp.patch.open_groups.insert(file);
        }
        app.insp.patch.confirm_unpatch = Some((UnpatchScope::Selection, Instant::now()));
        run(&mut app, "open rig", 245.0);

        // Hanging a batch on a new truss (the element combo and the faces).
        app.insp.patch.confirm_unpatch = None;
        app.insp.prefs.patch.pattern = PlacePattern::Truss;
        app.insp.patch.element = ElementChoice::NewTruss(TrussKind::Straight);
        run(&mut app, "truss", 245.0);
        app.insp.patch.face = 2;
        app.insp.patch.fit_length = false;
        run(&mut app, "truss, fixed length", 245.0);

        // The library search, which also loads the library.
        app.insp.patch.source = PatchSource::Library;
        app.patch_search = "chauvet spot".into();
        run(&mut app, "library", 245.0);

        // A ComboBox's width is a floor, not a cap: both of these carry a
        // name long enough to widen the panel unless it is elided.
        app.patch_manufacturer = Some("Chauvet Professional Lighting Company".into());
        run(&mut app, "long maker name", 245.0);
        let mut truss = crate::stage::Truss::straight();
        truss.name = "Upstage mother grid, downstage side".into();
        app.stage.trusses.push(truss);
        app.insp.prefs.patch.pattern = PlacePattern::Truss;
        app.insp.patch.element = ElementChoice::Existing(ElementRef::Truss(
            app.stage.trusses.len() - 1,
        ));
        run(&mut app, "long element name", 245.0);

        // A+ doubles every hand-painted pixel, so the panel may be twice as
        // wide — but no wider, and it still has to settle.
        app.zoom.inspector = 2.0;
        run(&mut app, "library at zoom 2", 490.0);
    }
}
