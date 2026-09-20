//! Pure address / name / recents maths for the Patch tab (area F). No egui.
//!
//! Addresses are 1-based and absolute: universe 1 is 1–512, universe 2
//! starts at 513, and so on up to [`crate::net::DMX_SLOTS`], the hard
//! ceiling. A fixture at `from` with a span of `n` channels occupies
//! `from..=from+n-1`. Nothing here hardcodes a universe count — see
//! [`universe`] and [`universe_base`].

use super::type_stem;
use crate::net::{DMX_SLOTS, DMX_UNIVERSES};
use crate::showbuddy::Fixture;

/// Slots in one DMX universe.
const UNIVERSE: u16 = 512;

/// Why [`plan_addresses`] stopped handing out addresses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PlanStop {
    /// Every copy got an address.
    Complete,
    /// Only this many fit before the last slot.
    Overflow(usize),
}

/// (from, to) of every fixture in the rig, ShowBuddy's included.
pub(crate) fn ranges(fixtures: &[Fixture]) -> Vec<(u16, u16)> {
    fixtures.iter().map(|f| (f.from, f.to)).collect()
}

/// Which slots are taken, indexed by address (`used[0]` is unused).
fn occupancy(ranges: &[(u16, u16)]) -> Vec<bool> {
    let mut used = vec![false; DMX_SLOTS + 1];
    for &(a, b) in ranges {
        for slot in a.max(1)..=b.min(DMX_SLOTS as u16) {
            used[slot as usize] = true;
        }
    }
    used
}

/// One past the highest address in use (1 for an empty rig), never over the
/// last slot.
pub(crate) fn next_free(ranges: &[(u16, u16)]) -> u16 {
    ranges
        .iter()
        .map(|r| r.1)
        .max()
        .map_or(1, |m| m.saturating_add(1))
        .min(DMX_SLOTS as u16)
}

/// The lowest address at or after `start` where a `span`-channel fixture
/// fits in a hole, or `None` when nothing does. With `same_universe` the
/// hole may not straddle 512/513.
pub(crate) fn first_free(
    ranges: &[(u16, u16)],
    span: u16,
    start: u16,
    same_universe: bool,
) -> Option<u16> {
    first_free_in_map(&occupancy(ranges), span, start, same_universe)
}

/// [`first_free`] against an occupancy map the caller already has.
///
/// `plan_addresses` places up to `count` copies, and asking `first_free`
/// each time rebuilt the whole `DMX_SLOTS + 1` map from scratch per copy —
/// on the UI thread, every frame the Patch tab is open. One map, reused and
/// marked as copies are placed, does the same work once.
fn first_free_in_map(
    used: &[bool],
    span: u16,
    start: u16,
    same_universe: bool,
) -> Option<u16> {
    let span = span.max(1);
    let last = DMX_SLOTS as u16;
    let mut a = start.max(1);
    while a as usize + span as usize - 1 <= last as usize {
        let end = a + span - 1;
        if same_universe && universe(a) != universe(end) {
            // Straddling the edge: jump to the start of the next universe.
            // This used to be a hard `UNIVERSE + 1`, which only ever meant
            // "universe 2" — past that it is a *backward* jump with no
            // increment, and `first_free` runs on the UI thread every frame
            // the Patch tab is open.
            let next = universe_base(universe(a) + 1);
            if next <= a {
                return None;
            }
            a = next;
            continue;
        }
        if (a..=end).all(|s| !used[s as usize]) {
            return Some(a);
        }
        a += 1;
    }
    None
}

/// The address the universe chip jumps to: the lowest free slot for a
/// `span`-channel fixture *inside* universe `u`, or that universe's own
/// first address when nothing fits in it.
///
/// [`first_free`] scans on to the last slot and, with `same_universe`, hops
/// to the next universe outright — so a question about one universe would
/// otherwise be answered with an address in a later one.
pub(crate) fn first_free_in(
    ranges: &[(u16, u16)],
    span: u16,
    u: u8,
    same_universe: bool,
) -> u16 {
    let base = universe_base(u);
    first_free(ranges, span, base, same_universe)
        .filter(|&a| universe(a) == u)
        .unwrap_or(base)
}

/// Indices of the fixtures a `span`-channel light at `from` would collide
/// with.
pub(crate) fn overlaps(ranges: &[(u16, u16)], from: u16, span: u16) -> Vec<usize> {
    let end = from.saturating_add(span.max(1) - 1);
    ranges
        .iter()
        .enumerate()
        .filter(|(_, (a, b))| *a <= end && from <= *b)
        .map(|(i, _)| i)
        .collect()
}

