//! The right-hand Inspector: a `Presets · Selection · Stage · Build · Patch`
//! tab strip, a context strip summarising what is picked on the stage, and
//! one file per tab. The shell owns the strip, the context row, the
//! cosmetic preferences in `inspector.json` and the hotkeys; the tabs own
//! their bodies (`App::inspector_<tab>_tab`).
//!
//! Every tab is written in the kit vocabulary of `theme.rs` so five areas
//! produce one panel. The rules, binding for every tab:
//!
//! 1. Headings are `theme::section` / `section_with`, or `App::insp_fold`
//!    when collapsible; never `ui.heading`, `ui.separator`, `ui.weak` or
//!    `CollapsingHeader` inside a tab.
//! 2. Captions are `theme::hint` (one line, ≤ ~60 chars at 230 px; never
//!    more than two in a row); longer help goes into `theme::help(ui, text)`
//!    in a `section_with` right slot.
//! 3. Status words are `theme::pill`; counts are `theme::count_pill`.
//! 4. Raised groups are `theme::card` with `card_title`; lists, drop wells
//!    and previews are `theme::well`; cards do not nest.
//! 5. Lists are `theme::list_row`; empty lists show `theme::empty_state`.
//! 6. Label/value rows are `theme::kv_grid(ui, "insp_<tab>_<name>", ..)`
//!    with `DragValue`s ≤ 64 px wide; every Grid / ComboBox / ScrollArea
//!    salt starts with `insp_<tab>_`; repeated rows use
//!    `ui.push_id(("insp", tab.key(), i), ..)`.
//! 7. Icon controls are `theme::tool_button` / `toggle_icon` / `wide_button`
//!    / `segmented` inside `theme::toolbar`; no emoji or Unicode glyph
//!    buttons ("➕", "◀") in new code.
//! 8. Buttons that need a selection go through `tool_button(.., Err(why))`
//!    or `theme::gated` with a full-sentence reason.
//! 9. Every control has `.on_hover_text` with a full sentence ending in a
//!    period.
//! 10. Button rows are `theme::toolbar` — never a plain `ui.horizontal` of
//!     more than three buttons.
//! 11. Pads are `theme::pad_button` with `pad_columns`.
//! 12. No fixed widths > 150 px; `ComboBox::width(ui.available_width())` in
//!     a full row; `TextEdit::desired_width(f32::INFINITY)` or
//!     `available_width() - reserved`.
//! 13. Custom painting scales with `theme::zoom_of(ui)`.
//! 14. Colours only from `theme::*`; the raid blue SELECT ring is
//!     stage-only; addresses and metres in `ui.monospace`.
//! 15. Mutations inside row closures are deferred into `Option` / `Vec`
//!     locals and applied after (the side.rs pattern).
//! 16. Every user-visible mutation pushes one human sentence to `self.log`;
//!     cosmetic toggles do not.
//! 17. Destructive actions are `wide_button(.., Tone::Danger)` /
//!     `danger_button`, or an ordinary button inside a `confirm` window —
//!     never a bare red label.
//! 18. Cosmetic state goes in `self.insp.prefs` + `mark_dirty()`; show data
//!     follows the brief's persistence rules.
//!
//! Every selection change made from a tab ends with
//! `self.sync_selection_units(); self.sel_fixture = self.stage.last_selected;`.

use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{self, Color32};
use serde::{Deserialize, Serialize};

use super::{
    icons::{self, Icon},
    theme,
};
use crate::app::App;
use crate::showbuddy::Patch;
use crate::stage::{
    fixture_level, fixture_swatch, ElementRef, Settings, StageView, TrussKind, TOWER_SLOTS,
};

pub(crate) mod addressing; // F (pure maths)
pub(crate) mod build; // C
pub(crate) mod patch; // F
pub(crate) mod presets; // D
pub(crate) mod selection; // E
pub(crate) mod stage; // B

pub(crate) const INSPECTOR_FILE: &str = "inspector.json";

/// The five tabs, in strip order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub(crate) enum InspectorTab {
    #[default]
    Presets,
    Selection,
    Stage,
    Build,
    Patch,
}

impl InspectorTab {
    pub const ALL: [InspectorTab; 5] =
        [Self::Presets, Self::Selection, Self::Stage, Self::Build, Self::Patch];

