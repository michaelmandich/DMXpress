//! Scene rendering: grid, stage box, depth-sorted lights/beams/towers, the
//! transform gizmo, selection labels, marquee overlay, and help text.

use std::collections::HashSet;

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Shape, Stroke};

use super::fixture::{classify, live_state, vis_curve, Archetype};
use super::gizmo::{Drag, GizmoPart, StageHandle};
use super::layout::TOWER_SLOTS;
use super::math::{dir_from_angles, v3, V3};
use super::render::{add_box, add_cylinder, mesh_shapes, surface_pool_shape, Mesh};
use super::settings::Settings;
use super::view::StageView;
use super::volumetric::{paint_callback, BeamSpec};
use crate::chase::ChaseConfig;
use crate::showbuddy::Patch;
use crate::transition::TransitionConfig;

impl StageView {
    /// Draw the whole 3D scene. `proj` holds the projected body position of
    /// each instance (computed once during input handling).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_scene(
        &self,
        painter: &egui::Painter,
        resp: &egui::Response,
        rect: Rect,
        patch: &Patch,
        buf: &[u8; crate::net::DMX_SLOTS],
        set: &Settings,
        proj: &[Option<(Pos2, f32)>],
        transition: Option<&TransitionConfig>,
        chase: Option<&ChaseConfig>,
    ) {
        // ---- static scene ----
        self.draw_grid(painter, rect);
        self.draw_stage_box(painter, rect, set);

        // ---- lights & beams, painter-sorted back to front ----
        struct Item {
            depth: f32,
            shapes: Vec<Shape>,
        }
        let eye = self.cam.eye();
        let mut items: Vec<Item> = Vec::new();
        let mut beams: Vec<BeamSpec> = Vec::new();
        let mut pools: Vec<Shape> = Vec::new();

        for (i, inst) in self.instances.iter().enumerate() {
            let Some(f) = patch.fixtures.get(inst.fixture) else {
                continue;
            };
            let t = &inst.t;
            let arch = classify(f);
            let live = live_state(f, buf);
            let mut shapes = Vec::new();

            // Mounting frame: f_fwd is the home beam direction.
            let f_fwd = dir_from_angles(t.yaw_deg, t.pitch_deg);
            let helper = if f_fwd.y.abs() > 0.9 {
                v3(0.0, 0.0, 1.0)
            } else {
                v3(0.0, 1.0, 0.0)
            };
            let r0b = helper.cross(f_fwd).norm();
            let u0b = f_fwd.cross(r0b).norm();
            // Spin the mounting frame around its forward axis (base spin).
            // This avoids gimbal tumbling near pitch ±90° (e.g. hung on a bar):
            // the base just rotates in place and the pan/tilt sweep follows.
            let (rs, rc) = t.roll_deg.to_radians().sin_cos();
            let r0 = r0b * rc + u0b * rs;
            let u0 = u0b * rc - r0b * rs;

            let s = set.light_scale * t.scale;
            let visual_opacity = inst.opacity.clamp(0.0, 1.0);
            let visual_alpha = (visual_opacity * 255.0).round() as u8;
            let housing = Color32::from_rgba_unmultiplied(
                48,
                48,
                48,
                visual_alpha,
            );
            let emit = if live.brightness > 0.02 {
                // Show the hue at a perceptual level so the lens still glows
                // its colour when the fixture runs dim.
                let m = live.brightness.max(1e-3);
                let disp = (0.3 + 0.7 * vis_curve(live.brightness)).min(1.0);
                let k = disp / m;
                Color32::from_rgba_unmultiplied(
                    (live.color.r() as f32 * k).min(255.0) as u8,
                    (live.color.g() as f32 * k).min(255.0) as u8,
                    (live.color.b() as f32 * k).min(255.0) as u8,
                    visual_alpha,
                )
            } else {
                Color32::from_rgba_unmultiplied(60, 60, 60, visual_alpha)
            };

            // Body mesh + emit point/direction per archetype.
            let mut mesh = Mesh::default();
            let (dir, apex) = match arch {
                Archetype::MovingPar | Archetype::Beam => {
                    // Pan spins the yoke around the mount axis, tilt the head.
                    let pan_a = ((live.pan - 0.5) * f.pan_range).to_radians();
                    let p_r = r0 * pan_a.cos() + u0 * pan_a.sin();
                    let p_u = p_r.cross(f_fwd).norm();
                    let tilt_a = ((live.tilt - 0.5) * f.tilt_range).to_radians();
                    let head_dir = (f_fwd * tilt_a.cos() + p_u * tilt_a.sin()).norm();

                    // Base slab at the mount point.
                    add_box(
                        &mut mesh,
                        t.pos,
                        r0 * (0.17 * s),
                        u0 * (0.17 * s),
                        f_fwd * (0.05 * s),
                        housing,
                        None,
                    );
                    // Two yoke prongs, rotating with pan.
                    for side in [-1.0f32, 1.0] {
                        add_box(
                            &mut mesh,
                            t.pos + f_fwd * (0.19 * s) + p_r * (0.15 * side * s),
                            p_r * (0.025 * s),
                            p_u * (0.055 * s),
                            f_fwd * (0.14 * s),
                            housing,
                            None,
                        );
                    }
                    // Head cylinder between the prongs; open end emits.
                    let pivot = t.pos + f_fwd * (0.27 * s);
                    add_cylinder(
                        &mut mesh,
                        pivot,
                        head_dir,
                        0.085 * s,
                        0.15 * s,
                        Color32::from_gray(58),
                        Some(emit),
                    );
                    (head_dir, pivot + head_dir * (0.15 * s))
                }
                Archetype::Bar => {
                    // 1:5 prism, long axis sideways, front face emits.
                    add_box(
                        &mut mesh,
                        t.pos,
                        r0 * (0.5 * s),
                        u0 * (0.1 * s),
                        f_fwd * (0.1 * s),
                        housing,
                        Some(emit),
                    );
                    (f_fwd, t.pos + f_fwd * (0.1 * s))
                }
                Archetype::Specialty => {
                    add_box(
                        &mut mesh,
                        t.pos,
                        r0 * (0.1 * s),
                        u0 * (0.1 * s),
                        f_fwd * (0.1 * s),
                        housing,
                        None,
                    );
                    (f_fwd, t.pos)
                }
                Archetype::Par => {
                    // Short circular can, front cap emits.
                    add_cylinder(
                        &mut mesh,
                        t.pos,
                        f_fwd,
                        0.12 * s,
                        0.09 * s,
                        housing,
                        Some(emit),
                    );
                    (f_fwd, t.pos + f_fwd * (0.09 * s))
                }
            };

            if !matches!(arch, Archetype::Specialty)
                && live.brightness > 0.02
                && visual_opacity > 0.001
            {
                // Beams are narrow and long; pars/bars throw a wide wash that
                // dissipates much sooner.
                let (narrow, min_half, max_len) = match arch {
                    Archetype::Beam => (0.35, 1.0, 16.0),
                    Archetype::MovingPar => (1.0, 6.0, 14.0),
                    _ => (1.0, 14.0, 10.0),
                };
                let half_deg = (f.beam_width * 0.5 * narrow * (0.35 + live.zoom * 1.3))
                    .clamp(min_half, 50.0);
                // Stop first on the raised stage top when the footprint lies
                // inside it, otherwise on the surrounding ground plane.
                let mut surface_hit: Option<(f32, f32)> = None; // (distance, y)
                if dir.y < -0.02 {
                    let ground_t = apex.y / -dir.y;
                    if ground_t > 0.0 {
                        surface_hit = Some((ground_t, 0.0));
                    }
                    let stage_t = (apex.y - set.stage_h) / -dir.y;
                    if stage_t > 0.0 {
                        let hit = apex + dir * stage_t;
                        if hit.x.abs() <= set.stage_half_w && hit.z.abs() <= set.stage_half_d {
                            surface_hit = Some((stage_t, set.stage_h));
                        }
                    }
                }
                let len = surface_hit.map_or(max_len, |(t, _)| t.min(max_len));
                if let Some((hit_t, hit_y)) = surface_hit.filter(|(t, _)| *t <= max_len) {
                    let mut hit = apex + dir * hit_t;
                    hit.y = hit_y;
                    let pool_radius = (0.05 + half_deg.to_radians().tan() * hit_t) * 1.18;
                    if let Some(pool) = surface_pool_shape(
                        &self.cam,
                        rect,
                        hit,
                        dir,
                        pool_radius,
                        live.color,
                        live.brightness,
                        set.beam_opacity * visual_opacity,
                    ) {
                        pools.push(pool);
                    }
                }
                beams.push(BeamSpec {
                    apex,
                    dir,
                    len,
                    half_angle: half_deg.to_radians(),
                    color: live.color,
                    brightness: live.brightness,
                    opacity: set.beam_opacity * visual_opacity,
                });
            }

            if let Some(Some((sp, z))) = proj.get(i).copied() {
                let r = (160.0 / z).clamp(3.0, 24.0);
                shapes.extend(mesh_shapes(&self.cam, rect, &mesh));
                // Orientation tick so off lights still show where they aim.
                if live.brightness <= 0.02 {
                    if let Some((tip, _)) = self.cam.project(rect, apex + dir * 0.7) {
                        shapes.push(Shape::line_segment(
                            [sp, tip],
                            Stroke::new(1.0, Color32::from_gray(70)),
                        ));
                    }
                }
                if self.selection.contains(&i) {
                    shapes.push(Shape::circle_stroke(
                        sp,
                        r * 1.2,
                        Stroke::new(2.0, Color32::YELLOW),
                    ));
                }
            }

            items.push(Item {
                depth: (t.pos - eye).len(),
                shapes,
            });
        }
        // Surface illumination belongs on the stage/ground, below physical
        // fixture bodies and towers.
        painter.extend(pools);
        // Towers (drawn with the same depth sort as the lights).
        let occupied: HashSet<(usize, usize)> =
            self.instances.iter().filter_map(|inst| inst.mount).collect();
        // Slots a light is currently hovering over (live snap target).
        let snap_targets: HashSet<(usize, usize)> =
            self.snap_preview.values().copied().collect();
        for (ti, tw) in self.towers.iter().enumerate() {
            let selected = self.sel_tower == Some(ti);
            let mut shapes = mesh_shapes(&self.cam, rect, &tw.mesh(selected));
            // Slot markers while placing lights or when the tower is picked.
            if selected || matches!(self.drag, Drag::Move) {
                for slot in 0..TOWER_SLOTS {
                    if let Some((sp, _)) = self.cam.project(rect, tw.slot_pos(slot)) {
                        if snap_targets.contains(&(ti, slot)) {
                            // Highlighted drop target.
                            shapes.push(Shape::circle_filled(
                                sp,
                                7.0,
                                Color32::from_rgba_unmultiplied(120, 230, 130, 90),
                            ));
                            shapes.push(Shape::circle_stroke(
                                sp,
                                8.0,
                                Stroke::new(2.0, Color32::from_rgb(120, 240, 130)),
                            ));
                        } else {
                            let col = if occupied.contains(&(ti, slot)) {
                                Color32::from_gray(75)
                            } else {
                                Color32::from_rgb(90, 170, 255)
                            };
                            shapes.push(Shape::circle_stroke(sp, 5.0, Stroke::new(1.5, col)));
                        }
                    }
                }
            }
            items.push(Item {
                depth: (tw.pos + v3(0.0, tw.height * 0.5, 0.0) - eye).len(),
                shapes,
            });
        }

        items.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        for it in items {
            painter.extend(it.shapes);
        }

        // Composite participating media after scene geometry. Thick/head-on
        // haze can now veil bodies behind it, while the emissive lens remains
        // faintly visible through physically bounded beam alpha.
        if let Some(callback) = paint_callback(&self.cam, rect, &beams) {
            painter.add(Shape::Callback(callback));
        }

        // ---- transform gizmo (Fusion-style axis arrows + rotation rings) ----
        if let Some(origin) = self.selection_centroid() {
            if let Some((o_s, _)) = self.cam.project(rect, origin) {
                let arm = self.gizmo_arm(rect);
                let active = match self.drag {
                    Drag::Gizmo(p) => Some(p),
                    _ => None,
                };
                let hover = if active.is_none() {
                    resp.hover_pos().and_then(|p| self.gizmo_pick(rect, origin, p))
                } else {
                    None
                };
                let lit = |part: GizmoPart| active == Some(part) || hover == Some(part);
                let tau = std::f32::consts::TAU;

                // Rotation rings first (so the arrows sit on top near centre).
                for part in [GizmoPart::RotX, GizmoPart::RotY, GizmoPart::RotZ] {
                    let (u, v) = Self::ring_basis(part.axis());
                    let mut pts: Vec<Pos2> = Vec::new();
                    for k in 0..=64 {
                        let a = k as f32 / 64.0 * tau;
                        if let Some((sp, _)) =
                            self.cam.project(rect, origin + (u * a.cos() + v * a.sin()) * arm)
                        {
                            pts.push(sp);
                        }
                    }
                    let w = if lit(part) { 3.0 } else { 1.6 };
                    let col = part.color();
                    for seg in pts.windows(2) {
                        painter.line_segment([seg[0], seg[1]], Stroke::new(w, col));
                    }
                }

                // Translation arrows.
                for part in [GizmoPart::TransX, GizmoPart::TransY, GizmoPart::TransZ] {
                    let ax = part.axis();
                    if let Some((tip, _)) = self.cam.project(rect, origin + ax * arm) {
                        let col = part.color();
                        let w = if lit(part) { 3.5 } else { 2.2 };
                        painter.line_segment([o_s, tip], Stroke::new(w, col));
                        let dir = tip - o_s;
                        let dl = dir.length().max(1.0);
                        let dn = dir / dl;
                        let perp = egui::vec2(-dn.y, dn.x);
                        let h = if lit(part) { 12.0 } else { 9.0 };
                        painter.add(Shape::convex_polygon(
                            vec![tip, tip - dn * h + perp * (h * 0.5), tip - dn * h - perp * (h * 0.5)],
                            col,
                            Stroke::NONE,
                        ));
                    }
                }
                painter.circle_filled(o_s, 3.0, Color32::from_gray(235));
            }
        }

        // ---- stage resize arrows (shown when the stage box is selected) ----
        if self.sel_stage {
            let arm = self.stage_arrow_arm(rect);
            let active = match self.drag {
                Drag::StageEdge(h) => Some(h),
                _ => None,
            };
            let hover = if active.is_none() {
                resp.hover_pos()
                    .and_then(|p| self.stage_handle_pick(rect, set, p))
            } else {
                None
            };
            for h in StageHandle::ALL {
                let a = h.anchor(set);
                if let (Some((o_s, _)), Some((tip, _))) = (
                    self.cam.project(rect, a),
                    self.cam.project(rect, a + h.axis() * arm),
                ) {
                    let lit = active == Some(h) || hover == Some(h);
                    let col = if lit {
                        Color32::from_rgb(255, 195, 80)
                    } else {
                        Color32::from_rgb(190, 145, 65)
                    };
                    let w = if lit { 3.2 } else { 1.8 };
                    painter.line_segment([o_s, tip], Stroke::new(w, col));
                    let dir = tip - o_s;
                    let dl = dir.length().max(1.0);
                    let dn = dir / dl;
                    let perp = egui::vec2(-dn.y, dn.x);
                    let hh = if lit { 11.0 } else { 8.0 };
                    painter.add(Shape::convex_polygon(
                        vec![
                            tip,
                            tip - dn * hh + perp * (hh * 0.5),
                            tip - dn * hh - perp * (hh * 0.5),
                        ],
                        col,
                        Stroke::NONE,
                    ));
                    painter.circle_filled(o_s, if lit { 3.0 } else { 2.2 }, col);
                }
            }
        }

        // Labels for selected lights.
        for &i in &self.selection {
            let Some(inst) = self.instances.get(i) else { continue };
            if let (Some(Some((sp, _))), Some(f)) =
                (proj.get(i), patch.fixtures.get(inst.fixture))
            {
                painter.text(
                    *sp + egui::vec2(0.0, 14.0),
                    Align2::CENTER_TOP,
                    &f.display,
                    FontId::proportional(10.0),
                    Color32::from_gray(210),
                );
            }
        }

        // Marquee overlay.
        if let Drag::Marquee(start) = self.drag {
            if let Some(end) = resp.interact_pointer_pos() {
                let r = Rect::from_two_pos(start, end);
                painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(100, 150, 255, 18));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_rgb(100, 150, 255)));
            }
        }

        painter.text(
            rect.left_bottom() + egui::vec2(6.0, -6.0),
            Align2::LEFT_BOTTOM,
            if self.fly_mode {
                "FREE FLY · WASD: move · Space/Shift: up/down · arrows/right-drag: look · scroll: speed · click/drag lights normally"
            } else {
                "drag light: move · gizmo arrows/rings: fine move & rotate · click stage: resize arrows · ⌘drag: height · drag empty: pan · ⇧drag empty: marquee · ⇧click: multi · ⌘click: select type · ⌘Z: undo · ⌘D: duplicate · ⌫: remove copy · right-drag: orbit · scroll: zoom"
            },
            FontId::proportional(10.5),
            Color32::from_gray(120),
        );
        self.draw_transition_overlay(painter, rect, transition);
        self.draw_chase_overlay(painter, rect, chase);
    }

    fn line3(&self, painter: &egui::Painter, rect: Rect, a: V3, b: V3, stroke: Stroke) {
        if let (Some((pa, _)), Some((pb, _))) =
            (self.cam.project(rect, a), self.cam.project(rect, b))
        {
            painter.line_segment([pa, pb], stroke);
        }
    }

    fn draw_grid(&self, painter: &egui::Painter, rect: Rect) {
        let stroke = Stroke::new(1.0, Color32::from_gray(30));
        let (n, s) = (8i32, 2.0f32);
        let ext = n as f32 * s;
        for i in -n..=n {
            let a = i as f32 * s;
            self.line3(painter, rect, v3(a, 0.0, -ext), v3(a, 0.0, ext), stroke);
            self.line3(painter, rect, v3(-ext, 0.0, a), v3(ext, 0.0, a), stroke);
        }
    }

    fn draw_stage_box(&self, painter: &egui::Painter, rect: Rect, set: &Settings) {
        let (hw, h, hd) = (set.stage_half_w, set.stage_h, set.stage_half_d);
        let edge_col = if self.sel_stage {
            Color32::from_rgb(190, 145, 65)
        } else {
            Color32::from_gray(95)
        };
        let top = [
            v3(-hw, h, -hd),
            v3(hw, h, -hd),
            v3(hw, h, hd),
            v3(-hw, h, hd),
        ];
        let bot = [
            v3(-hw, 0.0, -hd),
            v3(hw, 0.0, -hd),
            v3(hw, 0.0, hd),
            v3(-hw, 0.0, hd),
        ];
        let pts: Option<Vec<Pos2>> = top
            .iter()
            .map(|p| self.cam.project(rect, *p).map(|(q, _)| q))
            .collect();
        if let Some(pts) = pts {
            painter.add(Shape::convex_polygon(
                pts,
                Color32::from_gray(36),
                Stroke::new(1.0, edge_col),
            ));
        }
        let edge = Stroke::new(1.0, edge_col);
        for k in 0..4 {
            self.line3(painter, rect, bot[k], bot[(k + 1) % 4], edge);
            self.line3(painter, rect, bot[k], top[k], edge);
        }
    }
}
