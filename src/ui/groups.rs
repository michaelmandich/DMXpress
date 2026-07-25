//! Floating "Groups" pool window: store the current stage selection as a
//! named, recallable group (the renamed grandMA3 Group pool). Groups are the
//! selection object every later effect (Spread, Phaser, Stack) builds on.

use eframe::egui;

use super::{apply_zoom, theme, zoom_controls};
use crate::app::App;
use crate::group::{self, Group, GroupMode};

impl App {
    /// Save the current stage selection as a new group.
    pub(crate) fn store_group(&mut self) {
        let fixtures = self.stage.selected_fixtures();
        if fixtures.is_empty() {
            self.log.push("Groups: select fixtures first".into());
            return;
        }
        let name = if self.group_name.trim().is_empty() {
            format!("Group {}", self.groups.len() + 1)
        } else {
            self.group_name.trim().to_string()
        };
        self.log
            .push(format!("Stored group \"{name}\" ({} fixtures)", fixtures.len()));
        self.groups.push(Group {
            name,
            fixtures,
            mode: GroupMode::Individual,
        });
        group::save_groups(&self.groups);
        self.group_name.clear();
    }

    /// Replace the stage selection with the fixtures of group `idx`, or add
    /// them to it when `additive` (shift-click) — so several groups can be
    /// combined like multi-selecting single lights.
    pub(crate) fn recall_group_add(&mut self, idx: usize, additive: bool) {
        let Some(fixtures) = self.groups.get(idx).map(|g| g.fixtures.clone()) else {
            return;
        };
        // Chaining: a plain click starts a new route, shift-click extends it.
        // The Orders window turns that chain into a custom effect order.
        if additive {
            if !self.group_chain.contains(&idx) {
                self.group_chain.push(idx);
            }
        } else {
            self.group_chain.clear();
            self.group_chain.push(idx);
        }
        if !additive {
            self.stage.selection.clear();
        }
        self.stage.sel_tower = None;
        for &fi in &fixtures {
            self.stage.select_fixture(fi, true);
        }
        if !additive || self.sel_fixture.is_none() {
            self.sel_fixture = fixtures.first().copied();
        }
        self.stage.last_selected = fixtures.first().copied().or(self.sel_fixture);
    }

    /// Replace the stage selection with the fixtures of group `idx`.
    pub(crate) fn recall_group(&mut self, idx: usize) {
        self.recall_group_add(idx, false);
    }

    /// Overwrite group `idx` with the current selection.
    pub(crate) fn update_group(&mut self, idx: usize) {
        let fixtures = self.stage.selected_fixtures();
        if fixtures.is_empty() || idx >= self.groups.len() {
            return;
        }
        self.groups[idx].fixtures = fixtures;
        group::save_groups(&self.groups);
        self.log
            .push(format!("Updated group \"{}\"", self.groups[idx].name));
    }

    /// Remove group `idx`.
    pub(crate) fn delete_group(&mut self, idx: usize) {
        if idx < self.groups.len() {
            let g = self.groups.remove(idx);
            // Chained indices shift with the pool.
            self.group_chain.retain(|&i| i != idx);
            for i in &mut self.group_chain {
                if *i > idx {
                    *i -= 1;
                }
            }
            group::save_groups(&self.groups);
            self.log.push(format!("Deleted group \"{}\"", g.name));
        }
    }

    pub(crate) fn groups_window(&mut self, ctx: &egui::Context) {
        if !self.show_groups {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_groups;
        // Deferred actions so the pool is never mutated mid-iteration.
        let mut do_store = false;
        let mut do_recall: Option<(usize, bool)> = None;
        let mut do_update: Option<usize> = None;
        let mut do_delete: Option<usize> = None;
        let mut do_mode: Option<(usize, GroupMode)> = None;
        let mut do_chain_mode: Option<GroupMode> = None;
        let mut do_release: Option<Vec<usize>> = None;

        // A multi-light step of the active order welds its lights together no
        // matter what mode a group carries, so say so instead of letting the
        // pool claim otherwise.
        let order_bound = self.order_bound_fixtures();
        let order_name = self
            .active_order
            .and_then(|i| self.orders.get(i))
            .map(|o| o.name.clone());
        let chain: Vec<usize> = self.group_chain.clone();
        let chain_fixtures: Vec<usize> = chain
            .iter()
            .filter_map(|&gi| self.groups.get(gi))
            .flat_map(|g| g.fixtures.iter().copied())
            .collect();
        let chain_locked = chain_fixtures.iter().any(|fi| order_bound.contains(fi));

        egui::Window::new("Groups")
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .default_size([320.0, 300.0])
            .default_pos([screen.right() - 360.0, 150.0])
            .show(ctx, |ui| {
                zoom_controls(ui, &mut self.zoom.groups);
                apply_zoom(ui, self.zoom.groups);

                let cur = self.stage.selected_fixtures();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.group_name)
                            .hint_text("name…")
                            .desired_width(140.0),
                    );
                    if ui
                        .add_enabled(!cur.is_empty(), egui::Button::new("＋ Store"))
                        .on_hover_text("Save the current selection as a group")
                        .clicked()
                    {
                        do_store = true;
                    }
                });
                ui.weak(format!(
                    "{} fixture(s) selected · click recalls · ⇧ click adds",
                    cur.len()
                ));
                if !order_bound.is_empty() {
                    let name = order_name.as_deref().unwrap_or("?");
                    ui.horizontal_wrapped(|ui| {
                        theme::pill(ui, "ORDER", theme::WARN);
                        ui.colored_label(
                            theme::WARN,
                            format!(
                                "\"{name}\" holds {} light(s) in folded steps — they move as \
                                 one and cannot be picked apart, whatever mode is shown below.",
                                order_bound.len()
                            ),
                        );
                    });
                }
                ui.separator();

