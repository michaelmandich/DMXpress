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
use crate::stage::{Sweep, SweepShape};

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
        let id = order::next_id(&self.orders);
        self.orders.push(Order { id, name, steps });
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
        let chain: Vec<(u32, String, Vec<usize>)> = self
            .group_chain
            .iter()
            .filter_map(|&gi| self.groups.get(gi))
            .map(|g| (g.id, g.name.clone(), g.fixtures.clone()))
            .collect();
        if chain.is_empty() {
            self.log
                .push("Orders: click a group, then shift-click more to chain them".into());
            return;
        }
        let mut placed: HashSet<usize> = HashSet::new();
        let mut steps: Vec<OrderStep> = Vec::new();
        for (id, name, fixtures) in chain {
            let whole = fixtures.len();
            let fresh: Vec<usize> = fixtures
                .into_iter()
                .filter(|fi| placed.insert(*fi))
                .collect();
            if fresh.is_empty() {
                continue;
            }
            if as_units {
                // Only a step that still stands for the *whole* group may
                // follow it: once an earlier group has claimed some of its
                // lights the step is a subset, and re-reading the group
                // later would silently pull those lights back in.
                let group = (fresh.len() == whole).then_some(id);
                steps.push(OrderStep {
                    label: name,
                    fixtures: fresh,
                    group,
                });
            } else {
                for fi in fresh {
                    steps.push(OrderStep::from_fixtures(
                        format!("{name} · {}", self.fixture_label(fi)),
                        vec![fi],
                    ));
                }
            }
        }
        self.push_order(steps);
    }

    /// Turn the sweep draft into an order: one step per sweep, in the order
    /// they were drawn.
    ///
    /// A multi-light step is a folded step — its lights share one phase and
    /// the step counts as a single light along the route — which is exactly
    /// what "every light in this group gets the same value" means once an
    /// effect fans down the order.
    pub(crate) fn store_order_from_sweep(&mut self) {
        let Some(sw) = &self.stage.sweep else {
            return;
        };
        let also_groups = sw.also_groups;
        // Instance indices are what the stage drags over; orders and groups
        // both speak patch-fixture indices. Dedupe within a step but keep
        // the sweep's sequence — that ordering is the whole point and is
        // destroyed by anything that routes through the selection.
        let mut placed: HashSet<usize> = HashSet::new();
        let mut sweeps: Vec<Vec<usize>> = Vec::new();
        for step in &sw.steps {
            let mut fixtures: Vec<usize> = Vec::new();
            for &inst in step {
                let Some(fi) = self.stage.instances.get(inst).map(|i| i.fixture) else {
                    continue;
                };
                if placed.insert(fi) {
                    fixtures.push(fi);
                }
            }
            if !fixtures.is_empty() {
                sweeps.push(fixtures);
            }
        }
        if sweeps.is_empty() {
            self.log.push("Sweep: nothing caught yet".into());
            return;
        }
        let base = if self.order_name.trim().is_empty() {
            format!("Sweep {}", self.orders.len() + 1)
        } else {
            self.order_name.trim().to_string()
        };
        let mut steps: Vec<OrderStep> = Vec::new();
        for (k, fixtures) in sweeps.into_iter().enumerate() {
            let label = format!("{base} {}", k + 1);
            if also_groups {
                // Store the sweep as a group and point the step at it, so
                // editing the group later moves the route with it.
                let g = crate::group::Group {
                    id: crate::group::next_id(&self.groups),
                    name: label,
                    fixtures,
                    mode: crate::group::GroupMode::AsFixture,
                };
                steps.push(OrderStep::from_group(&g));
                self.groups.push(g);
            } else {
                steps.push(OrderStep::from_fixtures(label, fixtures));
            }
        }
        if also_groups {
            crate::group::save_groups(&self.groups);
        }
        self.push_order(steps);
        if let Some(sw) = &mut self.stage.sweep {
            sw.steps.clear();
        }
    }

    /// Store the raw selection as an order, one step per light in patch order.
    pub(crate) fn store_order_from_selection(&mut self) {
        let steps: Vec<OrderStep> = self
            .stage
            .selected_fixtures()
            .into_iter()
            .map(|fi| OrderStep::from_fixtures(self.fixture_label(fi), vec![fi]))
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
                    steps.push(OrderStep::from_fixtures(self.fixture_label(fi), vec![fi]));
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
        let Some(fixtures) = self
            .orders
            .get(idx)
            .map(|o| o.resolved_fixtures(&self.groups))
        else {
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
        let mut do_sweep_store = false;
        let mut do_sweep_undo = false;
        let mut do_sweep_clear = false;
        // The tool's own state lives on the stage, where the dragging
        // happens; the window only reads and sets it.
        let sweep_on = self.stage.sweep.is_some();
        let sweep_shape = self.stage.sweep.as_ref().map(|s| s.shape).unwrap_or_default();
        let sweep_groups = self.stage.sweep.as_ref().is_some_and(|s| s.also_groups);
        let sweep_steps = self.stage.sweep.as_ref().map_or(0, |s| s.steps.len());
        let sweep_lights = self.stage.sweep.as_ref().map_or(0, |s| s.lights());
        let mut set_shape: Option<SweepShape> = None;
        let mut set_groups: Option<bool> = None;
        let mut set_armed: Option<bool> = None;

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
                // A bounded row: a bare right-to-left layout would claim the
                // window's whole remaining height.
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        zoom_controls(ui, &mut self.zoom.orders);
                    });
                });
                apply_zoom(ui, self.zoom.orders);

                theme::section(ui, "Sweep tool");
                ui.horizontal(|ui| {
                    let mut armed = sweep_on;
                    if ui
                        .selectable_label(armed, if armed { "◉ Sweeping" } else { "Arm sweep" })
                        .on_hover_text(
                            "Drag shapes over the rig. Each sweep becomes the next step of                              a route, in the order you draw them.",
                        )
                        .clicked()
                    {
                        armed = !armed;
                        set_armed = Some(armed);
                    }
                    ui.add_enabled_ui(sweep_on, |ui| {
                        for sh in [SweepShape::Rect, SweepShape::Circle] {
                            if ui
                                .selectable_label(sweep_shape == sh, sh.label())
                                .clicked()
                            {
                                set_shape = Some(sh);
                            }
                        }
                    });
                });
                if sweep_on {
                    theme::hint(
                        ui,
                        "Drag on empty stage to sweep · ⇧ on release adds the catch to the                          step you just laid, for a step made of separate patches of rig.",
                    );
                    ui.horizontal_wrapped(|ui| {
                        theme::pill(
                            ui,
                            &format!("{sweep_steps} step(s) · {sweep_lights} light(s)"),
                            if sweep_steps > 0 { theme::ACCENT_SOFT } else { theme::WARN },
                        );
                        let mut g = sweep_groups;
                        if ui
                            .checkbox(&mut g, "also save groups")
                            .on_hover_text(
                                "Store each sweep as a named group too, and keep the step                                  pointing at it so editing the group updates the route.",
                            )
                            .changed()
                        {
                            set_groups = Some(g);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(sweep_steps > 0, egui::Button::new("Store as order"))
                            .clicked()
                        {
                            do_sweep_store = true;
                        }
                        if ui
                            .add_enabled(sweep_steps > 0, egui::Button::new("Undo sweep"))
                            .clicked()
                        {
                            do_sweep_undo = true;
                        }
                        if ui
                            .add_enabled(sweep_steps > 0, egui::Button::new("Clear"))
                            .clicked()
                        {
                            do_sweep_clear = true;
                        }
                    });
                }
                ui.separator();

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

        if let Some(armed) = set_armed {
            self.stage.sweep = armed.then(Sweep::default);
            if armed {
                self.log
                    .push("Sweep tool armed — drag over the rig to lay a route".into());
            }
        }
        if let Some(sh) = set_shape {
            if let Some(sw) = &mut self.stage.sweep {
                sw.shape = sh;
            }
        }
        if let Some(g) = set_groups {
            if let Some(sw) = &mut self.stage.sweep {
                sw.also_groups = g;
            }
        }
        if do_sweep_undo {
            if let Some(sw) = &mut self.stage.sweep {
                sw.steps.pop();
            }
        }
        if do_sweep_clear {
            if let Some(sw) = &mut self.stage.sweep {
                sw.steps.clear();
            }
        }
        if do_sweep_store {
            self.store_order_from_sweep();
        }
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
        // Resolve group-backed steps against the pool *before* taking the
        // mutable borrow on `self.orders`: an edit rewrites a step's fixture
        // list, and it has to rewrite the membership the group holds now, not
        // the copy the step was stored with.
        let live_members = |step: Option<&OrderStep>| -> Option<Vec<usize>> {
            step.map(|s| s.members(&self.groups).to_vec())
        };
        let this_members = live_members(self.orders.get(order).and_then(|o| o.steps.get(k)));
        let prev_members = k
            .checked_sub(1)
            .and_then(|p| live_members(self.orders.get(order).and_then(|o| o.steps.get(p))));
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
                let moved = this_members.unwrap_or(step.fixtures);
                let prev = &mut o.steps[k - 1];
                // Two steps folded together are no longer either group, so
                // the merged step takes the fixtures and drops the link.
                prev.group = None;
                if let Some(live) = prev_members {
                    prev.fixtures = live;
                }
                prev.fixtures.extend(moved);
                prev.fixtures.dedup();
                prev.label = format!("{} + {}", prev.label, step.label);
            }
            StepAction::Split => {
                let step = o.steps.remove(k);
                let members = this_members.unwrap_or(step.fixtures);
                for (n, fi) in members.into_iter().enumerate() {
                    o.steps.insert(
                        k + n,
                        OrderStep::from_fixtures(format!("{} {}", step.label, n + 1), vec![fi]),
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
