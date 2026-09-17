//! Placement patterns and mounting primitives for the Inspector's Patch
//! tab: where a freshly patched batch lands (line, grid, arc, circle, on the
//! floor), which slots on a tower or truss face are free, how a block of
//! lights centres on them, and the mount label a light carries.
//!
//! Everything here works on explicit instance-index lists, never on the
//! selection, so the Patch wizard (which has just rebuilt the patch and has
//! nothing selected) and the Selection tab can share one implementation.

use serde::{Deserialize, Serialize};

use super::geometry::SnapTarget;
use super::layout::{layout_key, ElementRef, Tower, Truss, TrussKind, TOWER_SLOTS};
use super::math::{angles_from_dir, dir_from_angles, v3, V3};
use super::view::StageView;
use crate::showbuddy::Patch;

/// How high a floor-standing light sits.
const FLOOR_Y: f32 = 0.25;
/// Floor lights are angled up into the rig rather than straight ahead.
const FLOOR_PITCH: f32 = 65.0;
/// How far under an element the lights that found no slot are parked.
const BENEATH: f32 = 0.6;
/// How far downstage each further new truss steps, so two batches hung one
/// after the other do not land inside each other.
const TRUSS_STEP: f32 = 1.2;
/// The same, across the stage, for towers.
const TOWER_STEP: f32 = 1.5;

/// The shape a batch of lights is arranged in when it is patched (or when a
/// selection is re-placed). `Truss` and `Tower` are mounts, not shapes:
/// they produce no positions of their own, the lights clip onto slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) enum PlacePattern {
    /// Leave the lights where the patch put them.
    Keep,
    #[default]
    LineX,
    LineZ,
    Grid,
    Arc,
    Circle,
    Floor,
    Truss,
    Tower,
}

impl PlacePattern {
    pub const ALL: [PlacePattern; 9] = [
        Self::Keep,
        Self::LineX,
        Self::LineZ,
        Self::Grid,
        Self::Arc,
        Self::Circle,
        Self::Floor,
        Self::Truss,
        Self::Tower,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Keep => "Keep",
            Self::LineX => "Line X",
            Self::LineZ => "Line Z",
            Self::Grid => "Grid",
            Self::Arc => "Arc",
            Self::Circle => "Circle",
            Self::Floor => "Floor",
            Self::Truss => "Truss",
            Self::Tower => "Tower",
        }
    }

    /// Whether the lights hang on an element instead of standing free.
    pub fn is_mount(self) -> bool {
        matches!(self, Self::Truss | Self::Tower)
    }
}

/// Everything [`pattern_positions`] and [`StageView::place_instances`] need.
/// One value describes the whole batch; the pattern picks which fields it
/// reads.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlaceSpec {
    pub pattern: PlacePattern,
    /// Centre of the shape: x/z of the run, y the hanging height.
    pub origin: V3,
    /// Metres between neighbours.
    pub spacing: f32,
    /// Cap on the whole run's width, so a big batch shrinks to fit the
    /// stage instead of running off it.
    pub max_extent: Option<f32>,
    /// Z of a line along X (X of a line along Z); the grid's centre row.
    pub row: f32,
    /// Grid columns; 0 = square-ish.
    pub columns: usize,
    pub radius: f32,
    pub arc_deg: f32,
    /// Arc / circle: turn each light toward the centre.
    pub face_centre: bool,
    pub yaw: f32,
    pub pitch: f32,
}

