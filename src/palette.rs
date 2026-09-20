//! Palettes — DMXpress's renamed grandMA3 *referenced presets*.
//!
//! Where a ShowBuddy `.prt` is a whole-rig snapshot, a palette stores just one
//! *feature* (Color, Position, …) for the fixtures you had selected. Recalling
//! a palette drops those values into the programmer and records a *reference*
//! back to the palette, so a cue can remember "Color Palette 3" instead of raw
//! RGB — edit the palette and everything that points at it follows.

use serde::{Deserialize, Serialize};

use crate::showbuddy::{Fixture, Role};

const PALETTES_FILE: &str = "palettes.json";

/// HSV → RGB, hue in degrees. Shared by the preset generators and the deck
/// artwork so a "rainbow" means the same thing everywhere.
pub fn hsv(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i as i32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

/// The attribute family a palette (or channel) belongs to. Channels are sorted
/// into exactly one feature so palettes stay focused (a Color palette never
/// disturbs Position, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Feature {
    Dimmer,
    Position,
    Color,
    Beam,
    Focus,
    Control,
}

impl Feature {
    /// Pool order shown in the UI.
    pub const ALL: [Feature; 6] = [
        Feature::Dimmer,
        Feature::Position,
        Feature::Color,
        Feature::Beam,
        Feature::Focus,
        Feature::Control,
    ];

    /// Which feature a channel role contributes to.
    pub fn of(role: Role) -> Feature {
        match role {
            Role::Dimmer => Feature::Dimmer,
            Role::Red
            | Role::Green
            | Role::Blue
            | Role::White
            | Role::Amber
            | Role::Uv
            | Role::Cyan
            | Role::Magenta
            | Role::Yellow
            | Role::Color => Feature::Color,
            Role::Pan | Role::PanFine | Role::Tilt | Role::TiltFine => Feature::Position,
            Role::Zoom | Role::Focus | Role::Iris => Feature::Focus,
            Role::Strobe | Role::Shutter | Role::Gobo | Role::Prism | Role::Frost => Feature::Beam,
            Role::Speed | Role::Other => Feature::Control,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Feature::Dimmer => "Dimmer",
            Feature::Position => "Position",
            Feature::Color => "Color",
            Feature::Beam => "Beam",
            Feature::Focus => "Focus",
            Feature::Control => "Control",
        }
    }

    /// Compact form for tight controls; always paired with a tooltip.
    pub fn short(self) -> &'static str {
        match self {
            Feature::Dimmer => "Dim",
            Feature::Position => "Pos",
            Feature::Color => "Col",
            Feature::Beam => "Beam",
            Feature::Focus => "Foc",
            Feature::Control => "Ctrl",
        }
    }
}

/// A stable pointer to a palette: the channels a cue or the programmer link to.
/// Carries the feature so the resolver can fall back gracefully if the palette
/// was deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteRef {
    pub feature: Feature,
    pub id: u32,
}

/// A named, feature-scoped set of channel values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Palette {
    /// Stable id (never reused) so references survive reordering/deletion.
    pub id: u32,
    pub feature: Feature,
    pub name: String,
    /// 0-based DMX index → value. Only channels belonging to `feature`.
    pub values: Vec<(usize, u8)>,
}

impl Palette {
    pub fn reference(&self) -> PaletteRef {
        PaletteRef {
            feature: self.feature,
            id: self.id,
        }
    }
}

/// Load saved palettes (empty if the file is missing or unreadable).
pub fn load_palettes() -> Vec<Palette> {
    std::fs::read_to_string(PALETTES_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist palettes to disk (best-effort).
pub fn save_palettes(palettes: &[Palette]) {
    if let Ok(json) = serde_json::to_string_pretty(palettes) {
        let _ = std::fs::write(PALETTES_FILE, json);
    }
}

const SEQUENCES_FILE: &str = "sequences.json";

/// How a palette sequence disperses the colour change across the rig.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SeqPattern {
    /// The change rolls across the fixtures in patch order.
    #[default]
    Wave,
    /// The change sweeps from both ends toward the middle.
    Wings,
    /// Each fixture gets a random offset (shimmering dispersal).
    Random,
}

impl SeqPattern {
    pub const ALL: [SeqPattern; 3] = [SeqPattern::Wave, SeqPattern::Wings, SeqPattern::Random];

    pub fn label(self) -> &'static str {
        match self {
            SeqPattern::Wave => "Wave",
            SeqPattern::Wings => "Wings",
            SeqPattern::Random => "Random",
        }
    }
}

