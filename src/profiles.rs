//! Built-in fixture profiles and the user patch.
//!
//! ShowBuddy only knows about fixtures defined in its own library; these
//! profiles let extra lights be patched directly in DMXpress. Channel maps
//! were transcribed from the manufacturers' DMX charts:
//! - Chauvet Professional *Maverick MK2 Spot* (32ch and 24ch) — Maverick
//!   MK2 Spot User Manual Rev. 3.
//! - Chauvet DJ *Intimidator Spot 475ZX* (16ch) — user manual Rev. 1.
//! - Chauvet DJ *Intimidator Spot 475Z* (16ch) — user manual Rev. 1.
//! - Chauvet DJ *Intimidator Spot 375Z IRC* (15ch) — user manual Rev. 7.
//! - Chauvet DJ *Intimidator Trio* (30ch — user manual Rev. 3; 15ch — Rev. 5).
//! - Chauvet Professional *Rogue R1 Wash* (14ch) — Rogue R1 Wash DMX chart
//!   Rev. 1.
//! - SHEHDS *JMS WEBB LED Bee Eye 19x40W* with ring (31ch) — SHEHDS manual.
//! - Betopper *LF2405* matrix strobe (11ch) — user manual Rev. 1.01.
//! - A generic 4-channel RGBW par.
//!
//! Channel names are chosen so `Channel::role()` classifies them correctly
//! (Pan/Tilt/Dimmer/RGBW/Color/Strobe/Zoom/Speed).
//!
//! User-patched fixtures are persisted to `patch_user.json` and appended to
//! the ShowBuddy patch on every (re)load.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::fixturedb::{self, Library};
use crate::showbuddy::{Band, Channel, Fixture, Patch, Role};

pub const USER_PATCH_FILE: &str = "patch_user.json";
/// How many recently patched profiles `UserPatch::recent` keeps.
pub const RECENT_CAP: usize = 8;

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
        name: "Maverick MK2 Spot (24ch)",
        pan_range: 540.0,
        tilt_range: 270.0,
        beam_width: 22.0,
        build: maverick_mk2_spot_24,
    },
    Profile {
        name: "Intimidator Spot 475ZX (16ch)",
        pan_range: 540.0,
        tilt_range: 270.0,
        beam_width: 20.0,
        build: intimidator_spot_475zx,
    },
    Profile {
        name: "Intimidator Spot 475Z (16ch)",
        pan_range: 540.0,
        tilt_range: 270.0,
        beam_width: 20.0,
        build: intimidator_spot_475z,
    },
    Profile {
        name: "Intimidator Spot 375Z IRC (15ch)",
        pan_range: 540.0,
        tilt_range: 270.0,
        beam_width: 16.0,
        build: intimidator_spot_375z,
    },
    Profile {
        name: "Intimidator Trio (30ch)",
        pan_range: 540.0,
        tilt_range: 270.0,
        beam_width: 18.0,
        build: intimidator_trio,
    },
    Profile {
        name: "Intimidator Trio (15ch)",
        pan_range: 540.0,
        tilt_range: 270.0,
        beam_width: 18.0,
        build: intimidator_trio_15,
    },
    Profile {
        name: "Rogue R1 Wash (14ch)",
        pan_range: 540.0,
        tilt_range: 230.0,
        beam_width: 30.0,
        build: rogue_r1_wash,
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
        name: "Betopper LF2405 Matrix Strobe (11ch)",
        pan_range: 0.0,
        tilt_range: 0.0,
        beam_width: 40.0,
        build: betopper_lf2405,
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
    Band { kind, min, max, label: label.to_string(), gobo: None }
}

/// Continuous 0-255 value channel. These built-ins leave `role` unset and
/// lean on name inference, the way they always have.
fn v(name: &str) -> Channel {
    Channel { name: name.into(), bands: vec![band('V', 0, 255, "")], role: None }
}

/// Dimmer channel (first band kind `D` also marks it for role inference).
fn d(name: &str) -> Channel {
    Channel { name: name.into(), bands: vec![band('D', 0, 255, "")], role: None }
}

/// Switched/stepped channel from `(min, max, label)` triples.
fn s(name: &str, bands: &[(u8, u8, &str)]) -> Channel {
    Channel {
        name: name.into(),
        bands: bands.iter().map(|&(lo, hi, l)| band('S', lo, hi, l)).collect(),
        role: None,
    }
}

