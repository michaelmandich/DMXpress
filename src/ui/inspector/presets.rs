//! Inspector · Presets tab (area D).
//!
//! Store the programmer as a look, then get it back with one press. A
//! preset carries a stable id, so every reference to it — a board pad, a
//! deck key, an audio trigger, the chase source — survives a rename, a
//! reorder or a delete.
//!
//! What the tab shows, top to bottom: a toolbar (search, grid/list, sort,
//! the board), status pills, the store card, the recall strip (cut · fade ·
//! as set), the pinned strip, the presets themselves as pads or rows —
//! grouped into folders you can drag onto — the edit-pad card, the recent
//! strip, the ShowBuddy banks and the output row.
//!
//! Every pad is colour-coded from its own values: the mixed emitter colour
//! for a colour look, amber for a dimmer-only one, deep blue for a position
//! and plain raised grey for anything else, unless a colour was picked by
//! hand. The same colour reaches the Presets board and the Stream Deck.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Vec2};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::audio::TriggerSource;
use crate::chase::ChaseSource;
use crate::net::{self, Frame};
use crate::oscillator::Look;
use crate::preset::{self, SavedOsc, UserPreset};
use crate::showbuddy::{Patch, Role};
use crate::streamdeck::{preset_key_look, KeyIcon};
use crate::transition::TransitionRun;
use crate::ui::board::pad_symbol;
use crate::ui::{icons, role_color, theme};

use super::Icon;

/// Presets-tab preferences, persisted at `InspectorPrefs.presets`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PresetsPrefs {
    /// Pads or rows.
    pub view: PresetView,
    /// The order presets are listed in.
    pub sort: PresetSort,
    /// How big a pad is drawn.
    pub pad: PadSize,
    /// What a plain click does about fading.
    pub recall: RecallMode,
    /// Seconds used by `RecallMode::Fade`.
    pub fade_secs: f32,
}

impl Default for PresetsPrefs {
    fn default() -> Self {
        Self {
            view: PresetView::Grid,
            sort: PresetSort::Manual,
            pad: PadSize::M,
            recall: RecallMode::AsSet,
            fade_secs: 2.0,
        }
    }
}

/// Pads or rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) enum PresetView {
    #[default]
    Grid,
    List,
}

/// The order the pool is listed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) enum PresetSort {
    /// Pool order — the only one you can drag to reorder in.
    #[default]
    Manual,
    Name,
    Hue,
    Newest,
}

impl PresetSort {
    pub(crate) const ALL: [PresetSort; 4] =
        [PresetSort::Manual, PresetSort::Name, PresetSort::Hue, PresetSort::Newest];

    pub(crate) fn label(self) -> &'static str {
        match self {
            PresetSort::Manual => "Manual",
            PresetSort::Name => "Name",
            PresetSort::Hue => "Hue",
            PresetSort::Newest => "Newest",
        }
    }
}

/// How wide a pad is drawn, which decides how many fit across.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) enum PadSize {
    S,
    #[default]
    M,
    L,
}

impl PadSize {
    pub(crate) const ALL: [PadSize; 3] = [PadSize::S, PadSize::M, PadSize::L];

    /// The narrowest a pad of this size may be drawn.
    pub(crate) fn min_w(self) -> f32 {
        match self {
            PadSize::S => 44.0,
            PadSize::M => 60.0,
            PadSize::L => 84.0,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            PadSize::S => "Small pads",
            PadSize::M => "Medium pads",
            PadSize::L => "Large pads",
        }
    }
}

/// What a plain click does about fading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) enum RecallMode {
    /// Whatever the Transition window is set to.
    #[default]
    AsSet,
    Cut,
    Fade,
}

/// Dragging a preset out of the pool (by id, never by index).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct PresetDrag(pub u32);
/// Dragging one board pad onto another (by slot index).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct PadDrag(pub usize);
/// Dragging a folder header to reorder the folders.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct FolderDrag(pub String);

/// What kind of look a preset's values add up to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwatchKind {
    Colour,
    Dimmer,
    Position,
    Other,
    Empty,
}

/// A preset's representative colour and what it was derived from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Swatch {
    pub rgb: [u8; 3],
    pub kind: SwatchKind,
}

/// What one preset holds, for its badges and the search terms.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PresetContents {
    pub colour: bool,
    pub dimmer: bool,
    pub position: bool,
    pub beam: bool,
    pub oscs: usize,
    pub fixtures: usize,
}

/// What a board pad points at while it is being edited.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) enum PadTarget {
    #[default]
    None,
    /// A native preset by id.
    User(u32),
    /// A ShowBuddy preset by (bank name, preset name).
    Bank(String, String),
}

/// The Presets board window's own state: arrange mode and the slot editor.
#[derive(Default)]
pub(crate) struct PresetBoardUi {
    pub arrange: bool,
    pub slot: Option<usize>,
    pub target: PadTarget,
    pub color: [u8; 3],
    pub label: String,
    pub icon: KeyIcon,
}

/// The inline "Edit pad" card, open on one preset at a time.
#[derive(Default)]
pub(crate) struct PresetEdit {
    pub id: u32,
    pub name: String,
    pub auto_color: bool,
    pub color: [u8; 3],
    pub symbol: String,
    pub icon: KeyIcon,
    pub own_fade: bool,
    pub fade_secs: f32,
    pub pinned: bool,
    pub focus_pending: bool,
}

/// Transient Presets-tab state: the search box, the caches, the editors.
#[derive(Default)]
pub(crate) struct PresetsUi {
    pub search: String,
    /// Folder the next stored preset is filed into; "" = top level.
    pub store_into: String,
    /// `Some` while the inline new-folder row is open.
    pub new_folder: Option<String>,
    pub edit: Option<PresetEdit>,
    /// (folder being renamed, the text typed so far).
    pub rename_folder: Option<(String, String)>,
    pub rename_folder_focus: bool,
    pub swatches: HashMap<u32, Swatch>,
    pub contents: HashMap<u32, PresetContents>,
    pub bank_swatches: HashMap<(usize, usize), [u8; 3]>,
    /// (patched fixtures, first address, banks) — the caches re-derive when
    /// the rig changes shape under them.
    pub cache_key: (usize, u16, usize),
    /// Ids recalled this session, most recent first.
    pub recent: VecDeque<u32>,
    /// A just-stored preset, scrolled into view for one frame.
    pub reveal: Option<u32>,
    pub board: PresetBoardUi,
}

/// One thing a click asked for, applied after every loop has finished.
enum PresetAction {
    Recall { idx: usize, fade: Option<f32> },
    RecallMasked(usize),
    Edit(u32),
    Update(usize),
    Pin(usize),
    /// (preset id being dragged, pool index to land in front of).
    MoveBefore(u32, usize),
    ToBoard(usize),
    ChaseSource(usize),
    Duplicate(usize),
    Delete(usize),
    /// (preset id, folder name; empty = top level).
    File(u32, String),
    ToggleFolder(String),
    /// Open the inline rename field on a folder. Deferred like every other
    /// action: `presets_pool` restores its own `rename_folder` local after
    /// the folder loop, so a write through `self` from inside the loop is
    /// clobbered the same frame.
    RenameFolderStart(String),
    RenameFolder(String, String),
    DeleteFolder(usize),
    MoveFolder(usize, usize),
    StoreInto(String),
    FillPage(String),
    RecallBank(usize, usize, Option<f32>),
    ImportBank(usize, usize),
    ImportWholeBank(usize),
    BankToBoard(usize, usize),
    BankChaseSource(usize, usize),
}

// ---- pure helpers ----

/// The fixture and channel role at a 0-based DMX index — the `role_at`
/// scan, hoisted out of `App` so the caches can be built from a borrow.
pub(crate) fn role_at_in(patch: &Patch, addr0: usize) -> Option<(usize, Role)> {
    for (fi, f) in patch.fixtures.iter().enumerate() {
        let from0 = f.from as usize - 1;
        if addr0 >= from0 && addr0 < from0 + f.channel_count() {
            return f.channels.get(addr0 - from0).map(|c| (fi, c.role()));
        }
    }
    None
}

/// Additive emitters, as the tint each one contributes at full.
fn tint_of(role: Role) -> Option<[f32; 3]> {
    match role {
        Role::Amber => Some([255.0, 180.0, 60.0]),
        Role::Uv => Some([120.0, 40.0, 255.0]),
        Role::Cyan => Some([0.0, 220.0, 255.0]),
        Role::Magenta => Some([255.0, 0.0, 200.0]),
        Role::Yellow => Some([255.0, 230.0, 0.0]),
        _ => None,
    }
}

fn rgb_of(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

/// The colour a preset's values add up to: RGBW mixed like a palette
/// swatch, the additive emitters added on top, the whole thing scaled by
/// its dimmer (never below 35 %, so a preset at 5 % still reads).
pub(crate) fn swatch_from_values(
    values: &[(usize, u8)],
    role_at: &dyn Fn(usize) -> Option<Role>,
) -> Swatch {
    let (mut r, mut g, mut b, mut w) = (0u8, 0u8, 0u8, 0u8);
    let mut tint = [0.0f32; 3];
    let mut coloured = false;
    let mut dimmer: Option<u8> = None;
    let mut position = false;
    let mut other = false;
    for &(a, v) in values {
        let Some(role) = role_at(a) else { continue };
        match role {
            Role::Red => {
                r = r.max(v);
                coloured = true;
            }
            Role::Green => {
                g = g.max(v);
                coloured = true;
            }
            Role::Blue => {
                b = b.max(v);
                coloured = true;
            }
            Role::White => {
                w = w.max(v);
                coloured = true;
            }
            Role::Dimmer => dimmer = Some(dimmer.unwrap_or(0).max(v)),
            Role::Pan | Role::PanFine | Role::Tilt | Role::TiltFine => position = true,
            _ => match tint_of(role) {
                Some(t) => {
                    let k = v as f32 / 255.0;
                    for (acc, c) in tint.iter_mut().zip(t) {
                        *acc += c * k;
                    }
                    coloured = true;
                }
                None => other = true,
            },
        }
    }
    let kind = if coloured {
        SwatchKind::Colour
    } else if dimmer.is_some() {
        SwatchKind::Dimmer
    } else if position {
        SwatchKind::Position
    } else if other {
        SwatchKind::Other
    } else {
        SwatchKind::Empty
    };
    let level = dimmer.map(|d| (d as f32 / 255.0).max(0.35));
    let rgb = match kind {
        SwatchKind::Colour => {
            let mut c = [r, g, b].map(|c| (c as u16 + w as u16).min(255) as f32);
            for (v, t) in c.iter_mut().zip(tint) {
                *v = (*v + t).min(255.0) * level.unwrap_or(1.0);
            }
            c.map(|v| v.round() as u8)
        }
        SwatchKind::Dimmer => rgb_of(role_color(Role::Dimmer))
            .map(|c| (c as f32 * level.unwrap_or(1.0)).round() as u8),
        SwatchKind::Position => [40, 80, 110],
        SwatchKind::Other | SwatchKind::Empty => rgb_of(theme::RAISED),
    };
    Swatch { rgb, kind }
}

/// What a preset holds: the roles its values touch, plus the effects the
/// record itself carries.
pub(crate) fn contents_from_values(
    values: &[(usize, u8)],
    role_at: &dyn Fn(usize) -> Option<(usize, Role)>,
    p: &UserPreset,
) -> PresetContents {
    let mut c = PresetContents {
        oscs: p.oscs.len(),
        ..PresetContents::default()
    };
    let mut fixtures: HashSet<usize> = HashSet::new();
    for &(a, _) in values {
        let Some((fi, role)) = role_at(a) else { continue };
        fixtures.insert(fi);
        match role {
            Role::Red
            | Role::Green
            | Role::Blue
            | Role::White
            | Role::Amber
            | Role::Uv
            | Role::Cyan
            | Role::Magenta
            | Role::Yellow => c.colour = true,
            Role::Dimmer => c.dimmer = true,
            Role::Pan | Role::PanFine | Role::Tilt | Role::TiltFine => c.position = true,
            Role::Zoom
            | Role::Focus
            | Role::Iris
            | Role::Gobo
            | Role::Prism
            | Role::Frost
            | Role::Color => c.beam = true,
            _ => {}
        }
    }
    c.fixtures = fixtures.len();
    c
}

/// A colour's hue in degrees; `None` for greys, which sort last.
pub(crate) fn hue_of(rgb: [u8; 3]) -> Option<f32> {
    let [r, g, b] = rgb.map(|v| v as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    if d < 0.06 {
        return None;
    }
    let h = if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    Some(h.rem_euclid(360.0))
}

/// Whether a preset answers the search box: every term has to be part of
/// its name, folder or symbol, or name one of the badges it carries.
pub(crate) fn preset_matches(
    query: &str,
    name: &str,
    folder: &str,
    symbol: &str,
    c: &PresetContents,
    pinned: bool,
    own_fade: bool,
) -> bool {
    let hay = format!("{name} {folder} {symbol}").to_lowercase();
    let mut words: Vec<&str> = Vec::new();
    if c.colour {
        words.push("colour");
        words.push("color");
    }
    if c.dimmer {
        words.push("dimmer");
    }
    if c.position {
        words.push("position");
    }
    if c.beam {
        words.push("beam");
    }
    if c.oscs > 0 {
        words.push("wave");
    }
    if own_fade {
        words.push("fade");
    }
    if pinned {
        words.push("pin");
    }
    query.split_whitespace().all(|term| {
        let term = term.to_lowercase();
        hay.contains(&term) || words.iter().any(|w| *w == term)
    })
}

/// Pool indices in display order: the search applied, then the sort.
pub(crate) fn filter_sort(
    presets: &[UserPreset],
    contents: &HashMap<u32, PresetContents>,
    faces: &dyn Fn(u32) -> [u8; 3],
    search: &str,
    sort: PresetSort,
) -> Vec<usize> {
    let blank = PresetContents::default();
    let mut out: Vec<usize> = presets
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            let c = contents.get(&p.id).unwrap_or(&blank);
            preset_matches(search, &p.name, &p.folder, &p.symbol, c, p.pinned, p.fade.is_some())
        })
        .map(|(i, _)| i)
        .collect();
    match sort {
        PresetSort::Manual => {}
        PresetSort::Name => out.sort_by_key(|&i| presets[i].name.to_lowercase()),
        PresetSort::Newest => out.sort_by_key(|&i| std::cmp::Reverse(presets[i].id)),
        PresetSort::Hue => out.sort_by(|&a, &b| {
            match (hue_of(faces(presets[a].id)), hue_of(faces(presets[b].id))) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        }),
    }
    out
}

/// The fade a recall actually uses: an explicit one (Shift-click), else the
/// preset's own, else what the recall strip says.
pub(crate) fn effective_fade(
    explicit: Option<f32>,
    own: Option<f32>,
    mode: RecallMode,
    secs: f32,
) -> Option<f32> {
    explicit.or(own).or(match mode {
        RecallMode::AsSet => None,
        RecallMode::Cut => Some(0.0),
        RecallMode::Fade => Some(secs),
    })
}

/// Old pool index → new index after removing `i`.
pub(crate) fn delete_map(len: usize, i: usize) -> Vec<Option<usize>> {
    (0..len)
        .map(|a| match a.cmp(&i) {
            Ordering::Less => Some(a),
            Ordering::Equal => None,
            Ordering::Greater => Some(a - 1),
        })
        .collect()
}

/// Old pool index → new index after inserting one entry at `at`.
pub(crate) fn insert_map(len: usize, at: usize) -> Vec<Option<usize>> {
    (0..len).map(|a| Some(if a >= at { a + 1 } else { a })).collect()
}

/// Old pool index → new index after moving `src` in front of `dst`.
pub(crate) fn move_map(len: usize, src: usize, dst: usize) -> Vec<Option<usize>> {
    let dst2 = if src < dst { dst - 1 } else { dst };
    (0..len)
        .map(|a| {
            Some(if a == src {
                dst2
            } else {
                let a2 = if a > src { a - 1 } else { a };
                if a2 >= dst2 {
                    a2 + 1
                } else {
                    a2
                }
            })
        })
        .collect()
}

