//! Custom effect orders — the route an effect travels along.
//!
//! By default a spread fans out in patch order: fixture 0, then 1, then 2.
//! An *order* replaces that with an explicit sequence of steps, and a step may
//! hold several fixtures at once, in which case they all receive the same
//! phase and the step counts as a single light. That makes an order the one
//! place that answers "what comes next, and what moves together" — which also
//! settles the ambiguity of a fixture belonging to several groups.

use serde::{Deserialize, Serialize};

const ORDERS_FILE: &str = "orders.json";

/// One position along an order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrderStep {
    /// Where the step came from — a group name, or a fixture name.
    pub label: String,
    /// Patch-fixture indices sitting on this step. More than one makes the
    /// step a super-fixture: every member shares the step's phase.
    ///
    /// For a group-backed step this is the membership as it stood when the
    /// step was stored; [`OrderStep::members`] prefers the group's live one.
    pub fixtures: Vec<usize>,
    /// The group this step stands for, if it was built from one. Keeping the
    /// link (rather than only the copied fixtures the pool button used to
    /// leave behind) is what lets "each group of 8 is one step" survive
    /// editing that group.
    #[serde(default)]
    pub group: Option<u32>,
}

impl OrderStep {
    /// A plain step over an explicit fixture list.
    pub fn from_fixtures(label: String, fixtures: Vec<usize>) -> Self {
        Self {
            label,
            fixtures,
            group: None,
        }
    }

    /// A step that stands for a whole group and follows its edits.
    pub fn from_group(group: &crate::group::Group) -> Self {
        Self {
            label: group.name.clone(),
            fixtures: group.fixtures.clone(),
            group: Some(group.id),
        }
    }

    /// The fixtures on this step right now: the linked group's current
    /// membership when it still exists, else the stored list. A group that
    /// has been deleted leaves the step frozen at its last known members
    /// rather than silently emptying the order.
    pub fn members<'a>(&'a self, groups: &'a [crate::group::Group]) -> &'a [usize] {
        if let Some(id) = self.group {
            if let Some(g) = groups.iter().find(|g| g.id == id) {
                return &g.fixtures;
            }
        }
        &self.fixtures
    }

    /// Several fixtures acting as one light.
    pub fn is_unit(&self) -> bool {
        self.fixtures.len() > 1
    }
}

/// A named sequence of steps.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Order {
    /// Stable identity, so a layer can remember the route it travels along
    /// across pool edits (see [`crate::group::Group::id`]).
    #[serde(default)]
    pub id: u32,
    pub name: String,
    pub steps: Vec<OrderStep>,
}

impl Order {
    /// Every fixture the order touches, in step order, resolving
    /// group-backed steps against the live pool.
    pub fn resolved_fixtures(&self, groups: &[crate::group::Group]) -> Vec<usize> {
        self.steps
            .iter()
            .flat_map(|s| s.members(groups).iter().copied())
            .collect()
    }
}

/// The first id not already taken (see [`crate::group::next_id`]).
pub fn next_id(orders: &[Order]) -> u32 {
    orders.iter().map(|o| o.id).max().unwrap_or(0) + 1
}

/// Hand an id to every order saved before ids existed.
pub fn assign_ids(orders: &mut [Order]) {
    let mut next = orders.iter().map(|o| o.id).max().unwrap_or(0) + 1;
    for o in orders.iter_mut() {
        if o.id == 0 {
            o.id = next;
            next += 1;
        }
    }
}

/// Load the saved orders (empty if the file is missing or unreadable).
pub fn load_orders() -> Vec<Order> {
    let mut orders: Vec<Order> = std::fs::read_to_string(ORDERS_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    assign_ids(&mut orders);
    orders
}

/// Persist the orders to disk (best-effort).
pub fn save_orders(orders: &[Order]) {
    if let Ok(json) = serde_json::to_string_pretty(orders) {
        let _ = std::fs::write(ORDERS_FILE, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::{Group, GroupMode};

    fn group(id: u32, fixtures: Vec<usize>) -> Group {
        Group {
            id,
            name: format!("g{id}"),
            fixtures,
            mode: GroupMode::AsFixture,
        }
    }

    /// A group-backed step follows the group, so re-cutting a group moves
    /// every route built from it instead of leaving stale copies behind.
    #[test]
    fn a_group_step_follows_the_group() {
        let mut groups = vec![group(7, vec![1, 2, 3])];
        let step = OrderStep::from_group(&groups[0]);
        assert_eq!(step.members(&groups), &[1, 2, 3]);
        groups[0].fixtures = vec![4, 5];
        assert_eq!(step.members(&groups), &[4, 5]);
    }

    /// A deleted group leaves its step frozen at the last known members
    /// rather than silently emptying the order mid-show.
    #[test]
    fn a_deleted_group_leaves_the_step_standing() {
        let step = OrderStep::from_group(&group(7, vec![1, 2]));
        assert_eq!(step.members(&[]), &[1, 2]);
    }

    /// Plain steps are untouched by the pool.
    #[test]
    fn a_plain_step_ignores_the_groups() {
        let step = OrderStep::from_fixtures("x".into(), vec![9]);
        assert_eq!(step.members(&[group(1, vec![1, 2])]), &[9]);
    }

    /// Ids are handed out above whatever a show restored, and legacy
    /// entries (`id: 0`) are filled in without colliding.
    #[test]
    fn ids_fill_in_without_colliding() {
        let mut orders = vec![
            Order { id: 0, name: "old".into(), steps: vec![] },
            Order { id: 5, name: "new".into(), steps: vec![] },
            Order { id: 0, name: "older".into(), steps: vec![] },
        ];
        assign_ids(&mut orders);
        let ids: Vec<u32> = orders.iter().map(|o| o.id).collect();
        assert_eq!(ids, vec![6, 5, 7]);
        assert_eq!(next_id(&orders), 8);
    }
}
