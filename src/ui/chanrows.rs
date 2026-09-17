//! The channel-control row model: which rows the selected lights produce,
//! how they group and sort, and the pure "arm" (selection) operations.
//!
//! Everything here is plain data and functions so it can be unit-tested
//! without a window. The panel in `channels.rs` builds rows every frame
//! from the current selection, lays them out under feature headings, and
//! applies the operations the toolbar, keyboard and context menu queue up.
//!
//! One row is one channel of one light in single-fixture mode, or one
//! channel *type* across every selected light in multi-fixture mode —
//! keyed by role, cell (pixel / head), fine-ness and the per-fixture
//! ordinal, so a mixed rig's lone dimmers merge into one row while a
//! moving head's six gobo channels stay six rows.

use std::collections::{HashMap, HashSet};

use eframe::egui::Color32;

use super::{role_color, theme};
use crate::net;
use crate::palette::Feature;
use crate::showbuddy::{Fixture, Role};

/// How the rows are ordered.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum ChanSort {
    /// Under feature headings, in console order.
    #[default]
    Feature,
    /// Flat, by DMX address.
    Address,
}

/// Channel-control UI state: not saved, not part of undo.
#[derive(Default)]
pub(crate) struct ChanUi {
    /// Empty shows everything.
    pub filter: String,
    pub sort: ChanSort,
    /// Folded headings. Kept across fixture changes on purpose.
    pub collapsed: HashSet<ChanGroup>,
    pub hide_fine: bool,
    pub only_armed: bool,
    /// Key of the last plain-clicked row: where a Shift-range starts.
    pub anchor: Option<String>,
    /// Key and `InputState::time` of the last name click: a double-click
    /// only counts when both clicks landed on the same row.
    pub last_click: Option<(String, f64)>,
}

/// The headings rows sit under, in the order an operator expects.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum ChanGroup {
    Dimmer,
    Position,
    Color,
    Beam,
    Focus,
    Control,
    Other,
}

impl ChanGroup {
    pub const ALL: [ChanGroup; 7] = [
        ChanGroup::Dimmer,
        ChanGroup::Position,
        ChanGroup::Color,
        ChanGroup::Beam,
        ChanGroup::Focus,
        ChanGroup::Control,
        ChanGroup::Other,
    ];

    pub fn of(role: Role) -> Self {
        match role {
            Role::Other => ChanGroup::Other,
            r => match Feature::of(r) {
                Feature::Dimmer => ChanGroup::Dimmer,
                Feature::Position => ChanGroup::Position,
                Feature::Color => ChanGroup::Color,
                Feature::Beam => ChanGroup::Beam,
                Feature::Focus => ChanGroup::Focus,
                Feature::Control => ChanGroup::Control,
            },
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ChanGroup::Dimmer => "Dimmer",
            ChanGroup::Position => "Position",
            ChanGroup::Color => "Color",
            ChanGroup::Beam => "Beam",
            ChanGroup::Focus => "Focus",
            ChanGroup::Control => "Control",
            ChanGroup::Other => "Other",
        }
    }

    pub fn rank(self) -> usize {
        Self::ALL.iter().position(|g| *g == self).unwrap_or(Self::ALL.len())
    }

    /// The swatch colour on the heading: the group's signature role.
    pub fn tint(self) -> Color32 {
        match self {
            ChanGroup::Dimmer => role_color(Role::Dimmer),
            ChanGroup::Position => role_color(Role::Pan),
            ChanGroup::Color => role_color(Role::Color),
            ChanGroup::Beam => role_color(Role::Gobo),
            ChanGroup::Focus => role_color(Role::Zoom),
            ChanGroup::Control => role_color(Role::Speed),
            ChanGroup::Other => theme::TEXT_DIM,
        }
    }
}

/// Console order inside a group. Exhaustive, so a new role must be placed.
pub(crate) fn role_rank(r: Role) -> u8 {
    match r {
        Role::Dimmer => 0,
        Role::Pan => 1,
        Role::PanFine => 2,
        Role::Tilt => 3,
        Role::TiltFine => 4,
        Role::Red => 5,
        Role::Green => 6,
        Role::Blue => 7,
        Role::White => 8,
        Role::Amber => 9,
        Role::Uv => 10,
        Role::Cyan => 11,
        Role::Magenta => 12,
        Role::Yellow => 13,
        Role::Color => 14,
        Role::Shutter => 15,
        Role::Strobe => 16,
        Role::Gobo => 17,
        Role::Prism => 18,
        Role::Frost => 19,
        Role::Zoom => 20,
        Role::Focus => 21,
        Role::Iris => 22,
        Role::Speed => 23,
        Role::Other => 24,
    }
}

