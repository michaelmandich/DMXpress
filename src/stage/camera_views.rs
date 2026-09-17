//! Quick views, framing, camera glides, the previous-camera history and the
//! saved camera bookmarks (cameras.json) for the stage.
//!
//! Everything here moves the camera and nothing else: no light, tower or
//! truss is touched, so none of it pushes a stage undo step. The Inspector's
//! Stage tab drives it; `previous_camera` is the escape hatch.

use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::path::Path;

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::gizmo::Drag;
use super::math::{v3, Camera, CameraSnapshot, V3};
use super::settings::Settings;
use super::view::StageView;

pub(crate) const CAMERAS_FILE: &str = "cameras.json";

/// How many poses "previous camera" can walk back through.
pub(crate) const CAM_HISTORY: usize = 10;

/// How long a glide takes, in seconds.
const TWEEN_SECS: f32 = 0.35;

/// The orbit pitch clamp the drag handler enforces (input.rs).
const PITCH_MIN: f32 = -0.15;
const PITCH_MAX: f32 = 1.5;
/// The orbit distance clamp the scroll handler enforces (input.rs).
const DIST_MIN: f32 = 3.0;
const DIST_MAX: f32 = 80.0;

/// A named camera pose that travels with the show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraBookmark {
    pub name: String,
    pub(crate) camera: CameraSnapshot,
    #[serde(default)]
    pub fly_mode: bool,
    /// 1..=9, unique across the list.
    #[serde(default)]
    pub hotkey: Option<u8>,
    #[serde(default)]
    pub color: Option<[u8; 3]>,
}

/// The bookmarks in `cameras.json`; empty when absent or unreadable.
pub fn load_cameras() -> Vec<CameraBookmark> {
    load_cameras_from(Path::new(CAMERAS_FILE))
}

/// Write the bookmarks to `cameras.json`; errors are swallowed.
pub fn save_cameras(list: &[CameraBookmark]) {
    save_cameras_to(Path::new(CAMERAS_FILE), list)
}

pub(crate) fn load_cameras_from(path: &Path) -> Vec<CameraBookmark> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub(crate) fn save_cameras_to(path: &Path, list: &[CameraBookmark]) {
    if let Ok(text) = serde_json::to_string_pretty(list) {
        let _ = std::fs::write(path, text);
    }
}

/// Give `idx` the digit `key`, taking it off whoever held it. `None` clears
/// the entry's own key. Out-of-range indices change nothing.
pub(crate) fn assign_hotkey(list: &mut [CameraBookmark], idx: usize, key: Option<u8>) {
    if idx >= list.len() {
        return;
    }
    if let Some(k) = key {
        for b in list.iter_mut() {
            if b.hotkey == Some(k) {
                b.hotkey = None;
            }
        }
    }
    list[idx].hotkey = key;
}

/// Which bookmark answers to digit `key`, if any.
pub(crate) fn bookmark_by_hotkey(list: &[CameraBookmark], key: u8) -> Option<usize> {
    list.iter().position(|b| b.hotkey == Some(key))
}

/// `base` when it is free, else "base 2", "base 3", … — the first name the
/// list does not already carry.
pub(crate) fn unique_name(list: &[CameraBookmark], base: &str) -> String {
    let base = if base.trim().is_empty() { "Camera" } else { base.trim() };
    if !list.iter().any(|b| b.name == base) {
        return base.to_owned();
    }
    for n in 2..1000 {
        let candidate = format!("{base} {n}");
        if !list.iter().any(|b| b.name == candidate) {
            return candidate;
        }
    }
    base.to_owned()
}

/// One of the canonical angles the Stage tab's pad grid offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuickView {
    Iso,
    Front,
    Back,
    Left,
    Right,
    Top,
    Foh,
}