/// A stored look turned into a native preset: its non-zero base values and
/// the oscillators that were actually running.
pub(crate) fn preset_from_look(look: &Look, name: String, folder: String, id: u32) -> UserPreset {
    let values: Vec<(usize, u8)> = look
        .base
        .iter()
        .enumerate()
        .filter(|&(_, &v)| v != 0)
        .map(|(a, &v)| (a, v))
        .collect();
    let oscs: Vec<(usize, SavedOsc)> = look
        .oscs
        .iter()
        .filter(|(_, o)| o.enabled)
        .map(|(&a, o)| {
            (
                a,
                SavedOsc {
                    invert: o.invert,
                    amount: o.amount,
                    phase: o.phase,
                    subdiv: o.subdiv,
                    shape: o.shape,
                    custom_wave: o.custom_wave.clone(),
                    master_beat: o.master_beat,
                    local_beats: o.local_beats,
                    local_tempo: o.local_tempo,
                },
            )
        })
        .collect();
    UserPreset {
        id,
        name,
        folder,
        values,
        oscs,
        speed: look.speed,
        tempo: look.tempo,
        master_speed: look.master_speed,
        color: None,
        symbol: String::new(),
        icon: KeyIcon::None,
        fade: None,
        pinned: false,
    }
}

/// The tint a folder header wears: the mean of its members' faces.
pub(crate) fn folder_tint(faces: &[[u8; 3]]) -> Color32 {
    if faces.is_empty() {
        return theme::TEXT_DIM;
    }
    let mut sum = [0u32; 3];
    for f in faces {
        for (s, c) in sum.iter_mut().zip(f) {
            *s += *c as u32;
        }
    }
    let n = faces.len() as u32;
    Color32::from_rgb((sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8)
}

/// How wide a text field may ask to be when `buttons` icon buttons share
/// its row: what is left after them and the field's own margin. Asking for
/// more than this widens the whole Inspector, one frame at a time.
fn field_width(ui: &egui::Ui, buttons: usize, margin_x: f32) -> f32 {
    let btn = theme::tool_size(ui, None).x + ui.spacing().item_spacing.x;
    (ui.available_width() - buttons as f32 * btn - margin_x).max(40.0)
}

/// A deck artwork's name, short enough for a 100 px value column (the full
/// names live in the menu the button opens).
fn icon_short(icon: KeyIcon) -> &'static str {
    match icon {
        KeyIcon::None => "None",
        KeyIcon::Bullseye => "Bullseye",
        KeyIcon::StageLight => "Light",
        KeyIcon::LeftRight => "Sideways",
        KeyIcon::UpDown => "Up/down",
        KeyIcon::Wave => "Wave",
        KeyIcon::Burst => "Burst",
        KeyIcon::Ring => "Ring",
        KeyIcon::Rainbow => "Rainbow",
    }
}

/// The four-character face a preset shows when none was typed.
pub(crate) fn symbol_of(p: &UserPreset) -> String {
    if p.symbol.is_empty() {
        pad_symbol(&p.name)
    } else {
        p.symbol.clone()
    }
}

impl App {
    /// Re-derive the swatch and contents caches: cleared whenever the rig
    /// changes shape under them, then filled for anything still missing.
    /// Reads `patch` / `user_presets` / `banks` while writing `insp.presets`
    /// — distinct fields, so one pass does both.
    pub(crate) fn refresh_preset_caches(&mut self) {
        let key = (
            self.patch.fixtures.len(),
            self.patch.fixtures.first().map_or(0, |f| f.from),
            self.banks.len(),
        );
        let patch = &self.patch;
        let presets = &self.user_presets;
        let banks = &self.banks;
        let cache = &mut self.insp.presets;
        if cache.cache_key != key {
            cache.swatches.clear();
            cache.contents.clear();
            cache.bank_swatches.clear();
            cache.cache_key = key;
        }
        let role = |a: usize| role_at_in(patch, a);
        let role_only = |a: usize| role(a).map(|(_, r)| r);
        for p in presets {
            cache.swatches.entry(p.id).or_insert_with(|| swatch_from_values(&p.values, &role_only));
            cache
                .contents
                .entry(p.id)
                .or_insert_with(|| contents_from_values(&p.values, &role, p));
        }
        for (bi, bank) in banks.iter().enumerate() {
            for (pi, p) in bank.presets.iter().enumerate() {
                cache.bank_swatches.entry((bi, pi)).or_insert_with(|| match &p.data {
                    Some(d) => {
                        let values: Vec<(usize, u8)> =
                            d.values.iter().map(|&(a, v)| (a as usize - 1, v)).collect();
                        let s = swatch_from_values(&values, &role_only);
                        match s.kind {
                            SwatchKind::Other | SwatchKind::Empty => preset_key_look(&p.name).0,
                            _ => s.rgb,
                        }
                    }
                    None => preset_key_look(&p.name).0,
                });
            }
        }
    }

    /// The colour a preset wears: the hand-picked one, else the cached
    /// swatch, else a plain raised pad.
    pub(crate) fn preset_face(&self, p: &UserPreset) -> [u8; 3] {
        p.color
            .or_else(|| self.insp.presets.swatches.get(&p.id).map(|s| s.rgb))
            .unwrap_or_else(|| rgb_of(theme::RAISED))
    }

    /// The colour a ShowBuddy bank preset wears.
    pub(crate) fn bank_face(&self, bi: usize, pi: usize) -> [u8; 3] {
        match self.insp.presets.bank_swatches.get(&(bi, pi)) {
            Some(c) => *c,
            None => self
                .banks
                .get(bi)
                .and_then(|b| b.presets.get(pi))
                .map_or([150, 150, 160], |p| preset_key_look(&p.name).0),
        }
    }

    /// Where preset `id` sits in the pool right now.
    pub(crate) fn preset_index_by_id(&self, id: u32) -> Option<usize> {
        self.user_presets.iter().position(|p| p.id == id)
    }

    /// Follow every index-based reference through a pool edit: `map[old]`
    /// is the new index, `None` when that preset is gone.
    pub(crate) fn remap_preset_refs(&mut self, map: &[Option<usize>]) {
        let mut lost_triggers: Vec<String> = Vec::new();
        let mut audio_changed = false;
        for t in &mut self.audio_triggers {
            if let Some(TriggerSource::Preset(old)) = t.source {
                let new = map.get(old).copied().flatten();
                if new.is_none() {
                    lost_triggers.push(t.name.clone());
                }
                let next = new.map(TriggerSource::Preset);
                if t.source != next {
                    t.source = next;
                    audio_changed = true;
                }
            }
        }
        for name in lost_triggers {
            self.log.push(format!("Audio trigger \"{name}\" lost its preset"));
        }
        if audio_changed {
            self.persist_audio();
        }
        if let Some(ChaseSource::User(old)) = self.chase.source {
            let new = map.get(old).copied().flatten();
            self.chase.source = new.map(ChaseSource::User);
            if new.is_none() {
                self.log.push("Chase source cleared — its preset was deleted".into());
            }
        }
        if let Some(old) = self.active_user_preset {
            self.active_user_preset = map.get(old).copied().flatten();
        }
    }

    /// Whether there is anything worth storing: a base value, a running
    /// oscillator or a channel dialled in on the encoders. Mirrors
    /// `store_user_preset`'s own early return.
    pub(crate) fn programmer_has_look(&self) -> bool {
        let look = match &self.transition_run {
            Some(run) => run.pending(),
            None => &self.live,
        };
        look.base.iter().any(|&v| v != 0)
            || look.oscs.values().any(|o| o.enabled)
            || self.encoder_layer.values().any(|&v| v != 0)
    }

    /// The colour the programmer would be stored as, for the store card's
    /// live swatch.
    fn programmer_face(&self) -> Option<[u8; 3]> {
        let look = match &self.transition_run {
            Some(run) => run.pending(),
            None => &self.live,
        };
        let mut values: Vec<(usize, u8)> = look
            .base
            .iter()
            .enumerate()
            .filter(|&(_, &v)| v != 0)
            .map(|(a, &v)| (a, v))
            .collect();
        values.retain(|(a, _)| !self.encoder_layer.contains_key(a));
        values.extend(self.encoder_layer.iter().filter(|(_, &v)| v != 0).map(|(&a, &v)| (a, v)));
        if values.is_empty() {
            return None;
        }
        let patch = &self.patch;
        let role = |a: usize| role_at_in(patch, a).map(|(_, r)| r);
        Some(swatch_from_values(&values, &role).rgb)
    }

    /// Run `f` with the Transition window's duration temporarily replaced.
    /// Safe because `TransitionRun::new` / `push` copy the config when the
    /// run starts — the per-frame tick never reads `transition.duration`
    /// again, so putting the operator's own value back cannot disturb a
    /// fade already under way.
    fn with_fade(&mut self, fade: Option<f32>, f: impl FnOnce(&mut Self)) {
        // No explicit time means the Transition window's Presets recall
        // slot — the global in simple mode, its own row in advanced.
        let s = fade.unwrap_or_else(|| {
            self.transition
                .fade(crate::transition::TransitionTarget::PresetRecall)
        });
        let keep = self.transition.duration;
        self.transition.duration = s.max(0.0);
        f(self);
        self.transition.duration = keep;
    }

    /// Recall native preset `idx`, fading by the one rule: an explicit fade
    /// wins, then the preset's own, then the recall strip's mode.
    pub(crate) fn recall_user_preset(&mut self, idx: usize, fade: Option<f32>) {
        let Some(id) = self.user_presets.get(idx).map(|p| p.id) else { return };
        let own = self.user_presets[idx].fade;
        let mode = self.insp.prefs.presets.recall;
        let secs = self.insp.prefs.presets.fade_secs;
        let f = effective_fade(fade, own, mode, secs);
        self.with_fade(f, |a| a.apply_user_preset(idx));
        let recent = &mut self.insp.presets.recent;
        recent.retain(|&r| r != id);
        recent.push_front(id);
        while recent.len() > 6 {
            recent.pop_back();
        }
    }

    /// Recall a ShowBuddy bank preset under the same fade rule.
    pub(crate) fn recall_bank_preset(&mut self, bi: usize, pi: usize, fade: Option<f32>) {
        let mode = self.insp.prefs.presets.recall;
        let secs = self.insp.prefs.presets.fade_secs;
        let f = effective_fade(fade, None, mode, secs);
        self.with_fade(f, |a| a.apply_preset(bi, pi));
    }

    /// Store the programmer as a new preset, filed into `folder` when that
    /// folder exists. Returns the new pool index.
    pub(crate) fn store_look_into(&mut self, name: String, folder: String) -> Option<usize> {
        let name = if name.trim().is_empty() {
            format!("Preset {}", self.user_presets.len() + 1)
        } else {
            name.trim().to_owned()
        };
        let before = self.user_presets.len();
        self.store_user_preset(name.clone());
        if self.user_presets.len() <= before {
            return None;
        }
        let idx = self.user_presets.len() - 1;
        let filed = !folder.is_empty() && self.preset_folders.contains(&folder);
        if filed {
            self.user_presets[idx].folder = folder.clone();
        }
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.insp.presets.reveal = Some(self.user_presets[idx].id);
        if filed {
            self.log.push(format!("Stored preset \"{name}\" in folder \"{folder}\""));
        }
        Some(idx)
    }

    /// Overwrite the lit preset with the current look, keeping everything
    /// that is not the look itself.
    pub(crate) fn update_preset_in_place(&mut self, idx: usize) -> bool {
        let Some(old) = self.user_presets.get(idx).cloned() else { return false };
        let before = self.user_presets.len();
        self.store_user_preset(old.name.clone());
        if self.user_presets.len() <= before {
            return false;
        }
        // The id `store_user_preset` handed out is simply skipped — ids are
        // never reused, so the pool keeps its stable references.
        let mut newp = self.user_presets.pop().unwrap();
        newp.id = old.id;
        newp.folder = old.folder;
        newp.color = old.color;
        newp.symbol = old.symbol;
        newp.icon = old.icon;
        newp.fade = old.fade;
        newp.pinned = old.pinned;
        self.user_presets[idx] = newp;
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.forget_preset_cache(old.id);
        self.log.push(format!("Updated preset \"{}\" with the current look", old.name));
        true
    }

    /// Drop one preset's cached swatch and contents, so the next frame
    /// derives them again.
    fn forget_preset_cache(&mut self, id: u32) {
        self.insp.presets.swatches.remove(&id);
        self.insp.presets.contents.remove(&id);
    }

    /// Drop every id-keyed scrap the Presets tab is holding. `cache_key`
    /// only notices the *rig* changing shape, so a wholesale swap of the
    /// pool — undo / redo of a configuration, a backup restore, File → New
    /// show — leaves ids in place with different looks behind them, and the
    /// swatches, badges and hue sort would stay on the old values for ever.
    pub(crate) fn forget_preset_caches(&mut self) {
        let cache = &mut self.insp.presets;
        cache.swatches.clear();
        cache.contents.clear();
        cache.bank_swatches.clear();
        cache.recent.clear();
        cache.edit = None;
    }

    /// Rename a preset, keeping the board's auto-generated pad labels in
    /// step with it.
    pub(crate) fn rename_preset(&mut self, idx: usize, name: &str) {
        let Some(p) = self.user_presets.get_mut(idx) else { return };
        let old = std::mem::replace(&mut p.name, name.to_owned());
        let (id, old_symbol, new_symbol) = (p.id, pad_symbol(&old), pad_symbol(name));
        let mut touched = false;
        for slot in self.preset_deck.iter_mut().flatten() {
            if slot.preset == id {
                slot.name = name.to_owned();
                if slot.label == old_symbol {
                    slot.label = new_symbol.clone();
                }
                touched = true;
            }
        }
        if touched {
            crate::preset_deck::save_preset_deck(&self.preset_deck);
        }
        self.log.push(format!("Renamed preset \"{old}\" → \"{name}\""));
    }

    /// Apply the Edit pad card: name, colour, symbol, artwork, fade, pin.
    pub(crate) fn apply_preset_edit(&mut self, e: &PresetEdit) -> bool {
        let Some(idx) = self.preset_index_by_id(e.id) else { return false };
        let before_face = self.preset_face(&self.user_presets[idx]);
        let name = e.name.trim().to_owned();
        if !name.is_empty() && name != self.user_presets[idx].name {
            self.rename_preset(idx, &name);
        }
        let p = &mut self.user_presets[idx];
        let color = (!e.auto_color).then_some(e.color);
        let fade = e.own_fade.then_some(e.fade_secs);
        let changed = p.color != color
            || p.symbol != e.symbol
            || p.icon != e.icon
            || p.fade != fade
            || p.pinned != e.pinned;
        p.color = color;
        p.symbol = e.symbol.trim().to_owned();
        p.icon = e.icon;
        p.fade = fade;
        p.pinned = e.pinned;
        let name = p.name.clone();
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.forget_preset_cache(e.id);
        // A pad that wore the derived colour follows the new one; a pad
        // someone coloured by hand keeps what it was given.
        let face = self.preset_face(&self.user_presets[idx]);
        let mut touched = false;
        if face != before_face {
            for slot in self.preset_deck.iter_mut().flatten() {
                if slot.preset == e.id && slot.color == before_face {
                    slot.color = face;
                    touched = true;
                }
            }
        }
        if touched {
            crate::preset_deck::save_preset_deck(&self.preset_deck);
        }
        if changed {
            self.log.push(format!("Edited preset \"{name}\""));
        }
        true
    }

    /// File a preset into a folder (or back to the top level).
    pub(crate) fn set_preset_folder(&mut self, idx: usize, folder: &str) {
        let Some(p) = self.user_presets.get_mut(idx) else { return };
        if p.folder == folder {
            return;
        }
        p.folder = folder.to_owned();
        let name = p.name.clone();
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.log.push(if folder.is_empty() {
            format!("Preset \"{name}\" → top level")
        } else {
            format!("Preset \"{name}\" → folder \"{folder}\"")
        });
    }