    pub fn label(self) -> &'static str {
        match self {
            Self::Presets => "Presets",
            Self::Selection => "Selection",
            Self::Stage => "Stage",
            Self::Build => "Build",
            Self::Patch => "Patch",
        }
    }

    /// Id salts and PNG names.
    pub fn key(self) -> &'static str {
        match self {
            Self::Presets => "presets",
            Self::Selection => "selection",
            Self::Stage => "stage",
            Self::Build => "build",
            Self::Patch => "patch",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Presets => Icon::Presets,
            Self::Selection => Icon::Selection,
            Self::Stage => Icon::Stage,
            Self::Build => Icon::Build,
            Self::Patch => Icon::Patch,
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Presets => "Store and recall looks, ShowBuddy banks and blackout (Alt+1).",
            Self::Selection => "Move, rotate and arrange what is selected on the stage (Alt+2).",
            Self::Stage => "Camera, saved views and what the stage shows (Alt+3).",
            Self::Build => "Add towers and truss, name them, save setups (Alt+4).",
            Self::Patch => "Patch fixtures quickly and hang them on the rig (Alt+5).",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Self {
        Self::ALL[i % Self::ALL.len()]
    }

    #[cfg(test)]
    pub fn next(self) -> Self {
        Self::from_index(self.index() + 1)
    }

    #[cfg(test)]
    pub fn prev(self) -> Self {
        Self::from_index(self.index() + Self::ALL.len() - 1)
    }

    /// The count (or letter) shown at the segment's corner, with its colour.
    pub fn badge(self, app: &App) -> Option<(String, Color32)> {
        match self {
            Self::Presets => {
                let n = app.user_presets.len();
                (n > 0).then(|| (n.to_string(), theme::TEXT_DIM))
            }
            Self::Selection => {
                let n = app.stage.selected_fixtures().len();
                if n > 0 {
                    Some((n.to_string(), theme::ACCENT_SOFT))
                } else if app.stage.sel_tower.is_some() {
                    Some(("T".into(), theme::ACCENT_MUTED))
                } else if app.stage.sel_truss.is_some() {
                    Some(("F".into(), theme::ACCENT_MUTED))
                } else if app.stage.sel_stage {
                    Some(("S".into(), theme::ACCENT_MUTED))
                } else {
                    None
                }
            }
            Self::Stage => None,
            Self::Build => {
                let n = app.stage.towers.len() + app.stage.trusses.len();
                (n > 0).then(|| (n.to_string(), theme::TEXT_DIM))
            }
            Self::Patch => {
                let warn = app.patch.warnings.len();
                let n = app.patch.fixtures.len();
                if warn > 0 {
                    Some((warn.to_string(), theme::WARN))
                } else if n > 0 {
                    Some((n.to_string(), theme::TEXT_DIM))
                } else {
                    None
                }
            }
        }
    }
}

/// Cosmetic Inspector preferences: never in Configuration or undo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct InspectorPrefs {
    pub tab: InspectorTab,
    pub display: crate::stage::DisplayToggles,
    /// Mirrors `StageView.fly_mode`.
    pub fly_mode: bool,
    pub follow_selection: bool,
    /// Fold keys whose state differs from their default.
    pub folded: BTreeSet<String>,
    /// Docked panel width; `None` = 260.
    pub width: Option<f32>,
    pub presets: presets::PresetsPrefs,
    pub selection: selection::SelectionPrefs,
    pub stage: stage::StagePrefs,
    pub build: build::BuildPrefs,
    pub patch: patch::PatchPrefs,
}

impl Default for InspectorPrefs {
    fn default() -> Self {
        Self {
            tab: InspectorTab::Presets,
            display: crate::stage::DisplayToggles::default(),
            fly_mode: false,
            follow_selection: false,
            folded: BTreeSet::new(),
            width: None,
            presets: Default::default(),
            selection: Default::default(),
            stage: Default::default(),
            build: Default::default(),
            patch: Default::default(),
        }
    }
}

impl InspectorPrefs {
    pub fn load() -> Self {
        std::fs::read_to_string(INSPECTOR_FILE)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(INSPECTOR_FILE, json);
        }
    }
}

/// What "the stage focus" looked like last frame, for follow-selection.
type FocusKey = (usize, Option<usize>, Option<usize>, bool, Option<usize>);

/// Inspector shell state; one field on `App`.
#[derive(Default)]
pub(crate) struct InspectorUi {
    pub prefs: InspectorPrefs,
    /// Set by any prefs change; `flush` saves and clears it.
    pub dirty: bool,
    pub presets: presets::PresetsUi,
    pub selection: selection::SelectionUi,
    pub stage: stage::StageUi,
    pub build: build::BuildUi,
    pub patch: patch::PatchUi,
    last_focus: Option<FocusKey>,
    /// The panel's resize handle was being dragged last frame; the width
    /// is stored when the drag ends, never when content merely widens it.
    resizing: bool,
}

impl InspectorUi {
    pub fn load() -> Self {
        Self { prefs: InspectorPrefs::load(), ..Default::default() }
    }

