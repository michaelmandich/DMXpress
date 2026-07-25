//! Floating "Groups" pool window: store the current stage selection as a
//! named, recallable group (the renamed grandMA3 Group pool). Groups are the
//! selection object every later effect (Spread, Phaser, Stack) builds on.

use eframe::egui;

use super::{apply_zoom, zoom_controls};
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

        egui::Window::new("👥 Groups")
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
                        let unit = if g.mode == GroupMode::AsFixture { " · 1→many" } else { "" };
                        let label = format!("{}\n{} fx{}", g.name, g.fixtures.len(), unit);
                        let resp = ui.add_sized(
                            [86.0, 44.0],
                            egui::SelectableLabel::new(active, label),
                        );
                        if resp.clicked() {
                            let shift = ui.input(|i| i.modifiers.shift);
                            do_recall = Some((i, shift));
                        }
                        resp.context_menu(|ui| {
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
        if let Some(i) = do_delete {
            self.delete_group(i);
        }
    }
}