/// A saved palette sequence: the colours plus how the cycle moves — speed,
/// spacing across the rig, dispersal pattern, and the shape of each change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaletteSeq {
    pub name: String,
    /// Folder name; empty = root of the pool.
    #[serde(default)]
    pub folder: String,
    /// Palette ids, in cycle order.
    pub ids: Vec<u32>,
    /// Relative width/dwell of each id. Missing entries default to 1.0 for
    /// backward compatibility. Repeated ids are intentional separate bands.
    #[serde(default)]
    pub weights: Vec<f32>,
    /// Beats per colour step.
    pub beats_per: f32,
    /// Follow taps and the master BPM; off keeps the launch-time tempo.
    #[serde(default = "default_true")]
    pub master_beat: bool,
    /// Phase fanned across the fixtures (0 = all together, 1 = the whole
    /// cycle spread over the rig).
    #[serde(default)]
    pub spread: f32,
    #[serde(default)]
    pub pattern: SeqPattern,
    /// Shape of each change: 0 = smooth crossfade, 1 = hard snap.
    #[serde(default)]
    pub shape: f32,
}

fn default_true() -> bool {
    true
}

/// Saved sequences plus their folder list.
#[derive(Default, Serialize, Deserialize)]
pub struct SeqStore {
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub seqs: Vec<PaletteSeq>,
}

