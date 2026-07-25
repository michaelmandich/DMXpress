//! Left "Fixtures" raid-grid panel and right "Inspector" panel (presets,
//! selection inspector, saved setups, stage controls).

use eframe::egui;

use super::{apply_zoom, zoom_controls};
use crate::app::App;
use crate::net::Frame;
use crate::oscillator::Look;
use crate::preset;
use crate::stage::{self, StageView};

impl App {
    pub(crate) fn fixtures_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("fixtures").min_width(220.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Fixtures");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    zoom_controls(ui, &mut self.zoom.fixtures);
                });
            });
            ui.separator();
            if self.patch.fixtures.is_empty() {
                ui.label("No patch loaded.");
            }
            ui.weak("Raid-style grid — click to select · ⇧ multi · ⌘ same type");
            let buf = *self.net.dmx.lock();
            let z = self.zoom.fixtures;
            egui::ScrollArea::vertical().show(ui, |ui| {
                // Compact "raid frame" tiles in a wrapping grid: each fixture
                // is a ~2:3 rectangle filled with its live colour.
                let tile = egui::vec2(58.0 * z, 38.0 * z);
                let gap = 4.0 * z;
                ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                let mut click: Option<usize> = None;
                let mut mods = egui::Modifiers::default();
                let painter = ui.painter().clone();
                ui.horizontal_wrapped(|ui| {
                    for (i, f) in self.patch.fixtures.iter().enumerate() {
                        let in_sel =
                            self.stage.fixture_selected(i) || self.sel_fixture == Some(i);
                        let swatch = stage::fixture_swatch(f, &buf);
                        let (rect, resp) =
                            ui.allocate_exact_size(tile, egui::Sense::click());
                        let rounding = 4.0 * z;
                        painter.rect_filled(rect, rounding, swatch);
                        let lum = 0.299 * swatch.r() as f32
                            + 0.587 * swatch.g() as f32
                            + 0.114 * swatch.b() as f32;
                        let txt = if lum > 140.0 {
                            egui::Color32::BLACK
                        } else {
                            egui::Color32::from_gray(235)
                        };
                        let dim = txt.gamma_multiply(0.75);
                        // Fixture name (truncated to fit the tile width).
                        let maxch = (((tile.x - 8.0 * z) / (5.5 * z)) as usize).max(3);
                        let mut name = f.display.clone();
                        if name.chars().count() > maxch {
                            name = name.chars().take(maxch.saturating_sub(1)).collect::<String>()
                                + "…";
                        }
                        painter.text(
                            rect.left_top() + egui::vec2(4.0 * z, 3.0 * z),
                            egui::Align2::LEFT_TOP,
                            name,
                            egui::FontId::proportional(11.0 * z),
                            txt,
                        );
                        painter.text(
                            rect.left_bottom() + egui::vec2(4.0 * z, -3.0 * z),
                            egui::Align2::LEFT_BOTTOM,
                            format!("@{}", f.from),
                            egui::FontId::monospace(9.0 * z),
                            dim,
                        );
                        let stroke = if in_sel {
                            egui::Stroke::new(2.0 * z, egui::Color32::from_rgb(120, 180, 255))
                        } else {
                            egui::Stroke::new(1.0 * z, super::theme::EDGE)
                        };
                        painter.rect_stroke(rect, rounding, stroke);
                        let resp = resp.on_hover_text(format!(
                            "{}  ·  DMX {}..{}  ·  {}ch",
                            f.display,
                            f.from,
                            f.to,
                            f.channel_count()
                        ));
                        if resp.clicked() {
                            click = Some(i);
                            mods = ui.input(|inp| inp.modifiers);
                        }
                    }
                });
                if let Some(i) = click {
                    if mods.command {
                        self.stage.select_same_type(&self.patch, i);
                    } else if mods.shift {
                        self.stage.toggle_fixture(i);
                    } else {
                        self.stage.select_fixture(i, false);
                    }
                    self.sel_fixture = Some(i);
                }
            });
        });
    }

    pub(crate) fn inspector_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("inspector").min_width(230.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Inspector");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    zoom_controls(ui, &mut self.zoom.inspector);
                });
            });
            ui.separator();
            apply_zoom(ui, self.zoom.inspector);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt("inspector_scroll")
                .show(ui, |ui| {
                    // --- native DMXpress presets ---
                    let head = ui.heading("Presets");
                    // Dropping a preset on the heading moves it to the top level.
                    if let Some(src) = head.dnd_release_payload::<usize>() {
                        if *src < self.user_presets.len() {
                            self.user_presets[*src].folder.clear();
                            preset::save_presets(&self.preset_folders, &self.user_presets);
                        }
                    }
                    ui.separator();
                    let mut store_preset = false;
                    let mut add_folder = false;
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.preset_name)
                                .hint_text("name…")
                                .desired_width(90.0),
                        );
                        if ui
                            .button("＋ Store look")
                            .on_hover_text(
                                "Save the programmer's current values and running \
                                 oscillations as a preset",
                            )
                            .clicked()
                        {
                            store_preset = true;
                        }
                        if ui
                            .button("＋ Folder")
                            .on_hover_text(
                                "Add a folder — drag presets onto it to file them \
                                 (drop on the Presets heading to unfile)",
                            )
                            .clicked()
                        {
                            add_folder = true;
                        }
                        ui.checkbox(&mut self.preset_drag, "drag")
                            .on_hover_text(
                                "On: drag presets into folders / to reorder. \
                                 Off: click to apply presets.",
                            );
                    });
                    let mut apply_user: Option<usize> = None;
                    let mut over_user: Option<usize> = None;
                    let mut del_user: Option<usize> = None;
                    let mut move_before: Option<(usize, usize)> = None;
                    let mut move_into: Option<(usize, String)> = None;
                    let mut toggle_folder: Option<String> = None;
                    let mut del_folder: Option<usize> = None;
                    if self.user_presets.is_empty() && self.preset_folders.is_empty() {
                        ui.weak("No presets yet — set a look, then Store.");
                    }
                    let accent = ui.visuals().selection.bg_fill;
                    let drag_mode = self.preset_drag;
                    let mut preset_row = |ui: &mut egui::Ui, i: usize, indent: bool| {
                        let p = &self.user_presets[i];
                        let active = self.active_user_preset == Some(i);
                        let label = if indent {
                            format!("   ⭐ {}", p.name)
                        } else {
                            format!("⭐ {}", p.name)
                        };
                        let resp = if drag_mode {
                            let id = egui::Id::new(("user_preset_drag", i));
                            ui.dnd_drag_source(id, i, |ui| {
                                let _ = ui.selectable_label(active, label);
                            })
                            .response
                        } else {
                            ui.selectable_label(active, label)
                        };
                        if resp.clicked() {
                            apply_user = Some(i);
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Overwrite with current look").clicked() {
                                over_user = Some(i);
                                ui.close_menu();
                            }
                            if ui.button("Delete").clicked() {
                                del_user = Some(i);
                                ui.close_menu();
                            }
                        });
                        // Drop another preset here → reorder in front of this one.
                        if let Some(src) = resp.dnd_release_payload::<usize>() {
                            move_before = Some((*src, i));
                        } else if resp.dnd_hover_payload::<usize>().is_some() {
                            ui.painter().hline(
                                resp.rect.x_range(),
                                resp.rect.top(),
                                egui::Stroke::new(2.0, accent),
                            );
                        }
                    };
                    for i in 0..self.user_presets.len() {
                        if self.user_presets[i].folder.is_empty() {
                            preset_row(ui, i, false);
                        }
                    }
                    for (fi, folder) in self.preset_folders.iter().enumerate() {
                        let open = self.open_user_folders.contains(folder);
                        let resp =
                            ui.selectable_label(open, format!("📁 {folder}"));
                        if resp.clicked() {
                            toggle_folder = Some(folder.clone());
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Delete folder").clicked() {
                                del_folder = Some(fi);
                                ui.close_menu();
                            }
                        });
                        // Drop a preset on the folder to file it there.
                        if let Some(src) = resp.dnd_release_payload::<usize>() {
                            move_into = Some((*src, folder.clone()));
                        } else if resp.dnd_hover_payload::<usize>().is_some() {
                            ui.painter().rect_stroke(
                                resp.rect,
                                3.0,
                                egui::Stroke::new(1.5, accent),
                            );
                        }
                        if open {
                            for i in 0..self.user_presets.len() {
                                if self.user_presets[i].folder == *folder {
                                    preset_row(ui, i, true);
                                }
                            }
                        }
                    }
                    drop(preset_row);
                    if store_preset {
                        let name = if self.preset_name.trim().is_empty() {
                            format!("Preset {}", self.user_presets.len() + 1)
                        } else {
                            self.preset_name.trim().to_string()
                        };
                        self.store_user_preset(name);
                        self.preset_name.clear();
                    }
                    if add_folder {
                        let name = if self.preset_name.trim().is_empty() {
                            format!("Folder {}", self.preset_folders.len() + 1)
                        } else {
                            self.preset_name.trim().to_string()
                        };
                        if !self.preset_folders.contains(&name) {
                            self.preset_folders.push(name.clone());
                            preset::save_presets(&self.preset_folders, &self.user_presets);
                        }
                        self.open_user_folders.insert(name);
                        self.preset_name.clear();
                    }
                    if let Some(name) = toggle_folder {
                        if !self.open_user_folders.remove(&name) {
                            self.open_user_folders.insert(name);
                        }
                    }
                    if let Some(fi) = del_folder {
                        let name = self.preset_folders.remove(fi);
                        for p in &mut self.user_presets {
                            if p.folder == name {
                                p.folder.clear();
                            }
                        }
                        self.open_user_folders.remove(&name);
                        preset::save_presets(&self.preset_folders, &self.user_presets);
                        self.log.push(format!(
                            "Deleted folder \"{name}\" (presets kept)"
                        ));
                    }
                    if let Some((src, folder)) = move_into {
                        if src < self.user_presets.len()
                            && self.user_presets[src].folder != folder
                        {
                            self.user_presets[src].folder = folder;
                            preset::save_presets(&self.preset_folders, &self.user_presets);
                        }
                    }
                    if let Some((src, dst)) = move_before {
                        if src != dst && src < self.user_presets.len() && dst < self.user_presets.len() {
                            let active = self.active_user_preset;
                            let mut p = self.user_presets.remove(src);
                            let dst2 = if src < dst { dst - 1 } else { dst };
                            p.folder = self.user_presets[dst2].folder.clone();
                            self.user_presets.insert(dst2, p);
                            // Keep the active highlight on the same preset.
                            self.active_user_preset = active.map(|a| {
                                if a == src {
                                    dst2
                                } else {
                                    let a2 = if a > src { a - 1 } else { a };
                                    if a2 >= dst2 { a2 + 1 } else { a2 }
                                }
                            });
                            preset::save_presets(&self.preset_folders, &self.user_presets);
                        }
                    }
                    if let Some(i) = apply_user {
                        self.apply_user_preset(i);
                    }
                    if let Some(i) = over_user {
                        let name = self.user_presets[i].name.clone();
                        let folder = self.user_presets[i].folder.clone();
                        let before = self.user_presets.len();
                        self.store_user_preset(name);
                        if self.user_presets.len() > before {
                            let mut newp = self.user_presets.pop().unwrap();
                            newp.folder = folder;
                            self.user_presets[i] = newp;
                            preset::save_presets(&self.preset_folders, &self.user_presets);
                        }
                    }
                    if let Some(i) = del_user {
                        if i < self.user_presets.len() {
                            let p = self.user_presets.remove(i);
                            preset::save_presets(&self.preset_folders, &self.user_presets);
                            if self.active_user_preset == Some(i) {
                                self.active_user_preset = None;
                            }
                            self.log.push(format!("Deleted preset \"{}\"", p.name));
                        }
                    }

                    ui.add_space(8.0);
                    // --- ShowBuddy presets ---
                    ui.heading("ShowBuddy");
                    ui.separator();
                    let mut clicked: Option<(usize, usize)> = None;
                    egui::ScrollArea::vertical()
                        .max_height(ui.available_height() * 0.45)
                        .id_salt("presets")
                        .show(ui, |ui| {
                            if self.banks.is_empty() {
                                ui.weak("No ShowBuddy presets found.");
                            }
                            for (bi, bank) in self.banks.iter().enumerate() {
                                let open = self.open_bank == Some(bi);
                                if ui
                                    .selectable_label(open, format!("📁 {}", bank.name))
                                    .clicked()
                                {
                                    self.open_bank = if open { None } else { Some(bi) };
                                }
                                if open {
                                    for (pi, p) in bank.presets.iter().enumerate() {
                                        let active = self.active_preset == Some((bi, pi));
                                        if ui
                                            .selectable_label(active, format!("   {}", p.name))
                                            .clicked()
                                        {
                                            clicked = Some((bi, pi));
                                        }
                                    }
                                }
                            }
                        });
                    if let Some((bi, pi)) = clicked {
                        self.apply_preset(bi, pi);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Blackout all").clicked() {
                            *self.net.dmx.lock() = Frame::black();
                            self.live = Look::black();
                            self.active_preset = None;
                            self.active_user_preset = None;
                            self.active_phasers.clear();
                            self.hold_overrides.clear();
                            self.transition_run = None;
                            self.chase.enabled = false;
                            self.chase_run = None;
                        }
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.heading("Inspector");
                    ui.separator();
                    self.stage.inspector_ui(ui, &self.patch);
                    ui.add_space(12.0);
                    ui.separator();

                    // --- saved setups (named arrangements of lights + towers) ---
                    egui::CollapsingHeader::new("Setups")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.setup_name)
                                        .hint_text("setup name")
                                        .desired_width(110.0),
                                );
                                if ui.button("Save").clicked() {
                                    let name = self.setup_name.trim().to_string();
                                    if self.stage.save_setup(&self.patch, &name) {
                                        self.log.push(format!("Setup '{name}' saved"));
                                    }
                                }
                            });
                            let mut load: Option<String> = None;
                            let mut remove: Option<String> = None;
                            for name in StageView::list_setups() {
                                ui.horizontal(|ui| {
                                    ui.label(&name);
                                    if ui.small_button("Load").clicked() {
                                        load = Some(name.clone());
                                    }
                                    if ui
                                        .small_button("✕")
                                        .on_hover_text("Delete this saved setup")
                                        .clicked()
                                    {
                                        remove = Some(name.clone());
                                    }
                                });
                            }
                            if let Some(name) = load {
                                if self.stage.load_setup(&self.patch, &self.settings, &name) {
                                    self.log.push(format!("Setup '{name}' loaded"));
                                } else {
                                    self.log.push(format!("Setup '{name}' failed to load"));
                                }
                            }
                            if let Some(name) = remove {
                                StageView::delete_setup(&name);
                                self.log.push(format!("Setup '{name}' deleted"));
                            }
                        });

                    ui.add_space(8.0);
                    if ui.button("➕ Add tower").on_hover_text(
                        "Floor stand with one crossbar (4 top + 4 bottom light slots). \
                         Drag lights near the blue rings to snap them on.",
                    ).clicked() {
                        self.stage.add_tower(&self.patch);
                    }
                    if ui.button("Reset light positions…").clicked() {
                        self.confirm_reset = true;
                    }
                    ui.add_space(8.0);
                    ui.weak("Stage controls");
                    ui.weak("• drag light: move on floor plane");
                    ui.weak("• ⌘+drag: change height");
                    ui.weak("• ⇧+click: multi-select");
                    ui.weak("• drag empty space: pan camera");
                    ui.weak("• ⇧+drag empty: marquee select");
                    ui.weak("• ⌘D: duplicate · ⌫: remove copy");
                    ui.weak("• drag light onto tower ring: snap");
                    ui.weak("• right-drag: orbit camera");
                    ui.weak("• middle-drag: pan · scroll: zoom");
                });
        });
    }
}
