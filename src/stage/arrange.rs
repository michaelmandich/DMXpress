//! Selection transform tools for the Inspector's Selection tab (area E):
//! align, distribute, mirror, arrange in a shape, aim and hang, all as
//! `StageView` methods that follow the stage's undo contract — return
//! early when nothing would change, else `push_undo()` first, clear both
//! `mount` and `truss_mount` on every instance whose position is written,
//! `save(patch)` last.
//!
//! The geometry lives in free functions at the top (`align_values`,
//! `distribute_values`, `mirror_transform`, `shape_points`, `aim_angles`,
//! `aim_pan_tilt`, …) so it is unit-testable without a GPU, a patch or a
//! file on disk; the methods underneath only decide *which* lights the
//! numbers land on.

use std::collections::HashSet;

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::fixture::{classify, Archetype};
use super::geometry::SnapTarget;
use super::gizmo::Drag;
use super::layout::{ElementRef, Instance, LightTransform, Tower, Truss, TrussKind};
use super::math::{angles_from_dir, dir_from_angles, rotate_about, v3, V3};
use super::settings::Settings;
use super::view::StageView;
use crate::showbuddy::Patch;

/// Nudge steps (metres) the step chips offer.
pub(crate) const NUDGE_STEPS: [f32; 6] = [0.01, 0.05, 0.1, 0.25, 0.5, 1.0];
/// Turn steps (degrees) the step chips offer.
pub(crate) const ROT_STEPS: [f32; 5] = [1.0, 5.0, 15.0, 45.0, 90.0];
/// Grid pitches (metres) "Snap positions" offers.
pub(crate) const SNAP_STEPS: [f32; 4] = [0.1, 0.25, 0.5, 1.0];

/// Positions closer than this count as the same point.
const EPS: f32 = 1e-4;

/// The four keys that nudge.
const ARROWS: [egui::Key; 4] =
    [egui::Key::ArrowLeft, egui::Key::ArrowRight, egui::Key::ArrowUp, egui::Key::ArrowDown];

// ---------------------------------------------------------------- types ----

/// One world axis of a light's position.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn get(self, p: V3) -> f32 {
        match self {
            Axis::X => p.x,
            Axis::Y => p.y,
            Axis::Z => p.z,
        }
    }

    pub fn set(self, p: &mut V3, v: f32) {
        match self {
            Axis::X => p.x = v,
            Axis::Y => p.y = v,
            Axis::Z => p.z = v,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
        }
    }
}

/// Which end of the selection an align snaps to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AlignMode {
    Min,
    Mid,
    Max,
}

impl AlignMode {
    pub fn label(self) -> &'static str {
        match self {
            AlignMode::Min => "min",
            AlignMode::Mid => "mid",
            AlignMode::Max => "max",
        }
    }
}

/// Even spacing between the outermost two, or a fixed gap from the lowest.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum DistributeMode {
    Even,
    Gap(f32),
}

/// The plane a mirror reflects across.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MirrorPlane {
    X,
    Z,
}

impl MirrorPlane {
    pub fn label(self) -> &'static str {
        match self {
            MirrorPlane::X => "X",
            MirrorPlane::Z => "Z",
        }
    }
}

/// What a mirror reflects about.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub(crate) enum MirrorPivot {
    #[default]
    Selection,
    Stage,
}

/// The order a tool walks the selection in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub(crate) enum OrderBy {
    #[default]
    Address,
    Selection,
    PosX,
    PosZ,
}

impl OrderBy {
    pub const ALL: [OrderBy; 4] = [OrderBy::Address, OrderBy::Selection, OrderBy::PosX, OrderBy::PosZ];

    pub fn label(self) -> &'static str {
        match self {
            OrderBy::Address => "By address",
            OrderBy::Selection => "Pick order",
            OrderBy::PosX => "By X",
            OrderBy::PosZ => "By Z",
        }
    }
}

/// Which way an arranged light ends up pointing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub(crate) enum Facing {
    #[default]
    Keep,
    Inward,
    Outward,
}

impl Facing {
    pub const ALL: [Facing; 3] = [Facing::Keep, Facing::Inward, Facing::Outward];

    pub fn label(self) -> &'static str {
        match self {
            Facing::Keep => "Keep",
            Facing::Inward => "Inward",
            Facing::Outward => "Outward",
        }
    }
}

/// The shape `shape_points` lays the selection out in.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum ArrangeShape {
    /// `spacing` is already the fitted spacing when the operator asked for
    /// a total length (`length / (n - 1)`).
    Line {
        heading_deg: f32,
        spacing: f32,
    },
    Arc {
        radius: f32,
        heading_deg: f32,
        sweep_deg: f32,
        facing: Facing,
    },
    Circle {
        radius: f32,
        start_deg: f32,
        facing: Facing,
    },
    Grid {
        cols: usize,
        spacing_x: f32,
        spacing_z: f32,
    },
}

impl ArrangeShape {
    pub fn name(self) -> &'static str {
        match self {
            ArrangeShape::Line { .. } => "line",
            ArrangeShape::Arc { .. } => "arc",
            ArrangeShape::Circle { .. } => "circle",
            ArrangeShape::Grid { .. } => "grid",
        }
    }
}

/// Where an aim tool points the selection.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum AimTarget {
    Point(V3),
    StageCentre,
    Centroid,
    StraightDown,
    StraightUp,
    Level,
}

impl AimTarget {
    pub fn name(self) -> String {
        match self {
            AimTarget::Point(p) => format!("({:.1}, {:.1}, {:.1})", p.x, p.y, p.z),
            AimTarget::StageCentre => "the stage centre".into(),
            AimTarget::Centroid => "their own centre".into(),
            AimTarget::StraightDown => "straight down".into(),
            AimTarget::StraightUp => "straight up".into(),
            AimTarget::Level => "the horizon".into(),
        }
    }
}

/// How a hang spends the free slots of a face.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub(crate) enum HangFill {
    FromStart,
    Centred,
    #[default]
    Spread,
}

impl HangFill {
    pub const ALL: [HangFill; 3] = [HangFill::FromStart, HangFill::Centred, HangFill::Spread];

    pub fn label(self) -> &'static str {
        match self {
            HangFill::FromStart => "From start",
            HangFill::Centred => "Centred",
            HangFill::Spread => "Spread",
        }
    }
}

/// One light on the transform clipboard, relative to the copied centroid.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ClipItem {
    pub offset: V3,
    pub t: LightTransform,
    pub opacity: f32,
}

/// A copied placement: one light, or a whole group's pattern.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TransformClip {
    pub centroid: V3,
    pub items: Vec<ClipItem>,
}

impl TransformClip {
    /// The one-line description the Clip page shows under the buttons.
    pub fn summary(&self) -> String {
        match self.items.len() {
            0 => "Clipboard empty".into(),
            1 => {
                let it = &self.items[0];
                let p = self.centroid + it.offset;
                format!(
                    "Clipboard: 1 light · ({:.2}, {:.2}, {:.2}) · yaw {:.0} / pitch {:.0}",
                    p.x, p.y, p.z, it.t.yaw_deg, it.t.pitch_deg
                )
            }
            n => format!("Clipboard: {n} lights (pattern)"),
        }
    }
}

/// Which parts of a copied transform a paste writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PasteParts {
    pub position: bool,
    pub rotation: bool,
    pub scale: bool,
    pub opacity: bool,
}

impl Default for PasteParts {
    fn default() -> Self {
        Self { position: true, rotation: true, scale: false, opacity: false }
    }
}

impl PasteParts {
    /// "position + rotation" for the log line.
    pub fn words(self) -> String {
        let mut v: Vec<&str> = Vec::new();
        if self.position {
            v.push("position");
        }
        if self.rotation {
            v.push("rotation");
        }
        if self.scale {
            v.push("size");
        }
        if self.opacity {
            v.push("opacity");
        }
        if v.is_empty() {
            "nothing".into()
        } else {
            v.join(" + ")
        }
    }

    pub fn any(self) -> bool {
        self.position || self.rotation || self.scale || self.opacity
    }
}

/// What a tool did: how many lights it moved, how many it had to leave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub changed: usize,
    pub skipped: usize,
}

impl Outcome {
    fn of(changed: usize) -> Self {
        Self { changed, skipped: 0 }
    }
}

/// Everything the Selection tab's header needs about what is picked.
#[derive(Clone, Debug, Default)]
pub(crate) struct SelectionSummary {
    pub lights: usize,
    pub fixtures: usize,
    pub heads: usize,
    pub mounted: usize,
    /// How many instances share the anchor light's fixture.
    pub copies_of_first: usize,
    /// The anchor light's DMX range, only when one fixture is selected.
    pub dmx: Option<(u16, u16)>,
    /// Distinct archetypes, in first-seen order.
    pub archetypes: Vec<Archetype>,
    /// Type stem → how many selected fixtures carry it (≤ 3 rows, the rest
    /// folded into one `("…", k)`).
    pub names: Vec<(String, usize)>,
    /// The anchor light's mount: which element, which slot.
    pub first_mount: Option<(ElementRef, usize)>,
    pub bounds: Option<(V3, V3)>,
    pub mixed_rotation: bool,
    /// With no lights selected: the picked element, its hung lights, its slots.
    pub element: Option<(ElementRef, usize, usize)>,
}

/// Pre-edit copy of the selected transforms (or the picked element) so one
/// DragValue gesture undoes as a single step.
pub(crate) struct EditPre {
    lights: Vec<(usize, LightTransform, f32)>,
    tower: Option<(usize, Tower)>,
    truss: Option<(usize, Truss)>,
}

// ----------------------------------------------------------- pure maths ----

/// The coordinate every light lands on for an align.
pub(crate) fn align_values(vals: &[f32], mode: AlignMode) -> f32 {
    if vals.is_empty() {
        return 0.0;
    }
    match mode {
        AlignMode::Min => vals.iter().copied().fold(f32::INFINITY, f32::min),
        AlignMode::Max => vals.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        AlignMode::Mid => vals.iter().sum::<f32>() / vals.len() as f32,
    }
}

/// Evenly spaced (or fixed-gap) coordinates, returned in the INPUT order but
/// carrying the rank each value had — so the lights keep their left-to-right
/// order and simply space out.
pub(crate) fn distribute_values(vals: &[f32], mode: DistributeMode) -> Vec<f32> {
    let n = vals.len();
    if n < 2 {
        return vals.to_vec();
    }
    let mut rank: Vec<usize> = (0..n).collect();
    rank.sort_by(|&a, &b| vals[a].partial_cmp(&vals[b]).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b)));
    let lo = vals.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = vals.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut out = vec![0.0; n];
    for (k, &i) in rank.iter().enumerate() {
        out[i] = match mode {
            DistributeMode::Even => lo + (hi - lo) * k as f32 / (n - 1) as f32,
            DistributeMode::Gap(g) => lo + g * k as f32,
        };
    }
    out
}

/// Reflect one light's placement across `plane` through `pivot`. Pitch is
/// untouched — a mirror swaps left and right, not up and down.
pub(crate) fn mirror_transform(t: &mut LightTransform, plane: MirrorPlane, pivot: V3) {
    match plane {
        MirrorPlane::X => {
            t.pos.x = 2.0 * pivot.x - t.pos.x;
            t.yaw_deg = -t.yaw_deg;
        }
        MirrorPlane::Z => {
            t.pos.z = 2.0 * pivot.z - t.pos.z;
            t.yaw_deg = 180.0 - t.yaw_deg;
        }
    }
    t.roll_deg = (-t.roll_deg).rem_euclid(360.0);
}

/// Reflect a truss's pose across `plane`: the mirrored heading (a radius run
/// starts where its old sweep ended, so `yaw' = -(yaw + arc)` across X), the
/// mirrored tilt axis and the mirrored lean. Derived from `Truss::frame`,
/// which is why the pitch flips for a Z mirror but not for an X one.
pub(crate) fn mirror_truss_pose(
    kind: TrussKind,
    yaw_deg: f32,
    arc_deg: f32,
    pitch_deg: f32,
    roll_deg: f32,
    plane: MirrorPlane,
) -> (f32, f32, f32) {
    let end = match kind {
        TrussKind::Straight => yaw_deg,
        TrussKind::Radius => yaw_deg + arc_deg,
    };
    match plane {
        MirrorPlane::X => (-end, pitch_deg, -roll_deg),
        MirrorPlane::Z => (180.0 - end, -pitch_deg, -roll_deg),
    }
}

