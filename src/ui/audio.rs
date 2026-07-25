//! Floating "Audio" window: pick any computer audio source, watch its
//! spectrum, let the beat tracker press TAP, and draw trigger bands on the
//! graph — "when the bass crosses this line, throw this palette at that
//! group".

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};

use super::{apply_zoom, theme, zoom_controls};
use crate::app::App;
use crate::audio::{self, hz_to_x, x_to_hz, AudioTrigger, TriggerMode, TriggerSource, BINS};
use crate::scene::MergeMode;

/// Named starting points for the band pickers.
const BAND_PRESETS: [(&str, f32, f32); 7] = [
    ("Sub", 30.0, 60.0),
    ("Bass", 60.0, 150.0),
    ("Low-mid", 150.0, 400.0),
    ("Mid", 400.0, 1000.0),
    ("High-mid", 1000.0, 3000.0),
    ("Presence", 3000.0, 8000.0),
    ("Air", 8000.0, 16000.0),
];

/// Distinguishable overlay colours, cycled by trigger index.
const TRIGGER_COLORS: [Color32; 6] = [
    Color32::from_rgb(0x6F, 0xB6, 0xB5),
    Color32::from_rgb(0xF0, 0xC2, 0x30),
    Color32::from_rgb(0x8C, 0xC9, 0x7F),
    Color32::from_rgb(0xE0, 0x7A, 0x5F),
    Color32::from_rgb(0xA0, 0x8F, 0xD8),
    Color32::from_rgb(0x6F, 0x9F, 0xD8),
];

fn trigger_color(i: usize) -> Color32 {
    TRIGGER_COLORS[i % TRIGGER_COLORS.len()]
}

/// What a drag on the graph is editing, classified once when it starts.
#[derive(Clone, Copy)]
enum GraphDrag {
    Lo,
    Hi,
    Threshold,
    /// Slide the whole band; `grab` = pointer offset from the low edge,
    /// `width` = band width, both in unit-x.
    Move { grab: f32, width: f32 },
    /// Sweep a fresh band from `x0`.
    Paint { x0: f32 },
}

impl App {
    /// Write the machine-local audio state (device, follow, triggers).
    fn persist_audio(&self) {
        audio::save_audio(&audio::AudioFile {
            device: self.audio_device_pref.as_ref().map(|(n, _)| n.clone()),
            loopback: self.audio_device_pref.as_ref().is_some_and(|&(_, l)| l),
            follow_beat: self.audio_follow_beat,
            triggers: self.audio_triggers.clone(),
        });
    }

    pub(crate) fn audio_window(&mut self, ctx: &egui::Context) {
        if !self.show_audio {
            return;
        }
        if self.audio_devices.is_empty() {
            self.audio_devices = audio::list_sources();
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_audio;
        let mut dirty = false;
        let mut do_start = false;
        let mut do_stop = false;
        let mut do_add = false;
        let mut do_remove: Option<usize> = None;

        egui::Window::new("Audio")
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .default_size([520.0, 560.0])
            .default_pos([screen.center().x - 260.0, 80.0])
            .show(ctx, |ui| {
                zoom_controls(ui, &mut self.zoom.audio);
                apply_zoom(ui, self.zoom.audio);

                // ---- source ----
                theme::section(ui, "Source");
                ui.horizontal(|ui| {
                    let selected = self
                        .audio_device_pref
                        .as_ref()
                        .map(|(n, l)| {
                            audio::AudioSource {
                                name: n.clone(),
                                loopback: *l,
                            }
                            .label()
                        })
                        .unwrap_or_else(|| "pick a source…".into());
                    egui::ComboBox::from_id_salt("audio-source")
                        .selected_text(selected)
                        .width(240.0)
                        .show_ui(ui, |ui| {
                            for src in &self.audio_devices {
                                let is = self
                                    .audio_device_pref
                                    .as_ref()
                                    .is_some_and(|(n, l)| *n == src.name && *l == src.loopback);
                                if ui.selectable_label(is, src.label()).clicked() {
                                    self.audio_device_pref =
                                        Some((src.name.clone(), src.loopback));
                                    dirty = true;
                                }
                            }
                        });
                    if ui.small_button("rescan").clicked() {
                        self.audio_devices = audio::list_sources();
                    }
                    if self.audio.is_running() {
                        if ui.button("Stop").clicked() {
                            do_stop = true;
                        }
                        theme::pill(ui, "LIVE", theme::OK);
                    } else if ui
                        .add_enabled(self.audio_device_pref.is_some(), egui::Button::new("Start"))
                        .clicked()
                    {
                        do_start = true;
                    }
                });
                if let Some(err) = self.audio.error() {
                    ui.colored_label(theme::DANGER, err);
                }
                theme::hint(
                    ui,
                    "System-audio sources capture whatever the computer is playing \
                     (a video, a stream). On macOS/Linux that needs a virtual \
                     device such as BlackHole; microphones work everywhere.",
                );

                // ---- beat ----
                ui.add_space(6.0);
                theme::section(ui, "Beat");
                let analysis = self.audio.analysis();
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.audio_follow_beat, "Follow the music")
                        .on_hover_text(
                            "Each detected beat presses TAP for you: the master BPM \
                             adapts and every synced look drifts onto the beat",
                        )
                        .changed()
                    {
                        dirty = true;
                    }
                    // Beat flash dot.
                    let (dot, _) =
                        ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::hover());
                    let age = self
                        .audio_beat_flash
                        .map_or(1.0, |t| t.elapsed().as_secs_f32() / 0.18);
                    let k = (1.0 - age).clamp(0.0, 1.0);
                    ui.painter().circle_filled(
                        dot.center(),
                        5.0 + 4.0 * k,
                        theme::ACCENT_SOFT.gamma_multiply(0.25 + 0.75 * k),
                    );
                    if self.audio.is_running() && analysis.bpm > 0.0 {
                        ui.label(format!("{:.1} bpm heard", analysis.bpm));
                        let conf = analysis.confidence;
                        theme::pill(
                            ui,
                            &format!("{:.0}% sure", conf * 100.0),
                            if conf > 0.5 {
                                theme::OK
                            } else if conf > 0.2 {
                                theme::WARN
                            } else {
                                theme::TEXT_DIM
                            },
                        );
                    } else {
                        ui.weak("listening for a pulse…");
                    }
                });