/// A channel name taken apart: what it is, which cell (pixel / head) it
/// belongs to, and whether it is the fine byte of something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NameParts {
    pub base: String,
    pub cell: Option<String>,
    pub fine: bool,
}

/// Lower-cases and strips one cell token, first hit wins: a trailing
/// "(head N)" → `hN`, a trailing "(x,y)" → `x,y`, a leading "pixel N" →
/// `pN`, and on per-pixel roles a bare number after the first word → `N`.
/// Fine is the role, the word "fine",
/// or a "panf"/"tiltf" spelling. The base is what is left, without "fine".
pub(crate) fn split_name(name: &str, role: Role) -> NameParts {
    let lower = name.trim().to_lowercase();
    let mut rest = lower.clone();
    let mut cell: Option<String> = None;

    if let Some(open) = rest.rfind(" (head ") {
        if rest.ends_with(')') {
            let n: String = rest[open + 7..rest.len() - 1].trim().to_string();
            if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
                cell = Some(format!("h{n}"));
                rest.truncate(open);
            }
        }
    }
    if cell.is_none() && rest.ends_with(')') {
        if let Some(open) = rest.rfind(" (") {
            let inner = &rest[open + 2..rest.len() - 1];
            if inner.contains(',') && inner.chars().all(|c| c.is_ascii_digit() || c == ',') {
                cell = Some(inner.to_string());
                rest.truncate(open);
            }
        }
    }
    if cell.is_none() {
        if let Some(after) = rest.strip_prefix("pixel ") {
            let n: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !n.is_empty() {
                cell = Some(format!("p{n}"));
                rest = after[n.len()..].trim().to_string();
            }
        }
    }
    // A bare number after the first word is a pixel or head only on the
    // roles that come per pixel or head ("Red 1", "Tilt 2"); on a wheel or
    // a macro ("Gobo wheel 1", "No function 2") it is part of the name.
    let per_cell = matches!(
        role,
        Role::Red
            | Role::Green
            | Role::Blue
            | Role::White
            | Role::Amber
            | Role::Uv
            | Role::Cyan
            | Role::Magenta
            | Role::Yellow
            | Role::Dimmer
            | Role::Pan
            | Role::PanFine
            | Role::Tilt
            | Role::TiltFine
    );
    if cell.is_none() && per_cell {
        let words: Vec<&str> = rest.split(' ').collect();
        if let Some(at) = words
            .iter()
            .enumerate()
            .skip(1)
            .position(|(_, w)| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()))
            .map(|p| p + 1)
        {
            cell = Some(words[at].to_string());
            let kept: Vec<&str> = words
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != at)
                .map(|(_, w)| *w)
                .collect();
            rest = kept.join(" ");
        }
    }

    let spelled_fine = (lower.starts_with("pan") || lower.starts_with("tilt"))
        && !lower.contains(' ')
        && lower.ends_with('f');
    let fine = matches!(role, Role::PanFine | Role::TiltFine)
        || lower.split(|c: char| !c.is_alphanumeric()).any(|w| w == "fine")
        || spelled_fine;

    let mut base: String = rest
        .split(|c: char| c == ' ' || c == '_' && false)
        .filter(|w| *w != "fine" && !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if spelled_fine && base.ends_with('f') {
        base.pop();
    }
    let base = base.split_whitespace().collect::<Vec<_>>().join(" ");
    NameParts { base, cell, fine }
}

/// The identity the Stream Deck encoder uses for a channel type — `r:<TAG>`
/// for classified channels, `n:<lower name>` for the rest. `encoder.rs`
/// calls this too, so the two can never drift.
pub(crate) fn role_key(role: Role, name: &str) -> String {
    let tag = role.tag();
    if tag.is_empty() {
        format!("n:{}", name.to_lowercase())
    } else {
        format!("r:{tag}")
    }
}