/// Where `n` lights land. Empty for [`PlacePattern::Keep`] and for the two
/// mounting patterns, which go through [`StageView::mount_instances`].
pub(crate) fn pattern_positions(spec: &PlaceSpec, n: usize) -> Vec<V3> {
    if n == 0 || matches!(spec.pattern, PlacePattern::Keep) || spec.pattern.is_mount() {
        return Vec::new();
    }
    // The run is capped, not the gap: twenty lights in eight metres want a
    // 0.4 m pitch whatever the spacing box says.
    let step = match spec.max_extent {
        Some(m) if m > 0.0 => spec.spacing.min(m / n as f32),
        _ => spec.spacing,
    };
    let centred = |k: usize, count: usize| (k as f32 - (count as f32 - 1.0) / 2.0) * step;
    let o = spec.origin;
    match spec.pattern {
        PlacePattern::LineX => {
            (0..n).map(|k| v3(o.x + centred(k, n), o.y, spec.row)).collect()
        }
        PlacePattern::LineZ => {
            (0..n).map(|k| v3(spec.row, o.y, o.z + centred(k, n))).collect()
        }
        PlacePattern::Floor => {
            (0..n).map(|k| v3(o.x + centred(k, n), FLOOR_Y, spec.row)).collect()
        }
        PlacePattern::Grid => {
            // Never wider than the batch: a block centred on a column count
            // the lights cannot fill sits that many half-steps off-origin,
            // and the `max_extent` cap only scales `step`, never the offset.
            let cols = if spec.columns > 0 {
                spec.columns
            } else {
                (n as f32).sqrt().ceil() as usize
            }
            .clamp(1, n);
            let rows = n.div_ceil(cols);
            (0..n)
                .map(|k| {
                    let (r, c) = (k / cols, k % cols);
                    v3(
                        o.x + (c as f32 - (cols as f32 - 1.0) / 2.0) * step,
                        o.y,
                        spec.row + (r as f32 - (rows as f32 - 1.0) / 2.0) * step,
                    )
                })
                .collect()
        }
        PlacePattern::Arc => {
            let r = spec.radius.max(0.1);
            (0..n)
                .map(|k| {
                    let a = if n <= 1 {
                        0.0
                    } else {
                        -spec.arc_deg * 0.5 + spec.arc_deg * k as f32 / (n as f32 - 1.0)
                    };
                    o + dir_from_angles(a, 0.0) * r
                })
                .collect()
        }
        PlacePattern::Circle => {
            let r = spec.radius.max(0.1);
            (0..n)
                .map(|k| o + dir_from_angles(k as f32 * 360.0 / n as f32, 0.0) * r)
                .collect()
        }
        PlacePattern::Keep | PlacePattern::Truss | PlacePattern::Tower => Vec::new(),
    }
}

/// The run [`StageView::add_truss_fitted`] builds, before it is placed on
/// the stage. The Patch tab sizes its "room for all N" hint from the same
/// value, so the promise and the truss can never disagree.
pub(crate) fn fitted_truss(kind: TrussKind, length: Option<f32>) -> Truss {
    let mut t = match kind {
        TrussKind::Straight => Truss::straight(),
        TrussKind::Radius => Truss::radius(),
    };
    if let Some(l) = length {
        t.length = l.clamp(0.5, 12.0);
    }
    t
}

/// The run of `want` slots centred within `n_free` free ones (fewer when
/// there are not enough).
pub(crate) fn centred_slots(n_free: usize, want: usize) -> std::ops::Range<usize> {
    let k = want.min(n_free);
    let start = (n_free - k) / 2;
    start..start + k
}

impl StageView {
    /// The first instance of each of `keys` (`display@from`), in key order;
    /// keys with no fixture or no instance are skipped. The only handle the
    /// Patch wizard has on the lights it just made, because `rebuild_patch`
    /// invalidates every index.
    pub(crate) fn instances_for_keys(&self, patch: &Patch, keys: &[String]) -> Vec<usize> {
        let mut out = Vec::new();
        for key in keys {
            let Some(fi) = patch.fixtures.iter().position(|f| layout_key(f) == *key) else {
                continue;
            };
            if let Some(i) = self.instances.iter().position(|inst| inst.fixture == fi) {
                out.push(i);
            }
        }
        out
    }

    /// Arrange `insts` in `spec`'s shape: one undo step, the layout saved.
    /// A pattern with no positions (Keep, or a mount) changes nothing and
    /// pushes no undo step.
    pub(crate) fn place_instances(&mut self, patch: &Patch, insts: &[usize], spec: &PlaceSpec) {
        if insts.is_empty() || pattern_positions(spec, insts.len()).is_empty() {
            return;
        }
        self.push_undo();
        self.apply_place(insts, spec);
        self.save(patch);
    }

