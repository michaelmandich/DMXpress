//! Floating "Orders" pool window: build and manage custom effect routes.
//!
//! An order is the sequence a spread travels along, replacing the default
//! patch order. Chain groups together in the Groups window (click, then
//! shift-click) and store that chain here — either one step per light, or one
//! step per group so each group behaves as a single super-fixture.

use std::collections::HashSet;

use eframe::egui;

use super::{apply_zoom, theme, zoom_controls};
use crate::app::App;
use crate::order::{self, Order, OrderStep};

impl App {
    fn fixture_label(&self, fi: usize) -> String {
        self.patch
            .fixtures
            .get(fi)
            .map(|f| f.display.clone())
            .unwrap_or_else(|| format!("fx {fi}"))
    }

    /// Store `steps` as a new order, activate it, and open it for editing.
    fn push_order(&mut self, steps: Vec<OrderStep>) {
        if steps.is_empty() {
            self.log.push("Orders: nothing to store".into());
            return;
        }
        let name = if self.order_name.trim().is_empty() {
            format!("Order {}", self.orders.len() + 1)
        } else {
            self.order_name.trim().to_string()
        };
        self.log
            .push(format!("Stored order \"{name}\" ({} steps)", steps.len()));
        self.orders.push(Order { name, steps });
        order::save_orders(&self.orders);
        self.order_name.clear();
        self.order_edit = Some(self.orders.len() - 1);
        self.active_order = Some(self.orders.len() - 1);
    }

    /// Turn the chain of shift-clicked groups into an order. With `as_units`
    /// each group becomes one step (a super-fixture); otherwise every light
    /// gets its own step, still sequenced group by group. A light already
    /// placed by an earlier group is not repeated.
    pub(crate) fn store_order_from_chain(&mut self, as_units: bool) {
        let chain: Vec<(String, Vec<usize>)> = self
            .group_chain
            .iter()
            .filter_map(|&gi| self.groups.get(gi))
            .map(|g| (g.name.clone(), g.fixtures.clone()))
            .collect();
        if chain.is_empty() {
            self.log
                .push("Orders: click a group, then shift-click more to chain them".into());
            return;
        }
        let mut placed: HashSet<usize> = HashSet::new();
        let mut steps: Vec<OrderStep> = Vec::new();
        for (name, fixtures) in chain {
            let fresh: Vec<usize> = fixtures
                .into_iter()
                .filter(|fi| placed.insert(*fi))
                .collect();
            if fresh.is_empty() {
                continue;
            }
            if as_units {
                steps.push(OrderStep {
                    label: name,
                    fixtures: fresh,
                });
            } else {
                for fi in fresh {
                    steps.push(OrderStep {
                        label: format!("{name} · {}", self.fixture_label(fi)),
                        fixtures: vec![fi],
                    });
                }
            }
        }
        self.push_order(steps);
    }

    /// Store the raw selection as an order, one step per light in patch order.
    pub(crate) fn store_order_from_selection(&mut self) {
        let steps: Vec<OrderStep> = self
            .stage
            .selected_fixtures()
            .into_iter()
            .map(|fi| OrderStep {
                label: self.fixture_label(fi),
                fixtures: vec![fi],
            })
            .collect();
        if steps.is_empty() {
            self.log.push("Orders: select fixtures first".into());
            return;
        }
        self.push_order(steps);
    }

