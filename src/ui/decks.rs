//! The executor bar — DMXpress's renamed grandMA3 *decks*.
//!
//! A bottom strip of playback faders, one per stack. Each deck drives its
//! stack's master level (so you can fade a whole cue list in and out), with Go
//! to advance cues and Off to release the stack. A Grand Master on the left
//! scales every dimmer in the rig.

use eframe::egui;

use crate::app::App;

impl App {
    /// Release stack `idx` so it stops contributing to the output.
    pub(crate) fn release_stack(&mut self, idx: usize) {
        if let Some(st) = self.stacks.get_mut(idx) {
            st.release();
            self.log.push(format!("Released \"{}\"", st.name));
        }
    }

    pub(crate) fn executor_bar(&mut self, ctx: &egui::Context) {
        if !self.show_decks {
            return;
        }
        let mut do_go: Option<usize> = None;
        let mut do_off: Option<usize> = None;

        egui::TopBottomPanel::bottom("executors")
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.strong("GM");
                        ui.add(
                            egui::Slider::new(&mut self.grand_master, 0.0..=1.0)
                                .vertical()
                                .show_value(false),
                        )
                        .on_hover_text("Grand Master — scales every dimmer");
                        ui.weak(format!("{:.0}%", self.grand_master * 100.0));
                    });
                    ui.separator();

                    if self.stacks.is_empty() {
                        ui.weak("No stacks yet — open Stacks to build a cue list.");
                        return;
                    }

                    egui::ScrollArea::horizontal().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for i in 0..self.stacks.len() {
                                ui.group(|ui| {
                                    ui.vertical(|ui| {
                                        let playing = self.stacks[i].current.is_some();
                                        let name = self.stacks[i].name.clone();
                                        let title = egui::RichText::new(name).strong().color(
                                            if playing {
                                                egui::Color32::from_rgb(120, 230, 140)
                                            } else {
                                                ui.visuals().text_color()
                                            },
                                        );
                                        ui.label(title);
                                        let cue = match self.stacks[i].current {
                                            Some(c) => self.stacks[i]
                                                .cues
                                                .get(c)
                                                .map(|q| format!("> {}", q.name))
                                                .unwrap_or_else(|| "—".into()),
                                            None => "—".into(),
                                        };
                                        ui.weak(cue);
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::Slider::new(
                                                    &mut self.stacks[i].level,
                                                    0.0..=1.0,
                                                )
                                                .vertical()
                                                .show_value(false),
                                            )
                                            .on_hover_text("Master level");
                                            ui.vertical(|ui| {
                                                if ui.button("Go").clicked() {
                                                    do_go = Some(i);
                                                }
                                                if ui
                                                    .add_enabled(
                                                        playing,
                                                        egui::Button::new("Off"),
                                                    )
                                                    .clicked()
                                                {
                                                    do_off = Some(i);
                                                }
                                            });
                                        });
                                    });
                                });
                            }
                        });
                    });
                });
                ui.add_space(2.0);
            });

        if let Some(i) = do_go {
            self.go_stack(i);
        }
        if let Some(i) = do_off {
            self.release_stack(i);
        }
    }
}
