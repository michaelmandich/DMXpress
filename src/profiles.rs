//! Built-in fixture profiles and the user patch.
//!
//! ShowBuddy only knows about fixtures defined in its own library; these
//! profiles let extra lights be patched directly in DMXpress. Channel maps
//! were transcribed from the manufacturers' DMX charts:
//! - Chauvet Professional *Maverick MK2 Spot* (32ch) — Maverick MK2 Spot
//!   User Manual Rev. 3.
//! - Chauvet DJ *Intimidator Spot 475ZX* (16ch) — user manual Rev. 1.
//! - Chauvet DJ *Intimidator Trio* (30ch) — user manual Rev. 3.
//! - SHEHDS *JMS WEBB LED Bee Eye 19x40W* with ring (31ch) — SHEHDS manual.
//! - A generic 4-channel RGBW par.
//!
//! Channel names are chosen so `Channel::role()` classifies them correctly
//! (Pan/Tilt/Dimmer/RGBW/Color/Strobe/Zoom/Speed).
//!
//! User-patched fixtures are persisted to `patch_user.json` and appended to
//! the ShowBuddy patch on every (re)load.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::showbuddy::{Band, Channel, Fixture, Patch};

pub const USER_PATCH_FILE: &str = "patch_user.json";

/// One built-in fixture definition.
pub struct Profile {
    pub name: &'static str,
    /// Movement geometry (degrees) for the 3D stage view.
    pub pan_range: f32,
    pub tilt_range: f32,
    pub beam_width: f32,
    build: fn() -> Vec<Channel>,
}

impl Profile {
    pub fn channels(&self) -> Vec<Channel> {
        (self.build)()
    }

    pub fn channel_count(&self) -> usize {
        self.channels().len()
    }

    /// Materialize a patched fixture at 1-based DMX address `from`.
    pub fn to_fixture(&self, display: String, from: u16) -> Fixture {
        let channels = self.channels();
        let to = from + channels.len() as u16 - 1;
        Fixture {
            display,
            file: PathBuf::from(format!("builtin:{}", self.name)),
            from,
            to,
            x: 0.5,
            y: 0.5,
            pan_range: self.pan_range,
            tilt_range: self.tilt_range,
            beam_width: self.beam_width,
            channels,
        }
    }
}

pub static PROFILES: &[Profile] = &[
    Profile {
        name: "Maverick MK2 Spot (32ch)",
        pan_range: 540.0,
        tilt_range: 270.0,
        beam_width: 22.0,
        build: maverick_mk2_spot,
    },
    Profile {
        name: "Intimidator Spot 475ZX (16ch)",
        pan_range: 540.0,
        tilt_range: 270.0,
        beam_width: 20.0,
        build: intimidator_spot_475zx,
    },
    Profile {
        name: "Intimidator Trio (30ch)",
        pan_range: 540.0,
        tilt_range: 270.0,
        beam_width: 18.0,
        build: intimidator_trio,
    },
    Profile {
        name: "SHEHDS Bee Eye 19x40 Ring (31ch)",
        pan_range: 540.0,
        tilt_range: 250.0,
        beam_width: 25.0,
        build: shehds_bee_eye_19x40,
    },
    Profile {
        name: "25 ch banger (25ch)",
        pan_range: 540.0,
        tilt_range: 270.0,
        beam_width: 20.0,
        build: banger_25ch,
    },
    Profile {
        name: "Generic RGBW Par (4ch)",
        pan_range: 0.0,
        tilt_range: 0.0,
        beam_width: 30.0,
        build: generic_rgbw_par,
    },
    Profile {
        name: "SlimPAR T12 BT (7ch)",
        pan_range: 0.0,
        tilt_range: 0.0,
        beam_width: 30.0,
        build: chauvet_par_7ch,
    },
    Profile {
        name: "SlimPAR T6 BT (7ch)",
        pan_range: 0.0,
        tilt_range: 0.0,
        beam_width: 28.0,
        build: chauvet_par_7ch,
    },
    Profile {
        name: "SlimPAR Q12 BT (8ch)",
        pan_range: 0.0,
        tilt_range: 0.0,
        beam_width: 30.0,
        build: chauvet_par_8ch,
    },
    Profile {
        name: "Level Q7 IP (7ch)",
        pan_range: 0.0,
        tilt_range: 0.0,
        beam_width: 30.0,
        build: level_q7_7ch,
    },
    Profile {
        name: "Fogger (2ch)",
        pan_range: 0.0,
        tilt_range: 0.0,
        beam_width: 0.0,
        build: fogger_2ch,
    },
];