/// The start address of every copy in the batch.
///
/// Contiguous (the Patch window's rule): each copy runs straight on from
/// the previous one. `skip_taken`: each copy takes the first free hole at
/// or after the cursor instead, hopping over lights that are already
/// patched.
pub(crate) fn plan_addresses(
    ranges: &[(u16, u16)],
    span: u16,
    count: u16,
    start: u16,
    skip_taken: bool,
    same_universe: bool,
) -> (Vec<u16>, PlanStop) {
    let span = span.max(1);
    let mut starts: Vec<u16> = Vec::new();
    if count == 0 {
        return (starts, PlanStop::Complete);
    }
    if skip_taken {
        let mut used = occupancy(ranges);
        let mut cursor = start.max(1);
        for _ in 0..count {
            let Some(a) = first_free_in_map(&used, span, cursor, same_universe) else { break };
            starts.push(a);
            for slot in a..=(a + span - 1).min(DMX_SLOTS as u16) {
                used[slot as usize] = true;
            }
            cursor = a + span;
        }
    } else {
        let mut from = start.max(1);
        for _ in 0..count {
            if same_universe && universe(from) != universe(from + span - 1) {
                let next = universe_base(universe(from) + 1);
                if next <= from {
                    break;
                }
                from = next;
            }
            if from as usize + span as usize - 1 > DMX_SLOTS {
                break;
            }
            starts.push(from);
            from += span;
        }
    }
    let stop = if starts.len() == count as usize {
        PlanStop::Complete
    } else {
        PlanStop::Overflow(starts.len())
    };
    (starts, stop)
}

/// How many of `count` copies fit contiguously before the last slot.
pub(crate) fn fits(from: u16, span: u16, count: u16) -> u16 {
    let span = span.max(1) as usize;
    let mut n = 0;
    let mut a = from.max(1) as usize;
    while n < count && a + span - 1 <= DMX_SLOTS {
        n += 1;
        a += span;
    }
    n
}

/// Which universe an absolute address falls in, counting from 1.
pub(crate) fn universe(addr: u16) -> u8 {
    ((addr.max(1) - 1) / UNIVERSE + 1).min(DMX_UNIVERSES as u16) as u8
}

/// The first absolute address of universe `u`.
pub(crate) fn universe_base(u: u8) -> u16 {
    (u.max(1) as u16 - 1) * UNIVERSE + 1
}

/// Distinct slots in use per universe (an overlap counts once).
pub(crate) fn universe_usage(ranges: &[(u16, u16)]) -> [u16; DMX_UNIVERSES] {
    let used = occupancy(ranges);
    let mut out = [0u16; DMX_UNIVERSES];
    for (slot, on) in used.iter().enumerate().skip(1) {
        if *on {
            out[universe(slot as u16) as usize - 1] += 1;
        }
    }
    out
}

/// "Maverick MK2 Spot (32ch)" → "Maverick MK2 Spot".
pub(crate) fn short_name(profile_name: &str) -> String {
    let t = profile_name.trim();
    if let Some(i) = t.rfind(" (") {
        let tail = &t[i + 2..];
        if tail.ends_with("ch)") && tail[..tail.len() - 3].chars().all(|c| c.is_ascii_digit()) {
            return t[..i].to_string();
        }
    }
    t.to_string()
}

/// The number at the end of a display name ("Spot 12" → 12). `None` when
/// there is none, or when the digits are the whole name ("12").
fn trailing_number(display: &str) -> Option<u16> {
    let t = display.trim_end();
    let digits = t.len() - t.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 || !t[..t.len() - digits].ends_with(' ') {
        return None;
    }
    t[t.len() - digits..].parse().ok()
}

/// Where a new batch of `stem`s starts numbering: one past the highest
/// number already in the rig (a bare "Spot" counts as 1), else 1.
pub(crate) fn next_number(existing: &[&str], stem: &str) -> u16 {
    let stem = stem.trim();
    if stem.is_empty() {
        return 1;
    }
    let mut highest: Option<u16> = None;
    for display in existing {
        let d = display.trim();
        // The bare name is tested first: a stem that itself ends in a digit
        // ("Truss 2", "… Mode 1") has a shorter `type_stem` than itself, so
        // the stem comparison alone would skip its own bare light and
        // restart the series at 1 for ever.
        let n = if d.eq_ignore_ascii_case(stem) {
            1
        } else if type_stem(d).eq_ignore_ascii_case(stem) {
            trailing_number(d).unwrap_or(1)
        } else {
            continue;
        };
        highest = Some(highest.map_or(n, |h: u16| h.max(n)));
    }
    highest.map_or(1, |h| h.saturating_add(1))
}