/// Where `n` lights go when laid out in `shape` about `centre`, with the yaw
/// each one takes when the shape dictates one.
pub(crate) fn shape_points(shape: ArrangeShape, n: usize, centre: V3) -> Vec<(V3, Option<f32>)> {
    let mut out: Vec<(V3, Option<f32>)> = Vec::with_capacity(n);
    if n == 0 {
        return out;
    }
    match shape {
        ArrangeShape::Line { heading_deg, spacing } => {
            let dir = dir_from_angles(heading_deg, 0.0);
            for k in 0..n {
                let d = (k as f32 - (n as f32 - 1.0) / 2.0) * spacing;
                out.push((centre + dir * d, None));
            }
        }
        ArrangeShape::Arc { radius, heading_deg, sweep_deg, facing } => {
            for k in 0..n {
                let a = if n == 1 {
                    heading_deg
                } else {
                    heading_deg - sweep_deg / 2.0 + sweep_deg * k as f32 / (n - 1) as f32
                };
                let p = centre + dir_from_angles(a, 0.0) * radius;
                out.push((p, face_yaw(facing, p, centre)));
            }
        }
        ArrangeShape::Circle { radius, start_deg, facing } => {
            for k in 0..n {
                let a = start_deg + 360.0 * k as f32 / n as f32;
                let p = centre + dir_from_angles(a, 0.0) * radius;
                out.push((p, face_yaw(facing, p, centre)));
            }
        }
        ArrangeShape::Grid { cols, spacing_x, spacing_z } => {
            let cols = cols.max(1);
            let rows = n.div_ceil(cols);
            for k in 0..n {
                let (r, c) = (k / cols, k % cols);
                let x = (c as f32 - (cols as f32 - 1.0) / 2.0) * spacing_x;
                let z = (r as f32 - (rows as f32 - 1.0) / 2.0) * spacing_z;
                out.push((centre + v3(x, 0.0, z), None));
            }
        }
    }
    out
}

/// The yaw a light at `p` takes to face the shape's centre (or away from it).
fn face_yaw(facing: Facing, p: V3, centre: V3) -> Option<f32> {
    let d = match facing {
        Facing::Keep => return None,
        Facing::Inward => centre - p,
        Facing::Outward => p - centre,
    };
    (d.len() > EPS).then(|| angles_from_dir(d.norm()).0)
}

/// Mounting yaw and pitch so a light at `from` points at `to`. `None` when
/// the two coincide — there is no direction to take.
pub(crate) fn aim_angles(from: V3, to: V3) -> Option<(f32, f32)> {
    let d = to - from;
    (d.len() > EPS).then(|| angles_from_dir(d.norm()))
}

/// Pan and tilt as DMX fractions (0..1) so a moving head mounted at `t`
/// puts its beam on `target`, or `None` when the head cannot reach it.
///
/// The head frame is exactly `draw.rs`'s: forward from yaw/pitch, a helper
/// axis, then the right/up pair spun by `roll_deg`; the beam leaves along
/// `f·cos(tilt) + p_u·sin(tilt)` where `p_u = -u0·cos(pan) + r0·sin(pan)`.
pub(crate) fn aim_pan_tilt(
    t: &LightTransform,
    pan_range: f32,
    tilt_range: f32,
    target: V3,
) -> Option<(f32, f32)> {
    if pan_range <= 0.0 || tilt_range <= 0.0 {
        return None;
    }
    let d = target - t.pos;
    if d.len() <= EPS {
        return None;
    }
    let d = d.norm();
    let f = dir_from_angles(t.yaw_deg, t.pitch_deg);
    let helper = if f.y.abs() > 0.9 { v3(0.0, 0.0, 1.0) } else { v3(0.0, 1.0, 0.0) };
    let r0b = helper.cross(f).norm();
    let u0b = f.cross(r0b).norm();
    let (rs, rc) = t.roll_deg.to_radians().sin_cos();
    let r0 = r0b * rc + u0b * rs;
    let u0 = u0b * rc - r0b * rs;

    let tilt = d.dot(f).clamp(-1.0, 1.0).acos();
    let pan = if tilt.sin().abs() < EPS {
        0.0
    } else {
        let q = (d - f * tilt.cos()).norm();
        q.dot(r0).atan2(-q.dot(u0))
    };
    let pi = std::f32::consts::PI;
    let encode = |pan: f32, tilt: f32| -> Option<(f32, f32)> {
        let p = 0.5 + pan.to_degrees() / pan_range;
        let ti = 0.5 + tilt.to_degrees() / tilt_range;
        ((0.0..=1.0).contains(&p) && (0.0..=1.0).contains(&ti)).then_some((p, ti))
    };
    // The yoke reaches most points two ways round; take the one that needs
    // the least pan travel, so a head does not swing through the truss.
    let mut best: Option<(f32, (f32, f32))> = None;
    for (p, ti) in [(pan, tilt), (pan + pi, -tilt), (pan - pi, -tilt)] {
        if let Some(v) = encode(p, ti) {
            if best.map_or(true, |(cost, _)| p.abs() < cost) {
                best = Some((p.abs(), v));
            }
        }
    }
    best.map(|(_, v)| v)
}

/// Every `n`-th entry of `sorted`, starting at `offset`.
pub(crate) fn every_nth(sorted: &[usize], n: usize, offset: usize) -> Vec<usize> {
    let n = n.max(1);
    let off = offset % n;
    sorted.iter().enumerate().filter(|(i, _)| i % n == off).map(|(_, &v)| v).collect()
}

/// `k` slots spread as widely as `free` allows (the ends always taken).
pub(crate) fn spread_pick(free: &[usize], k: usize) -> Vec<usize> {
    let m = free.len();
    if k == 0 || m == 0 {
        return Vec::new();
    }
    if k == 1 {
        return vec![free[(m - 1) / 2]];
    }
    let k = k.min(m);
    (0..k)
        .map(|i| free[((i as f32 * (m - 1) as f32 / (k - 1) as f32).round() as usize).min(m - 1)])
        .collect()
}

/// A pan/tilt fixture: aiming it turns the head, not the body.
pub(crate) fn is_head(a: Archetype) -> bool {
    matches!(a, Archetype::MovingPar | Archetype::Beam)
}

// ------------------------------------------------------ StageView tools ----