    /// Move a preset in front of another one, adopting its folder.
    pub(crate) fn move_preset_before(&mut self, src: usize, dst: usize) {
        if src == dst || src >= self.user_presets.len() || dst >= self.user_presets.len() {
            return;
        }
        let len = self.user_presets.len();
        let mut p = self.user_presets.remove(src);
        let dst2 = if src < dst { dst - 1 } else { dst };
        p.folder = self.user_presets[dst2].folder.clone();
        self.user_presets.insert(dst2, p);
        self.remap_preset_refs(&move_map(len, src, dst));
        preset::save_presets(&self.preset_folders, &self.user_presets);
    }

    /// Copy a preset, with a fresh id, right after the original.
    pub(crate) fn duplicate_preset(&mut self, idx: usize) -> Option<usize> {
        let mut copy = self.user_presets.get(idx)?.clone();
        copy.id = self.next_preset_id;
        self.next_preset_id += 1;
        copy.name = format!("{} copy", copy.name);
        copy.pinned = false;
        let name = copy.name.clone();
        let len = self.user_presets.len();
        self.user_presets.insert(idx + 1, copy);
        self.remap_preset_refs(&insert_map(len, idx + 1));
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.log.push(format!("Duplicated preset → \"{name}\""));
        Some(idx + 1)
    }

    /// Delete a preset. Board pads keep the id: the pad shows a `?` face
    /// and says so when pressed, rather than silently recalling something
    /// else.
    pub(crate) fn delete_preset(&mut self, idx: usize) {
        if idx >= self.user_presets.len() {
            return;
        }
        let len = self.user_presets.len();
        let p = self.user_presets.remove(idx);
        self.remap_preset_refs(&delete_map(len, idx));
        self.forget_preset_cache(p.id);
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.log.push(format!("Deleted preset \"{}\"", p.name));
    }

    /// Pin or unpin a preset, which puts it in the strip at the top.
    pub(crate) fn toggle_pin(&mut self, idx: usize) {
        let Some(p) = self.user_presets.get_mut(idx) else { return };
        p.pinned = !p.pinned;
        let (pinned, name) = (p.pinned, p.name.clone());
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.log.push(if pinned {
            format!("Pinned preset \"{name}\"")
        } else {
            format!("Unpinned preset \"{name}\"")
        });
    }

    /// Add a folder; refuses an empty or duplicate name.
    pub(crate) fn add_preset_folder(&mut self, name: &str) -> bool {
        let name = name.trim().to_owned();
        if name.is_empty() {
            self.log.push("Folder: give it a name first".into());
            return false;
        }
        if self.preset_folders.contains(&name) {
            self.log.push(format!("Folder \"{name}\" already exists"));
            return false;
        }
        self.preset_folders.push(name.clone());
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.insp.set_open(&folder_key(&name), true, true);
        self.insp.presets.store_into = name.clone();
        self.log.push(format!("Added folder \"{name}\""));
        true
    }

    /// Rename a folder and everything filed in it.
    pub(crate) fn rename_preset_folder(&mut self, old: &str, new: &str) -> bool {
        let new = new.trim().to_owned();
        if new.is_empty() || new == old {
            return false;
        }
        if self.preset_folders.contains(&new) {
            self.log.push(format!("Folder \"{new}\" already exists"));
            return false;
        }
        let Some(fi) = self.preset_folders.iter().position(|f| f == old) else { return false };
        self.preset_folders[fi] = new.clone();
        for p in &mut self.user_presets {
            if p.folder == old {
                p.folder = new.clone();
            }
        }
        if self.insp.presets.store_into == old {
            self.insp.presets.store_into = new.clone();
        }
        let was_open = self.insp.is_open(&folder_key(old), true);
        self.insp.set_open(&folder_key(old), true, true);
        self.insp.set_open(&folder_key(&new), true, was_open);
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.log.push(format!("Renamed folder \"{old}\" → \"{new}\""));
        true
    }

    /// Delete a folder; its presets go back to the top level.
    pub(crate) fn delete_preset_folder(&mut self, fi: usize) {
        if fi >= self.preset_folders.len() {
            return;
        }
        let name = self.preset_folders.remove(fi);
        for p in &mut self.user_presets {
            if p.folder == name {
                p.folder.clear();
            }
        }
        if self.insp.presets.store_into == name {
            self.insp.presets.store_into.clear();
        }
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.log.push(format!("Deleted folder \"{name}\" (presets kept)"));
    }

    /// Reorder the folders.
    pub(crate) fn move_preset_folder(&mut self, src: usize, dst: usize) {
        if src == dst || src >= self.preset_folders.len() || dst >= self.preset_folders.len() {
            return;
        }
        let name = self.preset_folders.remove(src);
        let dst2 = if src < dst { dst - 1 } else { dst };
        self.preset_folders.insert(dst2, name.clone());
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.log.push(format!("Folder \"{name}\" moved"));
    }

    /// Recall a preset onto the selected lights only: their channels take
    /// the preset's values, everything else keeps playing.
    pub(crate) fn apply_user_preset_masked(&mut self, idx: usize) {
        let Some(p) = self.user_presets.get(idx).cloned() else { return };
        let lights = self.stage.selected_fixtures();
        if lights.is_empty() {
            return;
        }
        let mut mask: HashSet<usize> = HashSet::new();
        for fi in &lights {
            if let Some(f) = self.patch.fixtures.get(*fi) {
                let from0 = f.from as usize - 1;
                for a in from0..(from0 + f.channel_count()).min(net::DMX_SLOTS) {
                    mask.insert(a);
                }
            }
        }
        let own = p.fade;
        let mode = self.insp.prefs.presets.recall;
        let secs = self.insp.prefs.presets.fade_secs;
        let fade = effective_fade(None, own, mode, secs);
        let n = lights.len();
        self.with_fade(fade, |a| {
            let mut target = match &a.transition_run {
                Some(run) => run.pending().clone(),
                None => a.live.clone(),
            };
            for &(addr, v) in &p.values {
                if mask.contains(&addr) && addr < net::DMX_SLOTS {
                    target.base[addr] = v;
                    target.oscs.remove(&addr);
                }
            }
            for (addr, osc) in p.osc_map() {
                if mask.contains(&addr) {
                    target.oscs.insert(addr, osc);
                }
            }
            a.live_active.extend(mask.iter().copied());
            a.live_refs.retain(|addr, _| !mask.contains(addr));
            if a.transition.duration <= 0.0 {
                a.transition_run = None;
                a.live = target;
                *a.net.dmx.lock() = a.live.render();
            } else {
                let positions = a.fixture_positions();
                if let Some(run) = &mut a.transition_run {
                    run.push(target, &a.transition, &a.patch, &positions);
                } else {
                    let from = std::mem::replace(&mut a.live, Look::black());
                    a.transition_run =
                        Some(TransitionRun::new(from, target, &a.transition, &a.patch, &positions));
                }
            }
        });
        self.log.push(format!("Recalled preset \"{}\" onto {n} selected lights", p.name));
    }

    /// Copy one ShowBuddy preset into the native pool.
    pub(crate) fn import_bank_preset(
        &mut self,
        bi: usize,
        pi: usize,
        folder: &str,
    ) -> Option<usize> {
        let (look, name) = self.load_look(bi, pi)?;
        let folder = folder.to_owned();
        if !folder.is_empty() && !self.preset_folders.contains(&folder) {
            self.preset_folders.push(folder.clone());
        }
        let id = self.next_preset_id;
        self.next_preset_id += 1;
        self.user_presets.push(preset_from_look(&look, name, folder, id));
        preset::save_presets(&self.preset_folders, &self.user_presets);
        Some(self.user_presets.len() - 1)
    }

    /// Copy a whole ShowBuddy bank into a folder of its own name.
    pub(crate) fn import_bank(&mut self, bi: usize) {
        let Some(bank) = self.banks.get(bi) else { return };
        let (folder, count) = (bank.name.clone(), bank.presets.len());
        let mut done = 0;
        for pi in 0..count {
            if self.import_bank_preset(bi, pi, &folder).is_some() {
                done += 1;
            } else {
                self.log.push(format!("Bank \"{folder}\": preset {} would not load", pi + 1));
            }
        }
        self.insp.set_open(&folder_key(&folder), true, true);
        self.log.push(format!("Imported {done} presets from bank \"{folder}\""));
    }

    /// Every channel to zero, every effect stopped — the Presets tab's own
    /// blackout, which also drops the programmer.
    pub(crate) fn inspector_blackout(&mut self) {
        *self.net.dmx.lock() = Frame::black();
        self.live = Look::black();
        self.active_preset = None;
        self.active_user_preset = None;
        self.active_phasers.clear();
        self.hold_overrides.clear();
        self.transition_run = None;
        self.chase.enabled = false;
        self.chase_run = None;
        self.log.push("Blackout — all channels to zero".into());
    }

    /// The command bar's `preset <n|name>`.
    pub(crate) fn command_recall_preset(&mut self, arg: &str) {
        let arg = arg.trim();
        if let Ok(n) = arg.parse::<usize>() {
            if n >= 1 && n <= self.user_presets.len() {
                self.recall_user_preset(n - 1, None);
            } else {
                self.log
                    .push(format!("No preset {n} (the pool holds {})", self.user_presets.len()));
            }
            return;
        }
        let want = arg.to_lowercase();
        match self.user_presets.iter().position(|p| p.name.to_lowercase().starts_with(&want)) {
            Some(i) => self.recall_user_preset(i, None),
            None => self.log.push("Usage: preset <n|name>".into()),
        }
    }
}

/// The shell fold key a folder's open state is stored under.
fn folder_key(name: &str) -> String {
    format!("presets.folder.{name}")
}

// ---- the pool, drawn ----

/// One preset, read out of the pool before any drawing starts, so the pad
/// and row loops never borrow `App`.
struct PadInfo {
    idx: usize,
    id: u32,
    face: [u8; 3],
    symbol: String,
    name: String,
    folder: String,
    contents: PresetContents,
    active: bool,
    pinned: bool,
    own_fade: Option<f32>,
    on_board: bool,
}

/// Everything the pad and row loops need besides the presets themselves.
#[derive(Clone, Copy)]
struct PoolCtx<'a> {
    /// Which strip these pads belong to, so one preset can appear in the
    /// pinned strip, the grid and the recent strip at once.
    scope: &'static str,
    folders: &'a [String],
    pad_size: PadSize,
    /// The preset whose Edit card is open.
    editing: Option<u32>,
    /// Manual sort with no search running: a drag reorders the pool.
    reorder: bool,
    searching: bool,
    has_selection: bool,
    /// A just-stored preset, scrolled into view once.
    reveal: Option<u32>,
    /// The seconds the "Recall with … fade" menu item offers.
    fade_secs: f32,
}

/// The sentence a pad or row shows on hover.
fn pad_tooltip(info: &PadInfo) -> String {
    let mut parts = vec![format!("\"{}\"", info.name)];
    if !info.folder.is_empty() {
        parts.push(format!("folder {}", info.folder));
    }
    let c = &info.contents;
    if c.fixtures > 0 {
        parts.push(format!("{} light{}", c.fixtures, if c.fixtures == 1 { "" } else { "s" }));
    }
    let mut roles: Vec<&str> = Vec::new();
    if c.colour {
        roles.push("colour");
    }
    if c.dimmer {
        roles.push("dimmer");
    }
    if c.position {
        roles.push("position");
    }
    if c.beam {
        roles.push("beam");
    }
    if !roles.is_empty() {
        parts.push(roles.join(", "));
    }
    if c.oscs > 0 {
        parts.push(format!("{} wave{}", c.oscs, if c.oscs == 1 { "" } else { "s" }));
    }
    if let Some(f) = info.own_fade {
        parts.push(format!("fade {f:.1} s"));
    }
    if info.pinned {
        parts.push("pinned".into());
    }
    if info.on_board {
        parts.push("on the board".into());
    }
    format!(
        "{} — click to recall, Shift-click to cut, Ctrl-click to put it on the board, \
         double-click to edit, right-click for more.",
        parts.join(" · ")
    )
}

