//! Floating "Views" window: save and recall named workspace layouts.

use eframe::egui;

use super::{apply_zoom, zoom_controls};
use crate::app::App;
use crate::view::{self, View};

impl App {
    /// Snapshot the current panel-visibility flags into a named view.
    fn capture_view(&self, name: String) -> View {
        View {
            name,
            camera: Some(self.stage.cam.snapshot()),
            fly_mode: self.stage.fly_mode,
            artnet: self.show_artnet,
            transition: self.show_transition,
            chases: self.show_chases,
            groups: self.show_groups,
            orders: self.show_orders,
            palettes: self.show_palettes,
            phasers: self.show_phasers,
            stacks: self.show_stacks,
            decks: self.show_decks,
            command: self.show_command,
            log: self.show_log,
            osc: self.show_osc,
        }
    }

    /// Restore panel visibility from a saved view.
    fn apply_view(&mut self, v: &View) {
        if let Some(camera) = &v.camera {
            self.stage.cam.apply_snapshot(camera);
            self.stage.fly_mode = v.fly_mode;
        }
        self.show_artnet = v.artnet;
        self.show_transition = v.transition;
        self.show_chases = v.chases;
        self.show_groups = v.groups;
        self.show_orders = v.orders;
        self.show_palettes = v.palettes;
        self.show_phasers = v.phasers;
        self.show_stacks = v.stacks;
        self.show_decks = v.decks;
        self.show_command = v.command;
        self.show_log = v.log;
        self.show_osc = v.osc;
        self.log.push(format!("View → {}", v.name));
    }

    pub(crate) fn views_window(&mut self, ctx: &egui::Context) {
        if !self.show_views {
            return;
        }
        let mut open = self.show_views;
        let mut do_apply: Option<usize> = None;
        let mut do_delete: Option<usize> = None;
        let mut do_save = false;

        egui::Window::new("Views")
            .open(&mut open)
            .default_width(220.0)
            .show(ctx, |ui| {
                zoom_controls(ui, &mut self.zoom.views);
                apply_zoom(ui, self.zoom.views);

                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut self.view_name);
                    if ui.button("＋ Save").clicked() {
                        do_save = true;
                    }
                });
                ui.separator();

                ui.weak(
                    "Views save both the workspace windows and the current stage camera position.",
                );
                ui.separator();

                if self.views.is_empty() {
                    ui.weak("No views yet. Arrange the workspace and camera, then Save.");
                }
                for (i, v) in self.views.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.button(&v.name).clicked() {
                            do_apply = Some(i);
                        }
                        if ui.small_button("🗑").on_hover_text("Delete").clicked() {
                            do_delete = Some(i);
                        }
                    });
                }
            });

        self.show_views = open;

        if do_save {
            let name = if self.view_name.trim().is_empty() {
                format!("View {}", self.views.len() + 1)
            } else {
                self.view_name.trim().to_string()
            };
            let v = self.capture_view(name);
            self.log.push(format!("Saved view \"{}\"", v.name));
            self.views.push(v);
            self.view_name.clear();
            view::save_views(&self.views);
        }
        if let Some(i) = do_apply {
            let v = self.views[i].clone();
            self.apply_view(&v);
        }
        if let Some(i) = do_delete {
            if i < self.views.len() {
                self.views.remove(i);
                view::save_views(&self.views);
            }
        }
    }
}
