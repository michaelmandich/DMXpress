//! Floating "Palettes" pool window: feature-scoped, referenced presets.
//!
//! Pick a feature tab (Color, Position, …), select fixtures, set the values you
//! want, and Store. Recalling drops those values into the programmer and links
//! the channels back to the palette so cues can reference it.

use eframe::egui;

use super::{apply_zoom, zoom_controls};
use crate::app::{App, Ramp};
use crate::palette::{self, Feature, Palette, PaletteSeq, SeqPattern};
use crate::showbuddy::Role;

impl App {
    /// Reverse-lookup the channel role at a 0-based DMX index.
    pub(crate) fn role_at(&self, addr0: usize) -> Option<Role> {
        for f in &self.patch.fixtures {
            let from0 = f.from as usize - 1;
            if addr0 >= from0 && addr0 < from0 + f.channel_count() {
                return f.channels.get(addr0 - from0).map(|c| c.role());
            }
        }
        None
    }

    /// Whether the channel at a 0-based DMX index is stepped (colour wheels,
    /// macros — anything with switched bands rather than a continuous fade).
    /// A single `S,0,255` band is really a continuous channel (some fixture
    /// files mark plain RGB that way), so only genuinely segmented bands
    /// count.
    pub(crate) fn channel_is_stepped(&self, addr0: usize) -> bool {
        for f in &self.patch.fixtures {
            let from0 = f.from as usize - 1;
            if addr0 >= from0 && addr0 < from0 + f.channel_count() {
                return f.channels.get(addr0 - from0).is_some_and(|c| {
                    // Dimmers compressed into their dim band fade smoothly.
                    c.dim_range().is_none()
                        && c.bands
                            .iter()
                            .any(|b| b.kind == 'S' && !(b.min == 0 && b.max == 255))
                });
            }
        }
        false
    }

    /// Store the selected fixtures' channels of `feature` as a new palette.
    pub(crate) fn store_palette(&mut self, feature: Feature) {
        let fixtures = self.stage.selected_fixtures();
        if fixtures.is_empty() {
            self.log.push("Palettes: select fixtures first".into());
            return;
        }
        let mut values = Vec::new();
        for &fi in &fixtures {
            let Some(f) = self.patch.fixtures.get(fi) else {
                continue;
            };
            for (ci, ch) in f.channels.iter().enumerate() {
                if Feature::of(ch.role()) != feature {
                    continue;
                }
                let addr = f.from as usize + ci;
                if (1..=crate::net::DMX_SLOTS).contains(&addr) {
                    values.push((addr - 1, self.live.base[addr - 1]));
                }
            }
        }
        if values.is_empty() {
            self.log
                .push(format!("Palettes: selection has no {} channels", feature.label()));
            return;
        }
        let name = if self.palette_name.trim().is_empty() {
            let n = self.palettes.iter().filter(|p| p.feature == feature).count() + 1;
            format!("{} {}", feature.label(), n)
        } else {
            self.palette_name.trim().to_string()
        };
        let id = self.next_palette_id;
        self.next_palette_id += 1;
        self.log.push(format!(
            "Stored {} palette \"{}\" ({} ch)",
            feature.label(),
            name,
            values.len()
        ));
        self.palettes.push(Palette {
            id,
            feature,
            name,
            values,
        });
        palette::save_palettes(&self.palettes);
        self.palette_name.clear();
    }

