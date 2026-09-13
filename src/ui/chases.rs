//! Floating "Chases" window: non-destructive moving overlays that inject a
//! preset — a rotatable sphere band, a linear wave, random glitter, or a
//! single pulse — plus the dimmer throb button.

use eframe::egui;

use super::{apply_zoom, zoom_controls};
use crate::app::App;
use crate::chase::{ChaseKind, ChaseSource};

impl App {
    /// Every look a chase can inject — native presets first, then the
    /// ShowBuddy banks — flattened into one ordered list. The Chases window
    /// and the Stream Deck's preset knob both step through this, so they
    /// always agree on what "the next preset" means.
    pub(crate) fn chase_sources(&self) -> Vec<(ChaseSource, String)> {
        let mut sources: Vec<(ChaseSource, String)> = self
            .user_presets
            .iter()
            .enumerate()
            .map(|(i, p)| (ChaseSource::User(i), p.name.clone()))
            .collect();
        sources.extend(self.banks.iter().enumerate().flat_map(|(bi, b)| {
            b.presets
                .iter()
                .enumerate()
                .map(move |(pi, p)| (ChaseSource::Bank(bi, pi), p.name.clone()))
        }));
        sources
    }

    /// Name of the look the chase is set to inject, if it still exists.
    pub(crate) fn chase_source_name(&self) -> Option<String> {
        let source = self.chase.source?;
        self.chase_sources()
            .into_iter()
            .find(|(cs, _)| *cs == source)
            .map(|(_, n)| n)
    }

