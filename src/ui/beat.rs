//! Floating "Beat" window: master BPM plus rhythm-game style tap sync.
//!
//! Tap the big button (or Space while the window is open) along with the
//! band: the master BPM adapts to your recent taps and every look's beat
//! clock drifts onto the tap, so beat-synced oscillators settle onto the bar
//! without a visible phase jump.
//! The first tap after a pause marks the bar's downbeat ("1"); keep tapping
//! quarter notes to re-sync as often as needed.

use std::time::{Duration, Instant};

use eframe::egui;

use crate::app::App;
use crate::oscillator;

use super::theme;

/// Taps further apart than this start a fresh run (and a fresh downbeat).
const TAP_TIMEOUT: f32 = 2.5;
/// Rolling window: at most this many taps (8 intervals) shape the BPM, so the
/// tempo keeps adapting if the band drifts.
const MAX_TAPS: usize = 9;
/// Fastest the time machine will run animation time.
const TIME_RATE_MAX: f32 = 4.0;

impl App {
    /// One tap: adapt the master BPM and gently pull every participating
    /// clock toward the tap.
    pub(crate) fn beat_tap(&mut self) {
        let now = Instant::now();
        if self
            .beat_taps
            .last()
            .is_some_and(|t| now.duration_since(*t).as_secs_f32() > TAP_TIMEOUT)
        {
            self.beat_taps.clear();
        }
        let fresh = self.beat_taps.is_empty();
        self.beat_taps.push(now);
        if self.beat_taps.len() > MAX_TAPS {
            self.beat_taps.remove(0);
        }
        if self.beat_taps.len() >= 2 {
            let span = now.duration_since(self.beat_taps[0]).as_secs_f32();
            let bpm = 60.0 * (self.beat_taps.len() - 1) as f32 / span;
            self.master_bpm = bpm.clamp(30.0, 300.0);
            self.master_bpm_on = true;
        }
        // The first tap of a run marks the bar's downbeat; later taps nudge
        // the clocks onto the nearest beat.
        self.drift_beat_clocks(if fresh { 4.0 } else { 1.0 });
    }

    /// Queue a smooth correction toward the nearest `quantum` beats.
    fn drift_beat_clocks(&mut self, quantum: f32) {
        self.live.drift_beats(quantum);
        if let Some(run) = &mut self.transition_run {
            run.drift_beats(quantum);
        }
        if let Some(run) = &mut self.chase_run {
            run.drift_beats(quantum);
        }
        if self.cycle_master_beat {
            let target = (self.cycle_beats / quantum).round() * quantum;
            self.cycle_beat_nudge = target - self.cycle_beats;
        }
    }
    /// Run the time machine for this frame: flip the pendulum if it has swung
    /// far enough, throw the clock back if stutter is looping, then publish the
    /// resulting warp so every look integrates against it.
    pub(crate) fn advance_time_machine(&mut self) {
        let beats = self.live.beat_clock();

        if self.pendulum {
            let span = self.pendulum_beats.max(0.25);
            if (beats - self.pendulum_anchor).abs() >= span {
                self.time_reverse = !self.time_reverse;
                self.pendulum_anchor = beats;
            }
        } else {
            self.pendulum_anchor = beats;
        }

        if self.stutter {
            let span = self.stutter_beats.max(1.0 / 16.0);
            let anchor = *self.stutter_anchor.get_or_insert(beats);
            let travelled = beats - anchor;
            if travelled.abs() >= span {
                // Reversed time walks the loop the other way, so throw the
                // clock back along whichever direction it came from.
                self.live.shift_beats(-travelled.signum() * span);
            }
        } else {
            self.stutter_anchor = None;
        }

        let rate = self.time_rate.clamp(0.0, TIME_RATE_MAX);
        oscillator::set_time_warp(if self.time_reverse { -rate } else { rate });
    }

    /// Back to plain forward real time.
    pub(crate) fn reset_time_machine(&mut self) {
        self.time_rate = 1.0;
        self.time_reverse = false;
        self.pendulum = false;
        self.stutter = false;
        self.stutter_anchor = None;
        oscillator::set_time_warp(1.0);
    }