    /// Drop a palette's values into the programmer and link the channels to
    /// it, fading them in over the Palettes window's fade time (stepped
    /// channels snap halfway through instead of sweeping).
    pub(crate) fn recall_palette(&mut self, id: u32) {
        let Some(p) = self.palettes.iter().find(|p| p.id == id).cloned() else {
            return;
        };
        // Settle any running fade so the recall is visible immediately.
        if self.transition_run.take().is_some() {
            self.live = crate::oscillator::Look::from_frame(*self.net.dmx.lock());
        }
        let fade = self
            .palette_transition
            .duration(self.transition.duration, self.palette_fade_s);
        let now = std::time::Instant::now();
        let reference = p.reference();
        for &(addr0, v) in &p.values {
            if fade > 0.01 {
                self.base_fades.insert(
                    addr0,
                    Ramp {
                        from: self.live.base[addr0] as f32,
                        to: v as f32,
                        start: now,
                        dur: fade,
                        stepped: self.channel_is_stepped(addr0),
                        remove_after: false,
                    },
                );
            } else {
                self.base_fades.remove(&addr0);
                self.live.base[addr0] = v;
            }
            self.live_active.insert(addr0);
            self.live_refs.insert(addr0, reference);
        }
        self.log.push(if fade > 0.01 {
            format!(
                "Recalled {} palette \"{}\" ({fade:.1}s fade)",
                p.feature.label(),
                p.name
            )
        } else {
            format!("Recalled {} palette \"{}\"", p.feature.label(), p.name)
        });
    }

    /// Overwrite palette `id` with the current selection's feature channels.
    pub(crate) fn update_palette(&mut self, id: u32) {
        let Some(feature) = self.palettes.iter().find(|p| p.id == id).map(|p| p.feature) else {
            return;
        };
        let fixtures = self.stage.selected_fixtures();
        if fixtures.is_empty() {
            self.log.push("Palettes: select fixtures first".into());
            return;
        }
        let mut values = Vec::new();
        for &fi in &fixtures {
            let Some(f) = self.patch.fixtures.get(fi) else {
                continue;
            };
            for (ci, ch) in f.channels.iter().enumerate() {
                if Feature::of(ch.role()) != feature {
                    continue;
                }
                let addr = f.from as usize + ci;
                if (1..=crate::net::DMX_SLOTS).contains(&addr) {
                    values.push((addr - 1, self.live.base[addr - 1]));
                }
            }
        }
        if let Some(p) = self.palettes.iter_mut().find(|p| p.id == id) {
            p.values = values;
            let name = p.name.clone();
            palette::save_palettes(&self.palettes);
            self.log.push(format!("Updated palette \"{name}\""));
        }
    }

    /// Remove palette `id` and drop any programmer links to it.
    pub(crate) fn delete_palette(&mut self, id: u32) {
        if let Some(pos) = self.palettes.iter().position(|p| p.id == id) {
            let p = self.palettes.remove(pos);
            self.live_refs.retain(|_, r| r.id != id);
            let mut i = 0;
            while i < self.cycle_ids.len() {
                if self.cycle_ids[i] == id {
                    self.cycle_ids.remove(i);
                    if i < self.cycle_weights.len() {
                        self.cycle_weights.remove(i);
                    }
                } else {
                    i += 1;
                }
            }
            palette::save_palettes(&self.palettes);
            self.log.push(format!("Deleted palette \"{}\"", p.name));
        }
    }

    /// Empty the programmer: nothing held, nothing active.
    pub(crate) fn clear_programmer(&mut self) {
        self.live = crate::oscillator::Look::black();
        self.live_active.clear();
        self.live_refs.clear();
        self.transition_run = None;
        self.base_fades.clear();
        self.osc_ramps.clear();
        self.log.push("Programmer cleared".into());
    }

    /// Representative colour for a palette tile (real colour for Color palettes,
    /// a feature tint otherwise).
    fn palette_swatch(&self, p: &Palette) -> egui::Color32 {
        let (mut r, mut g, mut b, mut w) = (0u8, 0u8, 0u8, 0u8);
        let mut colored = false;
        for &(addr0, v) in &p.values {
            match self.role_at(addr0) {
                Some(Role::Red) => {
                    r = r.max(v);
                    colored = true;
                }
                Some(Role::Green) => {
                    g = g.max(v);
                    colored = true;
                }
                Some(Role::Blue) => {
                    b = b.max(v);
                    colored = true;
                }
                Some(Role::White) => {
                    w = w.max(v);
                    colored = true;
                }
                _ => {}
            }
        }
        if colored {
            let mix = |c: u8| (c as u16 + w as u16).min(255) as u8;
            egui::Color32::from_rgb(mix(r), mix(g), mix(b))
        } else {
            feature_color(p.feature)
        }
    }

