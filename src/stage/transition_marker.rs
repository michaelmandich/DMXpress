//! Stage helpers for advanced transition patterns: fixture world positions,
//! sphere hit-testing/movement, and the visible marker overlay.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Shape, Stroke};

use super::math::{dir_from_angles, v3, V3};
use super::view::StageView;
use crate::chase::ChaseConfig;
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

        let half = chase.band_deg * 0.5;
        let head_az = chase.sphere.yaw_deg + chase.active_head.unwrap_or(0.0) * 360.0;
        self.draw_scan_plane(painter, rect, origin, extent, head_az - half, col, 8);
        self.draw_scan_plane(painter, rect, origin, extent, head_az + half, col, 8);
        self.draw_scan_plane(painter, rect, origin, extent, head_az, col, 42);
        self.draw_scan_arrow(painter, rect, origin, extent, head_az, col);

        painter.text(
            center + egui::vec2(0.0, radius + 8.0),
            Align2::CENTER_TOP,
            "Spherical chase",
            FontId::proportional(11.0),
            Color32::from_gray(220),
        );
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