    /// Switch tabs; shows next frame. A no-op when already there.
    pub fn set_tab(&mut self, t: InspectorTab) {
        if self.prefs.tab != t {
            self.prefs.tab = t;
            self.dirty = true;
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Write inspector.json iff something changed.
    pub fn flush(&mut self) {
        if self.dirty {
            self.prefs.save();
            self.dirty = false;
        }
    }

    pub fn is_open(&self, key: &str, default_open: bool) -> bool {
        default_open != self.prefs.folded.contains(key)
    }

    pub fn set_open(&mut self, key: &str, default_open: bool, open: bool) {
        let changed = if open != default_open {
            self.prefs.folded.insert(key.to_owned())
        } else {
            self.prefs.folded.remove(key)
        };
        if changed {
            self.dirty = true;
        }
    }
}

/// What the stage has picked, for the context strip (pure, unit-tested).
pub(crate) enum Focus {
    Nothing,
    Lights {
        instances: usize,
        fixtures: usize,
        /// Type stem → how many selected fixtures carry it, most first.
        groups: Vec<(String, usize)>,
        mounted: usize,
        /// The patch index the strip describes for a single light.
        first: usize,
    },
    Tower {
        index: usize,
        mounted: usize,
        name: String,
    },
    Truss {
        index: usize,
        kind: TrussKind,
        mounted: usize,
        slots: usize,
        name: String,
    },
    Stage,
}

/// Lights win over a picked tower, which wins over a truss, which wins
/// over the stage box; an out-of-range index is nothing.
pub(crate) fn focus_of(stage: &StageView, patch: &Patch) -> Focus {
    if !stage.selection.is_empty() {
        let valid: Vec<usize> =
            stage.selection.iter().copied().filter(|&i| i < stage.instances.len()).collect();
        if valid.is_empty() {
            return Focus::Nothing;
        }
        let fixtures = stage.selected_fixtures();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for &fi in &fixtures {
            if let Some(f) = patch.fixtures.get(fi) {
                *counts.entry(type_stem(&f.display).to_owned()).or_default() += 1;
            }
        }
        let mut groups: Vec<(String, usize)> = counts.into_iter().collect();
        groups.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mounted = valid
            .iter()
            .filter(|&&i| {
                let inst = &stage.instances[i];
                inst.mount.is_some() || inst.truss_mount.is_some()
            })
            .count();
        let first = stage
            .last_selected
            .filter(|&fi| stage.fixture_selected(fi))
            .or(fixtures.first().copied())
            .unwrap_or(0);
        return Focus::Lights { instances: valid.len(), fixtures: fixtures.len(), groups, mounted, first };
    }
    if let Some(i) = stage.sel_tower {
        if i >= stage.towers.len() {
            return Focus::Nothing;
        }
        let mounted =
            stage.instances.iter().filter(|inst| inst.mount.is_some_and(|(t, _)| t == i)).count();
        return Focus::Tower { index: i, mounted, name: stage.element_name(ElementRef::Tower(i)) };
    }
    if let Some(i) = stage.sel_truss {
        let Some(tr) = stage.trusses.get(i) else { return Focus::Nothing };
        let mounted = stage
            .instances
            .iter()
            .filter(|inst| inst.truss_mount.is_some_and(|(t, _)| t == i))
            .count();
        return Focus::Truss {
            index: i,
            kind: tr.kind,
            mounted,
            slots: tr.total_slots(),
            name: stage.element_name(ElementRef::Truss(i)),
        };
    }
    if stage.sel_stage {
        return Focus::Stage;
    }
    Focus::Nothing
}

/// (title, detail) for the context strip.
pub(crate) fn focus_text(f: &Focus, patch: &Patch, set: &Settings) -> (String, String) {
    match f {
        Focus::Nothing => {
            if patch.fixtures.is_empty() {
                ("Nothing patched".into(), "Open the Patch tab to add lights".into())
            } else {
                ("Nothing selected".into(), "Click a light on the stage or in Fixtures".into())
            }
        }
        Focus::Lights { instances, fixtures, groups, mounted, first } => {
            if *fixtures == 1 {
                return match patch.fixtures.get(*first) {
                    Some(fx) => (
                        fx.display.clone(),
                        format!("DMX {}–{} · {} ch", fx.from, fx.to, fx.channel_count()),
                    ),
                    None => ("1 light".into(), String::new()),
                };
            }
            let title = if instances > fixtures {
                format!("{fixtures} lights ({instances} copies)")
            } else {
                format!("{fixtures} lights")
            };
            let mut detail = groups
                .iter()
                .take(2)
                .map(|(n, c)| format!("{n} ×{c}"))
                .collect::<Vec<_>>()
                .join(" · ");
            if groups.len() > 2 {
                detail.push_str(&format!(" +{} more", groups.len() - 2));
            }
            if *mounted > 0 {
                detail.push_str(&format!(" · {mounted} hung"));
            }
            (title, detail)
        }
        Focus::Tower { index, mounted, name } => {
            let title = if name.is_empty() { format!("Tower {}", index + 1) } else { name.clone() };
            (title, format!("{mounted} / {TOWER_SLOTS} slots"))
        }
        Focus::Truss { index, kind, mounted, slots, name } => {
            let (fallback, faces) = match kind {
                TrussKind::Straight => ("F34 truss", 4),
                TrussKind::Radius => ("Radius truss", 2),
            };
            let title = if name.is_empty() { format!("{fallback} {}", index + 1) } else { name.clone() };
            // Slots sit every half metre along the run, so the run length
            // reads back from the slot count.
            let run = (slots / faces) as f32 * 0.5;
            (title, format!("{mounted} / {slots} slots · {run:.1} m"))
        }
        Focus::Stage => (
            "Stage box".into(),
            format!(
                "{:.1} × {:.1} m · {:.1} m high",
                set.stage_half_w * 2.0,
                set.stage_half_d * 2.0,
                set.stage_h
            ),
        ),
    }
}

/// A fixture's type: its display name with one trailing number stripped
/// ("SlimPAR 3" → "SlimPAR").
pub(crate) fn type_stem(display: &str) -> &str {
    let t = display.trim_end();
    let stripped = t.trim_end_matches(|c: char| c.is_ascii_digit());
    if stripped.len() < t.len() && stripped.ends_with(' ') {
        stripped.trim_end()
    } else {
        t
    }
}

/// The stage's gestures, keycap → effect.
pub(crate) const STAGE_HELP: &[(&str, &str)] = &[
    ("drag", "Move a light on the floor"),
    ("⌘ drag", "Change its height"),
    ("⇧ click", "Add to the selection"),
    ("⌘ click", "Select every light of that type"),
    ("drag empty", "Pan the camera"),
    ("⇧ drag empty", "Marquee select"),
    ("right drag", "Orbit"),
    ("middle drag · wheel", "Pan · zoom"),
    ("arrows · rings", "Fine move · rotate (gizmo)"),
    ("drag onto ring", "Snap onto a tower or truss"),
    ("⌘Z · ⌘D · ⌫", "Undo · duplicate · remove copy"),
    ("Alt 1–5 · Alt I", "Inspector tabs · fold the Inspector"),
];

/// What the context strip asked for this frame, applied after its closure.
enum CtxAction {
    All,
    SameType(usize),
    Clear,
    Goto(InspectorTab),
}

impl App {
    /// The prefs the stage mirrors, stamped whether or not the panel — or
    /// the tab that owns the control — is drawn this frame. The arrow-key
    /// nudge settings belong here as much as the display toggles do:
    /// `StageView::nudge_keys` runs over the stage canvas on every tab, so
    /// leaving them to the Selection tab's body left the saved step and
    /// axes unread until that tab was opened once.
    fn stamp_stage_prefs(&mut self) {
        self.stage.show = self.insp.prefs.display;
        self.stage.nudge_step = self.insp.prefs.selection.nudge_step();
        self.stage.nudge_camera_relative = self.insp.prefs.selection.camera_axes;
    }

    /// The docked / popped / folded Inspector. Copies the display toggles
    /// onto the stage first (so a folded panel still applies them), then the
    /// hotkeys, the panel, the width settle and the prefs flush.
    pub(crate) fn inspector_panel(&mut self, ctx: &egui::Context) {
        self.stamp_stage_prefs();

        // Alt+1…5 jump (and unfold), Alt+I folds — only with nothing focused.
        let free = ctx.memory(|m| m.focused().is_none());
        let alt = ctx.input(|i| {
            i.modifiers.alt && !i.modifiers.command && !i.modifiers.ctrl && !i.modifiers.shift
        });
        if free && alt {
            let keys = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5];
            for (n, key) in keys.into_iter().enumerate() {
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, key)) {
                    self.insp.set_tab(InspectorTab::from_index(n));
                    self.collapsed.remove("inspector");
                }
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::I)) {
                if !self.collapsed.remove("inspector") {
                    self.collapsed.insert("inspector");
                }
            }
        }

        let stored = self.insp.prefs.width;
        let width = self.dockable_side(
            ctx,
            "inspector",
            "Inspector",
            egui::panel::Side::Right,
            230.0,
            stored.unwrap_or(260.0),
            |app, ui| app.inspector_body(ui),
        );
        // Remember the width once a drag of the panel's handle has settled.
        // egui also widens a panel whose content overflows it; that is not
        // a preference, so it is never stored.
        let resize_id = egui::Id::new("inspector").with("__resize");
        if ctx.is_being_dragged(resize_id) {
            self.insp.resizing = true;
        } else if self.insp.resizing {
            self.insp.resizing = false;
            if let Some(w) = width {
                if stored.map_or(true, |s| (s - w).abs() > 1.0) {
                    self.insp.prefs.width = Some(w);
                    self.insp.mark_dirty();
                }
            }
        }
        self.insp.flush();
    }

    /// Follow-selection, the tab strip, the context strip and the active
    /// tab's body in its own scroll area.
    fn inspector_body(&mut self, ui: &mut egui::Ui) {
        let key: FocusKey = (
            self.stage.selection.len(),
            self.stage.sel_tower,
            self.stage.sel_truss,
            self.stage.sel_stage,
            self.stage.last_selected,
        );
        if self.insp.prefs.follow_selection
            && Some(key) != self.insp.last_focus
            && (key.0 > 0 || key.1.is_some() || key.2.is_some() || key.3)
        {
            self.insp.set_tab(InspectorTab::Selection);
        }
        self.insp.last_focus = Some(key);

        // Everything the strip shows, computed before any closure.
        let selected = self.stage.selected_fixtures();
        let names: Vec<&str> = selected
            .iter()
            .take(6)
            .filter_map(|&fi| self.patch.fixtures.get(fi).map(|f| f.display.as_str()))
            .collect();
        let hints: Vec<String> = InspectorTab::ALL
            .iter()
            .map(|t| {
                if *t == InspectorTab::Selection && !names.is_empty() {
                    let more = selected.len().saturating_sub(names.len());
                    let list = if more > 0 {
                        format!("{} +{more} more", names.join(", "))
                    } else {
                        names.join(", ")
                    };
                    format!("{}\nSelected: {list}", t.hint())
                } else {
                    t.hint().to_owned()
                }
            })
            .collect();
        let badges: Vec<Option<(String, Color32)>> =
            InspectorTab::ALL.iter().map(|t| t.badge(self)).collect();
        let segs: Vec<theme::Segment> = InspectorTab::ALL
            .iter()
            .enumerate()
            .map(|(i, t)| theme::Segment {
                icon: Some(t.icon()),
                label: t.label(),
                hint: &hints[i],
                badge: badges[i].clone(),
            })
            .collect();
        if let Some(i) = theme::segmented(ui, "insp_tabs", &segs, self.insp.prefs.tab.index()) {
            self.insp.set_tab(InspectorTab::from_index(i));
        }
        ui.add_space(6.0);
        self.context_strip(ui);
        ui.add_space(6.0);

        let tab = self.insp.prefs.tab;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt(("insp_scroll", tab.key()))
            .show(ui, |ui| match tab {
                InspectorTab::Presets => self.inspector_presets_tab(ui),
                InspectorTab::Selection => self.inspector_selection_tab(ui),
                InspectorTab::Stage => self.inspector_stage_tab(ui),
                InspectorTab::Build => self.inspector_build_tab(ui),
                InspectorTab::Patch => self.inspector_patch_tab(ui),
            });
    }

    /// The always-visible summary of what is picked: swatch or element
    /// icon, two lines of text, BLACKOUT / FROZEN pills, a clear button and
    /// a jump arrow. Click → Selection (Patch when nothing is patched);
    /// right-click for the select verbs.
    fn context_strip(&mut self, ui: &mut egui::Ui) {
        let z = theme::zoom_of(ui);
        let focus = focus_of(&self.stage, &self.patch);
        let (title, detail) = focus_text(&focus, &self.patch, &self.settings);
        let nothing_patched = self.patch.fixtures.is_empty();
        let has_focus = !matches!(focus, Focus::Nothing);
        let on_selection_tab = self.insp.prefs.tab == InspectorTab::Selection;
        let (blackout, frozen) = (self.blackout, self.frozen);
        let first_light = match &focus {
            Focus::Lights { first, .. } => Some(*first),
            _ => None,
        };
        // The live colour of the first selected light, and its peek line.
        let (swatch, peek) = match first_light.and_then(|fi| self.patch.fixtures.get(fi)) {
            Some(f) => {
                let buf = *self.net.dmx.lock();
                let c = fixture_swatch(f, &buf);
                let level = fixture_level(f, &buf);
                (
                    Some(c),
                    Some(format!("RGB {} {} {} · {:.0} %", c.r(), c.g(), c.b(), level * 100.0)),
                )
            }
            None => (None, None),
        };
        let (icon, icon_color) = match &focus {
            Focus::Nothing => (Icon::Selection, theme::TEXT_DIM.gamma_multiply(0.6)),
            Focus::Lights { .. } => (Icon::Light, theme::TEXT_DIM),
            Focus::Tower { .. } => (Icon::Tower, theme::TEXT_DIM),
            Focus::Truss { kind: TrussKind::Straight, .. } => (Icon::TrussStraight, theme::TEXT_DIM),
            Focus::Truss { kind: TrussKind::Radius, .. } => (Icon::TrussRadius, theme::TEXT_DIM),
            Focus::Stage => (Icon::Stage, theme::TEXT_DIM),
        };
        let hover_text = if has_focus {
            "Open the Selection tab for what is selected on the stage. Right-click for select-all / same type."
        } else {
            "Nothing is selected — click a light on the stage or use the Fixtures panel."
        };
        let mut follow = self.insp.prefs.follow_selection;
        let mut act: Option<CtxAction> = None;

        // The row is registered first so the buttons placed on it sit on top.
        let h = 30.0 * z;
        let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), h));
        let row = ui.interact(rect, ui.id().with("insp_ctx"), egui::Sense::click());
        let fill = if row.hovered() { theme::WELL.lerp_to_gamma(theme::HOVER, 0.5) } else { theme::WELL };
        ui.painter().rect(rect, egui::Rounding::same(6.0), fill, egui::Stroke::new(1.0, theme::EDGE));
        let inner = rect.shrink2(egui::vec2(8.0, 4.0));

        // Left: the swatch or the element icon.
        let mark = 14.0 * z;
        let mark_rect =
            egui::Rect::from_center_size(egui::Pos2::new(inner.left() + mark * 0.5, inner.center().y), egui::Vec2::splat(mark));
        match swatch {
            Some(c) => theme::paint_swatch(ui.painter(), mark_rect, c, 3.0 * z),
            None => icons::draw(ui.painter(), mark_rect, icon, icon_color),
        }
        if let Some(peek) = peek {
            ui.interact(mark_rect, ui.id().with("insp_ctx_peek"), egui::Sense::hover()).on_hover_text(peek);
        }
        let text_x = mark_rect.right() + 6.0 * z;

        // Right: jump arrow, clear button, status pills — laid right to left.
        let title_font = egui::FontId::new(13.0 * z, theme::medium());
        let title_w = ui.painter().layout_no_wrap(title.clone(), title_font.clone(), theme::TEXT).size().x;
        let buttons_w = if on_selection_tab { 0.0 } else { 16.0 * z }
            + if has_focus { ui.spacing().interact_size.y + 4.0 * z } else { 0.0 };
        let room_for_pills = inner.right() - text_x - title_w - buttons_w >= 60.0;
        let controls = ui.allocate_new_ui(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0 * z;
                if !on_selection_tab {
                    let (jr, _) = ui.allocate_exact_size(egui::Vec2::splat(12.0 * z), egui::Sense::hover());
                    icons::draw(ui.painter(), jr, Icon::Jump, theme::ACCENT_SOFT);
                }
                if has_focus
                    && theme::tool_button(ui, Icon::Close, None, "Clear the selection.", Ok(())).clicked()
                {
                    act = Some(CtxAction::Clear);
                }
                if room_for_pills {
                    if frozen {
                        theme::pill(ui, "FROZEN", theme::ACCENT_SOFT);
                    }
                    if blackout {
                        theme::pill(ui, "BLACKOUT", theme::WARN);
                    }
                }
                ui.min_rect()
            },
        );
        let used = controls.inner;
        let text_right = if used.is_positive() { used.left() - 6.0 * z } else { inner.right() };
        let text_w = (text_right - text_x).max(8.0);

        // Two stacked lines, ellipsis-truncated.
        let p = ui.painter();
        let mut job = egui::text::LayoutJob::simple_singleline(title, title_font, theme::TEXT);
        job.wrap = egui::text::TextWrapping {
            max_width: text_w,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('\u{2026}'),
        };
        let tg = p.layout_job(job);
        let mut job = egui::text::LayoutJob::simple_singleline(
            detail,
            egui::FontId::new(11.0 * z, egui::FontFamily::Proportional),
            theme::TEXT_DIM,
        );
        job.wrap = egui::text::TextWrapping {
            max_width: text_w,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('\u{2026}'),
        };
        let dg = p.layout_job(job);
        let total_h = tg.size().y + dg.size().y;
        let ty = inner.center().y - total_h * 0.5;
        let th = tg.size().y;
        p.galley(egui::Pos2::new(text_x, ty), tg, theme::TEXT);
        p.galley(egui::Pos2::new(text_x, ty + th), dg, theme::TEXT_DIM);

        // Settle the cursor under the row whatever the controls used.
        ui.allocate_rect(rect, egui::Sense::hover());

        let row = row.on_hover_text(hover_text);
        if row.clicked() {
            act = Some(CtxAction::Goto(if nothing_patched {
                InspectorTab::Patch
            } else {
                InspectorTab::Selection
            }));
        }
        row.context_menu(|ui| {
            if ui.button("Select every light").clicked() {
                act = Some(CtxAction::All);
                ui.close_menu();
            }
            let same = ui.add_enabled(first_light.is_some(), egui::Button::new("Select same type"));
            if same.clicked() {
                if let Some(fi) = first_light {
                    act = Some(CtxAction::SameType(fi));
                }
                ui.close_menu();
            }
            if ui.add_enabled(has_focus, egui::Button::new("Clear selection")).clicked() {
                act = Some(CtxAction::Clear);
                ui.close_menu();
            }
            ui.separator();
            ui.checkbox(&mut follow, "Follow selection").on_hover_text(
                "Jump to the Selection tab whenever something new is picked on the stage.",
            );
        });
        if follow != self.insp.prefs.follow_selection {
            self.insp.prefs.follow_selection = follow;
            self.insp.mark_dirty();
        }

        match act {
            Some(CtxAction::All) => {
                if self.stage.select_all_fixtures() {
                    self.sync_selection_units();
                    self.sel_fixture = self.stage.last_selected;
                    self.log.push(format!("Selected all {} lights", self.patch.fixtures.len()));
                }
            }
            Some(CtxAction::SameType(fi)) => {
                if let Some(f) = self.patch.fixtures.get(fi) {
                    let stem = type_stem(&f.display).to_owned();
                    self.stage.select_same_type(&self.patch, fi);
                    self.sync_selection_units();
                    self.sel_fixture = self.stage.last_selected;
                    self.log.push(format!("Selected every {stem}"));
                }
            }
            Some(CtxAction::Clear) => {
                self.stage.clear_selection();
                self.stage.sel_stage = false;
                self.sync_selection_units();
                self.sel_fixture = None;
                self.log.push("Selection cleared".into());
            }
            Some(CtxAction::Goto(t)) => self.insp.set_tab(t),
            None => {}
        }
    }

    /// A collapsible section whose open state persists in inspector.json
    /// under `key` ("<tab>.<section>"). The body gets `&mut App`, so hold no
    /// other borrow of `self` across the call.
    pub(crate) fn insp_fold(
        &mut self,
        ui: &mut egui::Ui,
        key: &'static str,
        title: &str,
        default_open: bool,
        badge: Option<(&str, Color32)>,
        body: impl FnOnce(&mut App, &mut egui::Ui),
    ) {
        let mut open = self.insp.is_open(key, default_open);
        let toggled = theme::fold(ui, ("insp_fold", key), title, badge, &mut open, |ui| body(self, ui));
        if toggled {
            self.insp.set_open(key, default_open, open);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{Frame, DMX_SLOTS};
    use crate::stage::headless::{render_frames, save};
    use crate::stage::{v3, Instance, LightTransform, Tower, Truss};

    fn lit(pixels: &[u8]) -> usize {
        pixels.chunks(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120).count()
    }

    /// Teal pixels (the accent hue: green and blue well above red) inside
    /// `x0..x1 × y0..y1` — the selected segment's lit body and underline.
    fn teal(pixels: &[u8], size: [u32; 2], x: (u32, u32), y: (u32, u32)) -> usize {
        let w = size[0] as usize;
        let mut n = 0;
        for yy in y.0..y.1.min(size[1]) {
            for xx in x.0..x.1.min(size[0]) {
                let i = (yy as usize * w + xx as usize) * 4;
                let (r, g, b) = (pixels[i] as i32, pixels[i + 1] as i32, pixels[i + 2] as i32);
                if g - r >= 14 && b - r >= 14 && g > 40 {
                    n += 1;
                }
            }
        }
        n
    }

    /// A four-channel built-in profile, so "DMX 17–20 · 4 ch" reads true.
    fn par_patch() -> Patch {
        let profile = crate::profiles::PROFILES
            .iter()
            .find(|p| p.channel_count() == 4)
            .expect("a 4-channel built-in profile");
        Patch {
            fixtures: vec![profile.to_fixture("Par 1".into(), 17), profile.to_fixture("Par 2".into(), 21)],
            warnings: Vec::new(),
        }
    }

    /// The arrow-key nudge works over the stage canvas whichever tab is up,
    /// but the step and the camera-relative axes were only copied onto the
    /// stage from inside the Selection tab's body. On any launch that did
    /// not draw that tab — the saved tab is anything else, or the Inspector
    /// comes up folded — the stage quietly used `StageView::new()`'s 0.1 m
    /// world-axis defaults instead of what inspector.json says.
    #[test]
    fn the_nudge_prefs_reach_the_stage_without_the_selection_tab() {
        let mut app = crate::app::App::new();
        app.insp.prefs.tab = InspectorTab::Presets;
        app.insp.prefs.selection.nudge_step_i = crate::stage::NUDGE_STEPS.len() - 1;
        app.insp.prefs.selection.camera_axes = true;
        app.stage.nudge_step = 0.1;
        app.stage.nudge_camera_relative = false;
        app.stamp_stage_prefs();
        assert_eq!(app.stage.nudge_step, app.insp.prefs.selection.nudge_step());
        assert!(app.stage.nudge_camera_relative);
        // And back again, so it tracks rather than latching once.
        app.insp.prefs.selection.nudge_step_i = 0;
        app.insp.prefs.selection.camera_axes = false;
        app.stamp_stage_prefs();
        assert_eq!(app.stage.nudge_step, crate::stage::NUDGE_STEPS[0]);
        assert!(!app.stage.nudge_camera_relative);
    }

    #[test]
    fn tab_metadata_is_unique_and_cycles() {
        let all = InspectorTab::ALL;
        for (i, a) in all.iter().enumerate() {
            assert_eq!(a.index(), i);
            for b in &all[i + 1..] {
                assert_ne!(a.label(), b.label());
                assert_ne!(a.key(), b.key());
                assert!(a.icon() != b.icon());
            }
        }
        assert_eq!(InspectorTab::Patch.next(), InspectorTab::Presets);
        assert_eq!(InspectorTab::Presets.prev(), InspectorTab::Patch);
        assert_eq!(InspectorTab::from_index(7), InspectorTab::Stage);
    }

    #[test]
    fn type_stem_strips_one_numeric_suffix() {
        assert_eq!(type_stem("SlimPAR 3"), "SlimPAR");
        assert_eq!(type_stem("SlimPAR 3 "), "SlimPAR");
        assert_eq!(type_stem("LED Bar"), "LED Bar");
        assert_eq!(type_stem("12"), "12");
        assert_eq!(type_stem("Spot 1 2"), "Spot 1");
        assert_eq!(type_stem(""), "");
    }

    #[test]
    fn focus_text_reads_like_the_operator_expects() {
        let patch = par_patch();
        let set = Settings::default();
        let lights = |instances, fixtures, groups: Vec<(String, usize)>, first| Focus::Lights {
            instances,
            fixtures,
            groups,
            mounted: 0,
            first,
        };
        let (t, d) = focus_text(&lights(3, 3, vec![("SlimPAR".into(), 3)], 0), &patch, &set);
        assert_eq!((t.as_str(), d.as_str()), ("3 lights", "SlimPAR ×3"));
        let (t, _) = focus_text(&lights(5, 3, vec![("SlimPAR".into(), 3)], 0), &patch, &set);
        assert_eq!(t, "3 lights (5 copies)");
        let four = vec![("A".into(), 2), ("B".into(), 1), ("C".into(), 1), ("D".into(), 1)];
        let (_, d) = focus_text(&lights(5, 5, four, 0), &patch, &set);
        assert!(d.ends_with(" +2 more"), "{d}");
        let (t, d) = focus_text(&lights(1, 1, vec![("Par".into(), 1)], 0), &patch, &set);
        assert_eq!((t.as_str(), d.as_str()), ("Par 1", "DMX 17–20 · 4 ch"));
        let truss = Focus::Truss {
            index: 1,
            kind: TrussKind::Straight,
            mounted: 4,
            slots: 24,
            name: "F34 truss 2".into(),
        };
        let (t, d) = focus_text(&truss, &patch, &set);
        assert_eq!(t, "F34 truss 2");
        assert!(d.starts_with("4 / 24 slots"), "{d}");
        let radius = Focus::Truss {
            index: 0,
            kind: TrussKind::Radius,
            mounted: 2,
            slots: 12,
            name: "Radius truss 1".into(),
        };
        let (t, _) = focus_text(&radius, &patch, &set);
        assert!(t.starts_with("Radius truss"));
        let tower = Focus::Tower { index: 0, mounted: 2, name: "Tower 1".into() };
        let (t, d) = focus_text(&tower, &patch, &set);
        assert_eq!((t.as_str(), d.as_str()), ("Tower 1", "2 / 8 slots"));
        let (t, _) = focus_text(&Focus::Nothing, &patch, &set);
        assert_eq!(t, "Nothing selected");
        let empty = Patch { fixtures: Vec::new(), warnings: Vec::new() };
        let (t, _) = focus_text(&Focus::Nothing, &empty, &set);
        assert_eq!(t, "Nothing patched");
        let (t, d) = focus_text(&Focus::Stage, &patch, &set);
        assert_eq!((t.as_str(), d.as_str()), ("Stage box", "8.0 × 6.0 m · 1.0 m high"));
    }

    #[test]
    fn focus_of_prefers_lights_then_tower_then_truss_then_stage() {
        let patch = par_patch();
        let mut stage = StageView::new();
        stage.layout_path = "does-not-exist.json".into();
        stage.sync(&patch, &Settings::default());
        assert_eq!(stage.instances.len(), 2);
        stage.select_fixture(0, false);
        stage.select_fixture(1, true);
        match focus_of(&stage, &patch) {
            Focus::Lights { fixtures, instances, groups, mounted, .. } => {
                assert_eq!((fixtures, instances, mounted), (2, 2, 0));
                assert_eq!(groups, vec![("Par".to_string(), 2)]);
            }
            _ => panic!("lights expected"),
        }
        // A second copy of fixture 0, hung on a truss slot.
        let copy = Instance {
            fixture: 0,
            t: LightTransform { pos: v3(1.0, 3.0, -4.0), yaw_deg: 0.0, pitch_deg: -90.0, roll_deg: 0.0, scale: 1.0 },
            opacity: 1.0,
            mount: None,
            truss_mount: Some((0, 3)),
        };
        stage.instances.push(copy);
        stage.selection.insert(2);
        match focus_of(&stage, &patch) {
            Focus::Lights { fixtures, instances, mounted, .. } => {
                assert_eq!((fixtures, instances, mounted), (2, 3, 1));
            }
            _ => panic!("lights expected"),
        }
        stage.clear_selection();
        stage.towers.push(Tower::default());
        stage.trusses.push(Truss::straight());
        stage.sel_tower = Some(0);
        stage.sel_truss = Some(0);
        assert!(matches!(focus_of(&stage, &patch), Focus::Tower { mounted: 0, .. }));
        stage.sel_tower = None;
        match focus_of(&stage, &patch) {
            Focus::Truss { slots, mounted, name, .. } => {
                assert_eq!((slots, mounted), (24, 1));
                assert_eq!(name, "F34 truss 1");
            }
            _ => panic!("truss expected"),
        }
        stage.sel_truss = None;
        stage.sel_stage = true;
        assert!(matches!(focus_of(&stage, &patch), Focus::Stage));
        stage.sel_stage = false;
        assert!(matches!(focus_of(&stage, &patch), Focus::Nothing));
        stage.sel_truss = Some(9);
        assert!(matches!(focus_of(&stage, &patch), Focus::Nothing));
        stage.sel_truss = None;
        stage.selection.insert(99);
        assert!(matches!(focus_of(&stage, &patch), Focus::Nothing));
    }

    #[test]
    fn prefs_round_trip_and_defaults() {
        let d: InspectorPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(d, InspectorPrefs::default());
        assert_eq!(d.tab, InspectorTab::Presets);
        assert!(d.display.grid && d.display.labels && d.display.help);
        assert!(d.folded.is_empty());
        assert_eq!(d.width, None);
        let mut p = InspectorPrefs::default();
        p.tab = InspectorTab::Build;
        p.display.grid = false;
        p.folded.insert("stage.controls".into());
        p.width = Some(300.0);
        let back: InspectorPrefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        let partial: InspectorPrefs =
            serde_json::from_str(r#"{"tab":"Stage","display":{"beams":false},"bogus":1}"#).unwrap();
        assert_eq!(partial.tab, InspectorTab::Stage);
        assert!(!partial.display.beams);
        assert!(partial.display.labels);
    }

    #[test]
    fn fold_state_defaults_and_overrides() {
        let mut ui = InspectorUi::default();
        assert!(ui.is_open("x", true));
        ui.set_open("x", true, false);
        assert!(!ui.is_open("x", true));
        assert!(ui.dirty);
        assert!(ui.prefs.folded.contains("x"));
        ui.set_open("x", true, true);
        assert!(!ui.prefs.folded.contains("x"));
        let mut fresh = InspectorUi::default();
        fresh.set_open("y", false, false);
        assert!(fresh.prefs.folded.is_empty());
        assert!(!fresh.dirty);
    }

    #[test]
    fn set_tab_marks_dirty_only_on_change() {
        let mut ui = InspectorUi::default();
        ui.set_tab(InspectorTab::Presets);
        assert!(!ui.dirty);
        ui.set_tab(InspectorTab::Build);
        assert!(ui.dirty);
        assert_eq!(ui.prefs.tab, InspectorTab::Build);
    }

    /// Every tab at the panel's 230 px minimum, a zoom-2 scene and a
    /// truss-focus scene, written to `target/inspector_*.png`. Read-only on
    /// the working directory: prefs are poked directly (never `set_tab`),
    /// so `dirty` stays false and `flush` writes nothing.
    #[test]
    fn inspector_tabs_render_headless() {
        let mut app = crate::app::App::new();
        *app.net.dmx.lock() = Frame([170u8; DMX_SLOTS]);
        app.collapsed.insert("channels");
        app.collapsed.remove("inspector");
        app.collapsed.remove("fixtures");
        app.show_log = false;
        app.show_osc = false;
        app.insp.prefs.width = Some(230.0);
        for i in 0..3.min(app.patch.fixtures.len()) {
            app.stage.select_fixture(i, true);
        }
        let size = [1000, 760];
        for tab in InspectorTab::ALL {
            app.insp.prefs.tab = tab;
            let Some(pixels) = render_frames(5, size, |ctx, frame| {
                if frame == 0 {
                    crate::ui::install_theme(ctx);
                } else {
                    app.draw_ui(ctx);
                }
            }) else {
                eprintln!("no GPU adapter — skipping");
                return;
            };
            save(&pixels, size, &format!("inspector_{}", tab.key()));
            assert!(lit(&pixels) > 1000, "{} came out black", tab.key());
            // The strip band (the legacy stubs widen the panel past 230 px
            // until their owners restyle them, so sample generously) must
            // carry the selected segment's teal body.
            let glow = teal(&pixels, size, (640, 1000), (85, 150));
            assert!(glow >= 200, "{}: only {glow} teal pixels in the strip band", tab.key());
        }
        // Zoom 2.0 on the Selection tab: icon-only strip, wrapped toolbars.
        app.zoom.inspector = 2.0;
        app.insp.prefs.tab = InspectorTab::Selection;
        let Some(pixels) = render_frames(5, size, |ctx, frame| {
            if frame == 0 {
                crate::ui::install_theme(ctx);
            } else {
                app.draw_ui(ctx);
            }
        }) else {
            return;
        };
        save(&pixels, size, "inspector_zoom2");
        assert!(lit(&pixels) > 1000, "zoom2 came out black");
        // A truss in focus (pushed in memory, never saved), zoom 1.
        app.zoom.inspector = 1.0;
        app.stage.clear_selection();
        app.stage.trusses.push(Truss::straight());
        app.stage.sel_truss = Some(app.stage.trusses.len() - 1);
        app.insp.prefs.tab = InspectorTab::Presets;
        let Some(pixels) = render_frames(5, size, |ctx, frame| {
            if frame == 0 {
                crate::ui::install_theme(ctx);
            } else {
                app.draw_ui(ctx);
            }
        }) else {
            return;
        };
        save(&pixels, size, "inspector_truss");
        assert!(lit(&pixels) > 1000, "truss scene came out black");
        assert!(!app.insp.dirty, "the headless scenes must not mark prefs dirty");
    }
}
