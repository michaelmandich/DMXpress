//! Floating "Stacks" window: build and play cue lists. Record the programmer
//! into cues (tracking — only active channels are stored), then Go to fade
//! between them. A playing stack shows beneath the programmer in the mixer.

use eframe::egui;

use super::{apply_zoom, zoom_controls};
use crate::app::App;
use crate::net::Frame;
use crate::palette::{Feature, PaletteRef};
use crate::stack::{self, Cue, CueVal, Stack};

impl App {
    /// Resolve one channel of a palette reference to its current value.
    pub(crate) fn resolve_palette_channel(&self, r: PaletteRef, addr0: usize) -> Option<u8> {
        self.palettes
            .iter()
            .find(|p| p.id == r.id)
            .and_then(|p| p.values.iter().find(|(a, _)| *a == addr0).map(|(_, v)| *v))
    }

    /// The tracked output frame at cue `cue_idx`: every cue up to and including
    /// it applied in order, palette references resolved live.
    pub(crate) fn tracked_frame(&self, stack_idx: usize, cue_idx: usize) -> Frame {
        let mut f = Frame::black();
        let Some(st) = self.stacks.get(stack_idx) else {
            return f;
        };
        for c in st.cues.iter().take(cue_idx + 1) {
            for &(a, val) in &c.values {
                f[a] = match val {
                    CueVal::Absolute(x) => x,
                    CueVal::Palette { reference, value } => {
                        self.resolve_palette_channel(reference, a).unwrap_or(value)
                    }
                };
            }
        }
        f
    }

    /// Record the active programmer channels into a new cue on stack `stack_idx`.
    pub(crate) fn store_cue(&mut self, stack_idx: usize) {
        if self.live_active.is_empty() {
            self.log
                .push("Store cue: programmer is empty (set a look first)".into());
            return;
        }
        let mut active: Vec<usize> = self.live_active.iter().copied().collect();
        active.sort_unstable();
        let values: Vec<(usize, CueVal)> = active
            .iter()
            .filter(|&&a| match self.role_at(a) {
                Some(role) => self.record_mask.contains(&Feature::of(role)),
                None => true,
            })
            .map(|&a| {
                let v = self.live.base[a];
                let val = match self.live_refs.get(&a) {
                    Some(r) => CueVal::Palette {
                        reference: *r,
                        value: v,
                    },
                    None => CueVal::Absolute(v),
                };
                (a, val)
            })
            .collect();
        if values.is_empty() {
            self.log
                .push("Store cue: record mask filtered out every active channel".into());
            return;
        }
        let fade = self.cue_fade;
        let Some(st) = self.stacks.get_mut(stack_idx) else {
            return;
        };
        let number = st.cues.last().map(|c| c.number + 1.0).unwrap_or(1.0);
        let name = format!("Cue {}", st.cues.len() + 1);
        let n = values.len();
        st.cues.push(Cue {
            number,
            name,
            fade,
            values,
        });
        stack::save_stacks(&self.stacks);
        self.log
            .push(format!("Stored cue {number:.0} ({n} ch) in \"{}\"", self.stacks[stack_idx].name));
    }

    /// Fade stack `stack_idx` to cue `cue_idx`.
    pub(crate) fn fire_cue(&mut self, stack_idx: usize, cue_idx: usize) {
        let frame = self.tracked_frame(stack_idx, cue_idx);
        let Some(st) = self.stacks.get_mut(stack_idx) else {
            return;
        };
        if cue_idx >= st.cues.len() {
            return;
        }
        let fade = st.cues[cue_idx].fade;
        let label = format!("{} · {}", st.name, st.cues[cue_idx].name);
        st.fire(cue_idx, frame, fade);
        self.cur_stack = Some(stack_idx);
        self.log.push(format!("Go → {label}"));
    }

    /// Advance stack `stack_idx` to the next cue (wraps at the end).
    pub(crate) fn go_stack(&mut self, stack_idx: usize) {
        let len = self.stacks.get(stack_idx).map(|s| s.cues.len()).unwrap_or(0);
        if len == 0 {
            self.log.push("Stack has no cues yet".into());
            return;
        }
        let next = match self.stacks[stack_idx].current {
            None => 0,
            Some(c) => (c + 1) % len,
        };
        self.fire_cue(stack_idx, next);
    }

    pub(crate) fn new_stack(&mut self) {
        let name = format!("Stack {}", self.stacks.len() + 1);
        self.stacks.push(Stack::new(name));
        self.cur_stack = Some(self.stacks.len() - 1);
        stack::save_stacks(&self.stacks);
    }

    pub(crate) fn delete_stack(&mut self, idx: usize) {
        if idx >= self.stacks.len() {
            return;
        }
        let s = self.stacks.remove(idx);
        self.cur_stack = if self.stacks.is_empty() {
            None
        } else {
            Some(idx.min(self.stacks.len() - 1))
        };
        stack::save_stacks(&self.stacks);
        self.log.push(format!("Deleted stack \"{}\"", s.name));
    }

    pub(crate) fn delete_cue(&mut self, stack_idx: usize, cue_idx: usize) {
        let Some(st) = self.stacks.get_mut(stack_idx) else {
            return;
        };
        if cue_idx >= st.cues.len() {
            return;
        }
        st.cues.remove(cue_idx);
        if let Some(c) = st.current {
            if c >= st.cues.len() {
                st.current = None;
            }
        }
        stack::save_stacks(&self.stacks);
    }

