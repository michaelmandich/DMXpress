//! Floating "Scenes" pool window: capture what the programmer is doing right
//! now — including every running wave, with its phase — and play it back as
//! its own mixer layer, so several effects can run at once.

use eframe::egui;

use super::{apply_zoom, theme, zoom_controls};
use crate::app::App;
use crate::preset::SavedOsc;
use crate::scene::{self, MergeMode, Scene};

/// Deferred row action, applied after the pool has been drawn.
#[derive(Clone, Copy)]
enum SceneAction {
    Toggle,
    Up,
    Down,
    Remove,
    Update,
}

impl App {
    /// Save whatever the programmer is holding as a new scene.
    pub(crate) fn capture_scene(&mut self) {
        let mut active: Vec<usize> = self.live_active.iter().copied().collect();
        active.sort_unstable();
        if active.is_empty() {
            self.log
                .push("Scenes: programmer is empty — build a look first".into());
            return;
        }
        let values: Vec<(usize, u8)> = active.iter().map(|&a| (a, self.live.base[a])).collect();
        let oscs: Vec<(usize, SavedOsc)> = self
            .live
            .oscs
            .iter()
            .filter(|(a, _)| self.live_active.contains(a))
            .map(|(&a, o)| {
                (
                    a,
                    SavedOsc {
                        invert: o.invert,
                        amount: o.amount,
                        phase: o.phase,
                        subdiv: o.subdiv,
                        shape: o.shape,
                        custom_wave: o.custom_wave.clone(),
                        master_beat: o.master_beat,
                        local_beats: o.local_beats,
                        local_tempo: o.local_tempo,
                    },
                )
            })
            .collect();
        let name = if self.scene_name.trim().is_empty() {
            format!("Scene {}", self.scenes.len() + 1)
        } else {
            self.scene_name.trim().to_string()
        };
        self.log.push(format!(
            "Captured scene \"{name}\" ({} channels, {} waves)",
            values.len(),
            oscs.len()
        ));
        self.scenes.push(Scene {
            name,
            color: [90, 150, 155],
            values,
            oscs,
            active,
            speed: self.live.speed,
            tempo: self.live.tempo,
            master_speed: self.live.master_speed,
            merge: MergeMode::Override,
            level: 1.0,
            fade: 0.0,
            hold: 0.0,
            order: self
                .active_order
                .and_then(|i| self.orders.get(i))
                .map(|o| o.name.clone()),
            run: None,
        });
        scene::save_scenes(&self.scenes);
        self.scene_name.clear();
    }

    /// Re-capture the programmer into an existing scene, keeping its playback
    /// settings.
    fn update_scene(&mut self, idx: usize) {
        if idx >= self.scenes.len() || self.live_active.is_empty() {
            self.log
                .push("Scenes: programmer is empty — nothing to update from".into());
            return;
        }
        let keep = self.scenes[idx].clone();
        let was_running = keep.is_running();
        self.capture_scene();
        let Some(mut fresh) = self.scenes.pop() else {
            return;
        };
        fresh.name = keep.name;
        fresh.color = keep.color;
        fresh.merge = keep.merge;
        fresh.level = keep.level;
        fresh.fade = keep.fade;
        fresh.hold = keep.hold;
        self.scenes[idx] = fresh;
        if was_running {
            self.scenes[idx].start();
        }
        scene::save_scenes(&self.scenes);
        self.log
            .push(format!("Updated scene \"{}\"", self.scenes[idx].name));
    }

    pub(crate) fn start_scene(&mut self, idx: usize) {
        if let Some(sc) = self.scenes.get_mut(idx) {
            sc.start();
            let name = sc.name.clone();
            self.log.push(format!("Scene \"{name}\" go"));
        }
    }

    pub(crate) fn stop_scene(&mut self, idx: usize) {
        if let Some(sc) = self.scenes.get_mut(idx) {
            sc.stop();
        }
    }

    pub(crate) fn stop_all_scenes(&mut self) {
        for sc in &mut self.scenes {
            sc.stop();
        }
        self.scene_chain = false;
        self.log.push("All scenes released".into());
    }

