//! Light placement transforms, scene instances, and floor-stand towers.

use eframe::egui::Color32;
use serde::{Deserialize, Serialize};

use super::math::{angles_from_dir, dir_from_angles, v3, V3};
use super::render::{add_box, Mesh};
use super::settings::Settings;
use crate::showbuddy::Fixture;

/// Editable placement of one light in the 3D scene.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LightTransform {
    pub pos: V3,
    /// Mounting orientation in degrees. Pan/tilt offsets are applied on top.
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    /// Spin of the fixture around its own forward (mounting) axis. For a
    /// base-mounted light (moving head) this is the only manual rotation —
    /// it spins the base, reorienting the pan/tilt sweep, with no gimbal
    /// tumbling. Pars ignore it (radially symmetric).
    #[serde(default)]
    pub roll_deg: f32,
    /// Per-fixture size multiplier on top of the global light scale.
    #[serde(default = "one")]
    pub scale: f32,
}

pub(crate) fn one() -> f32 {
    1.0
}

pub(crate) fn layout_key(f: &Fixture) -> String {
    format!("{}@{}", f.display, f.from)
}

pub(crate) fn default_transform(f: &Fixture, set: &Settings) -> LightTransform {
    LightTransform {
        // Spread initial placement using the ShowBuddy 2D layout.
        pos: v3((f.x - 0.5) * 12.0, set.default_height, (f.y - 0.5) * 10.0),
        yaw_deg: set.default_yaw,
        pitch_deg: set.default_pitch,
        roll_deg: 0.0,
        scale: 1.0,
    }
}

/// One visual light in the scene. Several instances may point at the same
/// patch fixture (physical duplicates wired to the same DMX addresses).
#[derive(Debug, Clone)]
pub(crate) struct Instance {
    /// Index into `patch.fixtures`.
    pub fixture: usize,
    pub t: LightTransform,
    /// Visualizer-only opacity for housing, lens, beam and surface pool.
    pub opacity: f32,
    /// Snapped onto a tower slot: (tower index, slot 0..8; 0..4 top, 4..8 bottom).
    pub mount: Option<(usize, usize)>,
    /// Snapped onto a truss slot: (truss index, slot) — see
    /// [`Truss::face_count`] and [`Truss::slot_pos_yaw`] for how the slot
    /// index maps to a mounting face (top/bottom, plus left/right on a
    /// straight run).
    pub truss_mount: Option<(usize, usize)>,
}

pub(crate) const TOWER_SLOTS: usize = 8;
/// Vertical offset of a mounted light from the crossbar centre.
const TOWER_SLOT_OFFSET: f32 = 0.2;

/// Floor stand: pole topped with one crossbar. Lights clip on top (pointing
/// up) or underneath (pointing down) — 4 slots per face, 8 total.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Tower {
    /// Base center on the floor (y = 0).
    pub pos: V3,
    pub yaw_deg: f32,
    pub height: f32,
    pub width: f32,
    /// Operator label shown in the Build outliner; empty = "Tower N" / "F34 truss N" / "Radius truss N".
    #[serde(default)]
    pub name: String,
    /// Composite this part belongs to; `None` = standalone.
    #[serde(default)]
    pub group: Option<ElementGroup>,
    /// Not drawn, not pickable, not a snap target; hung lights stay hung.
    #[serde(default)]
    pub hidden: bool,
}

impl Default for Tower {
    fn default() -> Self {
        Self {
            pos: v3(0.0, 0.0, 4.0),
            yaw_deg: 0.0,
            height: 3.2,
            width: 2.4,
            name: String::new(),
            group: None,
            hidden: false,
        }
    }
}

impl Tower {
    /// Horizontal crossbar direction.
    pub fn bar_dir(&self) -> V3 {
        dir_from_angles(self.yaw_deg + 90.0, 0.0)
    }

    /// Slots 0..4 sit on top of the bar (point up); 4..8 hang underneath
    /// (point down).
    pub fn slot_points_up(slot: usize) -> bool {
        slot < 4
    }

