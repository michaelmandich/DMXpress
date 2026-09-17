//! The stage side of the light / tower / truss editor grids the Inspector's
//! Selection tab embeds: what a grid reads out of the selection (values plus
//! the "mixed" flags a multi-selection needs), what it writes back, and the
//! fix-ups an element edit owes its hung lights.
//!
//! The grids themselves are drawn in `ui::inspector::selection` — `theme`
//! and `icons` are private to `crate::ui`, so the kit cannot be reached from
//! here. Everything in this file is therefore pure of egui and unit-testable.

use super::arrange::OrderBy;
use super::fixture::classify;
use super::layout::{ElementRef, TrussKind};
use super::math::{v3, V3};
use super::settings::Settings;
use super::view::StageView;
use crate::showbuddy::Patch;

/// What the element editor's footer asked for, applied by the tab.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum ElementAction {
    #[default]
    None,
    Delete,
    SelectMounted,
    HangLast,
    MirrorX,
    MirrorZ,
}

/// The placement quick-chips on an element editor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ElementSpot {
    Centre,
    Back,
    Front,
    Left,
    Right,
}

impl ElementSpot {
    pub const ALL: [ElementSpot; 5] = [
        ElementSpot::Centre,
        ElementSpot::Back,
        ElementSpot::Front,
        ElementSpot::Left,
        ElementSpot::Right,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ElementSpot::Centre => "Centre",
            ElementSpot::Back => "Back",
            ElementSpot::Front => "Front",
            ElementSpot::Left => "Left",
            ElementSpot::Right => "Right",
        }
    }
}

/// Floor X / Z for a quick placement. Upstage (back) is −Z, matching the
/// stage box and the nudge pad's chevrons.
pub(crate) fn element_spot(spot: ElementSpot, set: &Settings) -> (f32, f32) {
    match spot {
        ElementSpot::Centre => (0.0, 0.0),
        ElementSpot::Back => (0.0, -set.stage_half_d),
        ElementSpot::Front => (0.0, set.stage_half_d),
        ElementSpot::Left => (-set.stage_half_w, 0.0),
        ElementSpot::Right => (set.stage_half_w, 0.0),
    }
}

/// The values one transform grid shows for the whole selection: the
/// centroid, and the anchor light's rotation, size and opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LightEdit {
    pub pos: V3,
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub roll_deg: f32,
    pub scale: f32,
    pub opacity: f32,
}

/// Which of those values differ across the selection, so the grid can say
/// so instead of pretending the first light speaks for all of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LightMixed {
    pub pos: [bool; 3],
    pub yaw: bool,
    pub pitch: bool,
    pub roll: bool,
    pub scale: bool,
    pub opacity: bool,
}

/// Everything the transform grid needs about the selection.
pub(crate) struct LightEditor {
    pub values: LightEdit,
    pub mixed: LightMixed,
    /// Selected moving heads / everything else: which rotation rows show.
    pub heads: usize,
    pub pars: usize,
    pub count: usize,
}

/// Values closer than this read as the same number in the grid.
const EPS: f32 = 1e-4;

impl StageView {
    /// Read the selection into one editable row set, or `None` when nothing
    /// is selected.
    pub(crate) fn light_editor(&self, patch: &Patch) -> Option<LightEditor> {
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        let first = *sel.first()?;
        let centre = self.selection_centroid()?;
        let t0 = &self.instances[first].t;
        let values = LightEdit {
            pos: centre,
            yaw_deg: t0.yaw_deg,
            pitch_deg: t0.pitch_deg,
            roll_deg: t0.roll_deg,
            scale: t0.scale,
            opacity: self.instances[first].opacity,
        };
        let mut mixed = LightMixed::default();
        let (mut heads, mut pars) = (0, 0);
        let p0 = t0.pos;
        for &i in &sel {
            let inst = &self.instances[i];
            let t = &inst.t;
            mixed.pos[0] |= (t.pos.x - p0.x).abs() > EPS;
            mixed.pos[1] |= (t.pos.y - p0.y).abs() > EPS;
            mixed.pos[2] |= (t.pos.z - p0.z).abs() > EPS;
            mixed.yaw |= (t.yaw_deg - values.yaw_deg).abs() > EPS;
            mixed.pitch |= (t.pitch_deg - values.pitch_deg).abs() > EPS;
            mixed.roll |= (t.roll_deg - values.roll_deg).abs() > EPS;
            mixed.scale |= (t.scale - values.scale).abs() > EPS;
            mixed.opacity |= (inst.opacity - values.opacity).abs() > EPS;
            match patch.fixtures.get(inst.fixture).map(classify) {
                Some(a) if super::arrange::is_head(a) => heads += 1,
                _ => pars += 1,
            }
        }
        Some(LightEditor { values, mixed, heads, pars, count: sel.len() })
    }

