//! Stored fixture selections — the renamed grandMA3 *Group* pool.
//!
//! A group is just an ordered list of patch-fixture indices. Order is
//! preserved on purpose: later phases (Spread / Phaser) distribute effects
//! *along* the selection, so "which fixture is first" is meaningful.

use serde::{Deserialize, Serialize};

const GROUPS_FILE: &str = "groups.json";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupMode {
    /// Each member receives its own phase position.
    #[default]
    Individual,
    /// Every member moves/changes together as one super-fixture; effects
    /// spread from this group to the next unit instead of within the group.
    AsFixture,
}

impl GroupMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Individual => "Individual fixtures",
            Self::AsFixture => "One fixture",
        }
    }
}

/// A named, ordered selection of patch fixtures.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    /// Patch-fixture indices, in selection order.
    pub fixtures: Vec<usize>,
    #[serde(default)]
    pub mode: GroupMode,
}

/// Load the saved groups (empty if the file is missing or unreadable).
pub fn load_groups() -> Vec<Group> {
    std::fs::read_to_string(GROUPS_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the groups to disk (best-effort).
pub fn save_groups(groups: &[Group]) {
    if let Ok(json) = serde_json::to_string_pretty(groups) {
        let _ = std::fs::write(GROUPS_FILE, json);
    }
}