pub fn find(name: &str) -> Option<&'static Profile> {
    PROFILES.iter().find(|p| p.name == name)
}

// ---- channel builders ----

fn band(kind: char, min: u8, max: u8, label: &str) -> Band {
    Band { kind, min, max, label: label.to_string() }
}

/// Continuous 0-255 value channel.
fn v(name: &str) -> Channel {
    Channel { name: name.into(), bands: vec![band('V', 0, 255, "")] }
}

/// Dimmer channel (first band kind `D` also marks it for role inference).
fn d(name: &str) -> Channel {
    Channel { name: name.into(), bands: vec![band('D', 0, 255, "")] }
}

/// Switched/stepped channel from `(min, max, label)` triples.
fn s(name: &str, bands: &[(u8, u8, &str)]) -> Channel {
    Channel {
        name: name.into(),
        bands: bands.iter().map(|&(lo, hi, l)| band('S', lo, hi, l)).collect(),
    }
}

/// Chauvet Professional Maverick MK2 Spot, 32-channel mode (manual Rev. 3).
fn maverick_mk2_spot() -> Vec<Channel> {
    let gobo_wheel = |name: &str, g: [&str; 6]| {
        s(name, &[
            (0, 8, "Open"),
            (9, 17, g[0]),
            (18, 26, g[1]),
            (27, 35, g[2]),
            (36, 44, g[3]),
            (45, 53, g[4]),
            (54, 63, g[5]),
            (64, 73, "Gobo 6 shake"),
            (74, 82, "Gobo 5 shake"),
            (83, 91, "Gobo 4 shake"),
            (92, 100, "Gobo 3 shake"),
            (101, 109, "Gobo 2 shake"),
            (110, 118, "Gobo 1 shake"),
            (119, 127, "Open"),
            (128, 191, "Scroll CW"),
            (192, 255, "Scroll CCW"),
        ])
    };
    let gobo_rot = |name: &str| {
        s(name, &[
            (0, 63, "Index"),
            (64, 145, "Rotate CW"),
            (146, 149, "Stop"),
            (150, 231, "Rotate CCW"),
            (232, 255, "Bounce"),
        ])
    };
    vec![
        v("Pan"),
        v("Pan fine"),
        v("Tilt"),
        v("Tilt fine"),
        v("Pan/Tilt speed"),
        d("Dimmer"),
        v("Dimmer fine"),
        // Named "Shutter" (not "Strobe") on purpose: it's mostly open/close,
        // so role-based strobe effects land on ch 9 (virtual strobe) instead.
        s("Shutter", &[
            (0, 3, "Closed"),
            (4, 7, "Open"),
            (8, 76, "Strobe"),
            (77, 145, "Pulse"),
            (146, 215, "Random"),
            (216, 255, "Open"),
        ]),
        s("Virtual strobe", &[
            (0, 1, "Off"),
            (2, 128, "Shaking"),
            (129, 255, "Fade in/out"),
        ]),
        v("Cyan"),
        v("Magenta"),
        v("Yellow (CMY)"),
        v("CTO"),
        s("Color wheel", &[
            (0, 6, "Open"),
            (7, 13, "Red"),
            (14, 20, "Orange"),
            (21, 27, "Green"),
            (28, 34, "Blue"),
            (35, 41, "Magenta"),
            (42, 47, "Yellow"),
            (48, 59, "UV"),
            (60, 187, "Split colors"),
            (188, 219, "Scroll CW"),
            (220, 223, "Stop"),
            (224, 255, "Scroll CCW"),
        ]),
        gobo_wheel(
            "Gobo wheel 1",
            [
                "Circuits",
                "Ring of rings",
                "Checker vortex",
                "Triangle",
                "Star field",
                "Lenticular glass",
            ],
        ),
        gobo_rot("Gobo rotating 1"),
        v("Gobo wheel 1 index fine"),
        gobo_wheel(
            "Gobo wheel 2",
            [
                "Spiral",
                "Dot chiclets",
                "Splat breakup",
                "Wavy bar",
                "Shower glass",
                "Lenticular glass",
            ],
        ),
        gobo_rot("Gobo rotating 2"),
        v("Gobo wheel 2 index fine"),
        v("Focus"),
        v("Focus fine"),
        v("Auto focus"),
        v("Zoom"),
        v("Zoom fine"),
        s("Prism", &[(0, 4, "Off"), (5, 255, "Prism")]),
        s("Prism rotation", &[
            (0, 127, "Index"),
            (128, 189, "Rotate CW"),
            (190, 193, "Stop"),
            (194, 255, "Rotate CCW"),
        ]),
        s("Iris", &[
            (0, 63, "Big to small"),
            (64, 127, "Auto change"),
            (128, 191, "Zoom in/out slow"),
            (192, 255, "Zoom out/in slow"),
        ]),
        v("Frost"),
        s("CMY macro", &[(0, 9, "Off"), (10, 255, "Macro")]),
        v("CMY macro rate"),
        v("Control"),
    ]
}