/// The badges a preset carries, as (short word, colour).
fn badge_words(c: &PresetContents, own_fade: bool) -> Vec<(&'static str, Color32)> {
    let mut out: Vec<(&'static str, Color32)> = Vec::new();
    if c.position {
        out.push(("pos", role_color(Role::Pan)));
    }
    if c.beam {
        out.push(("beam", role_color(Role::Gobo)));
    }
    if c.oscs > 0 {
        out.push(("wave", theme::ACCENT_SOFT));
    }
    if own_fade {
        out.push(("fade", theme::TEXT_DIM));
    }
    out
}

/// The pin, fade, badge and board marks painted over a pad's face.
fn pad_overlay(p: &egui::Painter, rect: Rect, info: &PadInfo, z: f32) {
    // Top left, after the painter's own play mark.
    if info.pinned {
        let x = rect.left() + 5.0 + if info.active { 11.0 } else { 0.0 };
        let r = Rect::from_min_size(Pos2::new(x, rect.top() + 5.0), Vec2::splat(9.0 * z));
        icons::draw(p, r, Icon::Pin, theme::ACCENT_SOFT);
    }
    // Top right, laid out right to left.
    let mut right = rect.right() - 4.0 * z;
    if info.own_fade.is_some() {
        let s = 9.0 * z;
        let r = Rect::from_min_size(Pos2::new(right - s, rect.top() + 4.0 * z), Vec2::splat(s));
        icons::draw(p, r, Icon::Fade, theme::TEXT);
        right -= s + 3.0 * z;
    }
    let c = &info.contents;
    let dots = [
        (c.position, role_color(Role::Pan)),
        (c.beam, role_color(Role::Gobo)),
        (c.oscs > 0, theme::ACCENT_SOFT),
    ];
    let dot = 2.5 * z;
    for (on, color) in dots.into_iter().rev() {
        if !on {
            continue;
        }
        p.circle_filled(Pos2::new(right - dot, rect.top() + 4.0 * z + dot), dot, color);
        p.circle_stroke(
            Pos2::new(right - dot, rect.top() + 4.0 * z + dot),
            dot,
            egui::Stroke::new(0.5, Color32::from_black_alpha(90)),
        );
        right -= dot * 2.0 + 2.0 * z;
    }
    if info.on_board {
        p.circle_filled(
            Pos2::new(rect.right() - 5.0 * z, rect.bottom() - 5.0 * z),
            2.0 * z,
            theme::ACCENT_SOFT,
        );
    }
}

/// The context menu every pad and row carries.
fn preset_menu(
    ui: &mut egui::Ui,
    info: &PadInfo,
    ctx: &PoolCtx,
    acts: &mut Vec<PresetAction>,
) {
    let idx = info.idx;
    if ui.button("Recall").clicked() {
        acts.push(PresetAction::Recall { idx, fade: None });
        ui.close_menu();
    }
    if ui.button(format!("Recall with {:.1} s fade", ctx.fade_secs)).clicked() {
        acts.push(PresetAction::Recall { idx, fade: Some(ctx.fade_secs) });
        ui.close_menu();
    }
    if ui.button("Cut to (no fade)").clicked() {
        acts.push(PresetAction::Recall { idx, fade: Some(0.0) });
        ui.close_menu();
    }
    let masked = ui
        .add_enabled(ctx.has_selection, egui::Button::new("Recall onto selection"))
        .on_hover_text("Apply this preset's channels only on the selected lights.")
        .on_disabled_hover_text("Select some lights first.");
    if masked.clicked() {
        acts.push(PresetAction::RecallMasked(idx));
        ui.close_menu();
    }
    ui.separator();
    if ui.button("Update with current look").clicked() {
        acts.push(PresetAction::Update(idx));
        ui.close_menu();
    }
    if ui.button("Edit pad…").clicked() {
        acts.push(PresetAction::Edit(info.id));
        ui.close_menu();
    }
    if ui.button(if info.pinned { "Unpin" } else { "Pin" }).clicked() {
        acts.push(PresetAction::Pin(idx));
        ui.close_menu();
    }
    ui.menu_button("Move to", |ui| {
        if ui.selectable_label(info.folder.is_empty(), "(top level)").clicked() {
            acts.push(PresetAction::File(info.id, String::new()));
            ui.close_menu();
        }
        for f in ctx.folders {
            if ui.selectable_label(info.folder == *f, f).clicked() {
                acts.push(PresetAction::File(info.id, f.clone()));
                ui.close_menu();
            }
        }
    });
    let board = ui
        .add_enabled(!info.on_board, egui::Button::new("To board"))
        .on_hover_text("Put this preset on the next free pad of the Presets board.")
        .on_disabled_hover_text("Already on the board.");
    if board.clicked() {
        acts.push(PresetAction::ToBoard(idx));
        ui.close_menu();
    }
    if ui.button("Set as chase source").clicked() {
        acts.push(PresetAction::ChaseSource(idx));
        ui.close_menu();
    }
    if ui.button("Duplicate").clicked() {
        acts.push(PresetAction::Duplicate(idx));
        ui.close_menu();
    }
    ui.separator();
    if theme::danger_button(ui, "Delete").clicked() {
        acts.push(PresetAction::Delete(idx));
        ui.close_menu();
    }
}

/// What a click on a pad or a row asked for.
fn preset_click(resp: &egui::Response, ui: &egui::Ui, info: &PadInfo, acts: &mut Vec<PresetAction>) {
    if resp.double_clicked() {
        acts.push(PresetAction::Edit(info.id));
        return;
    }
    if resp.clicked() {
        let (shift, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
        if command {
            acts.push(PresetAction::ToBoard(info.idx));
        } else {
            acts.push(PresetAction::Recall {
                idx: info.idx,
                fade: shift.then_some(0.0),
            });
        }
    }
}

/// One pad: the face, its marks, the drag source and the menu.
fn preset_pad(
    ui: &mut egui::Ui,
    info: &PadInfo,
    size: Vec2,
    ctx: &PoolCtx,
    acts: &mut Vec<PresetAction>,
) {
    let z = theme::zoom_of(ui);
    // Allocated straight on the parent, not inside a `push_id`: a child `Ui`
    // is placed with `allocate_rect`, which a wrapping row cannot break
    // before, so a strip of pads would run off the panel instead of wrapping.
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let resp = ui.interact(
        rect,
        ui.id().with(("insp_presets_pad", ctx.scope, info.id)),
        Sense::click_and_drag(),
    );
    {
        let hover_drop = resp.dnd_hover_payload::<PresetDrag>().is_some();
        if ui.is_rect_visible(rect) {
            theme::paint_pad(
                ui.painter(),
                rect,
                theme::PadState {
                    fill: Some(Color32::from_rgb(info.face[0], info.face[1], info.face[2])),
                    active: info.active,
                    editing: ctx.editing == Some(info.id),
                    hovered: resp.hovered(),
                    drop_target: false,
                    dim: false,
                },
                theme::PadFace::Symbol(&info.symbol),
                &info.name,
            );
            pad_overlay(ui.painter(), rect, info, z);
            if hover_drop {
                // Insert-before mark on the pad's leading edge.
                ui.painter().vline(
                    rect.left() - 2.0,
                    rect.y_range(),
                    egui::Stroke::new(2.0, theme::ACCENT_SOFT),
                );
            }
        }
        if ctx.reveal == Some(info.id) {
            resp.scroll_to_me(None);
        }
        // The pin sits on top of the pad, with an id of its own.
        let mut pinned = false;
        if info.pinned || resp.hovered() {
            let x = rect.left() + 5.0 + if info.active { 11.0 } else { 0.0 };
            let pin_rect =
                Rect::from_min_size(Pos2::new(x - 2.0, rect.top() + 3.0), Vec2::splat(13.0 * z));
            let pin = ui.interact(
                pin_rect,
                ui.id().with(("insp_presets_pin", ctx.scope, info.id)),
                Sense::click(),
            );
            if !info.pinned && pin.hovered() {
                icons::draw(
                    ui.painter(),
                    pin_rect.shrink(2.0),
                    Icon::Pin,
                    theme::TEXT_DIM,
                );
            }
            pinned = pin
                .on_hover_text(if info.pinned {
                    "Unpin this preset — it leaves the strip at the top."
                } else {
                    "Pin this preset to the strip at the top of the tab."
                })
                .clicked();
        }
        if pinned {
            acts.push(PresetAction::Pin(info.idx));
        } else {
            preset_click(&resp, ui, info, acts);
        }
        if resp.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), PresetDrag(info.id));
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if let Some(src) = resp.dnd_release_payload::<PresetDrag>() {
            if ctx.reorder {
                acts.push(PresetAction::MoveBefore(src.0, info.idx));
            }
        }
        let resp = resp.on_hover_text(pad_tooltip(info));
        resp.context_menu(|ui| preset_menu(ui, info, ctx, acts));
    }
}

/// One row in list view.
fn preset_row(ui: &mut egui::Ui, info: &PadInfo, ctx: &PoolCtx, indent: f32, acts: &mut Vec<PresetAction>) {
    let z = theme::zoom_of(ui);
    let badges = badge_words(&info.contents, info.own_fade.is_some());
    let sub = (ctx.searching && !info.folder.is_empty()).then(|| info.folder.clone());
    let row = theme::Row {
        swatch: Some(Color32::from_rgb(info.face[0], info.face[1], info.face[2])),
        icon: None,
        title: &info.name,
        subtitle: sub.as_deref(),
        badges: &badges,
        selected: ctx.editing == Some(info.id),
        active: info.active,
        indent,
    };
    let resp = theme::list_row(ui, ("insp_presets_row", ctx.scope, info.id), &row);
    if ui.is_rect_visible(resp.rect) && (info.pinned || info.on_board) {
        let p = ui.painter();
        let mut x = resp.rect.right() - 6.0 * z;
        if info.active {
            x -= 14.0 * z;
        }
        if info.on_board {
            p.circle_filled(Pos2::new(x - 2.0 * z, resp.rect.center().y), 2.5 * z, theme::ACCENT_SOFT);
            x -= 9.0 * z;
        }
        if info.pinned {
            let r = Rect::from_center_size(
                Pos2::new(x - 5.0 * z, resp.rect.center().y),
                Vec2::splat(10.0 * z),
            );
            icons::draw(p, r, Icon::Pin, theme::ACCENT_SOFT);
        }
    }
    if ctx.reveal == Some(info.id) {
        resp.scroll_to_me(None);
    }
    preset_click(&resp, ui, info, acts);
    if resp.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), PresetDrag(info.id));
    }
    if let Some(src) = resp.dnd_release_payload::<PresetDrag>() {
        if ctx.reorder {
            acts.push(PresetAction::MoveBefore(src.0, info.idx));
        }
    } else if resp.dnd_hover_payload::<PresetDrag>().is_some() {
        ui.painter().hline(
            resp.rect.x_range(),
            resp.rect.top(),
            egui::Stroke::new(2.0, theme::ACCENT_SOFT),
        );
    }
    let resp = resp.on_hover_text(pad_tooltip(info));
    resp.context_menu(|ui| preset_menu(ui, info, ctx, acts));
}

/// A block of pads in the auto-column grid.
fn preset_pads(
    ui: &mut egui::Ui,
    pads: &[&PadInfo],
    ctx: &PoolCtx,
    indent: f32,
    acts: &mut Vec<PresetAction>,
) {
    if pads.is_empty() {
        return;
    }
    let z = theme::zoom_of(ui);
    let gap = 5.0 * z;
    // The column count comes from the full width, so an indented folder
    // block keeps the same grid as the top level — only its pads are a
    // little narrower.
    let cols = theme::pad_columns(ui.available_width(), ctx.pad_size.min_w() * z, gap);
    let full = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).max(24.0);
    let avail = ui.available_width() - indent;
    let w = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).max(24.0);
    // One height for every pad in the tab, so an indented block still shows
    // its names.
    let h = (full * 0.74).clamp(34.0 * z, 92.0 * z);
    for chunk in pads.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            if indent > 0.0 {
                ui.add_space(indent);
            }
            for info in chunk {
                preset_pad(ui, info, Vec2::new(w, h), ctx, acts);
            }
        });
        ui.add_space(gap);
    }
}

/// One folder, as it is read out of the pool before drawing.
struct FolderInfo {
    fi: usize,
    name: String,
    faces: Vec<[u8; 3]>,
    /// The first member's pool index, for "Recall first preset".
    first: Option<usize>,
    open: bool,
}

impl App {
    /// The Presets tab.
    pub(crate) fn inspector_presets_tab(&mut self, ui: &mut egui::Ui) {
        ui.push_id("insp_presets", |ui| {
            self.refresh_preset_caches();
            if !self.insp.presets.store_into.is_empty()
                && !self.preset_folders.contains(&self.insp.presets.store_into)
            {
                self.insp.presets.store_into.clear();
            }
            let mut acts: Vec<PresetAction> = Vec::new();
            self.presets_toolbar(ui, &mut acts);
            self.presets_status(ui);
            self.presets_store_card(ui, &mut acts);
            self.presets_recall_strip(ui);
            self.presets_pool(ui, &mut acts);
            self.presets_edit_card(ui);
            self.presets_recent(ui, &mut acts);
            self.presets_banks(ui);
            self.presets_output(ui);
            self.insp.presets.reveal = None;
            self.apply_preset_actions(acts);
        });
    }

    /// One preset read out of the pool for drawing.
    fn pad_info_at(&self, idx: usize) -> Option<PadInfo> {
        let p = self.user_presets.get(idx)?;
        Some(PadInfo {
            idx,
            id: p.id,
            face: self.preset_face(p),
            symbol: symbol_of(p),
            name: p.name.clone(),
            folder: p.folder.clone(),
            contents: self.insp.presets.contents.get(&p.id).copied().unwrap_or_default(),
            active: self.active_user_preset == Some(idx),
            pinned: p.pinned,
            own_fade: p.fade,
            on_board: self.preset_on_board(p.id),
        })
    }

