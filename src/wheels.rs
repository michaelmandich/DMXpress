//! Wheel and effect slots as deck tiles.
//!
//! An *island* is one effect across the rig — Gobo, Prism, Spinner, Light
//! ring, Macro, Auto, Fog… — found by looking at every fixture's channels,
//! merged across fixture types by name so "sun" on the mini beam and "sun"
//! on the 150 W spot are one tile. A stepped channel contributes one slot
//! per named band (holding the band's midpoint); a continuous effect
//! channel (a light ring at 0–255) is cut into off / low / mid / full.
//!
//! A slot stands for a palette ("Gobo: sun", "Light ring: full") that is
//! created the first time it is picked, holding that value on every light
//! that has it — so the palette cycle steps through effects exactly as it
//! steps through colours, and, these being stepped channels, every change
//! snaps.

use crate::app::App;
use crate::net;
use crate::palette::{self, Feature, Palette};
use crate::showbuddy::{Channel, Fixture, Role};
use crate::streamdeck::PaletteSub;

/// One named position shared by every wheel in the rig that has it.
pub(crate) struct WheelSlot {
    /// Display name — the band label as the first fixture spells it.
    pub label: String,
    /// 0-based DMX address → value, one per light with this slot.
    pub values: Vec<(usize, u8)>,
}

/// One effect across the rig and its slots.
pub(crate) struct Island {
    pub name: String,
    pub slots: Vec<WheelSlot>,
}

/// The palette a slot stands for.
pub(crate) fn slot_palette_name(island: &str, label: &str) -> String {
    format!("{island}: {}", label.trim().to_lowercase())
}

/// How a continuous effect channel is cut up.
pub(crate) const LEVELS: [(&str, u8); 4] = [("off", 0), ("low", 85), ("mid", 170), ("full", 255)];

/// Name fragments that mark a channel as housekeeping, not an effect.
const NOT_EFFECTS: [&str; 12] = [
    "strb", "stb", "strobe", "reset", "rst", "sound", "snd", "speed", "spd", "fine", "master", "dim",
];

