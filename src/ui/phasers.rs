//! Floating "Phasers" pool window: build a spread effect, apply it across the
//! current selection, and store it for reuse. Phasers arm the live oscillator
//! engine, so the Oscillator window still shows/edits the per-channel result.

use eframe::egui;

use std::collections::HashSet;

use super::{apply_zoom, zoom_controls};
use crate::app::{App, Ramp};
use crate::group::GroupMode;
use crate::net;
use crate::oscillator::{subdiv_label, Osc, SPEED_CHOICES};
use crate::palette::Feature;
use crate::phaser::{self, spread_phase, ChannelFilter, ComponentMode, Phaser, PhaserMode};
use crate::showbuddy::{Band, Role};

impl App {
    /// Arm the phaser's oscillators across every selected fixture's channels of
    /// the chosen feature, fanning the phase along the selection. `quiet`
    /// suppresses logging (used by live-apply, which re-applies every edit).
    pub(crate) fn apply_phaser(&mut self, ph: Phaser, quiet: bool) {
        if !ph.hold.is_empty() {
            self.apply_hold_phaser(&ph);
            return;
        }
        if !ph.static_pos.is_empty() {
            self.apply_pose_phaser(&ph);
            return;
        }
        // Selection first: applying with fixtures selected targets them.
        // With nothing selected the phaser falls back to the fixtures it was
        // created with (recorded at store time, re-bindable via 🔗).
        let sel = self.stage.selected_fixtures();
        let fixtures: Vec<usize> = if !sel.is_empty() {
            sel
        } else {
            self.patch
                .fixtures
                .iter()
                .enumerate()
                .filter(|(_, f)| {
                    ph.fixtures
                        .contains(&crate::profiles::fixture_key(&f.display, f.from))
                })
                .map(|(i, _)| i)
                .collect()
        };
        if fixtures.is_empty() {
            if !quiet {
                self.log.push(if ph.fixtures.is_empty() {
                    "Phaser: select fixtures first (or store it with a selection)".into()
                } else {
                    format!("Phaser \"{}\": none of its fixtures are patched", ph.name)
                });
            }
            return;
        }
        // Build effect units. Selected groups marked "One fixture" become a
        // single phase slot; all their members receive that slot's phase.
        // Remaining fixtures are one slot each.
        let selected: HashSet<usize> = fixtures.iter().copied().collect();
        let mut claimed: HashSet<usize> = HashSet::new();
        let mut units: Vec<Vec<usize>> = Vec::new();
        for group in &self.groups {
            if group.mode == GroupMode::AsFixture
                && !group.fixtures.is_empty()
                && group.fixtures.iter().all(|fi| selected.contains(fi))
                && group.fixtures.iter().all(|fi| !claimed.contains(fi))
            {
                claimed.extend(group.fixtures.iter().copied());
                units.push(group.fixtures.clone());
            }
        }
        for &fi in &fixtures {
            if claimed.insert(fi) {
                units.push(vec![fi]);
            }
        }
        let mut fixture_phase = std::collections::HashMap::new();
        for (k, unit) in units.iter().enumerate() {
            let phase = spread_phase(k, units.len(), ph.spread, ph.wings as usize);
            for &fi in unit {
                fixture_phase.insert(fi, phase);
            }
        }
        let is_add = ph.components.is_empty() && ph.mode == PhaserMode::Add;
        let fade = self
            .phaser_transition
            .duration(self.transition.duration, self.phaser_fade_s);
        let now = std::time::Instant::now();
        let beat_clock = self.live.beat_clock();
        if !is_add {
            self.live.speed = ph.speed;
        }
        let mut count = 0;
        let mut armed: Vec<usize> = Vec::new();
        for &fi in &fixtures {
            let phase = fixture_phase.get(&fi).copied().unwrap_or(0.0);
            let Some(f) = self.patch.fixtures.get(fi) else {
                continue;
            };
            for (ci, ch) in f.channels.iter().enumerate() {
                let role = ch.role();
                let component = ph
                    .components
                    .iter()
                    .find(|component| component.matches(role, &ch.name));
                if if ph.components.is_empty() {
                    !ph.matches_channel(role, &ch.name)
                } else {
                    component.is_none()
                } {
                    continue;
                }
                let addr = f.from as usize + ci;
                if !(1..=net::DMX_SLOTS).contains(&addr) {
                    continue;
                }
                let addr0 = addr - 1;
                if let Some(component) = component {
                    if matches!(component.mode, ComponentMode::Static | ComponentMode::StaticThenOscillation) {
                        if fade > 0.01 {
                            self.base_fades.insert(
                                addr0,
                                Ramp {
                                    from: self.live.base[addr0] as f32,
                                    to: component.static_value as f32,
                                    start: now,
                                    dur: fade,
                                    stepped: self.channel_is_stepped(addr0),
                                    remove_after: false,
                                },
                            );
                        } else {
                            self.base_fades.remove(&addr0);
                            self.live.base[addr0] = component.static_value;
                        }
                        self.live_active.insert(addr0);
                        self.live_refs.remove(&addr0);
                    }
                    if component.mode == ComponentMode::Static {
                        self.live.oscs.remove(&addr0);
                        self.osc_ramps.remove(&addr0);
                        self.add_overrides.remove(&addr0);
                        self.add_ramps.remove(&addr0);
                        armed.push(addr0);
                        count += 1;
                        continue;
                    }
                }
                if is_add {
                    // Flat add: a constant offset summed onto the output
                    // every frame until the phaser is stopped.
                    let delta = (ph.amount * 255.0).round() as i16
                        * if ph.invert { -1 } else { 1 };
                    if fade > 0.01 {
                        let cur = *self.add_overrides.get(&addr0).unwrap_or(&0);
                        self.add_overrides.insert(addr0, cur);
                        self.add_ramps.insert(
                            addr0,
                            Ramp {
                                from: cur as f32,
                                to: delta as f32,
                                start: now,
                                dur: fade,
                                stepped: false,
                                remove_after: false,
                            },
                        );
                    } else {
                        self.add_ramps.remove(&addr0);
                        self.add_overrides.insert(addr0, delta);
                    }
                    // Rides on top of the final mix, so a wave (e.g. a
                    // barrel roll) keeps running underneath the offset.
                    self.hold_overrides.remove(&addr0);
                } else {
                    // A phaser on both axes offsets the tilts a quarter cycle
                    // so pan+tilt trace a circle instead of a diagonal.
                    let has = |s: &str| {
                        if ph.components.is_empty() {
                            ph.targets.iter().any(|t| t.eq_ignore_ascii_case(s))
                        } else {
                            ph.components.iter().any(|c| c.target.eq_ignore_ascii_case(s))
                        }
                    };
                    let both_axes = if ph.targets.is_empty() && ph.components.is_empty() {
                        ph.feature == Feature::Position
                            && ph.filter == ChannelFilter::All
                    } else {
                        (has("PAN") || has("PANf")) && (has("TILT") || has("TILTf"))
                    };
                    let extra = if both_axes
                        && matches!(role, Role::Tilt | Role::TiltFine)
                    {
                        0.25
                    } else {
                        0.0
                    };
                    let amount = component.map_or(ph.amount, |c| c.amount);
                    let shape = component.map_or(ph.shape, |c| c.shape);
                    let subdiv = component.map_or(ph.subdiv, |c| c.subdiv);
                    let invert = component.map_or(ph.invert, |c| c.invert);
                    let component_phase = component.map_or(0.0, |c| c.phase);
                    let custom_wave = component
                        .and_then(|c| c.waveform_id)
                        .and_then(|id| self.custom_waveforms.iter().find(|w| w.id == id))
                        .cloned();
                    // Fade the wave in from the channel's current depth so
                    // switching phasers swells instead of jumping.
                    let start_amt = if fade > 0.01 {
                        let cur = self.live.oscs.get(&addr0).map_or(0.0, |o| o.amount);
                        self.osc_ramps.insert(
                            addr0,
                            Ramp {
                                from: cur,
                                to: amount,
                                start: now,
                                dur: fade,
                                stepped: false,
                                remove_after: false,
                            },
                        );
                        cur
                    } else {
                        self.osc_ramps.remove(&addr0);
                        amount
                    };
                    self.live.oscs.insert(
                        addr0,
                        Osc {
                            enabled: true,
                            invert,
                            amount: start_amt,
                            phase: phase + extra + component_phase,
                            subdiv,
                            shape,
                            master_beat: ph.master_beat,
                            local_beats: beat_clock,
                            local_tempo: self.live.tempo,
                            custom_wave,
                        },
                    );
                    self.live_active.insert(addr0);
                    self.live_refs.remove(&addr0);
                    self.hold_overrides.remove(&addr0);
                }
                armed.push(addr0);
                count += 1;
            }
        }
        // This phaser now owns these channels *within its layer*: flat adds
        // ride on top of waves (both tiles keep running), so an add only
        // steals from other adds and holds, and a wave from other waves,
        // poses and holds.
        for (name, addrs) in self.active_phasers.iter_mut() {
            if *name == ph.name {
                continue;
            }
            let other = self.phasers.iter().find(|p| p.name == *name);
            let steal = match other {
                Some(o) if !o.hold.is_empty() => true,
                Some(o) if !o.static_pos.is_empty() => !is_add,
                Some(o) => (o.mode == PhaserMode::Add) == is_add,
                None => true,
            };
            if steal {
                addrs.retain(|a| !armed.contains(a));
            }
        }
        self.active_phasers.retain(|_, v| !v.is_empty());
        if !armed.is_empty() {
            self.active_phasers.insert(ph.name.clone(), armed);
        }
        if !quiet {
            self.log.push(format!(
                "Applied phaser \"{}\" to {} fixtures ({} ch{})",
                ph.name,
                fixtures.len(),
                count,
                if is_add { ", flat add" } else { "" }
            ));
        }
    }

