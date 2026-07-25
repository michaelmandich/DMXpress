//! Right-hand transform editor for the current selection (lights or a tower).

use eframe::egui;

use super::fixture::{classify, Archetype};
use super::layout::TOWER_SLOTS;
use super::math::V3;
use super::view::StageView;
use crate::showbuddy::Patch;

impl StageView {
    /// Transform editor for the current selection (right-hand inspector).
    pub fn inspector_ui(&mut self, ui: &mut egui::Ui, patch: &Patch) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.fly_mode, false, "Orbit");
            ui.selectable_value(&mut self.fly_mode, true, "Free fly");
        });
        if self.fly_mode {
            ui.add(egui::Slider::new(&mut self.fly_speed, 0.5..=30.0).text("Fly speed"));
            ui.weak(
                "Hover stage: WASD move · Space up · Shift down · arrows/right-drag look · scroll adjusts speed",
            );
        } else {
            ui.weak("Right-drag orbit · middle-drag pan · scroll zoom");
        }
        ui.separator();

        // Tower editor when a tower is picked and no lights are selected.
        if self.selection.is_empty() {
            if let Some(ti) = self.sel_tower {
                if ti >= self.towers.len() {
                    self.sel_tower = None;
                } else {
                    ui.label(format!("Tower {}", ti + 1));
                    let mounted = self
                        .instances
                        .iter()
                        .filter(|inst| inst.mount.is_some_and(|(t, _)| t == ti))
                        .count();
                    ui.weak(format!(
                        "{mounted}/{TOWER_SLOTS} slots holding lights — drag a light \
                         near a blue ring to snap it on"
                    ));
                    ui.separator();
                    let mut changed = false;
                    let old_yaw = self.towers[ti].yaw_deg;
                    {
                        let tw = &mut self.towers[ti];
                        egui::Grid::new("tower_xform").num_columns(2).show(ui, |ui| {
                            ui.label("X");
                            changed |= ui
                                .add(egui::DragValue::new(&mut tw.pos.x).speed(0.05))
                                .changed();
                            ui.end_row();
                            ui.label("Z");
                            changed |= ui
                                .add(egui::DragValue::new(&mut tw.pos.z).speed(0.05))
                                .changed();
                            ui.end_row();
                            ui.label("Yaw °");
                            changed |= ui
                                .add(egui::DragValue::new(&mut tw.yaw_deg).speed(0.5))
                                .changed();
                            ui.end_row();
                            ui.label("Height");
                            changed |= ui
                                .add(egui::Slider::new(&mut tw.height, 1.2..=6.0))
                                .changed();
                            ui.end_row();
                            ui.label("Width");
                            changed |= ui
                                .add(egui::Slider::new(&mut tw.width, 0.8..=4.0))
                                .changed();
                            ui.end_row();
                        });
                    }
                    ui.add_space(6.0);
                    if ui.button("🗑 Delete tower").clicked() {
                        self.delete_tower(patch, ti);
                    } else if changed {
                        let dyaw = self.towers[ti].yaw_deg - old_yaw;
                        if dyaw != 0.0 {
                            self.spin_tower_mounts(ti, dyaw);
                        }
                        self.save(patch);
                    }
                    return;
                }
            }
        }

        let mut sel: Vec<usize> = self.selection.iter().copied().collect();
        sel.sort_unstable();
        sel.retain(|&i| i < self.instances.len());
        if sel.is_empty() {
            ui.weak("Nothing selected.\nClick or drag-select lights in the stage view.");
            return;
        }
        let mut changed = false;

        if sel.len() == 1 {
            let i = sel[0];
            let fi = self.instances[i].fixture;
            if let Some(f) = patch.fixtures.get(fi) {
                ui.label(format!("{}  [{}..{}]", f.display, f.from, f.to));
                let copies = self.instances.iter().filter(|inst| inst.fixture == fi).count();
                if copies > 1 {
                    ui.weak(format!(
                        "{:?} — {copies} copies share these channels",
                        classify(f)
                    ));
                } else {
                    ui.weak(format!("{:?}", classify(f)));
                }
            }
            if self.instances[i].mount.is_some() {
                ui.weak("⚓ Snapped to tower — drag away to detach");
            }
            ui.separator();
            // Moving heads (a base + pan/tilt) only get a single base-spin
            // control — rotating the fixture itself with yaw+pitch causes
            // gimbal tumbling when it hangs straight down on a bar. Pars and
            // other bodies keep full yaw/pitch freedom.
            let has_base = patch
                .fixtures
                .get(fi)
                .map(|f| matches!(classify(f), Archetype::MovingPar | Archetype::Beam))
                .unwrap_or(false);
            let mut pos_changed = false;
            let t = &mut self.instances[i].t;
            egui::Grid::new("xform").num_columns(2).show(ui, |ui| {
                ui.label("X");
                pos_changed |= ui.add(egui::DragValue::new(&mut t.pos.x).speed(0.01)).changed();
                ui.end_row();
                ui.label("Y (height)");
                pos_changed |= ui.add(egui::DragValue::new(&mut t.pos.y).speed(0.01)).changed();
                ui.end_row();
                ui.label("Z");
                pos_changed |= ui.add(egui::DragValue::new(&mut t.pos.z).speed(0.01)).changed();
                ui.end_row();
                if has_base {
                    ui.label("Base spin °")
                        .on_hover_text("Rotate the base around its mounting axis. The pan/tilt sweep follows.");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut t.roll_deg)
                                .speed(1.0)
                                .range(0.0..=360.0),
                        )
                        .changed();
                    ui.end_row();
                } else {
                    ui.label("Yaw °");
                    changed |= ui
                        .add(egui::DragValue::new(&mut t.yaw_deg).speed(0.5).range(-360.0..=360.0))
                        .changed();
                    ui.end_row();
                    ui.label("Pitch °");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut t.pitch_deg)
                                .speed(0.5)
                                .range(-360.0..=360.0),
                        )
                        .changed();
                    ui.end_row();
                }
                ui.label("Size ×")
                    .on_hover_text("Scale this fixture up or down.");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut t.scale)
                            .speed(0.02)
                            .range(0.05..=10.0),
                    )
                    .changed();
                ui.end_row();
            });
            ui.label("Visualizer");
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.instances[i].opacity, 0.0..=1.0)
                        .text("Housing opacity"),
                )
                .on_hover_text(
                    "Fade this fixture's complete visual contribution: housing, \
                     emitting lens, beam haze and surface pool",
                )
                .changed();
            if pos_changed {
                // Manually moving a snapped light pulls it off the tower
                // (rotating/spinning it does not — snapped lights stay
                // spinnable).
                self.instances[i].mount = None;
                changed = true;
            }
        } else {
            ui.label(format!("{} lights selected", sel.len()));
            ui.weak("Drag values to nudge all:");
            ui.separator();
            let mut d = V3::default();
            let (mut dyaw, mut dpitch, mut dspin) = (0f32, 0f32, 0f32);
            let mut scale_mul = 1.0f32;
            let mut opacity = self.instances[sel[0]].opacity;
            let mut opacity_changed = false;
            egui::Grid::new("xform_multi").num_columns(2).show(ui, |ui| {
                ui.label("ΔX");
                ui.add(egui::DragValue::new(&mut d.x).speed(0.01));
                ui.end_row();
                ui.label("ΔY");
                ui.add(egui::DragValue::new(&mut d.y).speed(0.01));
                ui.end_row();
                ui.label("ΔZ");
                ui.add(egui::DragValue::new(&mut d.z).speed(0.01));
                ui.end_row();
                ui.label("Size ×")
                    .on_hover_text("Multiply the size of every selected fixture.");
                ui.add(
                    egui::DragValue::new(&mut scale_mul)
                        .speed(0.02)
                        .range(0.1..=10.0),
                );
                ui.end_row();
                ui.label("ΔSpin °")
                    .on_hover_text("Spin base-mounted lights around their mount axis.");
                ui.add(egui::DragValue::new(&mut dspin).speed(1.0));
                ui.end_row();
                ui.label("ΔYaw °");
                ui.add(egui::DragValue::new(&mut dyaw).speed(0.5));
                ui.end_row();
                ui.label("ΔPitch °");
                ui.add(egui::DragValue::new(&mut dpitch).speed(0.5));
                ui.end_row();
                ui.label("Housing opacity");
                opacity_changed |= ui
                    .add(egui::Slider::new(&mut opacity, 0.0..=1.0))
                    .on_hover_text(
                        "Fade housing, emitting lens, beam haze and surface pool for \
                         all selected/grouped lights",
                    )
                    .changed();
                ui.end_row();
            });
            if d != V3::default()
                || dyaw != 0.0
                || dpitch != 0.0
                || dspin != 0.0
                || scale_mul != 1.0
                || opacity_changed
            {
                let detached = d != V3::default();
                for &i in &sel {
                    let inst = &mut self.instances[i];
                    if detached {
                        inst.mount = None;
                    }
                    inst.t.pos = inst.t.pos + d;
                    inst.t.yaw_deg += dyaw;
                    inst.t.pitch_deg += dpitch;
                    inst.t.roll_deg = (inst.t.roll_deg + dspin).rem_euclid(360.0);
                    inst.t.scale = (inst.t.scale * scale_mul).clamp(0.05, 10.0);
                    if opacity_changed {
                        inst.opacity = opacity;
                    }
                }
                changed = true;
            }
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .button("⧉ Duplicate")
                .on_hover_text(
                    "Copy the selected light(s). Copies follow the same DMX \
                     channels — for fixtures wired to several physical units. (⌘D)",
                )
                .clicked()
            {
                self.duplicate_selection(patch);
            }
            if ui
                .button("Remove copy")
                .on_hover_text(
                    "Delete the selected copies. The last copy of each fixture \
                     always stays. (⌫)",
                )
                .clicked()
            {
                self.delete_selection(patch);
            }
        });

        if changed {
            self.save(patch);
        }
    }
}