/// One line of the channel list.
#[derive(Debug, Clone)]
pub(crate) struct ChanRow {
    /// Identity, stable across regrouping and filtering.
    pub key: String,
    /// The encoder's key for this type — marks the knob's row.
    pub enc_key: String,
    pub role: Role,
    pub group: ChanGroup,
    /// The representative channel's name, verbatim.
    pub name: String,
    pub base: String,
    pub cell: Option<String>,
    pub fine: bool,
    /// N-th (role, cell, fine) channel within its fixture.
    pub ord: usize,
    /// 1-based DMX address of the representative.
    pub addr: usize,
    /// (patch fixture index, channel index) that created the row.
    pub repr: (usize, usize),
    /// The representative's (min, max) bands, sorted; empty unless stepped.
    pub bands: Vec<(u8, u8)>,
    pub stepped: bool,
    /// 0-based DMX indices; the representative's first. Never sorted.
    pub idxs: Vec<usize>,
}

/// Natural-numeric order for cell tokens: "h2" < "h10", "0,1" < "1,0".
fn cell_key(cell: &Option<String>) -> (u32, u32, String) {
    let Some(c) = cell else { return (0, 0, String::new()) };
    let mut nums = c
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u32>().unwrap_or(0));
    (nums.next().unwrap_or(0), nums.next().unwrap_or(0), c.clone())
}

/// The rows for `targets` (patch fixture indices), sorted for `sort`.
pub(crate) fn build_rows(fixtures: &[Fixture], targets: &[usize], sort: ChanSort) -> Vec<ChanRow> {
    let multi = targets.len() > 1;
    let mut rows: Vec<ChanRow> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for &fi in targets {
        let Some(f) = fixtures.get(fi) else { continue };
        let mut ords: HashMap<(Role, Option<String>, bool), usize> = HashMap::new();
        for (ci, ch) in f.channels.iter().enumerate() {
            let addr = f.from as usize + ci;
            if addr == 0 || addr > net::DMX_SLOTS {
                continue;
            }
            let idx = addr - 1;
            let role = ch.role();
            let parts = split_name(&ch.name, role);
            let ord = {
                let e = ords.entry((role, parts.cell.clone(), parts.fine)).or_insert(0);
                let o = *e;
                *e += 1;
                o
            };
            let key = if !multi {
                format!("a:{addr}")
            } else if role == Role::Other {
                role_key(role, &ch.name)
            } else {
                let mut k = format!("r:{}", role.tag());
                if let Some(c) = &parts.cell {
                    k.push('#');
                    k.push_str(c);
                }
                if parts.fine {
                    k.push_str("~f");
                }
                if ord > 0 {
                    k.push_str(&format!(":{ord}"));
                }
                k
            };
            if let Some(&p) = index.get(&key) {
                rows[p].idxs.push(idx);
                continue;
            }
            let stepped = ch.dim_range().is_none()
                && ch.bands.iter().any(|b| b.kind == 'S' && !(b.min == 0 && b.max == 255));
            let mut bands: Vec<(u8, u8)> =
                if stepped { ch.bands.iter().map(|b| (b.min, b.max)).collect() } else { Vec::new() };
            bands.sort_unstable();
            index.insert(key.clone(), rows.len());
            rows.push(ChanRow {
                key,
                enc_key: role_key(role, &ch.name),
                role,
                group: ChanGroup::of(role),
                name: ch.name.clone(),
                base: parts.base,
                cell: parts.cell,
                fine: parts.fine,
                ord,
                addr,
                repr: (fi, ci),
                bands,
                stepped,
                idxs: vec![idx],
            });
        }
    }
    match sort {
        ChanSort::Address => rows.sort_by_key(|r| r.idxs[0]),
        // Colour rows go pixel by pixel (R1 G1 B1 W1 R2 …); every other
        // group keeps console order, as in single mode.
        ChanSort::Feature if multi => rows.sort_by(|a, b| {
            let key = |r: &ChanRow| {
                let by_cell = r.group == ChanGroup::Color;
                (
                    r.group.rank(),
                    by_cell && r.cell.is_some(),
                    if by_cell { cell_key(&r.cell) } else { (0, 0, String::new()) },
                    role_rank(r.role),
                    r.cell.is_some(),
                    cell_key(&r.cell),
                    r.ord,
                    r.fine,
                    r.addr,
                )
            };
            key(a).cmp(&key(b))
        }),
        ChanSort::Feature => rows.sort_by_key(|r| (r.group.rank(), r.addr)),
    }
    rows
}