/// The display names a batch gets: a single first-of-its-name light keeps
/// the bare prefix, everything else is numbered.
pub(crate) fn display_names(base: &str, n: usize, first: u16) -> Vec<String> {
    let base = base.trim();
    if n == 1 && first == 1 {
        return vec![base.to_string()];
    }
    (0..n).map(|i| format!("{base} {}", first as usize + i)).collect()
}

/// A plain-text address sheet for the desk label, sorted by address, with
/// universe-local addresses. A light that straddles 512/513 (only possible
/// with "one universe" off) is printed as both halves, because there is no
/// universe-1 address above 512 to print it in.
pub(crate) fn rig_sheet(fixtures: &[Fixture], label_of: &dyn Fn(&Fixture) -> String) -> String {
    let mut rows: Vec<&Fixture> = fixtures.iter().collect();
    rows.sort_by_key(|f| (f.from, f.display.clone()));
    rows.iter()
        .map(|f| {
            let local = |a: u16| if universe(a) == 1 { a } else { a - UNIVERSE };
            let from = f.from.max(1);
            let to = f.to.min(DMX_SLOTS as u16).max(from);
            let span = if universe(from) == universe(to) {
                format!("U{} {:03}-{:03}", universe(from), local(from), local(to))
            } else {
                format!(
                    "U{} {:03}-{UNIVERSE} / U{} 001-{:03}",
                    universe(from),
                    local(from),
                    universe(to),
                    local(to)
                )
            };
            format!("{:<20} {span}  {}", f.display, label_of(f))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(display: &str, from: u16, span: u16) -> Fixture {
        let p = crate::profiles::PROFILES
            .iter()
            .find(|p| p.channel_count() == span as usize)
            .unwrap_or(&crate::profiles::PROFILES[0]);
        let mut f = p.to_fixture(display.to_string(), from);
        f.to = from + span - 1;
        f
    }

    #[test]
    fn next_free_is_one_for_empty_rig_and_max_to_plus_one_otherwise() {
        assert_eq!(next_free(&[]), 1);
        assert_eq!(next_free(&[(1, 16), (33, 40)]), 41);
        assert_eq!(next_free(&[(DMX_SLOTS as u16 - 7, DMX_SLOTS as u16)]), DMX_SLOTS as u16);
    }

    #[test]
    fn first_free_finds_lowest_gap_and_respects_universe_edge() {
        let r = [(1, 16), (25, 32)];
        assert_eq!(first_free(&r, 8, 1, false), Some(17));
        assert_eq!(first_free(&r, 9, 1, false), Some(33));
        assert_eq!(first_free(&[], 8, 510, true), Some(513));
        assert_eq!(first_free(&[], 8, 510, false), Some(510));
        // Right against the last slot, wherever that now is.
        assert_eq!(first_free(&[], 8, DMX_SLOTS as u16 - 4, false), None);
        assert_eq!(first_free(&[], 8, DMX_SLOTS as u16 - 7, false), Some(DMX_SLOTS as u16 - 7));
        assert_eq!(first_free(&[(1, DMX_SLOTS as u16)], 1, 1, false), None);
    }

    /// `universe` used to be `if addr <= 512 { 1 } else { 2 }`, so every
    /// address above 512 claimed to be in universe 2. Everything else here
    /// is built on it.
    #[test]
    fn an_address_reports_the_universe_it_is_actually_in() {
        assert_eq!(universe(1), 1);
        assert_eq!(universe(UNIVERSE), 1);
        assert_eq!(universe(UNIVERSE + 1), 2);
        for u in 1..=DMX_UNIVERSES as u8 {
            let base = universe_base(u);
            assert_eq!(universe(base), u, "first slot of universe {u}");
            assert_eq!(universe(base + UNIVERSE - 1), u, "last slot of universe {u}");
        }
        assert_eq!(universe(DMX_SLOTS as u16), DMX_UNIVERSES as u8);
        // Past the end it saturates rather than indexing off the end of a
        // per-universe array.
        assert_eq!(universe(u16::MAX), DMX_UNIVERSES as u8);
    }

    /// With `same_universe` on, a span that straddles an edge jumps to the
    /// start of the NEXT universe. It used to jump to a hardcoded 513 —
    /// which past universe 2 is *backwards*, and with no increment that is
    /// an infinite loop on the UI thread, which calls this every frame.
    #[test]
    fn a_straddling_span_moves_forward_and_terminates() {
        // Universes 1 and 2 completely full: the answer must be in 3 (or
        // None on a two-universe build), and it must arrive at all.
        let full = vec![(1, 2 * UNIVERSE)];
        let got = first_free(&full, 32, 1, true);
        if DMX_UNIVERSES >= 3 {
            assert_eq!(got, Some(2 * UNIVERSE + 1));
        } else {
            assert_eq!(got, None);
        }
        // Starting inside the last universe with everything taken simply
        // runs out, rather than looping.
        assert_eq!(first_free(&[(1, DMX_SLOTS as u16)], 4, 1, true), None);
    }

    #[test]
    fn first_free_in_never_leaves_the_universe_it_was_asked_about() {
        // 31 × 16ch fill 1–496, so only 497–512 is left in universe 1.
        let full: Vec<(u16, u16)> = (0..31).map(|i| (1 + i * 16, 16 + i * 16)).collect();
        assert_eq!(first_free_in(&full, 16, 1, true), 497);
        // A 32-channel batch does not fit in what is left, and `first_free`
        // answers a universe-1 question with a universe-2 address.
        assert_eq!(first_free(&full, 32, 1, true), Some(513));
        assert_eq!(first_free_in(&full, 32, 1, true), 1);
        // Universe 1 completely full: still universe 1.
        assert_eq!(first_free_in(&[(1, 512)], 4, 1, true), 1);
        assert_eq!(first_free_in(&[], 32, 2, true), 513);
        // Every universe the console has, not just the first two.
        for u in 1..=DMX_UNIVERSES as u8 {
            assert_eq!(first_free_in(&[], 32, u, true), universe_base(u), "universe {u}");
        }
        assert_eq!(first_free_in(&[(513, 600)], 8, 2, true), 601);
    }

    #[test]
    fn overlaps_lists_every_colliding_range() {
        let r = [(1, 8), (9, 16), (20, 24)];
        assert_eq!(overlaps(&r, 5, 8), vec![0, 1]);
        assert!(overlaps(&r, 17, 3).is_empty());
        assert_eq!(overlaps(&r, 24, 1), vec![2]);
    }

    #[test]
    fn plan_addresses_contiguous_stops_at_slot_limit() {
        // One copy fits before the last slot, the rest do not.
        let near = DMX_SLOTS as u16 - 24;
        let (starts, stop) = plan_addresses(&[], 16, 4, near, false, false);
        assert_eq!((starts.as_slice(), stop), (&[near][..], PlanStop::Overflow(1)));
        let (starts, stop) = plan_addresses(&[], 16, 3, 1, false, false);
        assert_eq!((starts.as_slice(), stop), (&[1u16, 17, 33][..], PlanStop::Complete));
        // An overlapping plan is still a plan: the pill warns, Patch works.
        let (starts, _) = plan_addresses(&[(1, 16)], 16, 2, 1, false, false);
        assert_eq!(starts, vec![1, 17]);
    }

    #[test]
    fn plan_addresses_skip_taken_hops_over_existing_lights() {
        let (starts, stop) = plan_addresses(&[(9, 16)], 8, 3, 1, true, false);
        assert_eq!((starts.as_slice(), stop), (&[1u16, 17, 25][..], PlanStop::Complete));
        // Copies never land on each other either.
        let (starts, _) = plan_addresses(&[], 4, 3, 1, true, false);
        assert_eq!(starts, vec![1, 5, 9]);
        // Nothing free at all.
        let (starts, stop) = plan_addresses(&[(1, DMX_SLOTS as u16)], 4, 2, 1, true, false);
        assert!(starts.is_empty());
        assert_eq!(stop, PlanStop::Overflow(0));
    }

    #[test]
    fn plan_addresses_can_stay_in_one_universe() {
        let (starts, _) = plan_addresses(&[], 16, 2, 500, false, true);
        assert_eq!(starts, vec![513, 529], "a light never straddles 512/513");
        let (starts, _) = plan_addresses(&[], 16, 2, 500, false, false);
        assert_eq!(starts, vec![500, 516]);
    }

    #[test]
    fn fits_stops_at_dmx_slots() {
        assert_eq!(fits(DMX_SLOTS as u16 - 24, 16, 5), 1);
        assert_eq!(fits(DMX_SLOTS as u16, 2, 1), 0);
        assert_eq!(fits(1, 8, 4), 4);
    }

    #[test]
    fn universe_and_usage() {
        assert_eq!(universe(512), 1);
        assert_eq!(universe(513), 2);
        let u = universe_usage(&[(1, 16), (10, 20), (500, 520)]);
        assert_eq!((u[0], u[1]), (33, 8));
        assert_eq!(u.len(), DMX_UNIVERSES);
        assert!(universe_usage(&[]).iter().all(|&n| n == 0));
        // Slots in the upper universes are counted against their own entry,
        // not folded into universe 2.
        let top = universe_usage(&[(DMX_SLOTS as u16 - 3, DMX_SLOTS as u16)]);
        assert_eq!(top[DMX_UNIVERSES - 1], 4);
    }

    #[test]
    fn short_name_strips_channel_suffix() {
        assert_eq!(short_name("Maverick MK2 Spot (32ch)"), "Maverick MK2 Spot");
        assert_eq!(short_name("Fogger"), "Fogger");
        assert_eq!(short_name("Thing (Big)"), "Thing (Big)");
    }

    #[test]
    fn name_stem_and_next_number() {
        assert_eq!(type_stem("Spot 12"), "Spot");
        assert_eq!(type_stem("Spot"), "Spot");
        assert_eq!(next_number(&["Spot 1", "spot 3", "Wash 2"], "Spot"), 4);
        assert_eq!(next_number(&["Spot"], "Spot"), 2);
        assert_eq!(next_number(&[], "Spot"), 1);
        assert_eq!(next_number(&["Wash 9"], "Spot"), 1);
        assert_eq!(next_number(&["Spot 1"], ""), 1);
    }

    #[test]
    fn next_number_counts_a_bare_base_that_ends_in_a_digit() {
        // "Truss 2" is a name, not a series: `type_stem` strips it to
        // "Truss", so the series used to restart at 1 for ever and every
        // press of Patch made another light called "Truss 2".
        assert_eq!(next_number(&["Truss 2"], "Truss 2"), 2);
        assert_eq!(next_number(&["Truss 2", "Truss 2 3"], "Truss 2"), 4);
        assert_eq!(next_number(&["MAC 700"], "MAC 700"), 2);
        // A shorter stem still reads the number off the same name.
        assert_eq!(next_number(&["Truss 2"], "Truss"), 3);
        // And the plain series is untouched.
        assert_eq!(next_number(&["Spot", "Spot 4"], "Spot"), 5);
    }

    #[test]
    fn display_names_numbers_only_batches() {
        assert_eq!(display_names("Spot", 1, 1), vec!["Spot"]);
        assert_eq!(display_names("Spot", 1, 2), vec!["Spot 2"]);
        assert_eq!(display_names("Spot", 3, 7), vec!["Spot 7", "Spot 8", "Spot 9"]);
        assert!(display_names("Spot", 0, 1).is_empty());
    }

    #[test]
    fn rig_sheet_lines_are_sorted_by_address_and_padded() {
        let fixtures = vec![fx("Spot 2", 33, 16), fx("Spot 1", 1, 16), fx("Far", 600, 8)];
        let sheet = rig_sheet(&fixtures, &|f| format!("<{}>", f.display));
        let lines: Vec<&str> = sheet.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("Spot 1"), "{}", lines[0]);
        assert!(lines[0].contains("U1 001-016"), "{}", lines[0]);
        assert!(lines[1].contains("U1 033-048"), "{}", lines[1]);
        assert!(lines[2].contains("U2 088-"), "{}", lines[2]);
        assert!(lines[0].contains("<Spot 1>"));
    }

    #[test]
    fn rig_sheet_splits_a_fixture_that_straddles_the_universe_edge() {
        // Reachable with "one universe" off: 500 + 32ch runs to 531, which
        // is not a universe-1 address at all.
        let sheet = rig_sheet(&[fx("Spot 1", 500, 32)], &|_| "<x>".into());
        assert!(sheet.contains("U1 500-512 / U2 001-019"), "{sheet}");
        // A 16-channel light lands exactly on the edge and does not split.
        let sheet = rig_sheet(&[fx("Spot 1", 497, 16)], &|_| "<x>".into());
        assert!(sheet.contains("U1 497-512  <x>"), "{sheet}");
        assert!(!sheet.contains('/'), "{sheet}");
    }
}