    pub(crate) fn palettes_window(&mut self, ctx: &egui::Context) {
        if !self.show_palettes {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_palettes;
        let mut do_store: Option<Feature> = None;
        let mut do_recall: Option<u32> = None;
        let mut do_update: Option<u32> = None;
        let mut do_delete: Option<u32> = None;
        let mut do_clear = false;
        let mut do_seq_save = false;
        let mut do_seq_load: Option<(usize, bool)> = None; // (index, start)
        let mut do_seq_update: Option<usize> = None;
        let mut do_seq_master: Option<usize> = None;
        let mut do_seq_move: Option<(usize, String)> = None;
        let mut do_seq_delete: Option<usize> = None;
        let mut do_folder_add = false;
        let mut do_folder_delete: Option<String> = None;

        egui::Window::new("Palettes")
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .default_size([360.0, 340.0])
            .default_pos([screen.right() - 400.0, 180.0])
            .show(ctx, |ui| {
                zoom_controls(ui, &mut self.zoom.palettes);
                apply_zoom(ui, self.zoom.palettes);

                // Feature tabs.
                ui.horizontal_wrapped(|ui| {
                    for feat in Feature::ALL {
                        let label = feat.label().to_string();
                        if ui
                            .selectable_label(self.palette_tab == feat, label)
                            .clicked()
                        {
                            self.palette_tab = feat;
                        }
                    }
                });
                ui.separator();

                let feat = self.palette_tab;
                let sel = self.stage.selected_fixtures();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.palette_name)
                            .hint_text("name…")
                            .desired_width(120.0),
                    );
                    if ui
                        .add_enabled(!sel.is_empty(), egui::Button::new(format!("＋ Store {}", feat.label())))
                        .on_hover_text("Store this feature for the selected fixtures")
                        .clicked()
                    {
                        do_store = Some(feat);
                    }
                });
                ui.horizontal(|ui| {
                    ui.weak(format!("{} active programmer ch", self.live_active.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("Clear")
                            .on_hover_text("Empty the programmer")
                            .clicked()
                        {
                            do_clear = true;
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Fade");
                    if ui
                        .small_button(self.palette_transition.short_label())
                        .on_hover_text(
                            "Transition binding: M = master transition, C = custom \
                             duration, — = none. Click to cycle.",
                        )
                        .clicked()
                    {
                        self.palette_transition = self.palette_transition.next();
                    }
                    ui.add(
                        egui::Slider::new(&mut self.palette_fade_s, 0.0..=10.0)
                            .suffix(" s")
                            .max_decimals(1),
                    )
                    .on_hover_text(
                        "Custom palette transition duration. It is used when the \
                         square says C; M follows the master Transition window.",
                    );
                });
                ui.separator();

                // Palette cycle: pick palettes (⇧click tiles), then rotate
                // through them on the beat — blue→yellow→blue, or any N.
                ui.horizontal_wrapped(|ui| {
                    ui.label("🔁 Cycle:");
                    if self.cycle_ids.is_empty() {
                        ui.weak("⇧click palettes to add…");
                    }
                    let mut remove: Option<usize> = None;
                    for (k, id) in self.cycle_ids.iter().enumerate() {
                        match self.palettes.iter().find(|p| p.id == *id) {
                            Some(p) => {
                                if ui
                                    .button(&p.name)
                                    .on_hover_text("Click to remove from the cycle")
                                    .clicked()
                                {
                                    remove = Some(k);
                                }
                            }
                            None => remove = Some(k),
                        }
                    }
                    if let Some(k) = remove {
                        self.cycle_ids.remove(k);
                        if k < self.cycle_weights.len() {
                            self.cycle_weights.remove(k);
                        }
                        self.cycle_seq = None;
                    }
                });
                ui.checkbox(&mut self.advanced_palette_mode, "Advanced segments")
                    .on_hover_text(
                        "Shows weighted color bands. Shift-click can add the same \
                         palette more than once; drag a band's right edge to resize it.",
                    );
                if self.advanced_palette_mode && !self.cycle_ids.is_empty() {
                    while self.cycle_weights.len() < self.cycle_ids.len() {
                        self.cycle_weights.push(1.0);
                    }
                    self.cycle_weights.truncate(self.cycle_ids.len());
                    let desired = egui::vec2(ui.available_width().max(180.0), 42.0);
                    let (bar, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
                    let total: f32 = self.cycle_weights.iter().sum::<f32>().max(0.05);
                    let mut x = bar.left();
                    for i in 0..self.cycle_ids.len() {
                        let width = bar.width() * self.cycle_weights[i] / total;
                        let rect = egui::Rect::from_min_max(
                            egui::pos2(x, bar.top()),
                            egui::pos2((x + width).min(bar.right()), bar.bottom()),
                        );
                        let color = self
                            .palettes
                            .iter()
                            .find(|p| p.id == self.cycle_ids[i])
                            .map_or(egui::Color32::DARK_GRAY, |p| self.palette_swatch(p));
                        ui.painter().rect_filled(rect.shrink(1.0), 2.0, color);
                        let handle = egui::Rect::from_center_size(
                            egui::pos2(rect.right(), rect.center().y),
                            egui::vec2(10.0, rect.height()),
                        );
                        let response = ui.interact(
                            handle,
                            egui::Id::new(("palette_segment_edge", i)),
                            egui::Sense::drag(),
                        );
                        if response.dragged() {
                            let dx = ui.input(|input| input.pointer.delta().x);
                            self.cycle_weights[i] =
                                (self.cycle_weights[i] + dx / bar.width() * total).max(0.08);
                            self.cycle_seq = None;
                        }
                        ui.painter().line_segment(
                            [handle.center_top(), handle.center_bottom()],
                            egui::Stroke::new(2.0, egui::Color32::WHITE),
                        );
                        x += width;
                    }
                }
                ui.horizontal(|ui| {
                    let can = self.cycle_ids.len() >= 2;
                    let label = if self.cycle_on { "⏹ Stop cycle" } else { "▶ Cycle" };
                    if ui
                        .add_enabled(can || self.cycle_on, egui::Button::new(label))
                        .on_hover_text("Rotate all lights through the picked palettes on the beat")
                        .clicked()
                    {
                        self.cycle_on = !self.cycle_on;
                        if !self.cycle_on {
                            self.cycle_seq = None;
                        }
                        if self.cycle_on {
                            self.cycle_beats = 0.0;
                            self.cycle_last = None;
                            self.cycle_tempo = if self.master_bpm_on {
                                self.master_bpm
                            } else {
                                self.live.tempo
                            };
                        }
                    }
                    const STEPS: [(&str, f32); 6] = [
                        ("4 bars", 16.0),
                        ("2 bars", 8.0),
                        ("1 bar", 4.0),
                        ("1/2", 2.0),
                        ("1/4", 1.0),
                        ("1/8", 0.5),
                    ];
                    let cur = STEPS
                        .iter()
                        .find(|(_, v)| (*v - self.cycle_beats_per).abs() < 0.01)
                        .map_or("1 bar", |(n, _)| n);
                    egui::ComboBox::from_id_salt("cycle_rate")
                        .selected_text(cur)
                        .width(70.0)
                        .show_ui(ui, |ui| {
                            for (name, v) in STEPS {
                                ui.selectable_value(&mut self.cycle_beats_per, v, name);
                            }
                        })
                        .response
                        .on_hover_text("How long each palette holds");
                    egui::ComboBox::from_id_salt("cycle_pat")
                        .selected_text(self.cycle_pattern.label())
                        .width(70.0)
                        .show_ui(ui, |ui| {
                            for pat in SeqPattern::ALL {
                                ui.selectable_value(&mut self.cycle_pattern, pat, pat.label());
                            }
                        })
                        .response
                        .on_hover_text("How the spacing disperses across the rig");
                });
                if ui
                    .checkbox(&mut self.cycle_master_beat, "♪ Follow master beat")
                    .on_hover_text(
                        "On: taps and Master BPM gently pull this cycle into sync. \
                         Off: the cycle keeps its current tempo",
                    )
                    .changed()
                {
                    self.cycle_tempo = if self.master_bpm_on {
                        self.master_bpm
                    } else {
                        self.live.tempo
                    };
                    self.cycle_beat_nudge = 0.0;
                }
                ui.horizontal(|ui| {
                    ui.label("Spacing");
                    ui.add(
                        egui::Slider::new(&mut self.cycle_spread, 0.0..=1.0)
                            .show_value(false),
                    )
                    .on_hover_text(
                        "0 = all lights change together, 1 = the whole cycle \
                         spread across the rig",
                    );
                    ui.label("Snap");
                    ui.add(
                        egui::Slider::new(&mut self.cycle_shape, 0.0..=1.0)
                            .show_value(false),
                    )
                    .on_hover_text("0 = smooth crossfade, 1 = hard snap on the step");
                });

                // Saved sequences: a cycle plus its motion, stored by name,
                // organised into folders like presets.
                egui::CollapsingHeader::new("🎞 Saved sequences")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.seq_name)
                                    .hint_text("sequence name…")
                                    .desired_width(110.0),
                            );
                            if ui
                                .add_enabled(
                                    self.cycle_ids.len() >= 2,
                                    egui::Button::new("＋ Save"),
                                )
                                .on_hover_text(
                                    "Save the current cycle — colours, rate, \
                                     spacing, pattern and snap — as a tile",
                                )
                                .clicked()
                            {
                                do_seq_save = true;
                            }
                            if ui
                                .add_enabled(
                                    !self.seq_name.trim().is_empty(),
                                    egui::Button::new("＋ Folder"),
                                )
                                .on_hover_text("Create a folder with the typed name")
                                .clicked()
                            {
                                do_folder_add = true;
                            }
                            ui.checkbox(&mut self.seq_drag_mode, "✋ Drag mode")
                                .on_hover_text(
                                    "On: drag tiles onto folders to move them. \
                                     Off: click tiles to start/stop as usual",
                                );
                        });
                        let seq_tile = |ui: &mut egui::Ui,
                                        i: usize,
                                        s: &PaletteSeq,
                                        running: bool,
                                        drag_mode: bool,
                                        folders: &[String],
                                        do_load: &mut Option<(usize, bool)>,
                                        do_upd: &mut Option<usize>,
                                        do_master: &mut Option<usize>,
                                        do_mv: &mut Option<(usize, String)>,
                                        do_del: &mut Option<usize>| {
                            let text = egui::RichText::new(format!(
                                "{}{}\n{}{} pal · {}",
                                if running { "▶ " } else { "" },
                                s.name,
                                if s.master_beat { "♪ " } else { "" },
                                s.ids.len(),
                                s.pattern.label()
                            ))
                            .size(11.0);
                            let mut btn = egui::Button::new(text)
                                .fill(super::theme::RAISED)
                                .min_size([84.0, 40.0].into());
                            if running {
                                btn = btn.stroke(egui::Stroke::new(
                                    2.0,
                                    super::theme::ACCENT_SOFT,
                                ));
                            }
                            let resp = if drag_mode {
                                ui.dnd_drag_source(
                                    egui::Id::new(("seq_drag", i)),
                                    i,
                                    |ui| {
                                        ui.add(btn);
                                    },
                                )
                                .response
                                .on_hover_text("Drag onto a folder to move")
                            } else {
                                ui.add(btn).on_hover_text(if running {
                                    "Click to stop"
                                } else {
                                    "Click to load and start this sequence"
                                })
                            };
                            if resp.clicked() {
                                *do_load = Some((i, !running));
                            }
                            resp.context_menu(|ui| {
                                if ui.button("Load (don't start)").clicked() {
                                    *do_load = Some((i, false));
                                    ui.close_menu();
                                }
                                if ui.button("Overwrite with current cycle").clicked() {
                                    *do_upd = Some(i);
                                    ui.close_menu();
                                }
                                let mut follow = s.master_beat;
                                if ui
                                    .checkbox(&mut follow, "Follow master beat")
                                    .on_hover_text(
                                        "Opt this card in or out of taps and Master BPM",
                                    )
                                    .changed()
                                {
                                    *do_master = Some(i);
                                    ui.close_menu();
                                }
                                ui.menu_button("Move to", |ui| {
                                    if !s.folder.is_empty() && ui.button("(root)").clicked()
                                    {
                                        *do_mv = Some((i, String::new()));
                                        ui.close_menu();
                                    }
                                    for f in folders {
                                        if *f != s.folder && ui.button(f).clicked() {
                                            *do_mv = Some((i, f.clone()));
                                            ui.close_menu();
                                        }
                                    }
                                });
                                ui.separator();
                                if ui.button("Delete").clicked() {
                                    *do_del = Some(i);
                                    ui.close_menu();
                                }
                            });
                        };
                        if self.seqs.is_empty() {
                            ui.weak("No sequences yet — build a cycle, then Save.");
                        }
                        let dragging_seq = egui::DragAndDrop::has_payload_of_type::<usize>(ui.ctx());
                        let drag_mode = self.seq_drag_mode;
                        egui::ScrollArea::vertical()
                            .id_salt("seq_pool_scroll")
                            .max_height(300.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                        let (root_rect, root_dropped) = ui
                            .dnd_drop_zone::<usize, _>(egui::Frame::none(), |ui| {
                                if dragging_seq {
                                    ui.weak("⬇ drop here for (root)");
                                }
                                ui.horizontal_wrapped(|ui| {
                                    for (i, s) in self.seqs.iter().enumerate() {
                                        if !s.folder.is_empty() {
                                            continue;
                                        }
                                        let running =
                                            self.cycle_on && self.cycle_seq == Some(i);
                                        seq_tile(
                                            ui,
                                            i,
                                            s,
                                            running,
                                            drag_mode,
                                            &self.seq_folders,
                                            &mut do_seq_load,
                                            &mut do_seq_update,
                                            &mut do_seq_master,
                                            &mut do_seq_move,
                                            &mut do_seq_delete,
                                        );
                                    }
                                });
                            });
                        let _ = root_rect;
                        if let Some(dropped) = root_dropped {
                            do_seq_move = Some((*dropped, String::new()));
                        }
                        for folder in self.seq_folders.clone() {
                            let head = egui::CollapsingHeader::new(format!("📁 {folder}"))
                                .default_open(false)
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        for (i, s) in self.seqs.iter().enumerate() {
                                            if s.folder != folder {
                                                continue;
                                            }
                                            let running = self.cycle_on
                                                && self.cycle_seq == Some(i);
                                            seq_tile(
                                                ui,
                                                i,
                                                s,
                                                running,
                                                drag_mode,
                                                &self.seq_folders,
                                                &mut do_seq_load,
                                                &mut do_seq_update,
                                                &mut do_seq_master,
                                                &mut do_seq_move,
                                                &mut do_seq_delete,
                                            );
                                        }
                                    });
                                });
                            head.header_response.context_menu(|ui| {
                                if ui.button("Delete folder").clicked() {
                                    do_folder_delete = Some(folder.clone());
                                    ui.close_menu();
                                }
                            });
                            // Drag-and-drop: dropping a sequence tile on the
                            // folder header moves it into the folder.
                            if head
                                .header_response
                                .dnd_hover_payload::<usize>()
                                .is_some()
                            {
                                ui.painter().rect_stroke(
                                    head.header_response.rect,
                                    4.0,
                                    egui::Stroke::new(
                                        2.0,
                                        egui::Color32::from_rgb(250, 165, 60),
                                    ),
                                );
                            }
                            if let Some(dropped) =
                                head.header_response.dnd_release_payload::<usize>()
                            {
                                do_seq_move = Some((*dropped, folder.clone()));
                            }
                        }
                            }); // sequence pool scroll area
                    });
                ui.separator();

                let here: Vec<Palette> = self
                    .palettes
                    .iter()
                    .filter(|p| p.feature == feat)
                    .cloned()
                    .collect();
                if here.is_empty() {
                    ui.weak(format!(
                        "No {} palettes yet — select fixtures, set values, Store.",
                        feat.label()
                    ));
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for p in &here {
                            let fill = self.palette_swatch(p);
                            let lum = 0.299 * fill.r() as f32
                                + 0.587 * fill.g() as f32
                                + 0.114 * fill.b() as f32;
                            let text_col = if lum > 140.0 {
                                egui::Color32::BLACK
                            } else {
                                egui::Color32::WHITE
                            };
                            let active = p.values.iter().any(|(a, _)| {
                                self.live_refs.get(a).is_some_and(|r| r.id == p.id)
                            });
                            let in_cycle = self.cycle_ids.contains(&p.id);
                            let text =
                                egui::RichText::new(format!(
                                    "{}{}\n{} ch",
                                    p.name,
                                    if in_cycle { " 🔁" } else { "" },
                                    p.values.len()
                                ))
                                    .color(text_col)
                                    .size(12.0);
                            let mut btn = egui::Button::new(text).fill(fill).min_size([84.0, 46.0].into());
                            if in_cycle {
                                btn = btn.stroke(egui::Stroke::new(2.0, egui::Color32::from_rgb(250, 165, 60)));
                            } else if active {
                                btn = btn.stroke(egui::Stroke::new(2.0, egui::Color32::from_rgb(120, 200, 255)));
                            }
                            let resp = ui.add(btn);
                            if resp.clicked() {
                                if ui.input(|i| i.modifiers.shift) {
                                    if self.advanced_palette_mode {
                                        self.cycle_ids.push(p.id);
                                        self.cycle_weights.push(1.0);
                                    } else if let Some(k) =
                                        self.cycle_ids.iter().position(|x| *x == p.id)
                                    {
                                        self.cycle_ids.remove(k);
                                        if k < self.cycle_weights.len() {
                                            self.cycle_weights.remove(k);
                                        }
                                    } else {
                                        self.cycle_ids.push(p.id);
                                        self.cycle_weights.push(1.0);
                                    }
                                    self.cycle_seq = None;
                                } else {
                                    do_recall = Some(p.id);
                                }
                            }
                            resp.context_menu(|ui| {
                                if ui.button("Recall").clicked() {
                                    do_recall = Some(p.id);
                                    ui.close_menu();
                                }
                                if ui
                                    .button(if in_cycle {
                                        "Remove from cycle"
                                    } else {
                                        "🔁 Add to cycle"
                                    })
                                    .clicked()
                                {
                                    if let Some(k) =
                                        self.cycle_ids.iter().position(|x| *x == p.id)
                                    {
                                        self.cycle_ids.remove(k);
                                        if k < self.cycle_weights.len() {
                                            self.cycle_weights.remove(k);
                                        }
                                    } else {
                                        self.cycle_ids.push(p.id);
                                        self.cycle_weights.push(1.0);
                                    }
                                    self.cycle_seq = None;
                                    ui.close_menu();
                                }
                                if ui.button("Update from selection").clicked() {
                                    do_update = Some(p.id);
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui.button("Delete").clicked() {
                                    do_delete = Some(p.id);
                                    ui.close_menu();
                                }
                            });
                        }
                    });
                });
            });
        self.show_palettes = open;

        if let Some(f) = do_store {
            self.store_palette(f);
        }
        if let Some(id) = do_recall {
            self.recall_palette(id);
        }
        if let Some(id) = do_update {
            self.update_palette(id);
        }
        if let Some(id) = do_delete {
            self.delete_palette(id);
        }
        if do_clear {
            self.clear_programmer();
        }
        let mut seqs_dirty = false;
        if do_seq_save {
            let name = if self.seq_name.trim().is_empty() {
                format!("Seq {}", self.seqs.len() + 1)
            } else {
                self.seq_name.trim().to_string()
            };
            self.seqs.push(PaletteSeq {
                name: name.clone(),
                folder: String::new(),
                ids: self.cycle_ids.clone(),
                weights: self.cycle_weights.clone(),
                beats_per: self.cycle_beats_per,
                master_beat: self.cycle_master_beat,
                spread: self.cycle_spread,
                pattern: self.cycle_pattern,
                shape: self.cycle_shape,
            });
            self.seq_name.clear();
            self.log.push(format!("Saved sequence \"{name}\""));
            seqs_dirty = true;
        }
        if do_folder_add {
            let name = self.seq_name.trim().to_string();
            if !name.is_empty() && !self.seq_folders.contains(&name) {
                self.seq_folders.push(name);
                self.seq_folders
                    .sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
                self.seq_name.clear();
                seqs_dirty = true;
            }
        }
        if let Some(folder) = do_folder_delete {
            self.seq_folders.retain(|f| *f != folder);
            for s in &mut self.seqs {
                if s.folder == folder {
                    s.folder.clear();
                }
            }
            seqs_dirty = true;
        }
        if let Some((i, start)) = do_seq_load {
            if let Some(s) = self.seqs.get(i).cloned() {
                if !start && self.cycle_on && self.cycle_seq == Some(i) {
                    self.cycle_on = false;
                    self.cycle_seq = None;
                } else {
                    self.cycle_ids = s.ids.clone();
                    self.cycle_weights = s.weights.clone();
                    while self.cycle_weights.len() < self.cycle_ids.len() {
                        self.cycle_weights.push(1.0);
                    }
                    self.cycle_seq = Some(i);
                    self.cycle_beats_per = s.beats_per;
                    self.cycle_master_beat = s.master_beat;
                    self.cycle_tempo = if self.master_bpm_on {
                        self.master_bpm
                    } else {
                        self.live.tempo
                    };
                    self.cycle_beat_nudge = 0.0;
                    self.cycle_spread = s.spread;
                    self.cycle_pattern = s.pattern;
                    self.cycle_shape = s.shape;
                    if start {
                        self.cycle_on = true;
                        self.cycle_beats = 0.0;
                        self.cycle_last = None;
                        self.log.push(format!("Sequence \"{}\" running", s.name));
                    }
                }
            }
        }
        if let Some(i) = do_seq_update {
            if let Some(s) = self.seqs.get_mut(i) {
                s.ids = self.cycle_ids.clone();
                s.weights = self.cycle_weights.clone();
                s.beats_per = self.cycle_beats_per;
                s.master_beat = self.cycle_master_beat;
                s.spread = self.cycle_spread;
                s.pattern = self.cycle_pattern;
                s.shape = self.cycle_shape;
                seqs_dirty = true;
            }
        }
        if let Some(i) = do_seq_master {
            if let Some(s) = self.seqs.get_mut(i) {
                s.master_beat = !s.master_beat;
                if self.cycle_on && self.cycle_seq == Some(i) {
                    self.cycle_master_beat = s.master_beat;
                    self.cycle_tempo = if self.master_bpm_on {
                        self.master_bpm
                    } else {
                        self.live.tempo
                    };
                    self.cycle_beat_nudge = 0.0;
                }
                seqs_dirty = true;
            }
        }
        if let Some((i, folder)) = do_seq_move {
            if let Some(s) = self.seqs.get_mut(i) {
                s.folder = folder;
                seqs_dirty = true;
            }
        }
        if let Some(i) = do_seq_delete {
            if i < self.seqs.len() {
                self.seqs.remove(i);
                match self.cycle_seq {
                    Some(k) if k == i => self.cycle_seq = None,
                    Some(k) if k > i => self.cycle_seq = Some(k - 1),
                    _ => {}
                }
                seqs_dirty = true;
            }
        }
        if seqs_dirty {
            palette::save_seqs(&self.seq_folders, &self.seqs);
        }
    }
}

/// Tint used for non-colour palette tiles.
fn feature_color(f: Feature) -> egui::Color32 {
    use egui::Color32 as C;
    match f {
        Feature::Dimmer => C::from_rgb(120, 100, 40),
        Feature::Position => C::from_rgb(40, 80, 110),
        Feature::Color => C::from_rgb(90, 60, 90),
        Feature::Beam => C::from_rgb(90, 90, 40),
        Feature::Focus => C::from_rgb(70, 60, 100),
        Feature::Control => C::from_rgb(70, 70, 70),
    }
}