    /// World position a light snapped into `slot` mounts at.
    pub fn slot_pos(&self, slot: usize) -> V3 {
        let k = (slot % 4) as f32;
        let off = if Self::slot_points_up(slot) {
            TOWER_SLOT_OFFSET
        } else {
            -TOWER_SLOT_OFFSET
        };
        self.pos
            + self.bar_dir() * ((k - 1.5) * self.width / 3.0)
            + v3(0.0, self.height + off, 0.0)
    }

    pub fn mesh(&self, selected: bool) -> Mesh {
        let mut m = Mesh::default();
        let col = if selected {
            Color32::from_gray(120)
        } else {
            Color32::from_gray(70)
        };
        let bar = self.bar_dir();
        let fwd = dir_from_angles(self.yaw_deg, 0.0);
        let up = v3(0.0, 1.0, 0.0);
        // Pole.
        add_box(
            &mut m,
            self.pos + up * (self.height * 0.5),
            bar * 0.045,
            up * (self.height * 0.5),
            fwd * 0.045,
            col,
            None,
        );
        // Single crossbar at the top.
        add_box(
            &mut m,
            self.pos + up * self.height,
            bar * (self.width * 0.5),
            up * 0.04,
            fwd * 0.04,
            col,
            None,
        );
        // Crossed feet.
        add_box(&mut m, self.pos + up * 0.03, bar * 0.5, up * 0.03, fwd * 0.06, col, None);
        add_box(&mut m, self.pos + up * 0.03, bar * 0.06, up * 0.03, fwd * 0.5, col, None);
        m
    }
}

/// Spacing between mount slots along a truss run.
const TRUSS_SLOT_SPACING: f32 = 0.5;
/// Vertical offset of a mounted light from the truss centreline.
const TRUSS_SLOT_OFFSET: f32 = 0.18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TrussKind {
    /// F34-style straight box truss run.
    Straight,
    /// Curved truss sweeping an arc of a given radius.
    Radius,
}

/// One tower or truss by index — the element reference every tab, the
/// builder, the placer and the stage share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ElementRef {
    Tower(usize),
    Truss(usize),
}

/// What a composite build made; the outliner folds a group's parts under
/// one header carrying this kind's icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum CompositeKind {
    Goalpost,
    Box,
    Ring,
    TowerPair,
    Arch,
}

/// Tag on every part of a composite. Groups have no list of their own:
/// they exist wherever a tower or truss carries the same id, so undo,
/// files and setups carry them for free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct ElementGroup {
    pub id: u32,
    pub kind: CompositeKind,
}

/// A straight or curved truss run. Lights clip onto slots spaced every
/// [`TRUSS_SLOT_SPACING`] along the run, on any of [`Truss::face_count`]
/// faces around the cross-section. The whole run can also tilt in 3D via
/// `pitch_deg`/`roll_deg` (see [`Truss::frame`]) — e.g. a curved run stood
/// up on edge against a wall.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Truss {
    pub kind: TrussKind,
    /// Straight: centre of the run, at bar height (y). Radius: centre of the
    /// circle the arc sweeps, at bar height (y).
    pub pos: V3,
    /// Straight: heading along the run. Radius: angle of the arc's start
    /// point (degrees, measured the same way as [`dir_from_angles`]).
    pub yaw_deg: f32,
    /// Tilt of the whole truss body out of the horizontal plane (degrees) —
    /// 0 = flat/hanging overhead, 90 = standing on edge (e.g. a ring stood
    /// up against a wall). See [`Self::frame`].
    #[serde(default)]
    pub pitch_deg: f32,
    /// Spin (degrees) of the tilt axis itself — which way a pitched truss
    /// leans/faces. No effect while `pitch_deg` is 0. See [`Self::frame`].
    #[serde(default)]
    pub roll_deg: f32,
    /// Straight only: F34 run length in metres (standard: 0.5 / 1 / 1.5 / 2 / 3 m).
    pub length: f32,
    /// Radius only: circle radius in metres.
    pub radius: f32,
    /// Radius only: sweep angle in degrees (e.g. 90 / 180 / 360).
    pub arc_deg: f32,
    /// Draw support legs down to the floor at both ends of the run.
    pub grounded: bool,
    /// Operator label shown in the Build outliner; empty = "Tower N" / "F34 truss N" / "Radius truss N".
    #[serde(default)]
    pub name: String,
    /// Composite this part belongs to; `None` = standalone.
    #[serde(default)]
    pub group: Option<ElementGroup>,
    /// Not drawn, not pickable, not a snap target; hung lights stay hung.
    #[serde(default)]
    pub hidden: bool,
}