fn title_case(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The effect island a channel belongs to, by role and name — prisms and
/// frost by role; spinners, light rings, macros, auto programs, fog and
/// circle effects by name; any other unclassified channel under its own
/// name. `None` for everything that isn't an effect.
pub(crate) fn effect_island(ch: &Channel) -> Option<String> {
    let n = ch.name.trim().to_lowercase();
    match ch.role() {
        Role::Prism => return Some("Prism".into()),
        Role::Frost => return Some("Frost".into()),
        Role::Other => {}
        _ => return None,
    }
    if n.is_empty() || NOT_EFFECTS.iter().any(|k| n.contains(k)) {
        return None;
    }
    let name = if n.contains("spin") {
        "Spinner"
    } else if n.contains("ring") {
        "Light ring"
    } else if n.contains("macro") {
        "Macro"
    } else if n.contains("auto") {
        "Auto"
    } else if n.contains("fog") || n.contains("smoke") || n.contains("haze") {
        "Fog"
    } else if n.contains("circ") {
        "Circle"
    } else {
        return Some(title_case(ch.name.trim()));
    };
    Some(name.to_string())
}

/// A channel's slots: its named bands with their midpoints, or — for a
/// continuous channel, when `levels` is on — the four levels. A lone
/// full-range band is a continuous channel wearing a label, not a slot.
fn channel_slots(ch: &Channel, levels: bool) -> Vec<(String, u8)> {
    let stepped: Vec<(String, u8)> = ch
        .bands
        .iter()
        .filter(|b| b.kind == 'S' && !b.label.trim().is_empty() && !(b.min == 0 && b.max == 255))
        .map(|b| (b.label.trim().to_string(), ((b.min as u16 + b.max as u16) / 2) as u8))
        .collect();
    if !stepped.is_empty() || !levels {
        return stepped;
    }
    LEVELS.iter().map(|(l, v)| ((*l).to_string(), *v)).collect()
}

/// Every island `pick` names across `fixtures`, in patch order, slots
/// merged by name (case-insensitive) within an island.
pub(crate) fn islands_in(
    fixtures: &[Fixture],
    levels: bool,
    pick: impl Fn(&Channel) -> Option<String>,
) -> Vec<Island> {
    let mut out: Vec<Island> = Vec::new();
    for f in fixtures {
        let from0 = (f.from as usize).saturating_sub(1);
        for (ci, ch) in f.channels.iter().enumerate() {
            let Some(name) = pick(ch) else {
                continue;
            };
            let a = from0 + ci;
            if a >= net::DMX_SLOTS {
                continue;
            }
            let slots = channel_slots(ch, levels);
            if slots.is_empty() {
                continue;
            }
            let idx = match out.iter().position(|i| i.name.eq_ignore_ascii_case(&name)) {
                Some(i) => i,
                None => {
                    out.push(Island { name: name.clone(), slots: Vec::new() });
                    out.len() - 1
                }
            };
            let island = &mut out[idx];
            for (label, v) in slots {
                match island.slots.iter_mut().find(|s| s.label.eq_ignore_ascii_case(&label)) {
                    Some(s) => {
                        if !s.values.iter().any(|(sa, _)| *sa == a) {
                            s.values.push((a, v));
                        }
                    }
                    None => island.slots.push(WheelSlot { label, values: vec![(a, v)] }),
                }
            }
        }
    }
    out
}

/// The gobo wheels, as one island.
pub(crate) fn gobo_islands(fixtures: &[Fixture]) -> Vec<Island> {
    islands_in(fixtures, false, |ch| (ch.role() == Role::Gobo).then(|| "Gobo".to_string()))
}

/// Every other effect, one island each.
pub(crate) fn effect_islands(fixtures: &[Fixture]) -> Vec<Island> {
    islands_in(fixtures, true, effect_island)
}

impl App {
    /// The islands a Palettes sub-page lists.
    pub(crate) fn wheel_islands(&self, sub: PaletteSub) -> Vec<Island> {
        match sub {
            PaletteSub::Colors => Vec::new(),
            PaletteSub::Gobos => gobo_islands(&self.patch.fixtures),
            PaletteSub::Effects(_) => effect_islands(&self.patch.fixtures),
        }
    }

    /// The palette id a slot stands for, once it has been picked.
    pub(crate) fn slot_palette_id(&self, island: &str, label: &str) -> Option<u32> {
        let name = slot_palette_name(island, label);
        self.palettes.iter().find(|p| p.name == name).map(|p| p.id)
    }

    /// The slot's palette: created on first pick, and refreshed to the
    /// rig's current channels on every pick so a repatch doesn't strand it.
    pub(crate) fn ensure_slot_palette(&mut self, island: &str, slot: &WheelSlot) -> u32 {
        let name = slot_palette_name(island, &slot.label);
        if let Some(p) = self.palettes.iter_mut().find(|p| p.name == name) {
            p.values = slot.values.clone();
            let id = p.id;
            palette::save_palettes(&self.palettes);
            return id;
        }
        // Filed under the feature its channels belong to (Beam for prisms,
        // Control for the rest), so it shows on that tab of the Palettes window.
        let feature = slot
            .values
            .first()
            .and_then(|(a, _)| self.role_at(*a))
            .map_or(Feature::Control, Feature::of);
        let id = self.next_palette_id;
        self.next_palette_id += 1;
        self.palettes.push(Palette {
            id,
            feature,
            name: name.clone(),
            values: slot.values.clone(),
        });
        palette::save_palettes(&self.palettes);
        self.log
            .push(format!("Stored palette \"{name}\" ({} ch)", slot.values.len()));
        id
    }

    /// Stored palettes that set any channel these islands cover and aren't
    /// a slot's own — the ones you built yourself, e.g. different gobos on
    /// different lights.
    pub(crate) fn wheel_palettes(&self, islands: &[Island]) -> Vec<u32> {
        let auto: Vec<String> = islands
            .iter()
            .flat_map(|i| i.slots.iter().map(move |s| slot_palette_name(&i.name, &s.label)))
            .collect();
        let addrs: std::collections::HashSet<usize> = islands
            .iter()
            .flat_map(|i| i.slots.iter().flat_map(|s| s.values.iter().map(|(a, _)| *a)))
            .collect();
        self.palettes
            .iter()
            .filter(|p| !auto.contains(&p.name) && p.values.iter().any(|(a, _)| addrs.contains(a)))
            .map(|p| p.id)
            .collect()
    }

    /// Put a palette into an island's lane, or take it out again. An empty
    /// lane is dropped so the readouts stay honest.
    pub(crate) fn toggle_lane_palette(&mut self, island: &str, id: u32) {
        let name = self
            .palettes
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| format!("#{id}"));
        let lane = self.effect_lanes.entry(island.to_string()).or_default();
        let on = match lane.iter().position(|x| *x == id) {
            Some(pos) => {
                lane.remove(pos);
                false
            }
            None => {
                lane.push(id);
                true
            }
        };
        let count = lane.len();
        if count == 0 {
            self.effect_lanes.remove(island);
        }
        self.log.push(match (on, count) {
            (true, 1) => format!("{island}: holding \"{name}\""),
            (true, n) => format!("{island}: \"{name}\" joins the lane ({n} stepping)"),
            (false, 0) => format!("{island}: lane cleared"),
            (false, 1) => format!("{island}: \"{name}\" dropped — holding the one left"),
            (false, n) => format!("{island}: \"{name}\" dropped ({n} stepping)"),
        });
    }

    /// Whether an island's lane holds this palette.
    pub(crate) fn lane_has(&self, island: &str, id: u32) -> bool {
        self.effect_lanes.get(island).is_some_and(|l| l.contains(&id))
    }

    /// Whether any lane has enough picks to be stepping.
    pub(crate) fn lanes_cycling(&self) -> bool {
        self.effect_lanes.values().any(|l| l.len() >= 2)
    }

    /// Put a palette into the cycle, or take it out again if it's already
    /// there. The deck's colour keys share this with the Palettes window.
    pub(crate) fn toggle_cycle_palette(&mut self, id: u32) {
        if let Some(pos) = self.cycle_ids.iter().position(|x| *x == id) {
            self.cycle_ids.remove(pos);
            if pos < self.cycle_weights.len() {
                self.cycle_weights.remove(pos);
            }
        } else {
            self.cycle_ids.push(id);
            self.cycle_weights.push(1.0);
        }
        self.cycle_seq = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::showbuddy::Band;

    fn chan(name: &str, bands: &[(char, u8, u8, &str)]) -> Channel {
        Channel {
            name: name.into(),
            bands: bands
                .iter()
                .map(|(k, lo, hi, l)| Band { kind: *k, min: *lo, max: *hi, label: (*l).into(), gobo: None })
                .collect(),
            role: None,
        }
    }

    /// Effects are picked by role and name; housekeeping channels are not.
    #[test]
    fn effect_islands_are_named_from_channels() {
        let island = |n: &str| effect_island(&chan(n, &[('V', 0, 255, "")]));
        assert_eq!(island("Prism").as_deref(), Some("Prism"));
        assert_eq!(island("SPINNER").as_deref(), Some("Spinner"));
        assert_eq!(island("LRING").as_deref(), Some("Light ring"));
        assert_eq!(island("Macro").as_deref(), Some("Macro"));
        assert_eq!(island("Auto run").as_deref(), Some("Auto"));
        assert_eq!(island("Fog").as_deref(), Some("Fog"));
        assert_eq!(island("Circle effect").as_deref(), Some("Circle"));
        assert_eq!(island("Staining").as_deref(), Some("Staining"));
        for housekeeping in ["Reset", "RST", "Sound control", "SndMd", "Auto speed", "Movement speed", "Dimmer", "Red", "Pan", "Gobo", "Strobe", "cirstbOI"] {
            assert_eq!(island(housekeeping), None, "{housekeeping} is not an effect");
        }
    }

    /// A continuous effect channel is cut into levels; a stepped one keeps
    /// its bands; a gobo channel never gets levels.
    #[test]
    fn continuous_channels_get_levels() {
        let ring = chan("LRING", &[('V', 0, 255, "")]);
        let names: Vec<String> = channel_slots(&ring, true).into_iter().map(|(l, _)| l).collect();
        assert_eq!(names, ["off", "low", "mid", "full"]);
        let prism = chan("Prism", &[('S', 0, 127, "off"), ('S', 128, 156, "on"), ('S', 157, 255, "spins>f")]);
        let slots = channel_slots(&prism, true);
        assert_eq!(slots[1], ("on".to_string(), 142));
        assert!(channel_slots(&ring, false).is_empty());
    }

    /// The rig on this machine (from the ShowBuddy cache): both spot models
    /// contribute gobos to one island with same-named slots merged, and the
    /// effects come out as islands of their own.
    #[test]
    fn rig_wheels_become_islands() {
        let fixtures = crate::showbuddy::load_cache();
        if fixtures.is_empty() {
            eprintln!("no showbuddy_cache.json here — skipping");
            return;
        }
        let gobos = gobo_islands(&fixtures);
        assert_eq!(gobos.len(), 1);
        let sun = gobos[0].slots.iter().find(|s| s.label.eq_ignore_ascii_case("sun")).expect("a sun gobo");
        assert!(sun.values.len() >= 2, "sun should be shared by several lights");

        let fx = effect_islands(&fixtures);
        let names: Vec<&str> = fx.iter().map(|i| i.name.as_str()).collect();
        for want in ["Prism", "Spinner", "Light ring", "Macro", "Auto", "Fog", "Circle"] {
            assert!(names.contains(&want), "missing island {want} in {names:?}");
        }
        let prism = fx.iter().find(|i| i.name == "Prism").unwrap();
        let labels: Vec<&str> = prism.slots.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["off", "on", "spins>f"]);
        let ring = fx.iter().find(|i| i.name == "Light ring").unwrap();
        let labels: Vec<&str> = ring.slots.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["off", "low", "mid", "full"]);
        let spinner = fx.iter().find(|i| i.name == "Spinner").unwrap();
        assert!(spinner.slots.iter().any(|s| s.label == "on s>f"));
    }
}
