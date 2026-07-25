//! Top toolbar, Art-Net output panel, and the floating Log window.

use std::net::Ipv4Addr;

use eframe::egui;

use super::{apply_zoom, icons, theme, zoom_controls};
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
                // The DMXexpress mark, uploaded once and reused every frame.
                let logo = self
                    .logo
                    .get_or_insert_with(|| icons::logo_texture(ui.ctx()));
                match logo {
                    Some(tex) => {
                        let h = 30.0;
                        let src = tex.size_vec2();
                        let size = egui::vec2(h * src.x / src.y.max(1.0), h);
                        ui.add(egui::Image::new(egui::load::SizedTexture::new(
                            tex.id(),
                            size,
                        )))
                        .on_hover_text("DMXexpress");
                    }
                    None => {
                        ui.heading("DMXpress");
                    }
                }
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
                let (icon, text, fill) = if self.frozen {
                    (icons::Icon::Play, "UNFREEZE", theme::ACCENT)
                } else {
                    (icons::Icon::Freeze, "FREEZE", theme::RAISED)
                };
                let freeze = ui
                    .scope(|ui| {
                        let v = &mut ui.visuals_mut().widgets;
                        v.inactive.weak_bg_fill = fill;
                        v.hovered.weak_bg_fill = fill.gamma_multiply(1.3);
                        v.active.weak_bg_fill = fill.gamma_multiply(1.5);
                        icons::tab(ui, icon, text, self.frozen)
                    })
                    .inner
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
                // Every pool and panel toggle, in one consistent icon row.
                let tabs: [(icons::Icon, &str, &mut bool, &str); 20] = [
                    (
                        icons::Icon::Artnet,
                        "Art-Net",
                        &mut self.show_artnet,
                        "Output nodes and universe targeting",
                    ),
                    (
                        icons::Icon::Wave,
                        "Oscillator",
                        &mut self.show_osc,
                        "Per-channel waveform engine",
                    ),
                    (
                        icons::Icon::Timer,
                        "Transition",
                        &mut self.show_transition,
                        "Spatial sweep between looks",
                    ),
                    (
                        icons::Icon::Chase,
                        "Chases",
                        &mut self.show_chases,
                        "Spatial chase patterns",
                    ),
                    (
                        icons::Icon::Group,
                        "Groups",
                        &mut self.show_groups,
                        "Stored fixture selections",
                    ),
                    (
                        icons::Icon::Order,
                        "Orders",
                        &mut self.show_orders,
                        "Custom routes effects travel along",
                    ),
                    (
                        icons::Icon::Scene,
                        "Scenes",
                        &mut self.show_scenes,
                        "Captured effects that layer and play together",
                    ),
                    (
                        icons::Icon::Palette,
                        "Palettes",
                        &mut self.show_palettes,
                        "Referenced looks and colour cycles",
                    ),
                    (
                        icons::Icon::Phaser,
                        "Phasers",
                        &mut self.show_phasers,
                        "Spread effects across the selection",
                    ),
                    (
                        icons::Icon::Beat,
                        "Beat",
                        &mut self.show_beat,
                        "Master BPM, tap sync, and the time machine",
                    ),
                    (
                        icons::Icon::Audio,
                        "Audio",
                        &mut self.show_audio,
                        "Listen to the computer: spectrum triggers and beat follow",
                    ),
                    (
                        icons::Icon::Stack,
                        "Stacks",
                        &mut self.show_stacks,
                        "Cue lists",
                    ),
                    (
                        icons::Icon::Deck,
                        "Decks",
                        &mut self.show_decks,
                        "Live playback faders",
                    ),
                    (
                        icons::Icon::Command,
                        "Cmd",
                        &mut self.show_command,
                        "Command line",
                    ),
                    (
                        icons::Icon::Views,
                        "Views",
                        &mut self.show_views,
                        "Saved workspace layouts",
                    ),
                    (icons::Icon::Log, "Log", &mut self.show_log, "Activity log"),
                    (
                        icons::Icon::Patch,
                        "Patch",
                        &mut self.show_patch,
                        "Fixtures and addresses",
                    ),
                    (
                        icons::Icon::Config,
                        "Configs",
                        &mut self.show_configs,
                        "Whole-show save slots",
                    ),
                    (
                        icons::Icon::Test,
                        "DMX",
                        &mut self.show_dmx_test,
                        "Channel monitor & connection tester",
                    ),
                    (
                        icons::Icon::Settings,
                        "Settings",
                        &mut self.show_settings,
                        "Stage and render options",
                    ),
                ];
                for (icon, label, flag, hint) in tabs {
                    if icons::tab(ui, icon, label, *flag).on_hover_text(hint).clicked() {
                        *flag = !*flag;
                    }
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
        egui::Window::new("Log")
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
