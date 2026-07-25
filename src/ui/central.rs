//! Central panel: the 3D stage view plus the channel-type control grid.

use std::collections::HashMap;

use eframe::egui;

use super::{apply_zoom, role_color, zoom_controls};
use crate::app::App;
use crate::net;
use crate::oscillator::Look;
use crate::showbuddy::Role;

impl App {
    pub(crate) fn central_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let mut buf = *self.net.dmx.lock();
            let orig = buf;
            let mut changed = false;

            // --- 3D stage: live beams, movable lights ---
            let stage_h = (ui.available_height() * 0.55).max(240.0);
            self.transition.active_progress =
                self.transition_run.as_ref().map(|r| r.progress());
            let chase_head = self.chase_run.as_ref().map(|r| r.head(&self.chase));
            self.chase.active_head = chase_head;
            let transition = if self.show_transition {
                Some(&mut self.transition)
            } else {
                None
            };
            let chase = if self.show_chases {
                Some(&mut self.chase)
            } else {
                None
            };
            self.stage.ui(
                ui,
                &self.patch,
                &buf,
                stage_h,
                &mut self.settings,
                transition,
                chase,
            );
            if let Some(i) = self.stage.last_selected {
                if self.sel_fixture != Some(i) {
                    self.sel_fixture = Some(i);
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Channel control");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    zoom_controls(ui, &mut self.zoom.central);
                });
            });

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt("central_scroll")
                .show(ui, |ui| {
                    apply_zoom(ui, self.zoom.central);

                    // --- Channel-type control ---
                    // The fixtures we expose: all stage-selected fixtures, else
                    // the single list-selected one.
                    let targets: Vec<usize> = {
                        let sf = self.stage.selected_fixtures();
                        if sf.len() > 1 {
                            sf
                        } else if let Some(i) = self.sel_fixture {
                            vec![i]
                        } else {
                            Vec::new()
                        }
                    };

                    if targets.is_empty() {
                        ui.label("Select a fixture from the list or stage view.");
                    } else {
                        let multi = targets.len() > 1;
                        // Build the controllable rows. One fixture → one row per
                        // channel. Several fixtures → one row per channel *type*
                        // (role; unclassified channels grouped by name), each
                        // driving that type across every selected fixture.
                        // (role, label, representative (fixture, channel), dmx indices)
                        let mut groups: Vec<(Role, String, (usize, usize), Vec<usize>)> =
                            Vec::new();
                        let mut index: HashMap<String, usize> = HashMap::new();
                        for &fi in &targets {
                            let Some(f) = self.patch.fixtures.get(fi) else {
                                continue;
                            };
                            for (ci, ch) in f.channels.iter().enumerate() {
                                let addr = f.from as usize + ci;
                                if addr == 0 || addr > net::DMX_SLOTS {
                                    continue;
                                }
                                let idx = addr - 1;
                                let role = ch.role();
                                if multi {
                                    let key = if role.tag().is_empty() {
                                        format!("n:{}", ch.name.to_lowercase())
                                    } else {
                                        format!("r:{}", role.tag())
                                    };
                                    if let Some(&p) = index.get(&key) {
                                        groups[p].3.push(idx);
                                    } else {
                                        index.insert(key, groups.len());
                                        groups.push((role, ch.name.clone(), (fi, ci), vec![idx]));
                                    }
                                } else {
                                    groups.push((
                                        role,
                                        format!("{:>3}  {}", addr, ch.name),
                                        (fi, ci),
                                        vec![idx],
                                    ));
                                }
                            }
                        }

                        ui.horizontal(|ui| {
                            if multi {
                                ui.heading(format!("Channel types — {} fixtures", targets.len()));
                            } else if let Some(f) = self.patch.fixtures.get(targets[0]) {
                                ui.heading(format!(
                                    "{} — {}ch @ DMX {}..{}",
                                    f.display,
                                    f.channel_count(),
                                    f.from,
                                    f.to
                                ));
                            }
                        });
                        ui.weak(if multi {
                            "Each row drives that channel type across all selected \
                             fixtures. Click a row to arm it for the Oscillator window."
                        } else {
                            "Click a row to arm it for the Oscillator window."
                        });
                        ui.separator();

                        let mut toggle: Option<Vec<usize>> = None;
                        egui::ScrollArea::vertical()
                            .id_salt("chan_ctrl")
                            .show(ui, |ui| {
                                for (role, label, repr, idxs) in &groups {
                                    let role = *role;
                                    let idx0 = idxs[0];
                                    let mut v = buf[idx0];
                                    let armed =
                                        idxs.iter().all(|i| self.sel_channels.contains(i));
                                    ui.horizontal(|ui| {
                                        let badge =
                                            egui::RichText::new(format!("{:>5}", role.tag()))
                                                .monospace()
                                                .size(10.0)
                                                .color(role_color(role));
                                        ui.label(badge);
                                        let text = if idxs.len() > 1 {
                                            format!("{}  ×{}", label, idxs.len())
                                        } else {
                                            label.clone()
                                        };
                                        if ui.selectable_label(armed, text).clicked() {
                                            toggle = Some(idxs.clone());
                                        }
                                        if ui
                                            .add(egui::Slider::new(&mut v, 0..=255).text(""))
                                            .changed()
                                        {
                                            for &i in idxs {
                                                buf[i] = v;
                                            }
                                            changed = true;
                                        }
                                        if ui.small_button("0").clicked() {
                                            for &i in idxs {
                                                buf[i] = 0;
                                            }
                                            changed = true;
                                        }
                                        if ui.small_button("255").clicked() {
                                            for &i in idxs {
                                                buf[i] = 255;
                                            }
                                            changed = true;
                                        }
                                        let (fi, ci) = *repr;
                                        if let Some(lbl) = self
                                            .patch
                                            .fixtures
                                            .get(fi)
                                            .and_then(|f| f.channels.get(ci))
                                            .and_then(|ch| ch.band_label(buf[idx0]))
                                        {
                                            ui.weak(lbl);
                                        }
                                    });
                                }

                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if ui.button("Blackout").clicked() {
                                        for &fi in &targets {
                                            if let Some(f) = self.patch.fixtures.get(fi) {
                                                for ci in 0..f.channel_count() {
                                                    let addr = f.from as usize + ci;
                                                    if addr >= 1 && addr <= net::DMX_SLOTS {
                                                        buf[addr - 1] = 0;
                                                    }
                                                }
                                            }
                                        }
                                        changed = true;
                                    }
                                    if ui.button("Full").clicked() {
                                        for &fi in &targets {
                                            if let Some(f) = self.patch.fixtures.get(fi) {
                                                for ci in 0..f.channel_count() {
                                                    let addr = f.from as usize + ci;
                                                    if addr >= 1 && addr <= net::DMX_SLOTS {
                                                        buf[addr - 1] = 255;
                                                    }
                                                }
                                            }
                                        }
                                        changed = true;
                                    }
                                    if !self.sel_channels.is_empty() {
                                        ui.separator();
                                        ui.label(format!("{} ch armed", self.sel_channels.len()));
                                        if ui.small_button("clear").clicked() {
                                            self.sel_channels.clear();
                                        }
                                    }
                                });
                            });

                        if let Some(idxs) = toggle {
                            let all = idxs.iter().all(|i| self.sel_channels.contains(i));
                            let additive =
                                ui.input(|inp| inp.modifiers.shift || inp.modifiers.command);
                            if !additive {
                                self.sel_channels.clear();
                            }
                            if all {
                                for i in &idxs {
                                    self.sel_channels.remove(i);
                                }
                            } else {
                                for i in idxs {
                                    self.sel_channels.insert(i);
                                }
                            }
                        }
                    }

                    if changed {
                        // A manual edit settles the look. If a transition was
                        // mid-flight, capture the edited output as the new base;
                        // otherwise fold only the changed channels into
                        // `live.base` so oscillators keep animating around them.
                        if self.transition_run.take().is_some() {
                            self.live = Look::from_frame(buf);
                        } else {
                            for i in 0..net::DMX_SLOTS {
                                if buf[i] != orig[i] {
                                    self.live.base[i] = buf[i];
                                }
                            }
                        }
                        // Every touched channel joins the programmer and stops
                        // tracking whatever palette it used to reference.
                        for i in 0..net::DMX_SLOTS {
                            if buf[i] != orig[i] {
                                self.live_active.insert(i);
                                self.live_refs.remove(&i);
                            }
                        }
                        *self.net.dmx.lock() = buf;
                    }
                });
        });
    }
}