                if self.groups.is_empty() {
                    ui.weak("No groups yet — select fixtures and press Store.");
                }
                ui.horizontal_wrapped(|ui| {
                    for (i, g) in self.groups.iter().enumerate() {
                        // Highlight a group when all its fixtures are selected
                        // (so combined shift-click groups all light up).
                        let active = !g.fixtures.is_empty()
                            && g.fixtures.iter().all(|fi| cur.contains(fi));
                        // What is actually binding these lights, order first.
                        let locked = g.fixtures.iter().any(|fi| order_bound.contains(fi));
                        let unit = if locked {
                            " · order"
                        } else if g.mode == GroupMode::AsFixture {
                            " · 1 light"
                        } else {
                            ""
                        };
                        // Position in the shift-click chain the Orders window
                        // turns into a route.
                        let label = match self.group_chain.iter().position(|&c| c == i) {
                            Some(k) => format!("{}\n{} fx{unit} · #{}", g.name, g.fixtures.len(), k + 1),
                            None => format!("{}\n{} fx{unit}", g.name, g.fixtures.len()),
                        };
                        let resp = ui.add_sized(
                            [86.0, 44.0],
                            egui::SelectableLabel::new(active, label),
                        );
                        if resp.clicked() {
                            let shift = ui.input(|i| i.modifiers.shift);
                            do_recall = Some((i, shift));
                        }
                        resp.context_menu(|ui| {
                            // Lead with the truth about this group right now.
                            if locked {
                                ui.colored_label(
                                    theme::WARN,
                                    format!(
                                        "Folded by order \"{}\"",
                                        order_name.as_deref().unwrap_or("?")
                                    ),
                                );
                                theme::hint(ui, "Its lights cannot be selected individually.");
                            } else if g.mode == GroupMode::AsFixture {
                                ui.colored_label(theme::ACCENT_SOFT, "Acts as one light");
                            } else {
                                theme::hint(ui, "Individual fixtures");
                            }
                            ui.separator();
                            if ui.button("Recall").clicked() {
                                do_recall = Some((i, false));
                                ui.close_menu();
                            }
                            if ui.button("Add to selection").clicked() {
                                do_recall = Some((i, true));
                                ui.close_menu();
                            }
                            if ui.button("Update from selection").clicked() {
                                do_update = Some(i);
                                ui.close_menu();
                            }
                            ui.separator();
                            ui.label("Effect phase mode");
                            for mode in [GroupMode::Individual, GroupMode::AsFixture] {
                                if ui
                                    .selectable_label(g.mode == mode, mode.label())
                                    .clicked()
                                {
                                    do_mode = Some((i, mode));
                                    ui.close_menu();
                                }
                            }
                            if locked
                                && ui
                                    .button("Release from order")
                                    .on_hover_text(
                                        "Split the order's folded steps so these lights \
                                         stand on their own again",
                                    )
                                    .clicked()
                            {
                                do_release = Some(g.fixtures.clone());
                                ui.close_menu();
                            }
                            // Bulk edits over the shift-clicked chain, so a
                            // whole route can be folded or freed in one go.
                            if chain.len() > 1 && chain.contains(&i) {
                                ui.separator();
                                ui.label(format!("Chained groups ({})", chain.len()));
                                if ui.button("All: individual fixtures").clicked() {
                                    do_chain_mode = Some(GroupMode::Individual);
                                    ui.close_menu();
                                }
                                if ui.button("All: one light each").clicked() {
                                    do_chain_mode = Some(GroupMode::AsFixture);
                                    ui.close_menu();
                                }
                                if chain_locked
                                    && ui.button("All: release from order").clicked()
                                {
                                    do_release = Some(chain_fixtures.clone());
                                    ui.close_menu();
                                }
                            }
                            ui.separator();
                            if ui.button("Delete").clicked() {
                                do_delete = Some(i);
                                ui.close_menu();
                            }
                        });
                    }
                });
            });
        self.show_groups = open;

        if do_store {
            self.store_group();
        }
        if let Some((i, additive)) = do_recall {
            self.recall_group_add(i, additive);
        }
        if let Some(i) = do_update {
            self.update_group(i);
        }
        if let Some((i, mode)) = do_mode {
            if let Some(group) = self.groups.get_mut(i) {
                group.mode = mode;
                let name = group.name.clone();
                group::save_groups(&self.groups);
                self.log.push(format!("Group \"{name}\": {}", mode.label()));
            }
        }
        if let Some(mode) = do_chain_mode {
            let mut changed = 0;
            for gi in &chain {
                if let Some(g) = self.groups.get_mut(*gi) {
                    if g.mode != mode {
                        g.mode = mode;
                        changed += 1;
                    }
                }
            }
            if changed > 0 {
                group::save_groups(&self.groups);
                self.log
                    .push(format!("{changed} chained group(s): {}", mode.label()));
            }
        }
        if let Some(fixtures) = do_release {
            self.release_from_order(&fixtures);
        }
        if let Some(i) = do_delete {
            self.delete_group(i);
        }
    }
}