impl QuickView {
    pub const ALL: [QuickView; 7] = [
        QuickView::Iso,
        QuickView::Front,
        QuickView::Back,
        QuickView::Left,
        QuickView::Right,
        QuickView::Top,
        QuickView::Foh,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Iso => "Iso",
            Self::Front => "Front",
            Self::Back => "Back",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Top => "Top",
            Self::Foh => "FOH",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Iso => "Iso: the three-quarter view the app opens on. Shift-click keeps the current zoom.",
            Self::Front => "Front: from the audience, looking at the stage. Shift-click keeps the current zoom.",
            Self::Back => "Back: from upstage, looking out at the room. Shift-click keeps the current zoom.",
            Self::Left => "Left: from stage right, looking across the rig. Shift-click keeps the current zoom.",
            Self::Right => "Right: from stage left, looking across the rig. Shift-click keeps the current zoom.",
            Self::Top => "Top: straight down, the plan view for laying lights out. Shift-click keeps the current zoom.",
            Self::Foh => "FOH: eye height out front, roughly what the operator sees. Shift-click keeps the current zoom.",
        }
    }

    /// (yaw, pitch) in radians. FOH's pitch is recomputed from the stage
    /// height in [`StageView::quick_view`] unless the zoom is kept.
    pub fn yaw_pitch(self) -> (f32, f32) {
        match self {
            Self::Iso => (0.5, 0.42),
            Self::Front => (0.0, 0.25),
            Self::Back => (PI, 0.25),
            Self::Left => (-FRAC_PI_2, 0.25),
            Self::Right => (FRAC_PI_2, 0.25),
            // Never exactly π/2: `Camera::basis` degenerates there.
            Self::Top => (0.0, PITCH_MAX),
            Self::Foh => (0.0, 0.0),
        }
    }
}

/// A camera glide in progress. `start` is filled on its first step, so
/// `go_to` needs no clock.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CamTween {
    pub from: CameraSnapshot,
    pub to: CameraSnapshot,
    pub start: Option<f64>,
    pub dur: f32,
}

/// Where the camera sits at `0 ≤ s ≤ 1` of a glide (pure). Yaw takes the
/// shortest arc, distance interpolates geometrically so a long dolly does
/// not start with a lurch, and `s = 1` returns `b` exactly.
pub(crate) fn tween_snapshot(a: &CameraSnapshot, b: &CameraSnapshot, s: f32) -> CameraSnapshot {
    if s >= 1.0 {
        return b.clone();
    }
    let s = s.max(0.0);
    let mut d = (b.yaw - a.yaw).rem_euclid(TAU);
    if d > PI {
        d -= TAU;
    }
    CameraSnapshot {
        yaw: a.yaw + d * s,
        pitch: a.pitch + (b.pitch - a.pitch) * s,
        dist: (a.dist.max(0.01).ln() + (b.dist.max(0.01).ln() - a.dist.max(0.01).ln()) * s).exp(),
        target: a.target + (b.target - a.target) * s,
        fov_y: a.fov_y + (b.fov_y - a.fov_y) * s,
    }
}

