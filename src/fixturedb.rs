//! The bundled fixture library — every mode of every fixture DMXpress knows
//! how to patch, built by `tools/build_library.py` from the Open Fixture
//! Library (MIT), the QLC+ fixture definitions (Apache-2.0) and GDTF Share.
//!
//! Every upstream describes a fixture once and lists its DMX modes
//! separately; patching works on one mode at a time, so the converters
//! flatten each mode into its own entry with the channel list already in
//! address order and every channel's [`Role`](crate::showbuddy::Role)
//! resolved from the source's own capability data rather than guessed from
//! the name. OFL is the most precise and wins where they overlap, QLC+ fills
//! in the models and alternate channel-count modes OFL doesn't carry, and
//! GDTF adds the manufacturers' own data for the current professional ranges.
//!
//! Uncompressed the library is ~14 MB of JSON, so it ships gzipped, and
//! nothing needs it until someone opens the patch browser — it loads lazily
//! on first search and stays cached after that.

use std::io::Read;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::showbuddy::{Channel, Fixture};

/// The bundled library, as shipped.
pub const LIBRARY_FILE: &str = "fixtures/library.json.gz";

/// Same data uncompressed, which `build_library.py` writes for any output
/// path not ending in `.gz`. Only a fallback: a stale copy left over from an
/// older DMXpress must never quietly win over the bundled library.
pub const LIBRARY_FILE_PLAIN: &str = "fixtures/library.json";

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

/// What people type instead of the name the library files a maker under.
/// Only abbreviations belong here — spacing and punctuation differences
/// ("clay paky" vs "claypaky") are handled by [`squash`].
const MAKER_ALIASES: &[(&str, &str)] = &[
    ("American DJ", "adj"),
    ("Chauvet Professional", "chauvet pro"),
    ("ETC", "electronic theatre controls"),
    ("GLP", "german light products"),
    ("High End Systems", "hes"),
    ("Martin", "martin professional"),
    ("Robe", "robe lighting"),
];