    /// Hand the chain on when the scene on stage runs out its hold time, so
    /// looks arrive back to back from whichever route each was captured on.
    pub(crate) fn advance_scene_chain(&mut self) {
        if !self.scene_chain {
            return;
        }
        let Some(done) = self.scenes.iter().position(|s| s.expired()) else {
            return;
        };
        let next = (done + 1) % self.scenes.len();
        self.scenes[done].stop();
        if next != done {
            self.scenes[next].start();
        }
        let name = self.scenes[next].name.clone();
        self.log.push(format!("Chain -> \"{name}\""));
    }

    /// Start the chain from the first scene, releasing anything else.
    fn start_chain(&mut self) {
        if self.scenes.is_empty() {
            return;
        }
        for sc in &mut self.scenes {
            sc.stop();
        }
        self.scenes[0].start();
        self.scene_chain = true;
        self.log.push("Scene chain running".into());
    }

    pub(crate) fn scenes_window(&mut self, ctx: &egui::Context) {
        if !self.show_scenes {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_scenes;
        let mut do_capture = false;
        let mut do_chain = false;
        let mut do_stop_all = false;
        let mut do_row: Option<(usize, SceneAction)> = None;
        let mut do_order: Option<String> = None;
        let mut dirty = false;

        egui::Window::new("Scenes")
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .default_size([430.0, 480.0])
            .default_pos([screen.right() - 470.0, 120.0])
            .show(ctx, |ui| {
                zoom_controls(ui, &mut self.zoom.scenes);
                apply_zoom(ui, self.zoom.scenes);

                theme::section(ui, "Capture");
                theme::hint(
                    ui,
                    "Takes the programmer exactly as it stands — base values and \
                     every running wave, with the phase it is at. Two scenes \
                     captured from different routes keep their own direction.",
                );
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.scene_name)
                            .hint_text("name…")
                            .desired_width(150.0),
                    );
                    let armed = !self.live_active.is_empty();
                    if ui
                        .add_enabled(armed, egui::Button::new("Capture"))
                        .on_hover_text("Store what is playing right now as a scene")
                        .clicked()
                    {
                        do_capture = true;
                    }
                    ui.label(format!("{} ch armed", self.live_active.len()));
                });

                ui.add_space(6.0);
                theme::section(ui, "Pool");
                theme::hint(
                    ui,
                    "Priority runs top to bottom: a scene folds over everything \
                     above it in this list. All of them sit under the programmer.",
                );
                ui.horizontal(|ui| {
                    let running = self.scenes.iter().filter(|s| s.is_running()).count();
                    if ui
                        .add_enabled(!self.scenes.is_empty(), egui::Button::new("Run chain"))
                        .on_hover_text(
                            "Play the pool in order, each scene handing to the next \
                             when its hold time runs out",
                        )
                        .clicked()
                    {
                        do_chain = true;
                    }
                    if ui
                        .add_enabled(running > 0, egui::Button::new("Stop all"))
                        .clicked()
                    {
                        do_stop_all = true;
                    }
                    if self.scene_chain {
                        theme::pill(ui, "CHAIN", theme::ACCENT);
                    }
                    theme::pill(
                        ui,
                        &format!("{running} live"),
                        if running > 0 {
                            theme::OK
                        } else {
                            theme::TEXT_DIM
                        },
                    );
                });

                if self.scenes.is_empty() {
                    theme::hint(
                        ui,
                        "No scenes yet — build a wave, then press Capture. Build a \
                         second one from another direction and run them together.",
                    );
                    return;
                }