/// Chauvet DJ Intimidator Spot 475ZX, 16-channel mode.
fn intimidator_spot_475zx() -> Vec<Channel> {
    vec![
        v("Pan"),
        v("Pan fine"),
        v("Tilt"),
        v("Tilt fine"),
        v("Pan/Tilt speed"),
        s("Color wheel", &[
            (0, 7, "White"),
            (8, 15, "Red"),
            (16, 23, "Yellow"),
            (24, 31, "Green"),
            (32, 39, "Blue"),
            (40, 47, "CTO"),
            (48, 55, "Cyan"),
            (56, 63, "Magenta"),
            (64, 68, "White"),
            (69, 189, "Index"),
            (190, 221, "Rainbow"),
            (222, 223, "Stop"),
            (224, 255, "Rainbow rev"),
        ]),
        v("Gobo wheel (rotating)"),
        v("Gobo rotation"),
        v("Gobo wheel (static)"),
        v("Prism"),
        v("Focus"),
        v("Zoom"),
        d("Dimmer"),
        s("Strobe", &[
            (0, 3, "Off"),
            (4, 7, "On"),
            (8, 76, "Strobe"),
            (77, 145, "Pulse"),
            (146, 215, "Random"),
            (216, 255, "On"),
        ]),
        v("Control"),
        v("Movement macros"),
    ]
}

/// Chauvet DJ Intimidator Trio, 30-channel mode (moving head, 3 RGBW zones).
fn intimidator_trio() -> Vec<Channel> {
    vec![
        v("Pan"),
        v("Pan fine"),
        v("Tilt"),
        v("Tilt fine"),
        v("Pan/Tilt speed"),
        v("Red 1"),
        v("Green 1"),
        v("Blue 1"),
        v("White 1"),
        v("Red 2"),
        v("Green 2"),
        v("Blue 2"),
        v("White 2"),
        v("Red 3"),
        v("Green 3"),
        v("Blue 3"),
        v("White 3"),
        v("No function 1"),
        v("No function 2"),
        v("No function 3"),
        v("No function 4"),
        v("Color macros"),
        v("Auto programs"),
        v("Auto rate"),
        d("Dimmer"),
        s("Strobe", &[
            (0, 19, "Closed"),
            (20, 24, "Open"),
            (25, 244, "Strobe"),
            (245, 255, "Open"),
        ]),
        v("Zoom"),
        v("Control"),
        v("Movement macros"),
        v("Rotation"),
    ]
}