    /// Break every multi-light step of the active order that touches
    /// `fixtures` back into one step per light, so those lights can be picked
    /// individually again. Returns how many steps were split.
    pub(crate) fn release_from_order(&mut self, fixtures: &[usize]) -> usize {
        let Some(oi) = self.active_order else {
            return 0;
        };
        let Some(order) = self.orders.get(oi) else {
            return 0;
        };
        let touched: HashSet<usize> = fixtures.iter().copied().collect();
        let mut steps: Vec<OrderStep> = Vec::new();
        let mut split = 0;
        for step in &order.steps {
            if step.is_unit() && step.fixtures.iter().any(|fi| touched.contains(fi)) {
                split += 1;
                for &fi in &step.fixtures {
                    steps.push(OrderStep {
                        label: self.fixture_label(fi),
                        fixtures: vec![fi],
                    });
                }
            } else {
                steps.push(step.clone());
            }
        }
        if split == 0 {
            return 0;
        }
        let name = self.orders[oi].name.clone();
        self.orders[oi].steps = steps;
        order::save_orders(&self.orders);
        self.log.push(format!(
            "Order \"{name}\": {split} step(s) split back into single lights"
        ));
        split
    }

    /// Select every fixture the order touches.
    pub(crate) fn recall_order(&mut self, idx: usize) {
        let Some(fixtures) = self.orders.get(idx).map(|o| o.fixtures()) else {
            return;
        };
        self.stage.selection.clear();
        self.stage.sel_tower = None;
        for &fi in &fixtures {
            self.stage.select_fixture(fi, true);
        }
        self.sel_fixture = fixtures.first().copied();
        self.stage.last_selected = fixtures.first().copied();
    }

    fn delete_order(&mut self, idx: usize) {
        if idx >= self.orders.len() {
            return;
        }
        let o = self.orders.remove(idx);
        order::save_orders(&self.orders);
        let shift = |slot: &mut Option<usize>| match *slot {
            Some(i) if i == idx => *slot = None,
            Some(i) if i > idx => *slot = Some(i - 1),
            _ => {}
        };
        shift(&mut self.active_order);
        shift(&mut self.order_edit);
        self.log.push(format!("Deleted order \"{}\"", o.name));
    }