    /// Recall a static pose or FX state: stop movement on the stored fixtures
    /// and drive the saved channels to their values (regardless of selection),
    /// fading over the Phasers window's fade time.
    pub(crate) fn apply_pose_phaser(&mut self, ph: &Phaser) {
        let fade = self
            .phaser_transition
            .duration(self.transition.duration, self.phaser_fade_s);
        let now = std::time::Instant::now();
        let mut armed: Vec<usize> = Vec::new();
        let mut nfix = 0;
        for (key, vals) in &ph.static_pos {
            let Some(f) = self
                .patch
                .fixtures
                .iter()
                .find(|f| crate::profiles::fixture_key(&f.display, f.from) == *key)
            else {
                continue;
            };
            nfix += 1;
            for &(ci, v) in vals {
                let addr = f.from as usize + ci;
                if !(1..=net::DMX_SLOTS).contains(&addr) {
                    continue;
                }
                let addr0 = addr - 1;
                self.live.oscs.remove(&addr0);
                self.osc_ramps.remove(&addr0);
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
                self.live_refs.remove(&addr0);
                self.hold_overrides.remove(&addr0);
                self.add_overrides.remove(&addr0);
                self.add_ramps.remove(&addr0);
                armed.push(addr0);
            }
        }
        if nfix == 0 {
            self.log.push(format!(
                "Pose \"{}\": none of its fixtures are patched",
                ph.name
            ));
            return;
        }
        for addrs in self.active_phasers.values_mut() {
            addrs.retain(|a| !armed.contains(a));
        }
        self.active_phasers.retain(|_, v| !v.is_empty());
        self.active_phasers.insert(ph.name.clone(), armed);
        self.log.push(if ph.feature == Feature::Position {
            format!(
                "Pose \"{}\": {} fixtures positioned, movement stopped",
                ph.name, nfix
            )
        } else {
            format!("FX \"{}\": {} fixtures set", ph.name, nfix)
        });
    }

    /// Activate a hold: force the stored channel values onto the output every
    /// frame — on top of presets, blackouts and the grand master — until the
    /// tile is clicked off.
    pub(crate) fn apply_hold_phaser(&mut self, ph: &Phaser) {
        let mut armed: Vec<usize> = Vec::new();
        let mut nfix = 0;
        for (key, vals) in &ph.hold {
            let Some(f) = self
                .patch
                .fixtures
                .iter()
                .find(|f| crate::profiles::fixture_key(&f.display, f.from) == *key)
            else {
                continue;
            };
            nfix += 1;
            for &(ci, v) in vals {
                let addr = f.from as usize + ci;
                if !(1..=net::DMX_SLOTS).contains(&addr) {
                    continue;
                }
                let addr0 = addr - 1;
                self.hold_overrides.insert(addr0, v);
                self.add_overrides.remove(&addr0);
                self.add_ramps.remove(&addr0);
                armed.push(addr0);
            }
        }
        if nfix == 0 {
            self.log.push(format!(
                "Hold \"{}\": none of its fixtures are patched",
                ph.name
            ));
            return;
        }
        for addrs in self.active_phasers.values_mut() {
            addrs.retain(|a| !armed.contains(a));
        }
        self.active_phasers.retain(|_, v| !v.is_empty());
        self.active_phasers.insert(ph.name.clone(), armed);
        self.log.push(format!(
            "Hold \"{}\" on: {} fixtures forced until stopped",
            ph.name, nfix
        ));
    }

