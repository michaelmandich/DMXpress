//! The Patch window (add/remove DMXpress-native fixtures from built-in
//! profiles) and the Configurations window (save/load complete show setups).

use eframe::egui;

use crate::app::App;
use crate::config;
use crate::fixturedb;
use crate::profiles::{self, UserFixture, PROFILES};

/// What the Patch window is about to add — either a library fixture or one
/// of the built-in profiles, flattened so the Add button doesn't care which.
#[derive(Clone)]
pub(crate) struct PatchSelection {
    pub label: String,
    pub channels: usize,
    pub pan: f32,
    pub tilt: f32,
    /// Goes straight into `UserFixture.profile`.
    pub profile: String,
}

impl App {
    /// The library fixture picked in the browser, else the built-in profile.
    pub(crate) fn patch_selection(&self) -> PatchSelection {
        if let Some(id) = self.patch_library_sel.as_deref() {
            if let Some(f) = self.library.find(id) {
                return PatchSelection {
                    label: format!("{} {}", f.model, f.mode).trim().to_string(),
                    channels: f.channels.len(),
                    pan: f.pan_range,
                    tilt: f.tilt_range,
                    profile: fixturedb::profile_ref(id),
                };
            }
        }
        let p = &PROFILES[self.patch_profile.min(PROFILES.len() - 1)];
        PatchSelection {
            label: p.name.to_string(),
            channels: p.channel_count(),
            pan: p.pan_range,
            tilt: p.tilt_range,
            profile: p.name.to_string(),
        }
    }

    pub(crate) fn patch_window(&mut self, ctx: &egui::Context) {
        if !self.show_patch {
            return;
        }
        let mut open = self.show_patch;
        let mut popped = self.popped_out.contains("patch");
        super::floating_panel(
            ctx,
            "patch",
            "Patch",
            &mut open,
            &mut popped,
            None,
            [380.0, 520.0],
            [160.0, 80.0],
            |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
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
                // ShowBuddy lives at a fixed absolute macOS path, so a show
                // built on top of it silently loses its rig elsewhere unless a
                // snapshot travels with it. Say so rather than leaving it to
                // the log.
                if self.include_showbuddy
                    && !std::path::Path::new(crate::showbuddy::DEFAULT_CONFIG).exists()
                {
                    let warn = ui.visuals().warn_fg_color;
                    let text = if self.showbuddy_patch.is_empty() {
                        "ShowBuddy is not reachable on this machine and no saved copy \
                         of its fixtures is available, so only DMXpress-patched \
                         fixtures are in the rig."
                            .to_string()
                    } else {
                        format!(
                            "ShowBuddy is not reachable on this machine; using the {} \
                             fixture(s) saved with this show.",
                            self.showbuddy_patch.len()
                        )
                    };
                    ui.colored_label(warn, egui::RichText::new(text).small());
                }
                ui.add_space(4.0);

                // The library is a few MB of JSON, so it waits until someone
                // actually opens this window to look for a fixture.
                if !self.library.is_loaded() {
                    self.library = fixturedb::Library::load();
                    if let Some(err) = self.library.error.clone() {
                        self.log.push(format!("Fixture library unavailable — {err}"));
                    } else {
                        self.log.push(format!(
                            "Fixture library: {} modes from {} manufacturers",
                            self.library.fixtures.len(),
                            self.library.manufacturers().len()
                        ));
                    }
                }

                ui.heading("Fixture library");
                ui.horizontal(|ui| {
                    ui.label("Search:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.patch_search)
                            .hint_text("chauvet spot…")
                            .desired_width(150.0),
                    );
                    let all = self.patch_manufacturer.is_none();
                    egui::ComboBox::from_id_salt("patch_manufacturer")
                        .selected_text(match &self.patch_manufacturer {
                            Some(m) => m.as_str(),
                            None => "All makers",
                        })
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(all, "All makers").clicked() {
                                self.patch_manufacturer = None;
                            }
                            for m in self.library.manufacturers() {
                                let on = self.patch_manufacturer.as_deref() == Some(m);
                                if ui.selectable_label(on, m).clicked() {
                                    self.patch_manufacturer = Some(m.to_string());
                                }
                            }
                        });
                });

                // A model contributes one row per channel mode, so a maker's
                // name alone can be a hundred rows; the list scrolls.
                const MAX_HITS: usize = 150;
                // Collected up front: the list borrows the library, and
                // picking a row has to mutate the selection.
                let hits: Vec<(String, String, String, usize)> = self
                    .library
                    .search(&self.patch_search, self.patch_manufacturer.as_deref(), MAX_HITS)
                    .into_iter()
                    .filter_map(|i| self.library.fixtures.get(i))
                    .map(|f| (f.id.clone(), f.title(), f.subtitle(), f.channels.len()))
                    .collect();