    pub(crate) fn orders_window(&mut self, ctx: &egui::Context) {
        if !self.show_orders {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_orders;
        // Deferred actions so the pool is never mutated mid-iteration.
        let mut do_store_steps = false;
        let mut do_store_units = false;
        let mut do_store_sel = false;
        let mut do_clear_chain = false;
        let mut do_activate: Option<Option<usize>> = None;
        let mut do_edit: Option<usize> = None;
        let mut do_recall: Option<usize> = None;
        let mut do_reverse: Option<usize> = None;
        let mut do_delete: Option<usize> = None;
        let mut do_split_all = false;
        let mut do_step: Option<(usize, usize, StepAction)> = None;

        // The active order overrules group modes: any multi-light step welds
        // its lights together, so the Groups window can still read
        // "Individual fixtures" while those lights move as one.
        let bound = self
            .active_order
            .and_then(|i| self.orders.get(i))
            .map(|o| {
                let units: Vec<&OrderStep> = o.steps.iter().filter(|s| s.is_unit()).collect();
                let lights: usize = units.iter().map(|s| s.fixtures.len()).sum();
                (o.name.clone(), units.len(), lights)
            })
            .filter(|&(_, steps, _)| steps > 0);

        egui::Window::new("Orders")
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .default_size([380.0, 460.0])
            .default_pos([screen.right() - 420.0, 200.0])
            .show(ctx, |ui| {
                zoom_controls(ui, &mut self.zoom.orders);
                apply_zoom(ui, self.zoom.orders);

                theme::section(ui, "Chain");
                if self.group_chain.is_empty() {
                    theme::hint(
                        ui,
                        "In Groups, click a group then shift-click more. \
                         The click order becomes the route.",
                    );
                } else {
                    ui.horizontal_wrapped(|ui| {
                        for (k, &gi) in self.group_chain.iter().enumerate() {
                            let name = self
                                .groups
                                .get(gi)
                                .map(|g| g.name.as_str())
                                .unwrap_or("?");
                            theme::pill(ui, &format!("{}  {name}", k + 1), theme::ACCENT_SOFT);
                        }
                        if ui.small_button("Clear").clicked() {
                            do_clear_chain = true;
                        }
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.order_name)
                            .hint_text("name…")
                            .desired_width(130.0),
                    );
                    let has_chain = !self.group_chain.is_empty();
                    if ui
                        .add_enabled(has_chain, egui::Button::new("Store lights"))
                        .on_hover_text("One step per light, sequenced group by group")
                        .clicked()
                    {
                        do_store_steps = true;
                    }
                    if ui
                        .add_enabled(has_chain, egui::Button::new("Store groups as fixtures"))
                        .on_hover_text(
                            "One step per group — each group takes a single phase \
                             slot and moves as one light",
                        )
                        .clicked()
                    {
                        do_store_units = true;
                    }
                });
                if ui
                    .button("Store selection in patch order")
                    .on_hover_text("Ignore the chain and use the raw selection")
                    .clicked()
                {
                    do_store_sel = true;
                }

                ui.add_space(6.0);
                theme::section(ui, "Orders");
                ui.horizontal_wrapped(|ui| {
                    let patch_order = ui
                        .selectable_label(self.active_order.is_none(), "Patch order")
                        .on_hover_text("Effects fan out by fixture number");
                    if patch_order.clicked() {
                        do_activate = Some(None);
                    }
                    for (i, o) in self.orders.iter().enumerate() {
                        let units = o.steps.iter().filter(|s| s.is_unit()).count();
                        let suffix = if units > 0 {
                            format!(" · {units} unit")
                        } else {
                            String::new()
                        };
                        let label = format!("{}\n{} steps{suffix}", o.name, o.steps.len());
                        let resp = ui.add_sized(
                            [110.0, 44.0],
                            egui::SelectableLabel::new(self.active_order == Some(i), label),
                        );
                        if resp.clicked() {
                            do_activate = Some(Some(i));
                            do_edit = Some(i);
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Edit").clicked() {
                                do_edit = Some(i);
                                ui.close_menu();
                            }
                            if ui.button("Select its fixtures").clicked() {
                                do_recall = Some(i);
                                ui.close_menu();
                            }
                            if ui.button("Reverse").clicked() {
                                do_reverse = Some(i);
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Delete").clicked() {
                                do_delete = Some(i);
                                ui.close_menu();
                            }
                        });
                    }
                });
                if self.orders.is_empty() {
                    theme::hint(ui, "No orders yet — chain some groups and store them.");
                }

                if let Some((name, steps, lights)) = &bound {
                    ui.add_space(6.0);
                    egui::Frame::none()
                        .fill(theme::WELL)
                        .stroke(egui::Stroke::new(1.0, theme::WARN))
                        .rounding(4.0)
                        .inner_margin(7.0)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                theme::pill(ui, "HEADS UP", theme::WARN);
                                ui.colored_label(
                                    theme::WARN,
                                    format!(
                                        "\"{name}\" folds {lights} lights into {steps} step(s)."
                                    ),
                                );
                            });
                            theme::hint(
                                ui,
                                "While it is active you cannot select those lights on their \
                                 own — clicking one takes the whole step. The Groups window \
                                 may still show them as individual fixtures; the order wins.",
                            );
                            if ui
                                .button("Split every folded step")
                                .on_hover_text(
                                    "Give each light its own step so they can be picked apart again",
                                )
                                .clicked()
                            {
                                do_split_all = true;
                            }
                        });
                }

                if let Some(idx) = self.order_edit.filter(|&i| i < self.orders.len()) {
                    ui.add_space(6.0);
                    theme::section(ui, &format!("Editing {}", self.orders[idx].name));
                    theme::hint(
                        ui,
                        "Merge folds a step into the one above it, so they share \
                         a phase and count as one light.",
                    );
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            let last = self.orders[idx].steps.len().saturating_sub(1);
                            for (k, step) in self.orders[idx].steps.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.label(format!("{}", k + 1));
                                    if step.is_unit() {
                                        theme::pill(
                                            ui,
                                            &format!("{} fx", step.fixtures.len()),
                                            theme::ACCENT_SOFT,
                                        );
                                    }
                                    ui.label(&step.label);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.small_button("x").on_hover_text("Remove").clicked()
                                            {
                                                do_step = Some((idx, k, StepAction::Remove));
                                            }
                                            if ui
                                                .add_enabled(
                                                    step.is_unit(),
                                                    egui::Button::new("split").small(),
                                                )
                                                .on_hover_text("Give each light its own step")
                                                .clicked()
                                            {
                                                do_step = Some((idx, k, StepAction::Split));
                                            }
                                            if ui
                                                .add_enabled(
                                                    k > 0,
                                                    egui::Button::new("merge").small(),
                                                )
                                                .on_hover_text("Fold into the step above")
                                                .clicked()
                                            {
                                                do_step = Some((idx, k, StepAction::MergeUp));
                                            }
                                            if ui
                                                .add_enabled(k < last, egui::Button::new("v").small())
                                                .clicked()
                                            {
                                                do_step = Some((idx, k, StepAction::Down));
                                            }
                                            if ui
                                                .add_enabled(k > 0, egui::Button::new("^").small())
                                                .clicked()
                                            {
                                                do_step = Some((idx, k, StepAction::Up));
                                            }
                                        },
                                    );
                                });
                            }
                        });
                }
            });
        self.show_orders = open;

        if do_clear_chain {
            self.group_chain.clear();
        }
        if do_store_steps {
            self.store_order_from_chain(false);
        }
        if do_store_units {
            self.store_order_from_chain(true);
        }
        if do_store_sel {
            self.store_order_from_selection();
        }
        if let Some(sel) = do_activate {
            self.active_order = sel;
            let what = sel
                .and_then(|i| self.orders.get(i))
                .map(|o| o.name.clone())
                .unwrap_or_else(|| "patch order".into());
            self.log.push(format!("Effects follow {what}"));
        }
        if let Some(i) = do_edit {
            self.order_edit = Some(i);
        }
        if let Some(i) = do_recall {
            self.recall_order(i);
        }
        if let Some(i) = do_reverse {
            if let Some(o) = self.orders.get_mut(i) {
                o.steps.reverse();
                order::save_orders(&self.orders);
            }
        }
        if let Some(i) = do_delete {
            self.delete_order(i);
        }
        if do_split_all {
            let fixtures: Vec<usize> = self.order_bound_fixtures().into_iter().collect();
            self.release_from_order(&fixtures);
        }
        if let Some((i, k, action)) = do_step {
            self.edit_step(i, k, action);
        }
    }

    fn edit_step(&mut self, order: usize, k: usize, action: StepAction) {
        let Some(o) = self.orders.get_mut(order) else {
            return;
        };
        if k >= o.steps.len() {
            return;
        }
        match action {
            StepAction::Up if k > 0 => o.steps.swap(k, k - 1),
            StepAction::Down if k + 1 < o.steps.len() => o.steps.swap(k, k + 1),
            StepAction::Remove => {
                o.steps.remove(k);
            }
            StepAction::MergeUp if k > 0 => {
                let step = o.steps.remove(k);
                let prev = &mut o.steps[k - 1];
                prev.fixtures.extend(step.fixtures);
                prev.label = format!("{} + {}", prev.label, step.label);
            }
            StepAction::Split => {
                let step = o.steps.remove(k);
                for (n, fi) in step.fixtures.into_iter().enumerate() {
                    o.steps.insert(
                        k + n,
                        OrderStep {
                            label: format!("{} {}", step.label, n + 1),
                            fixtures: vec![fi],
                        },
                    );
                }
            }
            _ => {}
        }
        order::save_orders(&self.orders);
    }
}

#[derive(Clone, Copy)]
enum StepAction {
    Up,
    Down,
    MergeUp,
    Split,
    Remove,
}