/// Stepped channel with its role pinned instead of inferred, for the odd
/// control whose manual wording would mislead the name classifier (e.g. a
/// "colour select" that only steers a built-in effect, not the output).
fn s_role(name: &str, role: Role, bands: &[(u8, u8, &str)]) -> Channel {
    Channel { role: Some(role), ..s(name, bands) }
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

/// Chauvet Professional Maverick MK2 Spot, 24-channel mode (manual Rev. 3).
/// The same controls as the 32-channel mode, in the same order, minus its
/// eight fine/auxiliary channels: fine dimmer, both gobo index fine channels,
/// fine focus, auto focus, fine zoom, CMY macro and CMY macro speed.
fn maverick_mk2_spot_24() -> Vec<Channel> {
    const DROPPED: [&str; 8] = [
        "Dimmer fine",
        "Gobo wheel 1 index fine",
        "Gobo wheel 2 index fine",
        "Focus fine",
        "Auto focus",
        "Zoom fine",
        "CMY macro",
        "CMY macro rate",
    ];
    maverick_mk2_spot()
        .into_iter()
        .filter(|c| !DROPPED.contains(&c.name.as_str()))
        .collect()
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

// ---- Chauvet DJ Intimidator family (shared channels) ----

/// Colour wheel shared by the Intimidator Spot 375Z IRC and 475Z.
fn intimidator_color_wheel() -> Channel {
    s("Color wheel", &[
        (0, 7, "White"),
        (8, 15, "Orange"),
        (16, 23, "Lime green"),
        (24, 31, "Cyan"),
        (32, 39, "Red"),
        (40, 47, "Green"),
        (48, 55, "Magenta"),
        (56, 63, "Yellow"),
        (64, 64, "White"),
        (65, 189, "Index"),
        (190, 221, "Rainbow"),
        (222, 223, "Stop"),
        (224, 255, "Rainbow rev"),
    ])
}

/// Shutter/strobe channel shared by the Intimidator Spot 375Z IRC and 475Z.
fn intimidator_strobe() -> Channel {
    s("Strobe", &[
        (0, 3, "Closed"),
        (4, 7, "Open"),
        (8, 76, "Strobe"),
        (77, 145, "Pulse"),
        (146, 215, "Random"),
        (216, 255, "Open"),
    ])
}

/// Function/control channel shared by the Intimidator spots. `optics_reset`
/// is what the 144–151 band does: focus + zoom reset on the 375Z, nothing on
/// the 475Z.
fn intimidator_control(optics_reset: &str) -> Channel {
    s("Control", &[
        (0, 7, "None"),
        (8, 15, "Blackout on P/T"),
        (16, 23, "Blackout on color"),
        (24, 31, "Blackout on gobo"),
        (32, 39, "Blackout on P/T + color"),
        (40, 47, "Blackout on P/T + gobo"),
        (48, 55, "Blackout on P/T + color + gobo"),
        (56, 95, "None"),
        (96, 103, "Pan reset"),
        (104, 111, "Tilt reset"),
        (112, 119, "Color reset"),
        (120, 127, "Gobo reset"),
        (128, 135, "None"),
        (136, 143, "Prism reset"),
        (144, 151, optics_reset),
        (152, 159, "Reset all"),
        (160, 255, "None"),
    ])
}

/// Movement-macro channel shared by the Intimidator spots and the Trio.
fn intimidator_movement_macros() -> Channel {
    s("Movement macros", &[
        (0, 7, "Off"),
        (8, 23, "Macro 1"),
        (24, 39, "Macro 2"),
        (40, 55, "Macro 3"),
        (56, 71, "Macro 4"),
        (72, 87, "Macro 5"),
        (88, 103, "Macro 6"),
        (104, 119, "Macro 7"),
        (120, 135, "Macro 8"),
        (136, 151, "Sound macro 1"),
        (152, 167, "Sound macro 2"),
        (168, 183, "Sound macro 3"),
        (184, 199, "Sound macro 4"),
        (200, 215, "Sound macro 5"),
        (216, 231, "Sound macro 6"),
        (232, 247, "Sound macro 7"),
        (248, 255, "Sound macro 8"),
    ])
}

/// Chauvet DJ Intimidator Spot 475Z, 16-channel mode (user manual Rev. 1).
/// Not the 475ZX: the wheels and prism bands differ.
fn intimidator_spot_475z() -> Vec<Channel> {
    vec![
        v("Pan"),
        v("Pan fine"),
        v("Tilt"),
        v("Tilt fine"),
        v("Pan/Tilt speed"),
        intimidator_color_wheel(),
        s("Gobo wheel (rotating)", &[
            (0, 7, "Open"),
            (8, 15, "Gobo 1"),
            (16, 23, "Gobo 2"),
            (24, 31, "Gobo 3"),
            (32, 39, "Gobo 4"),
            (40, 47, "Gobo 5"),
            (48, 55, "Gobo 6"),
            (56, 63, "Gobo 7"),
            (64, 71, "Gobo 7 shake"),
            (72, 79, "Gobo 6 shake"),
            (80, 87, "Gobo 5 shake"),
            (88, 95, "Gobo 4 shake"),
            (96, 103, "Gobo 3 shake"),
            (104, 111, "Gobo 2 shake"),
            (112, 119, "Gobo 1 shake"),
            (120, 127, "Open"),
            (128, 191, "Cycle"),
            (192, 255, "Cycle rev"),
        ]),
        s("Gobo rotation", &[
            (0, 63, "Index"),
            (64, 147, "Rotate"),
            (148, 231, "Rotate rev"),
            (232, 255, "Bounce"),
        ]),
        s("Gobo wheel (static)", &[
            (0, 6, "Open"),
            (7, 13, "Gobo 1"),
            (14, 20, "Gobo 2"),
            (21, 27, "Gobo 3"),
            (28, 34, "Gobo 4"),
            (35, 41, "Gobo 5"),
            (42, 48, "Gobo 6"),
            (49, 55, "Gobo 7"),
            (56, 63, "Gobo 8"),
            (64, 71, "Gobo 8 shake"),
            (72, 78, "Gobo 7 shake"),
            (79, 85, "Gobo 6 shake"),
            (86, 92, "Gobo 5 shake"),
            (93, 99, "Gobo 4 shake"),
            (100, 106, "Gobo 3 shake"),
            (107, 113, "Gobo 2 shake"),
            (114, 120, "Gobo 1 shake"),
            (121, 127, "Open"),
            (128, 191, "Cycle rev"),
            (192, 255, "Cycle"),
        ]),
        s("Prism", &[
            (0, 3, "Off"),
            (4, 6, "Round prism"),
            (7, 65, "Round rotate"),
            (66, 123, "Round rotate rev"),
            (124, 127, "Round prism"),
            (128, 131, "Off"),
            (132, 134, "Linear prism"),
            (135, 193, "Linear rotate"),
            (194, 251, "Linear rotate rev"),
            (252, 255, "Linear prism"),
        ]),
        v("Focus"),
        v("Zoom"),
        d("Dimmer"),
        intimidator_strobe(),
        intimidator_control("None"),
        intimidator_movement_macros(),
    ]
}

/// Chauvet DJ Intimidator Spot 375Z IRC, 15-channel mode (user manual Rev. 7).
fn intimidator_spot_375z() -> Vec<Channel> {
    vec![
        v("Pan"),
        v("Pan fine"),
        v("Tilt"),
        v("Tilt fine"),
        v("Pan/Tilt speed"),
        intimidator_color_wheel(),
        s("Gobo wheel", &[
            (0, 7, "Open"),
            (8, 15, "Gobo 1"),
            (16, 23, "Gobo 2"),
            (24, 31, "Gobo 3"),
            (32, 39, "Gobo 4"),
            (40, 47, "Gobo 5"),
            (48, 55, "Gobo 6"),
            (56, 63, "Gobo 7"),
            (64, 71, "Gobo 7 shake"),
            (72, 79, "Gobo 6 shake"),
            (80, 87, "Gobo 5 shake"),
            (88, 95, "Gobo 4 shake"),
            (96, 103, "Gobo 3 shake"),
            (104, 111, "Gobo 2 shake"),
            (112, 119, "Gobo 1 shake"),
            (120, 127, "Open"),
            (128, 189, "Cycle"),
            (190, 193, "Stop"),
            (194, 255, "Cycle rev"),
        ]),
        s("Gobo rotation", &[
            (0, 0, "Off"),
            (1, 63, "Index"),
            (64, 145, "Rotate"),
            (146, 149, "Stop"),
            (150, 231, "Rotate rev"),
            (232, 255, "Bounce"),
        ]),
        s("Prism", &[
            (0, 3, "Off"),
            (4, 6, "6-facet"),
            (7, 65, "6-facet rotate"),
            (66, 123, "6-facet rotate rev"),
            (124, 127, "6-facet"),
            (128, 131, "Off"),
            (132, 134, "5-facet"),
            (135, 193, "5-facet rotate"),
            (194, 251, "5-facet rotate rev"),
            (252, 255, "5-facet"),
        ]),
        v("Focus"),
        d("Dimmer"),
        intimidator_strobe(),
        intimidator_control("Focus/zoom reset"),
        intimidator_movement_macros(),
        v("Zoom"),
    ]
}

/// Chauvet DJ Intimidator Trio, 15-channel mode (user manual Rev. 5): the
/// three RGBW zones collapse into one set and pan/tilt lose their fine
/// channels.
fn intimidator_trio_15() -> Vec<Channel> {
    vec![
        v("Pan"),
        v("Tilt"),
        v("Pan/Tilt speed"),
        v("Red"),
        v("Green"),
        v("Blue"),
        v("White"),
        s("Auto programs", &[
            (0, 15, "Off"),
            (16, 31, "Zone macro 1"),
            (32, 47, "Zone macro 2"),
            (48, 63, "Zone macro 3"),
            (64, 79, "Zone macro 4"),
            (80, 95, "Zone macro 5"),
            (96, 111, "Zone macro 6"),
            (112, 127, "Zone macros 1-6"),
            (128, 143, "Auto 1"),
            (144, 159, "Auto 2"),
            (160, 175, "Auto 3"),
            (176, 191, "Auto 4"),
            (192, 207, "Auto 5"),
            (208, 223, "Auto 6"),
            (224, 239, "Auto 7"),
            (240, 255, "Autos 1-7"),
        ]),
        // "Auto rate", not "speed": the speed role belongs to pan/tilt.
        v("Auto rate"),
        d("Dimmer"),
        s("Strobe", &[
            (0, 19, "Closed"),
            (20, 24, "Open"),
            (25, 64, "Strobe"),
            (65, 69, "Open"),
            (70, 84, "Fast on / slow off"),
            (85, 89, "Open"),
            (90, 104, "Slow on / fast off"),
            (105, 109, "Open"),
            (110, 124, "Random"),
            (125, 129, "Open"),
            (130, 144, "Random fast on / slow off"),
            (145, 149, "Open"),
            (150, 164, "Random slow on / fast off"),
            (165, 169, "Open"),
            (170, 184, "Pulse 1"),
            (185, 189, "Open"),
            (190, 204, "Pulse 2"),
            (205, 209, "Open"),
            (210, 224, "Fade on/off"),
            (225, 229, "Open"),
            (230, 244, "Pulse 3"),
            (245, 255, "Open"),
        ]),
        v("Zoom"),
        s("Control", &[
            (0, 9, "None"),
            (10, 14, "Blackout on P/T"),
            (15, 49, "None"),
            (50, 54, "Pan reset"),
            (55, 59, "Tilt reset"),
            (60, 64, "Zoom reset"),
            (65, 69, "Rotation reset"),
            (70, 74, "Reset all"),
            (75, 79, "None"),
            (80, 84, "Reverse P/T"),
            (85, 89, "Reverse pan"),
            (90, 94, "Reverse tilt"),
            (95, 99, "Normal pan"),
            (100, 104, "Normal tilt"),
            (105, 109, "Normal P/T"),
            (110, 124, "None"),
            (125, 129, "Fan full"),
            (130, 134, "Fan auto"),
            (135, 139, "Dimmer fast"),
            (140, 144, "Dimmer smooth"),
            (145, 255, "None"),
        ]),
        intimidator_movement_macros(),
        // Only active while zoom (ch 12) is above 150.
        s("Rotation", &[
            (0, 63, "Index"),
            (64, 95, "Small shake"),
            (96, 127, "Large shake"),
            (128, 191, "Rotate"),
            (192, 255, "Rotate rev"),
        ]),
    ]
}

/// Chauvet Professional Rogue R1 Wash, 14-channel mode (DMX chart Rev. 1).
fn rogue_r1_wash() -> Vec<Channel> {
    let mut presets = vec![band('S', 0, 4, "Open")];
    for i in 0..34u8 {
        presets.push(band('S', 5 + 5 * i, 9 + 5 * i, &format!("Color {}", i + 1)));
    }
    presets.extend([
        band('S', 175, 179, "Closed"),
        band('S', 180, 201, "Scroll CW"),
        band('S', 202, 207, "Stop"),
        band('S', 208, 229, "Scroll CCW"),
        band('S', 230, 234, "Closed"),
        band('S', 235, 249, "Color scroll"),
        band('S', 250, 255, "Closed"),
    ]);
    vec![
        v("Pan"),
        v("Pan fine"),
        v("Tilt"),
        v("Tilt fine"),
        v("Pan/Tilt speed"),
        d("Dimmer"),
        s("Strobe", &[
            (0, 19, "Closed"),
            (20, 24, "Open"),
            (25, 64, "Fast to slow"),
            (65, 69, "Open"),
            (70, 84, "Fast on / ramp off"),
            (85, 89, "Open"),
            (90, 104, "Ramp off / fast on"),
            (105, 109, "Open"),
            (110, 124, "Random"),
            (125, 129, "Open"),
            (130, 144, "Random fast on / ramp off"),
            (145, 149, "Open"),
            (150, 164, "Random slow on / fast off"),
            (165, 169, "Open"),
            (170, 184, "Pulse"),
            (185, 189, "Open"),
            (190, 204, "Random pulse"),
            (205, 209, "Open"),
            (210, 224, "Fade on/off"),
            (225, 229, "Open"),
            (230, 244, "Pulse"),
            (245, 255, "Open"),
        ]),
        v("Red"),
        v("Green"),
        v("Blue"),
        v("White"),
        Channel { name: "Color presets".into(), bands: presets, role: None },
        v("Zoom"),
        s("Control", &[
            (0, 9, "None"),
            (10, 14, "Blackout on P/T"),
            (15, 49, "None"),
            (50, 54, "Pan reset"),
            (55, 59, "Tilt reset"),
            (60, 64, "Zoom reset"),
            (65, 69, "None"),
            (70, 74, "Reset all"),
            (75, 79, "None"),
            (80, 84, "Reverse P/T"),
            (85, 89, "Reverse pan"),
            (90, 94, "Reverse tilt"),
            (95, 99, "Normal pan"),
            (100, 104, "Normal tilt"),
            (105, 109, "Normal P/T"),
            (110, 119, "None"),
            (120, 124, "Fan eco"),
            (125, 129, "Fan full"),
            (130, 134, "Fan auto"),
            (135, 139, "Dimmer fast"),
            (140, 144, "Dimmer smooth"),
            (145, 255, "None"),
        ]),
    ]
}

/// Betopper LF2405 250 W matrix strobe (384× RGB, 24× 3535 white spots and
/// 256× 5730 white strips), 11-channel mode (user manual Rev. 1.01). A fixed
/// panel, not a moving head.
fn betopper_lf2405() -> Vec<Channel> {
    vec![
        d("Dimmer"),
        s("Strobe", &[(0, 0, "Off"), (1, 255, "Strobe")]),
        v("Red"),
        v("Green"),
        v("Blue"),
        v("White 3535 (spots)"),
        v("White 5730 (strips)"),
        // Only tints the built-in effects on ch 9; it is not a colour wheel,
        // so keep it out of the colour feature.
        s_role("Auto FX color", Role::Other, &[(0, 31, "Auto"), (32, 255, "Select")]),
        s("Auto programs", &[
            (0, 9, "Off"),
            (10, 230, "Auto FX 1-44"),
            (231, 255, "Sound FX 1-5"),
        ]),
        v("Auto rate"),
        // Which LED groups the built-in effects use (ch 9 at 10–194 only).
        s("Auto FX LED group", &[
            (0, 39, "RGB only"),
            (40, 79, "3535 white only"),
            (80, 119, "5730 white only"),
            (120, 159, "RGB + 5730 white"),
            (160, 199, "RGB + 3535 white"),
            (200, 239, "3535 + 5730 white"),
            (240, 255, "All"),
        ]),
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

/// A profile patched recently, cached with its label and channel count so the
/// Inspector's recents strip renders without loading the fixture library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProfile {
    /// `UserFixture.profile`: a built-in name or `lib:<id>`.
    pub profile: String,
    /// Short human label, e.g. "SlimPAR Q12 BT".
    pub label: String,
    pub channels: u16,
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
    /// Profiles patched most recently, newest first, at most `RECENT_CAP`.
    #[serde(default)]
    pub recent: Vec<RecentProfile>,
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
            recent: Vec::new(),
        }
    }
}

pub fn load_user_patch() -> UserPatch {
    let Ok(text) = std::fs::read_to_string(crate::paths::data_path(USER_PATCH_FILE)) else {
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
        recent: Vec::new(),
    }
}

pub fn save_user_patch(patch: &UserPatch) {
    if let Ok(json) = serde_json::to_string_pretty(patch) {
        let _ = std::fs::write(crate::paths::data_path(USER_PATCH_FILE), json);
    }
}

/// Move `r` to the front of `recent` (matching on `profile`, so a re-patch
/// refreshes the cached label), keeping at most `RECENT_CAP` entries.
pub fn note_recent(recent: &mut Vec<RecentProfile>, r: RecentProfile) {
    recent.retain(|p| p.profile != r.profile);
    recent.insert(0, r);
    recent.truncate(RECENT_CAP);
}
/// Stable identifier for a patched fixture (used by the exclusion list).
pub fn fixture_key(display: &str, from: u16) -> String {
    format!("{display}@{from}")
}

/// Drop excluded ShowBuddy fixtures, then append the user-patched ones.
///
/// A `UserFixture.profile` is either a built-in profile name or a `lib:` id
/// into the fixture library, so both kinds of patched light come back the
/// same way on load.
pub fn extend_patch(patch: &mut Patch, user: &UserPatch, library: &Library) {
    patch
        .fixtures
        .retain(|f| !user.excluded.contains(&fixture_key(&f.display, f.from)));
    for uf in &user.fixtures {
        let fixture = if let Some(id) = fixturedb::library_id(&uf.profile) {
            match library.find(id) {
                Some(entry) => entry.to_fixture(uf.display.clone(), uf.from),
                None => {
                    patch.warnings.push(format!(
                        "'{}': '{}' is not in the fixture library",
                        uf.display, id
                    ));
                    continue;
                }
            }
        } else {
            let Some(profile) = find(&uf.profile) else {
                patch
                    .warnings
                    .push(format!("'{}': unknown profile '{}'", uf.display, uf.profile));
                continue;
            };
            profile.to_fixture(uf.display.clone(), uf.from)
        };
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

    #[test]
    fn note_recent_dedups_and_caps() {
        let rp = |i: usize| RecentProfile { profile: format!("p{i}"), label: format!("P{i}"), channels: 4 };
        let mut r = Vec::new();
        for i in 0..10 {
            note_recent(&mut r, rp(i));
        }
        assert_eq!(r.len(), RECENT_CAP);
        assert_eq!(r[0].profile, "p9");
        note_recent(&mut r, RecentProfile { label: "new".into(), ..rp(5) });
        assert_eq!(r[0].profile, "p5");
        assert_eq!(r[0].label, "new");
        assert_eq!(r.iter().filter(|p| p.profile == "p5").count(), 1);
        assert_eq!(r.len(), RECENT_CAP);
    }

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
                ("Maverick MK2 Spot (24ch)", 24),
                ("Intimidator Spot 475ZX (16ch)", 16),
                ("Intimidator Spot 475Z (16ch)", 16),
                ("Intimidator Spot 375Z IRC (15ch)", 15),
                ("Intimidator Trio (30ch)", 30),
                ("Intimidator Trio (15ch)", 15),
                ("Rogue R1 Wash (14ch)", 14),
                ("SHEHDS Bee Eye 19x40 Ring (31ch)", 31),
                ("25 ch banger (25ch)", 25),
                ("Generic RGBW Par (4ch)", 4),
                ("SlimPAR T12 BT (7ch)", 7),
                ("SlimPAR T6 BT (7ch)", 7),
                ("SlimPAR Q12 BT (8ch)", 8),
                ("Level Q7 IP (7ch)", 7),
                ("Betopper LF2405 Matrix Strobe (11ch)", 11),
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
        let sixteen_bit = [
            "Maverick MK2 Spot (32ch)",
            "Maverick MK2 Spot (24ch)",
            "Intimidator Spot 475ZX (16ch)",
            "Intimidator Spot 475Z (16ch)",
            "Intimidator Spot 375Z IRC (15ch)",
            "Intimidator Trio (30ch)",
            "Rogue R1 Wash (14ch)",
            "SHEHDS Bee Eye 19x40 Ring (31ch)",
        ];
        for p in sixteen_bit.map(|n| find(n).unwrap()) {
            assert_eq!(role_at(p, Role::Pan), Some(0), "{}", p.name);
            assert_eq!(role_at(p, Role::PanFine), Some(1), "{}", p.name);
            assert_eq!(role_at(p, Role::Tilt), Some(2), "{}", p.name);
            assert_eq!(role_at(p, Role::TiltFine), Some(3), "{}", p.name);
            assert_eq!(role_at(p, Role::Speed), Some(4), "{}", p.name);
        }
        let maverick = find("Maverick MK2 Spot (32ch)").unwrap();
        assert_eq!(role_at(maverick, Role::Dimmer), Some(5));
        // Ch 8 is "Shutter" (Role::Other); the strobe role is ch 9's
        // virtual strobe, which does the actual visible strobing.
        assert_eq!(role_at(maverick, Role::Strobe), Some(8));
        assert_eq!(role_at(maverick, Role::Color), Some(13));
        assert_eq!(role_at(maverick, Role::Zoom), Some(23));
        let zx = find("Intimidator Spot 475ZX (16ch)").unwrap();
        assert_eq!(role_at(zx, Role::Color), Some(5));
        assert_eq!(role_at(zx, Role::Zoom), Some(11));
        assert_eq!(role_at(zx, Role::Dimmer), Some(12));
        assert_eq!(role_at(zx, Role::Strobe), Some(13));
        let trio = find("Intimidator Trio (30ch)").unwrap();
        assert_eq!(role_at(trio, Role::Red), Some(5));
        assert_eq!(role_at(trio, Role::Green), Some(6));
        assert_eq!(role_at(trio, Role::Blue), Some(7));
        assert_eq!(role_at(trio, Role::White), Some(8));
        assert_eq!(role_at(trio, Role::Dimmer), Some(24));
        assert_eq!(role_at(trio, Role::Zoom), Some(26));
        let bee = find("SHEHDS Bee Eye 19x40 Ring (31ch)").unwrap();
        assert_eq!(role_at(bee, Role::Zoom), Some(5));
        assert_eq!(role_at(bee, Role::Dimmer), Some(7));
        assert_eq!(role_at(bee, Role::Strobe), Some(8));
        assert_eq!(role_at(bee, Role::Red), Some(9));
        assert_eq!(role_at(bee, Role::White), Some(12));
        let banger = find("25 ch banger (25ch)").unwrap();
        assert_eq!(role_at(banger, Role::Pan), Some(0));
        assert_eq!(role_at(banger, Role::Tilt), Some(2));
        assert_eq!(role_at(banger, Role::Dimmer), Some(6));
        assert_eq!(role_at(banger, Role::Strobe), Some(7));
        assert_eq!(role_at(banger, Role::Red), Some(8));
        let par = find("Generic RGBW Par (4ch)").unwrap();
        assert_eq!(role_at(par, Role::Red), Some(0));
        assert_eq!(role_at(par, Role::White), Some(3));
    }

    /// Channel-by-channel role positions for the hand-transcribed
    /// Chauvet/Betopper modes, straight from their DMX charts.
    #[test]
    fn transcribed_modes_key_roles() {
        let role_at = |name: &str, role: Role| {
            find(name).unwrap().channels().iter().position(|c| c.role() == role)
        };
        let mav = "Maverick MK2 Spot (24ch)";
        assert_eq!(role_at(mav, Role::Dimmer), Some(5));
        assert_eq!(role_at(mav, Role::Shutter), Some(6));
        assert_eq!(role_at(mav, Role::Strobe), Some(7));
        assert_eq!(role_at(mav, Role::Cyan), Some(8));
        assert_eq!(role_at(mav, Role::Magenta), Some(9));
        assert_eq!(role_at(mav, Role::Yellow), Some(10));
        assert_eq!(role_at(mav, Role::Color), Some(12));
        assert_eq!(role_at(mav, Role::Gobo), Some(13));
        assert_eq!(role_at(mav, Role::Focus), Some(17));
        assert_eq!(role_at(mav, Role::Zoom), Some(18));
        assert_eq!(role_at(mav, Role::Prism), Some(19));
        assert_eq!(role_at(mav, Role::Iris), Some(21));
        assert_eq!(role_at(mav, Role::Frost), Some(22));

        let z = "Intimidator Spot 475Z (16ch)";
        assert_eq!(role_at(z, Role::Color), Some(5));
        assert_eq!(role_at(z, Role::Gobo), Some(6));
        assert_eq!(role_at(z, Role::Prism), Some(9));
        assert_eq!(role_at(z, Role::Focus), Some(10));
        assert_eq!(role_at(z, Role::Zoom), Some(11));
        assert_eq!(role_at(z, Role::Dimmer), Some(12));
        assert_eq!(role_at(z, Role::Strobe), Some(13));

        let z375 = "Intimidator Spot 375Z IRC (15ch)";
        assert_eq!(role_at(z375, Role::Color), Some(5));
        assert_eq!(role_at(z375, Role::Gobo), Some(6));
        assert_eq!(role_at(z375, Role::Prism), Some(8));
        assert_eq!(role_at(z375, Role::Focus), Some(9));
        assert_eq!(role_at(z375, Role::Dimmer), Some(10));
        assert_eq!(role_at(z375, Role::Strobe), Some(11));
        assert_eq!(role_at(z375, Role::Zoom), Some(14));

        let trio = "Intimidator Trio (15ch)";
        assert_eq!(role_at(trio, Role::Pan), Some(0));
        assert_eq!(role_at(trio, Role::Tilt), Some(1));
        assert_eq!(role_at(trio, Role::Speed), Some(2));
        assert_eq!(role_at(trio, Role::Red), Some(3));
        assert_eq!(role_at(trio, Role::Green), Some(4));
        assert_eq!(role_at(trio, Role::Blue), Some(5));
        assert_eq!(role_at(trio, Role::White), Some(6));
        assert_eq!(role_at(trio, Role::Dimmer), Some(9));
        assert_eq!(role_at(trio, Role::Strobe), Some(10));
        assert_eq!(role_at(trio, Role::Zoom), Some(11));
        assert_eq!(role_at(trio, Role::PanFine), None);

        let rogue = "Rogue R1 Wash (14ch)";
        assert_eq!(role_at(rogue, Role::Dimmer), Some(5));
        assert_eq!(role_at(rogue, Role::Strobe), Some(6));
        assert_eq!(role_at(rogue, Role::Red), Some(7));
        assert_eq!(role_at(rogue, Role::Green), Some(8));
        assert_eq!(role_at(rogue, Role::Blue), Some(9));
        assert_eq!(role_at(rogue, Role::White), Some(10));
        assert_eq!(role_at(rogue, Role::Color), Some(11));
        assert_eq!(role_at(rogue, Role::Zoom), Some(12));

        let lf = "Betopper LF2405 Matrix Strobe (11ch)";
        assert_eq!(role_at(lf, Role::Dimmer), Some(0));
        assert_eq!(role_at(lf, Role::Strobe), Some(1));
        assert_eq!(role_at(lf, Role::Red), Some(2));
        assert_eq!(role_at(lf, Role::Green), Some(3));
        assert_eq!(role_at(lf, Role::Blue), Some(4));
        assert_eq!(role_at(lf, Role::White), Some(5));
        let lf_roles: Vec<Role> = find(lf).unwrap().channels().iter().map(|c| c.role()).collect();
        assert_eq!(lf_roles[6], Role::White);
        // The effect-only selectors must not masquerade as colour controls.
        assert_eq!(&lf_roles[7..], [Role::Other, Role::Other, Role::Other, Role::Other]);
        assert_eq!(role_at(lf, Role::Pan), None);
    }

    /// The 24-channel Maverick is the 32-channel chart with exactly the
    /// documented eight channels removed, order preserved.
    #[test]
    fn maverick_24ch_matches_chart_order() {
        let names: Vec<String> = find("Maverick MK2 Spot (24ch)")
            .unwrap()
            .channels()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(
            names,
            [
                "Pan", "Pan fine", "Tilt", "Tilt fine", "Pan/Tilt speed", "Dimmer",
                "Shutter", "Virtual strobe", "Cyan", "Magenta", "Yellow (CMY)", "CTO",
                "Color wheel", "Gobo wheel 1", "Gobo rotating 1", "Gobo wheel 2",
                "Gobo rotating 2", "Focus", "Zoom", "Prism", "Prism rotation", "Iris",
                "Frost", "Control",
            ]
        );
    }

    /// Every stepped channel's bands must tile 0–255 with no gaps or
    /// overlaps — the cheapest check that a chart was transcribed cleanly.
    #[test]
    fn stepped_bands_tile_the_dmx_range() {
        for p in PROFILES {
            for ch in p.channels() {
                if !ch.bands.iter().all(|b| b.kind == 'S') {
                    continue;
                }
                let mut next = 0u16;
                for b in &ch.bands {
                    assert_eq!(b.min as u16, next, "{} / {}: gap before {}", p.name, ch.name, b.min);
                    assert!(b.max >= b.min, "{} / {}: inverted band", p.name, ch.name);
                    next = b.max as u16 + 1;
                }
                assert_eq!(next, 256, "{} / {}: bands stop at {}", p.name, ch.name, next - 1);
            }
        }
    }

    #[test]
    fn patched_addresses_span_channels() {
        let p = find("Generic RGBW Par (4ch)").unwrap();
        let f = p.to_fixture("Par 1".into(), 101);
        assert_eq!((f.from, f.to), (101, 104));
        assert_eq!(f.channel_count(), 4);
    }
}

/// Where each old fixture index has moved to in a rebuilt patch.
///
/// Groups, orders and layer targets all hold raw indices into
/// `patch.fixtures`, and patching one light in the middle renumbers
/// everything above it. `before` is [`fixture_key`] for each fixture at its
/// old index; the result maps old index → new index for every light that is
/// still patched, and simply omits the ones that are not.
///
/// A light is matched on its full key first, then on its display name alone,
/// so a fixture that was only readdressed keeps everything pointing at it.
pub fn index_remap(
    before: &[String],
    now: &[Fixture],
) -> std::collections::HashMap<usize, usize> {
    use std::collections::HashMap;
    let mut keyed: HashMap<&str, usize> = HashMap::new();
    let mut named: HashMap<&str, usize> = HashMap::new();
    let keys: Vec<String> = now
        .iter()
        .map(|f| fixture_key(&f.display, f.from))
        .collect();
    // First occurrence wins, so two lights sharing a display name remap
    // deterministically instead of depending on iteration order.
    for (i, f) in now.iter().enumerate() {
        keyed.entry(keys[i].as_str()).or_insert(i);
        named.entry(f.display.as_str()).or_insert(i);
    }
    let mut out = HashMap::new();
    for (old, key) in before.iter().enumerate() {
        let name = key.rsplit_once('@').map_or(key.as_str(), |(n, _)| n);
        if let Some(&i) = keyed.get(key.as_str()).or_else(|| named.get(name)) {
            out.insert(old, i);
        }
    }
    out
}

#[cfg(test)]
mod remap_tests {
    use super::*;

    fn fix(display: &str, from: u16) -> Fixture {
        Fixture {
            display: display.into(),
            file: std::path::PathBuf::new(),
            from,
            to: from,
            x: 0.0,
            y: 0.0,
            pan_range: 0.0,
            tilt_range: 0.0,
            beam_width: 0.0,
            channels: Vec::new(),
        }
    }

    fn keys(fs: &[Fixture]) -> Vec<String> {
        fs.iter().map(|f| fixture_key(&f.display, f.from)).collect()
    }

    /// Patching a light into the middle renumbers everything above it. Every
    /// group and route holding raw indices has to move with them, or it
    /// silently repoints at its neighbours.
    #[test]
    fn inserting_a_fixture_shifts_the_ones_above_it() {
        let before = keys(&[fix("a", 1), fix("b", 2), fix("c", 3)]);
        let now = vec![fix("a", 1), fix("new", 9), fix("b", 2), fix("c", 3)];
        let map = index_remap(&before, &now);
        assert_eq!(map.get(&0), Some(&0));
        assert_eq!(map.get(&1), Some(&2), "b moved up one");
        assert_eq!(map.get(&2), Some(&3), "c moved up one");
    }

    /// A light that only changed address is still the same light.
    #[test]
    fn a_readdressed_fixture_is_still_found() {
        let before = keys(&[fix("a", 1), fix("b", 2)]);
        let now = vec![fix("a", 1), fix("b", 77)];
        assert_eq!(index_remap(&before, &now).get(&1), Some(&1));
    }

    /// An unpatched light is omitted, so callers drop it rather than keep an
    /// index pointing at a stranger.
    #[test]
    fn an_unpatched_fixture_is_dropped_not_repointed() {
        let before = keys(&[fix("a", 1), fix("gone", 2), fix("c", 3)]);
        let now = vec![fix("a", 1), fix("c", 3)];
        let map = index_remap(&before, &now);
        assert_eq!(map.get(&0), Some(&0));
        assert_eq!(map.get(&1), None, "the unpatched light has no new home");
        assert_eq!(map.get(&2), Some(&1));
    }

    /// Two lights sharing a display name must not remap by luck.
    #[test]
    fn duplicate_names_remap_deterministically() {
        let before = keys(&[fix("par", 1), fix("par", 5)]);
        let now = vec![fix("par", 1), fix("par", 5)];
        let map = index_remap(&before, &now);
        assert_eq!(map.get(&0), Some(&0), "exact key wins over the name");
        assert_eq!(map.get(&1), Some(&1));
    }

    /// Unpatching everything leaves nothing to remap onto — callers must not
    /// treat that as "delete the pools".
    #[test]
    fn an_empty_patch_maps_nothing() {
        let before = keys(&[fix("a", 1)]);
        assert!(index_remap(&before, &[]).is_empty());
    }
}
