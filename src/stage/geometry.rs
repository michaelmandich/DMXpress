//! Snap-to-tower logic, selection centroid, and the transform-gizmo geometry
//! (handle sizing, ring basis, and screen-space picking).

use std::collections::{HashMap, HashSet};

use eframe::egui::{Pos2, Rect, Vec2};

use super::gizmo::{GizmoPart, StageHandle};
use super::layout::{Tower, TOWER_SLOTS};
use super::math::{seg_dist, v3, V3};
use super::settings::Settings;
use super::view::StageView;
use crate::showbuddy::Patch;

impl StageView {
    /// While dragging, work out which free tower slot each selected light is
    /// hovering over — by *screen* proximity, so you can simply drag a light's
    /// icon over a bar's slot ring (even though it's up in the air and the
    /// drag itself only moves along the floor). Each light claims a distinct
    /// slot. Returns instance index → (tower, slot).
    pub(crate) fn compute_snap(&self, rect: Rect) -> HashMap<usize, (usize, usize)> {
        const SNAP_PX: f32 = 34.0;
        // Slots already held by lights that aren't part of this drag.
        let mut taken: HashSet<(usize, usize)> = self
            .instances
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.selection.contains(i))
            .filter_map(|(_, inst)| inst.mount)
            .collect();
        let mut sel: Vec<usize> = self.selection.iter().copied().collect();
        sel.sort_unstable();
        let mut out = HashMap::new();
        for i in sel {
            let Some((lp, _)) = self
                .instances
                .get(i)
                .and_then(|inst| self.cam.project(rect, inst.t.pos))
            else {
                continue;
            };
            let mut best: Option<(usize, usize, f32)> = None;
            for (ti, tw) in self.towers.iter().enumerate() {
                for slot in 0..TOWER_SLOTS {
                    if taken.contains(&(ti, slot)) {
                        continue;
                    }
                    if let Some((sp, _)) = self.cam.project(rect, tw.slot_pos(slot)) {
                        let d = sp.distance(lp);
                        if d < SNAP_PX && best.map_or(true, |(_, _, bd)| d < bd) {
                            best = Some((ti, slot, d));
                        }
                    }
                }
            }
            if let Some((ti, slot, _)) = best {
                taken.insert((ti, slot));
                out.insert(i, (ti, slot));
            }
        }
        out
    }

    /// Commit the current snap preview: mount the hovering lights onto their
    /// slots (top face points up, bottom face points down) and clear the
    /// preview.
    pub(crate) fn commit_snap(&mut self, patch: &Patch) {
        let preview = std::mem::take(&mut self.snap_preview);
        for (i, (ti, slot)) in preview {
            let Some(tw) = self.towers.get(ti) else { continue };
            let (sp, yaw) = (tw.slot_pos(slot), tw.yaw_deg);
            if let Some(inst) = self.instances.get_mut(i) {
                inst.mount = Some((ti, slot));
                inst.t.pos = sp;
                // Top-face slots point straight up, bottom-face point down.
                // Spin (roll_deg) is preserved so a base-mounted head keeps
                // its orientation; rotate par-type lights from the inspector.
                inst.t.pitch_deg = if Tower::slot_points_up(slot) { 90.0 } else { -90.0 };
                inst.t.yaw_deg = yaw;
            }
        }
        self.save(patch);
    }

    /// Rotate the fixtures mounted on tower `ti` by `dyaw` degrees so their
    /// orientation stays locked to the tower as it spins. Top-face slots point
    /// up and bottom-face slots point down, so their base-spin frames are
    /// mirrored — the sign of the applied roll flips accordingly.
    pub(crate) fn spin_tower_mounts(&mut self, ti: usize, dyaw: f32) {
        let yaw = match self.towers.get(ti) {
            Some(tw) => tw.yaw_deg,
            None => return,
        };
        for inst in &mut self.instances {
            if let Some((t, slot)) = inst.mount {
                if t == ti {
                    let sign = if Tower::slot_points_up(slot) { 1.0 } else { -1.0 };
                    inst.t.roll_deg = (inst.t.roll_deg + dyaw * sign).rem_euclid(360.0);
                    inst.t.yaw_deg = yaw;
                }
            }
        }
    }

    /// Centre of the current light selection (the transform-gizmo origin).
    pub(crate) fn selection_centroid(&self) -> Option<V3> {
        let mut c = V3::default();
        let mut n = 0.0f32;
        for &i in &self.selection {
            if let Some(inst) = self.instances.get(i) {
                c = c + inst.t.pos;
                n += 1.0;
            }
        }
        (n > 0.0).then(|| c * (1.0 / n))
    }

    /// World length of a gizmo arm so it stays a roughly constant size on
    /// screen regardless of zoom.
    pub(crate) fn gizmo_arm(&self, rect: Rect) -> f32 {
        self.cam.world_per_pixel(rect) * 78.0
    }

    /// Two unit vectors spanning the plane perpendicular to `axis`.
    pub(crate) fn ring_basis(axis: V3) -> (V3, V3) {
        let a = axis.norm();
        let helper = if a.y.abs() > 0.9 {
            v3(1.0, 0.0, 0.0)
        } else {
            v3(0.0, 1.0, 0.0)
        };
        let u = helper.cross(a).norm();
        let v = a.cross(u).norm();
        (u, v)
    }

    /// Which gizmo handle (if any) the pointer is over, in screen space.
    pub(crate) fn gizmo_pick(&self, rect: Rect, origin: V3, ptr: Pos2) -> Option<GizmoPart> {
        let arm = self.gizmo_arm(rect);
        self.cam.project(rect, origin)?;
        let mut best: Option<(GizmoPart, f32)> = None;
        let mut consider = |part: GizmoPart, d: f32| {
            if d < 8.0 && best.map_or(true, |(_, bd)| d < bd) {
                best = Some((part, d));
            }
        };
        // Translation arrows — only the outer part of each shaft is grabbable
        // so the centre stays free for the rotation rings.
        for part in [GizmoPart::TransX, GizmoPart::TransY, GizmoPart::TransZ] {
            let ax = part.axis();
            let start = self.cam.project(rect, origin + ax * (arm * 0.35));
            let tip = self.cam.project(rect, origin + ax * arm);
            if let (Some((s, _)), Some((t, _))) = (start, tip) {
                consider(part, seg_dist(s, t, ptr));
            }
        }
        // Rotation rings.
        for part in [GizmoPart::RotX, GizmoPart::RotY, GizmoPart::RotZ] {
            let (u, v) = Self::ring_basis(part.axis());
            let mut prev: Option<Pos2> = None;
            for k in 0..=48 {
                let a = k as f32 / 48.0 * std::f32::consts::TAU;
                let p = origin + (u * a.cos() + v * a.sin()) * arm;
                match self.cam.project(rect, p) {
                    Some((sp, _)) => {
                        if let Some(pp) = prev {
                            consider(part, seg_dist(pp, sp, ptr));
                        }
                        prev = Some(sp);
                    }
                    None => prev = None,
                }
            }
        }
        best.map(|(p, _)| p)
    }

    /// Is the pointer over the stage box's top surface? (Used to select the
    /// stage so its resize arrows appear.)
    pub(crate) fn stage_box_hit(&self, rect: Rect, set: &Settings, ptr: Pos2) -> bool {
        let (hw, h, hd) = (set.stage_half_w, set.stage_h, set.stage_half_d);
        let corners = [
            v3(-hw, h, -hd),
            v3(hw, h, -hd),
            v3(hw, h, hd),
            v3(-hw, h, hd),
        ];
        let pts: Option<Vec<Pos2>> = corners
            .iter()
            .map(|p| self.cam.project(rect, *p).map(|(q, _)| q))
            .collect();
        let Some(pts) = pts else { return false };
        // Point-in-convex-quad: all cross products share a sign.
        let mut pos = false;
        let mut neg = false;
        for k in 0..4 {
            let a = pts[k];
            let b = pts[(k + 1) % 4];
            let cross = (b.x - a.x) * (ptr.y - a.y) - (b.y - a.y) * (ptr.x - a.x);
            if cross > 0.0 {
                pos = true;
            } else if cross < 0.0 {
                neg = true;
            }
        }
        !(pos && neg)
    }

    /// World length of a stage resize arrow (constant on-screen size).
    pub(crate) fn stage_arrow_arm(&self, rect: Rect) -> f32 {
        self.cam.world_per_pixel(rect) * 46.0
    }

    /// Which stage resize arrow (if any) the pointer is over.
    pub(crate) fn stage_handle_pick(
        &self,
        rect: Rect,
        set: &Settings,
        ptr: Pos2,
    ) -> Option<StageHandle> {
        let arm = self.stage_arrow_arm(rect);
        let mut best: Option<(StageHandle, f32)> = None;
        for h in StageHandle::ALL {
            let a = h.anchor(set);
            let tip = a + h.axis() * arm;
            if let (Some((s, _)), Some((t, _))) =
                (self.cam.project(rect, a), self.cam.project(rect, tip))
            {
                let d = seg_dist(s, t, ptr);
                if d < 9.0 && best.map_or(true, |(_, bd)| d < bd) {
                    best = Some((h, d));
                }
            }
        }
        best.map(|(h, _)| h)
    }

    /// Convert a pointer drag delta into world units along a stage arrow's
    /// outward axis (projected onto the on-screen arrow so it tracks 1:1).
    pub(crate) fn stage_handle_world_delta(
        &self,
        rect: Rect,
        set: &Settings,
        h: StageHandle,
        d: Vec2,
    ) -> f32 {
        let arm = self.stage_arrow_arm(rect);
        let a = h.anchor(set);
        let o_s = self.cam.project(rect, a).map(|x| x.0);
        let tip_s = self.cam.project(rect, a + h.axis() * arm).map(|x| x.0);
        if let (Some(o), Some(t)) = (o_s, tip_s) {
            let sa = t - o;
            let len = sa.length().max(1.0);
            let dir = sa / len;
            (d.x * dir.x + d.y * dir.y) * arm / len
        } else {
            0.0
        }
    }
}