    /// Write back whatever the grid changed. Position moves the whole
    /// selection by the delta (and unhooks what it moves); rotation, size
    /// and opacity are absolute on every selected light and keep their
    /// mounts. No undo step of its own — the caller checkpoints the gesture.
    pub(crate) fn apply_light_edit(
        &mut self,
        patch: &Patch,
        before: LightEdit,
        now: LightEdit,
    ) -> bool {
        if before == now {
            return false;
        }
        let sel = self.sorted_selection(patch, OrderBy::Selection);
        if sel.is_empty() {
            return false;
        }
        let d = now.pos - before.pos;
        let moved = d.len() > EPS;
        for &i in &sel {
            if moved {
                let inst = &mut self.instances[i];
                inst.mount = None;
                inst.truss_mount = None;
                inst.t.pos = inst.t.pos + d;
            }
            let inst = &mut self.instances[i];
            if (now.yaw_deg - before.yaw_deg).abs() > EPS {
                inst.t.yaw_deg = now.yaw_deg;
            }
            if (now.pitch_deg - before.pitch_deg).abs() > EPS {
                inst.t.pitch_deg = now.pitch_deg;
            }
            if (now.roll_deg - before.roll_deg).abs() > EPS {
                inst.t.roll_deg = now.roll_deg.rem_euclid(360.0);
            }
            if (now.scale - before.scale).abs() > EPS {
                inst.t.scale = now.scale.clamp(0.05, 10.0);
            }
            if (now.opacity - before.opacity).abs() > EPS {
                inst.opacity = now.opacity.clamp(0.0, 1.0);
            }
        }
        self.save(patch);
        true
    }

    /// After a tower grid edit: re-glue its lights to the turned bar, save.
    pub(crate) fn apply_tower_edit(&mut self, patch: &Patch, ti: usize, dyaw: f32) {
        if dyaw != 0.0 {
            self.spin_tower_mounts(ti, dyaw);
        }
        self.save(patch);
    }

    /// After a truss grid edit: re-glue its lights, re-index them onto the
    /// new slot spacing and drop the ones whose slot fell off the end (the
    /// old editor let them silently jump to another face). Returns how many
    /// came off, so the tab can say so.
    pub(crate) fn apply_truss_edit(
        &mut self,
        patch: &Patch,
        ti: usize,
        dyaw: f32,
        prev_per_face: usize,
    ) -> usize {
        let fell = self.unmount_overflow(ti, prev_per_face);
        self.spin_truss_mounts(ti, dyaw);
        self.save(patch);
        fell
    }

    /// Slots per face on truss `ti` right now — the number
    /// [`Self::apply_truss_edit`] wants handed back after a shape change.
    pub(crate) fn truss_per_face(&self, ti: usize) -> usize {
        self.trusses.get(ti).map_or(1, |tr| tr.slot_count())
    }

    /// Move a whole element onto one of the stage's quick spots. Height is
    /// kept; the hung lights come along.
    pub(crate) fn place_element(
        &mut self,
        patch: &Patch,
        r: ElementRef,
        spot: ElementSpot,
        set: &Settings,
    ) -> bool {
        let (x, z) = element_spot(spot, set);
        match r {
            ElementRef::Tower(i) => {
                let Some(tw) = self.towers.get(i) else { return false };
                if (tw.pos.x - x).abs() < EPS && (tw.pos.z - z).abs() < EPS {
                    return false;
                }
                self.push_undo();
                let tw = &mut self.towers[i];
                tw.pos = v3(x, tw.pos.y, z);
            }
            ElementRef::Truss(i) => {
                let Some(tr) = self.trusses.get(i) else { return false };
                if (tr.pos.x - x).abs() < EPS && (tr.pos.z - z).abs() < EPS {
                    return false;
                }
                self.push_undo();
                let tr = &mut self.trusses[i];
                tr.pos = v3(x, tr.pos.y, z);
                self.resync_truss_mounts(i);
            }
        }
        self.save(patch);
        true
    }

