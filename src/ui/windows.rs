//! Floating windows: Settings, the reset-confirmation dialog, and the
//! Oscillator control window.

use eframe::egui;

use super::{apply_zoom, zoom_controls};
use crate::app::App;
use crate::net;
use crate::oscillator::{
    self, custom_wave, subdiv_label, CustomWaveform, Look, Osc, SegmentKind, WavePoint,
    WaveTraversal, SPEED_CHOICES,
};
use crate::transition::{TransitionCurve, TransitionMode};

impl App {
    pub(crate) fn confirm_reset_window(&mut self, ctx: &egui::Context) {
        if !self.confirm_reset {
            return;
        }
        egui::Window::new("Reset light positions?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    "This moves every light back to its ShowBuddy stage position,\n\
                     removes duplicates and towers, and overwrites your saved\n\
                     layout. Save a setup first if you want to keep it.",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Reset layout").clicked() {
                        self.stage.reset_layout(&self.patch, &self.settings);
                        self.log.push("Light positions reset to ShowBuddy layout".into());
                        self.confirm_reset = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_reset = false;
                    }
                });
            });
    }

    pub(crate) fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = self.show_settings;
        let mut changed = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let s = &mut self.settings;
                ui.heading("Stage");
                egui::Grid::new("set_stage").num_columns(2).show(ui, |ui| {
                    ui.label("Width (m)");
                    let mut w = s.stage_half_w * 2.0;
                    if ui
                        .add(egui::DragValue::new(&mut w).speed(0.1).range(1.0..=40.0))
                        .changed()
                    {
                        s.stage_half_w = w / 2.0;
                        changed = true;
                    }
                    ui.end_row();
                    ui.label("Depth (m)");
                    let mut d = s.stage_half_d * 2.0;
                    if ui
                        .add(egui::DragValue::new(&mut d).speed(0.1).range(1.0..=40.0))
                        .changed()
                    {
                        s.stage_half_d = d / 2.0;
                        changed = true;
                    }
                    ui.end_row();
                    ui.label("Height (m)");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut s.stage_h)
                                .speed(0.05)
                                .range(0.0..=5.0),
                        )
                        .changed();
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.heading("Lights");
                egui::Grid::new("set_lights").num_columns(2).show(ui, |ui| {
                    ui.label("Body size ×");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut s.light_scale)
                                .speed(0.02)
                                .range(0.2..=5.0),
                        )
                        .changed();
                    ui.end_row();
                    ui.label("Beam opacity ×");
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut s.beam_opacity, 0.0..=3.0)
                                .fixed_decimals(2),
                        )
                        .on_hover_text(
                            "Strength of the \"air-catching\" beam haze. Lower it to \
                             see the overall colours, raise it for dense atmosphere.",
                        )
                        .changed();
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.heading("New light defaults");
                ui.weak("Used for unplaced lights and layout resets.");
                egui::Grid::new("set_defaults").num_columns(2).show(ui, |ui| {
                    ui.label("Height (m)");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut s.default_height)
                                .speed(0.05)
                                .range(0.0..=20.0),
                        )
                        .changed();
                    ui.end_row();
                    ui.label("Yaw °");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut s.default_yaw)
                                .speed(0.5)
                                .range(-360.0..=360.0),
                        )
                        .changed();
                    ui.end_row();
                    ui.label("Pitch °");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut s.default_pitch)
                                .speed(0.5)
                                .range(-180.0..=180.0),
                        )
                        .changed();
                    ui.end_row();
                });
                ui.weak("Pitch -90 = down, 90 = up, 0 = toward audience.");
            });
        if changed {
            self.settings.save();
        }
        self.show_settings = open;
    }

    pub(crate) fn transition_window(&mut self, ctx: &egui::Context) {
        if !self.show_transition {
            self.transition.selected = false;
            return;
        }
        let screen = ctx.screen_rect();
        let active_progress = self.transition_run.as_ref().map(|run| run.progress());
        let mut open = self.show_transition;
        let mut stop_at_current = false;
        egui::Window::new("Transition")
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .default_size([340.0, 320.0])
            .default_pos([screen.right() - 390.0, 430.0])
            .show(ctx, |ui| {
                zoom_controls(ui, &mut self.zoom.transition);
                apply_zoom(ui, self.zoom.transition);

                if let Some(progress) = active_progress {
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .show_percentage()
                            .text("Running"),
                    );
                    if ui.button("Stop at current look").clicked() {
                        stop_at_current = true;
                    }
                    ui.separator();
                }

                let tr = &mut self.transition;
                egui::Grid::new("transition_basic")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Duration");
                        ui.add(
                            egui::Slider::new(&mut tr.duration, 0.0..=20.0)
                                .suffix(" s")
                                .fixed_decimals(1),
                        );
                        ui.end_row();

                        ui.label("Mode");
                        egui::ComboBox::from_id_salt("transition_mode")
                            .selected_text(tr.mode.label())
                            .show_ui(ui, |ui| {
                                for mode in [
                                    TransitionMode::Simple,
                                    TransitionMode::SphereScan,
                                    TransitionMode::Radial,
                                ] {
                                    ui.selectable_value(&mut tr.mode, mode, mode.label());
                                }
                            });
                        ui.end_row();

                        ui.label("Curve");
                        egui::ComboBox::from_id_salt("transition_curve")
                            .selected_text(tr.curve.label())
                            .show_ui(ui, |ui| {
                                for curve in [TransitionCurve::Linear, TransitionCurve::Smooth] {
                                    ui.selectable_value(&mut tr.curve, curve, curve.label());
                                }
                            });
                        ui.end_row();
                    });

                if tr.mode.uses_sphere() {
                    ui.separator();
                    ui.checkbox(&mut tr.expanded, "Full editor (show sphere on stage)");
                    if tr.expanded {
                        egui::Grid::new("transition_sphere")
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
                                if tr.mode == TransitionMode::SphereScan {
                                    ui.label("Scan angle °");
                                    ui.add(
                                        egui::DragValue::new(&mut tr.sphere.yaw_deg)
                                            .speed(0.5)
                                            .range(-360.0..=360.0),
                                    );
                                    ui.end_row();
                                }
                                ui.label("Blend width");
                                ui.add(
                                    egui::Slider::new(&mut tr.edge_width, 0.02..=0.60)
                                        .fixed_decimals(2),
                                );
                                ui.end_row();
                            });
                        if tr.mode == TransitionMode::Radial {
                            ui.checkbox(&mut tr.blast_mode, "Blast mode");
                        }
                    } else {
                        ui.weak("Sphere is hidden until the full editor is open.");
                    }
                } else {
                    tr.expanded = false;
                }
            });
        if stop_at_current {
            self.transition_run = None;
            self.chase.enabled = false;
            self.chase_run = None;
            self.live = Look::from_frame(*self.net.dmx.lock());
        }
        self.show_transition = open;
        if !self.show_transition || !self.transition.stage_visible() {
            self.transition.selected = false;
        }
    }

    /// Floating oscillator window. Drives the channels armed in the centre
    /// "Channel control" panel.
    pub(crate) fn show_oscillator(&mut self, ctx: &egui::Context) {
        if !self.show_osc {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_osc;
        egui::Window::new("Oscillator")
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .default_size([320.0, 280.0])
            .default_pos([screen.right() - 360.0, 120.0])
            .show(ctx, |ui| {
                zoom_controls(ui, &mut self.zoom.osc);
                apply_zoom(ui, self.zoom.osc);

                // Global engine controls (shown only while oscillators run).
                if self.live.is_animated() {
                    let live = &mut self.live;
                    egui::Grid::new("osc_engine")
                        .num_columns(2)
                        .spacing([10.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("Master speed");
                            ui.add(
                                egui::Slider::new(&mut live.master_speed, 0.1..=4.0)
                                    .logarithmic(true)
                                    .fixed_decimals(2),
                            );
                            ui.end_row();
                            ui.label("Tempo");
                            ui.add(
                                egui::DragValue::new(&mut live.tempo)
                                    .speed(0.5)
                                    .range(20.0..=300.0)
                                    .suffix(" bpm"),
                            );
                            ui.end_row();
                            ui.label("Free speed");
                            ui.add(egui::Slider::new(&mut live.speed, 0.0..=1.0));
                            ui.end_row();
                        });
                    if ui.button("⏹ Stop all").clicked() {
                        // Freeze the current animated output as a static look.
                        let frozen = self.live.render();
                        self.live = Look::from_frame(frozen);
                        *self.net.dmx.lock() = frozen;
                    }
                } else {
                    ui.weak("No oscillators running.");
                }
                ui.separator();

                self.custom_waveforms_ui(ui);
                ui.separator();

                let mut sel: Vec<usize> = self
                    .sel_channels
                    .iter()
                    .copied()
                    .filter(|&i| i < net::DMX_SLOTS)
                    .collect();
                sel.sort_unstable();

                if sel.is_empty() {
                    ui.weak(
                        "Arm channels in the centre panel (click a channel type), \
                         then enable an oscillator here.",
                    );
                    return;
                }

                // Edit a copy of the first armed channel's oscillator and push
                // edits to the whole selection. With multiple channels the
                // Offset slider is a per-channel phase step.
                let multi = sel.len() > 1;
                let osc_at = |i: usize| self.live.oscs.get(&i).cloned();
                let mut cur = osc_at(sel[0]).unwrap_or(Osc {
                    enabled: false,
                    ..Osc::default()
                });
                if multi {
                    let p0 = cur.phase;
                    let p1 = osc_at(sel[1]).map(|o| o.phase).unwrap_or(p0);
                    cur.phase = (p1 - p0).rem_euclid(1.0);
                }
                let before = cur.clone();

                ui.label(format!("Editing {} channel(s)", sel.len()));
                egui::Grid::new("osc_params")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("On / Invert");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut cur.enabled, "Enabled");
                            ui.checkbox(&mut cur.invert, "Invert");
                        });
                        ui.end_row();

                        ui.label("Amount");
                        let mut amt = cur.amount * 100.0;
                        if ui
                            .add(egui::Slider::new(&mut amt, 0.0..=100.0).suffix("%"))
                            .changed()
                        {
                            cur.amount = amt / 100.0;
                        }
                        ui.end_row();

                        ui.label("Offset");
                        ui.add(egui::Slider::new(&mut cur.phase, 0.0..=1.0).fixed_decimals(2))
                            .on_hover_text(if multi {
                                "Phase step between successive armed channels \
                                 (0.25 → 0, .25, .5, .75, 0, ...) — makes chases"
                            } else {
                                "Phase offset within the cycle"
                            });
                        ui.end_row();

                        ui.label("Speed");
                        egui::ComboBox::from_id_salt("osc_speed")
                            .selected_text(subdiv_label(cur.subdiv))
                            .show_ui(ui, |ui| {
                                for (name, v) in SPEED_CHOICES {
                                    if ui.selectable_label(cur.subdiv == v, name).clicked() {
                                        cur.subdiv = v;
                                    }
                                }
                            });
                        ui.end_row();

                        ui.label("Waveform");
                        let wave_name = cur
                            .custom_wave
                            .as_ref()
                            .map_or("Built-in shape", |wave| wave.name.as_str());
                        egui::ComboBox::from_id_salt("osc_waveform")
                            .selected_text(wave_name)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(cur.custom_wave.is_none(), "Built-in shape")
                                    .clicked()
                                {
                                    cur.custom_wave = None;
                                }
                                for wave in &self.custom_waveforms {
                                    let selected = cur
                                        .custom_wave
                                        .as_ref()
                                        .is_some_and(|active| active.id == wave.id);
                                    if ui.selectable_label(selected, &wave.name).clicked() {
                                        cur.custom_wave = Some(wave.clone());
                                    }
                                }
                            });
                        ui.end_row();

                        if cur.custom_wave.is_none() {
                            ui.label("Shape");
                            let mut sh = cur.shape * 100.0;
                            if ui
                                .add(egui::Slider::new(&mut sh, 0.0..=100.0).suffix("%"))
                                .changed()
                            {
                                cur.shape = sh / 100.0;
                            }
                            ui.end_row();
                        }
                    });

                if cur != before {
                    let live = &mut self.live;
                    let phase_edited = cur.phase != before.phase;
                    for (k, &i) in sel.iter().enumerate() {
                        let o = live.oscs.entry(i).or_insert_with(|| Osc {
                            enabled: false,
                            ..Osc::default()
                        });
                        let keep_phase = o.phase;
                        *o = cur.clone();
                        o.phase = if multi {
                            if phase_edited {
                                (k as f32 * cur.phase).rem_euclid(1.0)
                            } else {
                                keep_phase
                            }
                        } else {
                            cur.phase
                        };
                    }
                }
            });
        self.show_osc = open;
    }

    fn custom_waveforms_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("✦ Custom waveforms")
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("＋ New").clicked() {
                        let id = self.custom_waveforms.iter().map(|w| w.id).max().unwrap_or(0) + 1;
                        self.waveform_edit = CustomWaveform {
                            id,
                            name: format!("Wave {id}"),
                            ..CustomWaveform::default()
                        };
                        self.waveform_edit_sel = None;
                    }
                    for (i, wave) in self.custom_waveforms.iter().enumerate() {
                        let selected = self.waveform_edit_sel == Some(i);
                        if ui.selectable_label(selected, &wave.name).clicked() {
                            self.waveform_edit = wave.clone();
                            self.waveform_edit_sel = Some(i);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut self.waveform_edit.name);
                    if ui.button("Save waveform").clicked() {
                        self.waveform_edit.points.sort_by(|a, b| a.x.total_cmp(&b.x));
                        if let Some(i) = self.waveform_edit_sel {
                            if i < self.custom_waveforms.len() {
                                self.custom_waveforms[i] = self.waveform_edit.clone();
                            }
                        } else {
                            self.custom_waveforms.push(self.waveform_edit.clone());
                            self.waveform_edit_sel = Some(self.custom_waveforms.len() - 1);
                        }
                        oscillator::save_waveforms(&self.custom_waveforms);
                    }
                    if ui
                        .add_enabled(self.waveform_edit_sel.is_some(), egui::Button::new("Delete"))
                        .clicked()
                    {
                        if let Some(i) = self.waveform_edit_sel.take() {
                            if i < self.custom_waveforms.len() {
                                self.custom_waveforms.remove(i);
                                oscillator::save_waveforms(&self.custom_waveforms);
                            }
                        }
                        self.waveform_edit = CustomWaveform::default();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Travel");
                    egui::ComboBox::from_id_salt("custom_wave_traversal")
                        .selected_text(self.waveform_edit.traversal.label())
                        .show_ui(ui, |ui| {
                            for traversal in WaveTraversal::ALL {
                                ui.selectable_value(
                                    &mut self.waveform_edit.traversal,
                                    traversal,
                                    traversal.label(),
                                );
                            }
                        });
                    ui.label("Stripes");
                    ui.add(
                        egui::DragValue::new(&mut self.waveform_edit.repeats)
                            .range(1..=16),
                    )
                    .on_hover_text("Repeat the drawn wave this many times per cycle");
                });

                let desired = egui::vec2(ui.available_width().max(320.0), 190.0);
                let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 8.0, egui::Color32::from_rgb(18, 20, 29));
                for k in 0..=4 {
                    let x = egui::lerp(rect.x_range(), k as f32 / 4.0);
                    painter.line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(1.0, super::theme::EDGE),
                    );
                }
                painter.line_segment(
                    [egui::pos2(rect.left(), rect.center().y), egui::pos2(rect.right(), rect.center().y)],
                    egui::Stroke::new(1.5, super::theme::TEXT_DIM),
                );
                let to_screen = |x: f32, y: f32| {
                    egui::pos2(
                        egui::lerp(rect.x_range(), x.clamp(0.0, 1.0)),
                        egui::lerp(rect.y_range(), 1.0 - (y.clamp(-1.0, 1.0) + 1.0) * 0.5),
                    )
                };
                let path: Vec<egui::Pos2> = (0..=160)
                    .map(|k| {
                        let x = k as f32 / 160.0;
                        to_screen(x, custom_wave(x, &self.waveform_edit))
                    })
                    .collect();
                painter.add(egui::Shape::line(
                    path,
                    egui::Stroke::new(2.5, egui::Color32::from_rgb(125, 190, 255)),
                ));
                for (i, point) in self.waveform_edit.points.iter().enumerate() {
                    let p = to_screen(point.x, point.y);
                    painter.circle_filled(
                        p,
                        if self.waveform_drag == Some(i) { 7.0 } else { 5.5 },
                        egui::Color32::from_rgb(245, 190, 90),
                    );
                    painter.circle_stroke(p, 7.5, egui::Stroke::new(1.0, egui::Color32::WHITE));
                }

                if resp.drag_started() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        self.waveform_drag = self
                            .waveform_edit
                            .points
                            .iter()
                            .enumerate()
                            .min_by(|(_, a), (_, b)| {
                                to_screen(a.x, a.y)
                                    .distance(pos)
                                    .total_cmp(&to_screen(b.x, b.y).distance(pos))
                            })
                            .and_then(|(i, point)|
                                (to_screen(point.x, point.y).distance(pos) <= 14.0).then_some(i));
                    }
                }
                if resp.dragged() {
                    if let (Some(i), Some(pos)) = (self.waveform_drag, resp.interact_pointer_pos()) {
                        let last = self.waveform_edit.points.len().saturating_sub(1);
                        if let Some(point) = self.waveform_edit.points.get_mut(i) {
                            if i != 0 && i != last {
                                point.x = ((pos.x - rect.left()) / rect.width()).clamp(0.01, 0.99);
                            }
                            point.y = (1.0 - (pos.y - rect.top()) / rect.height()) * 2.0 - 1.0;
                            point.y = point.y.clamp(-1.0, 1.0);
                        }
                        self.waveform_edit.points.sort_by(|a, b| a.x.total_cmp(&b.x));
                    }
                }
                if resp.drag_stopped() {
                    self.waveform_drag = None;
                }
                if resp.double_clicked() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let x = ((pos.x - rect.left()) / rect.width()).clamp(0.01, 0.99);
                        let y = custom_wave(x, &self.waveform_edit);
                        self.waveform_edit.points.push(WavePoint {
                            x,
                            y,
                            segment: SegmentKind::Linear,
                        });
                        self.waveform_edit.points.sort_by(|a, b| a.x.total_cmp(&b.x));
                    }
                }
                resp.context_menu(|ui| {
                    let pos = ui.ctx().pointer_interact_pos().unwrap_or(rect.center());
                    let x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                    let segment = self
                        .waveform_edit
                        .points
                        .windows(2)
                        .position(|pair| x >= pair[0].x && x <= pair[1].x)
                        .unwrap_or(0);
                    ui.label("This section");
                    for kind in SegmentKind::ALL {
                        if ui.button(kind.label()).clicked() {
                            if let Some(point) = self.waveform_edit.points.get_mut(segment) {
                                point.segment = kind;
                            }
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button("＋ Add break here").clicked() {
                        let y = custom_wave(x, &self.waveform_edit);
                        self.waveform_edit.points.push(WavePoint {
                            x: x.clamp(0.01, 0.99),
                            y,
                            segment: SegmentKind::Linear,
                        });
                        self.waveform_edit.points.sort_by(|a, b| a.x.total_cmp(&b.x));
                        ui.close_menu();
                    }
                    let nearest = self
                        .waveform_edit
                        .points
                        .iter()
                        .enumerate()
                        .min_by(|(_, a), (_, b)| (a.x - x).abs().total_cmp(&(b.x - x).abs()))
                        .map(|(i, _)| i);
                    if let Some(i) = nearest {
                        if i > 0 && i + 1 < self.waveform_edit.points.len()
                            && ui.button("Remove nearest break").clicked()
                        {
                            self.waveform_edit.points.remove(i);
                            ui.close_menu();
                        }
                    }
                });
                ui.weak(
                    "Drag circles · double-click to add a break · right-click a section for square, point-to-point, curved, or break controls",
                );

                if ui
                    .add_enabled(!self.sel_channels.is_empty(), egui::Button::new("Use on selected channels"))
                    .clicked()
                {
                    for &addr in &self.sel_channels {
                        let osc = self.live.oscs.entry(addr).or_default();
                        osc.enabled = true;
                        osc.custom_wave = Some(self.waveform_edit.clone());
                        self.live_active.insert(addr);
                    }
                }
            });
    }
}