    /// Jump every beat clock by `d` beats without disturbing the tempo.
    fn nudge_beats(&mut self, d: f32) {
        self.live.shift_beats(d);
        self.cycle_beats += d;
    }
    pub(crate) fn beat_window(&mut self, ctx: &egui::Context) {
        if !self.show_beat {
            return;
        }
        // Space taps whenever the window is open and nothing is typing.
        if !self.stage.fly_mode
            && ctx.input(|i| i.key_pressed(egui::Key::Space))
            && !ctx.wants_keyboard_input()
        {
            self.beat_tap();
        }

        let screen = ctx.screen_rect();
        let mut open = self.show_beat;
        let mut do_tap = false;
        egui::Window::new("Beat")
            .open(&mut open)
            .collapsible(true)
            .resizable(false)
            .default_pos([screen.center().x - 140.0, 120.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.master_bpm_on, "Master BPM")
                        .on_hover_text(
                            "Drive opted-in looks, phasers and palette sequences from \
                             this clock; individual cards can opt out",
                        );
                    ui.add_enabled(
                        self.master_bpm_on,
                        egui::DragValue::new(&mut self.master_bpm)
                            .range(30.0..=300.0)
                            .speed(0.2)
                            .fixed_decimals(1)
                            .suffix(" bpm"),
                    );
                    if ui.small_button("×2").clicked() {
                        self.master_bpm = (self.master_bpm * 2.0).min(300.0);
                    }
                    if ui.small_button("÷2").clicked() {
                        self.master_bpm = (self.master_bpm / 2.0).max(30.0);
                    }
                });
                ui.add_space(6.0);

                let resp = ui.add_sized(
                    [250.0, 64.0],
                    egui::Button::new(
                        egui::RichText::new("TAP  (space)").size(18.0).strong(),
                    ),
                );
                // Never keep keyboard focus: Space must reach the tap handler
                // above instead of "clicking" the focused button twice.
                resp.surrender_focus();
                if resp.clicked() {
                    do_tap = true;
                }