/// A string with everything but letters and digits taken out, so "claypaky"
/// finds "Clay Paky" and "prolights" finds "Pro-Lights".
fn squash(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// What [`Library::search`] matches against: the fixture's own words, its
/// maker's aliases, and the whole lot again with the punctuation squeezed
/// out so spacing never costs you a hit.
fn haystack(f: &LibraryFixture) -> String {
    let alias = MAKER_ALIASES
        .iter()
        .find(|(maker, _)| *maker == f.manufacturer)
        .map_or("", |(_, alias)| alias);
    let words = format!("{} {} {} {alias}", f.manufacturer, f.model, f.mode).to_lowercase();
    let squashed = squash(&words);
    format!("{words} {squashed}")
}

/// The loaded library. Empty when neither [`LIBRARY_FILE`] nor
/// [`LIBRARY_FILE_PLAIN`] is there, which just means the browser has nothing
/// to offer — everything else still works.
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

/// The library JSON, gunzipped if it needs it.
fn read_library() -> std::io::Result<String> {
    match std::fs::File::open(LIBRARY_FILE) {
        Ok(file) => {
            let mut text = String::new();
            flate2::read::GzDecoder::new(std::io::BufReader::new(file))
                .read_to_string(&mut text)?;
            Ok(text)
        }
        Err(gz_err) => std::fs::read_to_string(LIBRARY_FILE_PLAIN).map_err(|_| gz_err),
    }
}

impl Library {
    /// Reads the library off disk. Call once, lazily — it unpacks and parses
    /// well over ten megabytes.
    pub fn load() -> Self {
        let (fixtures, error) = match read_library() {
            Ok(text) => match serde_json::from_str::<LibraryFile>(&text) {
                Ok(parsed) => (parsed.fixtures, None),
                Err(e) => (Vec::new(), Some(format!("{LIBRARY_FILE} is malformed: {e}"))),
            },
            Err(e) => (Vec::new(), Some(format!("{LIBRARY_FILE}: {e}"))),
        };
        let haystacks = fixtures.iter().map(haystack).collect();
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
        // Each term also gets a punctuation-free form, so "claypaky sharpy"
        // and "clay paky sharpy" find the same light.
        let terms: Vec<(String, String)> = query
            .split_whitespace()
            .map(|t| {
                let lower = t.to_lowercase();
                let squashed = squash(&lower);
                (lower, squashed)
            })
            .collect();
        let mut out = Vec::new();
        for (i, f) in self.fixtures.iter().enumerate() {
            if manufacturer.is_some_and(|m| m != f.manufacturer) {
                continue;
            }
            let hay = &self.haystacks[i];
            if terms
                .iter()
                .all(|(raw, squashed)| hay.contains(raw.as_str()) || hay.contains(squashed.as_str()))
            {
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

    /// The bundled library has to actually be there, gunzip and parse — a
    /// schema drift in the converter would otherwise only show up as an empty
    /// browser at runtime. The floors are well under what
    /// `tools/build_library.py` currently emits (~7,400 modes / 2,200 models
    /// / 210 makers), so refreshing either upstream won't trip them, but
    /// dropping a whole source would.
    #[test]
    fn bundled_library_loads() {
        let lib = library();
        assert!(lib.is_loaded());
        assert!(lib.fixtures.len() > 6000, "only {} fixtures", lib.fixtures.len());
        assert!(lib.manufacturers().len() > 150, "only {} makers", lib.manufacturers().len());
        let models: std::collections::HashSet<_> =
            lib.fixtures.iter().map(|f| (&f.manufacturer, &f.model)).collect();
        assert!(models.len() > 1800, "only {} models", models.len());
    }

    /// No entry may be a duplicate of another: ids are what a patched fixture
    /// stores, and two rows a person can't tell apart are worse than one.
    #[test]
    fn entries_are_distinct() {
        let lib = library();
        let mut ids = std::collections::HashSet::new();
        let mut rows = std::collections::HashSet::new();
        for f in &lib.fixtures {
            assert!(ids.insert(&f.id), "duplicate id {}", f.id);
            let row = (&f.manufacturer, &f.model, &f.mode, f.channels.len());
            assert!(rows.insert(row), "two identical rows for {row:?}");
        }
    }

    /// The reason the library carries several entries per model: a fixture is
    /// switched to a channel mode at the back panel, and you have to patch
    /// the one it's actually in. The Intimidator Spot 375Z runs 9 or 15.
    #[test]
    fn fixtures_offer_their_alternate_channel_modes() {
        let lib = library();
        let hits = lib.search("intimidator spot 375z", None, 20);
        let widths: std::collections::BTreeSet<usize> =
            hits.iter().map(|i| lib.fixtures[*i].channels.len()).collect();
        assert!(
            widths.contains(&9) && widths.contains(&15),
            "expected 9- and 15-channel modes, got {widths:?}"
        );
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

    /// Nobody types the maker's full legal name. "ADJ" has to find American
    /// DJ, and neither spacing nor punctuation may cost a hit.
    #[test]
    fn search_finds_makers_by_the_name_people_type() {
        let lib = library();
        for (query, want) in [
            ("adj", "American DJ"),
            ("claypaky", "Clay Paky"),
            ("clay paky", "Clay Paky"),
            ("prolights", "Prolights"),
            ("german light products", "GLP"),
        ] {
            let hits = lib.search(query, None, 20);
            assert!(!hits.is_empty(), "{query:?} found nothing");
            assert!(
                hits.iter().any(|i| lib.fixtures[*i].manufacturer == want),
                "{query:?} found no {want}"
            );
        }
    }

    /// A pixel bar's per-pixel mode: the converter expands OFL's matrix
    /// template blocks, so this has to come back as real, separately
    /// addressable channels rather than being skipped.
    #[test]
    fn pixel_matrix_modes_expand() {
        let lib = library();
        let hits = lib.search("american dj revo 4", None, 20);
        let f = hits
            .iter()
            .map(|i| &lib.fixtures[*i])
            .find(|f| f.channels.len() > 200)
            .expect("expected the Revo 4 IR's per-pixel mode");
        // 8x8 pixels of RGBW.
        assert_eq!(f.channels.len(), 256);
        let reds = f.channels.iter().filter(|c| c.role() == Role::Red).count();
        assert_eq!(reds, 64, "expected one red emitter per pixel");
        assert!(f.channels.iter().all(|c| !c.name.contains("$pixelKey")));
    }
}
