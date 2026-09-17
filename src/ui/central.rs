//! Central panel: the 3D stage view plus the channel controls (`channels.rs`).
//!
//! The channel controls under the stage can be folded to one row or torn
//! off into their own OS window; either way the stage takes the room they
//! leave, so with the side panels folded too the screen is the visualiser.

use eframe::egui;

use super::{apply_zoom, icons, theme, zoom_controls};
use crate::app::App;

/// Key of the channel controls in `popped_out` / `collapsed`.
const CHANNELS: &str = "channels";

impl App {
    pub(crate) fn central_panel(&mut self, ctx: &egui::Context) {
        let popped = self.popped_out.contains(CHANNELS);
        let collapsed = self.collapsed.contains(CHANNELS);
        if popped {
            let mut zoom_level = self.zoom.central;
            let outcome = super::popped_viewport(
                ctx,
                CHANNELS,
                "Channel control",
                Some(&mut zoom_level),
                [760.0, 560.0],
                |ui| {
                    // The popped window has its own input: the undo chord is
                    // read from it here, since App::update only sees the main one.
                    self.undo_keys(ui.ctx());
                    self.channel_controls(ui);
                },
            );
            self.zoom.central = zoom_level;
            if outcome.dock {
                self.popped_out.remove(CHANNELS);
            }
            if outcome.closed {
                self.popped_out.remove(CHANNELS);
                self.collapsed.insert(CHANNELS);
            }
        }

        let mut pop = false;
        let mut fold = false;
        let mut unfold = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            let buf = *self.net.dmx.lock();

            // --- 3D stage: live beams, movable lights ---
            let stage_h = if popped {
                ui.available_height()
            } else if collapsed {
                // Leave exactly one row for the folded controls.
                let row = ui.spacing().interact_size.y + ui.spacing().item_spacing.y * 3.0 + 4.0;
                (ui.available_height() - row).max(240.0)
            } else {
                (ui.available_height() * 0.5).max(240.0)
            };
            self.transition.active_progress =
                self.transition_run.as_ref().map(|r| r.progress());
            let chase_head = self.chase_run.as_ref().map(|r| r.head(&self.chase));
            self.chase.active_head = chase_head;
            self.chase.active_step =
                self.chase_run.as_ref().map(|r| r.raw_progress(&self.chase));
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
            // The figure the phaser being edited would trace, on the lights it
            // would drive: the current selection, else the ones it's bound to.
            let trace = if self.show_phasers {
                let points = self.phaser_edit.path_points(160);
                (!points.is_empty()).then(|| {
                    let mut fixtures = self.stage.selected_fixtures();
                    if fixtures.is_empty() {
                        fixtures = self
                            .patch
                            .fixtures
                            .iter()
                            .enumerate()
                            .filter(|(_, f)| {
                                self.phaser_edit
                                    .fixtures
                                    .contains(&crate::profiles::fixture_key(&f.display, f.from))
                            })
                            .map(|(i, _)| i)
                            .collect();
                    }
                    crate::phaser::PhaserTrace { points, fixtures, color: self.phaser_edit.color }
                })
            } else {
                None
            };
            self.stage.ui(
                ui,
                &self.patch,
                &buf,
                &self.gobos,
                stage_h,
                &mut self.settings,
                transition,
                chase,
                trace.as_ref(),
            );
            if let Some(i) = self.stage.last_selected {
                if self.sel_fixture != Some(i) {
                    self.sel_fixture = Some(i);
                }
            }
            if popped {
                return;
            }
            ui.separator();
            if collapsed {
                ui.horizontal(|ui| {
                    if icons::icon_button(ui, icons::Icon::ChevronUp, None)
                        .on_hover_text("Show the channel controls")
                        .clicked()
                    {
                        unfold = true;
                    }
                    ui.label(
                        egui::RichText::new("Channel control")
                            .family(theme::medium())
                            .color(theme::TEXT_DIM),
                    );
                    if !self.sel_channels.is_empty() {
                        theme::hint(ui, format!("{} ch armed", self.sel_channels.len()));
                    }
                });
                return;
            }
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Channel control").family(theme::semibold()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::icon_button(ui, icons::Icon::ChevronDown, None)
                        .on_hover_text("Fold the channel controls away, giving the stage the room")
                        .clicked()
                    {
                        fold = true;
                    }
                    if icons::icon_button(ui, icons::Icon::PopOut, None)
                        .on_hover_text("Open the channel controls in their own window")
                        .clicked()
                    {
                        pop = true;
                    }
                    ui.add_space(4.0);
                    zoom_controls(ui, &mut self.zoom.central);
                });
            });
            apply_zoom(ui, self.zoom.central);
            self.channel_controls(ui);
        });
        if pop {
            self.popped_out.insert(CHANNELS);
        }
        if fold {
            self.collapsed.insert(CHANNELS);
        }
        if unfold {
            self.collapsed.remove(CHANNELS);
        }
    }
}