    /// Search, view, sort and the board toggle.
    fn presets_toolbar(&mut self, ui: &mut egui::Ui, acts: &mut Vec<PresetAction>) {
        let z = theme::zoom_of(ui);
        let mut clear = false;
        let mut enter = false;
        let sort_id = ui.make_persistent_id("insp_presets_sort_menu");
        theme::toolbar(ui, |ui| {
            let margin = egui::Margin {
                left: 18.0 * z,
                right: 15.0 * z,
                top: 2.0,
                bottom: 2.0,
            };
            let w = field_width(ui, 2, margin.sum().x);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.insp.presets.search)
                    .id_salt("insp_presets_search")
                    .hint_text("search presets…")
                    .margin(margin)
                    .desired_width(w),
            );
            let r = resp.rect;
            icons::draw(
                ui.painter(),
                Rect::from_center_size(
                    Pos2::new(r.left() - 9.0 * z, r.center().y),
                    Vec2::splat(11.0 * z),
                ),
                Icon::Search,
                theme::TEXT_DIM,
            );
            if !self.insp.presets.search.is_empty() {
                let cr = Rect::from_center_size(
                    Pos2::new(r.right() + 7.0 * z, r.center().y),
                    Vec2::splat(13.0 * z),
                );
                let x = ui.interact(cr, ui.id().with("insp_presets_search_clear"), Sense::click());
                icons::draw(
                    ui.painter(),
                    cr.shrink(2.0),
                    Icon::Close,
                    if x.hovered() { theme::TEXT } else { theme::TEXT_DIM },
                );
                if x.on_hover_text("Clear the search box.").clicked() {
                    clear = true;
                }
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                enter = true;
                resp.request_focus();
            }
            if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                clear = true;
            }
            let sort = theme::tool_button(
                ui,
                Icon::Sort,
                None,
                "Choose the order presets are listed in, and how big the pads are drawn.",
                Ok(()),
            );
            if sort.clicked() {
                ui.memory_mut(|m| m.toggle_popup(sort_id));
            }
            egui::popup_below_widget(
                ui,
                sort_id,
                &sort,
                egui::PopupCloseBehavior::CloseOnClickOutside,
                |ui| {
                    ui.set_min_width(140.0);
                    theme::label_dim(ui, "Order");
                    for s in PresetSort::ALL {
                        if ui
                            .selectable_label(self.insp.prefs.presets.sort == s, s.label())
                            .clicked()
                        {
                            self.insp.prefs.presets.sort = s;
                            self.insp.mark_dirty();
                        }
                    }
                    ui.add_space(4.0);
                    theme::label_dim(ui, "Pad size");
                    for p in PadSize::ALL {
                        if ui
                            .selectable_label(self.insp.prefs.presets.pad == p, p.label())
                            .clicked()
                        {
                            self.insp.prefs.presets.pad = p;
                            self.insp.mark_dirty();
                        }
                    }
                },
            );
            let board = theme::toggle_icon(
                ui,
                Icon::Board,
                None,
                self.show_preset_board,
                "Open the Presets board: a page of pads that recall looks, mirrored on the \
                 Stream Deck's Presets page.",
            );
            if board.clicked() {
                self.show_preset_board = !self.show_preset_board;
            }
        });
        let view = self.insp.prefs.presets.view;
        let segs = [
            theme::Segment {
                icon: Some(Icon::Grid),
                label: "Grid",
                hint: "Presets as pads, in their own colours.",
                badge: None,
            },
            theme::Segment {
                icon: Some(Icon::List),
                label: "List",
                hint: "One row per preset, with its badges.",
                badge: None,
            },
        ];
        if let Some(i) =
            theme::segmented(ui, "insp_presets_view", &segs, usize::from(view == PresetView::List))
        {
            self.insp.prefs.presets.view = if i == 1 { PresetView::List } else { PresetView::Grid };
            self.insp.mark_dirty();
        }
        if clear {
            self.insp.presets.search.clear();
        }
        let searching = !self.insp.presets.search.trim().is_empty();
        if searching {
            let total = self.user_presets.len();
            let hits = self.visible_presets().len();
            theme::hint(ui, format!("{hits} of {total} presets · Enter recalls the first"));
            if enter {
                if let Some(&idx) = self.visible_presets().first() {
                    acts.push(PresetAction::Recall { idx, fade: None });
                } else if let Some((bi, pi)) = self.first_bank_match() {
                    acts.push(PresetAction::RecallBank(bi, pi, None));
                }
            }
        }
    }

    /// The pool indices the search and sort leave visible.
    fn visible_presets(&self) -> Vec<usize> {
        let faces = |id: u32| -> [u8; 3] {
            self.user_presets
                .iter()
                .find(|p| p.id == id)
                .map_or_else(|| rgb_of(theme::RAISED), |p| self.preset_face(p))
        };
        filter_sort(
            &self.user_presets,
            &self.insp.presets.contents,
            &faces,
            &self.insp.presets.search,
            self.insp.prefs.presets.sort,
        )
    }

    /// The first ShowBuddy preset the search matches.
    fn first_bank_match(&self) -> Option<(usize, usize)> {
        let q = self.insp.presets.search.to_lowercase();
        for (bi, bank) in self.banks.iter().enumerate() {
            for (pi, p) in bank.presets.iter().enumerate() {
                let hay = format!("{} {}", bank.name, p.name).to_lowercase();
                if q.split_whitespace().all(|t| hay.contains(t)) {
                    return Some((bi, pi));
                }
            }
        }
        None
    }

    /// What is lit, fading or chasing right now.
    fn presets_status(&mut self, ui: &mut egui::Ui) {
        let lit = self.active_user_preset.and_then(|i| self.user_presets.get(i)).map(|p| p.name.clone());
        let bank_lit = self
            .active_preset
            .and_then(|(b, i)| self.banks.get(b).and_then(|bk| bk.presets.get(i)))
            .map(|p| p.name.clone());
        let fading = self.transition_run.is_some();
        let chasing = self.chase.enabled.then(|| self.chase_source_name()).flatten();
        if lit.is_none() && bank_lit.is_none() && !fading && chasing.is_none() {
            return;
        }
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            if let Some(name) = lit.or(bank_lit) {
                theme::pill(ui, &format!("lit: {name}"), theme::ACCENT_SOFT);
            }
            if fading {
                theme::pill(ui, "fading", theme::WARN);
            }
            if let Some(name) = chasing {
                theme::pill(ui, &format!("chase → {name}"), theme::ACCENT_MUTED);
            }
        });
        ui.add_space(2.0);
    }

    /// Name, folder and the Store button.
    fn presets_store_card(&mut self, ui: &mut egui::Ui, acts: &mut Vec<PresetAction>) {
        let has_look = self.programmer_has_look();
        let face = self.programmer_face().unwrap_or_else(|| rgb_of(theme::RAISED));
        let folders = self.preset_folders.clone();
        let lit = self.active_user_preset.filter(|i| *i < self.user_presets.len());
        let mut store = false;
        let mut new_folder_done: Option<Option<String>> = None;
        theme::card(ui, |ui| {
            ui.horizontal(|ui| {
                theme::swatch(ui, Color32::from_rgb(face[0], face[1], face[2]), 18.0)
                    .on_hover_text("The colour the stored preset would wear.");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.preset_name)
                        .id_salt("insp_presets_name")
                        .hint_text("New preset name…")
                        .desired_width(f32::INFINITY),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    store = true;
                }
            });
            ui.add_space(4.0);
            theme::toolbar(ui, |ui| {
                let selected = if self.insp.presets.store_into.is_empty() {
                    "(top level)".to_owned()
                } else {
                    self.insp.presets.store_into.clone()
                };
                egui::ComboBox::from_id_salt("insp_presets_store_folder")
                    .selected_text(selected)
                    .width(field_width(ui, 1, 8.0))
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.insp.presets.store_into.is_empty(), "(top level)")
                            .clicked()
                        {
                            self.insp.presets.store_into.clear();
                        }
                        for f in &folders {
                            if ui
                                .selectable_label(self.insp.presets.store_into == *f, f)
                                .clicked()
                            {
                                self.insp.presets.store_into = f.clone();
                            }
                        }
                        if ui.selectable_label(false, "New folder…").clicked() {
                            self.insp.presets.new_folder = Some(String::new());
                        }
                    })
                    .response
                    .on_hover_text("Where the next stored preset is filed.");
                let gate = match lit {
                    Some(_) => Ok(()),
                    None => Err("Recall a preset first — Update overwrites the lit one."),
                };
                if theme::tool_button(
                    ui,
                    Icon::Save,
                    None,
                    "Update: overwrite the lit preset with the current look, keeping its name, \
                     colour, symbol, artwork, fade, pin and folder.",
                    gate,
                )
                .clicked()
                {
                    if let Some(i) = lit {
                        acts.push(PresetAction::Update(i));
                    }
                }
            });
            if let Some(text) = &mut self.insp.presets.new_folder {
                ui.add_space(4.0);
                let mut done: Option<Option<String>> = None;
                theme::toolbar(ui, |ui| {
                    let w = field_width(ui, 2, 8.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(text)
                            .id_salt("insp_presets_new_folder")
                            .hint_text("Folder name…")
                            .desired_width(w),
                    );
                    // Take focus when the row opens, not every frame: that
                    // would hold it hostage from the rest of the tab.
                    if ui.memory(|m| m.focused().is_none()) {
                        resp.request_focus();
                    }
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if theme::tool_button(ui, Icon::Check, None, "Add this folder.", Ok(())).clicked()
                        || enter
                    {
                        done = Some(Some(text.clone()));
                    }
                    if theme::tool_button(ui, Icon::Close, None, "Never mind.", Ok(())).clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        done = Some(None);
                    }
                });
                new_folder_done = done;
            }
            ui.add_space(6.0);
            let gate = if has_look {
                Ok(())
            } else {
                Err("Set a look first — the programmer is empty.")
            };
            let resp = theme::gated(ui, gate, |ui| {
                theme::wide_button(ui, Some(Icon::Store), "Store look", theme::Tone::Accent)
            });
            if resp
                .on_hover_text(
                    "Save the programmer's current values and running oscillations as a preset, \
                     filed into the chosen folder.",
                )
                .clicked()
            {
                store = true;
            }
        });
        match new_folder_done {
            Some(Some(name)) => {
                self.insp.presets.new_folder = None;
                self.add_preset_folder(&name);
            }
            Some(None) => self.insp.presets.new_folder = None,
            None => {}
        }
        if store && has_look {
            let name = std::mem::take(&mut self.preset_name);
            let folder = self.insp.presets.store_into.clone();
            self.store_look_into(name, folder);
        }
    }

    /// Cut · Fade · As set, and what a click will actually do.
    fn presets_recall_strip(&mut self, ui: &mut egui::Ui) {
        let mode = self.insp.prefs.presets.recall;
        let secs = self.insp.prefs.presets.fade_secs;
        let dur = self
            .transition
            .fade(crate::transition::TransitionTarget::PresetRecall);
        let (label, color) = match mode {
            RecallMode::Cut => ("cut".to_owned(), theme::TEXT_DIM),
            RecallMode::Fade => (format!("{secs:.1} s"), theme::ACCENT_SOFT),
            RecallMode::AsSet if dur <= 0.0 => ("transition: cut".to_owned(), theme::WARN),
            RecallMode::AsSet => (format!("transition {dur:.1} s"), theme::ACCENT_SOFT),
        };
        theme::section_with(ui, "Recall", |ui| {
            theme::pill(ui, &label, color);
        });
        // Words, not icons: "cut" and "as set" have no glyph that reads, and
        // `segmented` keeps labels whenever the row fits.
        let segs = [
            theme::Segment {
                icon: None,
                label: "Cut",
                hint: "Cut — recall instantly, whatever the Transition window says.",
                badge: None,
            },
            theme::Segment {
                icon: None,
                label: "Fade",
                hint: "Fade — recall over the seconds set here.",
                badge: None,
            },
            theme::Segment {
                icon: None,
                label: "As set",
                hint: "As set — use the Transition window's duration.",
                badge: None,
            },
        ];
        let sel = match mode {
            RecallMode::Cut => 0,
            RecallMode::Fade => 1,
            RecallMode::AsSet => 2,
        };
        if let Some(i) = theme::segmented(ui, "insp_presets_recall", &segs, sel) {
            self.insp.prefs.presets.recall = match i {
                0 => RecallMode::Cut,
                1 => RecallMode::Fade,
                _ => RecallMode::AsSet,
            };
            self.insp.mark_dirty();
        }
        if mode == RecallMode::Fade {
            theme::toolbar(ui, |ui| {
                theme::label_dim(ui, "Seconds");
                let mut secs = self.insp.prefs.presets.fade_secs;
                let resp = ui.add(
                    egui::DragValue::new(&mut secs)
                        .speed(0.1)
                        .range(0.1..=20.0)
                        .fixed_decimals(1)
                        .suffix(" s"),
                );
                if resp.on_hover_text("How long a plain click takes to reach the look.").changed() {
                    self.insp.prefs.presets.fade_secs = secs;
                    self.insp.mark_dirty();
                }
            });
        }
        theme::hint(ui, "A preset's own fade wins over this; Shift-click always cuts.");
    }

    /// The pinned strip, the pool and its folders.
    fn presets_pool(&mut self, ui: &mut egui::Ui, acts: &mut Vec<PresetAction>) {
        let z = theme::zoom_of(ui);
        let folders = self.preset_folders.clone();
        let search_on = !self.insp.presets.search.trim().is_empty();
        let order = self.visible_presets();
        let infos: Vec<PadInfo> = order.iter().filter_map(|&i| self.pad_info_at(i)).collect();
        let ctx = PoolCtx {
            scope: "pool",
            folders: &folders,
            pad_size: self.insp.prefs.presets.pad,
            editing: self.insp.presets.edit.as_ref().map(|e| e.id),
            reorder: self.insp.prefs.presets.sort == PresetSort::Manual && !search_on,
            searching: search_on,
            has_selection: !self.stage.selection.is_empty(),
            reveal: self.insp.presets.reveal,
            fade_secs: self.insp.prefs.presets.fade_secs,
        };
        let list = self.insp.prefs.presets.view == PresetView::List;

        // Pinned: pool order, never filtered — it is the operator's own shelf.
        let pinned: Vec<PadInfo> = (0..self.user_presets.len())
            .filter(|&i| self.user_presets[i].pinned)
            .filter_map(|i| self.pad_info_at(i))
            .collect();
        if !pinned.is_empty() {
            let n = pinned.len();
            theme::section_with(ui, "Pinned", |ui| {
                theme::count_pill(ui, n, "preset");
            });
            let pin_ctx = PoolCtx { scope: "pinned", ..ctx };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(5.0 * z);
                for info in &pinned {
                    preset_pad(ui, info, Vec2::new(44.0 * z, 34.0 * z), &pin_ctx, acts);
                }
            });
            ui.add_space(6.0);
        }

        let total = self.user_presets.len();
        theme::section_with(ui, "Presets", |ui| {
            theme::help(
                ui,
                "Click a preset to recall it. Drag one onto a folder to file it, onto the \
                 Presets board to give it a pad, or onto this heading to unfile it.",
            );
            theme::count_pill(ui, total, "preset");
        });
        // While a preset is in the air the heading becomes the "unfile" target.
        if let Some(drag) = egui::DragAndDrop::payload::<PresetDrag>(ui.ctx()) {
            let band = Rect::from_min_size(
                Pos2::new(ui.min_rect().left(), ui.cursor().top() - 26.0 * z),
                Vec2::new(ui.available_width(), 24.0 * z),
            );
            let resp = ui.interact(band, ui.id().with("insp_presets_unfile"), Sense::click());
            if resp.hovered() {
                ui.painter().rect_stroke(
                    band,
                    egui::Rounding::same(5.0),
                    egui::Stroke::new(1.5, theme::ACCENT_SOFT),
                );
            }
            if resp.dnd_release_payload::<PresetDrag>().is_some() {
                acts.push(PresetAction::File(drag.0, String::new()));
            }
        }

        if total == 0 && folders.is_empty() {
            theme::empty_state(
                ui,
                Icon::Presets,
                "No presets yet",
                "Set a look on the stage, then Store.",
            );
            return;
        }
        if infos.is_empty() && search_on {
            theme::empty_state(ui, Icon::Search, "Nothing matches", "Try fewer words, or clear the search.");
        }

        if search_on {
            // One flat list, folders folded away: the search is the grouping.
            let all: Vec<&PadInfo> = infos.iter().collect();
            if list {
                for info in &all {
                    preset_row(ui, info, &ctx, 0.0, acts);
                }
            } else {
                preset_pads(ui, &all, &ctx, 0.0, acts);
            }
            return;
        }

        let top: Vec<&PadInfo> = infos.iter().filter(|i| i.folder.is_empty()).collect();
        if list {
            for info in &top {
                preset_row(ui, info, &ctx, 0.0, acts);
            }
        } else {
            preset_pads(ui, &top, &ctx, 0.0, acts);
        }

        let mut rename = self.insp.presets.rename_folder.take();
        let focus_rename = std::mem::take(&mut self.insp.presets.rename_folder_focus);
        for (fi, folder) in folders.iter().enumerate() {
            let members: Vec<&PadInfo> = infos.iter().filter(|i| i.folder == *folder).collect();
            let renaming = rename.as_ref().is_some_and(|(old, _)| old == folder);
            if renaming {
                let (old, text) = rename.as_mut().unwrap();
                let old = old.clone();
                let mut done = false;
                theme::toolbar(ui, |ui| {
                    let w = field_width(ui, 2, 8.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(text)
                            .id_salt(("insp_presets_folder_rename", old.as_str()))
                            .desired_width(w),
                    );
                    if focus_rename {
                        resp.request_focus();
                    }
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if theme::tool_button(ui, Icon::Check, None, "Rename this folder.", Ok(()))
                        .clicked()
                        || enter
                    {
                        acts.push(PresetAction::RenameFolder(old.clone(), text.clone()));
                        done = true;
                    }
                    if theme::tool_button(ui, Icon::Close, None, "Keep the old name.", Ok(()))
                        .clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        done = true;
                    }
                });
                if done {
                    rename = None;
                }
            } else {
                let info = FolderInfo {
                    fi,
                    name: folder.clone(),
                    faces: members.iter().map(|m| m.face).collect(),
                    first: members.first().map(|m| m.idx),
                    open: self.insp.is_open(&folder_key(folder), true),
                };
                let opened = self.folder_header(ui, &info, acts);
                if opened {
                    if list {
                        for m in &members {
                            preset_row(ui, m, &ctx, 14.0 * z, acts);
                        }
                    } else {
                        preset_pads(ui, &members, &ctx, 10.0 * z, acts);
                    }
                    ui.add_space(4.0);
                }
            }
        }
        self.insp.presets.rename_folder = rename;
    }

    /// One folder's header row; returns whether its block is open.
    fn folder_header(
        &mut self,
        ui: &mut egui::Ui,
        f: &FolderInfo,
        acts: &mut Vec<PresetAction>,
    ) -> bool {
        let z = theme::zoom_of(ui);
        let h = ui.spacing().interact_size.y + 4.0 * z;
        let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), h));
        let resp = ui.interact(
            rect,
            ui.id().with(("insp_presets_folder", f.name.as_str())),
            Sense::click_and_drag(),
        );
        let tint = folder_tint(&f.faces);
        if ui.is_rect_visible(rect) {
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(rect, egui::Rounding::same(4.0), theme::HOVER);
            }
            let cs = 12.0 * z;
            let cr = Rect::from_center_size(
                Pos2::new(rect.left() + 4.0 * z + cs * 0.5, rect.center().y),
                Vec2::splat(cs),
            );
            icons::draw(
                p,
                cr,
                if f.open { Icon::ChevronDown } else { Icon::ChevronRight },
                theme::TEXT_DIM,
            );
            let fs = 14.0 * z;
            let fr = Rect::from_center_size(
                Pos2::new(cr.right() + 4.0 * z + fs * 0.5, rect.center().y),
                Vec2::splat(fs),
            );
            icons::draw(p, fr, Icon::Folder, tint);
            p.text(
                Pos2::new(fr.right() + 6.0 * z, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &f.name,
                egui::FontId::new(13.0 * z, theme::semibold()),
                theme::TEXT,
            );
        }
        let count = f.faces.len();
        let store_gate = if self.programmer_has_look() {
            Ok(())
        } else {
            Err("Set a look first — the programmer is empty.")
        };
        let store = ui
            .allocate_new_ui(
                egui::UiBuilder::new()
                    .max_rect(rect.shrink2(Vec2::new(4.0 * z, 2.0)))
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0 * z;
                    let hit = theme::tool_button(
                        ui,
                        Icon::Store,
                        None,
                        "Store the current look straight into this folder.",
                        store_gate,
                    )
                    .clicked();
                    theme::pill(ui, &count.to_string(), theme::TEXT_DIM);
                    if !f.open {
                        // A closed folder still shows what colours it holds.
                        for face in f.faces.iter().take(8) {
                            let (r, _) =
                                ui.allocate_exact_size(Vec2::splat(6.0 * z), Sense::hover());
                            theme::paint_swatch(
                                ui.painter(),
                                r,
                                Color32::from_rgb(face[0], face[1], face[2]),
                                2.0 * z,
                            );
                        }
                    }
                    hit
                },
            )
            .inner;
        ui.allocate_rect(rect, Sense::hover());
        if store {
            acts.push(PresetAction::StoreInto(f.name.clone()));
        }
        // A preset dropped here is filed; a folder dropped here is reordered.
        if let Some(src) = resp.dnd_release_payload::<PresetDrag>() {
            acts.push(PresetAction::File(src.0, f.name.clone()));
        } else if let Some(src) = resp.dnd_release_payload::<FolderDrag>() {
            if let Some(from) = self.preset_folders.iter().position(|x| *x == src.0) {
                acts.push(PresetAction::MoveFolder(from, f.fi));
            }
        } else if resp.dnd_hover_payload::<PresetDrag>().is_some() {
            ui.painter().rect_stroke(
                rect,
                egui::Rounding::same(5.0),
                egui::Stroke::new(1.5, theme::ACCENT_SOFT),
            );
        } else if resp.dnd_hover_payload::<FolderDrag>().is_some() {
            ui.painter().hline(
                rect.x_range(),
                rect.top(),
                egui::Stroke::new(2.0, theme::ACCENT_SOFT),
            );
        }
        if resp.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), FolderDrag(f.name.clone()));
        }
        if resp.clicked() {
            acts.push(PresetAction::ToggleFolder(f.name.clone()));
        }
        let first = f.first;
        let name = f.name.clone();
        let fi = f.fi;
        resp.on_hover_text(format!(
            "Folder \"{name}\" — {count} preset{}. Click to open or close it, drag it to \
             reorder, drop a preset on it to file it.",
            if count == 1 { "" } else { "s" }
        ))
        .context_menu(|ui| {
            if ui.button("Rename folder…").clicked() {
                acts.push(PresetAction::RenameFolderStart(name.clone()));
                ui.close_menu();
            }
            if ui.button("Store into this folder").clicked() {
                acts.push(PresetAction::StoreInto(name.clone()));
                ui.close_menu();
            }
            if ui.button("Fill a board page with this folder").clicked() {
                acts.push(PresetAction::FillPage(name.clone()));
                ui.close_menu();
            }
            if ui
                .add_enabled(first.is_some(), egui::Button::new("Recall first preset"))
                .clicked()
            {
                if let Some(idx) = first {
                    acts.push(PresetAction::Recall { idx, fade: None });
                }
                ui.close_menu();
            }
            ui.separator();
            if theme::danger_button(ui, "Delete folder (presets are kept)").clicked() {
                acts.push(PresetAction::DeleteFolder(fi));
                ui.close_menu();
            }
        });
        f.open
    }

    /// The inline Edit pad card.
    fn presets_edit_card(&mut self, ui: &mut egui::Ui) {
        let Some(mut e) = self.insp.presets.edit.take() else { return };
        let Some(idx) = self.preset_index_by_id(e.id) else { return };
        let z = theme::zoom_of(ui);
        let derived_face = self
            .insp
            .presets
            .swatches
            .get(&e.id)
            .map_or_else(|| rgb_of(theme::RAISED), |s| s.rgb);
        let derived_symbol = pad_symbol(&self.user_presets[idx].name);
        let mut save = false;
        let mut cancel = false;
        ui.add_space(4.0);
        theme::card(ui, |ui| {
            let face = if e.auto_color { derived_face } else { e.color };
            let symbol =
                if e.symbol.trim().is_empty() { derived_symbol.clone() } else { e.symbol.clone() };
            theme::card_title(ui, "Edit pad", |ui| {
                let (r, _) =
                    ui.allocate_exact_size(Vec2::new(46.0 * z, 32.0 * z), Sense::hover());
                theme::paint_pad(
                    ui.painter(),
                    r,
                    theme::PadState {
                        fill: Some(Color32::from_rgb(face[0], face[1], face[2])),
                        ..theme::PadState::default()
                    },
                    theme::PadFace::Symbol(&symbol),
                    "",
                );
            });
            // The name takes the whole card: inside a `kv_grid` cell it
            // would have to fit a 100 px value column.
            let resp = ui.add(
                egui::TextEdit::singleline(&mut e.name)
                    .id_salt("insp_presets_edit_name")
                    .hint_text("Preset name…")
                    .desired_width(f32::INFINITY),
            );
            if e.focus_pending {
                resp.request_focus();
                e.focus_pending = false;
            }
            ui.add_space(4.0);
            theme::kv_grid(ui, "insp_presets_edit", |ui| {
                theme::label_dim(ui, "Colour");
                ui.horizontal(|ui| {
                    // The label column is 64 px and the card has 10 px
                    // margins, so this column has to stay under ~100 px.
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.checkbox(&mut e.auto_color, "auto")
                        .on_hover_text("Take the colour from the preset's own values.");
                    // `add_enabled_ui` builds a child `Ui`, which cannot wrap
                    // and claims the row; the button simply goes away instead.
                    if !e.auto_color {
                        ui.color_edit_button_srgb(&mut e.color)
                            .on_hover_text("Pick the pad's colour by hand.");
                    }
                });
                ui.end_row();

                theme::label_dim(ui, "Symbol");
                ui.add(
                    egui::TextEdit::singleline(&mut e.symbol)
                        .id_salt("insp_presets_edit_symbol")
                        .char_limit(4)
                        .desired_width(52.0 * z)
                        .hint_text(derived_symbol.as_str()),
                )
                .on_hover_text("Up to four characters, shown large on the pad and the deck key.");
                ui.end_row();

                theme::label_dim(ui, "Artwork");
                // `kv_grid`'s label column is 64 px, so the value column has
                // to stay under ~110 px or the whole Inspector widens.
                // Without `truncate` the button grows to fit its longest
                // label, and a `kv_grid` value column that wide widens the
                // whole Inspector.
                egui::ComboBox::from_id_salt("insp_presets_edit_icon")
                    .selected_text(icon_short(e.icon))
                    .truncate()
                    .width(70.0 * z)
                    .show_ui(ui, |ui| {
                        for icon in KeyIcon::ALL {
                            ui.selectable_value(&mut e.icon, icon, icon.label());
                        }
                    })
                    .response
                    .on_hover_text("Artwork drawn behind the symbol on the deck key.");
                ui.end_row();

                theme::label_dim(ui, "Fade");
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.checkbox(&mut e.own_fade, "own")
                        .on_hover_text("Give this preset a recall time of its own.");
                    ui.add_enabled(
                        e.own_fade,
                        egui::DragValue::new(&mut e.fade_secs)
                            .speed(0.1)
                            .range(0.0..=20.0)
                            .fixed_decimals(1)
                            .suffix("s"),
                    )
                    .on_hover_text("Seconds this preset takes to arrive, whatever the strip says.");
                });
                ui.end_row();

                theme::label_dim(ui, "Pinned");
                ui.checkbox(&mut e.pinned, "")
                    .on_hover_text("Keep this preset in the strip at the top of the tab.");
                ui.end_row();
            });
            ui.add_space(4.0);
            theme::toolbar(ui, |ui| {
                if theme::accent_button(ui, "Save")
                    .on_hover_text("Write these changes to the preset.")
                    .clicked()
                {
                    save = true;
                }
                if ui
                    .button("Cancel")
                    .on_hover_text("Close the card and change nothing.")
                    .clicked()
                {
                    cancel = true;
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });
        if save {
            self.apply_preset_edit(&e);
        }
        if !save && !cancel {
            self.insp.presets.edit = Some(e);
        }
    }

    /// The last few looks recalled this session.
    fn presets_recent(&mut self, ui: &mut egui::Ui, acts: &mut Vec<PresetAction>) {
        let ids: Vec<u32> = self.insp.presets.recent.iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        let infos: Vec<PadInfo> = ids
            .iter()
            .filter_map(|&id| self.preset_index_by_id(id))
            .filter_map(|idx| self.pad_info_at(idx))
            .collect();
        if infos.is_empty() {
            return;
        }
        let z = theme::zoom_of(ui);
        let folders = self.preset_folders.clone();
        let ctx = PoolCtx {
            scope: "recent",
            folders: &folders,
            pad_size: PadSize::S,
            editing: self.insp.presets.edit.as_ref().map(|e| e.id),
            reorder: false,
            searching: false,
            has_selection: !self.stage.selection.is_empty(),
            reveal: None,
            fade_secs: self.insp.prefs.presets.fade_secs,
        };
        theme::section(ui, "Recent");
        egui::ScrollArea::horizontal().id_salt("insp_presets_recent").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0 * z;
                for info in &infos {
                    preset_pad(ui, info, Vec2::new(44.0 * z, 34.0 * z), &ctx, acts);
                }
            });
        });
        ui.add_space(4.0);
    }

    /// The ShowBuddy banks, as pads.
    fn presets_banks(&mut self, ui: &mut egui::Ui) {
        let n = self.banks.len();
        let badge = format!("{n} bank{}", if n == 1 { "" } else { "s" });
        self.insp_fold(
            ui,
            "presets.showbuddy",
            "ShowBuddy banks",
            false,
            (n > 0).then_some((badge.as_str(), theme::TEXT_DIM)),
            |app, ui| app.showbuddy_body(ui),
        );
    }

    fn showbuddy_body(&mut self, ui: &mut egui::Ui) {
        if self.banks.is_empty() {
            theme::pill(ui, "ShowBuddy off", theme::TEXT_DIM);
            theme::hint(ui, "Turn the ShowBuddy patch on in Patch to see its banks.");
            return;
        }
        let z = theme::zoom_of(ui);
        let mut acts: Vec<PresetAction> = Vec::new();
        let mut toggle: Option<usize> = None;
        let banks: Vec<(String, usize)> =
            self.banks.iter().map(|b| (b.name.clone(), b.presets.len())).collect();
        for (bi, (name, count)) in banks.iter().enumerate() {
            let open = self.open_bank == Some(bi);
            let count_text = count.to_string();
            let row = theme::Row {
                swatch: None,
                icon: Some(Icon::Bank),
                title: name,
                subtitle: None,
                badges: &[(count_text.as_str(), theme::TEXT_DIM)],
                selected: open,
                active: self.active_preset.is_some_and(|(b, _)| b == bi),
                indent: 0.0,
            };
            let resp = theme::list_row(ui, ("insp_presets_bank", bi), &row);
            if resp.clicked() {
                toggle = Some(bi);
            }
            resp.on_hover_text(format!(
                "Bank \"{name}\" — {count} ShowBuddy presets. Click to open it."
            ))
            .context_menu(|ui| {
                if ui.button(format!("Import whole bank into folder \"{name}\"")).clicked() {
                    acts.push(PresetAction::ImportWholeBank(bi));
                    ui.close_menu();
                }
            });
            if !open {
                continue;
            }
            // (index, name, face, symbol, lit) — read before drawing.
            let pads: Vec<(usize, String, [u8; 3], String, bool)> = self.banks[bi]
                .presets
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    let (_, label, _) = preset_key_look(&p.name);
                    (
                        pi,
                        p.name.clone(),
                        self.bank_face(bi, pi),
                        label,
                        self.active_preset == Some((bi, pi)),
                    )
                })
                .collect();
            let gap = 5.0 * z;
            let avail = ui.available_width() - 8.0 * z;
            let cols = theme::pad_columns(avail, 52.0 * z, gap);
            let w = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).max(24.0);
            let h = (w * 0.74).clamp(32.0 * z, 80.0 * z);
            for chunk in pads.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.add_space(8.0 * z);
                    ui.spacing_mut().item_spacing.x = gap;
                    for (pi, name, face, symbol, lit) in chunk {
                        let resp = theme::pad_button(
                            ui,
                            ("insp_presets_bankpad", bi, *pi),
                            Vec2::new(w, h),
                            theme::PadState {
                                fill: Some(Color32::from_rgb(face[0], face[1], face[2])),
                                active: *lit,
                                dim: !*lit,
                                ..theme::PadState::default()
                            },
                            theme::PadFace::Symbol(symbol),
                            name,
                        );
                        let pi = *pi;
                        if resp.clicked() {
                            let cut = ui.input(|i| i.modifiers.shift);
                            acts.push(PresetAction::RecallBank(bi, pi, cut.then_some(0.0)));
                        }
                        resp.on_hover_text(format!(
                            "\"{name}\" from ShowBuddy — click to recall, Shift-click to cut, \
                             right-click for more."
                        ))
                        .context_menu(|ui| {
                            if ui.button("Recall").clicked() {
                                acts.push(PresetAction::RecallBank(bi, pi, None));
                                ui.close_menu();
                            }
                            if ui.button("Cut to (no fade)").clicked() {
                                acts.push(PresetAction::RecallBank(bi, pi, Some(0.0)));
                                ui.close_menu();
                            }
                            if ui.button("To board").clicked() {
                                acts.push(PresetAction::BankToBoard(bi, pi));
                                ui.close_menu();
                            }
                            if ui.button("Set as chase source").clicked() {
                                acts.push(PresetAction::BankChaseSource(bi, pi));
                                ui.close_menu();
                            }
                            if ui.button("Import as native preset").clicked() {
                                acts.push(PresetAction::ImportBank(bi, pi));
                                ui.close_menu();
                            }
                        });
                    }
                });
                ui.add_space(gap);
            }
        }
        if let Some(bi) = toggle {
            self.open_bank = if self.open_bank == Some(bi) { None } else { Some(bi) };
        }
        self.apply_preset_actions(acts);
    }

    /// Blackout, the staged clear and the release.
    fn presets_output(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Output");
        if theme::wide_button(ui, Some(Icon::Blackout), "Blackout", theme::Tone::Danger)
            .on_hover_text(
                "Every channel to zero, every effect stopped — the programmer keeps nothing.",
            )
            .clicked()
        {
            self.inspector_blackout();
        }
        ui.add_space(4.0);
        let lit = self
            .active_user_preset
            .and_then(|i| self.user_presets.get(i))
            .map(|p| p.name.clone())
            .or_else(|| {
                self.active_preset
                    .and_then(|(b, i)| self.banks.get(b).and_then(|bk| bk.presets.get(i)))
                    .map(|p| p.name.clone())
            });
        let mut clear = false;
        let mut release = false;
        theme::toolbar(ui, |ui| {
            clear = theme::tool_button(
                ui,
                Icon::Reset,
                Some("Clear"),
                "Staged clear: encoders first, then effects, then blackout — one stage per press.",
                Ok(()),
            )
            .clicked();
            let gate = if lit.is_some() {
                Ok(())
            } else {
                Err("Nothing is lit — there is no highlight to drop.")
            };
            release = theme::tool_button(
                ui,
                Icon::Stop,
                Some("Release"),
                "Drop the lit-preset highlight without changing the output.",
                gate,
            )
            .clicked();
        });
        theme::hint(
            ui,
            match &lit {
                Some(name) => format!("Active: {name}"),
                None => "No preset active".to_owned(),
            },
        );
        if clear {
            self.clear_stage();
        }
        if release {
            self.active_preset = None;
            self.active_user_preset = None;
            self.log.push("Preset released".into());
        }
    }

    /// Everything a click asked for, applied once the loops are done.
    fn apply_preset_actions(&mut self, acts: Vec<PresetAction>) {
        for act in acts {
            match act {
                PresetAction::Recall { idx, fade } => self.recall_user_preset(idx, fade),
                PresetAction::RecallMasked(idx) => self.apply_user_preset_masked(idx),
                PresetAction::Edit(id) => {
                    if let Some(idx) = self.preset_index_by_id(id) {
                        let face = self.preset_face(&self.user_presets[idx]);
                        let secs = self.insp.prefs.presets.fade_secs;
                        let p = &self.user_presets[idx];
                        self.insp.presets.edit = Some(PresetEdit {
                            id,
                            name: p.name.clone(),
                            auto_color: p.color.is_none(),
                            color: p.color.unwrap_or(face),
                            symbol: p.symbol.clone(),
                            icon: p.icon,
                            own_fade: p.fade.is_some(),
                            fade_secs: p.fade.unwrap_or(secs),
                            pinned: p.pinned,
                            focus_pending: true,
                        });
                    }
                }
                PresetAction::Update(idx) => {
                    self.update_preset_in_place(idx);
                }
                PresetAction::Pin(idx) => self.toggle_pin(idx),
                PresetAction::MoveBefore(src_id, dst) => {
                    if let Some(src) = self.preset_index_by_id(src_id) {
                        self.move_preset_before(src, dst);
                    }
                }
                PresetAction::ToBoard(idx) => self.preset_board_add(idx),
                PresetAction::ChaseSource(idx) => {
                    if let Some(p) = self.user_presets.get(idx) {
                        let name = p.name.clone();
                        self.chase.source = Some(ChaseSource::User(idx));
                        self.log.push(format!("Chase injects \"{name}\""));
                        if self.chase.enabled {
                            self.start_chase();
                        }
                    }
                }
                PresetAction::Duplicate(idx) => {
                    self.duplicate_preset(idx);
                }
                PresetAction::Delete(idx) => self.delete_preset(idx),
                PresetAction::File(id, folder) => {
                    if let Some(idx) = self.preset_index_by_id(id) {
                        self.set_preset_folder(idx, &folder);
                    }
                }
                PresetAction::ToggleFolder(name) => {
                    let key = folder_key(&name);
                    let open = self.insp.is_open(&key, true);
                    self.insp.set_open(&key, true, !open);
                }
                PresetAction::RenameFolderStart(name) => {
                    self.insp.presets.rename_folder = Some((name.clone(), name));
                    self.insp.presets.rename_folder_focus = true;
                }
                PresetAction::RenameFolder(old, new) => {
                    self.rename_preset_folder(&old, &new);
                }
                PresetAction::DeleteFolder(fi) => self.delete_preset_folder(fi),
                PresetAction::MoveFolder(src, dst) => self.move_preset_folder(src, dst),
                PresetAction::StoreInto(folder) => {
                    if self.programmer_has_look() {
                        let name = std::mem::take(&mut self.preset_name);
                        self.store_look_into(name, folder);
                    }
                }
                PresetAction::FillPage(folder) => {
                    self.fill_preset_page_with_folder(&folder);
                    self.show_preset_board = true;
                }
                PresetAction::RecallBank(bi, pi, fade) => self.recall_bank_preset(bi, pi, fade),
                PresetAction::ImportBank(bi, pi) => {
                    let folder = self.banks.get(bi).map(|b| b.name.clone()).unwrap_or_default();
                    match self.import_bank_preset(bi, pi, &folder) {
                        Some(idx) => {
                            let name = self.user_presets[idx].name.clone();
                            self.log.push(format!("Imported \"{name}\" as a native preset"));
                        }
                        None => self.log.push("That ShowBuddy preset would not load".into()),
                    }
                }
                PresetAction::ImportWholeBank(bi) => self.import_bank(bi),
                PresetAction::BankToBoard(bi, pi) => self.preset_board_add_bank(bi, pi),
                PresetAction::BankChaseSource(bi, pi) => {
                    let name = self
                        .banks
                        .get(bi)
                        .and_then(|b| b.presets.get(pi))
                        .map(|p| p.name.clone());
                    if let Some(name) = name {
                        self.chase.source = Some(ChaseSource::Bank(bi, pi));
                        self.log.push(format!("Chase injects \"{name}\""));
                        if self.chase.enabled {
                            self.start_chase();
                        }
                    }
                }
            }
        }
    }
}