    /// Stop a running phaser: fade its oscillators / flat adds out over the
    /// window's fade time (or drop them immediately at 0), settling channels
    /// back to their base values, which stay asserted.
    pub(crate) fn stop_phaser(&mut self, name: &str) {
        if let Some(addrs) = self.active_phasers.remove(name) {
            // Only tear down the layers this phaser drives, so stopping a
            // flat add doesn't kill a wave riding the same channels (and
            // vice versa). Unknown names (unsaved editor applies) clear both.
            let (stop_osc, stop_add) = match self.phasers.iter().find(|p| p.name == name) {
                Some(p) if !p.hold.is_empty() || !p.static_pos.is_empty() => (true, true),
                Some(p) => (p.mode != PhaserMode::Add, p.mode == PhaserMode::Add),
                None => (true, true),
            };
            let fade = self
                .phaser_transition
                .duration(self.transition.duration, self.phaser_fade_s);
            let now = std::time::Instant::now();
            for a in addrs {
                self.hold_overrides.remove(&a);
                if stop_osc {
                    if fade > 0.01 {
                        if let Some(o) = self.live.oscs.get(&a) {
                            self.osc_ramps.insert(
                                a,
                                Ramp {
                                    from: o.amount,
                                    to: 0.0,
                                    start: now,
                                    dur: fade,
                                    stepped: false,
                                    remove_after: true,
                                },
                            );
                        }
                    } else {
                        self.live.oscs.remove(&a);
                        self.osc_ramps.remove(&a);
                    }
                }
                if stop_add {
                    if fade > 0.01 {
                        if let Some(&v) = self.add_overrides.get(&a) {
                            self.add_ramps.insert(
                                a,
                                Ramp {
                                    from: v as f32,
                                    to: 0.0,
                                    start: now,
                                    dur: fade,
                                    stepped: false,
                                    remove_after: true,
                                },
                            );
                        }
                    } else {
                        self.add_overrides.remove(&a);
                        self.add_ramps.remove(&a);
                    }
                }
            }
            self.log.push(format!("Stopped phaser \"{name}\""));
        }
    }

