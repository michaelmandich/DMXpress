//! The bundled fixture library — every mode of every fixture DMXpress knows
//! how to patch, converted from the Open Fixture Library (MIT).
//!
//! OFL describes a fixture once and lists its DMX modes separately; patching
//! works on one mode at a time, so `tools/ofl_to_library.py` flattens each
//! mode into its own entry with the channel list already in address order and
//! every channel's [`Role`](crate::showbuddy::Role) resolved from OFL's own
//! capability data rather than guessed from the name.
//!
//! The file is a few MB, and nothing needs it until someone opens the patch
//! browser, so it loads lazily on first search and stays cached after that.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::showbuddy::{Channel, Fixture};

pub const LIBRARY_FILE: &str = "fixtures/library.json";

/// Prefix marking a patched fixture as coming from the library, the way
/// `builtin:` marks one of the hand-written profiles. Also the type identity
/// the stage's "select same type" gesture groups on.
pub const LIB_PREFIX: &str = "lib:";

/// One patchable fixture mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryFixture {
    /// Stable `manufacturer/fixture/mode` key, e.g. `chauvet-dj/intimidator-spot-110/6ch`.
    pub id: String,
    pub manufacturer: String,
    pub model: String,
    pub mode: String,
    pub category: String,
    pub pan_range: f32,
    pub tilt_range: f32,
    pub beam_width: f32,
    pub channels: Vec<Channel>,
}

impl LibraryFixture {
    /// What the browser lists: "Chauvet DJ — Intimidator Spot 110".
    pub fn title(&self) -> String {
        format!("{} — {}", self.manufacturer, self.model)
    }

    /// "6-channel · 6 ch", or just the channel count for unnamed modes.
    pub fn subtitle(&self) -> String {
        if self.mode.is_empty() {
            format!("{} ch", self.channels.len())
        } else {
            format!("{} · {} ch", self.mode, self.channels.len())
        }
    }