                // ---- graph ----
                ui.add_space(6.0);
                theme::section(ui, "Trigger map");
                theme::hint(
                    ui,
                    "Live spectrum, low notes left. Drag on empty space to sweep a \
                     new band; drag a band's edges, body or threshold line to \
                     shape when it fires.",
                );
                self.trigger_graph(ui, &analysis.spectrum, &mut dirty);

                // ---- triggers ----
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Add trigger").clicked() {
                        do_add = true;
                    }
                    let live = self
                        .audio_triggers
                        .iter()
                        .filter(|t| t.enabled && t.env > 0.05)
                        .count();
                    if live > 0 {
                        theme::pill(ui, &format!("{live} firing"), theme::OK);
                    }
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for i in 0..self.audio_triggers.len() {
                        if self.trigger_row(ui, i, &mut dirty) {
                            do_remove = Some(i);
                        }
                        ui.add_space(4.0);
                    }
                    if self.audio_triggers.is_empty() {
                        theme::hint(
                            ui,
                            "No triggers yet — press Add, or just drag a band on \
                             the graph. Example: Bass over 0.7 throws a red \
                             palette at the back towers.",
                        );
                    }
                });
            });
        self.show_audio = open;

        if do_start {
            if let Some((name, loopback)) = self.audio_device_pref.clone() {
                self.audio.start(audio::AudioSource { name, loopback });
                self.log.push(format!(
                    "Audio listening to {}",
                    self.audio.source_label.as_deref().unwrap_or("?")
                ));
                dirty = true;
            }
        }
        if do_stop {
            self.audio.stop();
            self.log.push("Audio stopped".into());
        }
        if do_add {
            let mut t = AudioTrigger::new(self.audio_triggers.len() + 1);
            t.source = self.palettes.first().map(|p| TriggerSource::Palette(p.id));
            self.audio_triggers.push(t);
            self.audio_sel = Some(self.audio_triggers.len() - 1);
            dirty = true;
        }
        if let Some(i) = do_remove {
            self.audio_triggers.remove(i);
            match self.audio_sel {
                Some(s) if s == i => self.audio_sel = None,
                Some(s) if s > i => self.audio_sel = Some(s - 1),
                _ => {}
            }
            dirty = true;
        }
        if dirty {
            self.persist_audio();
        }
    }

    /// The spectrum with every trigger's band drawn over it, all draggable.
    fn trigger_graph(&mut self, ui: &mut egui::Ui, spectrum: &[f32; BINS], dirty: &mut bool) {
        let width = ui.available_width().max(320.0);
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(width, 150.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect.expand(1.0));
        painter.rect_filled(rect, 4.0, theme::WELL);

        // Frequency gridlines.
        for (hz, label) in [(100.0, "100"), (1000.0, "1k"), (10_000.0, "10k")] {
            let x = rect.left() + hz_to_x(hz) * rect.width();
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, theme::EDGE),
            );
            painter.text(
                Pos2::new(x + 3.0, rect.top() + 2.0),
                Align2::LEFT_TOP,
                label,
                FontId::proportional(9.0),
                theme::TEXT_DIM,
            );
        }

        // Spectrum bars.
        let bw = rect.width() / BINS as f32;
        for (i, v) in spectrum.iter().enumerate() {
            let h = v * (rect.height() - 4.0);
            if h < 0.5 {
                continue;
            }
            let x0 = rect.left() + i as f32 * bw;
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(x0 + 0.5, rect.bottom() - h),
                    Pos2::new(x0 + bw - 0.5, rect.bottom()),
                ),
                0.0,
                theme::ACCENT_MUTED.gamma_multiply(0.30 + 0.55 * v),
            );
        }

        // Trigger overlays: shaded fire-region above each threshold line.
        for (i, t) in self.audio_triggers.iter().enumerate() {
            let sel = self.audio_sel == Some(i);
            let col = trigger_color(i);
            let dim = if t.enabled { 1.0 } else { 0.35 };
            let x0 = rect.left() + hz_to_x(t.lo_hz) * rect.width();
            let x1 = rect.left() + hz_to_x(t.hi_hz) * rect.width();
            let y = rect.bottom() - t.threshold * rect.height();
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(x0, rect.top()), Pos2::new(x1, y)),
                0.0,
                col.gamma_multiply((if sel { 0.22 } else { 0.10 }) * dim),
            );
            for x in [x0, x1] {
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    Stroke::new(1.0, col.gamma_multiply(0.55 * dim)),
                );
            }
            painter.line_segment(
                [Pos2::new(x0, y), Pos2::new(x1, y)],
                Stroke::new(if sel { 2.0 } else { 1.0 }, col.gamma_multiply(dim)),
            );
            // Firing indicator: swells with the envelope.
            painter.circle_filled(
                Pos2::new((x0 + x1) * 0.5, y),
                3.5 + 3.5 * t.env,
                col.gamma_multiply((0.35 + 0.65 * t.env) * dim),
            );
            painter.text(
                Pos2::new(x0 + 3.0, (y - 12.0).max(rect.top() + 2.0)),
                Align2::LEFT_TOP,
                &t.name,
                FontId::proportional(9.5),
                col.gamma_multiply(dim),
            );
        }

        // ---- interactions ----
        let drag_id = egui::Id::new("audio-graph-drag");
        let to_unit_x = |px: f32| ((px - rect.left()) / rect.width()).clamp(0.0, 1.0);

        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let ux = to_unit_x(pos.x);
                let hit = self
                    .audio_triggers
                    .iter()
                    .position(|t| ux >= hz_to_x(t.lo_hz) && ux <= hz_to_x(t.hi_hz));
                if hit.is_some() {
                    self.audio_sel = hit;
                }
            }
        }

        if resp.drag_started() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let ux = to_unit_x(pos.x);
                let kind = match self.audio_sel.and_then(|s| self.audio_triggers.get(s)) {
                    Some(t) => {
                        let lx = hz_to_x(t.lo_hz);
                        let hx = hz_to_x(t.hi_hz);
                        let x0 = rect.left() + lx * rect.width();
                        let x1 = rect.left() + hx * rect.width();
                        let y = rect.bottom() - t.threshold * rect.height();
                        if (pos.x - x0).abs() < 6.0 {
                            GraphDrag::Lo
                        } else if (pos.x - x1).abs() < 6.0 {
                            GraphDrag::Hi
                        } else if pos.x > x0 && pos.x < x1 && (pos.y - y).abs() < 8.0 {
                            GraphDrag::Threshold
                        } else if pos.x > x0 && pos.x < x1 {
                            GraphDrag::Move {
                                grab: ux - lx,
                                width: hx - lx,
                            }
                        } else {
                            GraphDrag::Paint { x0: ux }
                        }
                    }
                    None => {
                        // Painting on an empty graph starts a fresh trigger.
                        let mut t = AudioTrigger::new(self.audio_triggers.len() + 1);
                        t.source =
                            self.palettes.first().map(|p| TriggerSource::Palette(p.id));
                        self.audio_triggers.push(t);
                        self.audio_sel = Some(self.audio_triggers.len() - 1);
                        GraphDrag::Paint { x0: ux }
                    }
                };
                ui.ctx().data_mut(|d| d.insert_temp(drag_id, kind));
            }
        }

        if resp.dragged() {
            if let (Some(pos), Some(kind), Some(sel)) = (
                resp.interact_pointer_pos(),
                ui.ctx().data_mut(|d| d.get_temp::<GraphDrag>(drag_id)),
                self.audio_sel,
            ) {
                if let Some(t) = self.audio_triggers.get_mut(sel) {
                    let ux = to_unit_x(pos.x);
                    match kind {
                        GraphDrag::Lo => {
                            t.lo_hz = x_to_hz(ux.min(hz_to_x(t.hi_hz) - 0.015));
                        }
                        GraphDrag::Hi => {
                            t.hi_hz = x_to_hz(ux.max(hz_to_x(t.lo_hz) + 0.015));
                        }
                        GraphDrag::Threshold => {
                            t.threshold =
                                ((rect.bottom() - pos.y) / rect.height()).clamp(0.02, 1.0);
                        }
                        GraphDrag::Move { grab, width } => {
                            let lo = (ux - grab).clamp(0.0, 1.0 - width);
                            t.lo_hz = x_to_hz(lo);
                            t.hi_hz = x_to_hz(lo + width);
                        }
                        GraphDrag::Paint { x0 } => {
                            let (a, b) = if ux < x0 { (ux, x0) } else { (x0, ux) };
                            t.lo_hz = x_to_hz(a);
                            t.hi_hz = x_to_hz(b.max(a + 0.015));
                        }
                    }
                }
            }
        }

        if resp.drag_stopped() {
            *dirty = true;
        }
    }

    /// One trigger's editor row. Returns true when it asks to be deleted.
    fn trigger_row(&mut self, ui: &mut egui::Ui, i: usize, dirty: &mut bool) -> bool {
        let sel = self.audio_sel == Some(i);
        let col = trigger_color(i);
        let mut remove = false;
        let frame = egui::Frame::none()
            .fill(if sel { theme::RAISED } else { theme::WELL })
            .stroke(Stroke::new(1.0, if sel { col } else { theme::EDGE }))
            .rounding(4.0)
            .inner_margin(7.0);
        let inner = frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                let t = &mut self.audio_triggers[i];
                if ui.checkbox(&mut t.enabled, "").changed() {
                    *dirty = true;
                }
                let (swatch, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                ui.painter().rect_filled(swatch, 2.0, col);
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut t.name)
                            .desired_width(110.0)
                            .font(egui::TextStyle::Small),
                    )
                    .changed()
                {
                    *dirty = true;
                }
                // Live band meter: how hot the band is against the threshold.
                let energy = t.prev_energy;
                let (meter, _) =
                    ui.allocate_exact_size(egui::vec2(56.0, 10.0), Sense::hover());
                ui.painter().rect_filled(meter, 2.0, theme::WELL);
                ui.painter().rect_filled(
                    Rect::from_min_size(
                        meter.min,
                        egui::vec2(meter.width() * energy.clamp(0.0, 1.0), meter.height()),
                    ),
                    2.0,
                    if energy >= t.threshold {
                        theme::OK
                    } else {
                        theme::ACCENT_MUTED
                    },
                );
                let tx = meter.left() + meter.width() * t.threshold;
                ui.painter().line_segment(
                    [Pos2::new(tx, meter.top()), Pos2::new(tx, meter.bottom())],
                    Stroke::new(1.0, theme::TEXT_DIM),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("x").on_hover_text("Delete").clicked() {
                        remove = true;
                    }
                });
            });

            ui.horizontal(|ui| {
                let t = &mut self.audio_triggers[i];
                let band_name = BAND_PRESETS
                    .iter()
                    .find(|(_, lo, hi)| {
                        (t.lo_hz / lo - 1.0).abs() < 0.05 && (t.hi_hz / hi - 1.0).abs() < 0.05
                    })
                    .map(|(n, _, _)| *n)
                    .unwrap_or("Custom");
                egui::ComboBox::from_id_salt(("audio-band", i))
                    .selected_text(band_name)
                    .width(86.0)
                    .show_ui(ui, |ui| {
                        for (n, lo, hi) in BAND_PRESETS {
                            if ui.selectable_label(band_name == n, n).clicked() {
                                t.lo_hz = lo;
                                t.hi_hz = hi;
                                *dirty = true;
                            }
                        }
                    });
                let mut lo = t.lo_hz;
                let mut hi = t.hi_hz;
                if ui
                    .add(
                        egui::DragValue::new(&mut lo)
                            .range(30.0..=15_500.0)
                            .speed(5.0)
                            .suffix(" Hz"),
                    )
                    .changed()
                {
                    t.lo_hz = lo.min(t.hi_hz - 5.0);
                    *dirty = true;
                }
                if ui
                    .add(
                        egui::DragValue::new(&mut hi)
                            .range(35.0..=16_000.0)
                            .speed(5.0)
                            .suffix(" Hz"),
                    )
                    .changed()
                {
                    t.hi_hz = hi.max(t.lo_hz + 5.0);
                    *dirty = true;
                }
                ui.label("over");
                if ui
                    .add(
                        egui::DragValue::new(&mut t.threshold)
                            .range(0.02..=1.0)
                            .speed(0.01)
                            .fixed_decimals(2),
                    )
                    .on_hover_text("Fires when the band's level crosses this")
                    .changed()
                {
                    *dirty = true;
                }
                egui::ComboBox::from_id_salt(("audio-mode", i))
                    .selected_text(t.mode.label())
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        for m in TriggerMode::ALL {
                            if ui
                                .selectable_value(&mut t.mode, m, m.label())
                                .on_hover_text(m.hint())
                                .changed()
                            {
                                *dirty = true;
                            }
                        }
                    });
                ui.label("atk");
                if ui
                    .add(
                        egui::DragValue::new(&mut t.attack_s)
                            .range(0.0..=2.0)
                            .speed(0.01)
                            .suffix(" s"),
                    )
                    .changed()
                {
                    *dirty = true;
                }
                ui.label("rel");
                if ui
                    .add(
                        egui::DragValue::new(&mut t.release_s)
                            .range(0.02..=5.0)
                            .speed(0.02)
                            .suffix(" s"),
                    )
                    .changed()
                {
                    *dirty = true;
                }
            });

            ui.horizontal(|ui| {
                // Group target.
                let group_label = self.audio_triggers[i]
                    .group
                    .and_then(|gi| self.groups.get(gi))
                    .map(|g| g.name.clone())
                    .unwrap_or_else(|| "All lights".into());
                let mut set_group: Option<Option<usize>> = None;
                egui::ComboBox::from_id_salt(("audio-group", i))
                    .selected_text(group_label)
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        let cur = self.audio_triggers[i].group;
                        if ui.selectable_label(cur.is_none(), "All lights").clicked() {
                            set_group = Some(None);
                        }
                        for (gi, g) in self.groups.iter().enumerate() {
                            if ui.selectable_label(cur == Some(gi), &g.name).clicked() {
                                set_group = Some(Some(gi));
                            }
                        }
                    });
                if let Some(g) = set_group {
                    self.audio_triggers[i].group = g;
                    *dirty = true;
                }

                ui.label("gets");

                // Payload.
                let source_label = match self.audio_triggers[i].source {
                    Some(TriggerSource::Palette(id)) => self
                        .palettes
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| format!("{} · {}", p.feature.short(), p.name))
                        .unwrap_or_else(|| "missing palette".into()),
                    Some(TriggerSource::Preset(pi)) => self
                        .user_presets
                        .get(pi)
                        .map(|p| format!("Preset · {}", p.name))
                        .unwrap_or_else(|| "missing preset".into()),
                    None => "pick a look…".into(),
                };
                let mut set_source: Option<TriggerSource> = None;
                egui::ComboBox::from_id_salt(("audio-source-pick", i))
                    .selected_text(source_label)
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        let cur = self.audio_triggers[i].source;
                        for p in &self.palettes {
                            let is = cur == Some(TriggerSource::Palette(p.id));
                            if ui
                                .selectable_label(
                                    is,
                                    format!("{} · {}", p.feature.short(), p.name),
                                )
                                .clicked()
                            {
                                set_source = Some(TriggerSource::Palette(p.id));
                            }
                        }
                        for (pi, p) in self.user_presets.iter().enumerate() {
                            let is = cur == Some(TriggerSource::Preset(pi));
                            if ui
                                .selectable_label(is, format!("Preset · {}", p.name))
                                .clicked()
                            {
                                set_source = Some(TriggerSource::Preset(pi));
                            }
                        }
                    });
                if let Some(s) = set_source {
                    self.audio_triggers[i].source = Some(s);
                    *dirty = true;
                }

                let t = &mut self.audio_triggers[i];
                egui::ComboBox::from_id_salt(("audio-merge", i))
                    .selected_text(t.merge.label())
                    .width(84.0)
                    .show_ui(ui, |ui| {
                        for m in MergeMode::ALL {
                            if ui
                                .selectable_value(&mut t.merge, m, m.label())
                                .on_hover_text(m.hint())
                                .changed()
                            {
                                *dirty = true;
                            }
                        }
                    });
            });
        });
        if inner.response.clicked() {
            self.audio_sel = Some(i);
        }
        remove
    }
}
