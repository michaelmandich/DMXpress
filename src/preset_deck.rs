//! The Presets board's pads — also the Stream Deck's Presets page.
use serde::{Deserialize, Serialize};

use crate::streamdeck::KeyIcon;

/// One pad: a native preset by id (or a ShowBuddy preset by name) and how it is drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetSlot {
    /// `UserPreset.id`; 0 when `bank` is set.
    pub preset: u32,
    /// A ShowBuddy preset instead: (bank name, preset name).
    #[serde(default)]
    pub bank: Option<(String, String)>,
    /// The preset's name when placed — the face shown if the id no longer resolves.
    #[serde(default)]
    pub name: String,
    pub color: [u8; 3],
    pub label: String,
    #[serde(default)]
    pub icon: KeyIcon,
}

/// Slots a page of the Presets board stores.
pub const PRESET_DECK_SLOTS: usize = 36;
const PRESET_DECK_FILE: &str = "preset_deck.json";

/// How many of those slots a page can actually show. The board and the
/// deck's Presets page are the same 4 × 9 grid, and it reserves three keys
/// (Select all, Clear, Tap), so the last three slots of every page have no
/// key anywhere. Writers clamp to this; the store stays a round 36 so the
/// page arithmetic (`idx / PRESET_DECK_SLOTS`) is unchanged.
pub fn preset_pads_per_page() -> usize {
    crate::streamdeck::page_pad_count(crate::ui::board::ROWS, crate::ui::board::COLS)
}

/// Whether slot `k` of the whole board sits on a key that exists.
fn slot_is_reachable(k: usize) -> bool {
    k % PRESET_DECK_SLOTS < preset_pads_per_page()
}

/// Read the board from disk (empty when absent or unreadable), padded to
/// whole pages.
pub fn load_preset_deck() -> Vec<Option<PresetSlot>> {
    let mut slots: Vec<Option<PresetSlot>> = std::fs::read_to_string(crate::paths::data_path(PRESET_DECK_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    pad_preset_deck(&mut slots);
    slots
}

/// Grow the board to whole pages: at least one, never a partial one.
pub fn pad_preset_deck(slots: &mut Vec<Option<PresetSlot>>) {
    slots.resize(preset_deck_pages(slots) * PRESET_DECK_SLOTS, None);
}

/// How many pages of pads the board holds (never fewer than one).
pub fn preset_deck_pages(slots: &[Option<PresetSlot>]) -> usize {
    slots.len().div_ceil(PRESET_DECK_SLOTS).max(1)
}

/// The first empty pad from `from_page`'s first pad onwards, wrapping round
/// to the earlier pages; `None` when every pad is taken. Slots with no key
/// on them ([`preset_pads_per_page`]) are never offered.
pub fn first_free_slot(slots: &[Option<PresetSlot>], from_page: usize) -> Option<usize> {
    let start = (from_page * PRESET_DECK_SLOTS).min(slots.len());
    (start..slots.len())
        .chain(0..start)
        .find(|&k| slots[k].is_none() && slot_is_reachable(k))
}

/// Write the board; errors are swallowed like every other state file.
pub fn save_preset_deck(slots: &[Option<PresetSlot>]) {
    if let Ok(json) = serde_json::to_string_pretty(slots) {
        let _ = std::fs::write(crate::paths::data_path(PRESET_DECK_FILE), json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(label: &str) -> Option<PresetSlot> {
        Some(PresetSlot {
            preset: 1,
            bank: None,
            name: label.to_owned(),
            color: [10, 20, 30],
            label: label.to_owned(),
            icon: KeyIcon::None,
        })
    }

    /// The board is always a whole number of pages, at least one.
    #[test]
    fn pad_and_pages() {
        let mut slots: Vec<Option<PresetSlot>> = Vec::new();
        assert_eq!(preset_deck_pages(&slots), 1);
        pad_preset_deck(&mut slots);
        assert_eq!(slots.len(), PRESET_DECK_SLOTS);

        let mut slots: Vec<Option<PresetSlot>> = vec![None; PRESET_DECK_SLOTS + 1];
        assert_eq!(preset_deck_pages(&slots), 2);
        pad_preset_deck(&mut slots);
        assert_eq!(slots.len(), 2 * PRESET_DECK_SLOTS);
        assert_eq!(preset_deck_pages(&slots), 2);
    }

    /// A new pad lands on the page you are looking at when it has room,
    /// and only then falls back to an earlier page.
    #[test]
    fn first_free_slot_prefers_current_page_then_earlier() {
        let mut slots: Vec<Option<PresetSlot>> = (0..2 * PRESET_DECK_SLOTS).map(|_| slot("x")).collect();
        slots[2] = None;
        slots[PRESET_DECK_SLOTS + 4] = None;
        assert_eq!(first_free_slot(&slots, 1), Some(PRESET_DECK_SLOTS + 4));
        assert_eq!(first_free_slot(&slots, 0), Some(2));
        slots[2] = slot("x");
        // Only the later page is free now: page 0 wraps forward to find it.
        assert_eq!(first_free_slot(&slots, 0), Some(PRESET_DECK_SLOTS + 4));
        slots[PRESET_DECK_SLOTS + 4] = slot("x");
        assert_eq!(first_free_slot(&slots, 0), None);
        assert_eq!(first_free_slot(&slots, 1), None);
    }

    /// A page stores 36 slots, but the 4 × 9 grid it is drawn on reserves
    /// three keys for Select all, Clear and Tap — so the last three slots of
    /// every page have no key on the board or on the deck. A new pad must
    /// never land in one: it would be saved, counted as "on the board" and
    /// be nowhere at all.
    #[test]
    fn first_free_slot_never_offers_a_slot_with_no_key_on_it() {
        let pads = preset_pads_per_page();
        assert!(pads > 0 && pads < PRESET_DECK_SLOTS, "{pads} pads of {PRESET_DECK_SLOTS}");
        let mut slots: Vec<Option<PresetSlot>> = vec![None; 2 * PRESET_DECK_SLOTS];
        for s in slots.iter_mut().take(pads) {
            *s = slot("x");
        }
        // Page 0's pads are full, so the next free one is page 1's first —
        // not one of page 0's three unreachable slots.
        assert_eq!(first_free_slot(&slots, 0), Some(PRESET_DECK_SLOTS));
        for s in slots.iter_mut().skip(PRESET_DECK_SLOTS).take(pads) {
            *s = slot("x");
        }
        assert_eq!(first_free_slot(&slots, 0), None);
        assert_eq!(first_free_slot(&slots, 1), None);
        assert!(slots[pads..PRESET_DECK_SLOTS].iter().all(Option::is_none));
    }

    /// A pad written by an older build (no `bank` / `name` / `icon` keys)
    /// still loads, with the defaults.
    #[test]
    fn slot_json_round_trips_without_optional_keys() {
        let json = r#"{"preset":7,"color":[1,2,3],"label":"WARM"}"#;
        let s: PresetSlot = serde_json::from_str(json).unwrap();
        assert_eq!(s.preset, 7);
        assert_eq!(s.bank, None);
        assert_eq!(s.name, "");
        assert_eq!(s.icon, KeyIcon::None);
        assert_eq!(s.label, "WARM");
        let back: PresetSlot = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
