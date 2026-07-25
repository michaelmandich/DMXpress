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
    let (mut r, mut g, mut b, mut w) = (0f32, 0f32, 0f32, 0f32);
    let mut dim: Option<f32> = None;
    let mut has_rgb = false;
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
            Role::Red => {
                r = r.max(v);
                has_rgb = true;
            }
            Role::Green => {
                g = g.max(v);
                has_rgb = true;
            }
            Role::Blue => {
                b = b.max(v);
                has_rgb = true;
            }
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
            Role::Strobe | Role::Speed | Role::Other => {}
        }
    }

    let color = if !has_rgb {
        let lvl = if w > 0.0 {
            w * dim.unwrap_or(1.0)
        } else {
            dim.unwrap_or(0.0)
        };
        let tint = wheel.unwrap_or(Color32::from_rgb(255, 255, 217));
        Color32::from_rgb(
            (tint.r() as f32 * lvl) as u8,
            (tint.g() as f32 * lvl) as u8,
            (tint.b() as f32 * lvl) as u8,
        )
    } else {
        let dim = dim.unwrap_or(1.0);
        Color32::from_rgb(
            ((r + w).min(1.0) * dim * 255.0) as u8,
            ((g + w).min(1.0) * dim * 255.0) as u8,
            ((b + w).min(1.0) * dim * 255.0) as u8,
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

/// Perceptual brightness curve: lifts the low end so a light running at
/// 10–30% (or even 1%) still reads as clearly coloured rather than near-black.
/// Roughly an sRGB-style gamma (linear in → perceived out).
pub(crate) fn vis_curve(b: f32) -> f32 {
    b.clamp(0.0, 1.0).powf(0.45)
}

/// Swatch for the fixture list: its live colour (scaled by output level), or
/// near-black when the fixture is off.
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