    /// The editor card's title: "Tower" / "F34 truss" / "Radius truss".
    pub(crate) fn element_kind_label(&self, r: ElementRef) -> &'static str {
        match r {
            ElementRef::Tower(_) => "Tower",
            ElementRef::Truss(i) => match self.trusses.get(i).map(|tr| tr.kind) {
                Some(TrussKind::Radius) => "Radius truss",
                _ => "F34 truss",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::{Settings, Truss};

    fn rig(name: &str, n: usize) -> (StageView, Patch) {
        let profile = crate::profiles::find("Generic RGBW Par (4ch)").expect("the built-in par");
        let patch = Patch {
            fixtures: (0..n)
                .map(|i| profile.to_fixture(format!("T {i}"), 1 + (i as u16) * 4))
                .collect(),
            warnings: Vec::new(),
        };
        let mut stage = StageView::new();
        stage.layout_path = std::env::temp_dir().join(format!("dmxpress_editor_{name}.json"));
        let _ = std::fs::remove_file(&stage.layout_path);
        stage.sync(&patch, &Settings::default());
        for (k, inst) in stage.instances.iter_mut().enumerate() {
            inst.t.pos = v3(k as f32, 4.0, 0.0);
        }
        stage.select_all_fixtures();
        (stage, patch)
    }

    #[test]
    fn light_editor_reads_the_centroid_and_flags_mixed_values() {
        let (mut stage, patch) = rig("read", 3);
        stage.instances[1].t.yaw_deg = 45.0;
        let e = stage.light_editor(&patch).expect("a selection");
        assert_eq!(e.count, 3);
        assert!((e.values.pos.x - 1.0).abs() < 1e-4, "{:?}", e.values.pos);
        assert_eq!(e.mixed.pos, [true, false, false]);
        assert!(e.mixed.yaw && !e.mixed.pitch && !e.mixed.scale);
        assert_eq!((e.heads, e.pars), (0, 3));
        stage.clear_selection();
        assert!(stage.light_editor(&patch).is_none());
        let _ = std::fs::remove_file(&stage.layout_path);
    }

    #[test]
    fn apply_light_edit_moves_by_delta_and_sets_rotation_absolutely() {
        let (mut stage, patch) = rig("apply", 3);
        stage.trusses.push(Truss::straight());
        stage.instances[0].truss_mount = Some((0, 1));
        let before = stage.light_editor(&patch).unwrap().values;
        let mut now = before;
        now.pos.x += 2.0;
        now.yaw_deg = 90.0;
        assert!(stage.apply_light_edit(&patch, before, now));
        for (k, inst) in stage.instances.iter().enumerate() {
            assert!((inst.t.pos.x - (k as f32 + 2.0)).abs() < 1e-4);
            assert!((inst.t.yaw_deg - 90.0).abs() < 1e-4);
        }
        // A typed move unhooks BOTH kinds of mount (the old editor cleared
        // only `mount`, so truss-hung lights snapped straight back).
        assert_eq!(stage.instances[0].truss_mount, None);
        assert_eq!(stage.instances[0].mount, None);
        // Nothing changed: no write at all.
        let same = stage.light_editor(&patch).unwrap().values;
        assert!(!stage.apply_light_edit(&patch, same, same));
        // A rotation-only edit keeps the mounts.
        stage.instances[0].truss_mount = Some((0, 1));
        let mut now = same;
        now.pitch_deg = -45.0;
        assert!(stage.apply_light_edit(&patch, same, now));
        assert_eq!(stage.instances[0].truss_mount, Some((0, 1)));
        let _ = std::fs::remove_file(&stage.layout_path);
    }

    #[test]
    fn truss_edit_reindexes_and_drops_what_fell_off() {
        let (mut stage, patch) = rig("shrink", 2);
        let mut tr = Truss::straight();
        tr.length = 3.0;
        stage.trusses.push(tr);
        let prev = stage.truss_per_face(0);
        assert_eq!(prev, 6);
        stage.instances[0].truss_mount = Some((0, 5));
        stage.instances[1].truss_mount = Some((0, 6));
        stage.trusses[0].length = 1.0;
        assert_eq!(stage.apply_truss_edit(&patch, 0, 0.0, prev), 1);
        assert_eq!(stage.instances[0].truss_mount, None);
        assert_eq!(stage.instances[1].truss_mount, Some((0, 2)));
        // …and the survivor really is back on the bottom face.
        let want = stage.trusses[0].slot_pos(2);
        assert!((stage.instances[1].t.pos - want).len() < 1e-4);
        let _ = std::fs::remove_file(&stage.layout_path);
    }

    #[test]
    fn element_spots_use_the_stage_size() {
        let set = Settings::default();
        assert_eq!(element_spot(ElementSpot::Centre, &set), (0.0, 0.0));
        assert_eq!(element_spot(ElementSpot::Back, &set), (0.0, -set.stage_half_d));
        assert_eq!(element_spot(ElementSpot::Front, &set), (0.0, set.stage_half_d));
        assert_eq!(element_spot(ElementSpot::Left, &set), (-set.stage_half_w, 0.0));
        assert_eq!(element_spot(ElementSpot::Right, &set), (set.stage_half_w, 0.0));
        let (mut stage, patch) = rig("place", 1);
        stage.towers.push(crate::stage::Tower::default());
        assert!(stage.place_element(&patch, ElementRef::Tower(0), ElementSpot::Left, &set));
        assert!((stage.towers[0].pos.x + set.stage_half_w).abs() < 1e-4);
        assert_eq!(stage.undo_stack.len(), 1);
        // Already there: no second undo step.
        assert!(!stage.place_element(&patch, ElementRef::Tower(0), ElementSpot::Left, &set));
        assert_eq!(stage.undo_stack.len(), 1);
        assert_eq!(stage.element_kind_label(ElementRef::Tower(0)), "Tower");
        let _ = std::fs::remove_file(&stage.layout_path);
    }
}