                ui.add_space(4.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let last = self.scenes.len() - 1;
                    for i in 0..self.scenes.len() {
                        let running = self.scenes[i].is_running();
                        let frame = egui::Frame::none()
                            .fill(if running { theme::RAISED } else { theme::WELL })
                            .stroke(egui::Stroke::new(
                                1.0,
                                if running { theme::ACCENT } else { theme::EDGE },
                            ))
                            .rounding(4.0)
                            .inner_margin(7.0);
                        frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let go = if running { "Stop" } else { "Go" };
                                if ui.button(go).clicked() {
                                    do_row = Some((i, SceneAction::Toggle));
                                }
                                ui.label(
                                    egui::RichText::new(&self.scenes[i].name)
                                        .strong()
                                        .color(if running { theme::ACCENT_SOFT } else { theme::TEXT }),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("x").on_hover_text("Delete").clicked() {
                                            do_row = Some((i, SceneAction::Remove));
                                        }
                                        if ui
                                            .add_enabled(
                                                i < last,
                                                egui::Button::new("v").small(),
                                            )
                                            .on_hover_text("Lower priority")
                                            .clicked()
                                        {
                                            do_row = Some((i, SceneAction::Down));
                                        }
                                        if ui
                                            .add_enabled(i > 0, egui::Button::new("^").small())
                                            .on_hover_text("Raise priority")
                                            .clicked()
                                        {
                                            do_row = Some((i, SceneAction::Up));
                                        }
                                        if ui
                                            .small_button("update")
                                            .on_hover_text("Re-capture from the programmer")
                                            .clicked()
                                        {
                                            do_row = Some((i, SceneAction::Update));
                                        }
                                    },
                                );
                            });

                            ui.horizontal(|ui| {
                                let sc = &mut self.scenes[i];
                                egui::ComboBox::from_id_salt(("scene-merge", i))
                                    .selected_text(sc.merge.label())
                                    .width(92.0)
                                    .show_ui(ui, |ui| {
                                        for mode in MergeMode::ALL {
                                            if ui
                                                .selectable_value(
                                                    &mut sc.merge,
                                                    mode,
                                                    mode.label(),
                                                )
                                                .on_hover_text(mode.hint())
                                                .changed()
                                            {
                                                dirty = true;
                                            }
                                        }
                                    });
                                if ui
                                    .add(
                                        egui::Slider::new(&mut sc.level, 0.0..=1.0)
                                            .show_value(false),
                                    )
                                    .on_hover_text("Level")
                                    .changed()
                                {
                                    dirty = true;
                                }
                                ui.label(format!("{:.0}%", sc.level * 100.0));
                            });

                            ui.horizontal(|ui| {
                                let sc = &mut self.scenes[i];
                                ui.label("fade");
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut sc.fade)
                                            .range(0.0..=30.0)
                                            .speed(0.1)
                                            .suffix(" s"),
                                    )
                                    .on_hover_text("Seconds to ease in on Go")
                                    .changed()
                                {
                                    dirty = true;
                                }
                                ui.label("hold");
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut sc.hold)
                                            .range(0.0..=600.0)
                                            .speed(0.25)
                                            .suffix(" s"),
                                    )
                                    .on_hover_text(
                                        "Seconds on stage before the chain moves on; \
                                         0 stays until stopped",
                                    )
                                    .changed()
                                {
                                    dirty = true;
                                }
                            });

                            let sc = &self.scenes[i];
                            ui.horizontal_wrapped(|ui| {
                                theme::hint(
                                    ui,
                                    format!(
                                        "{} ch · {} wave(s)",
                                        sc.active.len(),
                                        sc.oscs.len()
                                    ),
                                );
                                if let Some(order) = &sc.order {
                                    let order = order.clone();
                                    if theme::pill(ui, &order, theme::ACCENT_MUTED)
                                        .on_hover_text(
                                            "The route this was captured on — click to \
                                             make it active again",
                                        )
                                        .clicked()
                                    {
                                        do_order = Some(order);
                                    }
                                }
                            });
                        });
                        ui.add_space(4.0);
                    }
                });
            });
        self.show_scenes = open;

        if do_capture {
            self.capture_scene();
        }
        if do_chain {
            self.start_chain();
        }
        if do_stop_all {
            self.stop_all_scenes();
        }
        if let Some(name) = do_order {
            match self.orders.iter().position(|o| o.name == name) {
                Some(i) => {
                    self.active_order = Some(i);
                    self.log.push(format!("Effects follow {name}"));
                }
                None => self.log.push(format!("Order \"{name}\" no longer exists")),
            }
        }
        if let Some((i, action)) = do_row {
            match action {
                SceneAction::Toggle => {
                    if self.scenes[i].is_running() {
                        self.stop_scene(i);
                    } else {
                        self.start_scene(i);
                    }
                }
                SceneAction::Up if i > 0 => {
                    self.scenes.swap(i, i - 1);
                    dirty = true;
                }
                SceneAction::Down if i + 1 < self.scenes.len() => {
                    self.scenes.swap(i, i + 1);
                    dirty = true;
                }
                SceneAction::Remove => {
                    let sc = self.scenes.remove(i);
                    self.log.push(format!("Deleted scene \"{}\"", sc.name));
                    dirty = true;
                }
                SceneAction::Update => self.update_scene(i),
                _ => {}
            }
        }
        if dirty {
            scene::save_scenes(&self.scenes);
        }
    }
}