    /// Write the pattern onto the instances. Mounts are cleared first —
    /// `StageView::ui` re-glues a mounted light to its slot every frame, so
    /// a forgotten clear makes the move look like a no-op.
    fn apply_place(&mut self, insts: &[usize], spec: &PlaceSpec) {
        let positions = pattern_positions(spec, insts.len());
        for (k, &i) in insts.iter().enumerate() {
            let Some(&pos) = positions.get(k) else { break };
            let Some(inst) = self.instances.get_mut(i) else { continue };
            inst.mount = None;
            inst.truss_mount = None;
            inst.t.pos = pos;
            inst.t.yaw_deg = if spec.face_centre
                && matches!(spec.pattern, PlacePattern::Arc | PlacePattern::Circle)
            {
                angles_from_dir((spec.origin - pos).norm()).0
            } else {
                spec.yaw
            };
            inst.t.pitch_deg = if spec.pattern == PlacePattern::Floor {
                FLOOR_PITCH
            } else {
                spec.pitch
            };
        }
    }

    /// Slots of `face` on `target` not held by any instance whose index is
    /// outside `exclude`. Tower face 0 = slots 0..4 (top), face 1 = 4..8
    /// (under); truss face `f` = `f*slot_count()..(f+1)*slot_count()`.
    ///
    /// A hidden element offers none: it is not drawn and `compute_snap`
    /// refuses to catch a dragged light on it, so nothing may be hung on it
    /// from the Patch or Selection tabs either.
    pub(crate) fn free_slots(&self, target: ElementRef, face: usize, exclude: &[usize]) -> Vec<usize> {
        let range = match target {
            ElementRef::Tower(i) => {
                if face >= 2 || self.towers.get(i).is_none_or(|tw| tw.hidden) {
                    return Vec::new();
                }
                let per = TOWER_SLOTS / 2;
                face * per..(face + 1) * per
            }
            ElementRef::Truss(i) => {
                let Some(tr) = self.trusses.get(i) else { return Vec::new() };
                if tr.hidden || face >= tr.face_count() {
                    return Vec::new();
                }
                let n = tr.slot_count();
                face * n..(face + 1) * n
            }
        };
        range
            .filter(|&slot| {
                !self.instances.iter().enumerate().any(|(k, inst)| {
                    if exclude.contains(&k) {
                        return false;
                    }
                    match target {
                        ElementRef::Tower(i) => inst.mount == Some((i, slot)),
                        ElementRef::Truss(i) => inst.truss_mount == Some((i, slot)),
                    }
                })
            })
            .collect()
    }

    /// Hang `insts` on one face of an element, through the same
    /// `commit_snap` path a drag gesture uses, so mounted lights behave
    /// exactly like dragged-on ones. Without an `anchor` the block is
    /// centred on the free run; with one the slots nearest it are taken
    /// (the rig card's `+1`). Lights that found no slot are parked in a
    /// line beneath the element and returned.
    pub(crate) fn mount_instances(
        &mut self,
        patch: &Patch,
        insts: &[usize],
        target: ElementRef,
        face: usize,
        anchor: Option<usize>,
    ) -> (usize, Vec<usize>) {
        if insts.is_empty() {
            return (0, Vec::new());
        }
        self.push_undo();
        let free = self.free_slots(target, face, insts);
        let chosen: Vec<usize> = match anchor {
            Some(a) => {
                let mut f = free;
                f.sort_by_key(|s| s.abs_diff(a));
                f.truncate(insts.len());
                f
            }
            None => {
                let run = centred_slots(free.len(), insts.len());
                free[run].to_vec()
            }
        };
        for (k, &i) in insts.iter().enumerate() {
            let Some(&slot) = chosen.get(k) else { break };
            let snap = match target {
                ElementRef::Tower(ti) => SnapTarget::Tower(ti, slot),
                ElementRef::Truss(ti) => SnapTarget::Truss(ti, slot),
            };
            self.snap_preview.insert(i, snap);
        }
        self.commit_snap(patch);
        let leftover: Vec<usize> = insts.iter().skip(chosen.len()).copied().collect();
        if !leftover.is_empty() {
            let (origin, row) = self.beneath(target);
            let spec = PlaceSpec {
                pattern: PlacePattern::LineX,
                origin,
                spacing: 0.5,
                max_extent: None,
                row,
                columns: 0,
                radius: 1.0,
                arc_deg: 0.0,
                face_centre: false,
                yaw: 0.0,
                pitch: -90.0,
            };
            self.apply_place(&leftover, &spec);
            self.save(patch);
        }
        (chosen.len(), leftover)
    }

