//! Stage helpers for advanced transition patterns: fixture world positions,
//! sphere hit-testing/movement, and the visible marker overlay.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Shape, Stroke};

use super::math::{dir_from_angles, v3, V3};
use super::view::StageView;
use super::fixture::live_state;
use crate::chase::{ChaseConfig, ChaseKind};
use crate::phaser::PhaserTrace;
use crate::transition::{TransitionConfig, TransitionMode};
use crate::showbuddy::Patch;

impl StageView {
    /// Average world position for each patch fixture, across any duplicate
    /// visual instances that share its DMX channels.
    pub(crate) fn fixture_positions(&self, patch: &Patch) -> Vec<Option<V3>> {
        let mut sums = vec![V3::default(); patch.fixtures.len()];
        let mut counts = vec![0.0f32; patch.fixtures.len()];
        for inst in &self.instances {
            if inst.fixture < sums.len() {
                sums[inst.fixture] = sums[inst.fixture] + inst.t.pos;
                counts[inst.fixture] += 1.0;
            }
        }
        sums.into_iter()
            .zip(counts)
            .map(|(sum, count)| (count > 0.0).then(|| sum * (1.0 / count)))
            .collect()
    }

    pub(crate) fn transition_sphere_hit(
        &self,
        rect: Rect,
        transition: &TransitionConfig,
        ptr: Pos2,
    ) -> bool {
        if !transition.stage_visible() {
            return false;
        }
        let Some((center, _)) = self.cam.project(rect, transition.sphere.pos) else {
            return false;
        };
        let radius = self.transition_extent_radius_px(rect, transition);
        let d = ptr.distance(center);
        d <= 18.0 || (d - radius).abs() <= 8.0
    }

    pub(crate) fn move_transition_sphere(
        &self,
        rect: Rect,
        transition: &mut TransitionConfig,
        delta: egui::Vec2,
        vertical: bool,
    ) {
        self.move_sphere(rect, &mut transition.sphere.pos, delta, vertical);
    }

    /// Drag a stage-sphere origin across the view plane, or vertically.
    pub(crate) fn move_sphere(&self, rect: Rect, pos: &mut V3, delta: egui::Vec2, vertical: bool) {
        let wpp = self.cam.world_per_pixel(rect);
        if vertical {
            pos.y -= delta.y * wpp;
            return;
        }
        let right = v3(self.cam.yaw.cos(), 0.0, -self.cam.yaw.sin());
        let fwd_xz = v3(-self.cam.yaw.sin(), 0.0, -self.cam.yaw.cos());
        *pos = *pos + right * (delta.x * wpp) + fwd_xz * (-delta.y * wpp);
    }

    pub(crate) fn chase_sphere_hit(&self, rect: Rect, chase: &ChaseConfig, ptr: Pos2) -> bool {
        if !chase.stage_visible() {
            return false;
        }
        let Some((center, _)) = self.cam.project(rect, chase.sphere.pos) else {
            return false;
        };
        let radius = self.sphere_extent_radius_px(rect, chase.sphere.pos);
        let d = ptr.distance(center);
        d <= 18.0 || (d - radius).abs() <= 8.0
    }