    pub(crate) fn stacks_window(&mut self, ctx: &egui::Context) {
        if !self.show_stacks {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_stacks;
        let mut dirty = false;
        let mut do_new = false;
        let mut do_clear = false;
        let mut do_go: Option<usize> = None;
        let mut do_store: Option<usize> = None;
        let mut do_delete_stack: Option<usize> = None;
        let mut do_fire: Option<(usize, usize)> = None;
        let mut do_delete_cue: Option<(usize, usize)> = None;

        egui::Window::new("Stacks")
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .default_size([400.0, 380.0])
            .default_pos([screen.left() + 80.0, 120.0])
            .show(ctx, |ui| {
                zoom_controls(ui, &mut self.zoom.stacks);
                apply_zoom(ui, self.zoom.stacks);

                ui.horizontal_wrapped(|ui| {
                    for i in 0..self.stacks.len() {
                        let active = self.cur_stack == Some(i);
                        let playing = self.stacks[i].current.is_some();
                        let label = if playing {
                            format!("> {}", self.stacks[i].name)
                        } else {
                            self.stacks[i].name.clone()
                        };
                        if ui.selectable_label(active, label).clicked() {
                            self.cur_stack = Some(i);
                        }
                    }
                    if ui.button("New").clicked() {
                        do_new = true;
                    }
                });
                ui.separator();

                let Some(si) = self.cur_stack.filter(|&i| i < self.stacks.len()) else {
                    ui.weak("No stack selected. Press New to create a cue list.");
                    return;
                };

                ui.horizontal(|ui| {
                    dirty |= ui
                        .add(
                            egui::TextEdit::singleline(&mut self.stacks[si].name)
                                .desired_width(150.0),
                        )
                        .changed();
                    if ui.button("Delete list").clicked() {
                        do_delete_stack = Some(si);
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .button("Go")
                        .on_hover_text("Fade to the next cue")
                        .clicked()
                    {
                        do_go = Some(si);
                    }
                    ui.separator();
                    ui.label("Fade:");
                    ui.add(
                        egui::DragValue::new(&mut self.cue_fade)
                            .range(0.0..=60.0)
                            .speed(0.1)
                            .suffix(" s"),
                    );
                    if ui
                        .add_enabled(
                            !self.live_active.is_empty(),
                            egui::Button::new("Store cue"),
                        )
                        .on_hover_text("Record the programmer as a new cue")
                        .clicked()
                    {
                        do_store = Some(si);
                    }
                    if ui.button("Clear prog").clicked() {
                        do_clear = true;
                    }
                });
                ui.weak(format!("{} programmer ch active", self.live_active.len()));
                ui.horizontal_wrapped(|ui| {
                    ui.label("Record:")
                        .on_hover_text("Which features Store records into a cue");
                    for f in Feature::ALL {
                        let on = self.record_mask.contains(&f);
                        if ui
                            .selectable_label(on, f.short())
                            .on_hover_text(f.label())
                            .clicked()
                        {
                            if on {
                                self.record_mask.remove(&f);
                            } else {
                                self.record_mask.insert(f);
                            }
                        }
                    }
                });
                ui.separator();

                let cur = self.stacks[si].current;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.stacks[si].cues.is_empty() {
                        ui.weak("No cues yet — build a look, then press Store cue.");
                        return;
                    }
                    egui::Grid::new("cue_grid")
                        .striped(true)
                        .num_columns(5)
                        .show(ui, |ui| {
                            ui.strong("#");
                            ui.strong("Name");
                            ui.strong("Fade");
                            ui.label("");
                            ui.label("");
                            ui.end_row();
                            for ci in 0..self.stacks[si].cues.len() {
                                let is_cur = cur == Some(ci);
                                let num = self.stacks[si].cues[ci].number;
                                let num_txt = egui::RichText::new(format!("{num:.0}"));
                                ui.label(if is_cur {
                                    num_txt.strong().color(egui::Color32::from_rgb(120, 230, 140))
                                } else {
                                    num_txt
                                });
                                dirty |= ui
                                    .add(
                                        egui::TextEdit::singleline(
                                            &mut self.stacks[si].cues[ci].name,
                                        )
                                        .desired_width(130.0),
                                    )
                                    .changed();
                                dirty |= ui
                                    .add(
                                        egui::DragValue::new(&mut self.stacks[si].cues[ci].fade)
                                            .range(0.0..=60.0)
                                            .speed(0.1)
                                            .suffix(" s"),
                                    )
                                    .changed();
                                if ui.button("Go").on_hover_text("Go to this cue").clicked() {
                                    do_fire = Some((si, ci));
                                }
                                if ui.small_button("x").clicked() {
                                    do_delete_cue = Some((si, ci));
                                }
                                ui.end_row();
                            }
                        });
                });
            });
        self.show_stacks = open;

        if do_new {
            self.new_stack();
        }
        if let Some(i) = do_delete_stack {
            self.delete_stack(i);
        }
        if let Some(i) = do_go {
            self.go_stack(i);
        }
        if let Some(i) = do_store {
            self.store_cue(i);
        }
        if do_clear {
            self.clear_programmer();
        }
        if let Some((si, ci)) = do_fire {
            self.fire_cue(si, ci);
        }
        if let Some((si, ci)) = do_delete_cue {
            self.delete_cue(si, ci);
        }
        if dirty {
            stack::save_stacks(&self.stacks);
        }
    }
}
