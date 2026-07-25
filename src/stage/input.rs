//! Pointer and keyboard handling for the stage widget: camera control,
//! selection, dragging (move / tower / pan / gizmo / marquee), snap preview,
//! and the edit shortcuts. Delegates all rendering to [`StageView::draw_scene`].

use eframe::egui::{self, Color32, Pos2, Rect, Sense};

use super::gizmo::{Drag, GizmoPart};
use super::math::{rotate_about, v3, V3};
use super::settings::Settings;
use super::view::StageView;
use crate::chase::ChaseConfig;
use crate::showbuddy::Patch;
use crate::transition::{TransitionConfig, TransitionMode};

impl StageView {
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        patch: &Patch,
        buf: &[u8; crate::net::DMX_SLOTS],
        height: f32,
        set: &mut Settings,
        mut transition: Option<&mut TransitionConfig>,
        mut chase: Option<&mut ChaseConfig>,
    ) {
        let covered = {
            let mut seen = vec![false; patch.fixtures.len()];
            let mut ok = true;
            for inst in &self.instances {
                match seen.get_mut(inst.fixture) {
                    Some(s) => *s = true,
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            ok && seen.iter().all(|&s| s)
        };
        if !covered {
            self.sync(patch, set);
        }

        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_gray(10));

        let mods = ui.input(|i| i.modifiers);

        // ---- camera ----
        if resp.hovered() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                if self.fly_mode {
                    self.fly_speed = (self.fly_speed * (1.0 + scroll * 0.0015)).clamp(0.5, 30.0);
                } else {
                    self.cam.dist = (self.cam.dist * (1.0 - scroll * 0.0015)).clamp(3.0, 80.0);
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Secondary)
            || (mods.alt && resp.dragged_by(egui::PointerButton::Primary))
        {
            let d = ui.input(|i| i.pointer.delta());
            if self.fly_mode {
                self.cam.free_look(-d.x * 0.008, d.y * 0.008);
            } else {
                self.cam.yaw -= d.x * 0.008;
                self.cam.pitch = (self.cam.pitch + d.y * 0.008).clamp(-0.15, 1.5);
            }
        }
        if resp.dragged_by(egui::PointerButton::Middle) {
            let d = ui.input(|i| i.pointer.delta());
            let wpp = self.cam.world_per_pixel(rect);
            let right = v3(self.cam.yaw.cos(), 0.0, -self.cam.yaw.sin());
            self.cam.target = self.cam.target - right * (d.x * wpp) + v3(0.0, d.y * wpp, 0.0);
        }
        if self.fly_mode && resp.hovered() && !ui.ctx().wants_keyboard_input() {
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            let (forward, back, left, right_key, up, down, turn_l, turn_r, turn_u, turn_d) =
                ui.input(|i| {
                    (
                        i.key_down(egui::Key::W),
                        i.key_down(egui::Key::S),
                        i.key_down(egui::Key::A),
                        i.key_down(egui::Key::D),
                        i.key_down(egui::Key::Space),
                        i.modifiers.shift,
                        i.key_down(egui::Key::ArrowLeft),
                        i.key_down(egui::Key::ArrowRight),
                        i.key_down(egui::Key::ArrowUp),
                        i.key_down(egui::Key::ArrowDown),
                    )
                });
            let (cam_right, _, cam_forward) = self.cam.basis();
            let mut movement = V3::default();
            if forward {
                movement = movement + cam_forward;
            }
            if back {
                movement = movement - cam_forward;
            }
            if left {
                movement = movement - cam_right;
            }
            if right_key {
                movement = movement + cam_right;
            }
            if up {
                movement = movement + v3(0.0, 1.0, 0.0);
            }
            if down {
                movement = movement - v3(0.0, 1.0, 0.0);
            }
            if movement.len() > 0.01 {
                self.cam.translate(movement.norm() * (self.fly_speed * dt));
                ui.ctx().request_repaint();
            }
            let yaw = (turn_l as i8 - turn_r as i8) as f32 * 1.5 * dt;
            let pitch = (turn_d as i8 - turn_u as i8) as f32 * 1.5 * dt;
            if yaw != 0.0 || pitch != 0.0 {
                self.cam.free_look(yaw, pitch);
                ui.ctx().request_repaint();
            }
        }

        // Keep tower-mounted lights glued to their slots.
        for inst in &mut self.instances {
            if let Some((ti, slot)) = inst.mount {
                match self.towers.get(ti) {
                    Some(tw) => inst.t.pos = tw.slot_pos(slot),
                    None => inst.mount = None,
                }
            }
        }

        // Projected body positions (for hit-testing and drawing).
        let proj: Vec<Option<(Pos2, f32)>> = self
            .instances
            .iter()
            .map(|inst| self.cam.project(rect, inst.t.pos))
            .collect();

        // ---- selection + move (primary button, not while orbiting) ----
        let hit_test = |ptr: Pos2| {
            proj.iter()
                .enumerate()
                .filter_map(|(i, p)| p.map(|(sp, z)| (i, sp.distance(ptr), z)))
                .filter(|(_, d, _)| *d < 14.0)
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .map(|(i, _, _)| i)
        };

        // Screen-space segments (pole + crossbars) for tower picking.
        let tower_segs: Vec<Vec<(Pos2, Pos2)>> = self
            .towers
            .iter()
            .map(|tw| {
                let bar = tw.bar_dir();
                let top = tw.pos + v3(0.0, tw.height, 0.0);
                [
                    (tw.pos, top),
                    (top - bar * (tw.width * 0.5), top + bar * (tw.width * 0.5)),
                ]
                .iter()
                .filter_map(|(a, b)| {
                    Some((self.cam.project(rect, *a)?.0, self.cam.project(rect, *b)?.0))
                })
                .collect()
            })
            .collect();
        let tower_hit = |ptr: Pos2| -> Option<usize> {
            let mut best: Option<(usize, f32)> = None;
            for (ti, segs) in tower_segs.iter().enumerate() {
                for (a, b) in segs {
                    let ab = *b - *a;
                    let t = ((ptr - *a).dot(ab) / ab.length_sq().max(1e-4)).clamp(0.0, 1.0);
                    let d = (*a + ab * t).distance(ptr);
                    if d < 12.0 && best.map_or(true, |(_, bd)| d < bd) {
                        best = Some((ti, d));
                    }
                }
            }
            best.map(|(ti, _)| ti)
        };

        // Stationary clicks (egui reports these separately from drags):
        // plain = select one / clear, ⇧ = toggle, ⌘ = select all of that type.
        if resp.clicked() && !mods.alt {
            if let Some(ptr) = resp.interact_pointer_pos() {
                let on_transition = transition
                    .as_deref()
                    .is_some_and(|tr| self.transition_sphere_hit(rect, tr, ptr));
                let on_chase = !on_transition
                    && chase
                        .as_deref()
                        .is_some_and(|c| self.chase_sphere_hit(rect, c, ptr));
                let on_gizmo = !on_transition
                    && !on_chase
                    && self
                        .selection_centroid()
                        .and_then(|o| self.gizmo_pick(rect, o, ptr))
                        .is_some();
                if on_transition {
                    select_sphere(&mut transition, &mut chase, true);
                    self.selection.clear();
                    self.sel_tower = None;
                    self.sel_stage = false;
                } else if on_chase {
                    select_sphere(&mut transition, &mut chase, false);
                    self.selection.clear();
                    self.sel_tower = None;
                    self.sel_stage = false;
                } else if on_gizmo {
                    // A click on a gizmo handle must not change the selection.
                } else if let Some(i) = hit_test(ptr) {
                    deselect_spheres(&mut transition, &mut chase);
                    self.sel_tower = None;
                    self.sel_stage = false;
                    let fi = self.instances[i].fixture;
                    if mods.command {
                        self.select_same_type(patch, fi);
                    } else if mods.shift {
                        if !self.selection.insert(i) {
                            self.selection.remove(&i);
                        }
                        self.last_selected = Some(fi);
                    } else {
                        self.selection.clear();
                        self.selection.insert(i);
                        self.last_selected = Some(fi);
                    }
                } else if let Some(ti) = tower_hit(ptr) {
                    deselect_spheres(&mut transition, &mut chase);
                    self.selection.clear();
                    self.sel_tower = Some(ti);
                    self.sel_stage = false;
                } else if self.sel_stage && self.stage_handle_pick(rect, set, ptr).is_some() {
                    // A click on a stage resize arrow keeps the selection.
                } else if self.stage_box_hit(rect, set, ptr) {
                    // Click the stage box itself to reveal its resize arrows.
                    deselect_spheres(&mut transition, &mut chase);
                    self.selection.clear();
                    self.sel_tower = None;
                    self.sel_stage = true;
                } else if !mods.shift && !mods.command {
                    deselect_spheres(&mut transition, &mut chase);
                    self.selection.clear();
                    self.sel_tower = None;
                    self.sel_stage = false;
                }
            }
        }

        if !mods.alt {
            if resp.drag_started_by(egui::PointerButton::Primary) {
                if let Some(ptr) = resp.interact_pointer_pos() {
                    let on_transition = transition
                        .as_deref()
                        .is_some_and(|tr| self.transition_sphere_hit(rect, tr, ptr));
                    let on_chase = !on_transition
                        && chase
                            .as_deref()
                            .is_some_and(|c| self.chase_sphere_hit(rect, c, ptr));
                    let gizmo = if on_transition || on_chase {
                        None
                    } else {
                        self.selection_centroid()
                            .and_then(|o| self.gizmo_pick(rect, o, ptr))
                    };
                    if on_transition {
                        select_sphere(&mut transition, &mut chase, true);
                        self.selection.clear();
                        self.sel_tower = None;
                        self.drag = Drag::MoveTransitionSphere;
                    } else if on_chase {
                        select_sphere(&mut transition, &mut chase, false);
                        self.selection.clear();
                        self.sel_tower = None;
                        self.drag = Drag::MoveChaseSphere;
                    } else if let Some(part) = gizmo {
                        // Grab a gizmo handle: fine, axis-locked manipulation.
                        self.push_undo();
                        if part.is_rot() {
                            if let Some(origin) = self.selection_centroid() {
                                if let Some((o_s, _)) = self.cam.project(rect, origin) {
                                    self.gizmo_last_angle =
                                        (ptr.y - o_s.y).atan2(ptr.x - o_s.x);
                                }
                            }
                        }
                        for &j in &self.selection {
                            if let Some(inst) = self.instances.get_mut(j) {
                                inst.mount = None;
                            }
                        }
                        self.drag = Drag::Gizmo(part);
                    } else if let Some(i) = hit_test(ptr) {
                        self.sel_tower = None;
                        self.sel_stage = false;
                        if mods.shift {
                            if !self.selection.insert(i) {
                                self.selection.remove(&i);
                            }
                        } else if !self.selection.contains(&i) {
                            self.selection.clear();
                            self.selection.insert(i);
                        }
                        self.last_selected = Some(self.instances[i].fixture);
                        // Dragging detaches from towers (re-snaps on drop).
                        self.push_undo();
                        for &j in &self.selection {
                            if let Some(inst) = self.instances.get_mut(j) {
                                inst.mount = None;
                            }
                        }
                        self.drag = Drag::Move;
                    } else if let Some(ti) = tower_hit(ptr) {
                        self.selection.clear();
                        self.sel_tower = Some(ti);
                        self.sel_stage = false;
                        self.push_undo();
                        self.drag = Drag::MoveTower;
                    } else if self.sel_stage {
                        if let Some(h) = self.stage_handle_pick(rect, set, ptr) {
                            // Grab a stage resize arrow.
                            self.drag = Drag::StageEdge(h);
                        } else if mods.shift {
                            self.sel_tower = None;
                            self.drag = Drag::Marquee(ptr);
                        } else {
                            self.selection.clear();
                            self.sel_tower = None;
                            self.drag = Drag::PanCam;
                        }
                    } else if mods.shift {
                        // ⇧+drag on empty space = marquee multi-select.
                        self.sel_tower = None;
                        self.drag = Drag::Marquee(ptr);
                    } else {
                        // Plain drag on empty space pans the camera.
                        self.selection.clear();
                        self.sel_tower = None;
                        self.drag = Drag::PanCam;
                    }
                }
            }
            if resp.dragged_by(egui::PointerButton::Primary) {
                let d = resp.drag_delta();
                let wpp = self.cam.world_per_pixel(rect);
                let right = v3(self.cam.yaw.cos(), 0.0, -self.cam.yaw.sin());
                let fwd_xz = v3(-self.cam.yaw.sin(), 0.0, -self.cam.yaw.cos());
                match self.drag {
                    Drag::Move => {
                        for &i in &self.selection {
                            let Some(inst) = self.instances.get_mut(i) else { continue };
                            if mods.command {
                                // Vertical move.
                                inst.t.pos.y -= d.y * wpp;
                            } else {
                                // Ground-plane move.
                                inst.t.pos =
                                    inst.t.pos + right * (d.x * wpp) + fwd_xz * (-d.y * wpp);
                            }
                        }
                        // Snap preview: if a light's icon hovers over a free
                        // tower slot (in screen space), pull it onto the slot.
                        let preview = self.compute_snap(rect);
                        for (&i, &(ti, slot)) in &preview {
                            if let Some(pos) = self.towers.get(ti).map(|tw| tw.slot_pos(slot)) {
                                if let Some(inst) = self.instances.get_mut(i) {
                                    inst.t.pos = pos;
                                }
                            }
                        }
                        self.snap_preview = preview;
                    }
                    Drag::MoveTower => {
                        if let Some(tw) =
                            self.sel_tower.and_then(|ti| self.towers.get_mut(ti))
                        {
                            if mods.command {
                                tw.height = (tw.height - d.y * wpp).clamp(1.2, 6.0);
                            } else {
                                tw.pos =
                                    tw.pos + right * (d.x * wpp) + fwd_xz * (-d.y * wpp);
                                tw.pos.y = 0.0;
                            }
                        }
                    }
                    Drag::MoveTransitionSphere => {
                        if let Some(tr) = transition.as_deref_mut() {
                            if mods.shift && tr.mode == TransitionMode::SphereScan {
                                tr.sphere.yaw_deg = (tr.sphere.yaw_deg + d.x * 0.25)
                                    .rem_euclid(360.0);
                            } else {
                                self.move_transition_sphere(rect, tr, d, mods.command);
                            }
                        }
                    }
                    Drag::MoveChaseSphere => {
                        if let Some(c) = chase.as_deref_mut() {
                            if mods.shift {
                                c.sphere.yaw_deg =
                                    (c.sphere.yaw_deg + d.x * 0.25).rem_euclid(360.0);
                            } else {
                                self.move_sphere(rect, &mut c.sphere.pos, d, mods.command);
                            }
                        }
                    }
                    Drag::PanCam => {
                        // Slide the camera target across the view plane.
                        self.cam.target =
                            self.cam.target - right * (d.x * wpp) + v3(0.0, d.y * wpp, 0.0);
                    }
                    Drag::Gizmo(part) => {
                        if let Some(origin) = self.selection_centroid() {
                            let arm = self.gizmo_arm(rect);
                            let sel: Vec<usize> = self.selection.iter().copied().collect();
                            if part.is_rot() {
                                // Rotate around the picked world axis through
                                // the selection centre.
                                if let (Some(ptr), Some((o_s, _))) = (
                                    resp.interact_pointer_pos(),
                                    self.cam.project(rect, origin),
                                ) {
                                    let cur = (ptr.y - o_s.y).atan2(ptr.x - o_s.x);
                                    let mut dth = cur - self.gizmo_last_angle;
                                    let pi = std::f32::consts::PI;
                                    let tau = std::f32::consts::TAU;
                                    while dth > pi {
                                        dth -= tau;
                                    }
                                    while dth < -pi {
                                        dth += tau;
                                    }
                                    self.gizmo_last_angle = cur;
                                    let axis = part.axis();
                                    let deg = dth.to_degrees();
                                    for i in sel {
                                        let Some(inst) = self.instances.get_mut(i) else {
                                            continue;
                                        };
                                        inst.t.pos =
                                            origin + rotate_about(inst.t.pos - origin, axis, dth);
                                        match part {
                                            GizmoPart::RotY => inst.t.yaw_deg += deg,
                                            GizmoPart::RotX => inst.t.pitch_deg += deg,
                                            GizmoPart::RotZ => {
                                                inst.t.roll_deg =
                                                    (inst.t.roll_deg + deg).rem_euclid(360.0)
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            } else {
                                // Translate along the picked world axis. The
                                // pointer delta is projected onto the on-screen
                                // axis so the motion tracks the cursor 1:1.
                                let ax = part.axis();
                                let o_s = self.cam.project(rect, origin).map(|x| x.0);
                                let tip_s = self.cam.project(rect, origin + ax * arm).map(|x| x.0);
                                if let (Some(o_s), Some(tip_s)) = (o_s, tip_s) {
                                    let sa = tip_s - o_s;
                                    let len = sa.length().max(1.0);
                                    let dir = sa / len;
                                    let amt = d.x * dir.x + d.y * dir.y;
                                    let world = amt * arm / len;
                                    let mv = ax * world;
                                    for i in sel {
                                        if let Some(inst) = self.instances.get_mut(i) {
                                            inst.t.pos = inst.t.pos + mv;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Drag::StageEdge(h) => {
                        let world = self.stage_handle_world_delta(rect, set, h, d);
                        h.apply(set, world);
                    }
                    _ => {}
                }
            }
            if resp.drag_stopped_by(egui::PointerButton::Primary) {
                match std::mem::replace(&mut self.drag, Drag::None) {
                    Drag::Move => {
                        self.commit_snap(patch);
                    }
                    Drag::MoveTower => self.save(patch),
                    Drag::Marquee(start) => {
                        if let Some(end) = resp.interact_pointer_pos() {
                            let sel = Rect::from_two_pos(start, end);
                            for (i, p) in proj.iter().enumerate() {
                                if let Some((sp, _)) = p {
                                    if sel.contains(*sp) {
                                        self.selection.insert(i);
                                        self.last_selected =
                                            self.instances.get(i).map(|inst| inst.fixture);
                                    }
                                }
                            }
                        }
                    }
                    Drag::None => {}
                    Drag::PanCam => {}
                    Drag::MoveTransitionSphere => {}
                    Drag::MoveChaseSphere => {}
                    Drag::StageEdge(_) => set.save(),
                    Drag::Gizmo(_) => self.save(patch),
                }
            }
        }

        // Keyboard: ⌘Z undoes, ⌘D duplicates, ⌫ removes copies / a tower.
        if resp.hovered() {
            let (undo, dup, del) = ui.input(|inp| {
                (
                    inp.modifiers.command && inp.key_pressed(egui::Key::Z),
                    inp.modifiers.command && inp.key_pressed(egui::Key::D),
                    inp.key_pressed(egui::Key::Backspace) || inp.key_pressed(egui::Key::Delete),
                )
            });
            if undo {
                self.undo(patch);
            }
            if dup && !self.selection.is_empty() {
                self.duplicate_selection(patch);
            }
            if del {
                if !self.selection.is_empty() {
                    self.delete_selection(patch);
                } else if let Some(ti) = self.sel_tower.take() {
                    self.delete_tower(patch, ti);
                }
            }
        }

        // ---- render the scene ----
        self.draw_scene(
            &painter,
            &resp,
            rect,
            patch,
            buf,
            set,
            &proj,
            transition.as_deref(),
            chase.as_deref(),
        );
    }
}

/// Clear the "selected" highlight on whichever stage spheres are present.
fn deselect_spheres(
    transition: &mut Option<&mut TransitionConfig>,
    chase: &mut Option<&mut ChaseConfig>,
) {
    if let Some(tr) = transition.as_deref_mut() {
        tr.selected = false;
    }
    if let Some(c) = chase.as_deref_mut() {
        c.selected = false;
    }
}

/// Highlight exactly one stage sphere (transition or chase), clearing the
/// other so only a single marker is ever active.
fn select_sphere(
    transition: &mut Option<&mut TransitionConfig>,
    chase: &mut Option<&mut ChaseConfig>,
    want_transition: bool,
) {
    if let Some(tr) = transition.as_deref_mut() {
        tr.selected = want_transition;
    }
    if let Some(c) = chase.as_deref_mut() {
        c.selected = !want_transition;
    }
}