/// Does a row survive the filter? Every whitespace-separated token must be
/// found somewhere: name, base, role tag, group, "fine", the address (single
/// mode), the cell token, or the current band label.
pub(crate) fn row_matches(row: &ChanRow, filter_lower: &str, band: Option<&str>, single: bool) -> bool {
    if filter_lower.trim().is_empty() {
        return true;
    }
    let name = row.name.to_lowercase();
    let tag = row.role.tag().to_lowercase();
    let group = row.group.label().to_lowercase();
    let addr = if single { row.addr.to_string() } else { String::new() };
    let band = band.map(|b| b.to_lowercase()).unwrap_or_default();
    let cell = row.cell.clone().unwrap_or_default();
    filter_lower.split_whitespace().all(|t| {
        name.contains(t)
            || row.base.contains(t)
            || (!tag.is_empty() && tag.contains(t))
            || group.contains(t)
            || (row.fine && "fine".contains(t))
            || (!addr.is_empty() && addr == t)
            || (!cell.is_empty() && cell == t)
            || (!band.is_empty() && band.contains(t))
    })
}

/// A relative move, saturating at the ends.
pub(crate) fn nudged(cur: u8, delta: i32) -> u8 {
    (cur as i32 + delta).clamp(0, 255) as u8
}

/// What "similar" means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Sim {
    Role,
    Group,
    Cell,
    /// A coarse channel and its fine byte.
    Pair,
    Name,
}

/// A change to the armed set, queued during a frame and applied in order.
#[derive(Clone, Debug)]
pub(crate) enum ArmOp {
    /// Arm only these.
    Replace(Vec<usize>),
    Add(Vec<usize>),
    Remove(Vec<usize>),
    /// All armed → remove them all, else insert them all.
    Toggle(Vec<usize>),
    /// The anchor row through this row, in visible order, additive.
    Range { to_key: String },
    All,
    None,
    Invert,
    /// Add every visible row like the armed ones.
    Similar(Sim),
}

fn fully_armed(sel: &HashSet<usize>, row: &ChanRow) -> bool {
    !row.idxs.is_empty() && row.idxs.iter().all(|i| sel.contains(i))
}

pub(crate) fn is_pair(a: &ChanRow, b: &ChanRow) -> bool {
    if a.cell != b.cell {
        return false;
    }
    let role_pair = matches!(
        (a.role, b.role),
        (Role::Pan, Role::PanFine)
            | (Role::PanFine, Role::Pan)
            | (Role::Tilt, Role::TiltFine)
            | (Role::TiltFine, Role::Tilt)
    );
    role_pair || (a.group == b.group && a.base == b.base && a.fine != b.fine)
}