impl Truss {
    pub fn straight() -> Self {
        Self {
            kind: TrussKind::Straight,
            pos: v3(0.0, 3.2, -4.0),
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            roll_deg: 0.0,
            length: 3.0,
            radius: 2.0,
            arc_deg: 90.0,
            grounded: true,
            name: String::new(),
            group: None,
            hidden: false,
        }
    }

    pub fn radius() -> Self {
        Self {
            kind: TrussKind::Radius,
            pos: v3(0.0, 3.2, -4.0),
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            roll_deg: 0.0,
            length: 2.0,
            radius: 2.0,
            arc_deg: 90.0,
            grounded: false,
            name: String::new(),
            group: None,
            hidden: false,
        }
    }

    /// Orthonormal orientation frame: `normal` is the axis mount slots and
    /// the cross-section offset along — world up when `pitch_deg =
    /// roll_deg = 0`, but any 3D direction once tilted; `a`/`b` span the
    /// plane perpendicular to `normal` (the run/arc plane), used by
    /// [`Self::plane_dir`]. At `pitch_deg = roll_deg = 0` this reduces
    /// exactly to the old flat behaviour (`normal` = world up, `a` = world
    /// +Z, `b` = world +X), so untouched trusses look and behave
    /// identically to before this field existed.
    fn frame(&self) -> (V3, V3, V3) {
        let (ps, pc) = self.pitch_deg.to_radians().sin_cos();
        let (rs, rc) = self.roll_deg.to_radians().sin_cos();
        let normal = v3(ps * rs, pc, ps * rc);
        let a = v3(pc * rs, -ps, pc * rc);
        let b = v3(rc, 0.0, -rs);
        (normal, a, b)
    }

    /// In-plane direction at angle `deg`, measured the same way as the old
    /// flat-truss [`dir_from_angles`] but tilted by this truss's
    /// [`Self::frame`].
    fn plane_dir(&self, deg: f32) -> V3 {
        let (_, a, b) = self.frame();
        let (s, c) = deg.to_radians().sin_cos();
        a * c + b * s
    }

    /// Straight run length, or curved arc length, in metres.
    fn run_length(&self) -> f32 {
        match self.kind {
            TrussKind::Straight => self.length.max(0.1),
            TrussKind::Radius => self.radius.max(0.1) * self.arc_deg.to_radians().abs(),
        }
    }

    /// Mount slots per face; total slots = [`Self::face_count`] × this.
    pub fn slot_count(&self) -> usize {
        ((self.run_length() / TRUSS_SLOT_SPACING).round() as i32).clamp(1, 40) as usize
    }

    /// Mounting faces around the cross-section. A straight F34-style run is
    /// a square box truss, so lights can clip onto any of its 4 sides (top,
    /// bottom, left, right). A curved run only has top/bottom.
    pub fn face_count(&self) -> usize {
        match self.kind {
            TrussKind::Straight => 4,
            TrussKind::Radius => 2,
        }
    }

    /// Total mount slots across every face.
    pub fn total_slots(&self) -> usize {
        self.slot_count() * self.face_count()
    }