/// The orbit target and distance that fit every point on screen (pure).
/// `None` when there is nothing to fit.
pub(crate) fn frame_for(points: &[V3], fov_y: f32, aspect: f32) -> Option<(V3, f32)> {
    let first = *points.first()?;
    let (mut lo, mut hi) = (first, first);
    for p in points {
        lo = v3(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = v3(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    let centre = (lo + hi) * 0.5;
    let r = points
        .iter()
        .map(|p| (*p - centre).len())
        .fold(0.0_f32, f32::max)
        .max(1.0);
    let half = (fov_y * 0.5).clamp(0.05, 1.5);
    // The horizontal half-angle, so a narrow panel pulls the camera back.
    let hx = (half.tan() * aspect.max(0.1)).atan();
    let eff = half.min(hx).max(0.05);
    let dist = (r / eff.sin() * 1.12).clamp(DIST_MIN, DIST_MAX);
    Some((centre, dist))
}

impl StageView {
    /// The aspect of the rect the stage last drew into; 1.6 before the first
    /// frame, when the rect is still the placeholder.
    pub(crate) fn aspect(&self) -> f32 {
        if self.last_rect.height() > 1.0 {
            (self.last_rect.width() / self.last_rect.height()).clamp(0.1, 10.0)
        } else {
            1.6
        }
    }

    /// Every point the rig itself occupies: each light, each tower's base,
    /// top and slots, each truss's centre and slots. Empty on a bare stage.
    pub(crate) fn rig_extent_points(&self) -> Vec<V3> {
        let mut pts: Vec<V3> = self.instances.iter().map(|inst| inst.t.pos).collect();
        for tw in &self.towers {
            pts.push(tw.pos);
            pts.push(tw.pos + v3(0.0, tw.height, 0.0));
            for s in 0..super::layout::TOWER_SLOTS {
                pts.push(tw.slot_pos(s));
            }
        }
        for tr in &self.trusses {
            pts.push(tr.pos);
            for s in 0..tr.total_slots() {
                pts.push(tr.slot_pos(s));
            }
        }
        pts
    }

    /// The rig plus the eight stage-box corners, so an empty rig still
    /// frames the stage rather than nothing at all.
    pub(crate) fn rig_points(&self, set: &Settings) -> Vec<V3> {
        let mut pts = self.rig_extent_points();
        pts.extend(stage_box_corners(set));
        pts
    }

    /// What "the selection" means to the framing buttons: the selected
    /// lights, else the picked tower, else the picked truss, else the stage
    /// box, else nothing.
    pub(crate) fn selection_points(&self, set: &Settings) -> Option<Vec<V3>> {
        if !self.selection.is_empty() {
            let pts: Vec<V3> = self
                .selection
                .iter()
                .filter_map(|&i| self.instances.get(i).map(|inst| inst.t.pos))
                .collect();
            if !pts.is_empty() {
                return Some(pts);
            }
        }
        if let Some(tw) = self.sel_tower.and_then(|i| self.towers.get(i)) {
            let mut pts = vec![tw.pos, tw.pos + v3(0.0, tw.height, 0.0)];
            for s in 0..super::layout::TOWER_SLOTS {
                pts.push(tw.slot_pos(s));
            }
            return Some(pts);
        }
        if let Some(tr) = self.sel_truss.and_then(|i| self.trusses.get(i)) {
            let mut pts = vec![tr.pos];
            for s in 0..tr.total_slots() {
                pts.push(tr.slot_pos(s));
            }
            return Some(pts);
        }
        if self.sel_stage {
            return Some(stage_box_corners(set).to_vec());
        }
        None
    }

    /// Remember where the camera is, so "previous camera" can come back.
    /// Repeats and overflow past [`CAM_HISTORY`] are dropped.
    pub(crate) fn remember_camera(&mut self) {
        let now = self.cam.snapshot();
        if self.cam_history.last() == Some(&now) {
            return;
        }
        self.cam_history.push(now);
        if self.cam_history.len() > CAM_HISTORY {
            let drop = self.cam_history.len() - CAM_HISTORY;
            self.cam_history.drain(0..drop);
        }
    }

    /// Move the camera to `to`, gliding when `animate`. The pose it leaves
    /// goes on the history first.
    pub(crate) fn go_to(&mut self, to: CameraSnapshot, animate: bool) {
        if !self.camera_is(&to) {
            self.remember_camera();
        }
        if animate {
            self.cam_tween = Some(CamTween {
                from: self.cam.snapshot(),
                to,
                start: None,
                dur: TWEEN_SECS,
            });
        } else {
            self.cam.apply_snapshot(&to);
            self.cam_tween = None;
        }
    }

    /// Put the camera on a canonical angle. Unless `keep_zoom`, the whole
    /// rig is re-framed at the same time.
    pub(crate) fn quick_view(
        &mut self,
        q: QuickView,
        set: &Settings,
        keep_zoom: bool,
        animate: bool,
    ) {
        let mut snap = self.cam.snapshot();
        let (yaw, pitch) = q.yaw_pitch();
        snap.yaw = yaw;
        snap.pitch = pitch;
        if !keep_zoom {
            if let Some((centre, dist)) = frame_for(&self.rig_points(set), snap.fov_y, self.aspect())
            {
                snap.target = centre;
                snap.dist = dist;
            }
            if q == QuickView::Foh {
                // Out front at head height, looking at the deck.
                snap.target = v3(0.0, set.stage_h + 1.0, 0.0);
                snap.dist = 12.0;
                snap.pitch = ((1.7 - snap.target.y) / snap.dist)
                    .clamp(-1.0, 1.0)
                    .asin()
                    .clamp(PITCH_MIN, PITCH_MAX);
            }
        }
        snap.dist = snap.dist.clamp(DIST_MIN, DIST_MAX);
        self.go_to(snap, animate);
    }

    /// Fit whatever is selected on screen, keeping the current angle.
    /// False when nothing is selected.
    pub(crate) fn frame_selection(&mut self, set: &Settings, animate: bool) -> bool {
        let Some(pts) = self.selection_points(set) else {
            return false;
        };
        let mut snap = self.cam.snapshot();
        let Some((centre, dist)) = frame_for(&pts, snap.fov_y, self.aspect()) else {
            return false;
        };
        snap.target = centre;
        snap.dist = dist;
        self.go_to(snap, animate);
        true
    }

    /// Fit the whole rig and the stage box on screen.
    pub(crate) fn frame_all(&mut self, set: &Settings, animate: bool) {
        let mut snap = self.cam.snapshot();
        if let Some((centre, dist)) = frame_for(&self.rig_points(set), snap.fov_y, self.aspect()) {
            snap.target = centre;
            snap.dist = dist;
            self.go_to(snap, animate);
        }
    }

    /// Re-centre on the selection without changing the zoom or the angle.
    /// False when there is nothing to look at.
    pub(crate) fn look_at_selection(&mut self, animate: bool) -> bool {
        let target = self
            .selection_centroid()
            .or_else(|| self.sel_tower.map(super::layout::ElementRef::Tower).and_then(|r| self.element_centre(r)))
            .or_else(|| self.sel_truss.map(super::layout::ElementRef::Truss).and_then(|r| self.element_centre(r)));
        let Some(target) = target else {
            return false;
        };
        let mut snap = self.cam.snapshot();
        snap.target = target;
        self.go_to(snap, animate);
        true
    }

    /// Back to the camera the app opens on.
    pub(crate) fn reset_camera(&mut self, animate: bool) {
        self.go_to(Camera::default().snapshot(), animate);
    }

    /// Step back through the history without re-recording, so repeated
    /// presses walk backwards. False once the history is empty.
    pub(crate) fn previous_camera(&mut self, animate: bool) -> bool {
        let Some(prev) = self.cam_history.pop() else {
            return false;
        };
        if animate {
            self.cam_tween = Some(CamTween {
                from: self.cam.snapshot(),
                to: prev,
                start: None,
                dur: TWEEN_SECS,
            });
        } else {
            self.cam.apply_snapshot(&prev);
            self.cam_tween = None;
        }
        true
    }

    /// Turn the camera by hand (the Inspector's nudge row). In fly mode the
    /// camera turns in place instead of orbiting.
    pub(crate) fn orbit_by(&mut self, dyaw: f32, dpitch: f32) {
        self.remember_camera();
        self.cam_tween = None;
        if self.fly_mode {
            self.cam.free_look(dyaw, dpitch);
        } else {
            self.cam.yaw += dyaw;
            self.cam.pitch = (self.cam.pitch + dpitch).clamp(PITCH_MIN, PITCH_MAX);
        }
    }

    /// Dolly in (`factor < 1`) or out, inside the orbit clamps.
    pub(crate) fn zoom_by(&mut self, factor: f32) {
        self.remember_camera();
        self.cam_tween = None;
        self.cam.dist = (self.cam.dist * factor).clamp(DIST_MIN, DIST_MAX);
    }

    /// Whether the camera is (near enough) at `s`: a tenth of a degree of
    /// angle, five centimetres of distance and target.
    pub(crate) fn camera_is(&self, s: &CameraSnapshot) -> bool {
        let dy = (self.cam.yaw - s.yaw).rem_euclid(TAU);
        let dy = dy.min(TAU - dy);
        dy < 0.01
            && (self.cam.pitch - s.pitch).abs() < 0.01
            && (self.cam.dist - s.dist).abs() < 0.05
            && (self.cam.target.x - s.target.x).abs() < 0.05
            && (self.cam.target.y - s.target.y).abs() < 0.05
            && (self.cam.target.z - s.target.z).abs() < 0.05
    }

    /// Whether the camera is looking down one of the quick-view angles, so
    /// its pad can light up. Distance and target are ignored — a quick view
    /// is an angle, not a zoom.
    pub(crate) fn camera_is_quick(&self, q: QuickView) -> bool {
        let (yaw, pitch) = q.yaw_pitch();
        let dy = (self.cam.yaw - yaw).rem_euclid(TAU);
        let dy = dy.min(TAU - dy);
        if dy >= 0.02 {
            return false;
        }
        match q {
            // FOH's tilt depends on the stage height, so it owns the band
            // between level and the bottom of the orbit clamp.
            QuickView::Foh => self.cam.pitch <= 0.02 && self.cam.pitch >= PITCH_MIN - 0.01,
            _ => (self.cam.pitch - pitch).abs() < 0.02,
        }
    }

    /// Whether a pointer gesture is in progress (tracking holds off during
    /// a marquee, which changes the selection every frame).
    pub(crate) fn is_dragging(&self) -> bool {
        !matches!(self.drag, Drag::None)
    }

    /// This camera as a bookmark, ready to be named and saved.
    pub(crate) fn bookmark(&self, name: String) -> CameraBookmark {
        CameraBookmark {
            name,
            camera: self.cam.snapshot(),
            fly_mode: self.fly_mode,
            hotkey: None,
            color: None,
        }
    }

    /// Go to a saved camera, restoring the navigation mode it was saved in.
    pub(crate) fn recall_bookmark(&mut self, b: &CameraBookmark, animate: bool) {
        self.fly_mode = b.fly_mode;
        self.go_to(b.camera.clone(), animate);
    }

    /// Advance a glide by one frame, or cancel it the moment the operator
    /// touches the camera themselves. Called once per frame from
    /// `StageView::ui`, whether or not a glide is running, because the same
    /// test tells the camera tour to stop.
    pub(crate) fn step_cam_tween(&mut self, ui: &egui::Ui, resp: &egui::Response) {
        if self.camera_input_active(ui, resp) {
            self.cam_tween = None;
            self.user_moved = true;
            return;
        }
        let Some(mut t) = self.cam_tween.take() else {
            return;
        };
        let now = ui.input(|i| i.time);
        let start = *t.start.get_or_insert(now);
        let x = (((now - start) / t.dur.max(0.01) as f64).clamp(0.0, 1.0)) as f32;
        // Smoothstep: ease in and out, so the move reads as a camera move
        // and not as a cut.
        let s = x * x * (3.0 - 2.0 * x);
        let snap = tween_snapshot(&t.from, &t.to, s);
        self.cam.apply_snapshot(&snap);
        if x < 1.0 {
            self.cam_tween = Some(t);
        }
        ui.ctx().request_repaint();
    }

    /// Is the operator driving the camera right now?
    fn camera_input_active(&self, ui: &egui::Ui, resp: &egui::Response) -> bool {
        use egui::{Key, PointerButton};
        let mods = ui.input(|i| i.modifiers);
        if resp.dragged_by(PointerButton::Secondary)
            || resp.dragged_by(PointerButton::Middle)
            || (mods.alt && resp.dragged_by(PointerButton::Primary))
            || matches!(self.drag, Drag::PanCam)
        {
            return true;
        }
        if resp.hovered() && ui.input(|i| i.raw_scroll_delta.y != 0.0) {
            return true;
        }
        if self.fly_mode && resp.hovered() {
            const FLY_KEYS: [Key; 9] = [
                Key::W,
                Key::A,
                Key::S,
                Key::D,
                Key::Space,
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::ArrowUp,
                Key::ArrowDown,
            ];
            if ui.input(|i| FLY_KEYS.iter().any(|k| i.key_down(*k))) {
                return true;
            }
        }
        false
    }
}

/// The eight corners of the stage riser.
fn stage_box_corners(set: &Settings) -> [V3; 8] {
    let (w, d, h) = (set.stage_half_w, set.stage_half_d, set.stage_h);
    [
        v3(-w, 0.0, -d),
        v3(w, 0.0, -d),
        v3(w, 0.0, d),
        v3(-w, 0.0, d),
        v3(-w, h, -d),
        v3(w, h, -d),
        v3(w, h, d),
        v3(-w, h, d),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::layout::{Instance, LightTransform, Tower, Truss};
    use eframe::egui::Rect;

    fn snap(yaw: f32, pitch: f32, dist: f32) -> CameraSnapshot {
        CameraSnapshot { yaw, pitch, dist, target: v3(0.0, 1.5, 0.0), fov_y: 55.0_f32.to_radians() }
    }

    fn light(pos: V3) -> Instance {
        Instance {
            fixture: 0,
            t: LightTransform { pos, yaw_deg: 0.0, pitch_deg: -90.0, roll_deg: 0.0, scale: 1.0 },
            opacity: 1.0,
            mount: None,
            truss_mount: None,
        }
    }

    #[test]
    fn quick_views_stay_inside_the_orbit_clamps() {
        let set = Settings::default();
        for q in QuickView::ALL {
            let mut v = StageView::new();
            v.quick_view(q, &set, false, false);
            assert!(
                (PITCH_MIN..=PITCH_MAX).contains(&v.cam.pitch),
                "{}: pitch {} outside the orbit clamp",
                q.label(),
                v.cam.pitch
            );
            assert!(
                (DIST_MIN..=DIST_MAX).contains(&v.cam.dist),
                "{}: dist {}",
                q.label(),
                v.cam.dist
            );
            assert!(v.camera_is_quick(q), "{} does not read back as itself", q.label());
            let (right, _, fwd) = v.cam.basis();
            assert!(right.len() > 0.5 && right.len().is_finite(), "{}: degenerate basis", q.label());
            assert!(fwd.len() > 0.5, "{}: degenerate forward", q.label());
            // Keeping the zoom only turns the camera.
            let mut v = StageView::new();
            let before = v.cam.snapshot();
            v.quick_view(q, &set, true, false);
            assert_eq!(v.cam.target, before.target, "{} moved the target", q.label());
            assert_eq!(v.cam.dist, before.dist, "{} changed the distance", q.label());
        }
    }

    #[test]
    fn quick_views_are_distinct_angles() {
        for (i, a) in QuickView::ALL.iter().enumerate() {
            for b in &QuickView::ALL[i + 1..] {
                let mut v = StageView::new();
                v.quick_view(*a, &Settings::default(), false, false);
                assert!(!v.camera_is_quick(*b), "{} reads as {}", a.label(), b.label());
                assert_ne!(a.label(), b.label());
            }
        }
    }

    #[test]
    fn frame_for_puts_every_point_on_screen() {
        let c = v3(1.0, 2.0, -3.0);
        let (hw, hh, hd) = (3.0, 1.0, 2.0);
        let pts: Vec<V3> = [-1.0_f32, 1.0]
            .iter()
            .flat_map(|sx| {
                [-1.0_f32, 1.0].iter().flat_map(move |sy| {
                    [-1.0_f32, 1.0]
                        .iter()
                        .map(move |sz| v3(c.x + sx * hw, c.y + sy * hh, c.z + sz * hd))
                })
            })
            .collect();
        let fov = 55.0_f32.to_radians();
        let mut last_dist = f32::INFINITY;
        for aspect in [0.8_f32, 1.6, 2.4] {
            let (centre, dist) = frame_for(&pts, fov, aspect).expect("eight points frame");
            assert!((centre.x - c.x).abs() < 1e-4 && (centre.z - c.z).abs() < 1e-4);
            let cam = Camera { yaw: 0.5, pitch: 0.42, dist, target: centre, fov_y: fov };
            let rect = Rect::from_min_size(
                eframe::egui::Pos2::ZERO,
                eframe::egui::vec2(1000.0 * aspect, 1000.0),
            );
            let safe = rect.shrink2(eframe::egui::vec2(rect.width() * 0.03, rect.height() * 0.03));
            for p in &pts {
                let (sp, _) = cam.project(rect, *p).expect("in front of the camera");
                assert!(safe.contains(sp), "aspect {aspect}: {sp:?} outside {safe:?}");
            }
            // A narrower panel has to pull further back.
            assert!(dist <= last_dist + 1e-3, "aspect {aspect} did not get closer");
            last_dist = dist;
        }
    }

    #[test]
    fn frame_for_of_nothing_is_none_and_one_point_is_the_minimum() {
        assert!(frame_for(&[], 55.0_f32.to_radians(), 1.6).is_none());
        let (c, d) = frame_for(&[v3(2.0, 1.0, 0.0)], 55.0_f32.to_radians(), 1.6).unwrap();
        assert_eq!(c, v3(2.0, 1.0, 0.0));
        assert_eq!(d, DIST_MIN);
    }

    #[test]
    fn rig_points_include_the_stage_box_when_the_rig_is_empty() {
        let v = StageView::new();
        assert!(v.rig_extent_points().is_empty());
        assert_eq!(v.rig_points(&Settings::default()).len(), 8);
    }

    #[test]
    fn selection_points_prefer_lights_then_tower_then_truss_then_stage() {
        let set = Settings::default();
        let mut v = StageView::new();
        v.layout_path = "does-not-exist.json".into();
        v.instances.push(light(v3(1.0, 2.0, 3.0)));
        v.instances.push(light(v3(-1.0, 2.0, -3.0)));
        v.instances.push(light(v3(0.0, 5.0, 0.0)));
        v.selection.insert(0);
        v.selection.insert(1);
        let pts = v.selection_points(&set).expect("lights");
        assert_eq!(pts.len(), 2);
        v.selection.clear();
        v.towers.push(Tower::default());
        v.sel_tower = Some(0);
        assert_eq!(v.selection_points(&set).unwrap().len(), 2 + super::super::layout::TOWER_SLOTS);
        v.sel_tower = None;
        v.trusses.push(Truss::straight());
        v.sel_truss = Some(0);
        let want = 1 + v.trusses[0].total_slots();
        assert_eq!(v.selection_points(&set).unwrap().len(), want);
        v.sel_truss = None;
        v.sel_stage = true;
        assert_eq!(v.selection_points(&set).unwrap().len(), 8);
        v.sel_stage = false;
        assert!(v.selection_points(&set).is_none());
        assert!(!v.frame_selection(&set, false));
    }

    #[test]
    fn tween_takes_the_shortest_yaw_arc_and_ends_exactly() {
        let a = snap(3.0, 0.2, 4.0);
        let b = snap(-3.0, 0.5, 64.0);
        let mid = tween_snapshot(&a, &b, 0.5);
        assert!(mid.yaw.abs() > 3.0, "the arc went the long way: {}", mid.yaw);
        assert!((mid.dist - 16.0).abs() < 1e-3, "{}", mid.dist);
        assert!((mid.pitch - 0.35).abs() < 1e-5, "{}", mid.pitch);
        assert_eq!(tween_snapshot(&a, &b, 1.0), b);
        let start = tween_snapshot(&a, &b, 0.0);
        assert_eq!(start.yaw, a.yaw);
        assert!((start.dist - a.dist).abs() < 1e-4);
    }

    #[test]
    fn go_to_without_animation_applies_and_records_history() {
        let mut v = StageView::new();
        let poses: Vec<CameraSnapshot> =
            (0..12).map(|i| snap(0.1 * i as f32, 0.3, 10.0 + i as f32)).collect();
        for p in &poses {
            v.go_to(p.clone(), false);
        }
        assert_eq!(v.cam.snapshot(), poses[11]);
        assert_eq!(v.cam_history.len(), CAM_HISTORY);
        assert!(v.previous_camera(false));
        assert_eq!(v.cam.snapshot(), poses[10]);
        for _ in 0..9 {
            assert!(v.previous_camera(false));
        }
        assert!(!v.previous_camera(false));
    }

    #[test]
    fn go_to_with_animation_starts_a_tween() {
        let mut v = StageView::new();
        let before = v.cam.snapshot();
        let to = snap(2.0, 0.6, 30.0);
        v.go_to(to.clone(), true);
        assert_eq!(v.cam.snapshot(), before, "the camera jumped instead of gliding");
        let t = v.cam_tween.as_ref().expect("a glide");
        assert_eq!(t.to, to);
        assert!(t.start.is_none());
        assert_eq!(tween_snapshot(&t.from, &t.to, 1.0), to);
    }

    #[test]
    fn bookmark_hotkeys_are_unique() {
        let mut v: Vec<CameraBookmark> = ["a", "b", "c"]
            .iter()
            .map(|n| CameraBookmark {
                name: (*n).to_owned(),
                camera: snap(0.0, 0.3, 10.0),
                fly_mode: false,
                hotkey: None,
                color: None,
            })
            .collect();
        assign_hotkey(&mut v, 0, Some(3));
        assert_eq!(bookmark_by_hotkey(&v, 3), Some(0));
        assign_hotkey(&mut v, 2, Some(3));
        assert_eq!(v[0].hotkey, None);
        assert_eq!(v[2].hotkey, Some(3));
        assert_eq!(bookmark_by_hotkey(&v, 3), Some(2));
        assign_hotkey(&mut v, 2, None);
        assert_eq!(bookmark_by_hotkey(&v, 3), None);
        assign_hotkey(&mut v, 99, Some(1));
        assert_eq!(bookmark_by_hotkey(&v, 1), None);
    }

    #[test]
    fn unique_name_counts_up() {
        let named = |names: &[&str]| -> Vec<CameraBookmark> {
            names
                .iter()
                .map(|n| CameraBookmark {
                    name: (*n).to_owned(),
                    camera: snap(0.0, 0.3, 10.0),
                    fly_mode: false,
                    hotkey: None,
                    color: None,
                })
                .collect()
        };
        assert_eq!(unique_name(&named(&["Camera", "Camera 2"]), "Camera"), "Camera 3");
        assert_eq!(unique_name(&[], "Camera"), "Camera");
        assert_eq!(unique_name(&named(&["Wide"]), "Tight"), "Tight");
        assert_eq!(unique_name(&named(&["Camera"]), "  "), "Camera 2");
    }

    #[test]
    fn bookmarks_round_trip_through_json() {
        let dir = std::env::temp_dir().join("dmxpress-cameras-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("cameras.json");
        let list = vec![
            CameraBookmark {
                name: "Front of house".into(),
                camera: snap(0.2, 0.4, 14.0),
                fly_mode: true,
                hotkey: Some(1),
                color: Some([240, 120, 40]),
            },
            CameraBookmark {
                name: "Top plan".into(),
                camera: snap(0.0, 1.5, 22.0),
                fly_mode: false,
                hotkey: None,
                color: None,
            },
        ];
        save_cameras_to(&path, &list);
        assert_eq!(load_cameras_from(&path), list);
        // A record written before fly / hotkey / colour existed.
        let legacy = r#"[{"name":"x","camera":{"yaw":0.1,"pitch":0.2,"dist":9.0,
            "target":{"x":0.0,"y":1.0,"z":0.0},"fov_y":0.9}}]"#;
        std::fs::write(&path, legacy).unwrap();
        let back = load_cameras_from(&path);
        assert_eq!(back.len(), 1);
        assert!(!back[0].fly_mode && back[0].hotkey.is_none() && back[0].color.is_none());
        let _ = std::fs::remove_file(&path);
        assert!(load_cameras_from(&dir.join("nope.json")).is_empty());
    }

    #[test]
    fn camera_is_uses_tolerances() {
        let mut v = StageView::new();
        assert!(v.camera_is(&v.cam.snapshot()));
        let mut wrapped = v.cam.snapshot();
        wrapped.yaw += TAU;
        assert!(v.camera_is(&wrapped), "a full turn is the same angle");
        let here = v.cam.snapshot();
        v.orbit_by(0.3, 0.0);
        assert!(!v.camera_is(&here));
    }

    #[test]
    fn orbit_and_zoom_stay_clamped() {
        let mut v = StageView::new();
        v.orbit_by(0.0, 5.0);
        assert_eq!(v.cam.pitch, PITCH_MAX);
        v.orbit_by(0.0, -9.0);
        assert_eq!(v.cam.pitch, PITCH_MIN);
        v.zoom_by(100.0);
        assert_eq!(v.cam.dist, DIST_MAX);
        v.zoom_by(0.0001);
        assert_eq!(v.cam.dist, DIST_MIN);
    }

    #[test]
    fn recall_restores_the_navigation_mode() {
        let mut v = StageView::new();
        let mut b = v.bookmark("Wide".into());
        b.fly_mode = true;
        b.camera = snap(1.0, 0.5, 25.0);
        v.recall_bookmark(&b, false);
        assert!(v.fly_mode);
        assert!(v.camera_is(&b.camera));
    }
}