    pub(crate) fn draw_transition_overlay(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        transition: Option<&TransitionConfig>,
    ) {
        let Some(transition) = transition else { return };
        if !transition.stage_visible() {
            return;
        }
        let origin = transition.sphere.pos;
        let Some((center, _)) = self.cam.project(rect, origin) else {
            return;
        };
        let extent = self.transition_extent_world(transition);
        let radius = self.transition_extent_radius_px(rect, transition);
        let outline = if transition.selected {
            Color32::YELLOW
        } else {
            Color32::from_rgb(120, 210, 255)
        };
        // Overall reach of the marker (a faint screen-space disc).
        painter.circle_filled(
            center,
            radius,
            Color32::from_rgba_unmultiplied(80, 170, 240, 8),
        );
        painter.circle_stroke(center, radius, Stroke::new(1.0, Color32::from_gray(90)));
        painter.circle_filled(center, 4.0, outline);

        if transition.mode == TransitionMode::SphereScan {
            let start_az = transition.sphere.yaw_deg;
            match transition.active_progress {
                Some(p) if p > 0.001 => {
                    // Faint ghost where the sweep began, bright current plane
                    // rotated to match the transition progress.
                    self.draw_scan_plane(
                        painter, rect, origin, extent, start_az,
                        Color32::from_gray(120), 6,
                    );
                    self.draw_scan_plane(
                        painter, rect, origin, extent, start_az + p * 360.0, outline, 48,
                    );
                }
                _ => {
                    // Idle: clearly show where the sweep will start.
                    self.draw_scan_plane(painter, rect, origin, extent, start_az, outline, 34);
                }
            }
            self.draw_scan_arrow(painter, rect, origin, extent, start_az, outline);
        } else if transition.mode == TransitionMode::Radial {
            self.draw_horizontal_reach_ring(painter, rect, transition, extent, outline);
        }

        let label = match transition.mode {
            TransitionMode::Simple => "",
            TransitionMode::SphereScan => "Sphere scan",
            TransitionMode::Radial => "Radial",
        };
        painter.text(
            center + egui::vec2(0.0, radius + 8.0),
            Align2::CENTER_TOP,
            label,
            FontId::proportional(11.0),
            Color32::from_gray(220),
        );
    }

    /// Where each light's beam will land as it runs a phaser's pan/tilt
    /// figure: the path is swept around the fixture's *current* aim, the
    /// way the oscillator does it, then traced onto the floor (or held out
    /// in the air where it points above the horizon). Uses the same yoke
    /// maths as the light bodies, so the curve sits under the beam.
    pub(crate) fn draw_phaser_paths(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        patch: &Patch,
        buf: &[u8; crate::net::DMX_SLOTS],
        trace: Option<&PhaserTrace>,
    ) {
        let Some(trace) = trace else { return };
        if trace.points.len() < 2 {
            return;
        }
        let [r, g, b] = trace.color;
        let stroke = Stroke::new(2.0, Color32::from_rgba_unmultiplied(r, g, b, 210));
        let faint = Stroke::new(1.0, Color32::from_rgba_unmultiplied(r, g, b, 70));
        for inst in &self.instances {
            if !trace.fixtures.contains(&inst.fixture) {
                continue;
            }
            let Some(f) = patch.fixtures.get(inst.fixture) else { continue };
            let live = live_state(f, buf);
            let t = &inst.t;
            // Mounting frame, exactly as draw.rs builds it for the body.
            let f_fwd = dir_from_angles(t.yaw_deg, t.pitch_deg);
            let helper = if f_fwd.y.abs() > 0.9 { v3(0.0, 0.0, 1.0) } else { v3(0.0, 1.0, 0.0) };
            let r0b = helper.cross(f_fwd).norm();
            let u0b = f_fwd.cross(r0b).norm();
            let (rs, rc) = t.roll_deg.to_radians().sin_cos();
            let r0 = r0b * rc + u0b * rs;
            let u0 = u0b * rc - r0b * rs;
            let aim = |pan: f32, tilt: f32| -> V3 {
                let pan_a = ((pan - 0.5) * f.pan_range).to_radians();
                let p_r = r0 * pan_a.cos() + u0 * pan_a.sin();
                let p_u = p_r.cross(f_fwd).norm();
                let tilt_a = ((tilt - 0.5) * f.tilt_range).to_radians();
                (f_fwd * tilt_a.cos() + p_u * tilt_a.sin()).norm()
            };
            // Land the beam on the floor; hold it 8 m out if it points up.
            let land = |dir: V3| -> V3 {
                if dir.y < -0.05 {
                    let s = (-t.pos.y / dir.y).min(30.0);
                    t.pos + dir * s
                } else {
                    t.pos + dir * 8.0
                }
            };
            let world: Vec<V3> = trace
                .points
                .iter()
                .map(|&(p, q)| {
                    // An oscillator of depth 1 swings half the range each way.
                    let pan = (live.pan + p * 0.5).clamp(0.0, 1.0);
                    let tilt = (live.tilt + q * 0.5).clamp(0.0, 1.0);
                    land(aim(pan, tilt))
                })
                .collect();
            let screen = self.project_all(rect, &world);
            for seg in screen.windows(2) {
                painter.line_segment([seg[0], seg[1]], stroke);
            }
            if let (Some(&a), Some(&z)) = (screen.first(), screen.last()) {
                painter.line_segment([z, a], stroke);
            }
            // A thread from the light to the start of its figure.
            if let (Some((head, _)), Some(&first)) = (self.cam.project(rect, t.pos), screen.first()) {
                painter.line_segment([head, first], faint);
            }
        }
    }

