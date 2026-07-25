//! Top toolbar, Art-Net output panel, and the floating Log window.

use std::net::Ipv4Addr;

use eframe::egui;

use super::{apply_zoom, zoom_controls};
use crate::app::App;
use crate::net::NetCmd;

impl App {
    pub(crate) fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("top_tabs")
                // Trackpad-scrolled: no scroll bar drawn over the buttons.
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        self.top_bar_contents(ui);
                    });
                });
        });
    }

    fn top_bar_contents(&mut self, ui: &mut egui::Ui) {
        {
            {
                ui.heading("DMXpress");
                ui.separator();
                ui.label("Universe:");
                if ui
                    .add(egui::DragValue::new(&mut self.universe).range(0..=32767))
                    .on_hover_text(
                        "Base universe. Slots 1-512 go here, slots 513-1024 to the next one.",
                    )
                    .changed()
                {
                    let _ = self.net.cmd_tx.send(NetCmd::SetUniverse(self.universe));
                }
                ui.separator();
                if ui.button("ArtPoll").clicked() {
                    let _ = self.net.cmd_tx.send(NetCmd::Poll);
                }
                ui.separator();
                let freeze_text = if self.frozen {
                    "▶ UNFREEZE"
                } else {
                    "❄ FREEZE"
                };
                let freeze = ui
                    .add(
                        egui::Button::new(freeze_text)
                            .fill(if self.frozen {
                                egui::Color32::from_rgb(35, 125, 190)
                            } else {
                                egui::Color32::from_rgb(55, 65, 80)
                            }),
                    )
                    .on_hover_text(
                        "Hold the exact current output and pause every oscillator, \
                         transition, chase, palette cycle, cue fade, and timed ramp. \
                         Unfreeze resumes every path at the same phase.",
                    );
                if freeze.clicked() {
                    self.set_frozen(!self.frozen);
                }
                ui.separator();
                if ui.button("Reload ShowBuddy patch").clicked() {
                    self.rebuild_patch();
                }
                ui.separator();
                if ui
                    .selectable_label(self.show_artnet, "📡 Art-Net")
                    .clicked()
                {
                    self.show_artnet = !self.show_artnet;
                }
                if ui
                    .selectable_label(self.show_osc, "🌊 Oscillator")
                    .clicked()
                {
                    self.show_osc = !self.show_osc;
                }
                if ui
                    .selectable_label(self.show_transition, "⏱ Transition")
                    .clicked()
                {
                    self.show_transition = !self.show_transition;
                }
                if ui
                    .selectable_label(self.show_chases, "🌀 Chases")
                    .clicked()
                {
                    self.show_chases = !self.show_chases;
                }
                if ui
                    .selectable_label(self.show_groups, "👥 Groups")
                    .clicked()
                {
                    self.show_groups = !self.show_groups;
                }
                if ui
                    .selectable_label(self.show_palettes, "🎨 Palettes")
                    .clicked()
                {
                    self.show_palettes = !self.show_palettes;
                }
                if ui
                    .selectable_label(self.show_phasers, "🌈 Phasers")
                    .clicked()
                {
                    self.show_phasers = !self.show_phasers;
                }
                if ui
                    .selectable_label(self.show_beat, "🥁 Beat")
                    .on_hover_text("Master BPM & tap-to-the-band sync")
                    .clicked()
                {
                    self.show_beat = !self.show_beat;
                }
                if ui
                    .selectable_label(self.show_stacks, "🎬 Stacks")
                    .clicked()
                {
                    self.show_stacks = !self.show_stacks;
                }
                if ui
                    .selectable_label(self.show_decks, "🎚 Decks")
                    .clicked()
                {
                    self.show_decks = !self.show_decks;
                }
                if ui
                    .selectable_label(self.show_command, "⌨ Cmd")
                    .clicked()
                {
                    self.show_command = !self.show_command;
                }
                if ui
                    .selectable_label(self.show_views, "🗂 Views")
                    .clicked()
                {
                    self.show_views = !self.show_views;
                }
                if ui.selectable_label(self.show_log, "📜 Log").clicked() {
                    self.show_log = !self.show_log;
                }
                if ui.selectable_label(self.show_patch, "🔌 Patch").clicked() {
                    self.show_patch = !self.show_patch;
                }
                if ui
                    .selectable_label(self.show_configs, "💾 Configs")
                    .clicked()
                {
                    self.show_configs = !self.show_configs;
                }
                if ui
                    .selectable_label(self.show_dmx_test, "🧪 DMX")
                    .on_hover_text("Channel monitor & connection tester")
                    .clicked()
                {
                    self.show_dmx_test = !self.show_dmx_test;
                }
                if ui.button("⚙ Settings").clicked() {
                    self.show_settings = !self.show_settings;
                }
            }
        }
    }

    pub(crate) fn artnet_window(&mut self, ctx: &egui::Context) {
        if !self.show_artnet {
            return;
        }
        egui::TopBottomPanel::top("artnet_panel")
            .resizable(true)
            .default_height(170.0)
            .show(ctx, |ui| {
                ui.heading("Art-Net Output");
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.nodes.is_empty() {
                        ui.label("Searching... (ArtPoll broadcast)");
                    }
                    let mut new_target: Option<Option<Ipv4Addr>> = None;
                    for node in &self.nodes {
                        let selected = self.selected == Some(node.ip);
                        let label = format!("{}  —  {}", node.ip, node.short_name);
                        if ui.selectable_label(selected, label).clicked() {
                            new_target = Some(Some(node.ip));
                        }
                        ui.small(&node.long_name);
                        ui.separator();
                    }
                    if ui
                        .selectable_label(self.selected.is_none(), "Broadcast 255.255.255.255")
                        .clicked()
                    {
                        new_target = Some(None);
                    }
                    if let Some(t) = new_target {
                        self.selected = t;
                        let _ = self.net.cmd_tx.send(NetCmd::SetTarget(t));
                    }

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Manual target IP:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.manual_ip)
                                .hint_text("e.g. 192.168.1.50")
                                .desired_width(140.0),
                        );
                        if ui.button("Use").clicked() {
                            match self.manual_ip.trim().parse::<Ipv4Addr>() {
                                Ok(ip) => {
                                    self.selected = Some(ip);
                                    let _ = self.net.cmd_tx.send(NetCmd::SetTarget(Some(ip)));
                                    self.log.push(format!("Manual target set: {ip}"));
                                }
                                Err(_) => self.log.push("Invalid IP address".into()),
                            }
                        }
                    });
                });
            });
    }

    pub(crate) fn log_window(&mut self, ctx: &egui::Context) {
        if !self.show_log {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_log;
        egui::Window::new("📜 Log")
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .default_size([360.0, 160.0])
            .default_pos([12.0, screen.bottom() - 200.0])
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    zoom_controls(ui, &mut self.zoom.log);
                });
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        apply_zoom(ui, self.zoom.log);
                        for line in &self.log {
                            ui.monospace(line);
                        }
                    });
            });
        self.show_log = open;
    }
}