    pub(crate) fn phasers_window(&mut self, ctx: &egui::Context) {
        if !self.show_phasers {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_phasers;
        let mut do_apply = false;
        let mut live_toggled_on = false;
        let mut do_clear = false;
        let mut do_store = false;
        let mut do_store_pose = false;
        let mut do_store_hold = false;
        let mut do_store_lock = false;
        let mut do_store_fx = false;
        let mut do_load_apply: Option<usize> = None;
        let mut do_stop: Option<usize> = None;
        let mut do_update: Option<usize> = None;
        let mut do_delete: Option<usize> = None;
        let mut do_edit: Option<usize> = None;
        let mut do_bind: Option<usize> = None;
        let mut do_unbind: Option<usize> = None;
        let mut do_master_beat: Option<usize> = None;
        let edit_before = self.phaser_edit.clone();
        let name_before = self.phaser_name.clone();

        egui::Window::new("🌈 Phasers")
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .default_size([680.0, 760.0])
            .min_size([560.0, 560.0])
            .default_pos([screen.right() - 720.0, 90.0])
            .show(ctx, |ui| {
                zoom_controls(ui, &mut self.zoom.phasers);
                apply_zoom(ui, self.zoom.phasers);

                let sel = self.stage.selected_fixtures();
                // Channel-type chips: the same grouping as the collective
                // channel list — built from the selected fixtures (or the
                // whole patch when nothing is selected).
                let chip_src: Vec<usize> = if sel.is_empty() {
                    (0..self.patch.fixtures.len()).collect()
                } else {
                    sel.clone()
                };
                let mut chips: Vec<(String, String)> = Vec::new(); // (target, label)
                let mut seen: HashSet<String> = HashSet::new();
                for fi in chip_src {
                    let Some(f) = self.patch.fixtures.get(fi) else {
                        continue;
                    };
                    for ch in &f.channels {
                        let tag = ch.role().tag();
                        let key = if tag.is_empty() {
                            ch.name.clone()
                        } else {
                            tag.to_string()
                        };
                        if seen.insert(key.to_ascii_lowercase()) {
                            chips.push((key, ch.name.clone()));
                        }
                    }
                }
                // Keep stored/component targets the current rig doesn't have.
                for t in &self.phaser_edit.targets {
                    if !chips.iter().any(|(k, _)| k.eq_ignore_ascii_case(t)) {
                        chips.push((t.clone(), t.clone()));
                    }
                }
                for component in &self.phaser_edit.components {
                    if !chips.iter().any(|(key, _)| key.eq_ignore_ascii_case(&component.target)) {
                        chips.push((component.target.clone(), component.target.clone()));
                    }
                }
                const COMMON: [&str; 10] =
                    ["DIM", "PAN", "TILT", "RED", "GRN", "BLU", "WHT", "COL", "STRB", "ZOOM"];
                let mut easy: Vec<(String, String)> = Vec::new();
                for key in COMMON {
                    if let Some(chip) = chips.iter().find(|(candidate, _)| candidate.eq_ignore_ascii_case(key)) {
                        easy.push(chip.clone());
                    }
                }
                let special: Vec<(String, String)> = chips
                    .iter()
                    .filter(|(key, _)| !easy.iter().any(|(easy_key, _)| easy_key.eq_ignore_ascii_case(key)))
                    .cloned()
                    .collect();
                let waveforms = &self.custom_waveforms;
                let e = &mut self.phaser_edit;
                let tint = egui::Color32::from_rgb(
                    22 + e.color[0] / 10,
                    22 + e.color[1] / 10,
                    27 + e.color[2] / 10,
                );

                egui::Frame::none()
                    .fill(tint)
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgb(e.color[0] / 2, e.color[1] / 2, e.color[2] / 2),
                    ))
                    .rounding(9.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.heading("Build this phaser");
                            ui.label("ⓘ").on_hover_text(
                                "Add several channel components to one phaser. Each can be \
                                 static, animated, or move to a static value before animating.",
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.color_edit_button_srgb(&mut e.color)
                                    .on_hover_text("Phaser card colour");
                            });
                        });
                        ui.label(egui::RichText::new("Common channels").strong().color(egui::Color32::from_gray(220)));
                        ui.horizontal_wrapped(|ui| {
                            for (key, label) in &easy {
                                let on = e.components.iter().any(|c| c.target.eq_ignore_ascii_case(key))
                                    || (e.components.is_empty() && e.targets.iter().any(|t| t.eq_ignore_ascii_case(key)));
                                let text = if on { format!("✓ {label}") } else { format!("＋ {label}") };
                                if ui.add_sized([92.0, 30.0], egui::Button::new(text).selected(on)).clicked() {
                                    if on {
                                        e.components.retain(|c| !c.target.eq_ignore_ascii_case(key));
                                        e.targets.retain(|t| !t.eq_ignore_ascii_case(key));
                                    } else {
                                        if e.components.is_empty() {
                                            for target in e.targets.drain(..) {
                                                let mut migrated = crate::phaser::PhaserComponent::for_target(target);
                                                migrated.amount = e.amount;
                                                migrated.shape = e.shape;
                                                migrated.subdiv = e.subdiv;
                                                migrated.invert = e.invert;
                                                e.components.push(migrated);
                                            }
                                        }
                                        e.components.push(crate::phaser::PhaserComponent::for_target(key.clone()));
                                    }
                                }
                            }
                            egui::ComboBox::from_id_salt("phaser_add_special")
                                .selected_text("＋ Add special…")
                                .show_ui(ui, |ui| {
                                    for (key, label) in &special {
                                        let on = e.components.iter().any(|c| c.target.eq_ignore_ascii_case(key));
                                        if ui.selectable_label(on, label).clicked() {
                                            if on {
                                                e.components.retain(|c| !c.target.eq_ignore_ascii_case(key));
                                            } else {
                                                if e.components.is_empty() {
                                                    for target in e.targets.drain(..) {
                                                        let mut migrated = crate::phaser::PhaserComponent::for_target(target);
                                                        migrated.amount = e.amount;
                                                        migrated.shape = e.shape;
                                                        migrated.subdiv = e.subdiv;
                                                        migrated.invert = e.invert;
                                                        e.components.push(migrated);
                                                    }
                                                }
                                                e.components.push(crate::phaser::PhaserComponent::for_target(key.clone()));
                                            }
                                        }
                                    }
                                })
                                .response
                                .on_hover_text("Every less-common channel found on the selected fixtures");
                            ui.label("ⓘ").on_hover_text(
                                "Add special exposes gobo, prism, fine, macro and fixture-specific \
                                 channels. Add as many components as this phaser needs.",
                            );
                        });
                    });

                ui.add_space(8.0);
                let mut remove_component = None;
                for (index, component) in e.components.iter_mut().enumerate() {
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgb(28, 30, 39))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(62, 67, 86)))
                        .rounding(8.0)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{}  {}", index + 1, component.target))
                                        .size(15.0)
                                        .strong()
                                        .color(egui::Color32::from_rgb(220, 225, 240)),
                                );
                                egui::ComboBox::from_id_salt(("component_mode", index))
                                    .selected_text(component.mode.label())
                                    .show_ui(ui, |ui| {
                                        for mode in ComponentMode::ALL {
                                            ui.selectable_value(&mut component.mode, mode, mode.label());
                                        }
                                    });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("✕").on_hover_text("Remove component").clicked() {
                                        remove_component = Some(index);
                                    }
                                });
                            });
                            if matches!(component.mode, ComponentMode::Static | ComponentMode::StaticThenOscillation) {
                                ui.horizontal(|ui| {
                                    ui.label("Static value");
                                    ui.add(egui::Slider::new(&mut component.static_value, 0..=255));
                                    ui.weak("The channel moves here first");
                                });
                            }
                            if component.mode != ComponentMode::Static {
                                egui::Grid::new(("component_grid", index))
                                    .num_columns(4)
                                    .spacing([10.0, 5.0])
                                    .show(ui, |ui| {
                                        ui.label("Amount");
                                        ui.add(egui::Slider::new(&mut component.amount, 0.0..=1.0));
                                        ui.label("Rate");
                                        egui::ComboBox::from_id_salt(("component_rate", index))
                                            .selected_text(subdiv_label(component.subdiv))
                                            .show_ui(ui, |ui| {
                                                for (name, value) in SPEED_CHOICES {
                                                    ui.selectable_value(&mut component.subdiv, value, name);
                                                }
                                            });
                                        ui.end_row();
                                        ui.label("Waveform");
                                        let selected_wave = component
                                            .waveform_id
                                            .and_then(|id| waveforms.iter().find(|wave| wave.id == id))
                                            .map_or("Built-in", |wave| wave.name.as_str());
                                        egui::ComboBox::from_id_salt(("component_wave", index))
                                            .selected_text(selected_wave)
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(&mut component.waveform_id, None, "Built-in");
                                                for wave in waveforms {
                                                    ui.selectable_value(
                                                        &mut component.waveform_id,
                                                        Some(wave.id),
                                                        &wave.name,
                                                    );
                                                }
                                            });
                                        if component.waveform_id.is_none() {
                                            ui.label("Shape");
                                            ui.add(egui::Slider::new(&mut component.shape, 0.0..=1.0))
                                                .on_hover_text("Triangle → sine → square");
                                        } else {
                                            ui.label("Custom");
                                            ui.weak("Edited in Oscillator");
                                        }
                                        ui.end_row();
                                        ui.label("Phase");
                                        ui.add(egui::Slider::new(&mut component.phase, 0.0..=1.0));
                                        ui.checkbox(&mut component.invert, "Invert");
                                        ui.end_row();
                                    });
                            }
                        });
                    ui.add_space(5.0);
                }
                if let Some(index) = remove_component {
                    e.components.remove(index);
                }

                // Shared motion controls fan all animated components across fixtures.
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(23, 25, 32))
                    .rounding(7.0)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(&mut e.master_beat, "♪ Master beat");
                            ui.label("Spread");
                            ui.add_sized([120.0, 20.0], egui::Slider::new(&mut e.spread, 0.0..=2.0));
                            ui.label("Wings");
                            ui.add_sized([90.0, 20.0], egui::Slider::new(&mut e.wings, 1..=8));
                            if e.components.is_empty() {
                                ui.separator();
                                ui.label("Legacy mode");
                                ui.selectable_value(&mut e.mode, PhaserMode::Wave, "Wave");
                                ui.selectable_value(&mut e.mode, PhaserMode::Add, "Flat add");
                            }
                        });
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    let can_apply =
                        !sel.is_empty() || !self.phaser_edit.fixtures.is_empty();
                    if ui
                        .add_enabled(can_apply, egui::Button::new("▶ Apply"))
                        .on_hover_text(
                            "Apply to the selection — or, with nothing selected, \
                             to the fixtures the phaser was stored with",
                        )
                        .clicked()
                    {
                        do_apply = true;
                    }
                    if ui
                        .checkbox(&mut self.phaser_live, "Live")
                        .on_hover_text(
                            "Re-apply the phaser to the selection on every edit — \
                             as if Apply were pressed each time you change something",
                        )
                        .changed()
                        && self.phaser_live
                    {
                        live_toggled_on = true;
                    }
                    if ui
                        .button("Clear FX")
                        .on_hover_text("Remove all oscillators (revert to base values)")
                        .clicked()
                    {
                        do_clear = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Fade");
                    if ui
                        .small_button(self.phaser_transition.short_label())
                        .on_hover_text(
                            "Transition binding: M = master transition, C = custom \
                             duration, — = none. Click to cycle.",
                        )
                        .clicked()
                    {
                        self.phaser_transition = self.phaser_transition.next();
                    }
                    ui.add(
                        egui::Slider::new(&mut self.phaser_fade_s, 0.0..=10.0)
                            .suffix(" s")
                            .max_decimals(1),
                    )
                    .on_hover_text(
                        "Custom phaser transition duration. It is used when the \
                         square says C; M follows the master Transition window.",
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.phaser_name)
                            .hint_text("name…")
                            .desired_width(120.0),
                    );
                    if ui.button("＋ Store").clicked() {
                        do_store = true;
                    }
                    if ui
                        .add_enabled(!sel.is_empty(), egui::Button::new("📌 Store pose"))
                        .on_hover_text(
                            "Save each selected light's current pan/tilt as a static \
                             position tile — clicking it stops movement and recalls the pose",
                        )
                        .clicked()
                    {
                        do_store_pose = true;
                    }
                    if ui
                        .add_enabled(!sel.is_empty(), egui::Button::new("🔒 Store hold"))
                        .on_hover_text(
                            "Save the selected fixtures' non-zero channel values as an \
                             always-on tile — it overrides presets, blackouts and the \
                             grand master until clicked off (e.g. keep the smoker running)",
                        )
                        .clicked()
                    {
                        do_store_hold = true;
                    }
                    if ui
                        .add_enabled(!sel.is_empty(), egui::Button::new("🔐 Lock fixtures"))
                        .on_hover_text(
                            "Freeze the selected fixtures exactly as they look right \
                             now — every channel forced (zeros included) until the \
                             tile is clicked off",
                        )
                        .clicked()
                    {
                        do_store_lock = true;
                    }
                });

                // Gobo / prism / effect wheels: drive the selection's slot
                // channels directly and store combos as FX tiles.
                egui::CollapsingHeader::new("🎭 Gobos & effects")
                    .default_open(false)
                    .show(ui, |ui| {
                        if sel.is_empty() {
                            ui.weak(
                                "Select fixtures (e.g. the Mavericks) to drive \
                                 their gobo, prism and effect channels.",
                            );
                            return;
                        }
                        struct FxGroup {
                            label: String,
                            addrs: Vec<usize>,
                            bands: Vec<Band>,
                        }
                        let mut groups: Vec<FxGroup> = Vec::new();
                        for &fi in &sel {
                            let Some(f) = self.patch.fixtures.get(fi) else {
                                continue;
                            };
                            for (ci, ch) in f.channels.iter().enumerate() {
                                if !is_fx_channel(&ch.name) {
                                    continue;
                                }
                                let addr = f.from as usize + ci;
                                if !(1..=net::DMX_SLOTS).contains(&addr) {
                                    continue;
                                }
                                match groups
                                    .iter_mut()
                                    .find(|g| g.label.eq_ignore_ascii_case(&ch.name))
                                {
                                    Some(g) => g.addrs.push(addr - 1),
                                    None => groups.push(FxGroup {
                                        label: ch.name.clone(),
                                        addrs: vec![addr - 1],
                                        bands: ch.bands.clone(),
                                    }),
                                }
                            }
                        }
                        if groups.is_empty() {
                            ui.weak("The selection has no gobo/prism/effect channels.");
                            return;
                        }
                        let mut set_vals: Vec<(Vec<usize>, u8)> = Vec::new();
                        for g in &groups {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    egui::RichText::new(&g.label).small().strong(),
                                );
                                let cur = self.live.base[g.addrs[0]];
                                let mut v = cur;
                                if ui
                                    .add(egui::DragValue::new(&mut v).range(0..=255))
                                    .on_hover_text("Fine value (drag)")
                                    .changed()
                                {
                                    set_vals.push((g.addrs.clone(), v));
                                }
                                for b in &g.bands {
                                    let on = (b.min..=b.max).contains(&cur);
                                    let name = if b.label.is_empty() {
                                        format!("{}–{}", b.min, b.max)
                                    } else {
                                        b.label.clone()
                                    };
                                    if ui
                                        .selectable_label(
                                            on,
                                            egui::RichText::new(name).small(),
                                        )
                                        .clicked()
                                    {
                                        let mid =
                                            ((b.min as u16 + b.max as u16) / 2) as u8;
                                        set_vals.push((g.addrs.clone(), mid));
                                    }
                                }
                            });
                        }
                        for (addrs, v) in set_vals {
                            for a in addrs {
                                self.base_fades.remove(&a);
                                self.live.base[a] = v;
                                self.live_active.insert(a);
                                self.live_refs.remove(&a);
                            }
                        }
                        if ui
                            .button("＋ Store FX tile")
                            .on_hover_text(
                                "Save the selection's gobo/prism/effect values as a \
                                 tile (named from the field above) — e.g. one per \
                                 gobo, or a gobo + prism spin combo",
                            )
                            .clicked()
                        {
                            do_store_fx = true;
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.phaser_edit_mode, "✏ Edit mode")
                        .on_hover_text(
                            "Click tiles to load them into the editor and tweak them \
                             in place — without applying or stopping anything",
                        )
                        .clicked()
                    {
                        self.phaser_edit_mode = !self.phaser_edit_mode;
                        if !self.phaser_edit_mode {
                            self.phaser_edit_sel = None;
                        }
                    }
                    if let Some(i) = self.phaser_edit_sel {
                        if let Some(ph) = self.phasers.get(i) {
                            ui.colored_label(
                                egui::Color32::from_rgb(250, 210, 90),
                                format!("editing \"{}\"", ph.name),
                            );
                        }
                    }
                });
                ui.weak(if self.phaser_edit_mode {
                    "Click a tile to edit it · changes save instantly · right-click for more:"
                } else {
                    "Click to start/stop · right-click to edit:"
                });

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.columns(2, |cols| {
                        cols[0].strong("🔆 Intensity & color");
                        cols[1].strong("✛ Movement");
                        let (mut left, mut right) = (Vec::new(), Vec::new());
                        for (i, ph) in self.phasers.iter().enumerate() {
                            if ph.is_movement() {
                                right.push(i);
                            } else {
                                left.push(i);
                            }
                        }
                        for (c, list) in [(0usize, left), (1, right)] {
                            cols[c].horizontal_wrapped(|ui| {
                                for i in list {
                                    let ph = &self.phasers[i];
                                    let on = self.active_phasers.contains_key(&ph.name);
                                    let [r, g, b] = ph.color;
                                    let fill = if on {
                                        egui::Color32::from_rgb(r, g, b)
                                    } else {
                                        egui::Color32::from_rgb(
                                            r / 4 + 10,
                                            g / 4 + 10,
                                            b / 4 + 10,
                                        )
                                    };
                                    let lum = 0.299 * r as f32
                                        + 0.587 * g as f32
                                        + 0.114 * b as f32;
                                    let txt = if on && lum > 145.0 {
                                        egui::Color32::BLACK
                                    } else if on {
                                        egui::Color32::WHITE
                                    } else {
                                        egui::Color32::from_gray(150)
                                    };
                                    let sub = if !ph.hold.is_empty() {
                                        "Hold".to_string()
                                    } else if !ph.static_pos.is_empty() {
                                        if ph.feature == Feature::Position {
                                            "Pose".to_string()
                                        } else {
                                            "FX".to_string()
                                        }
                                    } else {
                                        let mut s = if !ph.components.is_empty() {
                                            ph.components
                                                .iter()
                                                .map(|component| component.target.as_str())
                                                .collect::<Vec<_>>()
                                                .join("+")
                                        } else if !ph.targets.is_empty() {
                                            ph.targets.join("+")
                                        } else if ph.filter == ChannelFilter::All {
                                            ph.feature.label().to_string()
                                        } else {
                                            ph.filter.label().to_string()
                                        };
                                        if s.len() > 18 {
                                            s.truncate(17);
                                            s.push('…');
                                        }
                                        if ph.mode == PhaserMode::Add {
                                            format!("＋{s}")
                                        } else {
                                            s
                                        }
                                    };
                                    let bound = if ph.fixtures.is_empty() { "" } else { "🔗" };
                                    let beat = if ph.master_beat { "♪" } else { "" };
                                    let label = egui::RichText::new(format!(
                                        "{}\n{}{}{}{}",
                                        ph.name,
                                        if on { "▶ " } else { "" },
                                        bound,
                                        beat,
                                        sub
                                    ))
                                    .size(11.5)
                                    .strong()
                                    .color(txt);
                                    let editing = self.phaser_edit_sel == Some(i);
                                    let btn = egui::Button::new(label)
                                        .fill(fill)
                                        .rounding(7.0)
                                        .stroke(if editing {
                                            egui::Stroke::new(
                                                2.0,
                                                egui::Color32::from_rgb(250, 210, 90),
                                            )
                                        } else if on {
                                            egui::Stroke::new(1.5, egui::Color32::WHITE)
                                        } else {
                                            egui::Stroke::new(
                                                1.0,
                                                egui::Color32::from_gray(70),
                                            )
                                        });
                                    let resp = ui.add_sized([106.0, 64.0], btn);
                                    if resp.clicked() {
                                        if self.phaser_edit_mode {
                                            do_edit = Some(i);
                                        } else if on {
                                            do_stop = Some(i);
                                        } else {
                                            do_load_apply = Some(i);
                                        }
                                    }
                                    resp.context_menu(|ui| {
                                        if ui
                                            .button("✏ Edit")
                                            .on_hover_text(
                                                "Load into the editor and tweak in place — \
                                                 without applying or stopping it",
                                            )
                                            .clicked()
                                        {
                                            do_edit = Some(i);
                                            ui.close_menu();
                                        }
                                        if ui.button("Apply").clicked() {
                                            do_load_apply = Some(i);
                                            ui.close_menu();
                                        }
                                        if on && ui.button("Stop").clicked() {
                                            do_stop = Some(i);
                                            ui.close_menu();
                                        }
                                        let mut follow = ph.master_beat;
                                        if ui
                                            .checkbox(&mut follow, "Follow master beat")
                                            .on_hover_text(
                                                "Opt this card in or out of taps and Master BPM",
                                            )
                                            .changed()
                                        {
                                            do_master_beat = Some(i);
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                        if ui
                                            .add_enabled(
                                                !sel.is_empty(),
                                                egui::Button::new("🔗 Bind to selection"),
                                            )
                                            .on_hover_text(
                                                "Remember the selected fixtures: with nothing \
                                                 selected, the tile applies to them (a live \
                                                 selection still wins)",
                                            )
                                            .clicked()
                                        {
                                            do_bind = Some(i);
                                            ui.close_menu();
                                        }
                                        if !ph.fixtures.is_empty()
                                            && ui.button("Unbind (use selection)").clicked()
                                        {
                                            do_unbind = Some(i);
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                        if ui
                                            .button("Overwrite with editor")
                                            .on_hover_text(
                                                "Replace this phaser's settings with the \
                                                 editor's current values",
                                            )
                                            .clicked()
                                        {
                                            do_update = Some(i);
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                        if ui.button("Delete").clicked() {
                                            do_delete = Some(i);
                                            ui.close_menu();
                                        }
                                    });
                                }
                            });
                        }
                    });
                });
            });
        self.show_phasers = open;

        // Edit mode: write editor changes straight back into the selected
        // pool tile (and rename it from the name field), saving as we go. If
        // it happens to be running, re-apply quietly so the tweak is heard.
        if let Some(i) = self.phaser_edit_sel {
            if i >= self.phasers.len() {
                self.phaser_edit_sel = None;
            } else if self.phaser_edit != edit_before || self.phaser_name != name_before {
                let old_name = self.phasers[i].name.clone();
                let was_active = self.active_phasers.contains_key(&old_name);
                // Flipping a running tile between wave and flat add moves it
                // to the other layer — stop the old incarnation first so its
                // oscillators or offset don't linger.
                if was_active
                    && self.phasers[i].mode != self.phaser_edit.mode
                    && self.phasers[i].static_pos.is_empty()
                    && self.phasers[i].hold.is_empty()
                {
                    self.stop_phaser(&old_name);
                }
                let mut ph = self.phaser_edit.clone();
                ph.name = if self.phaser_name.trim().is_empty() {
                    old_name.clone()
                } else {
                    self.phaser_name.trim().to_string()
                };
                self.phasers[i] = ph.clone();
                if ph.name != old_name {
                    if let Some(addrs) = self.active_phasers.remove(&old_name) {
                        self.active_phasers.insert(ph.name.clone(), addrs);
                    }
                }
                phaser::save_phasers(&self.phasers);
                if was_active && ph.static_pos.is_empty() && ph.hold.is_empty() {
                    self.apply_phaser(ph, true);
                }
            }
        }

        // Live-apply: any edit (or switching live on) re-applies the editor's
        // phaser to the selection, quietly to avoid log spam. Pose/hold tiles
        // only act when explicitly applied, and edit mode never auto-applies.
        let live_apply = self.phaser_live
            && self.phaser_edit_sel.is_none()
            && self.phaser_edit.static_pos.is_empty()
            && self.phaser_edit.hold.is_empty()
            && (live_toggled_on || self.phaser_edit != edit_before)
            && !self.stage.selected_fixtures().is_empty();

        if do_apply || live_apply {
            let ph = self.phaser_edit.clone();
            self.apply_phaser(ph, !do_apply);
        }
        if do_clear {
            self.live.oscs.clear();
            self.active_phasers.clear();
            self.hold_overrides.clear();
            self.add_overrides.clear();
            self.osc_ramps.clear();
            self.add_ramps.clear();
            self.log.push("Cleared effects (reverted to base)".into());
        }
        if do_store {
            let mut ph = self.phaser_edit.clone();
            // A plain Store always makes a live wave/add tile — scrub any
            // pose or hold snapshot the editor picked up from another tile.
            ph.static_pos = Vec::new();
            ph.hold = Vec::new();
            // Remember the fixtures it was built with: applying later with
            // nothing selected targets these.
            let keys: Vec<String> = self
                .stage
                .selected_fixtures()
                .iter()
                .filter_map(|&fi| self.patch.fixtures.get(fi))
                .map(|f| crate::profiles::fixture_key(&f.display, f.from))
                .collect();
            if !keys.is_empty() {
                ph.fixtures = keys;
            }
            ph.name = if self.phaser_name.trim().is_empty() {
                format!("Phaser {}", self.phasers.len() + 1)
            } else {
                self.phaser_name.trim().to_string()
            };
            self.log.push(format!("Stored phaser \"{}\"", ph.name));
            self.phasers.push(ph);
            phaser::save_phasers(&self.phasers);
            self.phaser_name.clear();
        }
        if do_store_pose {
            let sel = self.stage.selected_fixtures();
            let mut static_pos: Vec<(String, Vec<(usize, u8)>)> = Vec::new();
            for fi in sel {
                let Some(f) = self.patch.fixtures.get(fi) else { continue };
                let mut vals = Vec::new();
                for (ci, ch) in f.channels.iter().enumerate() {
                    if matches!(
                        ch.role(),
                        Role::Pan | Role::PanFine | Role::Tilt | Role::TiltFine
                    ) {
                        let addr = f.from as usize + ci;
                        if (1..=net::DMX_SLOTS).contains(&addr) {
                            vals.push((ci, self.live.base[addr - 1]));
                        }
                    }
                }
                if !vals.is_empty() {
                    static_pos
                        .push((crate::profiles::fixture_key(&f.display, f.from), vals));
                }
            }
            if static_pos.is_empty() {
                self.log
                    .push("Pose: select fixtures with pan/tilt first".into());
            } else {
                let mut ph = self.phaser_edit.clone();
                ph.feature = Feature::Position;
                ph.filter = ChannelFilter::All;
                ph.targets = Vec::new();
                ph.hold = Vec::new();
                ph.static_pos = static_pos;
                ph.name = if self.phaser_name.trim().is_empty() {
                    format!("Pose {}", self.phasers.len() + 1)
                } else {
                    self.phaser_name.trim().to_string()
                };
                self.log.push(format!(
                    "Stored pose \"{}\" ({} fixtures)",
                    ph.name,
                    ph.static_pos.len()
                ));
                self.phasers.push(ph);
                phaser::save_phasers(&self.phasers);
                self.phaser_name.clear();
            }
        }
        if do_store_hold {
            let sel = self.stage.selected_fixtures();
            let mut hold: Vec<(String, Vec<(usize, u8)>)> = Vec::new();
            for fi in sel {
                let Some(f) = self.patch.fixtures.get(fi) else { continue };
                let mut vals = Vec::new();
                for ci in 0..f.channels.len() {
                    let addr = f.from as usize + ci;
                    if (1..=net::DMX_SLOTS).contains(&addr) {
                        let v = self.live.base[addr - 1];
                        if v != 0 {
                            vals.push((ci, v));
                        }
                    }
                }
                if !vals.is_empty() {
                    hold.push((crate::profiles::fixture_key(&f.display, f.from), vals));
                }
            }
            if hold.is_empty() {
                self.log.push(
                    "Hold: set some non-zero levels on the selected fixtures first"
                        .into(),
                );
            } else {
                let mut ph = self.phaser_edit.clone();
                ph.static_pos = Vec::new();
                ph.targets = Vec::new();
                ph.hold = hold;
                ph.name = if self.phaser_name.trim().is_empty() {
                    format!("Hold {}", self.phasers.len() + 1)
                } else {
                    self.phaser_name.trim().to_string()
                };
                self.log.push(format!(
                    "Stored hold \"{}\" ({} fixtures)",
                    ph.name,
                    ph.hold.len()
                ));
                self.phasers.push(ph);
                phaser::save_phasers(&self.phasers);
                self.phaser_name.clear();
            }
        }
        if do_store_lock {
            let sel = self.stage.selected_fixtures();
            let out = *self.net.dmx.lock();
            let mut hold: Vec<(String, Vec<(usize, u8)>)> = Vec::new();
            for fi in sel {
                let Some(f) = self.patch.fixtures.get(fi) else { continue };
                let mut vals = Vec::new();
                for ci in 0..f.channels.len() {
                    let addr = f.from as usize + ci;
                    if (1..=net::DMX_SLOTS).contains(&addr) {
                        vals.push((ci, out[addr - 1]));
                    }
                }
                if !vals.is_empty() {
                    hold.push((crate::profiles::fixture_key(&f.display, f.from), vals));
                }
            }
            if hold.is_empty() {
                self.log.push("Lock: select fixtures first".into());
            } else {
                let mut ph = self.phaser_edit.clone();
                ph.static_pos = Vec::new();
                ph.targets = Vec::new();
                ph.hold = hold;
                ph.name = if self.phaser_name.trim().is_empty() {
                    format!("Lock {}", self.phasers.len() + 1)
                } else {
                    self.phaser_name.trim().to_string()
                };
                self.log.push(format!(
                    "Locked \"{}\" — {} fixtures frozen as they look now (every channel)",
                    ph.name,
                    ph.hold.len()
                ));
                self.phasers.push(ph);
                phaser::save_phasers(&self.phasers);
                self.phaser_name.clear();
            }
        }
        if do_store_fx {
            let sel = self.stage.selected_fixtures();
            let mut pose: Vec<(String, Vec<(usize, u8)>)> = Vec::new();
            for fi in sel {
                let Some(f) = self.patch.fixtures.get(fi) else { continue };
                let mut vals = Vec::new();
                for (ci, ch) in f.channels.iter().enumerate() {
                    if !is_fx_channel(&ch.name) {
                        continue;
                    }
                    let addr = f.from as usize + ci;
                    if (1..=net::DMX_SLOTS).contains(&addr) {
                        vals.push((ci, self.live.base[addr - 1]));
                    }
                }
                if !vals.is_empty() {
                    pose.push((crate::profiles::fixture_key(&f.display, f.from), vals));
                }
            }
            if pose.is_empty() {
                self.log
                    .push("FX: select fixtures with gobo/prism/effect channels first".into());
            } else {
                let mut ph = self.phaser_edit.clone();
                ph.feature = Feature::Beam;
                ph.filter = ChannelFilter::All;
                ph.targets = Vec::new();
                ph.hold = Vec::new();
                ph.static_pos = pose;
                ph.name = if self.phaser_name.trim().is_empty() {
                    format!("FX {}", self.phasers.len() + 1)
                } else {
                    self.phaser_name.trim().to_string()
                };
                self.log.push(format!(
                    "Stored FX \"{}\" ({} fixtures)",
                    ph.name,
                    ph.static_pos.len()
                ));
                self.phasers.push(ph);
                phaser::save_phasers(&self.phasers);
                self.phaser_name.clear();
            }
        }
        if let Some(i) = do_edit {
            if let Some(ph) = self.phasers.get(i).cloned() {
                self.phaser_name = ph.name.clone();
                self.phaser_edit = ph;
                self.phaser_edit_mode = true;
                self.phaser_edit_sel = Some(i);
            }
        }
        if let Some(i) = do_bind {
            let keys: Vec<String> = self
                .stage
                .selected_fixtures()
                .iter()
                .filter_map(|&fi| self.patch.fixtures.get(fi))
                .map(|f| crate::profiles::fixture_key(&f.display, f.from))
                .collect();
            if let Some(ph) = self.phasers.get_mut(i) {
                let n = keys.len();
                ph.fixtures = keys;
                let name = ph.name.clone();
                phaser::save_phasers(&self.phasers);
                if self.phaser_edit_sel == Some(i) {
                    self.phaser_edit.fixtures = self.phasers[i].fixtures.clone();
                }
                self.log
                    .push(format!("Bound \"{name}\" to {n} fixtures"));
            }
        }
        if let Some(i) = do_unbind {
            if let Some(ph) = self.phasers.get_mut(i) {
                ph.fixtures.clear();
                let name = ph.name.clone();
                phaser::save_phasers(&self.phasers);
                if self.phaser_edit_sel == Some(i) {
                    self.phaser_edit.fixtures.clear();
                }
                self.log
                    .push(format!("\"{name}\" now applies to the selection"));
            }
        }
        if let Some(i) = do_master_beat {
            if let Some(ph) = self.phasers.get_mut(i) {
                ph.master_beat = !ph.master_beat;
                let name = ph.name.clone();
                let follows = ph.master_beat;
                if self.phaser_edit_sel == Some(i) {
                    self.phaser_edit.master_beat = follows;
                }
                phaser::save_phasers(&self.phasers);
                self.log.push(format!(
                    "\"{name}\" {} the master beat",
                    if follows { "follows" } else { "ignores" }
                ));
                if let Some(addrs) = self.active_phasers.get(&name).cloned() {
                    let beat_clock = self.live.beat_clock();
                    for a in addrs {
                        if let Some(o) = self.live.oscs.get_mut(&a) {
                            if !follows {
                                o.local_beats = beat_clock;
                                o.local_tempo = self.live.tempo;
                            }
                            o.master_beat = follows;
                        }
                    }
                }
            }
        }
        if let Some(i) = do_load_apply {
            if let Some(ph) = self.phasers.get(i).cloned() {
                self.phaser_edit = ph.clone();
                // The editor only builds live wave/add phasers — don't let a
                // pose, FX or lock tile's stored snapshot leak into the next
                // Apply or Store.
                self.phaser_edit.static_pos = Vec::new();
                self.phaser_edit.hold = Vec::new();
                // Tiles stored before fixtures were recorded: remember the
                // selection they're first applied to, so clicking them later
                // with nothing selected targets the same lights.
                if ph.fixtures.is_empty() && ph.static_pos.is_empty() && ph.hold.is_empty() {
                    let keys: Vec<String> = self
                        .stage
                        .selected_fixtures()
                        .iter()
                        .filter_map(|&fi| self.patch.fixtures.get(fi))
                        .map(|f| crate::profiles::fixture_key(&f.display, f.from))
                        .collect();
                    if !keys.is_empty() {
                        if let Some(p) = self.phasers.get_mut(i) {
                            p.fixtures = keys.clone();
                        }
                        self.phaser_edit.fixtures = keys;
                        phaser::save_phasers(&self.phasers);
                    }
                }
                self.apply_phaser(ph, false);
            }
        }
        if let Some(i) = do_stop {
            if let Some(name) = self.phasers.get(i).map(|p| p.name.clone()) {
                self.stop_phaser(&name);
            }
        }
        if let Some(i) = do_update {
            if let Some(ph) = self.phasers.get_mut(i) {
                let name = ph.name.clone();
                let pose = std::mem::take(&mut ph.static_pos);
                let hold = std::mem::take(&mut ph.hold);
                let bound = std::mem::take(&mut ph.fixtures);
                *ph = self.phaser_edit.clone();
                ph.name = name;
                if !pose.is_empty() {
                    ph.static_pos = pose;
                }
                if !hold.is_empty() {
                    ph.hold = hold;
                }
                if ph.fixtures.is_empty() {
                    ph.fixtures = bound;
                }
                phaser::save_phasers(&self.phasers);
                self.log.push("Updated phaser".into());
            }
        }
        if let Some(i) = do_delete {
            if i < self.phasers.len() {
                let ph = self.phasers.remove(i);
                self.stop_phaser(&ph.name);
                phaser::save_phasers(&self.phasers);
                self.log.push(format!("Deleted phaser \"{}\"", ph.name));
                match self.phaser_edit_sel {
                    Some(s) if s == i => self.phaser_edit_sel = None,
                    Some(s) if s > i => self.phaser_edit_sel = Some(s - 1),
                    _ => {}
                }
            }
        }
    }
}

/// Channels the Gobos & effects section drives (and FX tiles store): gobo
/// wheels and rotation, prisms, frost, iris, animation and macro channels.
fn is_fx_channel(name: &str) -> bool {
    let l = name.to_lowercase();
    ["gobo", "prism", "frost", "iris", "animation", "macro", "effect"]
        .iter()
        .any(|k| l.contains(k))
}