/// SHEHDS JMS WEBB LED Bee Eye 19x40W with ring, 31-channel mode.
fn shehds_bee_eye_19x40() -> Vec<Channel> {
    vec![
        v("Pan"),
        v("Pan fine"),
        v("Tilt"),
        v("Tilt fine"),
        v("Pan/Tilt speed"),
        v("Zoom"),
        s("Barrel roll", &[
            (0, 155, "Off"),
            (156, 205, "CW roll"),
            (206, 255, "CCW roll"),
        ]),
        d("Dimmer"),
        s("Strobe", &[
            (0, 3, "Open"),
            (4, 203, "Strobe"),
            (204, 255, "Random"),
        ]),
        v("Red"),
        v("Green"),
        v("Blue"),
        v("White"),
        v("CTO"),
        v("Color macros"),
        v("Static effect"),
        v("Dynamic effect"),
        v("Effect rate"),
        v("BackColor R"),
        v("BackColor G"),
        v("BackColor B"),
        v("BackColor W"),
        s("Reset", &[(0, 250, "Off"), (251, 255, "Reset")]),
        v("Ring flash"),
        v("Ring R"),
        v("Ring G"),
        v("Ring B"),
        v("Ring mode"),
        v("Ring rate"),
        v("Ring BG"),
        v("Ring BG level"),
    ]
}

/// Plain 4-channel RGBW par can.
fn generic_rgbw_par() -> Vec<Channel> {
    vec![v("Red"), v("Green"), v("Blue"), v("White")]
}

/// Chauvet DJ BT-series RGB par, 7-channel mode (SlimPAR T6 BT / T12 BT):
/// RGB, strobe, color macro, auto/sound mode, dimmer last.
fn chauvet_par_7ch() -> Vec<Channel> {
    vec![
        v("Red"),
        v("Green"),
        v("Blue"),
        s("Strobe", &[
            (0, 9, "Open"),
            (10, 255, "Strobe slow → fast"),
        ]),
        s("Color macro", &[
            (0, 9, "Off"),
            (10, 255, "Macros"),
        ]),
        s("Programs", &[
            (0, 9, "Off"),
            (10, 255, "Auto/sound programs"),
        ]),
        d("Dimmer"),
    ]
}

/// Chauvet Level Q7 IP, 7-channel mode:
/// RGBW, color correction, color macros, dimmer last.
fn level_q7_7ch() -> Vec<Channel> {
    vec![
        v("Red"),
        v("Green"),
        v("Blue"),
        v("White"),
        s("Color correction", &[
            (0, 9, "Off"),
            (10, 255, "Color correction"),
        ]),
        s("Color macro", &[
            (0, 9, "Off"),
            (10, 255, "Macros"),
        ]),
        d("Dimmer"),
    ]
}

/// Chauvet DJ RGBA par, 8-channel mode (SlimPAR Q12 BT):
/// RGBA, strobe, color macro, auto/sound mode, dimmer last.
fn chauvet_par_8ch() -> Vec<Channel> {
    vec![
        v("Red"),
        v("Green"),
        v("Blue"),
        v("Amber"),
        s("Strobe", &[
            (0, 9, "Open"),
            (10, 255, "Strobe slow → fast"),
        ]),
        s("Color macro", &[
            (0, 9, "Off"),
            (10, 255, "Macros"),
        ]),
        s("Programs", &[
            (0, 9, "Off"),
            (10, 255, "Auto/sound programs"),
        ]),
        d("Dimmer"),
    ]
}

/// Generic 2-channel fogger: fan output first, heater second. No light
/// output — classifies as `Specialty` so the stage draws a small box only.
fn fogger_2ch() -> Vec<Channel> {
    vec![v("Fan"), v("Heat")]
}