    /// Which face `slot` sits on: 0 = top (points up), 1 = bottom (points
    /// down), 2 = left, 3 = right (left/right only exist on a straight run
    /// — see [`Self::face_count`]).
    fn slot_face(&self, slot: usize) -> usize {
        slot / self.slot_count()
    }

    /// Roll-compensation sign for [`crate::stage::StageView::spin_truss_mounts`]:
    /// top/bottom faces are gimbal-locked (yaw has no visual effect while
    /// pointing straight up/down), so a truss-yaw edit needs a compensating
    /// roll spin; left/right faces point sideways and already track yaw
    /// directly, so no extra roll compensation is needed.
    pub fn slot_roll_sign(&self, slot: usize) -> f32 {
        match self.slot_face(slot) {
            0 => 1.0,
            1 => -1.0,
            _ => 0.0,
        }
    }

    /// Distance along the run of slot index `k` (within one face), centred
    /// on the run's midpoint.
    fn slot_offset(&self, k: usize) -> f32 {
        let n = self.slot_count();
        (k as f32 - (n as f32 - 1.0) / 2.0) * TRUSS_SLOT_SPACING
    }

    /// World position, mounting yaw and pitch a light snapped into `slot`
    /// gets. Each face's light points straight outward along that face's
    /// normal (top/bottom faces point straight up/down when flat; a
    /// straight run's left/right faces point out sideways) — tilting the
    /// whole truss via `pitch_deg`/`roll_deg` carries every slot's facing
    /// with it.
    pub fn slot_pos_yaw(&self, slot: usize) -> (V3, f32, f32) {
        let n = self.slot_count();
        let k = slot % n;
        let face = self.slot_face(slot);
        let along = self.slot_offset(k);
        let (normal, _, _) = self.frame();
        let (base, n_dir) = match self.kind {
            TrussKind::Straight => {
                let fwd = self.plane_dir(self.yaw_deg);
                let side = fwd.cross(normal).norm();
                let base = self.pos + fwd * along;
                let n_dir = match face {
                    0 => normal,
                    1 => normal * -1.0,
                    2 => side * -1.0,
                    _ => side,
                };
                (base, n_dir)
            }
            TrussKind::Radius => {
                let r = self.radius.max(0.1);
                let center_deg = self.yaw_deg + self.arc_deg * 0.5;
                let ang = center_deg + (along / r).to_degrees();
                let dir = self.plane_dir(ang);
                let base = self.pos + dir * r;
                let n_dir = if face == 0 { normal } else { normal * -1.0 };
                (base, n_dir)
            }
        };
        let (yaw, pitch) = angles_from_dir(n_dir);
        (base + n_dir * TRUSS_SLOT_OFFSET, yaw, pitch)
    }

    pub fn slot_pos(&self, slot: usize) -> V3 {
        self.slot_pos_yaw(slot).0
    }