    /// (origin, row) for the line of lights that did not fit on `target`.
    fn beneath(&self, target: ElementRef) -> (V3, f32) {
        match target {
            ElementRef::Tower(i) => match self.towers.get(i) {
                Some(tw) => (tw.pos + v3(0.0, tw.height - BENEATH, 0.0), tw.pos.z),
                None => (v3(0.0, 2.0, 0.0), 0.0),
            },
            ElementRef::Truss(i) => match self.trusses.get(i) {
                Some(tr) => (tr.pos + v3(0.0, -BENEATH, 0.0), tr.pos.z),
                None => (v3(0.0, 2.0, 0.0), 0.0),
            },
        }
    }

    /// A new truss run, sized to the batch that is about to hang on it,
    /// picked and saved — on disk before `rebuild_patch` re-reads the
    /// layout file. Returns its index.
    ///
    /// Self-contained rather than built on the Build tab's own add-element
    /// helper: this one sets a length and steps each further run downstage
    /// instead of across, so two batches never land inside each other.
    pub(crate) fn add_truss_fitted(
        &mut self,
        patch: &Patch,
        kind: TrussKind,
        length: Option<f32>,
    ) -> usize {
        self.push_undo();
        let mut t = fitted_truss(kind, length);
        t.pos.z += self.trusses.len() as f32 * TRUSS_STEP;
        let ti = self.trusses.len();
        self.trusses.push(t);
        self.sel_truss = Some(ti);
        self.sel_tower = None;
        self.selection.clear();
        self.save(patch);
        ti
    }

    /// A new floor stand for a batch to clip onto, stepped across the stage
    /// from the last one. Returns its index.
    pub(crate) fn add_tower_fitted(&mut self, patch: &Patch) -> usize {
        self.push_undo();
        let mut t = Tower::default();
        t.pos.x = self.towers.len() as f32 * TOWER_STEP - 2.0;
        let ti = self.towers.len();
        self.towers.push(t);
        self.sel_tower = Some(ti);
        self.sel_truss = None;
        self.selection.clear();
        self.save(patch);
        ti
    }

