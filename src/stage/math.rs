//! Vector math, camera projection, and small geometry helpers.

use eframe::egui::{Pos2, Rect};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub(crate) struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub(crate) const fn v3(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

impl std::ops::Add for V3 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl std::ops::Sub for V3 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl std::ops::Mul<f32> for V3 {
    type Output = Self;
    fn mul(self, s: f32) -> Self {
        v3(self.x * s, self.y * s, self.z * s)
    }
}
impl V3 {
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: Self) -> Self {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn len(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn norm(self) -> Self {
        let l = self.len();
        if l > 1e-6 {
            self * (1.0 / l)
        } else {
            self
        }
    }
}

/// Unit direction from yaw (around +Y) and pitch (0 = horizon, -90 = down).
pub(crate) fn dir_from_angles(yaw_deg: f32, pitch_deg: f32) -> V3 {
    let (y, p) = (yaw_deg.to_radians(), pitch_deg.to_radians());
    v3(p.cos() * y.sin(), p.sin(), p.cos() * y.cos())
}

pub(crate) struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub target: V3,
    pub fov_y: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CameraSnapshot {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub target: V3,
    pub fov_y: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.5,
            pitch: 0.42,
            dist: 17.0,
            target: v3(0.0, 1.5, 0.0),
            fov_y: 55.0_f32.to_radians(),
        }
    }
}

impl Camera {
    fn orbit_offset(&self) -> V3 {
        v3(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        ) * self.dist
    }

    pub fn eye(&self) -> V3 {
        self.target + self.orbit_offset()
    }

    pub(crate) fn basis(&self) -> (V3, V3, V3) {
        let fwd = (self.target - self.eye()).norm();
        let right = fwd.cross(v3(0.0, 1.0, 0.0)).norm();
        let up = right.cross(fwd);
        (right, up, fwd)
    }

    /// World point -> (screen pos, camera depth). None if behind the camera.
    pub fn project(&self, rect: Rect, p: V3) -> Option<(Pos2, f32)> {
        let (right, up, fwd) = self.basis();
        let d = p - self.eye();
        let z = d.dot(fwd);
        if z < 0.05 {
            return None;
        }
        let scale = rect.height() * 0.5 / (self.fov_y * 0.5).tan();
        Some((
            Pos2::new(
                rect.center().x + d.dot(right) * scale / z,
                rect.center().y - d.dot(up) * scale / z,
            ),
            z,
        ))
    }

    /// Approximate world units per screen pixel at the target depth.
    pub fn world_per_pixel(&self, rect: Rect) -> f32 {
        self.dist * (self.fov_y * 0.5).tan() * 2.0 / rect.height()
    }

    /// Turn in place rather than orbiting the focal point.
    pub fn free_look(&mut self, yaw_delta: f32, pitch_delta: f32) {
        let eye = self.eye();
        self.yaw += yaw_delta;
        self.pitch = (self.pitch + pitch_delta).clamp(-1.5, 1.5);
        self.target = eye - self.orbit_offset();
    }

    /// Translate camera and focus together, preserving its view direction.
    pub fn translate(&mut self, delta: V3) {
        self.target = self.target + delta;
    }

    pub fn snapshot(&self) -> CameraSnapshot {
        CameraSnapshot {
            yaw: self.yaw,
            pitch: self.pitch,
            dist: self.dist,
            target: self.target,
            fov_y: self.fov_y,
        }
    }

    pub fn apply_snapshot(&mut self, view: &CameraSnapshot) {
        self.yaw = view.yaw;
        self.pitch = view.pitch;
        self.dist = view.dist.max(0.1);
        self.target = view.target;
        self.fov_y = view.fov_y;
    }
}

/// Shortest distance from point `p` to the screen segment `a`–`b`.
pub(crate) fn seg_dist(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-4)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// Rotate `v` around unit-ish `axis` by `ang` radians (Rodrigues).
pub(crate) fn rotate_about(v: V3, axis: V3, ang: f32) -> V3 {
    let a = axis.norm();
    let (s, c) = ang.sin_cos();
    v * c + a.cross(v) * s + a * (a.dot(v) * (1.0 - c))
}