/// Load saved palette sequences (empty if the file is missing/unreadable).
pub fn load_seqs() -> SeqStore {
    std::fs::read_to_string(SEQUENCES_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist palette sequences to disk (best-effort).
pub fn save_seqs(folders: &[String], seqs: &[PaletteSeq]) {
    let store = SeqStore {
        folders: folders.to_vec(),
        seqs: seqs.to_vec(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&store) {
        let _ = std::fs::write(SEQUENCES_FILE, json);
    }
}

/// Re-aim a palette's stored values at a set of effect units, matching
/// channels by what they *do* rather than by the addresses the palette
/// happens to hold.
///
/// A palette is a flat list of absolute DMX addresses captured from whatever
/// was selected when it was stored, so recalling one onto different lights
/// used to be meaningless. Reading each stored address back to the role of
/// the channel it lands on turns it into a template — "Red 255, Blue 40" —
/// that any fixture with those channels can answer, whatever its patch
/// address or channel order.
///
/// The palette's *shape* across the rig survives by resampling: the fixtures
/// it was stored from are laid along `units`, so a palette stored as a
/// gradient stays a gradient over more or fewer lights. Because a group
/// folded into one order step is a single unit, every light in it reads the
/// same source and lands on the same colour — which is what makes a palette
/// obey a group at all.
///
/// Values stored at addresses no patched fixture owns cannot be translated,
/// so they are returned untouched.
pub(crate) fn fan(
    values: &[(usize, u8)],
    fixtures: &[Fixture],
    units: &[Vec<usize>],
) -> Vec<(usize, u8)> {
    // What the palette says, per fixture it was actually stored from.
    let mut src: Vec<Vec<(Role, &str, u8)>> = Vec::new();
    for f in fixtures {
        let from0 = f.from as usize - 1;
        let mine: Vec<(Role, &str, u8)> = f
            .channels
            .iter()
            .enumerate()
            .filter_map(|(ci, ch)| {
                values
                    .iter()
                    .find(|(pa, _)| *pa == from0 + ci)
                    .map(|&(_, v)| (ch.role(), ch.name.as_str(), v))
            })
            .collect();
        if !mine.is_empty() {
            src.push(mine);
        }
    }
    if src.is_empty() || units.is_empty() {
        return values.to_vec();
    }
    let (n, m) = (src.len(), units.len());
    let mut out: Vec<(usize, u8)> = Vec::new();
    for (k, unit) in units.iter().enumerate() {
        let from = &src[(k * n / m).min(n - 1)];
        for &fi in unit {
            let Some(f) = fixtures.get(fi) else { continue };
            let from0 = f.from as usize - 1;
            for (ci, ch) in f.channels.iter().enumerate() {
                let role = ch.role();
                // Role-less channels (a fogger's "Fan") match by name, the
                // same rule the phaser targets use.
                let hit = from.iter().find(|(r, nm, _)| {
                    if role.tag().is_empty() {
                        nm.eq_ignore_ascii_case(&ch.name)
                    } else {
                        *r == role
                    }
                });
                if let Some(&(_, _, v)) = hit {
                    let a = from0 + ci;
                    if a < crate::net::DMX_SLOTS {
                        out.push((a, v));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod fan_tests {
    use super::*;
    use crate::showbuddy::Channel;

    fn ch(name: &str, role: Role) -> Channel {
        Channel { name: name.into(), bands: Vec::new(), role: Some(role) }
    }

    /// An RGB light at `from`, three channels: Red, Green, Blue.
    fn rgb(display: &str, from: u16) -> Fixture {
        Fixture {
            display: display.into(),
            file: std::path::PathBuf::new(),
            from,
            to: from + 2,
            x: 0.0,
            y: 0.0,
            pan_range: 0.0,
            tilt_range: 0.0,
            beam_width: 0.0,
            channels: vec![ch("Red", Role::Red), ch("Green", Role::Green), ch("Blue", Role::Blue)],
        }
    }

    /// Same colours, but patched with the channels in a different order and
    /// at a different address — the case raw addresses cannot survive.
    fn bgr(display: &str, from: u16) -> Fixture {
        Fixture {
            channels: vec![ch("Blue", Role::Blue), ch("Green", Role::Green), ch("Red", Role::Red)],
            ..rgb(display, from)
        }
    }

    /// The headline fix: every light in one order step reads the same source
    /// fixture, so a group gets one colour instead of a gradient.
    #[test]
    fn a_folded_step_gives_every_light_in_it_the_same_colour() {
        // Stored from two lights: red, then green.
        let fixtures = vec![rgb("a", 1), rgb("b", 4), rgb("c", 7), rgb("d", 10)];
        let values = vec![(0, 255), (1, 0), (2, 0), (3, 0), (4, 255), (5, 0)];
        // Two units, the second folding c and d together.
        let units = vec![vec![0], vec![2, 3]];
        let out = fan(&values, &fixtures, &units);
        let at = |a: usize| out.iter().find(|(x, _)| *x == a).map(|&(_, v)| v);
        // Unit 0 takes source 0 (red).
        assert_eq!((at(0), at(1)), (Some(255), Some(0)));
        // Unit 1 takes source 1 (green) — and BOTH its lights get it.
        assert_eq!((at(6), at(7)), (Some(0), Some(255)));
        assert_eq!((at(9), at(10)), (Some(0), Some(255)));
    }

    /// Channels are matched by role, so a differently-ordered fixture at a
    /// different address still lands on the right colours.
    #[test]
    fn colours_follow_the_role_not_the_address() {
        let fixtures = vec![rgb("src", 1), bgr("dst", 4)];
        // Source: Red 200, Blue 50.
        let values = vec![(0, 200), (1, 0), (2, 50)];
        let out = fan(&values, &fixtures, &[vec![1]]);
        let at = |a: usize| out.iter().find(|(x, _)| *x == a).map(|&(_, v)| v);
        // dst is Blue, Green, Red at 4..6 → 0-based 3, 4, 5.
        assert_eq!(at(3), Some(50), "blue went to the blue channel");
        assert_eq!(at(4), Some(0));
        assert_eq!(at(5), Some(200), "red went to the red channel");
    }

    /// A one-fixture palette paints every target the same, however many.
    #[test]
    fn one_source_colours_the_whole_selection() {
        let fixtures = vec![rgb("a", 1), rgb("b", 4), rgb("c", 7)];
        let values = vec![(0, 10), (1, 20), (2, 30)];
        let out = fan(&values, &fixtures, &[vec![1], vec![2]]);
        let at = |a: usize| out.iter().find(|(x, _)| *x == a).map(|&(_, v)| v);
        assert_eq!((at(3), at(4), at(5)), (Some(10), Some(20), Some(30)));
        assert_eq!((at(6), at(7), at(8)), (Some(10), Some(20), Some(30)));
    }

    /// Values at addresses nothing owns cannot be translated, so they are
    /// handed back untouched rather than silently dropped.
    #[test]
    fn untranslatable_values_pass_straight_through() {
        let values = vec![(900, 5)];
        assert_eq!(fan(&values, &[rgb("a", 1)], &[vec![0]]), values);
    }
}