impl StageView {
    /// Valid selected instance indices in `order`, ties by instance index.
    pub(crate) fn sorted_selection(&self, patch: &Patch, order: OrderBy) -> Vec<usize> {
        let mut sel: Vec<usize> =
            self.selection.iter().copied().filter(|&i| i < self.instances.len()).collect();
        sel.sort_unstable();
        let key = |i: usize| -> f32 {
            let inst = &self.instances[i];
            match order {
                OrderBy::Address => {
                    patch.fixtures.get(inst.fixture).map_or(0.0, |f| f.from as f32)
                }
                OrderBy::Selection => i as f32,
                OrderBy::PosX => inst.t.pos.x,
                OrderBy::PosZ => inst.t.pos.z,
            }
        };
        sel.sort_by(|&a, &b| {
            key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b))
        });
        sel
    }

    /// Is this instance a moving head?
    fn instance_is_head(&self, patch: &Patch, i: usize) -> bool {
        self.instances
            .get(i)
            .and_then(|inst| patch.fixtures.get(inst.fixture))
            .is_some_and(|f| is_head(classify(f)))
    }

    /// The anchor light: the last picked fixture's first selected instance.
    fn anchor_instance(&self, sel: &[usize]) -> Option<usize> {
        if let Some(fi) = self.last_selected {
            if let Some(&i) = sel.iter().find(|&&i| self.instances[i].fixture == fi) {
                return Some(i);
            }
        }
        sel.first().copied()
    }

    /// Everything the Selection tab's header card shows.
    pub(crate) fn selection_summary(&self, patch: &Patch) -> SelectionSummary {
        let sel = self.sorted_selection(patch, OrderBy::Address);
        let mut s = SelectionSummary { lights: sel.len(), ..Default::default() };
        if sel.is_empty() {
            let picked =
                self.sel_tower.map(ElementRef::Tower).or(self.sel_truss.map(ElementRef::Truss));
            s.element = picked.and_then(|r| {
                let (hung, slots) = match r {
                    ElementRef::Tower(i) => {
                        let tw = self.towers.get(i)?;
                        let _ = tw;
                        (
                            self.instances
                                .iter()
                                .filter(|inst| inst.mount.is_some_and(|(t, _)| t == i))
                                .count(),
                            super::layout::TOWER_SLOTS,
                        )
                    }
                    ElementRef::Truss(i) => {
                        let tr = self.trusses.get(i)?;
                        (
                            self.instances
                                .iter()
                                .filter(|inst| inst.truss_mount.is_some_and(|(t, _)| t == i))
                                .count(),
                            tr.total_slots(),
                        )
                    }
                };
                Some((r, hung, slots))
            });
            return s;
        }
        let mut fixtures: Vec<usize> = sel.iter().map(|&i| self.instances[i].fixture).collect();
        fixtures.sort_unstable();
        fixtures.dedup();
        s.fixtures = fixtures.len();
        let (mut lo, mut hi) = (self.instances[sel[0]].t.pos, self.instances[sel[0]].t.pos);
        let first_rot = {
            let t = &self.instances[sel[0]].t;
            (t.yaw_deg, t.pitch_deg, t.roll_deg)
        };
        for &i in &sel {
            let inst = &self.instances[i];
            let p = inst.t.pos;
            lo = v3(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = v3(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
            if inst.mount.is_some() || inst.truss_mount.is_some() {
                s.mounted += 1;
            }
            if self.instance_is_head(patch, i) {
                s.heads += 1;
            }
            let r = (inst.t.yaw_deg, inst.t.pitch_deg, inst.t.roll_deg);
            if (r.0 - first_rot.0).abs() > 1e-3
                || (r.1 - first_rot.1).abs() > 1e-3
                || (r.2 - first_rot.2).abs() > 1e-3
            {
                s.mixed_rotation = true;
            }
            if let Some(f) = patch.fixtures.get(inst.fixture) {
                let a = classify(f);
                if !s.archetypes.contains(&a) {
                    s.archetypes.push(a);
                }
            }
        }
        s.bounds = Some((lo, hi));
        // Type pills: at most three, the rest folded into one.
        let mut counts: Vec<(String, usize)> = Vec::new();
        for &fi in &fixtures {
            if let Some(f) = patch.fixtures.get(fi) {
                let stem = crate::ui::inspector::type_stem(&f.display).to_owned();
                match counts.iter_mut().find(|(n, _)| *n == stem) {
                    Some((_, c)) => *c += 1,
                    None => counts.push((stem, 1)),
                }
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        if counts.len() > 3 {
            let rest: usize = counts[3..].iter().map(|(_, c)| *c).sum();
            counts.truncate(3);
            counts.push(("…".into(), rest));
        }
        s.names = counts;
        if let Some(i) = self.anchor_instance(&sel) {
            let inst = &self.instances[i];
            s.copies_of_first =
                self.instances.iter().filter(|o| o.fixture == inst.fixture).count();
            s.first_mount = inst
                .truss_mount
                .map(|(t, slot)| (ElementRef::Truss(t), slot))
                .or_else(|| inst.mount.map(|(t, slot)| (ElementRef::Tower(t), slot)));
            if s.fixtures == 1 {
                s.dmx = patch.fixtures.get(inst.fixture).map(|f| (f.from, f.to));
            }
        }
        s
    }

    /// The selection's axis-aligned bounds.
    pub(crate) fn selection_bounds(&self) -> Option<(V3, V3)> {
        let mut it = self.selection.iter().filter_map(|&i| self.instances.get(i)).map(|i| i.t.pos);
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| {
            (
                v3(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z)),
                v3(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z)),
            )
        }))
    }

    /// Is any selected light hung on a tower or truss?
    pub(crate) fn selection_has_mounted(&self) -> bool {
        self.selection
            .iter()
            .filter_map(|&i| self.instances.get(i))
            .any(|inst| inst.mount.is_some() || inst.truss_mount.is_some())
    }

    /// Unhook one light from whatever it hangs on. Every tool that writes a
    /// position calls this, or the per-frame re-glue snaps it straight back.
    fn detach(&mut self, i: usize) {
        if let Some(inst) = self.instances.get_mut(i) {
            inst.mount = None;
            inst.truss_mount = None;
        }
    }

    /// Snapshot what a typing/drag gesture is about to change.
    pub(crate) fn edit_pre(&self) -> EditPre {
        let mut sel: Vec<usize> =
            self.selection.iter().copied().filter(|&i| i < self.instances.len()).collect();
        sel.sort_unstable();
        EditPre {
            lights: sel
                .into_iter()
                .map(|i| (i, self.instances[i].t.clone(), self.instances[i].opacity))
                .collect(),
            tower: self.sel_tower.and_then(|i| self.towers.get(i).map(|t| (i, t.clone()))),
            truss: self.sel_truss.and_then(|i| self.trusses.get(i).map(|t| (i, t.clone()))),
        }
    }

    /// The first change of a gesture pushes ONE undo step holding the values
    /// from before the first tick; later ticks of the same gesture are free.
    pub(crate) fn undo_checkpoint(&mut self, open: &mut bool, mut pre: EditPre) {
        if *open {
            return;
        }
        // Swap the pre-edit values in, snapshot, swap the live ones back.
        for (i, t, opacity) in pre.lights.iter_mut() {
            if let Some(inst) = self.instances.get_mut(*i) {
                std::mem::swap(&mut inst.t, t);
                std::mem::swap(&mut inst.opacity, opacity);
            }
        }
        if let Some((i, tw)) = pre.tower.as_mut() {
            if let Some(live) = self.towers.get_mut(*i) {
                std::mem::swap(live, tw);
            }
        }
        if let Some((i, tr)) = pre.truss.as_mut() {
            if let Some(live) = self.trusses.get_mut(*i) {
                std::mem::swap(live, tr);
            }
        }
        self.push_undo();
        for (i, t, opacity) in pre.lights.iter_mut() {
            if let Some(inst) = self.instances.get_mut(*i) {
                std::mem::swap(&mut inst.t, t);
                std::mem::swap(&mut inst.opacity, opacity);
            }
        }
        if let Some((i, tw)) = pre.tower.as_mut() {
            if let Some(live) = self.towers.get_mut(*i) {
                std::mem::swap(live, tw);
            }
        }
        if let Some((i, tr)) = pre.truss.as_mut() {
            if let Some(live) = self.trusses.get_mut(*i) {
                std::mem::swap(live, tr);
            }
        }
        *open = true;
    }

    // ---- nudge ----

    /// Shift (and turn) the selection. `push` is false for the later frames
    /// of one held-down arrow-key run, which undoes as a single step.
    fn apply_nudge(
        &mut self,
        patch: &Patch,
        delta: V3,
        dyaw: f32,
        dpitch: f32,
        push: bool,
    ) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        let moving = delta != V3::default();
        if sel.is_empty() || (!moving && dyaw == 0.0 && dpitch == 0.0) {
            return Outcome::default();
        }
        if push {
            self.push_undo();
        }
        for &i in &sel {
            if moving {
                self.detach(i);
            }
            let head = self.instance_is_head(patch, i);
            let inst = &mut self.instances[i];
            inst.t.pos = inst.t.pos + delta;
            if head {
                inst.t.roll_deg = (inst.t.roll_deg + dyaw).rem_euclid(360.0);
            } else {
                inst.t.yaw_deg += dyaw;
                inst.t.pitch_deg += dpitch;
            }
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    /// Shift the selection by `delta` and turn it by `dyaw` / `dpitch`.
    pub(crate) fn nudge_selection(
        &mut self,
        patch: &Patch,
        delta: V3,
        dyaw: f32,
        dpitch: f32,
    ) -> Outcome {
        self.apply_nudge(patch, delta, dyaw, dpitch, true)
    }

    /// Move or turn the picked tower / truss; its hung lights come along.
    fn apply_nudge_element(&mut self, patch: &Patch, delta: V3, dyaw: f32, push: bool) -> bool {
        if delta == V3::default() && dyaw == 0.0 {
            return false;
        }
        if let Some(ti) = self.sel_tower.filter(|&i| i < self.towers.len()) {
            if push {
                self.push_undo();
            }
            let tw = &mut self.towers[ti];
            tw.pos.x += delta.x;
            tw.pos.z += delta.z;
            tw.height = (tw.height + delta.y).clamp(0.2, 12.0);
            tw.yaw_deg += dyaw;
            self.spin_tower_mounts(ti, dyaw);
            self.save(patch);
            return true;
        }
        if let Some(ti) = self.sel_truss.filter(|&i| i < self.trusses.len()) {
            if push {
                self.push_undo();
            }
            let tr = &mut self.trusses[ti];
            tr.pos = tr.pos + delta;
            tr.yaw_deg += dyaw;
            self.spin_truss_mounts(ti, dyaw);
            self.save(patch);
            return true;
        }
        false
    }

    /// Move or turn the picked tower / truss by one step.
    pub(crate) fn nudge_element(&mut self, patch: &Patch, delta: V3, dyaw: f32) -> bool {
        self.apply_nudge_element(patch, delta, dyaw, true)
    }

    /// Arrow keys: move the selection (or the picked element) by
    /// `nudge_step`. ⇧ ×10, ⌥ ÷10, ⌘↑/↓ change height. The caller guards
    /// hover, fly mode and keyboard focus; the keys are consumed either way,
    /// and one held run pushes exactly one undo step.
    pub(crate) fn nudge_keys(&mut self, ui: &egui::Ui, patch: &Patch) -> bool {
        use egui::Modifiers;
        let step = self.nudge_step.max(0.001);
        let (right, fwd) = if self.nudge_camera_relative {
            (
                v3(self.cam.yaw.cos(), 0.0, -self.cam.yaw.sin()),
                v3(-self.cam.yaw.sin(), 0.0, -self.cam.yaw.cos()),
            )
        } else {
            (v3(1.0, 0.0, 0.0), v3(0.0, 0.0, -1.0))
        };
        // Most specific chord first, so ⌘⇧↑ is never read as a bare ↑.
        let combos: [(Modifiers, f32, bool); 5] = [
            (Modifiers::COMMAND.plus(Modifiers::SHIFT), 10.0, true),
            (Modifiers::COMMAND, 1.0, true),
            (Modifiers::SHIFT, 10.0, false),
            (Modifiers::ALT, 0.1, false),
            (Modifiers::NONE, 1.0, false),
        ];
        let mut delta = V3::default();
        for (mods, mul, vertical) in combos {
            let hit = ui.input_mut(|i| ARROWS.map(|k| i.consume_key(mods, k)));
            let d = step * mul;
            if vertical {
                if hit[2] {
                    delta.y += d;
                }
                if hit[3] {
                    delta.y -= d;
                }
            } else {
                if hit[0] {
                    delta = delta - right * d;
                }
                if hit[1] {
                    delta = delta + right * d;
                }
                if hit[2] {
                    delta = delta + fwd * d;
                }
                if hit[3] {
                    delta = delta - fwd * d;
                }
            }
        }
        if delta == V3::default() {
            return false;
        }
        let push = !self.nudge_key_armed;
        self.nudge_key_armed = true;
        if self.selection.is_empty() {
            self.apply_nudge_element(patch, delta, 0.0, push)
        } else {
            self.apply_nudge(patch, delta, 0.0, 0.0, push).changed > 0
        }
    }

    /// End a held-arrow run once no arrow is down any more, so the next run
    /// pushes its own undo step. `nudge_keys` only runs while the stage or
    /// the Nudge card is hovered, and a key let go anywhere else must still
    /// close the run — so this is called once a frame from `StageView::ui`,
    /// hovered or not.
    pub(crate) fn release_nudge_keys(&mut self, ui: &egui::Ui) {
        if self.nudge_key_armed && ui.input(|i| !i.keys_down.iter().any(|k| ARROWS.contains(k))) {
            self.nudge_key_armed = false;
        }
    }

    // ---- typed edits (the transform grid) ----

    /// Move the selection so its centroid lands on `centroid`.
    pub(crate) fn set_selection_position(&mut self, patch: &Patch, centroid: V3) -> Outcome {
        let Some(old) = self.selection_centroid() else { return Outcome::default() };
        let d = centroid - old;
        if d.len() <= EPS {
            return Outcome::default();
        }
        self.apply_nudge(patch, d, 0.0, 0.0, true)
    }

    /// Set the given rotation fields on every selected light (mounts kept:
    /// a spin does not unhook anything).
    pub(crate) fn set_selection_rotation(
        &mut self,
        patch: &Patch,
        yaw: Option<f32>,
        pitch: Option<f32>,
        roll: Option<f32>,
    ) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.is_empty() || (yaw.is_none() && pitch.is_none() && roll.is_none()) {
            return Outcome::default();
        }
        // Every light already at these angles: a second click on a reset
        // chip must not spend an undo step on a write that changes nothing.
        if sel.iter().all(|&i| {
            let t = &self.instances[i].t;
            yaw.map_or(true, |v| (t.yaw_deg - v).abs() <= EPS)
                && pitch.map_or(true, |v| (t.pitch_deg - v).abs() <= EPS)
                && roll.map_or(true, |v| (t.roll_deg - v.rem_euclid(360.0)).abs() <= EPS)
        }) {
            return Outcome::default();
        }
        self.push_undo();
        for &i in &sel {
            let t = &mut self.instances[i].t;
            if let Some(v) = yaw {
                t.yaw_deg = v;
            }
            if let Some(v) = pitch {
                t.pitch_deg = v;
            }
            if let Some(v) = roll {
                t.roll_deg = v.rem_euclid(360.0);
            }
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    /// Set every selected light's size multiplier.
    pub(crate) fn set_selection_scale(&mut self, patch: &Patch, scale: f32) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        let scale = scale.clamp(0.05, 10.0);
        if sel.is_empty() || sel.iter().all(|&i| (self.instances[i].t.scale - scale).abs() <= EPS) {
            return Outcome::default();
        }
        self.push_undo();
        for &i in &sel {
            self.instances[i].t.scale = scale;
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    /// Set every selected light's housing opacity.
    pub(crate) fn set_selection_opacity(&mut self, patch: &Patch, opacity: f32) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        let opacity = opacity.clamp(0.0, 1.0);
        if sel.is_empty() || sel.iter().all(|&i| (self.instances[i].opacity - opacity).abs() <= EPS)
        {
            return Outcome::default();
        }
        self.push_undo();
        for &i in &sel {
            self.instances[i].opacity = opacity;
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    /// Back to the mounting angles a freshly placed light gets.
    pub(crate) fn reset_selection_rotation(&mut self, patch: &Patch, set: &Settings) -> Outcome {
        self.set_selection_rotation(
            patch,
            Some(set.default_yaw),
            Some(set.default_pitch),
            Some(0.0),
        )
    }

    // ---- align / distribute / tidy ----

    /// Put every selected light on one coordinate.
    pub(crate) fn align_selection(
        &mut self,
        patch: &Patch,
        axis: Axis,
        mode: AlignMode,
    ) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.is_empty() {
            return Outcome::default();
        }
        let vals: Vec<f32> = sel.iter().map(|&i| axis.get(self.instances[i].t.pos)).collect();
        let target = align_values(&vals, mode);
        if vals.iter().all(|v| (v - target).abs() <= EPS) {
            return Outcome::default();
        }
        self.push_undo();
        for &i in &sel {
            self.detach(i);
            let p = &mut self.instances[i].t.pos;
            axis.set(p, target);
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    /// Space the selection out along one axis.
    pub(crate) fn distribute_selection(
        &mut self,
        patch: &Patch,
        axis: Axis,
        mode: DistributeMode,
    ) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.len() < 2 {
            return Outcome::default();
        }
        let vals: Vec<f32> = sel.iter().map(|&i| axis.get(self.instances[i].t.pos)).collect();
        let lo = vals.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = vals.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if matches!(mode, DistributeMode::Even) && hi - lo < EPS {
            return Outcome::default();
        }
        let out = distribute_values(&vals, mode);
        if vals.iter().zip(&out).all(|(a, b)| (a - b).abs() <= EPS) {
            return Outcome::default();
        }
        self.push_undo();
        for (k, &i) in sel.iter().enumerate() {
            self.detach(i);
            let p = &mut self.instances[i].t.pos;
            axis.set(p, out[k]);
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    /// The axis the selection spreads out on most — what "by gap" uses.
    pub(crate) fn widest_axis(&self) -> Axis {
        let Some((lo, hi)) = self.selection_bounds() else { return Axis::X };
        let (dx, dz) = ((hi.x - lo.x).abs(), (hi.z - lo.z).abs());
        if dz > dx {
            Axis::Z
        } else {
            Axis::X
        }
    }

    /// Slide the selection so its centre sits over stage X = 0, Z = 0.
    pub(crate) fn centre_selection_on_stage(&mut self, patch: &Patch) -> Outcome {
        let Some(c) = self.selection_centroid() else { return Outcome::default() };
        let d = v3(-c.x, 0.0, -c.z);
        if d.len() <= EPS {
            return Outcome::default();
        }
        self.apply_nudge(patch, d, 0.0, 0.0, true)
    }

    /// Put every selected light at one height.
    pub(crate) fn set_selection_height(&mut self, patch: &Patch, y: f32) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.is_empty() || sel.iter().all(|&i| (self.instances[i].t.pos.y - y).abs() <= EPS) {
            return Outcome::default();
        }
        self.push_undo();
        for &i in &sel {
            self.detach(i);
            self.instances[i].t.pos.y = y;
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    /// Round every selected light onto a `step` metre grid.
    pub(crate) fn snap_selection_to_grid(&mut self, patch: &Patch, step: f32) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        let step = step.max(0.001);
        if sel.is_empty() {
            return Outcome::default();
        }
        let snap = |v: f32| (v / step).round() * step;
        let moved: Vec<usize> = sel
            .iter()
            .copied()
            .filter(|&i| {
                let p = self.instances[i].t.pos;
                (snap(p.x) - p.x).abs() > EPS
                    || (snap(p.y) - p.y).abs() > EPS
                    || (snap(p.z) - p.z).abs() > EPS
            })
            .collect();
        if moved.is_empty() {
            return Outcome::default();
        }
        self.push_undo();
        for &i in &moved {
            self.detach(i);
            let p = &mut self.instances[i].t.pos;
            *p = v3(snap(p.x), snap(p.y), snap(p.z));
        }
        self.save(patch);
        Outcome::of(moved.len())
    }

    /// Lift every second light by `delta` — the two-row look.
    pub(crate) fn stagger_selection(
        &mut self,
        patch: &Patch,
        axis: Axis,
        delta: f32,
        order: OrderBy,
    ) -> Outcome {
        let sel = self.sorted_selection(patch, order);
        if sel.len() < 2 || delta.abs() <= EPS {
            return Outcome::default();
        }
        self.push_undo();
        let mut n = 0;
        for (k, &i) in sel.iter().enumerate() {
            if k % 2 == 0 {
                continue;
            }
            self.detach(i);
            let p = &mut self.instances[i].t.pos;
            let v = axis.get(*p) + delta;
            axis.set(p, v);
            n += 1;
        }
        self.save(patch);
        Outcome::of(n)
    }

    /// Deterministic ±`radius` jitter on X and Z, so a rig stops looking
    /// like a spreadsheet. `seed` makes it repeatable.
    pub(crate) fn scatter_selection(&mut self, patch: &Patch, radius: f32, seed: u32) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Address);
        if sel.is_empty() || radius.abs() <= EPS {
            return Outcome::default();
        }
        self.push_undo();
        let mut state = seed | 1;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((state >> 8) as f32 / 16_777_216.0) * 2.0 - 1.0
        };
        for &i in &sel {
            self.detach(i);
            let (dx, dz) = (next() * radius, next() * radius);
            let p = &mut self.instances[i].t.pos;
            *p = *p + v3(dx, 0.0, dz);
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    // ---- mirror / rotate / spread ----

    /// Reflect the selection, optionally leaving the originals behind.
    pub(crate) fn mirror_selection(
        &mut self,
        patch: &Patch,
        plane: MirrorPlane,
        pivot: MirrorPivot,
        as_copy: bool,
    ) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.is_empty() {
            return Outcome::default();
        }
        let Some(c) = self.selection_centroid() else { return Outcome::default() };
        // A single light mirrored about its own centre never moves, so it
        // falls back to the stage plane — which is what the operator meant.
        let pivot = match pivot {
            MirrorPivot::Stage => v3(0.0, c.y, 0.0),
            MirrorPivot::Selection if sel.len() == 1 => v3(0.0, c.y, 0.0),
            MirrorPivot::Selection => c,
        };
        self.push_undo();
        let targets: Vec<usize> = if as_copy {
            let mut made = Vec::with_capacity(sel.len());
            for &i in &sel {
                let mut inst = self.instances[i].clone();
                inst.mount = None;
                inst.truss_mount = None;
                made.push(self.instances.len());
                self.instances.push(inst);
            }
            self.selection = made.iter().copied().collect();
            made
        } else {
            sel.clone()
        };
        for &i in &targets {
            self.detach(i);
            mirror_transform(&mut self.instances[i].t, plane, pivot);
        }
        self.save(patch);
        Outcome::of(targets.len())
    }

    /// Reflect a whole tower or truss across the stage's centre line.
    pub(crate) fn mirror_element(
        &mut self,
        patch: &Patch,
        r: ElementRef,
        plane: MirrorPlane,
    ) -> bool {
        match r {
            ElementRef::Tower(i) => {
                if i >= self.towers.len() {
                    return false;
                }
                self.push_undo();
                let old_yaw = self.towers[i].yaw_deg;
                let tw = &mut self.towers[i];
                match plane {
                    MirrorPlane::X => {
                        tw.pos.x = -tw.pos.x;
                        tw.yaw_deg = -tw.yaw_deg;
                    }
                    MirrorPlane::Z => {
                        tw.pos.z = -tw.pos.z;
                        tw.yaw_deg = 180.0 - tw.yaw_deg;
                    }
                }
                let dyaw = self.towers[i].yaw_deg - old_yaw;
                self.spin_tower_mounts(i, dyaw);
            }
            ElementRef::Truss(i) => {
                if i >= self.trusses.len() {
                    return false;
                }
                self.push_undo();
                let tr = &mut self.trusses[i];
                let (yaw, pitch, roll) = mirror_truss_pose(
                    tr.kind,
                    tr.yaw_deg,
                    tr.arc_deg,
                    tr.pitch_deg,
                    tr.roll_deg,
                    plane,
                );
                match plane {
                    MirrorPlane::X => tr.pos.x = -tr.pos.x,
                    MirrorPlane::Z => tr.pos.z = -tr.pos.z,
                }
                tr.yaw_deg = yaw;
                tr.pitch_deg = pitch;
                tr.roll_deg = roll;
                self.resync_truss_mounts(i);
            }
        }
        self.save(patch);
        true
    }

    /// Turn the whole selection about its centre; each light turns with it.
    pub(crate) fn rotate_selection(&mut self, patch: &Patch, deg: f32) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.is_empty() || deg == 0.0 {
            return Outcome::default();
        }
        let Some(origin) = self.selection_centroid() else { return Outcome::default() };
        self.push_undo();
        let rad = deg.to_radians();
        for &i in &sel {
            self.detach(i);
            let head = self.instance_is_head(patch, i);
            let inst = &mut self.instances[i];
            inst.t.pos = origin + rotate_about(inst.t.pos - origin, v3(0.0, 1.0, 0.0), rad);
            if head {
                inst.t.roll_deg = (inst.t.roll_deg + deg).rem_euclid(360.0);
            } else {
                inst.t.yaw_deg += deg;
            }
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    /// Push the selection out from (or pull it in toward) its centre on the
    /// floor plane. Height is kept.
    pub(crate) fn scale_spread(&mut self, patch: &Patch, k: f32) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.len() < 2 || (k - 1.0).abs() <= EPS {
            return Outcome::default();
        }
        let Some(origin) = self.selection_centroid() else { return Outcome::default() };
        self.push_undo();
        for &i in &sel {
            self.detach(i);
            let p = &mut self.instances[i].t.pos;
            p.x = origin.x + (p.x - origin.x) * k;
            p.z = origin.z + (p.z - origin.z) * k;
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    // ---- arrange ----

    /// Re-lay the selection into `shape` about `centre`.
    pub(crate) fn arrange_selection(
        &mut self,
        patch: &Patch,
        shape: ArrangeShape,
        order: OrderBy,
        centre: V3,
    ) -> Outcome {
        let sel = self.sorted_selection(patch, order);
        if sel.is_empty() {
            return Outcome::default();
        }
        let pts = shape_points(shape, sel.len(), centre);
        self.push_undo();
        for (k, &i) in sel.iter().enumerate() {
            self.detach(i);
            let head = self.instance_is_head(patch, i);
            let (pos, yaw) = pts[k];
            let inst = &mut self.instances[i];
            inst.t.pos = pos;
            if let Some(y) = yaw {
                if !head {
                    inst.t.yaw_deg = y;
                }
            }
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    // ---- aim ----

    /// Point the selection at `target`. Rotation only — mounts are kept, so
    /// a hung light stays hung. Moving heads opt in with `include_heads`.
    pub(crate) fn aim_selection(
        &mut self,
        patch: &Patch,
        set: &Settings,
        target: AimTarget,
        include_heads: bool,
    ) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.is_empty() {
            return Outcome::default();
        }
        let point = match target {
            AimTarget::Point(p) => Some(p),
            AimTarget::StageCentre => Some(v3(0.0, set.stage_h, 0.0)),
            AimTarget::Centroid => self.selection_centroid(),
            _ => None,
        };
        let mut out = Outcome::default();
        let mut writes: Vec<(usize, f32, f32)> = Vec::new();
        for &i in &sel {
            if self.instance_is_head(patch, i) && !include_heads {
                out.skipped += 1;
                continue;
            }
            let t = &self.instances[i].t;
            let angles = match target {
                AimTarget::StraightDown => Some((t.yaw_deg, -90.0)),
                AimTarget::StraightUp => Some((t.yaw_deg, 90.0)),
                AimTarget::Level => Some((t.yaw_deg, 0.0)),
                _ => point.and_then(|p| aim_angles(t.pos, p)),
            };
            match angles {
                Some((yaw, pitch)) => writes.push((i, yaw, pitch)),
                None => out.skipped += 1,
            }
        }
        if writes.is_empty() {
            return out;
        }
        self.push_undo();
        for (i, yaw, pitch) in &writes {
            let t = &mut self.instances[*i].t;
            t.yaw_deg = *yaw;
            t.pitch_deg = *pitch;
        }
        self.save(patch);
        out.changed = writes.len();
        out
    }

    /// Splay the selection's yaw evenly across `spread_deg` about its mean,
    /// ordered left to right. Moving heads keep their base.
    pub(crate) fn fan_selection(&mut self, patch: &Patch, spread_deg: f32) -> Outcome {
        let sel: Vec<usize> = self
            .sorted_selection(patch, OrderBy::PosX)
            .into_iter()
            .filter(|&i| !self.instance_is_head(patch, i))
            .collect();
        if sel.len() < 2 {
            return Outcome::default();
        }
        let mean: f32 =
            sel.iter().map(|&i| self.instances[i].t.yaw_deg).sum::<f32>() / sel.len() as f32;
        self.push_undo();
        let n = sel.len();
        for (k, &i) in sel.iter().enumerate() {
            self.instances[i].t.yaw_deg =
                mean - spread_deg / 2.0 + spread_deg * k as f32 / (n - 1) as f32;
        }
        self.save(patch);
        Outcome::of(n)
    }

    /// Each light aims at the mirror image of its own X — crossed washes.
    pub(crate) fn cross_aim_selection(&mut self, patch: &Patch, set: &Settings) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        let mut writes: Vec<(usize, f32, f32)> = Vec::new();
        let mut out = Outcome::default();
        for &i in &sel {
            if self.instance_is_head(patch, i) {
                out.skipped += 1;
                continue;
            }
            let p = self.instances[i].t.pos;
            match aim_angles(p, v3(-p.x, set.stage_h, p.z)) {
                Some((yaw, pitch)) => writes.push((i, yaw, pitch)),
                None => out.skipped += 1,
            }
        }
        if writes.is_empty() {
            return out;
        }
        self.push_undo();
        for (i, yaw, pitch) in &writes {
            let t = &mut self.instances[*i].t;
            t.yaw_deg = *yaw;
            t.pitch_deg = *pitch;
        }
        self.save(patch);
        out.changed = writes.len();
        out
    }

    // ---- hang ----

    /// Fill `face`'s free slots with the selection, in `order`, by `fill`.
    /// Lights with no slot left are untouched and counted as skipped.
    pub(crate) fn hang_selection(
        &mut self,
        patch: &Patch,
        target: ElementRef,
        face: usize,
        fill: HangFill,
        order: OrderBy,
    ) -> Outcome {
        if !matches!(self.drag, Drag::None) {
            return Outcome::default();
        }
        self.snap_preview.clear();
        let sel = self.sorted_selection(patch, order);
        if sel.is_empty() {
            return Outcome::default();
        }
        let free = self.free_slots(target, face, &sel);
        let k = sel.len().min(free.len());
        if k == 0 {
            return Outcome { changed: 0, skipped: sel.len() };
        }
        let chosen: Vec<usize> = match fill {
            HangFill::FromStart => free[..k].to_vec(),
            HangFill::Centred => free[super::place::centred_slots(free.len(), k)].to_vec(),
            HangFill::Spread => spread_pick(&free, k),
        };
        self.push_undo();
        for (&i, &slot) in sel.iter().zip(&chosen) {
            let t = match target {
                ElementRef::Tower(ti) => SnapTarget::Tower(ti, slot),
                ElementRef::Truss(ti) => SnapTarget::Truss(ti, slot),
            };
            self.snap_preview.insert(i, t);
        }
        self.commit_snap(patch);
        Outcome { changed: chosen.len(), skipped: sel.len() - chosen.len() }
    }

    /// Unhook the selection; the lights stay exactly where they hang.
    pub(crate) fn detach_selection(&mut self, patch: &Patch) -> Outcome {
        let sel: Vec<usize> = self
            .sorted_selection(patch, OrderBy::Selection)
            .into_iter()
            .filter(|&i| {
                self.instances[i].mount.is_some() || self.instances[i].truss_mount.is_some()
            })
            .collect();
        if sel.is_empty() {
            return Outcome::default();
        }
        self.push_undo();
        for &i in &sel {
            self.detach(i);
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    /// Re-glue every hung light in the selection to its slot — the fix after
    /// a truss has been turned or moved by hand.
    pub(crate) fn reseat_selection(&mut self, patch: &Patch) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        let mut towers: Vec<usize> = Vec::new();
        let mut trusses: Vec<usize> = Vec::new();
        let mut n = 0;
        for &i in &sel {
            let inst = &self.instances[i];
            if let Some((t, _)) = inst.mount {
                if !towers.contains(&t) {
                    towers.push(t);
                }
                n += 1;
            }
            if let Some((t, _)) = inst.truss_mount {
                if !trusses.contains(&t) {
                    trusses.push(t);
                }
                n += 1;
            }
        }
        if n == 0 {
            return Outcome::default();
        }
        self.push_undo();
        for ti in towers {
            self.spin_tower_mounts(ti, 0.0);
        }
        for ti in trusses {
            self.spin_truss_mounts(ti, 0.0);
        }
        self.save(patch);
        Outcome::of(n)
    }

    /// Re-index the lights hung on truss `ti` after its shape changed:
    /// `prev_per_face` slots per face became `slot_count()`, so every light
    /// keeps its face and its place along the run, and the ones whose slot
    /// fell off the end come off rather than silently jumping to another
    /// face. Returns how many were unhung.
    pub(crate) fn unmount_overflow(&mut self, ti: usize, prev_per_face: usize) -> usize {
        let Some(tr) = self.trusses.get(ti) else { return 0 };
        let (per_face, faces) = (tr.slot_count(), tr.face_count());
        let prev = prev_per_face.max(1);
        let mut n = 0;
        for inst in &mut self.instances {
            let Some((t, s)) = inst.truss_mount else { continue };
            if t != ti {
                continue;
            }
            let (face, k) = (s / prev, s % prev);
            if face >= faces || k >= per_face {
                inst.truss_mount = None;
                n += 1;
            } else {
                inst.truss_mount = Some((t, face * per_face + k));
            }
        }
        n
    }

    // ---- clipboard / copies ----

    /// Copy the selection's placement, relative to its own centroid.
    pub(crate) fn copy_transform(&self, patch: &Patch) -> Option<TransformClip> {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.is_empty() {
            return None;
        }
        let centroid = self.selection_centroid()?;
        Some(TransformClip {
            centroid,
            items: sel
                .iter()
                .map(|&i| ClipItem {
                    offset: self.instances[i].t.pos - centroid,
                    t: self.instances[i].t.clone(),
                    opacity: self.instances[i].opacity,
                })
                .collect(),
        })
    }

    /// Apply a copied placement: one copied light goes onto every selected
    /// light; a copied group of the same size restores its pattern about the
    /// current centre; any other count repeats the pattern.
    pub(crate) fn paste_transform(
        &mut self,
        patch: &Patch,
        clip: &TransformClip,
        parts: PasteParts,
    ) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.is_empty() || clip.items.is_empty() || !parts.any() {
            return Outcome::default();
        }
        let single = clip.items.len() == 1;
        let centre = if single { clip.centroid } else { self.selection_centroid().unwrap_or(clip.centroid) };
        self.push_undo();
        for (k, &i) in sel.iter().enumerate() {
            let it = &clip.items[k % clip.items.len()];
            if parts.position {
                self.detach(i);
            }
            let inst = &mut self.instances[i];
            if parts.position {
                inst.t.pos = centre + it.offset;
            }
            if parts.rotation {
                inst.t.yaw_deg = it.t.yaw_deg;
                inst.t.pitch_deg = it.t.pitch_deg;
                inst.t.roll_deg = it.t.roll_deg;
            }
            if parts.scale {
                inst.t.scale = it.t.scale;
            }
            if parts.opacity {
                inst.opacity = it.opacity;
            }
        }
        self.save(patch);
        Outcome::of(sel.len())
    }

    /// Give every selected light the anchor's rotation and size.
    pub(crate) fn match_anchor(&mut self, patch: &Patch) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.len() < 2 {
            return Outcome::default();
        }
        let Some(a) = self.anchor_instance(&sel) else { return Outcome::default() };
        let src = self.instances[a].t.clone();
        self.push_undo();
        for &i in &sel {
            if i == a {
                continue;
            }
            let t = &mut self.instances[i].t;
            t.yaw_deg = src.yaw_deg;
            t.pitch_deg = src.pitch_deg;
            t.roll_deg = src.roll_deg;
            t.scale = src.scale;
        }
        self.save(patch);
        Outcome::of(sel.len() - 1)
    }

    /// Copy the selection `count` times, each copy one `offset` further on.
    /// The selection afterwards is the originals plus every copy, so a fresh
    /// row can be aligned or hung straight away.
    pub(crate) fn duplicate_selection_n(
        &mut self,
        patch: &Patch,
        count: usize,
        offset: V3,
    ) -> Outcome {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.is_empty() || count == 0 {
            return Outcome::default();
        }
        self.push_undo();
        let mut made = 0;
        let mut new_sel: HashSet<usize> = sel.iter().copied().collect();
        for k in 1..=count {
            for &i in &sel {
                let mut inst: Instance = self.instances[i].clone();
                inst.mount = None;
                inst.truss_mount = None;
                inst.t.pos = inst.t.pos + offset * k as f32;
                new_sel.insert(self.instances.len());
                self.instances.push(inst);
                made += 1;
            }
        }
        self.selection = new_sel;
        self.save(patch);
        Outcome::of(made)
    }

    // ---- selection helpers (no undo, no save) ----

    /// Swap what is picked for what is not.
    pub(crate) fn invert_selection(&mut self) {
        let old = std::mem::take(&mut self.selection);
        for i in 0..self.instances.len() {
            if !old.contains(&i) {
                self.selection.insert(i);
            }
        }
        self.sel_tower = None;
        self.sel_truss = None;
        self.settle_anchor();
    }

    /// Every light that is not hung on anything.
    pub(crate) fn select_unmounted(&mut self) {
        self.selection.clear();
        for (i, inst) in self.instances.iter().enumerate() {
            if inst.mount.is_none() && inst.truss_mount.is_none() {
                self.selection.insert(i);
            }
        }
        self.sel_tower = None;
        self.sel_truss = None;
        self.settle_anchor();
    }

    /// Every light of the same archetype as the anchor.
    pub(crate) fn select_same_archetype(&mut self, patch: &Patch) {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        let Some(a) = self
            .anchor_instance(&sel)
            .and_then(|i| patch.fixtures.get(self.instances[i].fixture))
            .map(classify)
        else {
            return;
        };
        self.select_by_archetype(patch, &[a], false);
    }

    /// Every light whose archetype is in `kinds`.
    pub(crate) fn select_by_archetype(
        &mut self,
        patch: &Patch,
        kinds: &[Archetype],
        additive: bool,
    ) {
        if !additive {
            self.selection.clear();
        }
        for (i, inst) in self.instances.iter().enumerate() {
            if patch.fixtures.get(inst.fixture).is_some_and(|f| kinds.contains(&classify(f))) {
                self.selection.insert(i);
            }
        }
        self.sel_tower = None;
        self.sel_truss = None;
        self.settle_anchor();
    }

    /// Thin the selection (or the whole rig) to every n-th light by address.
    pub(crate) fn select_every_nth(&mut self, patch: &Patch, n: usize, offset: usize) {
        let pool = if self.selection.is_empty() {
            let mut all: Vec<usize> = (0..self.instances.len()).collect();
            all.sort_by_key(|&i| {
                patch.fixtures.get(self.instances[i].fixture).map_or(0, |f| f.from)
            });
            all
        } else {
            self.sorted_selection(patch, OrderBy::Address)
        };
        let keep = every_nth(&pool, n, offset);
        self.selection = keep.into_iter().collect();
        self.sel_tower = None;
        self.sel_truss = None;
        self.settle_anchor();
    }

    /// Point `last_selected` at something that is actually selected.
    fn settle_anchor(&mut self) {
        let keep = self
            .last_selected
            .filter(|&fi| self.fixture_selected(fi))
            .or_else(|| {
                let mut sel: Vec<usize> = self.selection.iter().copied().collect();
                sel.sort_unstable();
                sel.first().and_then(|&i| self.instances.get(i)).map(|inst| inst.fixture)
            });
        self.last_selected = keep;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::showbuddy::Patch;
    use crate::stage::Settings;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("dmxpress_arrange_{name}.json"))
    }

    fn par() -> &'static crate::profiles::Profile {
        crate::profiles::find("Generic RGBW Par (4ch)").expect("the built-in par")
    }

    fn rig(name: &str, n: usize) -> (StageView, Patch) {
        let patch = Patch {
            fixtures: (0..n)
                .map(|i| par().to_fixture(format!("T {i}"), 1 + (i as u16) * 4))
                .collect(),
            warnings: Vec::new(),
        };
        let mut stage = StageView::new();
        stage.layout_path = temp(name);
        let _ = std::fs::remove_file(&stage.layout_path);
        stage.sync(&patch, &Settings::default());
        stage.select_all_fixtures();
        (stage, patch)
    }

    fn rig_with_head(name: &str, n: usize) -> (StageView, Patch) {
        let head = crate::profiles::find("Intimidator Spot 475ZX (16ch)").expect("a head");
        let mut fixtures: Vec<crate::showbuddy::Fixture> = (0..n)
            .map(|i| par().to_fixture(format!("T {i}"), 1 + (i as u16) * 4))
            .collect();
        fixtures.push(head.to_fixture("Spot".into(), 400));
        let patch = Patch { fixtures, warnings: Vec::new() };
        let mut stage = StageView::new();
        stage.layout_path = temp(name);
        let _ = std::fs::remove_file(&stage.layout_path);
        stage.sync(&patch, &Settings::default());
        stage.select_all_fixtures();
        (stage, patch)
    }

    fn cleanup(stage: &StageView) {
        let _ = std::fs::remove_file(&stage.layout_path);
    }

    fn spread(stage: &mut StageView) {
        for (k, inst) in stage.instances.iter_mut().enumerate() {
            inst.t.pos = v3(k as f32, 4.0, 0.0);
        }
    }

    #[test]
    fn align_values_picks_min_mid_max() {
        let v = [1.0, 4.0, 7.0];
        assert_eq!(align_values(&v, AlignMode::Min), 1.0);
        assert_eq!(align_values(&v, AlignMode::Mid), 4.0);
        assert_eq!(align_values(&v, AlignMode::Max), 7.0);
        assert_eq!(align_values(&[], AlignMode::Mid), 0.0);
    }

    #[test]
    fn distribute_values_spaces_evenly_and_keeps_rank() {
        assert_eq!(distribute_values(&[5.0, 1.0, 3.0], DistributeMode::Even), vec![5.0, 1.0, 3.0]);
        assert_eq!(distribute_values(&[0.0, 1.0, 10.0], DistributeMode::Even), vec![0.0, 5.0, 10.0]);
        assert_eq!(distribute_values(&[0.0, 9.0, 4.0], DistributeMode::Gap(2.0)), vec![0.0, 4.0, 2.0]);
        assert_eq!(distribute_values(&[3.0], DistributeMode::Even), vec![3.0]);
    }

    #[test]
    fn align_and_distribute_work_on_every_axis() {
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            let (mut stage, patch) = rig(&format!("axis_{}", axis.label()), 3);
            for (k, inst) in stage.instances.iter_mut().enumerate() {
                inst.t.pos = v3(0.0, 4.0, 0.0);
                axis.set(&mut inst.t.pos, [0.0f32, 1.0, 10.0][k]);
            }
            let out = stage.distribute_selection(&patch, axis, DistributeMode::Even);
            assert_eq!(out.changed, 3);
            let mut got: Vec<f32> =
                stage.instances.iter().map(|i| axis.get(i.t.pos)).collect();
            got.sort_by(|a, b| a.partial_cmp(b).unwrap());
            assert!((got[1] - 5.0).abs() < 1e-4, "{axis:?}: {got:?}");
            let out = stage.align_selection(&patch, axis, AlignMode::Min);
            assert_eq!(out.changed, 3);
            assert!(stage.instances.iter().all(|i| axis.get(i.t.pos).abs() < 1e-4));
            // Nothing left to do: no second undo step.
            let depth = stage.undo_stack.len();
            assert_eq!(stage.align_selection(&patch, axis, AlignMode::Min), Outcome::default());
            assert_eq!(stage.undo_stack.len(), depth);
            cleanup(&stage);
        }
    }

    #[test]
    fn mirror_x_flips_position_and_yaw() {
        let mut t = LightTransform {
            pos: v3(2.0, 4.0, 1.0),
            yaw_deg: 30.0,
            pitch_deg: -90.0,
            roll_deg: 10.0,
            scale: 1.0,
        };
        let mut a = t.clone();
        mirror_transform(&mut a, MirrorPlane::X, v3(0.0, 4.0, 0.0));
        assert!((a.pos.x + 2.0).abs() < 1e-5 && (a.pos.z - 1.0).abs() < 1e-5);
        assert!((a.yaw_deg + 30.0).abs() < 1e-5);
        assert!((a.roll_deg - 350.0).abs() < 1e-3);
        assert_eq!(a.pitch_deg, -90.0);
        mirror_transform(&mut t, MirrorPlane::Z, v3(0.0, 4.0, 0.0));
        assert!((t.pos.z + 1.0).abs() < 1e-5 && (t.pos.x - 2.0).abs() < 1e-5);
        assert!((t.yaw_deg - 150.0).abs() < 1e-5);
        // The reflected direction really is the reflection.
        let d = dir_from_angles(30.0, 0.0);
        let m = dir_from_angles(150.0, 0.0);
        assert!((m.x - d.x).abs() < 1e-5 && (m.z + d.z).abs() < 1e-5);
    }

    #[test]
    fn mirror_selection_as_copy_doubles_the_rig() {
        let (mut stage, patch) = rig("mirror_copy", 3);
        spread(&mut stage);
        let out = stage.mirror_selection(&patch, MirrorPlane::X, MirrorPivot::Stage, true);
        assert_eq!(out.changed, 3);
        assert_eq!(stage.instances.len(), 6);
        assert_eq!(stage.selection.len(), 3);
        assert!(stage.selection.iter().all(|&i| i >= 3));
        assert_eq!(stage.undo_stack.len(), 1);
        cleanup(&stage);
    }

    #[test]
    fn mirror_truss_pose_reflects_heading_arc_and_lean() {
        // Straight: the heading just flips across X, and folds across Z.
        let (yaw, pitch, roll) =
            mirror_truss_pose(TrussKind::Straight, 30.0, 90.0, 20.0, 40.0, MirrorPlane::X);
        assert_eq!((yaw, pitch, roll), (-30.0, 20.0, -40.0));
        let (yaw, pitch, roll) =
            mirror_truss_pose(TrussKind::Straight, 30.0, 90.0, 20.0, 40.0, MirrorPlane::Z);
        assert_eq!((yaw, pitch, roll), (150.0, -20.0, -40.0));
        // Radius: the mirrored run starts where the old sweep ended.
        let (yaw, _, _) =
            mirror_truss_pose(TrussKind::Radius, 30.0, 90.0, 0.0, 0.0, MirrorPlane::X);
        assert_eq!(yaw, -120.0);
        let (yaw, _, _) =
            mirror_truss_pose(TrussKind::Radius, 30.0, 90.0, 0.0, 0.0, MirrorPlane::Z);
        assert_eq!(yaw, 60.0);
        // …and the mirrored arc really does cover the mirrored points.
        let mut tr = Truss::radius();
        tr.yaw_deg = 30.0;
        tr.arc_deg = 90.0;
        tr.pos = v3(1.0, 3.0, 0.0);
        let before: Vec<V3> = (0..tr.total_slots()).map(|s| tr.slot_pos(s)).collect();
        let (yaw, pitch, roll) = mirror_truss_pose(
            tr.kind, tr.yaw_deg, tr.arc_deg, tr.pitch_deg, tr.roll_deg, MirrorPlane::X,
        );
        tr.pos.x = -tr.pos.x;
        tr.yaw_deg = yaw;
        tr.pitch_deg = pitch;
        tr.roll_deg = roll;
        let after: Vec<V3> = (0..tr.total_slots()).map(|s| tr.slot_pos(s)).collect();
        for p in &before {
            let want = v3(-p.x, p.y, p.z);
            assert!(
                after.iter().any(|q| (*q - want).len() < 1e-3),
                "no mirrored slot near {want:?} in {after:?}"
            );
        }
    }

    #[test]
    fn mirror_element_moves_a_tower_and_its_lights() {
        let (mut stage, patch) = rig("mirror_el", 2);
        stage.towers.push(Tower::default());
        stage.towers[0].pos = v3(2.0, 0.0, 1.0);
        stage.towers[0].yaw_deg = 30.0;
        stage.instances[0].mount = Some((0, 0));
        stage.clear_selection();
        assert!(stage.mirror_element(&patch, ElementRef::Tower(0), MirrorPlane::X));
        assert!((stage.towers[0].pos.x + 2.0).abs() < 1e-5);
        assert!((stage.towers[0].yaw_deg + 30.0).abs() < 1e-5);
        assert_eq!(stage.instances[0].t.yaw_deg, stage.towers[0].yaw_deg);
        assert_eq!(stage.undo_stack.len(), 1);
        cleanup(&stage);
    }

    #[test]
    fn line_points_are_equally_spaced_and_centred() {
        let c = v3(0.0, 4.0, 0.0);
        let pts = shape_points(ArrangeShape::Line { heading_deg: 90.0, spacing: 1.0 }, 5, c);
        let xs: Vec<f32> = pts.iter().map(|(p, _)| p.x).collect();
        for (k, x) in xs.iter().enumerate() {
            assert!((x - (k as f32 - 2.0)).abs() < 1e-4, "{xs:?}");
        }
        assert!(pts.iter().all(|(p, y)| (p.y - 4.0).abs() < 1e-5 && p.z.abs() < 1e-4 && y.is_none()));
    }

    #[test]
    fn arc_points_lie_on_radius_and_span_sweep() {
        let c = v3(0.0, 4.0, 0.0);
        let shape = ArrangeShape::Arc {
            radius: 3.0,
            heading_deg: 0.0,
            sweep_deg: 90.0,
            facing: Facing::Keep,
        };
        let pts = shape_points(shape, 4, c);
        assert_eq!(pts.len(), 4);
        for (p, _) in &pts {
            assert!(((*p - c).len() - 3.0).abs() < 1e-4);
        }
        let first = angles_from_dir((pts[0].0 - c).norm()).0;
        let last = angles_from_dir((pts[3].0 - c).norm()).0;
        assert!((first + 45.0).abs() < 1e-3, "{first}");
        assert!((last - 45.0).abs() < 1e-3, "{last}");
        // One light sits on the heading itself.
        let one = shape_points(shape, 1, c);
        assert!((angles_from_dir((one[0].0 - c).norm()).0).abs() < 1e-3);
    }

    #[test]
    fn circle_closes_without_duplicate_point() {
        let c = v3(0.0, 4.0, 0.0);
        let pts = shape_points(
            ArrangeShape::Circle { radius: 2.0, start_deg: 0.0, facing: Facing::Keep },
            6,
            c,
        );
        assert_eq!(pts.len(), 6);
        for (a, _) in &pts {
            for (b, _) in &pts {
                if !std::ptr::eq(a, b) {
                    assert!((*a - *b).len() > 1e-3);
                }
            }
        }
        let a0 = angles_from_dir((pts[0].0 - c).norm()).0;
        let a1 = angles_from_dir((pts[1].0 - c).norm()).0;
        assert!(((a1 - a0).rem_euclid(360.0) - 60.0).abs() < 1e-2);
    }

    #[test]
    fn grid_fills_rows_then_columns() {
        let c = v3(0.0, 4.0, 0.0);
        let pts =
            shape_points(ArrangeShape::Grid { cols: 3, spacing_x: 1.0, spacing_z: 2.0 }, 7, c);
        assert_eq!(pts.len(), 7);
        let zs: Vec<f32> = pts.iter().map(|(p, _)| p.z).collect();
        let mut rows: Vec<f32> = zs.clone();
        rows.sort_by(|a, b| a.partial_cmp(b).unwrap());
        rows.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        assert_eq!(rows.len(), 3, "{zs:?}");
        assert!((rows[0] + 2.0).abs() < 1e-4 && (rows[2] - 2.0).abs() < 1e-4, "{rows:?}");
        // The short last row keeps the same columns; it does not re-centre.
        assert!((pts[6].0.x - pts[0].0.x).abs() < 1e-4);
        assert!((pts[0].0.x + 1.0).abs() < 1e-4);
    }

    #[test]
    fn facing_inward_yaws_toward_centre() {
        let c = v3(0.0, 4.0, 0.0);
        for facing in [Facing::Inward, Facing::Outward] {
            let pts = shape_points(
                ArrangeShape::Circle { radius: 2.0, start_deg: 12.0, facing },
                5,
                c,
            );
            for (p, yaw) in &pts {
                let yaw = yaw.expect("a facing yaw");
                let want = if facing == Facing::Inward { c - *p } else { *p - c };
                assert!(dir_from_angles(yaw, 0.0).dot(want.norm()) > 0.999);
            }
        }
        let keep = shape_points(
            ArrangeShape::Arc { radius: 2.0, heading_deg: 0.0, sweep_deg: 90.0, facing: Facing::Keep },
            3,
            c,
        );
        assert!(keep.iter().all(|(_, y)| y.is_none()));
    }

    #[test]
    fn arrange_order_by_address_follows_dmx() {
        let (mut stage, patch) = rig("order", 4);
        // Shuffle the positions so only the address can decide the order.
        for (k, inst) in stage.instances.iter_mut().enumerate() {
            inst.t.pos = v3(10.0 - k as f32, 4.0, 0.0);
        }
        let out = stage.arrange_selection(
            &patch,
            ArrangeShape::Line { heading_deg: 90.0, spacing: 1.0 },
            OrderBy::Address,
            v3(0.0, 4.0, 0.0),
        );
        assert_eq!(out.changed, 4);
        // Fixture 0 has the lowest address, so it takes the leftmost point.
        assert!((stage.instances[0].t.pos.x + 1.5).abs() < 1e-4);
        assert!((stage.instances[3].t.pos.x - 1.5).abs() < 1e-4);
        cleanup(&stage);
    }

    #[test]
    fn aim_angles_below_is_straight_down() {
        let (_, p) = aim_angles(v3(0.0, 4.0, 0.0), v3(0.0, 0.0, 0.0)).unwrap();
        assert!((p + 90.0).abs() < 1e-3);
        let (y, p) = aim_angles(v3(0.0, 4.0, 0.0), v3(1.0, 4.0, 0.0)).unwrap();
        assert!((y - 90.0).abs() < 1e-3 && p.abs() < 1e-3);
        let (y, p) = aim_angles(v3(0.0, 4.0, 0.0), v3(0.0, 4.0, 1.0)).unwrap();
        assert!(y.abs() < 1e-3 && p.abs() < 1e-3);
        assert!(aim_angles(v3(1.0, 1.0, 1.0), v3(1.0, 1.0, 1.0)).is_none());
        // The angles round-trip through the direction the stage draws with.
        let from = v3(-2.0, 5.0, 3.0);
        let to = v3(1.0, 1.0, -2.0);
        let (y, p) = aim_angles(from, to).unwrap();
        assert!(dir_from_angles(y, p).dot((to - from).norm()) > 0.9999);
    }

    #[test]
    fn aim_selection_leaves_heads_alone_unless_asked() {
        let (mut stage, patch) = rig_with_head("aim_heads", 3);
        spread(&mut stage);
        let set = Settings::default();
        let out = stage.aim_selection(&patch, &set, AimTarget::StageCentre, false);
        assert_eq!((out.changed, out.skipped), (3, 1));
        let head = stage.instances.len() - 1;
        assert_eq!(stage.instances[head].t.pitch_deg, set.default_pitch);
        let out = stage.aim_selection(&patch, &set, AimTarget::StageCentre, true);
        assert_eq!((out.changed, out.skipped), (4, 0));
        // Straight down / up / level only touch the pitch.
        stage.aim_selection(&patch, &set, AimTarget::StraightUp, false);
        assert!((stage.instances[0].t.pitch_deg - 90.0).abs() < 1e-4);
        stage.aim_selection(&patch, &set, AimTarget::Level, false);
        assert!(stage.instances[0].t.pitch_deg.abs() < 1e-4);
        // Aiming keeps mounts: it is a rotation, not a move.
        stage.instances[0].truss_mount = Some((0, 1));
        stage.aim_selection(&patch, &set, AimTarget::StraightDown, false);
        assert_eq!(stage.instances[0].truss_mount, Some((0, 1)));
        cleanup(&stage);
    }

    #[test]
    fn aim_pan_tilt_roundtrip() {
        let t = LightTransform {
            pos: v3(0.0, 5.0, 0.0),
            yaw_deg: 0.0,
            pitch_deg: -90.0,
            roll_deg: 0.0,
            scale: 1.0,
        };
        let target = v3(2.0, 0.0, 1.0);
        let (pan, tilt) = aim_pan_tilt(&t, 540.0, 270.0, target).expect("in range");
        assert!((0.0..=1.0).contains(&pan) && (0.0..=1.0).contains(&tilt));
        // Rebuild the beam direction exactly as draw.rs does.
        let f = dir_from_angles(t.yaw_deg, t.pitch_deg);
        let helper = if f.y.abs() > 0.9 { v3(0.0, 0.0, 1.0) } else { v3(0.0, 1.0, 0.0) };
        let r0b = helper.cross(f).norm();
        let u0b = f.cross(r0b).norm();
        let (rs, rc) = t.roll_deg.to_radians().sin_cos();
        let r0 = r0b * rc + u0b * rs;
        let u0 = u0b * rc - r0b * rs;
        let pan_a = ((pan - 0.5) * 540.0).to_radians();
        let p_r = r0 * pan_a.cos() + u0 * pan_a.sin();
        let p_u = p_r.cross(f).norm();
        let tilt_a = ((tilt - 0.5) * 270.0).to_radians();
        let head_dir = (f * tilt_a.cos() + p_u * tilt_a.sin()).norm();
        assert!(head_dir.dot((target - t.pos).norm()) > 0.999, "{head_dir:?}");
        // A head that cannot swing that far, and a fixture with no movement.
        assert!(aim_pan_tilt(&t, 540.0, 10.0, target).is_none());
        assert!(aim_pan_tilt(&t, 0.0, 270.0, target).is_none());
        assert!(aim_pan_tilt(&t, 540.0, 270.0, t.pos).is_none());
    }

    #[test]
    fn hang_fills_free_slots_and_skips_taken() {
        let (mut stage, patch) = rig("hang", 8);
        let mut tr = Truss::straight();
        tr.length = 3.0;
        stage.trusses.push(tr);
        assert_eq!(stage.trusses[0].slot_count(), 6);
        // One light already holds slot 2 and is not part of the hang.
        stage.instances[7].truss_mount = Some((0, 2));
        stage.selection.remove(&7);
        let out = stage.hang_selection(&patch, ElementRef::Truss(0), 0, HangFill::FromStart, OrderBy::Address);
        assert_eq!((out.changed, out.skipped), (5, 2));
        let mut slots: Vec<usize> = (0..7)
            .filter_map(|i| stage.instances[i].truss_mount.map(|(_, s)| s))
            .collect();
        slots.sort_unstable();
        assert_eq!(slots, vec![0, 1, 3, 4, 5]);
        for i in 0..7 {
            if let Some((_, s)) = stage.instances[i].truss_mount {
                let want = stage.trusses[0].slot_pos_yaw(s);
                assert!((stage.instances[i].t.pos - want.0).len() < 1e-4);
                assert!((stage.instances[i].t.pitch_deg - want.2).abs() < 1e-3);
            }
        }
        // Spread with 4 lights over 6 free slots takes both ends.
        let (mut stage, patch) = rig("hang_spread", 4);
        let mut tr = Truss::straight();
        tr.length = 3.0;
        stage.trusses.push(tr);
        let out = stage.hang_selection(&patch, ElementRef::Truss(0), 0, HangFill::Spread, OrderBy::Address);
        assert_eq!(out.changed, 4);
        let mut slots: Vec<usize> =
            stage.instances.iter().filter_map(|i| i.truss_mount.map(|(_, s)| s)).collect();
        slots.sort_unstable();
        assert_eq!(slots, vec![0, 2, 3, 5]);
        // Centred takes the middle run.
        let (mut stage2, patch2) = rig("hang_centred", 2);
        let mut tr = Truss::straight();
        tr.length = 3.0;
        stage2.trusses.push(tr);
        stage2.hang_selection(&patch2, ElementRef::Truss(0), 0, HangFill::Centred, OrderBy::Address);
        let mut slots: Vec<usize> =
            stage2.instances.iter().filter_map(|i| i.truss_mount.map(|(_, s)| s)).collect();
        slots.sort_unstable();
        assert_eq!(slots, vec![2, 3]);
        cleanup(&stage2);
        // Mid-drag it refuses outright.
        let depth = stage.undo_stack.len();
        stage.drag = Drag::Move;
        assert_eq!(
            stage.hang_selection(&patch, ElementRef::Truss(0), 1, HangFill::Spread, OrderBy::Address),
            Outcome::default()
        );
        assert_eq!(stage.undo_stack.len(), depth);
        cleanup(&stage);
    }

    #[test]
    fn spread_pick_takes_both_ends() {
        assert_eq!(spread_pick(&[0, 1, 2, 3, 4, 5], 4), vec![0, 2, 3, 5]);
        assert_eq!(spread_pick(&[0, 1, 2], 1), vec![1]);
        assert_eq!(spread_pick(&[], 3), Vec::<usize>::new());
        assert_eq!(spread_pick(&[7, 8], 5), vec![7, 8]);
    }

    #[test]
    fn nudge_detaches_only_when_moving() {
        let (mut stage, patch) = rig("nudge", 2);
        stage.trusses.push(Truss::straight());
        stage.instances[0].truss_mount = Some((0, 1));
        stage.instances[0].mount = None;
        stage.nudge_selection(&patch, V3::default(), 10.0, 0.0);
        assert_eq!(stage.instances[0].truss_mount, Some((0, 1)));
        assert!((stage.instances[0].t.yaw_deg - 10.0).abs() < 1e-4);
        stage.nudge_selection(&patch, v3(0.1, 0.0, 0.0), 0.0, 0.0);
        assert_eq!(stage.instances[0].truss_mount, None);
        assert_eq!(stage.instances[0].mount, None);
        cleanup(&stage);
    }

    #[test]
    fn nudge_element_spins_mounts() {
        let (mut stage, patch) = rig("nudge_el", 1);
        stage.towers.push(Tower::default());
        stage.instances[0].mount = Some((0, 0));
        stage.instances[0].t.roll_deg = 0.0;
        stage.clear_selection();
        stage.sel_tower = Some(0);
        assert!(stage.nudge_element(&patch, v3(0.5, 0.0, 0.0), 90.0));
        assert_eq!(stage.instances[0].t.yaw_deg, stage.towers[0].yaw_deg);
        assert!((stage.instances[0].t.roll_deg - 90.0).abs() < 1e-4);
        assert!((stage.towers[0].pos.x - 0.5).abs() < 1e-4);
        assert!(!stage.nudge_element(&patch, V3::default(), 0.0));
        cleanup(&stage);
    }

    #[test]
    fn undo_checkpoint_pushes_once_per_gesture() {
        let (mut stage, patch) = rig("checkpoint", 2);
        spread(&mut stage);
        stage.save(&patch);
        let before: Vec<V3> = stage.instances.iter().map(|i| i.t.pos).collect();
        let mut open = false;
        let pre = stage.edit_pre();
        stage.instances[0].t.pos.x += 1.0;
        stage.undo_checkpoint(&mut open, pre);
        assert!(open);
        let pre = stage.edit_pre();
        stage.instances[0].t.pos.x += 1.0;
        stage.undo_checkpoint(&mut open, pre);
        assert_eq!(stage.undo_stack.len(), 1);
        assert!((stage.instances[0].t.pos.x - before[0].x - 2.0).abs() < 1e-4);
        stage.undo(&patch);
        assert!((stage.instances[0].t.pos.x - before[0].x).abs() < 1e-4);
        cleanup(&stage);
    }

    /// Each tool is one undo step on its own: the stack grows by exactly
    /// one, and undoing it puts every light back where it was. The stack is
    /// cleared between tools because its real depth is only ten.
    #[test]
    fn every_tool_pushes_exactly_one_undo() {
        macro_rules! one_step {
            ($stage:expr, $patch:expr, $what:expr, $call:expr) => {{
                $stage.undo_stack.clear();
                let before: Vec<V3> = $stage.instances.iter().map(|i| i.t.pos).collect();
                let _ = $call;
                assert_eq!($stage.undo_stack.len(), 1, "{} is not one undo step", $what);
                assert!($stage.undo(&$patch), "{} left nothing to undo", $what);
                assert_eq!($stage.instances.len(), before.len(), "{} did not undo", $what);
                for (inst, p) in $stage.instances.iter().zip(&before) {
                    assert!((inst.t.pos - *p).len() < 1e-4, "{} did not undo cleanly", $what);
                }
            }};
        }
        let (mut stage, patch) = rig("one_undo", 4);
        spread(&mut stage);
        stage.trusses.push(Truss::straight());
        let set = Settings::default();
        one_step!(stage, patch, "align", stage.align_selection(&patch, Axis::X, AlignMode::Mid));
        one_step!(
            stage,
            patch,
            "distribute",
            stage.distribute_selection(&patch, Axis::X, DistributeMode::Gap(2.0))
        );
        one_step!(stage, patch, "centre", stage.centre_selection_on_stage(&patch));
        one_step!(
            stage,
            patch,
            "mirror",
            stage.mirror_selection(&patch, MirrorPlane::Z, MirrorPivot::Stage, false)
        );
        one_step!(
            stage,
            patch,
            "mirror as copy",
            stage.mirror_selection(&patch, MirrorPlane::X, MirrorPivot::Stage, true)
        );
        if !stage.all_selected() {
            stage.select_all_fixtures();
        }
        one_step!(stage, patch, "rotate", stage.rotate_selection(&patch, 45.0));
        one_step!(stage, patch, "spread", stage.scale_spread(&patch, 1.1));
        one_step!(
            stage,
            patch,
            "arrange",
            stage.arrange_selection(
                &patch,
                ArrangeShape::Circle { radius: 2.0, start_deg: 0.0, facing: Facing::Inward },
                OrderBy::Address,
                v3(0.0, 4.0, 0.0),
            )
        );
        one_step!(
            stage,
            patch,
            "aim",
            stage.aim_selection(&patch, &set, AimTarget::StageCentre, false)
        );
        one_step!(stage, patch, "fan", stage.fan_selection(&patch, 60.0));
        one_step!(stage, patch, "cross", stage.cross_aim_selection(&patch, &set));
        one_step!(stage, patch, "height", stage.set_selection_height(&patch, 2.5));
        one_step!(stage, patch, "snap", stage.snap_selection_to_grid(&patch, 0.4));
        one_step!(
            stage,
            patch,
            "stagger",
            stage.stagger_selection(&patch, Axis::Y, 0.5, OrderBy::Address)
        );
        one_step!(stage, patch, "scatter", stage.scatter_selection(&patch, 0.3, 7));
        one_step!(
            stage,
            patch,
            "nudge",
            stage.nudge_selection(&patch, v3(0.1, 0.0, 0.0), 0.0, 0.0)
        );
        one_step!(
            stage,
            patch,
            "duplicate",
            stage.duplicate_selection_n(&patch, 1, v3(0.6, 0.0, 0.0))
        );
        if !stage.all_selected() {
            stage.select_all_fixtures();
        }
        one_step!(
            stage,
            patch,
            "hang",
            stage.hang_selection(&patch, ElementRef::Truss(0), 0, HangFill::Spread, OrderBy::Address)
        );
        stage.hang_selection(&patch, ElementRef::Truss(0), 0, HangFill::Spread, OrderBy::Address);
        one_step!(stage, patch, "reseat", stage.reseat_selection(&patch));
        one_step!(stage, patch, "detach", stage.detach_selection(&patch));
        one_step!(stage, patch, "match", stage.match_anchor(&patch));
        one_step!(
            stage,
            patch,
            "rotation",
            stage.set_selection_rotation(&patch, Some(10.0), None, None)
        );
        one_step!(stage, patch, "scale", stage.set_selection_scale(&patch, 1.2));
        one_step!(stage, patch, "opacity", stage.set_selection_opacity(&patch, 0.5));
        one_step!(
            stage,
            patch,
            "reset rotation",
            stage.reset_selection_rotation(&patch, &set)
        );
        one_step!(
            stage,
            patch,
            "position",
            stage.set_selection_position(&patch, v3(1.0, 3.0, -1.0))
        );
        let clip = stage.copy_transform(&patch).unwrap();
        stage.nudge_selection(&patch, v3(2.0, 0.0, 0.0), 0.0, 0.0);
        one_step!(
            stage,
            patch,
            "paste",
            stage.paste_transform(&patch, &clip, PasteParts::default())
        );
        cleanup(&stage);
    }

    /// The three transform setters used to `push_undo` on any non-empty
    /// selection, so a reset button clicked when the value was already the
    /// default spent one of the ten undo steps on a write that changed
    /// nothing — ten clicks drained the operator's real edits out of the
    /// stack.
    #[test]
    fn transform_setters_cost_no_undo_step_when_nothing_changes() {
        let (mut stage, patch) = rig("noop_setters", 3);
        let set = Settings::default();
        stage.set_selection_scale(&patch, 1.4);
        stage.set_selection_opacity(&patch, 0.4);
        stage.set_selection_rotation(&patch, Some(12.0), Some(-3.0), Some(30.0));
        stage.undo_stack.clear();
        for _ in 0..12 {
            assert_eq!(stage.set_selection_scale(&patch, 1.4), Outcome::default());
            assert_eq!(stage.set_selection_opacity(&patch, 0.4), Outcome::default());
            assert_eq!(
                stage.set_selection_rotation(&patch, Some(12.0), Some(-3.0), Some(30.0)),
                Outcome::default()
            );
            // Roll is stored modulo 360, so the same angle round the clock
            // is the same angle.
            assert_eq!(
                stage.set_selection_rotation(&patch, None, None, Some(390.0)),
                Outcome::default()
            );
        }
        assert!(stage.undo_stack.is_empty(), "a no-op filled the undo stack");
        // The reset chips go through the same three, once each.
        stage.reset_selection_rotation(&patch, &set);
        stage.set_selection_scale(&patch, 1.0);
        stage.set_selection_opacity(&patch, 1.0);
        assert_eq!(stage.undo_stack.len(), 3);
        assert_eq!(stage.reset_selection_rotation(&patch, &set), Outcome::default());
        assert_eq!(stage.set_selection_scale(&patch, 1.0), Outcome::default());
        assert_eq!(stage.set_selection_opacity(&patch, 1.0), Outcome::default());
        assert_eq!(stage.undo_stack.len(), 3);
        // A real change still pushes exactly one.
        assert_eq!(stage.set_selection_scale(&patch, 1.5).changed, 3);
        assert_eq!(stage.undo_stack.len(), 4);
        cleanup(&stage);
    }

    /// One held-arrow run is one undo step, and the run ends wherever the
    /// key is let go: the disarm used to live inside `nudge_keys`, which
    /// only runs while the stage (or the Nudge card) is hovered, so a key
    /// released off the stage left the flag armed and the whole next run
    /// moved lights with no undo step behind it.
    #[test]
    fn a_run_released_off_the_stage_still_arms_the_next_undo_step() {
        let (mut stage, patch) = rig("nudge_arm", 2);
        spread(&mut stage);
        let ctx = egui::Context::default();
        let frame = |ctx: &egui::Context, events: Vec<egui::Event>, body: &mut dyn FnMut(&egui::Ui)| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| body(ui));
            });
        };
        let arrow = |pressed: bool| egui::Event::Key {
            key: egui::Key::ArrowUp,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        // 1. Over the stage, the arrow goes down: one undo step.
        frame(&ctx, vec![arrow(true)], &mut |ui| {
            stage.nudge_keys(ui, &patch);
            stage.release_nudge_keys(ui);
        });
        assert_eq!(stage.undo_stack.len(), 1);
        assert!(stage.nudge_key_armed);
        // 2. The pointer leaves the stage — `nudge_keys` no longer runs —
        //    and the key is let go there.
        frame(&ctx, vec![arrow(false)], &mut |ui| stage.release_nudge_keys(ui));
        assert!(!stage.nudge_key_armed, "the run never ended");
        // 3. Back over the stage, a fresh press: its own undo step.
        frame(&ctx, vec![arrow(true)], &mut |ui| {
            stage.nudge_keys(ui, &patch);
            stage.release_nudge_keys(ui);
        });
        assert_eq!(stage.undo_stack.len(), 2, "the second run pushed no undo step");
        cleanup(&stage);
    }

    #[test]
    fn empty_selection_is_a_noop() {
        let (mut stage, patch) = rig("noop", 3);
        let set = Settings::default();
        stage.clear_selection();
        let clip = TransformClip {
            centroid: V3::default(),
            items: vec![ClipItem {
                offset: V3::default(),
                t: stage.instances[0].t.clone(),
                opacity: 1.0,
            }],
        };
        let outs = [
            stage.align_selection(&patch, Axis::X, AlignMode::Min),
            stage.distribute_selection(&patch, Axis::Z, DistributeMode::Even),
            stage.centre_selection_on_stage(&patch),
            stage.mirror_selection(&patch, MirrorPlane::X, MirrorPivot::Stage, false),
            stage.rotate_selection(&patch, 90.0),
            stage.scale_spread(&patch, 1.1),
            stage.arrange_selection(
                &patch,
                ArrangeShape::Line { heading_deg: 0.0, spacing: 1.0 },
                OrderBy::Address,
                V3::default(),
            ),
            stage.aim_selection(&patch, &set, AimTarget::StageCentre, true),
            stage.fan_selection(&patch, 60.0),
            stage.cross_aim_selection(&patch, &set),
            stage.set_selection_height(&patch, 3.0),
            stage.snap_selection_to_grid(&patch, 0.5),
            stage.stagger_selection(&patch, Axis::Y, 0.5, OrderBy::Address),
            stage.scatter_selection(&patch, 0.3, 1),
            stage.nudge_selection(&patch, v3(1.0, 0.0, 0.0), 0.0, 0.0),
            stage.detach_selection(&patch),
            stage.reseat_selection(&patch),
            stage.match_anchor(&patch),
            stage.paste_transform(&patch, &clip, PasteParts::default()),
            stage.duplicate_selection_n(&patch, 2, v3(1.0, 0.0, 0.0)),
            stage.set_selection_position(&patch, v3(1.0, 1.0, 1.0)),
            stage.set_selection_rotation(&patch, Some(1.0), None, None),
            stage.set_selection_scale(&patch, 2.0),
            stage.set_selection_opacity(&patch, 0.2),
        ];
        for (k, out) in outs.iter().enumerate() {
            assert_eq!(*out, Outcome::default(), "tool {k} did something");
        }
        assert!(stage.undo_stack.is_empty());
        assert!(stage.copy_transform(&patch).is_none());
        cleanup(&stage);
    }

    #[test]
    fn paste_single_onto_many_and_pattern() {
        let (mut stage, patch) = rig("paste", 3);
        spread(&mut stage);
        // One light copied: its rotation lands on everything, positions stay.
        stage.instances[0].t.yaw_deg = 42.0;
        stage.selection = [0].into_iter().collect();
        let clip = stage.copy_transform(&patch).unwrap();
        stage.select_all_fixtures();
        let before: Vec<V3> = stage.instances.iter().map(|i| i.t.pos).collect();
        let parts = PasteParts { position: false, rotation: true, scale: false, opacity: false };
        assert_eq!(stage.paste_transform(&patch, &clip, parts).changed, 3);
        assert!(stage.instances.iter().all(|i| (i.t.yaw_deg - 42.0).abs() < 1e-4));
        assert!(stage.instances.iter().zip(&before).all(|(i, p)| (i.t.pos - *p).len() < 1e-6));
        // A whole group copied: the pattern restores about the new centre.
        let clip = stage.copy_transform(&patch).unwrap();
        stage.nudge_selection(&patch, v3(5.0, 0.0, 2.0), 0.0, 0.0);
        let out = stage.paste_transform(&patch, &clip, PasteParts::default());
        assert_eq!(out.changed, 3);
        let c = stage.selection_centroid().unwrap();
        for (k, inst) in stage.instances.iter().enumerate() {
            assert!((inst.t.pos - (c + clip.items[k].offset)).len() < 1e-4);
        }
        assert!((c.x - 6.0).abs() < 1e-3, "the group kept its new centre: {c:?}");
        cleanup(&stage);
    }

    #[test]
    fn duplicate_n_selects_originals_and_copies() {
        let (mut stage, patch) = rig("dup", 2);
        spread(&mut stage);
        let out = stage.duplicate_selection_n(&patch, 3, v3(1.0, 0.0, 0.0));
        assert_eq!(out.changed, 6);
        assert_eq!(stage.instances.len(), 8);
        assert_eq!(stage.selection.len(), 8);
        assert!((stage.instances[7].t.pos.x - (1.0 + 3.0)).abs() < 1e-4);
        assert_eq!(stage.undo_stack.len(), 1);
        cleanup(&stage);
    }

    #[test]
    fn every_nth_keeps_phase() {
        assert_eq!(every_nth(&[0, 1, 2, 3, 4, 5], 2, 1), vec![1, 3, 5]);
        assert_eq!(every_nth(&[0, 1, 2, 3, 4, 5], 3, 0), vec![0, 3]);
        assert_eq!(every_nth(&[9], 4, 2), Vec::<usize>::new());
        let (mut stage, patch) = rig("nth", 6);
        stage.select_every_nth(&patch, 2, 0);
        assert_eq!(stage.selection.len(), 3);
        let mut got: Vec<usize> = stage.selection.iter().copied().collect();
        got.sort_unstable();
        assert_eq!(got, vec![0, 2, 4]);
        cleanup(&stage);
    }

    #[test]
    fn invert_unmounted_archetype_selection() {
        let (mut stage, patch) = rig_with_head("sel_helpers", 4);
        stage.select_by_archetype(&patch, &[Archetype::MovingPar, Archetype::Beam], false);
        assert_eq!(stage.selection.len(), 1);
        stage.invert_selection();
        assert_eq!(stage.selection.len(), 4);
        stage.trusses.push(Truss::straight());
        stage.instances[0].truss_mount = Some((0, 0));
        stage.select_unmounted();
        assert_eq!(stage.selection.len(), 4);
        assert!(!stage.selection.contains(&0));
        // Same kind picks up the archetype of the anchor.
        stage.selection = [0].into_iter().collect();
        stage.last_selected = Some(0);
        stage.select_same_archetype(&patch);
        assert_eq!(stage.selection.len(), 4);
        cleanup(&stage);
    }

    #[test]
    fn summary_counts_heads_and_mounts() {
        let (mut stage, patch) = rig_with_head("summary", 3);
        spread(&mut stage);
        stage.trusses.push(Truss::straight());
        stage.instances[0].truss_mount = Some((0, 2));
        let s = stage.selection_summary(&patch);
        assert_eq!((s.lights, s.fixtures, s.heads, s.mounted), (4, 4, 1, 1));
        assert_eq!(s.names, vec![("T".to_string(), 3), ("Spot".to_string(), 1)]);
        assert_eq!(s.first_mount, Some((ElementRef::Truss(0), 2)));
        assert!(s.bounds.is_some());
        stage.clear_selection();
        stage.sel_truss = Some(0);
        let s = stage.selection_summary(&patch);
        assert_eq!(s.element, Some((ElementRef::Truss(0), 1, 24)));
        cleanup(&stage);
    }

    /// The tools, seen from the stage camera: a scattered rig, then the same
    /// rig arranged in a circle facing inward, mirrored across the stage's
    /// centre line, and aimed at the middle of the floor. Written to
    /// `target/arrange3d_*.png`, with the maths asserted alongside so a
    /// picture that looks right is right.
    #[test]
    fn arrange_tools_land_where_intended_headless() {
        use crate::net::DMX_SLOTS;
        use crate::stage::headless::{render_frames, save};

        let (mut stage, patch) = rig("render3d", 8);
        let set = Settings::default();
        // A believable mess to start from.
        for (k, inst) in stage.instances.iter_mut().enumerate() {
            let a = k as f32;
            inst.t.pos = v3(-3.5 + a * 0.9, 3.0 + (a * 0.7).sin() * 1.2, -2.0 + (a * 1.3).cos());
            inst.t.pitch_deg = -90.0;
            inst.t.yaw_deg = 0.0;
        }
        let gobos = crate::gobo::Catalogue::load();
        let buf = [170u8; DMX_SLOTS];
        let size = [1200, 700];
        let shot = |stage: &mut StageView, name: &str| -> bool {
            let mut settings = Settings::default();
            let Some(pixels) = render_frames(3, size, |ctx, _| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    stage.ui(ui, &patch, &buf, &gobos, 640.0, &mut settings, None, None, None);
                });
            }) else {
                eprintln!("no GPU adapter — skipping");
                return false;
            };
            save(&pixels, size, name);
            true
        };

        if !shot(&mut stage, "arrange3d_before") {
            cleanup(&stage);
            return;
        }

        // 1. A circle of radius 3 about (0, 4, 0), every light facing in.
        let centre = v3(0.0, 4.0, 0.0);
        stage.arrange_selection(
            &patch,
            ArrangeShape::Circle { radius: 3.0, start_deg: 0.0, facing: Facing::Inward },
            OrderBy::Address,
            centre,
        );
        for inst in &stage.instances {
            let d = inst.t.pos - centre;
            assert!((d.len() - 3.0).abs() < 1e-3, "off the circle: {:?}", inst.t.pos);
            assert!((inst.t.pos.y - 4.0).abs() < 1e-4, "off the ring's height");
            let facing = dir_from_angles(inst.t.yaw_deg, 0.0);
            assert!(facing.dot((centre - inst.t.pos).norm()) > 0.999, "not facing the middle");
        }
        shot(&mut stage, "arrange3d_circle");

        // 2. Shift the ring stage left, then mirror it across the centre line.
        stage.nudge_selection(&patch, v3(-3.0, 0.0, 0.0), 0.0, 0.0);
        let before: Vec<V3> = stage.instances.iter().map(|i| i.t.pos).collect();
        assert!(before.iter().all(|p| p.x < 0.5), "the ring should sit stage left");
        stage.mirror_selection(&patch, MirrorPlane::X, MirrorPivot::Stage, false);
        for (inst, p) in stage.instances.iter().zip(&before) {
            assert!((inst.t.pos.x + p.x).abs() < 1e-4, "x did not reflect");
            assert!((inst.t.pos.z - p.z).abs() < 1e-4, "z moved in an X mirror");
        }
        assert!(stage.instances.iter().all(|i| i.t.pos.x > -0.5), "the ring should now sit stage right");
        shot(&mut stage, "arrange3d_mirror");

        // 3. Aim the lot at the middle of the stage floor.
        let target = v3(0.0, set.stage_h, 0.0);
        let out = stage.aim_selection(&patch, &set, AimTarget::StageCentre, false);
        assert_eq!((out.changed, out.skipped), (8, 0));
        for inst in &stage.instances {
            let beam = dir_from_angles(inst.t.yaw_deg, inst.t.pitch_deg);
            let want = (target - inst.t.pos).norm();
            assert!(beam.dot(want) > 0.9999, "beam {beam:?} misses the stage centre");
            assert!(inst.t.pitch_deg < 0.0, "a light above the stage must aim downward");
        }
        shot(&mut stage, "arrange3d_aim");
        cleanup(&stage);
    }

    #[test]
    fn truss_shrink_unmounts_overflow() {
        let (mut stage, _patch) = rig("overflow", 2);
        let mut tr = Truss::straight();
        tr.length = 3.0;
        stage.trusses.push(tr);
        // Slot 5 is the far end of the top face; slot 6 is the bottom face's
        // first slot, which must stay on the bottom face after the shrink.
        stage.instances[0].truss_mount = Some((0, 5));
        stage.instances[1].truss_mount = Some((0, 6));
        stage.trusses[0].length = 1.0;
        assert_eq!(stage.trusses[0].slot_count(), 2);
        assert_eq!(stage.unmount_overflow(0, 6), 1);
        assert_eq!(stage.instances[0].truss_mount, None);
        assert_eq!(stage.instances[1].truss_mount, Some((0, 2)));
        assert_eq!(stage.unmount_overflow(0, 2), 0);
        assert_eq!(stage.unmount_overflow(9, 6), 0);
        cleanup(&stage);
    }
}