    /// The mounting faces an element offers, in slot order.
    pub(crate) fn element_faces(&self, target: ElementRef) -> &'static [&'static str] {
        match target {
            ElementRef::Truss(i)
                if self.trusses.get(i).is_some_and(|tr| tr.kind == TrussKind::Straight) =>
            {
                &["Top", "Bottom", "Left", "Right"]
            }
            _ => &["Top", "Under"],
        }
    }

    /// (free, total) slots on that face.
    pub(crate) fn element_free(&self, target: ElementRef, face: usize) -> (usize, usize) {
        let total = match target {
            ElementRef::Tower(i) => {
                if i < self.towers.len() && face < 2 {
                    TOWER_SLOTS / 2
                } else {
                    0
                }
            }
            ElementRef::Truss(i) => self
                .trusses
                .get(i)
                .filter(|tr| face < tr.face_count())
                .map_or(0, |tr| tr.slot_count()),
        };
        (self.free_slots(target, face, &[]).len(), total)
    }

    /// "truss 2" / "tower 1" for the first mounted instance of fixture `fi`.
    pub(crate) fn mount_label(&self, fi: usize) -> Option<String> {
        self.instances.iter().find_map(|inst| {
            if inst.fixture != fi {
                return None;
            }
            if let Some((t, _)) = inst.truss_mount {
                Some(format!("truss {}", t + 1))
            } else {
                inst.mount.map(|(t, _)| format!("tower {}", t + 1))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::layout::{Instance, LightTransform};
    use super::super::settings::Settings;

    fn spec(pattern: PlacePattern) -> PlaceSpec {
        PlaceSpec {
            pattern,
            origin: v3(0.0, 4.0, 0.0),
            spacing: 1.0,
            max_extent: None,
            row: 0.0,
            columns: 0,
            radius: 3.0,
            arc_deg: 120.0,
            face_centre: true,
            yaw: 0.0,
            pitch: -90.0,
        }
    }

    /// Three 4-channel lights, so a stage can be synced against them.
    fn par_patch(n: usize) -> Patch {
        let p = crate::profiles::PROFILES
            .iter()
            .find(|p| p.channel_count() == 4)
            .expect("a 4-channel built-in profile");
        Patch {
            fixtures: (0..n)
                .map(|i| p.to_fixture(format!("Par {}", i + 1), 1 + i as u16 * 4))
                .collect(),
            warnings: Vec::new(),
        }
    }

    /// A stage whose `save()` goes to a temp file, never `stage_layout.json`.
    fn temp_stage(patch: &Patch, name: &str) -> StageView {
        let mut view = StageView::new();
        view.layout_path = std::env::temp_dir().join(format!("dmxpress_place_{name}.json"));
        let _ = std::fs::remove_file(&view.layout_path);
        view.instances = patch
            .fixtures
            .iter()
            .enumerate()
            .map(|(fi, _)| Instance {
                fixture: fi,
                t: LightTransform {
                    pos: v3(0.0, 4.0, 0.0),
                    yaw_deg: 0.0,
                    pitch_deg: -90.0,
                    roll_deg: 0.0,
                    scale: 1.0,
                },
                opacity: 1.0,
                mount: None,
                truss_mount: None,
            })
            .collect();
        view
    }

    #[test]
    fn line_positions_are_centred_and_evenly_spaced() {
        let mut s = spec(PlacePattern::LineX);
        s.row = -2.0;
        let p = pattern_positions(&s, 4);
        let xs: Vec<f32> = p.iter().map(|v| v.x).collect();
        assert_eq!(xs, vec![-1.5, -0.5, 0.5, 1.5]);
        assert!(p.iter().all(|v| v.y == 4.0 && v.z == -2.0));
        let z = pattern_positions(&spec(PlacePattern::LineZ), 4);
        assert_eq!(z.iter().map(|v| v.z).collect::<Vec<_>>(), vec![-1.5, -0.5, 0.5, 1.5]);
        assert!(z.iter().all(|v| v.x == 0.0));
    }

    #[test]
    fn fit_caps_spacing_to_max_extent() {
        let mut s = spec(PlacePattern::LineX);
        s.max_extent = Some(8.0);
        let p = pattern_positions(&s, 20);
        let step = p[1].x - p[0].x;
        assert!((step - 0.4).abs() < 1e-5, "step {step}");
        // A batch that already fits keeps the spacing it was given.
        let p = pattern_positions(&s, 4);
        assert!((p[1].x - p[0].x - 1.0).abs() < 1e-5);
    }

    #[test]
    fn grid_positions_use_auto_columns() {
        let p = pattern_positions(&spec(PlacePattern::Grid), 6);
        let cols = p.iter().filter(|v| (v.z - p[0].z).abs() < 1e-5).count();
        assert_eq!(cols, 3, "three across, two deep");
        let rows: std::collections::BTreeSet<i32> =
            p.iter().map(|v| (v.z * 100.0).round() as i32).collect();
        assert_eq!(rows.len(), 2);
        let one = pattern_positions(&spec(PlacePattern::Grid), 1);
        assert_eq!((one[0].x, one[0].z), (0.0, 0.0));
        let mut s = spec(PlacePattern::Grid);
        s.columns = 2;
        let p = pattern_positions(&s, 6);
        assert_eq!(p.iter().filter(|v| (v.z - p[0].z).abs() < 1e-5).count(), 2);
    }

    #[test]
    fn grid_never_centres_on_more_columns_than_there_are_lights() {
        // Six lights, twelve columns asked for: one row centred on the
        // origin, not on a phantom twelve-wide block whose middle sits
        // 5.5 m to the left of it. "Fit" caps `step`, never the offset.
        let mut s = spec(PlacePattern::Grid);
        s.columns = 12;
        s.max_extent = Some(8.0);
        let p = pattern_positions(&s, 6);
        assert_eq!(
            p.iter().map(|v| v.x).collect::<Vec<_>>(),
            vec![-2.5, -1.5, -0.5, 0.5, 1.5, 2.5]
        );
        assert!(p.iter().all(|v| v.z == 0.0), "one row");
        s.columns = 32;
        s.max_extent = None;
        let p = pattern_positions(&s, 4);
        assert_eq!(p.iter().map(|v| v.x).collect::<Vec<_>>(), vec![-1.5, -0.5, 0.5, 1.5]);
        // A block narrower than the count still wraps as asked.
        s.columns = 2;
        let p = pattern_positions(&s, 4);
        assert_eq!(p.iter().map(|v| v.x).collect::<Vec<_>>(), vec![-0.5, 0.5, -0.5, 0.5]);
    }

    #[test]
    fn fitted_truss_sizes_the_run_the_patch_tab_promises() {
        // One slot per light on a fitted straight run, up to twelve metres.
        assert_eq!(fitted_truss(TrussKind::Straight, Some(3.0)).slot_count(), 6);
        assert_eq!(fitted_truss(TrussKind::Straight, Some(1.0)).slot_count(), 2);
        assert_eq!(fitted_truss(TrussKind::Straight, Some(20.0)).slot_count(), 24);
        // A radius run is sized by its arc, never by the batch…
        assert_eq!(fitted_truss(TrussKind::Radius, None).slot_count(), 6);
        assert_eq!(fitted_truss(TrussKind::Radius, Some(12.0)).slot_count(), 6);
        // …and a new tower always has four slots a face.
        assert_eq!(TOWER_SLOTS / 2, 4);
    }

    #[test]
    fn free_slots_counts_the_face_it_was_asked_about() {
        let patch = par_patch(6);
        let mut view = temp_stage(&patch, "faces");
        view.trusses.push(Truss::straight()); // 3 m → 6 slots on each of 4
        for (i, inst) in view.instances.iter_mut().enumerate() {
            inst.truss_mount = Some((0, 6 + i)); // the whole Bottom face
        }
        assert_eq!(view.element_free(ElementRef::Truss(0), 1), (0, 6), "Bottom is full");
        assert_eq!(view.element_free(ElementRef::Truss(0), 2), (6, 6), "Left is empty");
        assert_eq!(view.element_free(ElementRef::Truss(0), 3), (6, 6), "Right is empty");
    }

    #[test]
    fn free_slots_refuses_a_hidden_element() {
        let patch = par_patch(3);
        let mut view = temp_stage(&patch, "hidden");
        view.trusses.push(Truss::straight());
        view.towers.push(Tower::default());
        assert_eq!(view.element_free(ElementRef::Truss(0), 0), (6, 6));
        view.trusses[0].hidden = true;
        view.towers[0].hidden = true;
        // Not drawn, not pickable, not a snap target — so nothing may be
        // hung on it from the Patch tab either.
        assert!(view.free_slots(ElementRef::Truss(0), 0, &[]).is_empty());
        assert!(view.free_slots(ElementRef::Tower(0), 1, &[]).is_empty());
        assert_eq!(view.element_free(ElementRef::Truss(0), 0), (0, 6));
        let (hung, left) = view.mount_instances(&patch, &[0], ElementRef::Truss(0), 0, None);
        assert_eq!((hung, left.as_slice()), (0, &[0usize][..]));
        assert_eq!(view.instances[0].truss_mount, None);
        let _ = std::fs::remove_file(&view.layout_path);
    }

    #[test]
    fn circle_positions_sit_on_the_radius() {
        let p = pattern_positions(&spec(PlacePattern::Circle), 8);
        assert_eq!(p.len(), 8);
        for v in &p {
            let d = ((v.x * v.x) + (v.z * v.z)).sqrt();
            assert!((d - 3.0).abs() < 1e-4, "{d}");
            assert_eq!(v.y, 4.0);
        }
        assert!(p[0].x.abs() < 1e-4 && (p[0].z - 3.0).abs() < 1e-4, "first at angle 0");
    }

    #[test]
    fn arc_of_one_light_sits_at_the_arc_centre() {
        let p = pattern_positions(&spec(PlacePattern::Arc), 1);
        assert_eq!(p.len(), 1);
        assert!(p[0].x.abs() < 1e-4 && (p[0].z - 3.0).abs() < 1e-4);
    }

    #[test]
    fn arc_endpoints_are_mirrored() {
        let p = pattern_positions(&spec(PlacePattern::Arc), 5);
        assert!((p[0].x + p[4].x).abs() < 1e-4, "{} vs {}", p[0].x, p[4].x);
        assert!((p[0].z - p[4].z).abs() < 1e-4);
    }

    #[test]
    fn floor_is_low_and_keep_is_empty() {
        let p = pattern_positions(&spec(PlacePattern::Floor), 3);
        assert!(p.iter().all(|v| (v.y - FLOOR_Y).abs() < 1e-6));
        assert!(pattern_positions(&spec(PlacePattern::Keep), 3).is_empty());
        assert!(pattern_positions(&spec(PlacePattern::Truss), 3).is_empty());
        assert!(pattern_positions(&spec(PlacePattern::Tower), 3).is_empty());
    }

    #[test]
    fn centred_slots_picks_the_middle_block() {
        assert_eq!(centred_slots(6, 4), 1..5);
        assert_eq!(centred_slots(6, 6), 0..6);
        assert_eq!(centred_slots(6, 8), 0..6);
        assert_eq!(centred_slots(0, 3), 0..0);
    }

    #[test]
    fn free_slots_skips_slots_held_by_other_lights() {
        let patch = par_patch(3);
        let mut view = temp_stage(&patch, "free");
        view.trusses.push(Truss::straight()); // 3 m → 6 slots per face
        view.towers.push(Tower::default());
        view.instances[0].truss_mount = Some((0, 2));
        assert_eq!(view.free_slots(ElementRef::Truss(0), 0, &[1, 2]), vec![0, 1, 3, 4, 5]);
        assert_eq!(view.free_slots(ElementRef::Truss(0), 0, &[0, 1, 2]), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(view.free_slots(ElementRef::Truss(0), 1, &[]), (6..12).collect::<Vec<_>>());
        assert_eq!(view.free_slots(ElementRef::Tower(0), 1, &[]), vec![4, 5, 6, 7]);
        assert_eq!(view.free_slots(ElementRef::Tower(0), 9, &[]), Vec::<usize>::new());
        assert_eq!(view.free_slots(ElementRef::Truss(9), 0, &[]), Vec::<usize>::new());
        assert_eq!(view.element_free(ElementRef::Truss(0), 0), (5, 6));
        assert_eq!(view.element_faces(ElementRef::Truss(0)).len(), 4);
        assert_eq!(view.element_faces(ElementRef::Tower(0)), &["Top", "Under"]);
        assert_eq!(view.mount_label(0).as_deref(), Some("truss 1"));
        assert_eq!(view.mount_label(1), None);
    }

    #[test]
    fn mount_instances_hangs_in_a_centred_block_and_saves() {
        let patch = par_patch(3);
        let mut view = temp_stage(&patch, "mount");
        view.trusses.push(Truss::straight());
        let (n, left) = view.mount_instances(&patch, &[1, 2], ElementRef::Truss(0), 0, None);
        assert_eq!((n, left.len()), (2, 0));
        assert_eq!(view.instances[1].truss_mount, Some((0, 2)));
        assert_eq!(view.instances[2].truss_mount, Some((0, 3)));
        assert_eq!(view.instances[1].t.pos, view.trusses[0].slot_pos(2));
        assert!((view.instances[1].t.pitch_deg - 90.0).abs() < 0.5);
        assert_eq!(view.undo_stack.len(), 1);
        assert!(view.layout_path.exists(), "the layout was saved");
        let _ = std::fs::remove_file(&view.layout_path);
    }

    #[test]
    fn mount_instances_spills_leftovers_beneath() {
        let patch = par_patch(3);
        let mut view = temp_stage(&patch, "spill");
        let mut truss = Truss::straight();
        truss.length = 1.0; // 2 slots per face
        let y = truss.pos.y;
        view.trusses.push(truss);
        let (n, left) = view.mount_instances(&patch, &[0, 1, 2], ElementRef::Truss(0), 0, None);
        assert_eq!((n, left.as_slice()), (2, &[2usize][..]));
        assert_eq!(view.instances[2].truss_mount, None);
        assert!((view.instances[2].t.pos.y - (y - BENEATH)).abs() < 1e-4);
        let _ = std::fs::remove_file(&view.layout_path);
    }

    #[test]
    fn mount_on_tower_uses_under_face_with_anchor() {
        let patch = par_patch(3);
        let mut view = temp_stage(&patch, "tower");
        view.towers.push(Tower::default());
        let (n, left) = view.mount_instances(&patch, &[0], ElementRef::Tower(0), 1, Some(5));
        assert_eq!((n, left.len()), (1, 0));
        assert_eq!(view.instances[0].mount, Some((0, 5)));
        assert!((view.instances[0].t.pitch_deg + 90.0).abs() < 0.5);
        let _ = std::fs::remove_file(&view.layout_path);
    }

    #[test]
    fn place_instances_clears_mounts_and_pushes_one_undo() {
        let patch = par_patch(3);
        let mut view = temp_stage(&patch, "place");
        view.trusses.push(Truss::straight());
        view.mount_instances(&patch, &[0], ElementRef::Truss(0), 0, None);
        let before = view.undo_stack.len();
        view.place_instances(&patch, &[0, 1, 2], &spec(PlacePattern::LineX));
        assert_eq!(view.undo_stack.len(), before + 1);
        assert!(view.instances[0].mount.is_none() && view.instances[0].truss_mount.is_none());
        assert!((view.instances[0].t.pos.x + 1.0).abs() < 1e-4);
        assert!(view.undo(&patch));
        assert_eq!(view.instances[0].truss_mount, Some((0, 2)));
        // Keep changes nothing and pushes no step.
        let steps = view.undo_stack.len();
        view.place_instances(&patch, &[0, 1, 2], &spec(PlacePattern::Keep));
        assert_eq!(view.undo_stack.len(), steps);
        let _ = std::fs::remove_file(&view.layout_path);
    }

    #[test]
    fn instances_for_keys_maps_layout_keys_to_first_instance() {
        let patch = par_patch(3);
        let view = temp_stage(&patch, "keys");
        let keys: Vec<String> = vec!["Par 3@9".into(), "Par 1@1".into(), "Nope@77".into()];
        assert_eq!(view.instances_for_keys(&patch, &keys), vec![2, 0]);
    }

    #[test]
    fn add_truss_fitted_sets_length_and_steps_each_run_back() {
        let patch = par_patch(1);
        let mut view = temp_stage(&patch, "fitted");
        let ti = view.add_truss_fitted(&patch, TrussKind::Straight, Some(4.0));
        assert_eq!(ti, 0);
        assert_eq!(view.trusses[0].length, 4.0);
        assert_eq!(view.trusses[0].slot_count(), 8);
        assert_eq!(view.sel_truss, Some(0));
        assert_eq!(view.undo_stack.len(), 1);
        let ti = view.add_truss_fitted(&patch, TrussKind::Radius, None);
        assert_eq!(ti, 1);
        assert_eq!(view.trusses[1].kind, TrussKind::Radius);
        assert!(
            (view.trusses[1].pos.z - view.trusses[0].pos.z - TRUSS_STEP).abs() < 1e-4,
            "the second run steps clear of the first"
        );
        let ti = view.add_tower_fitted(&patch);
        assert_eq!((ti, view.sel_tower, view.sel_truss), (0, Some(0), None));
        let _ = std::fs::remove_file(&view.layout_path);
    }

    /// The five free-standing shapes, rendered on the real stage widget and
    /// written to `target/place_<pattern>.png` so the arrangement can be
    /// looked at, plus one batch hung on a truss.
    #[test]
    fn placement_patterns_render_headless() {
        use crate::stage::headless::{render_frames, save};
        let patch = par_patch(8);
        let mut settings = Settings::default();
        let gobos = crate::gobo::Catalogue::default();
        let buf = [170u8; crate::net::DMX_SLOTS];
        let size = [900, 600];
        let insts: Vec<usize> = (0..8).collect();
        for pattern in [
            PlacePattern::LineX,
            PlacePattern::Grid,
            PlacePattern::Arc,
            PlacePattern::Circle,
            PlacePattern::Floor,
            PlacePattern::Truss,
        ] {
            let mut view = temp_stage(&patch, "render");
            let mut s = spec(pattern);
            s.spacing = 1.2;
            s.radius = 3.5;
            if pattern == PlacePattern::Truss {
                let ti = view.add_truss_fitted(&patch, TrussKind::Straight, Some(4.0));
                view.mount_instances(&patch, &insts, ElementRef::Truss(ti), 1, None);
                // A new run is picked as it is made; nothing should be lit
                // up for the picture, so the truss itself can be seen.
                view.sel_truss = None;
                view.selection.clear();
                assert!(
                    view.instances.iter().all(|i| i.truss_mount.is_some()),
                    "every light should have found a slot on a 4 m run"
                );
            } else {
                view.place_instances(&patch, &insts, &s);
            }
            let Some(pixels) = render_frames(3, size, |ctx, _| {
                eframe::egui::CentralPanel::default().show(ctx, |ui| {
                    view.ui(ui, &patch, &buf, &gobos, 560.0, &mut settings, None, None, None);
                });
            }) else {
                eprintln!("no GPU adapter — skipping");
                return;
            };
            save(&pixels, size, &format!("place_{}", pattern.label().replace(' ', "_")));
            let lit = pixels
                .chunks(4)
                .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120)
                .count();
            assert!(lit > 1000, "{} came out black", pattern.label());
            let _ = std::fs::remove_file(&view.layout_path);
        }
    }
}
