//! Fixture archetype classification and live render state derived from the
//! DMX buffer.

use eframe::egui::Color32;

use crate::showbuddy::{Fixture, Role};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Archetype {
    /// Static color wash (RGB par can).
    Par,
    /// Linear wash bar (1:5 prism body).
    Bar,
    /// Pan/tilt wash (wide beam).
    MovingPar,
    /// Pan/tilt spot/beam (narrow beam, gobos/prisms).
    Beam,
    /// Everything else (fog, 1-channel boxes...) — small cube only.
    Specialty,
}

pub(crate) fn classify(f: &Fixture) -> Archetype {
    let (mut has_pan, mut has_tilt, mut has_rgb, mut spotty) = (false, false, false, false);
    for ch in &f.channels {
        match ch.role() {
            Role::Pan | Role::PanFine => has_pan = true,
            Role::Tilt | Role::TiltFine => has_tilt = true,
            Role::Red | Role::Green | Role::Blue => has_rgb = true,
            _ => {}
        }
        let n = ch.name.to_lowercase();
        if n.contains("gobo") || n.contains("prism") {
            spotty = true;
        }
    }
    let name = f.display.to_lowercase();
    if has_pan && has_tilt {
        if spotty || name.contains("beam") || name.contains("spot") {
            Archetype::Beam
        } else {
            Archetype::MovingPar
        }
    } else if ["bar", "strip", "batten", "bank"].iter().any(|k| name.contains(k)) {
        Archetype::Bar
    } else if has_rgb || name.contains("par") || name.contains("wash") {
        Archetype::Par
    } else {
        // Anything with a dimmer or color control still emits light.
        let lighty = f.channels.iter().any(|ch| {
            matches!(ch.role(), Role::Dimmer | Role::White | Role::Color)
        });
        if lighty {
            Archetype::Par
        } else {
            Archetype::Specialty
        }
    }
}

/// Map a color-wheel band label to a display tint.
fn color_from_label(label: &str) -> Option<Color32> {
    let l = label.to_lowercase();
    let c = if l.contains("white") || l.contains("open") {
        Color32::from_rgb(255, 255, 230)
    } else if l.contains("red") {
        Color32::from_rgb(255, 40, 30)
    } else if l.contains("orange") || l.contains("amber") {
        Color32::from_rgb(255, 140, 20)
    } else if l.contains("yellow") {
        Color32::from_rgb(255, 230, 40)
    } else if l.contains("green") {
        Color32::from_rgb(40, 255, 60)
    } else if l.contains("cyan") || l.contains("aqua") {
        Color32::from_rgb(40, 230, 255)
    } else if l.contains("blue") {
        Color32::from_rgb(50, 80, 255)
    } else if l.contains("purple") || l.contains("violet") || l.contains("uv") {
        Color32::from_rgb(160, 50, 255)
    } else if l.contains("magenta") || l.contains("pink") {
        Color32::from_rgb(255, 60, 200)
    } else {
        return None;
    };
    Some(c)
}

/// Live render state of one fixture, derived from the DMX buffer.
pub(crate) struct Live {
    pub color: Color32,
    /// 0..1 peak output level (drives beam alpha / glow).
    pub brightness: f32,
    /// 0..1 (16-bit where a fine channel exists).
    pub pan: f32,
    pub tilt: f32,
    /// 0..1; 0.5 when the fixture has no zoom channel.
    pub zoom: f32,
}