    pub(crate) fn chases_window(&mut self, ctx: &egui::Context) {
        if !self.show_chases {
            self.chase.selected = false;
            return;
        }
        // Flatten native presets + ShowBuddy banks once so the source picker
        // can borrow freely.
        let sources = self.chase_sources();
        let source_name = self
            .chase
            .source
            .and_then(|s| sources.iter().find(|(cs, _)| *cs == s))
            .map(|(_, n)| n.clone());

        let screen = ctx.screen_rect();
        let mut open = self.show_chases;
        let mut popped = self.popped_out.contains("chases");
        let mut do_start = false;
        let mut do_stop = false;
        super::floating_panel(
            ctx,
            "chases",
            "Chases",
            &mut open,
            &mut popped,
            [330.0, 360.0],
            [screen.right() - 380.0, 150.0],
            |ui| {
                zoom_controls(ui, &mut self.zoom.transition);
                apply_zoom(ui, self.zoom.transition);

                let tr = &mut self.chase;
                ui.horizontal_wrapped(|ui| {
                    for k in ChaseKind::ALL {
                        if ui.selectable_label(tr.kind == k, k.label()).clicked() {
                            tr.kind = k;
                        }
                    }
                });
                ui.weak(match tr.kind {
                    ChaseKind::Sphere => {
                        "A band of the preset orbits the sphere — tilt it to sweep \
                         vertically instead of around the room."
                    }
                    ChaseKind::Linear => {
                        "A flat wave of the preset travels across the rig in one \
                         direction, reverting behind itself."
                    }
                    ChaseKind::Boomerang => {
                        "A band travels across the rig, reflects at the far end, \
                         and retraces the same physical path."
                    }
                    ChaseKind::Stripes => {
                        "Several repeated bands travel together across the rig — \
                         use narrow width for thin, long stripes."
                    }
                    ChaseKind::Glitter => {
                        "Every fixture sparkles with the preset at random moments — \
                         density and rate set the feel."
                    }
                    ChaseKind::Pulse => {
                        "One single wave of the preset crosses the rig and stops — \
                         fire it on a hit."
                    }
                    ChaseKind::Ripple => {
                        "Rings spread outward from the sphere like a stone dropped \
                         in water. Ring count sets how many are in flight at once."
                    }
                    ChaseKind::Comet => {
                        "A bright head races across the rig dragging a fading tail \
                         behind it — width sets the tail length."
                    }
                    ChaseKind::Spiral => {
                        "A spiral arm sweeps round and outward together. Two or \
                         three arms turns it into a pinwheel."
                    }
                    ChaseKind::Cascade => {
                        "A wave falls down the rig's height no matter which way the \
                         sphere is aimed. On a rig hung all at one height it sweeps \
                         across instead."
                    }
                    ChaseKind::Chevron => {
                        "A mirrored pair of bands opens out from the middle of the \
                         rig and runs to both ends. Reverse closes them back in."
                    }
                    ChaseKind::Checker => {
                        "The rig splits into banks that flip against each other — \
                         two banks ping-pong, more banks blink like a blinder wall."
                    }
                });
                ui.separator();

                egui::ComboBox::from_label("Inject preset")
                    .selected_text(source_name.clone().unwrap_or_else(|| "— pick —".into()))
                    .show_ui(ui, |ui| {
                        if sources.is_empty() {
                            ui.weak("No presets yet — store a look first");
                        }
                        for (src, name) in &sources {
                            let sel = tr.source == Some(*src);
                            if ui.selectable_label(sel, name).clicked() {
                                tr.source = Some(*src);
                            }
                        }
                    });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if tr.kind == ChaseKind::Pulse {
                        if ui
                            .add_enabled(
                                tr.source.is_some(),
                                egui::Button::new("Send pulse"),
                            )
                            .on_hover_text("Fires one sweep — press again to re-fire")
                            .clicked()
                        {
                            do_start = true;
                        }
                        if tr.enabled {
                            ui.colored_label(
                                egui::Color32::from_rgb(255, 175, 85),
                                "sweeping",
                            );
                        }
                    } else if tr.enabled {
                        if ui.button("Stop chase").clicked() {
                            do_stop = true;
                        }
                        ui.colored_label(egui::Color32::from_rgb(255, 175, 85), "running");
                    } else if ui
                        .add_enabled(tr.source.is_some(), egui::Button::new("Start chase"))
                        .clicked()
                    {
                        do_start = true;
                    }
                });
                ui.separator();

                egui::Grid::new("chase_params")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(if tr.kind == ChaseKind::Glitter {
                            "Density"
                        } else {
                            "Band width"
                        });
                        ui.add_enabled(
                            !tr.auto_width,
                            egui::Slider::new(&mut tr.band_deg, 10.0..=90.0)
                                .suffix("°")
                                .fixed_decimals(0),
                        )
                        .on_hover_text(match tr.kind {
                            ChaseKind::Sphere => "How much of the circle the pulse covers (90° = a quarter sphere).",
                            ChaseKind::Glitter => "How long each sparkle stays lit.",
                            ChaseKind::Comet => "Tail length — the comet's streak runs about three times this.",
                            _ => "How much of the rig the wave covers at once.",
                        });
                        ui.end_row();

                        // Sizing the band by fixture count rather than by angle
                        // is what makes one chase look right on any rig.
                        ui.label("Fit to rig");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut tr.auto_width, "Auto")
                                .on_hover_text(
                                    "Size the band from how many lights it should cover \
                                     instead of a fixed angle, so the same chase reads \
                                     the same on a 4-light bar and a 40-light rig.",
                                );
                            ui.add_enabled(
                                tr.auto_width,
                                egui::DragValue::new(&mut tr.target_lights)
                                    .range(1..=24)
                                    .suffix(" lights"),
                            );
                        });
                        ui.end_row();

                        if let Some(label) = tr.kind.repeat_label() {
                            ui.label(label);
                            ui.add(egui::DragValue::new(&mut tr.stripe_count).range(2..=32));
                            ui.end_row();
                        }

                        ui.label("Speed");
                        ui.add(
                            egui::Slider::new(&mut tr.speed, 0.02..=2.0)
                                .suffix(match tr.kind {
                                    ChaseKind::Sphere => " rev/s",
                                    ChaseKind::Glitter => " spk/s",
                                    _ => " swp/s",
                                })
                                .fixed_decimals(2),
                        );
                        ui.end_row();

                        if tr.kind != ChaseKind::Glitter {
                            ui.label("Direction");
                            egui::ComboBox::from_id_salt("chase_dir")
                                .selected_text(if tr.direction >= 0.0 {
                                    "Forward"
                                } else {
                                    "Reverse"
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut tr.direction, 1.0, "Forward");
                                    ui.selectable_value(&mut tr.direction, -1.0, "Reverse");
                                });
                            ui.end_row();

                            ui.label("Tilt");
                            ui.add(
                                egui::Slider::new(&mut tr.pitch_deg, -90.0..=90.0)
                                    .suffix("°")
                                    .fixed_decimals(0),
                            )
                            .on_hover_text(
                                "Tilts the sweep: 0° travels around/across the room, \
                                 ±90° climbs straight up or down.",
                            );
                            ui.end_row();
                        }

                        ui.label("Edge");
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut tr.soft, true, "Soft pulse");
                            ui.selectable_value(&mut tr.soft, false, "Hard band");
                        });
                        ui.end_row();
                    });

                ui.separator();
                if ui
                    .button("Throb dimmers")
                    .on_hover_text(
                        "Surge every dimmer to full and decay back over half a \
                         second — mash it with the kicks",
                    )
                    .clicked()
                {
                    self.throb_at = Some(std::time::Instant::now());
                }

                ui.separator();
                ui.checkbox(&mut tr.expanded, "Full editor (show the shape on stage)")
                    .on_hover_text(
                        "Draws this chase's geometry in the 3D view — the travelling \
                         wall, rings, spiral arms or bank dividers, whichever this \
                         shape uses — and lets you drag its origin around.",
                    );
                if tr.expanded {
                    egui::Grid::new("chase_sphere")
                        .num_columns(2)
                        .spacing([10.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("X");
                            ui.add(egui::DragValue::new(&mut tr.sphere.pos.x).speed(0.05));
                            ui.end_row();
                            ui.label("Y");
                            ui.add(egui::DragValue::new(&mut tr.sphere.pos.y).speed(0.05));
                            ui.end_row();
                            ui.label("Z");
                            ui.add(egui::DragValue::new(&mut tr.sphere.pos.z).speed(0.05));
                            ui.end_row();
                            ui.label("Start angle °");
                            ui.add(
                                egui::DragValue::new(&mut tr.sphere.yaw_deg)
                                    .speed(0.5)
                                    .range(-360.0..=360.0),
                            );
                            ui.end_row();
                        });
                    ui.weak("Drag the sphere on stage · ⇧drag to spin the start angle.");
                } else {
                    ui.weak("Sphere is hidden until the full editor is open.");
                }
            },
        );
        if do_start {
            self.start_chase();
        }
        if do_stop {
            self.stop_chase();
        }
        self.show_chases = open;
        if popped {
            self.popped_out.insert("chases");
        } else {
            self.popped_out.remove("chases");
        }
        if !self.show_chases || !self.chase.stage_visible() {
            self.chase.selected = false;
        }
    }
}
