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

/// Taps further apart than this start a fresh run (and a fresh downbeat).
const TAP_TIMEOUT: f32 = 2.5;
/// Rolling window: at most this many taps (8 intervals) shape the BPM, so the
/// tempo keeps adapting if the band drifts.
const MAX_TAPS: usize = 9;

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
        egui::Window::new("🥁 Beat")
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
                // Keep the bar indicator animating while the window is open.
                ctx.request_repaint_after(Duration::from_millis(40));
            });
        self.show_beat = open;

        if do_tap {
            self.beat_tap();
        }
    }
}