                // 4-beat bar indicator (beat 1 in orange).
                let phase = if let Some(run) = &self.transition_run {
                    run.pending().beat_phase()
                } else {
                    self.live.beat_phase()
                };
                let beat = (phase.floor() as usize).min(3);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    for k in 0..4 {
                        let (rect, _) = ui
                            .allocate_exact_size(egui::vec2(36.0, 26.0), egui::Sense::hover());
                        let active = k == beat;
                        let col = if k == 0 {
                            egui::Color32::from_rgb(255, 165, 70)
                        } else {
                            egui::Color32::from_gray(220)
                        };
                        let fade = if active {
                            1.0 - phase.fract() * 0.75
                        } else {
                            0.18
                        };
                        ui.painter().circle_filled(
                            rect.center(),
                            if active { 11.0 } else { 7.0 },
                            col.gamma_multiply(fade),
                        );
                    }
                });
                ui.add_space(4.0);
                ui.weak(
                    "Tap with the band — the BPM adapts to your last taps. The first \
                     tap after a pause marks the bar's downbeat (\"1\"); every tap \
                     re-syncs the oscillators, as often as you need.",
                );
                if self.master_bpm_on {
                    ui.weak(format!(
                        "{} tap(s) in the current run",
                        self.beat_taps.len()
                    ));
                }

                ui.add_space(10.0);
                self.time_machine_ui(ui);

                // Keep the bar indicator animating while the window is open.
                ctx.request_repaint_after(Duration::from_millis(40));
            });
        self.show_beat = open;

        if do_tap {
            self.beat_tap();
        }
    }

    /// The time machine: everything that bends the flow of animation time
    /// rather than the tempo it is measured in.
    fn time_machine_ui(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Time machine");

        // Current state, at a glance.
        ui.horizontal(|ui| {
            if self.frozen {
                theme::pill(ui, "HELD", theme::WARN);
            } else if self.time_rate <= 0.001 {
                theme::pill(ui, "STOPPED", theme::WARN);
            } else {
                let dir = if self.time_reverse { "REVERSE" } else { "FORWARD" };
                let col = if self.time_reverse { theme::ACCENT_SOFT } else { theme::OK };
                theme::pill(ui, &format!("{dir}  {:.2}x", self.time_rate), col);
            }
            if self.pendulum {
                theme::pill(ui, "PENDULUM", theme::ACCENT_SOFT);
            }
            if self.stutter {
                theme::pill(ui, "STUTTER", theme::ACCENT_SOFT);
            }
        });
        ui.add_space(6.0);

        // Direction and hold. Freeze is the existing global transport, surfaced
        // here so the whole transport lives in one place.
        ui.horizontal(|ui| {
            let rev = self.time_reverse;
            if ui
                .selectable_label(rev, "  Reverse  ")
                .on_hover_text("Run every oscillator, chase and cycle backwards")
                .clicked()
            {
                self.time_reverse = true;
            }
            if ui
                .selectable_label(!rev, "  Forward  ")
                .on_hover_text("Normal direction")
                .clicked()
            {
                self.time_reverse = false;
            }
            ui.separator();
            let frozen = self.frozen;
            if ui
                .selectable_label(frozen, if frozen { "  Resume  " } else { "  Hold  " })
                .on_hover_text(
                    "Sample and hold: the last frame keeps transmitting and no \
                     clock advances. Resuming picks up exactly where it stopped.",
                )
                .clicked()
            {
                self.set_frozen(!frozen);
            }
        });
        ui.add_space(6.0);

        // Rate.
        ui.horizontal(|ui| {
            ui.label("Rate");
            for (label, mul) in [("1/4", 0.25), ("1/2", 0.5), ("1x", 1.0), ("2x", 2.0), ("4x", 4.0)]
            {
                let on = (self.time_rate - mul).abs() < 0.01;
                if ui.selectable_label(on, label).clicked() {
                    self.time_rate = mul;
                }
            }
        });
        ui.add(
            egui::Slider::new(&mut self.time_rate, 0.0..=TIME_RATE_MAX)
                .fixed_decimals(2)
                .suffix("x")
                .text("speed"),
        )
        .on_hover_text("0 stops animation time without freezing the output");
        ui.add_space(6.0);

        // Manual nudges: line the show up with a band that has drifted.
        ui.horizontal(|ui| {
            ui.label("Nudge");
            if ui.button("-1/4").clicked() {
                self.nudge_beats(-0.25);
            }
            if ui.button("+1/4").clicked() {
                self.nudge_beats(0.25);
            }
            ui.separator();
            if ui
                .button("Rewind 4")
                .on_hover_text("Throw every clock back one bar")
                .clicked()
            {
                self.nudge_beats(-4.0);
            }
            if ui
                .button("Downbeat")
                .on_hover_text("Snap the clocks to the nearest bar line")
                .clicked()
            {
                self.drift_beat_clocks(4.0);
            }
        });
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.pendulum, "Pendulum")
                .on_hover_text(
                    "Flip direction every so often, so movement sweeps out and \
                     retraces its path instead of looping back to the start",
                );
            ui.add_enabled(
                self.pendulum,
                egui::DragValue::new(&mut self.pendulum_beats)
                    .range(0.25..=32.0)
                    .speed(0.25)
                    .fixed_decimals(2)
                    .suffix(" beats"),
            );
        });

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.stutter, "Stutter")
                .on_hover_text(
                    "Trap the clock in a short loop — the look repeats the same \
                     fragment until you let it go",
                );
            ui.add_enabled(
                self.stutter,
                egui::DragValue::new(&mut self.stutter_beats)
                    .range(0.0625..=4.0)
                    .speed(0.0625)
                    .fixed_decimals(3)
                    .suffix(" beats"),
            );
            for (label, len) in [("1/16", 0.0625), ("1/8", 0.125), ("1/4", 0.25), ("1/2", 0.5)] {
                let on = self.stutter && (self.stutter_beats - len).abs() < 0.001;
                if ui.selectable_label(on, label).clicked() {
                    self.stutter_beats = len;
                    self.stutter = true;
                }
            }
        });

        ui.add_space(6.0);
        if ui
            .button("Reset time")
            .on_hover_text("Back to plain forward real time")
            .clicked()
        {
            self.reset_time_machine();
        }
        theme::hint(
            ui,
            "The time machine bends how fast time passes; the BPM above sets what \
             a beat means. Both together: half speed at 128 bpm still lands on the bar.",
        );
    }
}
