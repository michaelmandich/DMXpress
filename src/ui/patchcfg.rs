//! The Patch window (add/remove DMXpress-native fixtures from built-in
//! profiles) and the Configurations window (save/load complete show setups).

use eframe::egui;

use crate::app::App;
use crate::config;
use crate::profiles::{self, UserFixture, PROFILES};
impl App {
    pub(crate) fn patch_window(&mut self, ctx: &egui::Context) {
        if !self.show_patch {
            return;
        }
        let mut open = self.show_patch;
        egui::Window::new("Patch")
            .open(&mut open)
            .default_width(380.0)
            .max_height(520.0)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.label(
                    "Add fixtures from DMXpress's built-in profiles on top of \
                     the ShowBuddy patch.",
                );
                ui.add_space(4.0);
                if ui
                    .checkbox(&mut self.include_showbuddy, "Include ShowBuddy patch")
                    .on_hover_text(
                        "Untick to start fresh: only DMXpress-patched fixtures \
                         stay in the rig, and ShowBuddy's preset banks are \
                         hidden too. Everything comes back when re-ticked.",
                    )
                    .changed()
                {
                    self.save_user_patch();
                    self.rebuild_patch();
                }
                ui.add_space(4.0);

                self.patch_profile = self.patch_profile.min(PROFILES.len() - 1);
                let profile = &PROFILES[self.patch_profile];
                egui::ComboBox::from_label("Profile")
                    .selected_text(profile.name)
                    .show_ui(ui, |ui| {
                        for (i, p) in PROFILES.iter().enumerate() {
                            ui.selectable_value(&mut self.patch_profile, i, p.name);
                        }
                    });
                let profile = &PROFILES[self.patch_profile];
                ui.small(format!(
                    "{} channels, pan {}° / tilt {}°",
                    profile.channel_count(),
                    profile.pan_range,
                    profile.tilt_range
                ));
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.patch_name)
                            .hint_text(profile.name)
                            .desired_width(160.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Address:");
                    ui.add(
                        egui::DragValue::new(&mut self.patch_addr).range(1..=1024),
                    )
                    .on_hover_text("1-based DMX start address (513+ = second universe)");
                    ui.label("Count:");
                    ui.add(egui::DragValue::new(&mut self.patch_count).range(1..=32));
                });
                if ui.button("➕ Add").clicked() {
                    let span = profile.channel_count() as u16;
                    let base_name = if self.patch_name.trim().is_empty() {
                        profile.name.to_string()
                    } else {
                        self.patch_name.trim().to_string()
                    };
                    for i in 0..self.patch_count {
                        let from = self.patch_addr;
                        if from as usize + span as usize - 1 > crate::net::DMX_SLOTS {
                            self.log.push(format!(
                                "Patch stopped: address {from} + {span}ch exceeds slot {}",
                                crate::net::DMX_SLOTS
                            ));
                            break;
                        }
                        let display = if self.patch_count > 1 {
                            format!("{base_name} {}", i + 1)
                        } else {
                            base_name.clone()
                        };
                        self.log
                            .push(format!("Patched '{display}' at {from}-{}", from + span - 1));
                        self.user_fixtures.push(UserFixture {
                            profile: profile.name.to_string(),
                            display,
                            from,
                        });
                        self.patch_addr = from + span;
                    }
                    self.save_user_patch();
                    self.rebuild_patch();
                }

                ui.separator();
                ui.heading("DMXpress fixtures");
                if self.user_fixtures.is_empty() {
                    ui.small("None yet — everything comes from ShowBuddy.");
                }
                let mut remove: Option<usize> = None;
                for (i, uf) in self.user_fixtures.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.button("✖").on_hover_text("Unpatch").clicked() {
                            remove = Some(i);
                        }
                        let span = profiles::find(&uf.profile)
                            .map(|p| p.channel_count() as u16)
                            .unwrap_or(1);
                        ui.label(format!(
                            "{}  —  {}-{}  ({})",
                            uf.display,
                            uf.from,
                            uf.from + span - 1,
                            uf.profile
                        ));
                    });
                }
                if let Some(i) = remove {
                    let uf = self.user_fixtures.remove(i);
                    self.log.push(format!("Unpatched '{}'", uf.display));
                    self.save_user_patch();
                    self.rebuild_patch();
                }

                if self.include_showbuddy {
                    ui.separator();
                    ui.heading("ShowBuddy fixtures");
                    let mut exclude: Option<String> = None;
                    let mut any = false;
                    for f in &self.patch.fixtures {
                        if f.file.to_string_lossy().starts_with("builtin:") {
                            continue;
                        }
                        any = true;
                        ui.horizontal(|ui| {
                            if ui
                                .button("✖")
                                .on_hover_text("Remove from the rig (restorable below)")
                                .clicked()
                            {
                                exclude = Some(profiles::fixture_key(&f.display, f.from));
                            }
                            ui.label(format!("{}  —  {}-{}", f.display, f.from, f.to));
                        });
                    }
                    if !any {
                        ui.small("None (all removed or none patched in ShowBuddy).");
                    }
                    if let Some(key) = exclude {
                        self.log.push(format!("Removed ShowBuddy fixture '{key}'"));
                        self.excluded_fixtures.push(key);
                        self.save_user_patch();
                        self.rebuild_patch();
                    }

                    if !self.excluded_fixtures.is_empty() {
                        ui.add_space(4.0);
                        ui.label("Removed:");
                        let mut restore: Option<usize> = None;
                        for (i, key) in self.excluded_fixtures.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.button("↩").on_hover_text("Restore").clicked() {
                                    restore = Some(i);
                                }
                                ui.small(key.as_str());
                            });
                        }
                        let mut changed = false;
                        if let Some(i) = restore {
                            let key = self.excluded_fixtures.remove(i);
                            self.log.push(format!("Restored ShowBuddy fixture '{key}'"));
                            changed = true;
                        }
                        if ui.button("↩ Restore all").clicked() {
                            self.excluded_fixtures.clear();
                            self.log.push("Restored all ShowBuddy fixtures".into());
                            changed = true;
                        }
                        if changed {
                            self.save_user_patch();
                            self.rebuild_patch();
                        }
                    }
                }
            });
        self.show_patch = open;
    }

    pub(crate) fn configs_window(&mut self, ctx: &egui::Context) {
        if !self.show_configs {
            return;
        }
        let mut open = self.show_configs;
        egui::Window::new("Configurations")
            .open(&mut open)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.label(
                    "A configuration is the whole show: stage settings, light \
                     placement, patched fixtures, groups, palettes, phasers, \
                     stacks and views.",
                );
                ui.add_space(4.0);
                if ui
                    .button("🆕 New show…")
                    .on_hover_text(
                        "Clear groups, palettes, phasers, stacks and views to \
                         build a fresh show. Save a configuration first!",
                    )
                    .clicked()
                {
                    self.confirm_new_show = true;
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.config_name)
                            .hint_text("configuration name")
                            .desired_width(180.0),
                    );
                    if ui.button("💾 Save").clicked() && !self.config_name.trim().is_empty() {
                        let name = self.config_name.trim().to_string();
                        if config::save(&name, &self.snapshot_configuration()) {
                            self.log.push(format!("Saved configuration '{name}'"));
                        } else {
                            self.log.push(format!("Failed to save configuration '{name}'"));
                        }
                    }
                });

                ui.separator();
                ui.heading("Saved configurations");
                let names = config::list();
                if names.is_empty() {
                    ui.small("None saved yet.");
                }
                for name in names {
                    ui.horizontal(|ui| {
                        if ui.button("📂 Load").clicked() {
                            match config::load(&name) {
                                Some(cfg) => {
                                    self.apply_configuration(cfg);
                                    self.log.push(format!("Loaded configuration '{name}'"));
                                }
                                None => self
                                    .log
                                    .push(format!("Failed to load configuration '{name}'")),
                            }
                        }
                        if ui.button("🗑").on_hover_text("Delete").clicked() {
                            self.confirm_delete_config = Some(name.clone());
                        }
                        ui.label(&name);
                    });
                }
            });
        self.show_configs = open;

        // Delete confirmation.
        if let Some(name) = self.confirm_delete_config.clone() {
            egui::Window::new("Delete configuration?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("Delete configuration '{name}'? This cannot be undone."));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Delete").clicked() {
                            config::delete(&name);
                            self.log.push(format!("Deleted configuration '{name}'"));
                            self.confirm_delete_config = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_delete_config = None;
                        }
                    });
                });
        }

        // New-show confirmation.
        if self.confirm_new_show {
            egui::Window::new("Start a fresh show?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(
                        "This clears all groups, palettes, phasers, stacks, views\n\
                         and the programmer so you can build new presets from\n\
                         scratch. Stage settings and your DMXpress-patched\n\
                         fixtures are kept.\n\n\
                         Save a configuration first if you want the current\n\
                         show back later — this cannot be undone.",
                    );
                    ui.add_space(6.0);
                    ui.checkbox(
                        &mut self.new_show_drop_showbuddy,
                        "Remove ShowBuddy lights & presets (keep only DMXpress fixtures)",
                    );
                    ui.checkbox(
                        &mut self.new_show_reset_layout,
                        "Reset light positions to defaults",
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Start fresh").clicked() {
                            let (drop, reset) =
                                (self.new_show_drop_showbuddy, self.new_show_reset_layout);
                            self.new_show(drop, reset);
                            self.confirm_new_show = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_new_show = false;
                        }
                    });
                });
        }
    }
}