pub(crate) fn live_state(f: &Fixture, buf: &[u8; crate::net::DMX_SLOTS]) -> Live {
    // Additive emitters (RGB, amber, UV) accumulate into `add`; subtractive
    // CMY flags accumulate into `cmy` and are applied as a filter at the end,
    // so a CMY moving head tints its white engine the way the real one does.
    let mut add = [0f32; 3];
    let mut cmy = [0f32; 3];
    let mut w = 0f32;
    let mut dim: Option<f32> = None;
    let mut has_emitter = false;
    let (mut pan_raw, mut tilt_raw) = (0u16, 0u16);
    let (mut pan_set, mut tilt_set) = (false, false);
    let mut zoom = 0.5f32;
    // Color wheel slider: tint from the active band label ("Red", "Blue"...).
    let mut wheel: Option<Color32> = None;

    for (i, ch) in f.channels.iter().enumerate() {
        let addr = f.from as usize + i;
        if addr == 0 || addr > crate::net::DMX_SLOTS {
            continue;
        }
        let v8 = buf[addr - 1];
        let v = v8 as f32 / 255.0;
        match ch.role() {
            Role::White => w = w.max(v),
            Role::Dimmer => {
                // Dimmers compressed into a sub-band on the wire (strobe
                // sections elsewhere on the channel) read back normalized.
                let lvl = match ch.dim_range() {
                    Some((lo, hi)) if hi > lo => {
                        if v8 <= lo {
                            0.0
                        } else if v8 >= hi {
                            1.0
                        } else {
                            (v8 - lo) as f32 / (hi - lo) as f32
                        }
                    }
                    _ => v,
                };
                dim = Some(dim.unwrap_or(0.0).max(lvl));
            }
            Role::Color => {
                wheel = ch.band_label(v8).and_then(color_from_label).or(wheel);
            }
            Role::Pan => {
                pan_raw = (pan_raw & 0x00FF) | ((v8 as u16) << 8);
                pan_set = true;
            }
            Role::PanFine => pan_raw = (pan_raw & 0xFF00) | v8 as u16,
            Role::Tilt => {
                tilt_raw = (tilt_raw & 0x00FF) | ((v8 as u16) << 8);
                tilt_set = true;
            }
            Role::TiltFine => tilt_raw = (tilt_raw & 0xFF00) | v8 as u16,
            Role::Zoom => zoom = v,
            // Everything else either adds light of some hue, subtracts it, or
            // doesn't touch colour at all (strobe, gobo, focus, speed…).
            role => {
                if let Some(e) = role.emitter_rgb() {
                    for k in 0..3 {
                        add[k] = add[k].max(v * e[k]);
                    }
                    has_emitter = true;
                } else if let Some(sub) = role.subtractive_rgb() {
                    for k in 0..3 {
                        cmy[k] = cmy[k].max(v * sub[k]);
                    }
                }
            }
        }
    }

    let filter = [1.0 - cmy[0], 1.0 - cmy[1], 1.0 - cmy[2]];
    let color = if !has_emitter {
        let lvl = if w > 0.0 {
            w * dim.unwrap_or(1.0)
        } else {
            dim.unwrap_or(0.0)
        };
        let tint = wheel.unwrap_or(Color32::from_rgb(255, 255, 217));
        Color32::from_rgb(
            (tint.r() as f32 * lvl * filter[0]) as u8,
            (tint.g() as f32 * lvl * filter[1]) as u8,
            (tint.b() as f32 * lvl * filter[2]) as u8,
        )
    } else {
        let dim = dim.unwrap_or(1.0);
        Color32::from_rgb(
            ((add[0] + w).min(1.0) * filter[0] * dim * 255.0) as u8,
            ((add[1] + w).min(1.0) * filter[1] * dim * 255.0) as u8,
            ((add[2] + w).min(1.0) * filter[2] * dim * 255.0) as u8,
        )
    };
    let brightness = color.r().max(color.g()).max(color.b()) as f32 / 255.0;

    Live {
        color,
        brightness,
        pan: if pan_set { pan_raw as f32 / 65535.0 } else { 0.5 },
        tilt: if tilt_set { tilt_raw as f32 / 65535.0 } else { 0.5 },
        zoom,
    }
}

/// Which gobos a fixture is projecting right now, and how far each is turned.
pub(crate) struct GoboLive<'a> {
    /// Catalogue keys, one per wheel (a static and a rotating wheel at most).
    pub keys: [Option<&'a str>; 2],
    /// Rotation of each wheel's gobo, radians.
    pub angle: [f32; 2],
}