    pub fn mesh(&self, selected: bool) -> Mesh {
        let mut m = Mesh::default();
        let col = if selected {
            Color32::from_gray(120)
        } else {
            Color32::from_gray(70)
        };
        let up = v3(0.0, 1.0, 0.0);
        let leg = |m: &mut Mesh, top: V3, side: V3, fwd: V3, col: Color32| {
            let mid = top - up * (top.y * 0.5);
            add_box(m, mid, side * 0.045, up * (top.y * 0.5), fwd * 0.045, col, None);
        };
        // Thin rod between two arbitrary points — used for the diagonal
        // cross-braces of a box-truss lattice.
        let strut = |m: &mut Mesh, p0: V3, p1: V3, r: f32, col: Color32| {
            let d = p1 - p0;
            let len = d.len();
            if len < 1e-4 {
                return;
            }
            let dir = d * (1.0 / len);
            let helper = if dir.y.abs() > 0.9 { v3(1.0, 0.0, 0.0) } else { v3(0.0, 1.0, 0.0) };
            let perp1 = helper.cross(dir).norm();
            let perp2 = dir.cross(perp1).norm();
            add_box(m, (p0 + p1) * 0.5, dir * (len * 0.5), perp1 * r, perp2 * r, col, None);
        };
        let (nrm, _, _) = self.frame();
        match self.kind {
            TrussKind::Straight => {
                // F34-style box truss: 4 thin parallel chords at the
                // corners of a square cross-section, laced together with
                // zigzagging diagonal braces on all 4 faces.
                let dir = self.plane_dir(self.yaw_deg);
                let side = dir.cross(nrm).norm();
                const HALF_W: f32 = 0.15;
                const CHORD_R: f32 = 0.022;
                const BRACE_R: f32 = 0.013;
                let corner = |sx: f32, sy: f32| side * (sx * HALF_W) + nrm * (sy * HALF_W);
                let corners = [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)];
                for (sx, sy) in corners {
                    add_box(
                        &mut m,
                        self.pos + corner(sx, sy),
                        dir * (self.length * 0.5),
                        side * CHORD_R,
                        nrm * CHORD_R,
                        col,
                        None,
                    );
                }
                let half_len = self.length * 0.5;
                let bays = ((self.length / TRUSS_SLOT_SPACING).round() as i32).max(1) as usize;
                let bay_len = self.length / bays as f32;
                let faces = [
                    (corner(-1.0, 1.0), corner(1.0, 1.0)),   // top
                    (corner(-1.0, -1.0), corner(1.0, -1.0)), // bottom
                    (corner(-1.0, -1.0), corner(-1.0, 1.0)), // left
                    (corner(1.0, -1.0), corner(1.0, 1.0)),   // right
                ];
                for (fa, fb) in faces {
                    for i in 0..bays {
                        let t0 = -half_len + i as f32 * bay_len;
                        let t1 = t0 + bay_len;
                        let (start_off, end_off) = if i % 2 == 0 { (fa, fb) } else { (fb, fa) };
                        let p0 = self.pos + start_off + dir * t0;
                        let p1 = self.pos + end_off + dir * t1;
                        strut(&mut m, p0, p1, BRACE_R, col);
                    }
                }
                if self.grounded {
                    for end in [-1.0f32, 1.0] {
                        leg(&mut m, self.pos + dir * (self.length * 0.5 * end), side, dir, col);
                    }
                }
            }
            TrussKind::Radius => {
                const SEGMENTS: usize = 16;
                let r = self.radius.max(0.1);
                for i in 0..SEGMENTS {
                    let a0 = self.yaw_deg + self.arc_deg * (i as f32 / SEGMENTS as f32);
                    let a1 = self.yaw_deg + self.arc_deg * ((i + 1) as f32 / SEGMENTS as f32);
                    let p0 = self.pos + self.plane_dir(a0) * r;
                    let p1 = self.pos + self.plane_dir(a1) * r;
                    let seg = p1 - p0;
                    if seg.len() < 1e-4 {
                        continue;
                    }
                    let dir = seg.norm();
                    let perp = dir.cross(nrm).norm();
                    add_box(&mut m, (p0 + p1) * 0.5, seg * 0.5, nrm * 0.09, perp * 0.09, col, None);
                }
                if self.grounded {
                    for a in [self.yaw_deg, self.yaw_deg + self.arc_deg] {
                        let dir = self.plane_dir(a);
                        let side = dir.cross(nrm).norm();
                        leg(&mut m, self.pos + dir * r, side, dir, col);
                    }
                }
            }
        }
        m
    }
}

/// On-disk layout: light instances, towers and trusses. Older builds stored
/// a plain `{ key -> transform }` map, which is still read as a fallback.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct LayoutFile {
    #[serde(default)]
    pub instances: Vec<SavedInstance>,
    #[serde(default)]
    pub towers: Vec<Tower>,
    #[serde(default)]
    pub trusses: Vec<Truss>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SavedInstance {
    pub key: String,
    pub t: LightTransform,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub mount: Option<(usize, usize)>,
    #[serde(default)]
    pub truss_mount: Option<(usize, usize)>,
}