/// Apply one operation. `visible` lists row indices in display order —
/// All / None / Invert / Range work over those; Similar looks through
/// `candidates`, the rows the text filter lets through, so "armed only"
/// or "hide fine" never hide what Similar is meant to find.
pub(crate) fn apply_arm(
    sel: &mut HashSet<usize>,
    anchor: Option<&str>,
    op: ArmOp,
    rows: &[ChanRow],
    visible: &[usize],
    candidates: &[usize],
) {
    sel.retain(|&i| i < net::DMX_SLOTS);
    match op {
        ArmOp::Replace(idxs) => {
            sel.clear();
            sel.extend(idxs.into_iter().filter(|&i| i < net::DMX_SLOTS));
        }
        ArmOp::Add(idxs) => sel.extend(idxs.into_iter().filter(|&i| i < net::DMX_SLOTS)),
        ArmOp::Remove(idxs) => {
            for i in idxs {
                sel.remove(&i);
            }
        }
        ArmOp::Toggle(idxs) => {
            let all = !idxs.is_empty() && idxs.iter().all(|i| sel.contains(i));
            for i in idxs {
                if all {
                    sel.remove(&i);
                } else if i < net::DMX_SLOTS {
                    sel.insert(i);
                }
            }
        }
        ArmOp::Range { to_key } => {
            let pos = |key: &str| visible.iter().position(|&r| rows[r].key == key);
            let to = pos(&to_key);
            match (anchor.and_then(pos), to) {
                (Some(a), Some(b)) => {
                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                    for &r in &visible[lo..=hi] {
                        sel.extend(rows[r].idxs.iter().copied());
                    }
                }
                (None, Some(b)) => {
                    let idxs = rows[visible[b]].idxs.clone();
                    apply_arm(sel, anchor, ArmOp::Toggle(idxs), rows, visible, candidates);
                }
                _ => {}
            }
        }
        ArmOp::All => {
            for &r in visible {
                sel.extend(rows[r].idxs.iter().copied());
            }
        }
        ArmOp::None => sel.clear(),
        ArmOp::Invert => {
            for &r in visible {
                if fully_armed(sel, &rows[r]) {
                    for i in &rows[r].idxs {
                        sel.remove(i);
                    }
                } else {
                    sel.extend(rows[r].idxs.iter().copied());
                }
            }
        }
        ArmOp::Similar(sim) => {
            let seeds: Vec<&ChanRow> =
                candidates.iter().map(|&r| &rows[r]).filter(|r| fully_armed(sel, r)).collect();
            if seeds.is_empty() {
                return;
            }
            let mut add: Vec<usize> = Vec::new();
            for &r in candidates {
                let row = &rows[r];
                let like = seeds.iter().any(|s| match sim {
                    Sim::Role => s.role == row.role,
                    Sim::Group => s.group == row.group,
                    Sim::Cell => s.cell.is_some() && s.cell == row.cell,
                    Sim::Pair => is_pair(s, row),
                    Sim::Name => s.base == row.base,
                });
                if like {
                    add.extend(row.idxs.iter().copied());
                }
            }
            sel.extend(add);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::PROFILES;

    fn parts(name: &str, role: Role) -> (String, Option<String>, bool) {
        let p = split_name(name, role);
        (p.base, p.cell, p.fine)
    }

    #[test]
    fn names_split_into_base_cell_and_fine() {
        assert_eq!(parts("Red 1", Role::Red), ("red".into(), Some("1".into()), false));
        assert_eq!(parts("Red (0,0)", Role::Red), ("red".into(), Some("0,0".into()), false));
        assert_eq!(parts("Pan Fine (Head 1)", Role::PanFine), ("pan".into(), Some("h1".into()), true));
        assert_eq!(parts("Pixel 3 ColorAdd_R", Role::Red), ("coloradd_r".into(), Some("p3".into()), false));
        assert_eq!(parts("Dimmer fine", Role::Dimmer), ("dimmer".into(), None, true));
        assert_eq!(parts("Fine Dimmer", Role::Dimmer), ("dimmer".into(), None, true));
        assert_eq!(parts("Panf", Role::PanFine), ("pan".into(), None, true));
        // Wheel and macro numbers are names, not pixels.
        assert_eq!(
            parts("Gobo wheel 1 index fine", Role::Gobo),
            ("gobo wheel 1 index".into(), None, true)
        );
        assert_eq!(parts("No function 2", Role::Other), ("no function 2".into(), None, false));
        assert_eq!(parts("Tilt 2", Role::Tilt), ("tilt".into(), Some("2".into()), false));
        assert_eq!(parts("BackColor R", Role::Red), ("backcolor r".into(), None, false));
    }

    #[test]
    fn every_role_has_a_group_and_a_unique_rank() {
        let roles = [
            Role::Dimmer, Role::Red, Role::Green, Role::Blue, Role::White, Role::Amber, Role::Uv,
            Role::Cyan, Role::Magenta, Role::Yellow, Role::Color, Role::Strobe, Role::Shutter,
            Role::Pan, Role::PanFine, Role::Tilt, Role::TiltFine, Role::Zoom, Role::Focus,
            Role::Iris, Role::Gobo, Role::Prism, Role::Frost, Role::Speed, Role::Other,
        ];
        let mut ranks: Vec<u8> = roles.iter().map(|&r| role_rank(r)).collect();
        ranks.sort_unstable();
        ranks.dedup();
        assert_eq!(ranks.len(), roles.len(), "ranks are unique");
        assert_eq!(ChanGroup::of(Role::Other), ChanGroup::Other);
        assert!(role_rank(Role::Shutter) < role_rank(Role::Strobe));
        assert_eq!(role_rank(Role::PanFine), role_rank(Role::Pan) + 1);
        assert_eq!(role_key(Role::Other, "Reset"), "n:reset");
        assert_eq!(role_key(Role::Pan, "x"), "r:PAN");
    }

    fn fixture(name: &str, from: u16) -> Fixture {
        PROFILES
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("profile {name}"))
            .to_fixture(name.to_string(), from)
    }

    #[test]
    fn single_fixture_rows_group_in_console_order() {
        let f = fixture("Maverick MK2 Spot (32ch)", 1);
        let fixtures = vec![f];
        let rows = build_rows(&fixtures, &[0], ChanSort::Feature);
        assert_eq!(rows.len(), 32);
        // Groups never go backwards; addresses ascend inside a group.
        for w in rows.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            assert!(a.group.rank() <= b.group.rank(), "{} before {}", a.name, b.name);
            if a.group == b.group {
                assert!(a.addr < b.addr);
            }
        }
        for r in &rows {
            assert_eq!(r.idxs[0], r.addr - 1);
        }
        let dim = rows.iter().position(|r| r.role == Role::Dimmer && !r.fine).expect("dimmer");
        if let Some(fine) = rows.iter().position(|r| r.role == Role::Dimmer && r.fine) {
            assert_eq!(fine, dim + 1, "the fine byte follows its coarse byte");
        }
        let gobos = rows.iter().filter(|r| r.role == Role::Gobo).count();
        assert!(gobos >= 2, "the spot has several gobo-role channels");
        let keys: HashSet<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys.len(), rows.len(), "keys are unique");
        // Address sort: flat and ascending.
        let flat = build_rows(&fixtures, &[0], ChanSort::Address);
        for w in flat.windows(2) {
            assert!(w[0].addr < w[1].addr);
        }
    }

    #[test]
    fn two_of_a_kind_merge_row_by_row_and_a_mixed_rig_shares_its_dimmer() {
        let fixtures = vec![fixture("Maverick MK2 Spot (32ch)", 1), fixture("Maverick MK2 Spot (32ch)", 101)];
        let rows = build_rows(&fixtures, &[0, 1], ChanSort::Feature);
        assert_eq!(rows.len(), 32, "every channel type once");
        for r in &rows {
            assert_eq!(r.idxs.len(), 2, "{} drives both", r.name);
            assert!(r.idxs[0] < 100, "the first fixture leads");
            assert_eq!(r.idxs[1], r.idxs[0] + 100);
        }
        let keys: HashSet<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert!(keys.contains("r:DIM"));
        assert_eq!(keys.len(), rows.len());

        // A par with just a dimmer shares the spot's dimmer row.
        let par = PROFILES
            .iter()
            .find(|p| p.channel_count() < 10 && p.channels().iter().any(|c| c.role() == Role::Dimmer))
            .map(|p| p.to_fixture(p.name.to_string(), 200));
        if let Some(par) = par {
            let fixtures = vec![fixture("Maverick MK2 Spot (32ch)", 1), par];
            let rows = build_rows(&fixtures, &[0, 1], ChanSort::Feature);
            let dims: Vec<&ChanRow> = rows.iter().filter(|r| r.key == "r:DIM").collect();
            assert_eq!(dims.len(), 1);
            assert_eq!(dims[0].idxs.len(), 2);
        }
    }

    #[test]
    fn filter_tokens_all_have_to_land() {
        let f = fixture("Maverick MK2 Spot (32ch)", 1);
        let fixtures = vec![f];
        let rows = build_rows(&fixtures, &[0], ChanSort::Feature);
        let dim = rows.iter().find(|r| r.role == Role::Dimmer && !r.fine).unwrap();
        assert!(row_matches(dim, "dim", None, true));
        assert!(row_matches(dim, &dim.addr.to_string(), None, true));
        assert!(!row_matches(dim, "fine", None, true));
        assert!(row_matches(dim, "dimmer dim", None, true));
        assert!(!row_matches(dim, "dimmer red", None, true));
        assert!(row_matches(dim, "open", Some("Open"), true));
    }

    #[test]
    fn nudges_saturate() {
        assert_eq!(nudged(255, 1), 255);
        assert_eq!(nudged(0, -1), 0);
        assert_eq!(nudged(250, 10), 255);
        assert_eq!(nudged(5, -10), 0);
        assert_eq!(nudged(100, 1), 101);
    }

    fn row(key: &str, role: Role, base: &str, cell: Option<&str>, fine: bool, idx: usize) -> ChanRow {
        ChanRow {
            key: key.into(),
            enc_key: role_key(role, base),
            role,
            group: ChanGroup::of(role),
            name: base.into(),
            base: base.into(),
            cell: cell.map(str::to_string),
            fine,
            ord: 0,
            addr: idx + 1,
            repr: (0, idx),
            bands: Vec::new(),
            stepped: false,
            idxs: vec![idx],
        }
    }

    #[test]
    fn arm_operations() {
        let rows = vec![
            row("d", Role::Dimmer, "dimmer", None, false, 0),
            row("df", Role::Dimmer, "dimmer", None, true, 1),
            row("r2", Role::Red, "red", Some("2"), false, 2),
            row("g2", Role::Green, "green", Some("2"), false, 3),
            row("r3", Role::Red, "red", Some("3"), false, 4),
            row("p", Role::Pan, "pan", None, false, 5),
            row("pf", Role::PanFine, "pan", None, true, 6),
        ];
        let visible: Vec<usize> = (0..rows.len()).collect();
        let mut sel = HashSet::new();
        apply_arm(&mut sel, None, ArmOp::Replace(vec![0]), &rows, &visible, &visible);
        assert_eq!(sel, HashSet::from([0]));
        // Replace is idempotent; disarming a solo row is the click's job.
        apply_arm(&mut sel, None, ArmOp::Replace(vec![0]), &rows, &visible, &visible);
        assert_eq!(sel, HashSet::from([0]));
        apply_arm(&mut sel, None, ArmOp::None, &rows, &visible, &visible);
        assert!(sel.is_empty());
        apply_arm(&mut sel, None, ArmOp::Toggle(vec![2, 4]), &rows, &visible, &visible);
        assert_eq!(sel, HashSet::from([2, 4]));
        apply_arm(&mut sel, None, ArmOp::Toggle(vec![2, 4]), &rows, &visible, &visible);
        assert!(sel.is_empty());
        // A range from the anchor, additive; without an anchor, a toggle.
        apply_arm(&mut sel, Some("df"), ArmOp::Range { to_key: "g2".into() }, &rows, &visible, &visible);
        assert_eq!(sel, HashSet::from([1, 2, 3]));
        apply_arm(&mut sel, None, ArmOp::Range { to_key: "p".into() }, &rows, &visible, &visible);
        assert_eq!(sel, HashSet::from([1, 2, 3, 5]));
        apply_arm(&mut sel, None, ArmOp::Invert, &rows, &visible, &visible);
        assert_eq!(sel, HashSet::from([0, 4, 6]));
        apply_arm(&mut sel, None, ArmOp::None, &rows, &visible, &visible);
        assert!(sel.is_empty());
        apply_arm(&mut sel, None, ArmOp::All, &rows, &visible, &visible);
        assert_eq!(sel.len(), 7);
        // Similar: by role from a red seed; the fine partner; the cell.
        sel = HashSet::from([2]);
        apply_arm(&mut sel, None, ArmOp::Similar(Sim::Role), &rows, &visible, &visible);
        assert_eq!(sel, HashSet::from([2, 4]));
        sel = HashSet::from([0]);
        apply_arm(&mut sel, None, ArmOp::Similar(Sim::Pair), &rows, &visible, &visible);
        assert_eq!(sel, HashSet::from([0, 1]));
        sel = HashSet::from([5]);
        apply_arm(&mut sel, None, ArmOp::Similar(Sim::Pair), &rows, &visible, &visible);
        assert_eq!(sel, HashSet::from([5, 6]));
        sel = HashSet::from([2]);
        apply_arm(&mut sel, None, ArmOp::Similar(Sim::Cell), &rows, &visible, &visible);
        assert_eq!(sel, HashSet::from([2, 3]));
        // Similar looks past the view toggles: with only the armed row shown,
        // the other reds are still found.
        sel = HashSet::from([2]);
        apply_arm(&mut sel, None, ArmOp::Similar(Sim::Role), &rows, &[2], &visible);
        assert_eq!(sel, HashSet::from([2, 4]));
        // Stale indices are dropped.
        sel = HashSet::from([2, 5000]);
        apply_arm(&mut sel, None, ArmOp::Add(vec![3]), &rows, &visible, &visible);
        assert_eq!(sel, HashSet::from([2, 3]));
    }
}