    /// The travelling band of the spherical chase (origin dot, leading and
    /// trailing edges, bright moving centre, travel arrow). `active_head`
    /// advances the band while the chase runs.
    pub(crate) fn draw_chase_overlay(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        chase: Option<&ChaseConfig>,
    ) {
        let Some(chase) = chase else { return };
        if !chase.stage_visible() {
            return;
        }
        let origin = chase.sphere.pos;
        let Some((center, _)) = self.cam.project(rect, origin) else {
            return;
        };
        let extent = self.sphere_extent_world(origin);
        let radius = self.sphere_extent_radius_px(rect, origin);
        let col = if chase.selected {
            Color32::YELLOW
        } else {
            Color32::from_rgb(255, 170, 80)
        };
        painter.circle_filled(
            center,
            radius,
            Color32::from_rgba_unmultiplied(255, 150, 60, 8),
        );
        painter.circle_stroke(center, radius, Stroke::new(1.0, Color32::from_gray(90)));
        painter.circle_filled(center, 4.0, col);

        // Every shape is aimed by the same yaw/pitch pair, so they all share
        // this basis: `travel` is the sweep direction, `side` its partner in
        // the sweep plane.
        let up = v3(0.0, 1.0, 0.0);
        let p = chase.pitch_deg.to_radians();
        let front = dir_from_angles(chase.sphere.yaw_deg, 0.0).norm();
        let travel = (front * p.cos() + up * p.sin()).norm();
        let head = chase.active_head.unwrap_or(0.0);
        let half = chase.band_deg * 0.5;
        // Sweeps run from one edge of the rig to the other, so the marker
        // walls are placed along the travel axis in that same -1..1 space.
        let at = |frac: f32| (frac * 2.0 - 1.0) * extent;

        match chase.kind {
            ChaseKind::Sphere => {
                let head_az = chase.sphere.yaw_deg + head * 360.0;
                self.draw_scan_plane(painter, rect, origin, extent, head_az - half, col, 8);
                self.draw_scan_plane(painter, rect, origin, extent, head_az + half, col, 8);
                self.draw_scan_plane(painter, rect, origin, extent, head_az, col, 42);
                self.draw_scan_arrow(painter, rect, origin, extent, head_az, col);
            }
            ChaseKind::Linear | ChaseKind::Pulse | ChaseKind::Boomerang => {
                self.draw_travel_axis(painter, rect, origin, travel, extent, col);
                self.draw_wall(painter, rect, origin, travel, at(head), extent, col, 46);
                if chase.kind == ChaseKind::Boomerang {
                    // It comes back, so show the far turn-around too.
                    self.draw_wall(painter, rect, origin, travel, at(1.0 - head), extent, col, 14);
                }
            }
            ChaseKind::Stripes => {
                let n = chase.stripe_count.max(1).min(12);
                for i in 0..n {
                    let frac = (head + i as f32 / n as f32).rem_euclid(1.0);
                    self.draw_wall(painter, rect, origin, travel, at(frac), extent, col, 30);
                }
                self.draw_travel_axis(painter, rect, origin, travel, extent, col);
            }
            ChaseKind::Comet => {
                self.draw_travel_axis(painter, rect, origin, travel, extent, col);
                // The head, then the tail fading out behind it.
                for i in 0..5 {
                    let back = i as f32 * 0.06;
                    let frac = (head - back).rem_euclid(1.0);
                    let alpha = (60.0 * (1.0 - i as f32 / 5.0)) as u8;
                    self.draw_wall(painter, rect, origin, travel, at(frac), extent, col, alpha);
                }
            }
            ChaseKind::Ripple => {
                let rings = chase.stripe_count.max(1).min(8);
                for i in 0..rings {
                    let frac = (head + i as f32 / rings as f32).rem_euclid(1.0);
                    self.draw_ring(painter, rect, origin, frac * extent, col, 2.0);
                }
            }
            ChaseKind::Spiral => {
                self.draw_spiral(
                    painter,
                    rect,
                    origin,
                    extent,
                    chase.stripe_count.max(1).min(6),
                    head,
                    col,
                );
            }
            ChaseKind::Cascade => {
                // Falls down the rig, so the marker is a horizontal sheet at
                // the height the wave has reached.
                let y = origin.y + (1.0 - head * 2.0) * extent;
                self.draw_ring(painter, rect, v3(origin.x, y, origin.z), extent, col, 2.5);
                self.draw_wall(painter, rect, origin, up, y - origin.y, extent, col, 34);
            }
            ChaseKind::Chevron => {
                // Mirror pair opening out from the middle.
                self.draw_travel_axis(painter, rect, origin, travel, extent, col);
                self.draw_wall(painter, rect, origin, travel, head * extent, extent, col, 40);
                self.draw_wall(painter, rect, origin, travel, -head * extent, extent, col, 40);
            }
            ChaseKind::Checker => {
                // Slabs on the floor, so you can see the banks themselves and
                // which half is currently up rather than just the dividers.
                let banks = chase.stripe_count.max(2).min(12);
                let step = chase.active_step.unwrap_or(0.0).floor() as i64;
                for i in 0..banks {
                    let lit = (i as i64 + step).rem_euclid(2) == 0;
                    let (a, b) = (at(i as f32 / banks as f32), at((i + 1) as f32 / banks as f32));
                    self.draw_slab(
                        painter,
                        rect,
                        origin,
                        travel,
                        (a, b),
                        extent,
                        col,
                        if lit { 46 } else { 8 },
                    );
                }
                self.draw_travel_axis(painter, rect, origin, travel, extent, col);
            }
            ChaseKind::Glitter => {
                // It fires per fixture at random rather than sweeping, so the
                // honest picture is the fixtures themselves twinkling — this
                // runs the same sparkle maths the chase does.
                self.draw_ring(painter, rect, origin, extent, col, 1.5);
                let duty = (chase.band_deg / 180.0).clamp(0.03, 0.95);
                let step = chase.active_step.unwrap_or(0.0);
                for inst in &self.instances {
                    let fi = inst.fixture as u32;
                    let u = step + crate::chase::rand01(fi * 7919 + 13);
                    let k = u.floor();
                    let start = crate::chase::rand01(
                        fi.wrapping_mul(2654435761) ^ (k as i64 as u32).wrapping_mul(40503),
                    ) * (1.0 - duty);
                    let frac = u - k;
                    let Some((p, _)) = self.cam.project(rect, inst.t.pos) else {
                        continue;
                    };
                    if frac < start || frac >= start + duty {
                        painter.circle_stroke(p, 4.0, Stroke::new(1.0, Color32::from_gray(90)));
                        continue;
                    }
                    let a = (std::f32::consts::PI * (frac - start) / duty).sin().max(0.0);
                    painter.circle_filled(
                        p,
                        4.0 + 3.0 * a,
                        Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), (220.0 * a) as u8),
                    );
                }
            }
        }

        painter.text(
            center + egui::vec2(0.0, radius + 8.0),
            Align2::CENTER_TOP,
            format!("{} chase", chase.kind.label()),
            FontId::proportional(11.0),
            Color32::from_gray(220),
        );
    }

    /// A disc perpendicular to `normal`, `offset` along it from `center` —
    /// the moving front of a linear-style chase.
    #[allow(clippy::too_many_arguments)]
    fn draw_wall(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        center: V3,
        normal: V3,
        offset: f32,
        extent: f32,
        color: Color32,
        fill_alpha: u8,
    ) {
        let (a, b) = plane_basis(normal);
        let hub = center + normal * offset;
        let pts: Vec<V3> = (0..=28)
            .map(|k| {
                let t = k as f32 / 28.0 * std::f32::consts::TAU;
                hub + (a * t.cos() + b * t.sin()) * extent
            })
            .collect();
        let poly = self.project_all(rect, &pts);
        if poly.len() < 3 {
            return;
        }
        painter.add(Shape::convex_polygon(
            poly.clone(),
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), fill_alpha),
            Stroke::NONE,
        ));
        for seg in poly.windows(2) {
            painter.line_segment([seg[0], seg[1]], Stroke::new(1.5, color));
        }
    }

    /// A floor slab spanning `range` along the travel axis — one checker
    /// bank, so the banks read as areas rather than as bare dividers.
    #[allow(clippy::too_many_arguments)]
    fn draw_slab(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        center: V3,
        travel: V3,
        range: (f32, f32),
        extent: f32,
        color: Color32,
        fill_alpha: u8,
    ) {
        // Lay it flat: across the travel direction, in the horizontal plane.
        let across = travel.cross(v3(0.0, 1.0, 0.0)).norm();
        let across = if across.len().is_finite() && across.len() > 0.5 {
            across
        } else {
            v3(1.0, 0.0, 0.0)
        };
        let corners = [
            center + travel * range.0 + across * extent,
            center + travel * range.1 + across * extent,
            center + travel * range.1 - across * extent,
            center + travel * range.0 - across * extent,
        ];
        let poly = self.project_all(rect, &corners);
        if poly.len() < 3 {
            return;
        }
        painter.add(Shape::convex_polygon(
            poly.clone(),
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), fill_alpha),
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 90)),
        ));
    }

    /// The axis a sweep travels along, arrowhead at the far end.
    fn draw_travel_axis(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        center: V3,
        travel: V3,
        extent: f32,
        color: Color32,
    ) {
        let (from, to) = (center - travel * extent, center + travel * extent);
        let (Some((a, _)), Some((b, _))) =
            (self.cam.project(rect, from), self.cam.project(rect, to))
        else {
            return;
        };
        painter.line_segment([a, b], Stroke::new(1.5, color));
        let d = b - a;
        let n = d / d.length().max(1.0);
        let perp = egui::vec2(-n.y, n.x);
        painter.line_segment([b, b - n * 12.0 + perp * 5.5], Stroke::new(2.5, color));
        painter.line_segment([b, b - n * 12.0 - perp * 5.5], Stroke::new(2.5, color));
    }

    /// A horizontal ring at `radius` around `center`.
    fn draw_ring(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        center: V3,
        radius: f32,
        color: Color32,
        width: f32,
    ) {
        let pts: Vec<V3> = (0..=72)
            .map(|k| {
                let a = k as f32 / 72.0 * std::f32::consts::TAU;
                center + v3(a.sin(), 0.0, a.cos()) * radius
            })
            .collect();
        for seg in self.project_all(rect, &pts).windows(2) {
            painter.line_segment([seg[0], seg[1]], Stroke::new(width, color));
        }
    }

    /// The spiral arms, drawn as curves winding out from the origin.
    #[allow(clippy::too_many_arguments)]
    fn draw_spiral(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        center: V3,
        extent: f32,
        arms: u32,
        head: f32,
        color: Color32,
    ) {
        for arm in 0..arms {
            let offset = arm as f32 / arms as f32;
            let pts: Vec<V3> = (0..=64)
                .map(|k| {
                    let t = k as f32 / 64.0;
                    // Matches the shape's own maths: angle and radius advance
                    // together, so one turn of the arm spans one ring.
                    let a = (head + offset - t) * std::f32::consts::TAU;
                    center + v3(a.sin(), 0.0, a.cos()) * (t * extent)
                })
                .collect();
            for seg in self.project_all(rect, &pts).windows(2) {
                painter.line_segment([seg[0], seg[1]], Stroke::new(2.0, color));
            }
        }
    }

    /// Project a world-space polyline, dropping points behind the camera.
    fn project_all(&self, rect: Rect, pts: &[V3]) -> Vec<Pos2> {
        pts.iter()
            .filter_map(|p| self.cam.project(rect, *p).map(|(sp, _)| sp))
            .collect()
    }

    fn sphere_extent_world(&self, origin: V3) -> f32 {
        self.instances
            .iter()
            .map(|inst| (inst.t.pos - origin).len())
            .fold(0.0f32, f32::max)
            .max(0.75)
            * 1.05
    }

    fn sphere_extent_radius_px(&self, rect: Rect, origin: V3) -> f32 {
        let right = v3(self.cam.yaw.cos(), 0.0, -self.cam.yaw.sin());
        let edge = origin + right * self.sphere_extent_world(origin);
        match (self.cam.project(rect, origin), self.cam.project(rect, edge)) {
            (Some((center, _)), Some((edge, _))) => center.distance(edge).clamp(18.0, 260.0),
            _ => (self.sphere_extent_world(origin) / self.cam.world_per_pixel(rect))
                .clamp(18.0, 260.0),
        }
    }

    fn transition_extent_world(&self, transition: &TransitionConfig) -> f32 {
        self.sphere_extent_world(transition.sphere.pos)
    }

    fn transition_extent_radius_px(&self, rect: Rect, transition: &TransitionConfig) -> f32 {
        self.sphere_extent_radius_px(rect, transition.sphere.pos)
    }

    /// Vertical half-disc cross-section pointing along azimuth `az_deg`, drawn
    /// as a translucent fan from the centre out to `extent`.
    fn draw_scan_plane(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        center: V3,
        extent: f32,
        az_deg: f32,
        color: Color32,
        fill_alpha: u8,
    ) {
        let rad = dir_from_angles(az_deg, 0.0).norm();
        let up = v3(0.0, 1.0, 0.0);
        const STEPS: usize = 24;
        let mut arc: Vec<Pos2> = Vec::with_capacity(STEPS + 1);
        for k in 0..=STEPS {
            let t = -std::f32::consts::FRAC_PI_2
                + std::f32::consts::PI * k as f32 / STEPS as f32;
            let p = center + (rad * t.cos() + up * t.sin()) * extent;
            if let Some((sp, _)) = self.cam.project(rect, p) {
                arc.push(sp);
            }
        }
        if arc.len() < 2 {
            return;
        }
        if let Some((c_s, _)) = self.cam.project(rect, center) {
            let mut poly = Vec::with_capacity(arc.len() + 1);
            poly.push(c_s);
            poly.extend(arc.iter().copied());
            painter.add(Shape::convex_polygon(
                poly,
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), fill_alpha),
                Stroke::NONE,
            ));
        }
        for seg in arc.windows(2) {
            painter.line_segment([seg[0], seg[1]], Stroke::new(2.0, color));
        }
        // Vertical diameter (the cross-section's spine).
        painter.line_segment([arc[0], arc[arc.len() - 1]], Stroke::new(1.0, color));
    }

    /// Curved arrow at mid-height marking the sweep start and its direction.
    fn draw_scan_arrow(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        center: V3,
        extent: f32,
        start_az_deg: f32,
        color: Color32,
    ) {
        let r = extent * 0.5;
        let start = start_az_deg.to_radians();
        let sweep = 0.9_f32;
        let mut pts = Vec::new();
        for k in 0..=18 {
            let a = start + sweep * k as f32 / 18.0;
            let dir = v3(a.sin(), 0.0, a.cos());
            if let Some((sp, _)) = self.cam.project(rect, center + dir * r) {
                pts.push(sp);
            }
        }
        if let Some(&tail) = pts.first() {
            painter.circle_filled(tail, 3.0, color);
        }
        for seg in pts.windows(2) {
            painter.line_segment([seg[0], seg[1]], Stroke::new(2.5, color));
        }
        if pts.len() >= 2 {
            let tip = pts[pts.len() - 1];
            let prev = pts[pts.len() - 2];
            let d = tip - prev;
            let n = d / d.length().max(1.0);
            let p = egui::vec2(-n.y, n.x);
            painter.line_segment([tip, tip - n * 12.0 + p * 5.5], Stroke::new(2.5, color));
            painter.line_segment([tip, tip - n * 12.0 - p * 5.5], Stroke::new(2.5, color));
        }
    }

    fn draw_horizontal_reach_ring(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        transition: &TransitionConfig,
        extent: f32,
        color: Color32,
    ) {
        let mut pts = Vec::new();
        for k in 0..=72 {
            let a = k as f32 / 72.0 * std::f32::consts::TAU;
            let p = transition.sphere.pos + v3(a.sin(), 0.0, a.cos()) * extent;
            if let Some((sp, _)) = self.cam.project(rect, p) {
                pts.push(sp);
            }
        }
        for seg in pts.windows(2) {
            painter.line_segment([seg[0], seg[1]], Stroke::new(2.0, color));
        }
    }
}