/// Read the gobo wheels and rotation channels off the DMX buffer. `time` in
/// seconds drives continuous rotation.
pub(crate) fn gobo_state<'a>(
    f: &'a Fixture,
    buf: &[u8; crate::net::DMX_SLOTS],
    time: f64,
) -> GoboLive<'a> {
    let mut out = GoboLive { keys: [None; 2], angle: [0.0; 2] };
    let mut wheels: Vec<(usize, &'a crate::showbuddy::Channel, u8)> = Vec::new();
    let mut rotations: Vec<(usize, &'a crate::showbuddy::Channel, u8)> = Vec::new();
    for (i, ch) in f.channels.iter().enumerate() {
        let addr = f.from as usize + i;
        if addr == 0 || addr > crate::net::DMX_SLOTS {
            continue;
        }
        let v8 = buf[addr - 1];
        if ch.is_gobo_wheel() {
            if wheels.len() < 2 {
                wheels.push((i, ch, v8));
            }
        } else if ch.is_gobo_rotation() {
            rotations.push((i, ch, v8));
        }
    }
    for (w, (_, ch, v8)) in wheels.iter().enumerate() {
        out.keys[w] = ch.band_at(*v8).and_then(|b| b.gobo.as_deref());
    }
    for (ri, ch, v8) in rotations {
        if let Some(w) = wheel_for_rotation(&wheels, ri, &ch.name) {
            out.angle[w] = rotation_angle(v8, time);
        }
    }
    out
}

/// Which wheel a rotation channel turns: the one sharing its number ("Gobo
/// 2" ↔ "Gobo rotation 2"), else the one called rotating, else the nearest
/// wheel patched before it.
fn wheel_for_rotation(
    wheels: &[(usize, &crate::showbuddy::Channel, u8)],
    rotation_index: usize,
    rotation_name: &str,
) -> Option<usize> {
    if wheels.is_empty() {
        return None;
    }
    if let Some(digit) = rotation_name.chars().find(|c| c.is_ascii_digit()) {
        if let Some(w) = wheels.iter().position(|(_, ch, _)| ch.name.contains(digit)) {
            return Some(w);
        }
    }
    if let Some(w) = wheels.iter().position(|(_, ch, _)| ch.name.to_lowercase().contains("rot")) {
        return Some(w);
    }
    wheels.iter().rposition(|(ci, _, _)| *ci < rotation_index).or(Some(0))
}

/// The usual rotation channel layout: the lower half indexes the gobo to a
/// fixed angle, the upper half spins it, one way then the other, faster
/// further from the middle.
fn rotation_angle(v: u8, time: f64) -> f32 {
    use std::f32::consts::TAU;
    if v < 128 {
        return v as f32 / 127.0 * TAU;
    }
    let (speed, sign) = if v < 192 {
        ((v - 128) as f32 / 63.0, 1.0)
    } else {
        ((v - 192) as f32 / 63.0, -1.0)
    };
    ((time as f32) * speed * sign * 1.5) % TAU
}

/// Perceptual brightness curve: lifts the low end so a light running at
/// 10–30% (or even 1%) still reads as clearly coloured rather than near-black.
/// Roughly an sRGB-style gamma (linear in → perceived out).
pub(crate) fn vis_curve(b: f32) -> f32 {
    b.clamp(0.0, 1.0).powf(0.45)
}

/// Swatch for the fixture list: its live colour (scaled by output level), or
/// near-black when the fixture is off.
/// The fixture's live output level, 0 (dark) to 1 (full).
pub fn fixture_level(f: &Fixture, buf: &[u8; crate::net::DMX_SLOTS]) -> f32 {
    live_state(f, buf).brightness.clamp(0.0, 1.0)
}

pub fn fixture_swatch(f: &Fixture, buf: &[u8; crate::net::DMX_SLOTS]) -> Color32 {
    let live = live_state(f, buf);
    if live.brightness > 0.02 {
        // Normalise to the full hue, then re-apply a perceptual level so the
        // colour stays vivid at low output instead of collapsing to black.
        let m = live.brightness.max(1e-3);
        let disp = (0.22 + 0.78 * vis_curve(live.brightness)).min(1.0);
        let k = disp / m;
        Color32::from_rgb(
            (live.color.r() as f32 * k).min(255.0) as u8,
            (live.color.g() as f32 * k).min(255.0) as u8,
            (live.color.b() as f32 * k).min(255.0) as u8,
        )
    } else {
        Color32::from_gray(16)
    }
}
