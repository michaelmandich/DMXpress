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
    pub fixtures: Vec<usize>,
}

impl OrderStep {
    /// Several fixtures acting as one light.
    pub fn is_unit(&self) -> bool {
        self.fixtures.len() > 1
    }
}

/// A named sequence of steps.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Order {
    pub name: String,
    pub steps: Vec<OrderStep>,
}

impl Order {
    /// Every fixture the order touches, in step order.
    pub fn fixtures(&self) -> Vec<usize> {
        self.steps
            .iter()
            .flat_map(|s| s.fixtures.iter().copied())
            .collect()
    }
}

/// Load the saved orders (empty if the file is missing or unreadable).
pub fn load_orders() -> Vec<Order> {
    std::fs::read_to_string(ORDERS_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the orders to disk (best-effort).
pub fn save_orders(orders: &[Order]) {
    if let Ok(json) = serde_json::to_string_pretty(orders) {
        let _ = std::fs::write(ORDERS_FILE, json);
    }
}