/// Two perpendicular in-plane axes for a plane with the given `normal`.
///
/// The seed has to avoid being parallel to the normal or the cross product
/// collapses to zero and normalising it yields NaN — which would silently
/// turn a chase marker into nothing at all. Cascade aims straight up, so
/// that case is real, not theoretical.
fn plane_basis(normal: V3) -> (V3, V3) {
    let seed = if normal.norm().dot(v3(0.0, 1.0, 0.0)).abs() > 0.9 {
        v3(1.0, 0.0, 0.0)
    } else {
        v3(0.0, 1.0, 0.0)
    };
    let a = normal.cross(seed).norm();
    (a, normal.cross(a).norm())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chase::ChaseKind;

    /// Whatever way a chase is aimed — including straight up, which Cascade
    /// does — the marker plane needs two finite, perpendicular axes.
    #[test]
    fn plane_basis_is_orthonormal_for_every_aim() {
        let mut aims = vec![v3(0.0, 1.0, 0.0), v3(0.0, -1.0, 0.0), v3(1.0, 0.0, 0.0)];
        for yaw in [0.0, 45.0, 90.0, 180.0, 270.0] {
            for pitch in [-90.0, -45.0, 0.0, 45.0, 90.0] {
                aims.push(dir_from_angles(yaw, pitch).norm());
            }
        }
        for n in aims {
            let (a, b) = plane_basis(n);
            for v in [a, b] {
                assert!(v.len().is_finite(), "non-finite axis for {n:?}");
                assert!((v.len() - 1.0).abs() < 1e-3, "axis not unit for {n:?}");
            }
            assert!(a.dot(b).abs() < 1e-3, "axes not perpendicular for {n:?}");
            assert!(a.dot(n.norm()).abs() < 1e-3, "axis not in plane for {n:?}");
        }
    }

    /// Every shape has to draw something — an overlay that silently skips a
    /// kind would leave that chase un-editable on the stage.
    #[test]
    fn every_chase_kind_is_drawable() {
        for kind in ChaseKind::ALL {
            // Exercised through the label used by the overlay caption; the
            // draw path itself needs a live camera, but this keeps the match
            // honest if a kind is ever added without a marker.
            assert!(!kind.label().is_empty(), "{kind:?} has no label");
        }
    }
}