/// "25 ch banger" moving head (mirrors the ShowBuddy custom .dmx): pan/tilt,
/// spinner, master dimmer, strobe, main + D + L RGB(W) zones, ring, macros.
fn banger_25ch() -> Vec<Channel> {
    vec![
        v("Pan"),
        v("Panf"),
        v("Tilt"),
        v("Tiltf"),
        v("Tiltspd"),
        v("SPINNER"),
        d("Master"),
        v("STRB"),
        v("RED"),
        v("GRN"),
        v("BLU"),
        v("WHT"),
        v("DRED"),
        v("DGRN"),
        v("DBLU"),
        v("DWHITE"),
        v("LRED"),
        v("LGRN"),
        v("LBLU"),
        v("YSTB"),
        v("LRING"),
        v("Auto"),
        v("AUTO"),
        v("RST"),
        v("Ch 25"),
    ]
}

// ---- user patch persistence ----

/// One fixture the user added on top of the ShowBuddy patch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFixture {
    /// Built-in profile name (see [`PROFILES`]).
    pub profile: String,
    pub display: String,
    /// 1-based absolute DMX start address.
    pub from: u16,
}

/// On-disk DMXpress patch: user fixtures plus whether the ShowBuddy patch is
/// merged in at all (off = a fresh rig built only from built-in profiles).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPatch {
    #[serde(default = "yes")]
    pub include_showbuddy: bool,
    #[serde(default)]
    pub fixtures: Vec<UserFixture>,
    /// Individual ShowBuddy fixtures hidden from the rig (`display@from` keys).
    #[serde(default)]
    pub excluded: Vec<String>,
}

fn yes() -> bool {
    true
}

impl Default for UserPatch {
    fn default() -> Self {
        Self {
            include_showbuddy: true,
            fixtures: Vec::new(),
            excluded: Vec::new(),
        }
    }
}

pub fn load_user_patch() -> UserPatch {
    let Ok(text) = std::fs::read_to_string(USER_PATCH_FILE) else {
        return UserPatch::default();
    };
    if let Ok(p) = serde_json::from_str::<UserPatch>(&text) {
        return p;
    }
    // Legacy format: a bare fixture list.
    UserPatch {
        include_showbuddy: true,
        fixtures: serde_json::from_str(&text).unwrap_or_default(),
        excluded: Vec::new(),
    }
}

pub fn save_user_patch(patch: &UserPatch) {
    if let Ok(json) = serde_json::to_string_pretty(patch) {
        let _ = std::fs::write(USER_PATCH_FILE, json);
    }
}
/// Stable identifier for a patched fixture (used by the exclusion list).
pub fn fixture_key(display: &str, from: u16) -> String {
    format!("{display}@{from}")
}