                if let Some(err) = &self.library.error {
                    ui.colored_label(
                        egui::Color32::from_rgb(240, 170, 90),
                        egui::RichText::new(err).small(),
                    );
                } else if hits.is_empty() {
                    ui.small("No fixtures match that search.");
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("patch_library_hits")
                        .max_height(190.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for (id, title, subtitle, _) in &hits {
                                let on = self.patch_library_sel.as_deref() == Some(id.as_str());
                                let text = egui::RichText::new(format!("{title}\n{subtitle}")).size(11.5);
                                if ui.selectable_label(on, text).clicked() {
                                    self.patch_library_sel = Some(id.clone());
                                }
                            }
                        });
                    ui.small(format!(
                        "{} shown{} · {} in the library",
                        hits.len(),
                        if hits.len() == MAX_HITS { " (narrow the search for more)" } else { "" },
                        self.library.fixtures.len()
                    ));
                }

                ui.add_space(4.0);
                egui::CollapsingHeader::new("Built-in profiles")
                    .default_open(self.patch_library_sel.is_none())
                    .show(ui, |ui| {
                        self.patch_profile = self.patch_profile.min(PROFILES.len() - 1);
                        egui::ComboBox::from_id_salt("patch_builtin_profile")
                            .selected_text(PROFILES[self.patch_profile].name)
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                for (i, p) in PROFILES.iter().enumerate() {
                                    if ui
                                        .selectable_label(self.patch_profile == i, p.name)
                                        .clicked()
                                    {
                                        self.patch_profile = i;
                                        self.patch_library_sel = None;
                                    }
                                }
                            });
                    });

                // Whichever source is selected drives the Add button below.
                let sel = self.patch_selection();
                ui.add_space(2.0);
                ui.small(format!(
                    "Patching: {} — {} channels, pan {}° / tilt {}°",
                    sel.label, sel.channels, sel.pan, sel.tilt
                ));
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.patch_name)
                            .hint_text(&sel.label)
                            .desired_width(160.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Address:");
                    ui.add(
                        egui::DragValue::new(&mut self.patch_addr)
                            .range(1..=crate::net::DMX_SLOTS as u16),
                    )
                    .on_hover_text(format!(
                        "1-based DMX start address, 1–{} across {} universes",
                        crate::net::DMX_SLOTS,
                        crate::net::DMX_UNIVERSES
                    ));
                    ui.label("Count:");
                    ui.add(egui::DragValue::new(&mut self.patch_count).range(1..=32));
                });
                if ui.add_enabled(sel.channels > 0, egui::Button::new("Add")).clicked() {
                    let span = sel.channels as u16;
                    let base_name = if self.patch_name.trim().is_empty() {
                        sel.label.clone()
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
                            profile: sel.profile.clone(),
                            display,
                            from,
                        });
                        self.patch_addr = from + span;
                    }
                    profiles::note_recent(
                        &mut self.recent_profiles,
                        profiles::RecentProfile {
                            profile: sel.profile.clone(),
                            label: sel.label.clone(),
                            channels: sel.channels as u16,
                        },
                    );
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
                        if ui.button("x").on_hover_text("Unpatch").clicked() {
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
                        // Anything DMXpress patched itself — a built-in
                        // profile or a library fixture — is listed above.
                        let src = f.file.to_string_lossy();
                        if src.starts_with("builtin:") || src.starts_with(fixturedb::LIB_PREFIX) {
                            continue;
                        }
                        any = true;
                        ui.horizontal(|ui| {
                            if ui
                                .button("x")
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
                                if ui.button("Restore").clicked() {
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
                        if ui.button("Restore all").clicked() {
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
                }); // patch scroll area
            },
        );
        self.show_patch = open;
        if popped {
            self.popped_out.insert("patch");
        } else {
            self.popped_out.remove("patch");
        }
    }

    pub(crate) fn configs_window(&mut self, ctx: &egui::Context) {
        if !self.show_configs {
            return;
        }
        let mut open = self.show_configs;
        let mut popped = self.popped_out.contains("configs");
        super::floating_panel(
            ctx,
            "configs",
            "Configurations",
            &mut open,
            &mut popped,
            None,
            [360.0, 420.0],
            [180.0, 100.0],
            |ui| {
                ui.label(
                    "A configuration is the whole show: stage settings, light \
                     placement, patched fixtures, groups, palettes, phasers, \
                     stacks and views.",
                );
                ui.add_space(4.0);
                if ui
                    .button("New show…")
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
                    if ui.button("Save").clicked() && !self.config_name.trim().is_empty() {
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
                        if ui.button("Load").clicked() {
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
                        if ui.button("Delete").clicked() {
                            self.confirm_delete_config = Some(name.clone());
                        }
                        ui.label(&name);
                    });
                }
                self.safety_sections(ui);
            },
        );
        self.show_configs = open;
        if popped {
            self.popped_out.insert("configs");
        } else {
            self.popped_out.remove("configs");
        }

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