/// Six presets, two folders, a board page — enough of a pool to look at.
/// Pushed straight into memory: nothing here writes a file.
#[cfg(test)]
pub(crate) fn seed_demo_presets(app: &mut App) {
    use crate::preset_deck::{PresetSlot, PRESET_DECK_SLOTS};

    /// The first `n` fixtures' emitter channels, set to `rgb` (and a
    /// dimmer when asked). Empty when nothing is patched.
    fn colour(app: &App, n: usize, rgb: [u8; 3], dim: Option<u8>) -> Vec<(usize, u8)> {
        let mut out = Vec::new();
        for f in app.patch.fixtures.iter().take(n) {
            let from0 = f.from as usize - 1;
            for (ci, ch) in f.channels.iter().enumerate() {
                let v = match ch.role() {
                    Role::Red => rgb[0],
                    Role::Green => rgb[1],
                    Role::Blue => rgb[2],
                    Role::Dimmer => dim.unwrap_or(0),
                    _ => 0,
                };
                if v > 0 {
                    out.push((from0 + ci, v));
                }
            }
        }
        out
    }
    fn moves(app: &App, n: usize) -> Vec<(usize, u8)> {
        let mut out = Vec::new();
        for f in app.patch.fixtures.iter().take(n) {
            let from0 = f.from as usize - 1;
            for (ci, ch) in f.channels.iter().enumerate() {
                if matches!(ch.role(), Role::Pan | Role::Tilt) {
                    out.push((from0 + ci, 128));
                }
            }
        }
        out
    }

    let blank = |app: &mut App, name: &str, folder: &str, values: Vec<(usize, u8)>| {
        let id = app.next_preset_id;
        app.next_preset_id += 1;
        UserPreset {
            id,
            name: name.to_owned(),
            folder: folder.to_owned(),
            values,
            oscs: Vec::new(),
            speed: 0.357,
            tempo: 120.0,
            master_speed: 1.0,
                                color: None,
            symbol: String::new(),
            icon: KeyIcon::None,
            fade: None,
            pinned: false,
        }
    };

    app.user_presets.clear();
    app.preset_folders = vec!["Verse".into(), "Chorus".into()];
    app.next_preset_id = 1;

    let warm = colour(app, 6, [255, 120, 0], Some(220));
    let p = blank(app, "Warm wash", "", warm);
    app.user_presets.push(p);

    let cold = colour(app, 6, [0, 120, 255], Some(255));
    let mut p = blank(app, "Cold blue", "", cold);
    p.fade = Some(1.5);
    app.user_presets.push(p);

    let mut p = blank(app, "Pulse", "", colour(app, 3, [255, 255, 255], Some(180)));
    p.oscs = Vec::new();
    p.pinned = true;
    app.user_presets.push(p);

    let mut p = blank(app, "Dim only", "Verse", colour(app, 4, [0, 0, 0], Some(128)));
    p.pinned = true;
    app.user_presets.push(p);

    let mut p = blank(app, "Chorus A", "Chorus", colour(app, 5, [200, 40, 160], Some(255)));
    p.color = Some([200, 40, 160]);
    p.symbol = "CHOR".into();
    p.pinned = true;
    app.user_presets.push(p);

    let p = blank(app, "Chorus B", "Chorus", moves(app, 4));
    app.user_presets.push(p);

    // A stand-in ShowBuddy bank, so the bank block and a bank pad are drawn
    // on machines with no ShowBuddy install.
    app.banks = vec![crate::showbuddy::PresetBank {
        name: "General 100".into(),
        order: 0,
        presets: ["Red 100", "Blue 100", "Green 50", "Rainbow sweep"]
            .into_iter()
            .map(|name| crate::showbuddy::PresetRef {
                name: name.into(),
                path: std::path::PathBuf::new(),
                data: None,
            })
            .collect(),
    }];
    app.open_bank = Some(0);

    app.active_user_preset = Some(0);
    app.insp.presets.recent = [1u32, 2, 5].into_iter().collect();

    app.preset_deck = vec![None; PRESET_DECK_SLOTS];
    for (k, id) in [1u32, 2, 4, 5].into_iter().enumerate() {
        let idx = (id - 1) as usize;
        app.preset_deck[k] = app.preset_slot_for_user(idx);
    }
    app.preset_deck[2] = Some(PresetSlot {
        preset: 0,
        bank: Some(("General 100".into(), "Red 100".into())),
        name: "Red 100".into(),
        color: [255, 60, 50],
        label: "RED".into(),
        icon: KeyIcon::None,
    });
    app.preset_deck[4] = Some(PresetSlot {
        preset: 99,
        bank: None,
        name: "Gone away".into(),
        color: [120, 200, 90],
        label: "GONE".into(),
        icon: KeyIcon::None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioTrigger;
    use crate::net::DMX_SLOTS;
    use crate::preset_deck::PRESET_DECK_SLOTS;
    use crate::stage::headless::{render_frames, save};
    use crate::ui::inspector::InspectorTab;

    /// State files the App-level tests touch, put back when the test ends.
    struct Files(Vec<(&'static str, Option<Vec<u8>>)>);

    impl Files {
        fn hold(names: &[&'static str]) -> Self {
            Self(names.iter().map(|n| (*n, std::fs::read(n).ok())).collect())
        }
    }

    impl Drop for Files {
        fn drop(&mut self) {
            for (name, bytes) in &self.0 {
                match bytes {
                    Some(b) => {
                        let _ = std::fs::write(name, b);
                    }
                    None => {
                        let _ = std::fs::remove_file(name);
                    }
                }
            }
        }
    }

    /// A fake eight-channel fixture: dimmer, RGBW, amber, pan, tilt.
    fn roles(a: usize) -> Option<Role> {
        [
            Role::Dimmer,
            Role::Red,
            Role::Green,
            Role::Blue,
            Role::White,
            Role::Amber,
            Role::Pan,
            Role::Tilt,
        ]
        .get(a)
        .copied()
    }

    /// The same, in two fixtures of eight, with a gobo in the second.
    fn roles2(a: usize) -> Option<(usize, Role)> {
        let table = [
            Role::Dimmer,
            Role::Red,
            Role::Green,
            Role::Blue,
            Role::White,
            Role::Amber,
            Role::Pan,
            Role::Tilt,
        ];
        match a {
            0..=7 => Some((0, table[a])),
            8..=15 => Some((1, if a == 15 { Role::Gobo } else { table[a - 8] })),
            _ => None,
        }
    }

    fn preset(id: u32, name: &str, folder: &str) -> UserPreset {
        UserPreset {
            id,
            name: name.to_owned(),
            folder: folder.to_owned(),
            values: vec![(0, 200)],
            oscs: Vec::new(),
            speed: 0.3,
            tempo: 120.0,
            master_speed: 1.0,
                                color: None,
            symbol: String::new(),
            icon: KeyIcon::None,
            fade: None,
            pinned: false,
        }
    }

    /// "Rename folder…" used to write `insp.presets.rename_folder` straight
    /// through `self` from inside `presets_pool`'s folder loop, and the
    /// unconditional write-back after the loop put the stale local (`None`)
    /// back over it the same frame — so the inline field never appeared and
    /// a folder could not be renamed from its menu at all.
    #[test]
    fn rename_folder_from_the_menu_opens_the_inline_field() {
        let _files = Files::hold(&["presets.json"]);
        let mut app = App::new();
        app.preset_folders = vec!["Chorus".into()];
        app.user_presets = vec![preset(1, "Chorus A", "Chorus")];
        app.next_preset_id = 2;
        app.insp.set_open(&folder_key("Chorus"), true, false);
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let click = |pos: egui::Pos2, button: egui::PointerButton, pressed: bool| {
            egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE }
        };
        let mut pool_id = egui::Id::NULL;
        let mut header_gone = false;
        let mut header = egui::Pos2::ZERO;
        let mut item = egui::Pos2::ZERO;
        // A press and its release have to land on separate frames: egui only
        // takes a widget as a click candidate while the button is still down.
        for frame in 0..10 {
            let events = match frame {
                // Right-click the folder header to raise its menu…
                3 => vec![
                    egui::Event::PointerMoved(header),
                    click(header, egui::PointerButton::Secondary, true),
                ],
                4 => vec![click(header, egui::PointerButton::Secondary, false)],
                // …then press its first item, "Rename folder…".
                6 => vec![
                    egui::Event::PointerMoved(item),
                    click(item, egui::PointerButton::Primary, true),
                ],
                7 => vec![click(item, egui::PointerButton::Primary, false)],
                _ => Vec::new(),
            };
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(320.0, 700.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut acts: Vec<PresetAction> = Vec::new();
                    ui.push_id("pool", |ui| {
                        pool_id = ui.id();
                        app.presets_pool(ui, &mut acts);
                    });
                    // The tab applies the deferred actions after the pool.
                    app.apply_preset_actions(acts);
                });
            });
            let header_id = pool_id.with(("insp_presets_folder", "Chorus"));
            if let Some(r) = ctx.read_response(header_id) {
                header = r.rect.center();
            }
            // egui parks a context menu in an Area named after its button.
            if let Some(a) = egui::AreaState::load(&ctx, header_id.with("__menu")) {
                // Just inside the menu's first item.
                item = a.rect().left_top() + Vec2::new(30.0, 14.0);
            }
            header_gone = ctx.read_response(header_id).is_none();
        }
        assert_eq!(
            app.insp.presets.rename_folder,
            Some(("Chorus".to_owned(), "Chorus".to_owned())),
            "the rename never started"
        );
        // The header has given way to the inline field, which is the whole
        // point of the menu item.
        assert!(header_gone, "the folder still draws its header, not a rename field");
    }

    /// The swatch and contents caches are keyed by preset id, and
    /// `cache_key` only notices the *rig* changing shape. Nothing cleared
    /// them when the pool itself was swapped out, so undo of a preset
    /// Update put the old values back while the pad kept the new colour —
    /// for ever, and a fresh show handed id 1 the last show's swatch.
    #[test]
    fn replacing_the_pool_forgets_the_id_keyed_preset_caches() {
        let _files = Files::hold(&[
            "presets.json",
            "preset_deck.json",
            "groups.json",
            "orders.json",
            "palettes.json",
            "phasers.json",
            "stacks.json",
            "scenes.json",
            "views.json",
            "cameras.json",
        ]);
        let mut app = App::new();
        app.user_presets = vec![preset(1, "Look", "")];
        app.next_preset_id = 2;
        app.refresh_preset_caches();
        assert!(app.insp.presets.swatches.contains_key(&1));
        app.insp.presets.recent.push_back(1);
        // Ctrl+Shift+Z, a backup restore and loading a configuration all
        // come through `apply_configuration`.
        app.undo.observe(app.snapshot_json());
        app.user_presets[0].values = vec![(1, 255)];
        assert!(app.undo.observe(app.snapshot_json()), "the change was noticed");
        app.undo();
        assert_eq!(app.user_presets[0].values, vec![(0, 200)]);
        assert!(app.insp.presets.swatches.is_empty(), "the swatch outlived the pool");
        assert!(app.insp.presets.contents.is_empty());
        assert!(app.insp.presets.recent.is_empty());
        // File → New show resets `next_preset_id` to 1, so the first preset
        // of the fresh show would inherit the old one's swatch and badges.
        app.refresh_preset_caches();
        assert!(!app.insp.presets.swatches.is_empty());
        app.new_show(false, false);
        assert!(app.insp.presets.swatches.is_empty());
        assert!(app.insp.presets.contents.is_empty());
    }

    #[test]
    fn swatch_mixes_rgbw_and_tints() {
        let s = swatch_from_values(&[(1, 255), (4, 128)], &roles);
        assert_eq!((s.rgb, s.kind), ([255, 128, 128], SwatchKind::Colour));

        let s = swatch_from_values(&[(5, 255)], &roles);
        assert_eq!((s.rgb, s.kind), ([255, 180, 60], SwatchKind::Colour));

        let s = swatch_from_values(&[(0, 64)], &roles);
        assert_eq!(s.kind, SwatchKind::Dimmer);
        let amber = role_color(Role::Dimmer);
        assert_eq!(
            s.rgb,
            [amber.r(), amber.g(), amber.b()].map(|c| (c as f32 * 0.35).round() as u8)
        );

        let s = swatch_from_values(&[(6, 200), (7, 40)], &roles);
        assert_eq!((s.rgb, s.kind), ([40, 80, 110], SwatchKind::Position));

        let s = swatch_from_values(&[], &roles);
        assert_eq!((s.rgb, s.kind), (rgb_of(theme::RAISED), SwatchKind::Empty));

        // A dimmer never dims a colour below a third, so a low look still reads.
        let s = swatch_from_values(&[(1, 255), (0, 51)], &roles);
        assert_eq!(s.rgb, [89, 0, 0]);
        let s = swatch_from_values(&[(1, 255), (0, 255)], &roles);
        assert_eq!(s.rgb, [255, 0, 0]);
    }

    #[test]
    fn contents_read_roles_and_the_record() {
        let mut p = preset(1, "x", "");
        let c = contents_from_values(&[(6, 128), (7, 128)], &roles2, &p);
        assert!(c.position && !c.colour && !c.beam);
        assert_eq!(c.fixtures, 1);

        let c = contents_from_values(&[(15, 40)], &roles2, &p);
        assert!(c.beam && !c.position);

        // Two values in one fixture count as one light; two fixtures as two.
        let c = contents_from_values(&[(1, 10), (2, 10)], &roles2, &p);
        assert_eq!((c.fixtures, c.colour), (1, true));
        let c = contents_from_values(&[(1, 10), (9, 10)], &roles2, &p);
        assert_eq!(c.fixtures, 2);

        let blank = contents_from_values(&[], &roles2, &p);
        assert_eq!(blank, PresetContents::default());
        p.oscs = vec![(
            3,
            SavedOsc {
                invert: false,
                amount: 1.0,
                phase: 0.0,
                subdiv: None,
                shape: 0.5,
                custom_wave: None,
                master_beat: true,
                local_beats: 0.0,
                local_tempo: 120.0,
            },
        )];
        assert_eq!(contents_from_values(&[], &roles2, &p).oscs, 1);
        p.oscs.clear();
        assert_eq!(contents_from_values(&[], &roles2, &p).oscs, 0);
    }

    #[test]
    fn search_matches_terms_and_badge_words() {
        let waves = PresetContents { oscs: 2, ..PresetContents::default() };
        let none = PresetContents::default();
        let m = |q: &str, name: &str, folder: &str, symbol: &str, c: &PresetContents, pin: bool| {
            preset_matches(q, name, folder, symbol, c, pin, false)
        };
        assert!(m("war wa", "Warm wash", "", "", &none, false));
        assert!(!m("war zz", "Warm wash", "", "", &none, false));
        assert!(m("chorus", "Cold blue", "Chorus", "", &none, false));
        assert!(m("", "anything", "", "", &none, false));
        assert!(m("chor", "Chorus A", "Chorus", "CHOR", &none, false));
        assert!(m("CHOR", "x", "", "chor", &none, false));
        assert!(m("wave", "x", "", "", &waves, false));
        assert!(!m("wave", "x", "", "", &none, false));
        assert!(m("pin", "x", "", "", &none, true));
        assert!(!m("pin", "x", "", "", &none, false));
        assert!(preset_matches("fade", "x", "", "", &none, false, true));
    }

    #[test]
    fn filter_sort_orders_and_filters() {
        let presets =
            vec![preset(3, "beta", ""), preset(1, "Alpha", "Chorus"), preset(2, "gamma", "")];
        let contents: HashMap<u32, PresetContents> = HashMap::new();
        let faces = |id: u32| match id {
            3 => [200, 30, 30],   // red, hue 0
            1 => [30, 30, 200],   // blue, hue 240
            _ => [90, 90, 90],    // grey: no hue, sorts last
        };
        let get = |sort, search: &str| filter_sort(&presets, &contents, &faces, search, sort);
        assert_eq!(get(PresetSort::Manual, ""), vec![0, 1, 2]);
        assert_eq!(get(PresetSort::Name, ""), vec![1, 0, 2]);
        assert_eq!(get(PresetSort::Newest, ""), vec![0, 2, 1]);
        assert_eq!(get(PresetSort::Hue, ""), vec![0, 1, 2]);
        // The search narrows every order the same way.
        assert_eq!(get(PresetSort::Manual, "chorus"), vec![1]);
        assert_eq!(get(PresetSort::Name, "chorus"), vec![1]);
        assert!(get(PresetSort::Hue, "nothing").is_empty());
        assert_eq!(hue_of([90, 90, 90]), None);
        assert!(hue_of([200, 30, 30]).is_some_and(|h| h < 10.0));
    }

    #[test]
    fn index_maps_after_delete_insert_move() {
        assert_eq!(delete_map(5, 2), vec![Some(0), Some(1), None, Some(2), Some(3)]);
        assert_eq!(insert_map(4, 1), vec![Some(0), Some(2), Some(3), Some(4)]);
        assert_eq!(move_map(5, 0, 3), vec![Some(2), Some(0), Some(1), Some(3), Some(4)]);
        assert_eq!(move_map(5, 3, 0), vec![Some(1), Some(2), Some(3), Some(0), Some(4)]);
    }

    #[test]
    fn effective_fade_prefers_explicit_then_own_then_mode() {
        use RecallMode::*;
        assert_eq!(effective_fade(Some(0.0), Some(3.0), Fade, 2.0), Some(0.0));
        assert_eq!(effective_fade(None, Some(3.0), Cut, 2.0), Some(3.0));
        assert_eq!(effective_fade(None, None, Fade, 2.0), Some(2.0));
        assert_eq!(effective_fade(None, None, Cut, 9.0), Some(0.0));
        assert_eq!(effective_fade(None, None, AsSet, 9.0), None);
    }

    #[test]
    fn preset_from_look_keeps_only_enabled_oscs_and_non_zero_values() {
        let mut look = Look::black();
        look.base[3] = 200;
        look.base[4] = 0;
        let osc = crate::oscillator::Osc { enabled: true, ..Default::default() };
        look.oscs.insert(3, osc.clone());
        look.oscs.insert(9, crate::oscillator::Osc { enabled: false, ..osc });
        let p = preset_from_look(&look, "Imported".into(), "Bank".into(), 7);
        assert_eq!(p.values, vec![(3, 200)]);
        assert_eq!(p.oscs.len(), 1);
        assert_eq!(p.oscs[0].0, 3);
        assert_eq!((p.id, p.folder.as_str(), p.name.as_str()), (7, "Bank", "Imported"));
    }

    /// Deleting, duplicating and moving keep every index-based reference
    /// pointing at the right look — or say it lost it.
    #[test]
    fn remap_updates_audio_chase_and_active() {
        let _files = Files::hold(&["presets.json", "audio.json"]);
        let mut app = App::new();
        app.user_presets = vec![preset(1, "one", ""), preset(2, "two", ""), preset(3, "three", "")];
        app.next_preset_id = 4;
        let mut trig = AudioTrigger::new(1);
        trig.source = Some(TriggerSource::Preset(2));
        app.audio_triggers.push(trig);
        app.chase.source = Some(ChaseSource::User(1));
        app.active_user_preset = Some(2);

        app.delete_preset(1);
        assert_eq!(app.audio_triggers.last().unwrap().source, Some(TriggerSource::Preset(1)));
        assert_eq!(app.chase.source, None);
        assert_eq!(app.active_user_preset, Some(1));
        assert!(app.log.iter().any(|l| l.contains("Chase source cleared")));

        app.duplicate_preset(0);
        assert_eq!(app.audio_triggers.last().unwrap().source, Some(TriggerSource::Preset(2)));
        assert!(app.user_presets[1].name.ends_with(" copy"));
        assert_eq!(app.user_presets[1].id, 4);
        assert_eq!(app.active_user_preset, Some(2));

        // Moving the active preset to the front takes the highlight with it.
        app.move_preset_before(2, 0);
        assert_eq!(app.active_user_preset, Some(0));
    }

    /// Update keeps everything about a preset except the look itself.
    #[test]
    fn update_in_place_keeps_metadata() {
        let _files = Files::hold(&["presets.json"]);
        let mut app = App::new();
        let mut p = preset(7, "Verse blue", "Chorus");
        p.color = Some([1, 2, 3]);
        p.symbol = "VB".into();
        p.icon = KeyIcon::Ring;
        p.fade = Some(2.5);
        p.pinned = true;
        app.user_presets = vec![p];
        app.next_preset_id = 8;
        app.live = Look::black();
        app.live.base[0] = 200;
        app.live_active.insert(0);
        assert!(app.update_preset_in_place(0));
        let p = &app.user_presets[0];
        assert_eq!(p.id, 7);
        assert_eq!(p.color, Some([1, 2, 3]));
        assert_eq!(p.symbol, "VB");
        assert_eq!(p.icon, KeyIcon::Ring);
        assert_eq!(p.fade, Some(2.5));
        assert!(p.pinned);
        assert_eq!(p.folder, "Chorus");
        assert_eq!(p.values, vec![(0, 200)]);
    }

    /// A rename relabels the pads that wore the derived symbol, and leaves
    /// a hand-typed one alone.
    #[test]
    fn rename_relabels_auto_slots_only() {
        let _files = Files::hold(&["presets.json", "preset_deck.json"]);
        let mut app = App::new();
        app.user_presets = vec![preset(3, "Verse blue", "")];
        app.next_preset_id = 4;
        app.preset_deck = vec![None; PRESET_DECK_SLOTS];
        let slot = |label: &str| {
            Some(crate::preset_deck::PresetSlot {
                preset: 3,
                bank: None,
                name: "Verse blue".into(),
                color: [1, 2, 3],
                label: label.to_owned(),
                icon: KeyIcon::None,
            })
        };
        app.preset_deck[0] = slot("VERS");
        app.preset_deck[1] = slot("CUST");
        app.apply_preset_edit(&PresetEdit {
            id: 3,
            name: "Bridge".into(),
            auto_color: true,
            color: [0, 0, 0],
            ..PresetEdit::default()
        });
        assert_eq!(app.user_presets[0].name, "Bridge");
        let a = app.preset_deck[0].as_ref().unwrap();
        assert_eq!((a.label.as_str(), a.name.as_str()), ("BRID", "Bridge"));
        let b = app.preset_deck[1].as_ref().unwrap();
        assert_eq!((b.label.as_str(), b.name.as_str()), ("CUST", "Bridge"));
    }

    /// Store files into a folder that exists, and drops the filing when it
    /// does not.
    #[test]
    fn store_into_files_into_folder() {
        let _files = Files::hold(&["presets.json"]);
        let mut app = App::new();
        app.user_presets.clear();
        app.preset_folders = vec!["Looks".into()];
        app.next_preset_id = 5;
        app.live = Look::black();
        app.live.base[0] = 200;
        let idx = app.store_look_into("x".into(), "Looks".into()).expect("stored");
        assert_eq!(app.user_presets[idx].folder, "Looks");
        assert_eq!(app.user_presets[idx].id, 5);
        assert_eq!(app.next_preset_id, 6);
        let idx = app.store_look_into("y".into(), "Nope".into()).expect("stored");
        assert_eq!(app.user_presets[idx].folder, "");
        // An empty name still gets one.
        let idx = app.store_look_into("  ".into(), String::new()).expect("stored");
        assert!(!app.user_presets[idx].name.is_empty());
    }

    /// The fade rule: the preset's own time, then the strip's mode, and an
    /// explicit cut beats both — and the Transition window is left as found.
    #[test]
    fn recall_uses_own_fade_then_mode() {
        let _files = Files::hold(&["presets.json"]);
        let mut app = App::new();
        app.user_presets = vec![preset(1, "own fade", ""), preset(2, "plain", "")];
        app.user_presets[0].fade = Some(1.0);
        app.next_preset_id = 3;
        app.transition.duration = 0.0;
        app.insp.prefs.presets.recall = RecallMode::AsSet;

        app.recall_user_preset(0, None);
        assert!(app.transition_run.is_some(), "the preset's own fade should start a run");
        assert_eq!(app.transition.duration, 0.0, "the Transition window is put back");
        assert_eq!(app.insp.presets.recent.front(), Some(&1));

        app.transition_run = None;
        app.insp.prefs.presets.recall = RecallMode::Fade;
        app.insp.prefs.presets.fade_secs = 2.0;
        app.recall_user_preset(1, None);
        assert!(app.transition_run.is_some(), "the strip's fade should start a run");

        app.transition_run = None;
        app.recall_user_preset(1, Some(0.0));
        assert!(app.transition_run.is_none(), "an explicit cut never fades");
        assert_eq!(app.active_user_preset, Some(1));
        assert_eq!(app.insp.presets.recent.front(), Some(&2));
        // Recalling again does not push a duplicate.
        app.recall_user_preset(1, Some(0.0));
        assert_eq!(app.insp.presets.recent.len(), 2);
    }

    /// The command bar's `preset` verb, by number and by name.
    #[test]
    fn command_recalls_by_number_and_name() {
        let _files = Files::hold(&["presets.json"]);
        let mut app = App::new();
        app.user_presets = vec![preset(1, "Verse blue", ""), preset(2, "Chorus red", "")];
        app.next_preset_id = 3;
        app.transition.duration = 0.0;
        app.command_recall_preset("2");
        assert_eq!(app.active_user_preset, Some(1));
        app.command_recall_preset("verse");
        assert_eq!(app.active_user_preset, Some(0));
        app.command_recall_preset("nothing like this");
        assert!(app.log.last().is_some_and(|l| l.starts_with("Usage: preset")));
    }

    /// The Presets tab and the board, drawn through egui's own renderer:
    /// pads, list, zoom 2, and the board in both modes.
    #[test]
    fn presets_tab_renders_headless() {
        let _files = Files::hold(&["inspector.json", "preset_deck.json"]);
        let mut app = App::new();
        *app.net.dmx.lock() = Frame([170u8; DMX_SLOTS]);
        app.stage.layout_path = std::env::temp_dir().join("dmxpress_presets_test_layout.json");
        app.collapsed.remove("inspector");
        app.collapsed.insert("channels");
        app.show_log = false;
        app.show_osc = false;
        app.insp.prefs.tab = InspectorTab::Presets;
        app.insp.prefs.width = Some(230.0);
        seed_demo_presets(&mut app);
        let size = [1600, 1000];
        // The Inspector is sized by its contents: a row that does not wrap
        // pushes the panel wider every frame, so every scene measures it.
        let shot = |app: &mut App, name: &str, max_w: f32| -> bool {
            let mut width = 0.0f32;
            let Some(pixels) = render_frames(5, size, |ctx, frame| {
                if frame == 0 {
                    crate::ui::install_theme(ctx);
                } else {
                    app.draw_ui(ctx);
                }
                if let Some(state) =
                    egui::containers::panel::PanelState::load(ctx, egui::Id::new("inspector"))
                {
                    width = state.rect.width();
                }
            }) else {
                eprintln!("no GPU adapter — skipping");
                return false;
            };
            save(&pixels, size, name);
            let lit =
                pixels.chunks(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120).count();
            assert!(lit > 1000, "{name} came out black");
            eprintln!("{name}: Inspector {width:.0} px");
            assert!(width <= max_w, "{name}: the Inspector grew to {width:.0} px (max {max_w})");
            true
        };

        if !shot(&mut app, "ui_presets_headless", 231.0) {
            return;
        }
        // List view, a search running and the Edit card open.
        app.insp.prefs.presets.view = PresetView::List;
        app.insp.presets.search = "ch".into();
        // The ShowBuddy block, unfolded (the fold key holds "not the default").
        app.insp.prefs.folded.insert("presets.showbuddy".into());
        app.insp.presets.edit = Some(PresetEdit {
            id: 2,
            name: "Cold blue".into(),
            auto_color: true,
            color: [0, 120, 255],
            symbol: String::new(),
            icon: KeyIcon::None,
            own_fade: true,
            fade_secs: 1.5,
            pinned: false,
            focus_pending: false,
        });
        shot(&mut app, "ui_presets_headless_list", 231.0);

        // Back to pads, small, at double zoom.
        app.insp.prefs.presets.view = PresetView::Grid;
        app.insp.presets.search.clear();
        app.insp.presets.edit = None;
        app.insp.prefs.presets.pad = PadSize::S;
        app.zoom.inspector = 2.0;
        // Zoom 2 holds the same 230 px: every row wraps instead of widening.
        shot(&mut app, "ui_presets_headless_zoom2", 231.0);

        // The board: play mode, then arrange with the slot editor open.
        app.zoom.inspector = 1.0;
        app.insp.prefs.presets.pad = PadSize::M;
        app.show_preset_board = true;
        app.insp.presets.board.arrange = false;
        shot(&mut app, "ui_presets_board_play", 231.0);

        app.insp.presets.board.arrange = true;
        app.insp.presets.board.slot = Some(0);
        app.insp.presets.board.target = PadTarget::User(1);
        app.insp.presets.board.color = [255, 120, 0];
        app.insp.presets.board.label = "WARM".into();
        shot(&mut app, "ui_presets_board_arrange", 231.0);

        // An empty pool: the empty state, and the ShowBuddy banks as pads.
        app.show_preset_board = false;
        app.user_presets.clear();
        app.preset_folders.clear();
        app.active_user_preset = None;
        app.insp.presets.recent.clear();
        app.insp.presets.swatches.clear();
        app.insp.presets.contents.clear();
        shot(&mut app, "ui_presets_headless_banks", 231.0);

        assert!(!app.insp.dirty, "the headless scenes must not mark prefs dirty");
    }
}