/// Drop excluded ShowBuddy fixtures, then append the user-patched ones.
pub fn extend_patch(patch: &mut Patch, user: &UserPatch) {
    patch
        .fixtures
        .retain(|f| !user.excluded.contains(&fixture_key(&f.display, f.from)));
    for uf in &user.fixtures {
        let Some(profile) = find(&uf.profile) else {
            patch
                .warnings
                .push(format!("'{}': unknown profile '{}'", uf.display, uf.profile));
            continue;
        };
        let fixture = profile.to_fixture(uf.display.clone(), uf.from);
        for other in &patch.fixtures {
            if fixture.from <= other.to && other.from <= fixture.to {
                patch.warnings.push(format!(
                    "'{}' ({}-{}) overlaps '{}' ({}-{})",
                    fixture.display, fixture.from, fixture.to,
                    other.display, other.from, other.to
                ));
            }
        }
        patch.fixtures.push(fixture);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::showbuddy::Role;

    #[test]
    fn channel_counts_match_modes() {
        let counts: Vec<(&str, usize)> = PROFILES
            .iter()
            .map(|p| (p.name, p.channel_count()))
            .collect();
        assert_eq!(
            counts,
            vec![
                ("Maverick MK2 Spot (32ch)", 32),
                ("Intimidator Spot 475ZX (16ch)", 16),
                ("Intimidator Trio (30ch)", 30),
                ("SHEHDS Bee Eye 19x40 Ring (31ch)", 31),
                ("25 ch banger (25ch)", 25),
                ("Generic RGBW Par (4ch)", 4),
                ("SlimPAR T12 BT (7ch)", 7),
                ("SlimPAR T6 BT (7ch)", 7),
                ("SlimPAR Q12 BT (8ch)", 8),
                ("Level Q7 IP (7ch)", 7),
                ("Fogger (2ch)", 2),
            ]
        );
    }

    /// First channel with each role must be the intended one.
    #[test]
    fn key_roles_classify() {
        let role_at = |profile: &Profile, role: Role| {
            profile.channels().iter().position(|c| c.role() == role)
        };
        for p in PROFILES.iter().take(4) {
            assert_eq!(role_at(p, Role::Pan), Some(0), "{}", p.name);
            assert_eq!(role_at(p, Role::PanFine), Some(1), "{}", p.name);
            assert_eq!(role_at(p, Role::Tilt), Some(2), "{}", p.name);
            assert_eq!(role_at(p, Role::TiltFine), Some(3), "{}", p.name);
            assert_eq!(role_at(p, Role::Speed), Some(4), "{}", p.name);
        }
        let maverick = &PROFILES[0];
        assert_eq!(role_at(maverick, Role::Dimmer), Some(5));
        // Ch 8 is "Shutter" (Role::Other); the strobe role is ch 9's
        // virtual strobe, which does the actual visible strobing.
        assert_eq!(role_at(maverick, Role::Strobe), Some(8));
        assert_eq!(role_at(maverick, Role::Color), Some(13));
        assert_eq!(role_at(maverick, Role::Zoom), Some(23));
        let zx = &PROFILES[1];
        assert_eq!(role_at(zx, Role::Color), Some(5));
        assert_eq!(role_at(zx, Role::Zoom), Some(11));
        assert_eq!(role_at(zx, Role::Dimmer), Some(12));
        assert_eq!(role_at(zx, Role::Strobe), Some(13));
        let trio = &PROFILES[2];
        assert_eq!(role_at(trio, Role::Red), Some(5));
        assert_eq!(role_at(trio, Role::Green), Some(6));
        assert_eq!(role_at(trio, Role::Blue), Some(7));
        assert_eq!(role_at(trio, Role::White), Some(8));
        assert_eq!(role_at(trio, Role::Dimmer), Some(24));
        assert_eq!(role_at(trio, Role::Zoom), Some(26));
        let bee = &PROFILES[3];
        assert_eq!(role_at(bee, Role::Zoom), Some(5));
        assert_eq!(role_at(bee, Role::Dimmer), Some(7));
        assert_eq!(role_at(bee, Role::Strobe), Some(8));
        assert_eq!(role_at(bee, Role::Red), Some(9));
        assert_eq!(role_at(bee, Role::White), Some(12));
        let banger = &PROFILES[4];
        assert_eq!(role_at(banger, Role::Pan), Some(0));
        assert_eq!(role_at(banger, Role::Tilt), Some(2));
        assert_eq!(role_at(banger, Role::Dimmer), Some(6));
        assert_eq!(role_at(banger, Role::Strobe), Some(7));
        assert_eq!(role_at(banger, Role::Red), Some(8));
        let par = &PROFILES[5];
        assert_eq!(role_at(par, Role::Red), Some(0));
        assert_eq!(role_at(par, Role::White), Some(3));
    }

    #[test]
    fn patched_addresses_span_channels() {
        let p = find("Generic RGBW Par (4ch)").unwrap();
        let f = p.to_fixture("Par 1".into(), 101);
        assert_eq!((f.from, f.to), (101, 104));
        assert_eq!(f.channel_count(), 4);
    }
}