    /// Materialize a patched fixture at 1-based DMX address `from`, matching
    /// [`crate::profiles::Profile::to_fixture`].
    pub fn to_fixture(&self, display: String, from: u16) -> Fixture {
        let channels = self.channels.clone();
        let to = from + channels.len().max(1) as u16 - 1;
        Fixture {
            display,
            file: PathBuf::from(format!("{LIB_PREFIX}{}", self.id)),
            from,
            to,
            // Centre stage, same as a built-in profile: the 3D view has no
            // opinion about where a freshly patched light physically sits.
            x: 0.5,
            y: 0.5,
            pan_range: self.pan_range,
            tilt_range: self.tilt_range,
            beam_width: self.beam_width,
            channels,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct LibraryFile {
    #[serde(default)]
    fixtures: Vec<LibraryFixture>,
}

/// The loaded library. Empty when `fixtures/library.json` is missing, which
/// just means the browser has nothing to offer — everything else still works.
#[derive(Debug, Default)]
pub struct Library {
    pub fixtures: Vec<LibraryFixture>,
    /// Lowercased "manufacturer model mode" per fixture, for matching.
    haystacks: Vec<String>,
    /// Why the library is empty, when it is. One malformed entry fails the
    /// whole parse, and an empty browser hides that far too well.
    pub error: Option<String>,
    loaded: bool,
}

impl Library {
    /// Reads the library off disk. Call once, lazily — it parses a few MB.
    pub fn load() -> Self {
        let (fixtures, error) = match std::fs::read_to_string(LIBRARY_FILE) {
            Ok(text) => match serde_json::from_str::<LibraryFile>(&text) {
                Ok(parsed) => (parsed.fixtures, None),
                Err(e) => (Vec::new(), Some(format!("{LIBRARY_FILE} is malformed: {e}"))),
            },
            Err(e) => (Vec::new(), Some(format!("{LIBRARY_FILE}: {e}"))),
        };
        let haystacks = fixtures
            .iter()
            .map(|f| format!("{} {} {}", f.manufacturer, f.model, f.mode).to_lowercase())
            .collect();
        Self { fixtures, haystacks, error, loaded: true }
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Distinct manufacturers, in the order the library lists them.
    pub fn manufacturers(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for f in &self.fixtures {
            if out.last().is_none_or(|m| *m != f.manufacturer) {
                out.push(&f.manufacturer);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Indices of fixtures matching every whitespace-separated term in
    /// `query` (so "chauvet spot 110" narrows the way you'd expect),
    /// optionally pinned to one manufacturer, capped at `limit`.
    pub fn search(&self, query: &str, manufacturer: Option<&str>, limit: usize) -> Vec<usize> {
        let terms: Vec<String> =
            query.split_whitespace().map(|t| t.to_lowercase()).collect();
        let mut out = Vec::new();
        for (i, f) in self.fixtures.iter().enumerate() {
            if manufacturer.is_some_and(|m| m != f.manufacturer) {
                continue;
            }
            let hay = &self.haystacks[i];
            if terms.iter().all(|t| hay.contains(t.as_str())) {
                out.push(i);
                if out.len() >= limit {
                    break;
                }
            }
        }
        out
    }

    pub fn find(&self, id: &str) -> Option<&LibraryFixture> {
        self.fixtures.iter().find(|f| f.id == id)
    }
}

/// Splits a `UserFixture.profile` string into a library id, if it is one.
/// Anything else is a built-in profile name.
pub fn library_id(profile: &str) -> Option<&str> {
    profile.strip_prefix(LIB_PREFIX)
}

/// The `profile` string that patches `id` from the library.
pub fn profile_ref(id: &str) -> String {
    format!("{LIB_PREFIX}{id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::showbuddy::Role;

    fn library() -> Library {
        Library::load()
    }

    /// The bundled library has to actually be there and parse — a schema
    /// drift in the converter would otherwise only show up as an empty
    /// browser at runtime.
    #[test]
    fn bundled_library_loads() {
        let lib = library();
        assert!(lib.is_loaded());
        assert!(lib.fixtures.len() > 1000, "only {} fixtures", lib.fixtures.len());
        assert!(lib.manufacturers().len() > 50);
    }

    /// Every entry has to be patchable: a non-empty channel list whose span
    /// fits the DMX buffer, and an id the patch round-trip can resolve.
    #[test]
    fn every_entry_is_patchable() {
        let lib = library();
        for f in &lib.fixtures {
            assert!(!f.channels.is_empty(), "{} has no channels", f.id);
            assert!(f.channels.len() <= 512, "{} has {} channels", f.id, f.channels.len());
            assert!(!f.manufacturer.is_empty() && !f.model.is_empty(), "{} unnamed", f.id);
            let fixture = f.to_fixture("Test".into(), 1);
            assert_eq!(fixture.channel_count(), f.channels.len(), "{} span mismatch", f.id);
            assert_eq!(library_id(&profile_ref(&f.id)), Some(f.id.as_str()));
        }
    }

    /// The whole point of the explicit roles: a moving head's pan, tilt and
    /// colour channels have to come back classified, not as `Other`.
    #[test]
    fn moving_head_roles_resolve() {
        let lib = library();
        let hits = lib.search("intimidator spot 110", None, 8);
        assert!(!hits.is_empty(), "expected a Chauvet Intimidator Spot 110");
        let f = &lib.fixtures[hits[0]];
        let roles: Vec<Role> = f.channels.iter().map(|c| c.role()).collect();
        for want in [Role::Pan, Role::Tilt, Role::Dimmer, Role::Color] {
            assert!(roles.contains(&want), "{} missing {want:?} in {roles:?}", f.id);
        }
    }

    /// The Maverick MK Pyxis Basic mode, hand-transcribed from Chauvet's DMX
    /// chart rather than generated from OFL (it isn't in OFL). Its
    /// "Continuous Pan"/"Continuous Tilt" channels are a second, independent
    /// pan/tilt control and must NOT resolve to `Role::Pan`/`Role::Tilt` —
    /// `stage::fixture::live_state` treats the *last* `Role::Pan` channel as
    /// the fixture's position, so a second one would clobber the real pan
    /// channel's high byte with the continuous-rotation speed value.
    #[test]
    fn maverick_pyxis_basic_mode_is_patchable() {
        let lib = library();
        let hits = lib.search("maverick pyxis", None, 8);
        assert!(!hits.is_empty(), "expected the Chauvet Maverick MK Pyxis");
        let f = &lib.fixtures[hits[0]];
        assert_eq!(f.id, "chauvet-professional/maverick-mk-pyxis/26ch");
        assert_eq!(f.channels.len(), 26);
        let roles: Vec<Role> = f.channels.iter().map(|c| c.role()).collect();
        assert_eq!(roles.iter().filter(|r| **r == Role::Pan).count(), 1);
        assert_eq!(roles.iter().filter(|r| **r == Role::Tilt).count(), 1);
        for want in [Role::Pan, Role::PanFine, Role::Tilt, Role::TiltFine, Role::Zoom, Role::Color, Role::Gobo] {
            assert!(roles.contains(&want), "{} missing {want:?} in {roles:?}", f.id);
        }
    }

    /// Search narrows on every term, and the manufacturer filter pins it.
    #[test]
    fn search_narrows_by_term_and_maker() {
        let lib = library();
        let broad = lib.search("spot", None, 500).len();
        let narrow = lib.search("chauvet spot", None, 500).len();
        assert!(narrow > 0 && narrow < broad, "broad {broad}, narrow {narrow}");
        let maker = lib.fixtures[lib.search("", None, 1)[0]].manufacturer.clone();
        for i in lib.search("", Some(&maker), 50) {
            assert_eq!(lib.fixtures[i].manufacturer, maker);
        }
    }
}
