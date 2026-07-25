//! DMX channel monitor & connection tester: a scrollable live view of all
//! output channels, with per-channel overrides that are forced onto the wire
//! on top of everything — so you can verify the Art-Net link end to end.

use eframe::egui::{self, Color32, Rect, Sense};

use crate::app::App;
use crate::net;

impl App {
    pub(crate) fn dmx_test_window(&mut self, ctx: &egui::Context) {
        if !self.show_dmx_test {
            return;
        }
        let mut open = self.show_dmx_test;

        // Owner name for each address (for orientation while scrolling).
        let mut owner: Vec<Option<&str>> = vec![None; net::DMX_SLOTS];
        for f in &self.patch.fixtures {
            for a in f.from..=f.to {
                let i = a as usize;
                if (1..=net::DMX_SLOTS).contains(&i) {
                    owner[i - 1] = Some(&f.display);
                }
            }
        }
        // Copy of what is on the wire right now.
        let buf = *self.net.dmx.lock();
        let overrides = &mut self.test_overrides;

        egui::Window::new("DMX Monitor")
            .open(&mut open)
            .resizable(true)
            .default_size([380.0, 460.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "Live output, channels 1–{} — tick a channel to force it.",
                        net::DMX_SLOTS
                    ));
                });
                ui.horizontal(|ui| {
                    if overrides.is_empty() {
                        ui.weak("No overrides active.");
                    } else {
                        ui.colored_label(
                            Color32::from_rgb(235, 160, 70),
                            format!("{} override(s) forcing output", overrides.len()),
                        );
                        if ui.button("Clear all").clicked() {
                            overrides.clear();
                        }
                    }
                });
                ui.separator();

                let row_h = 18.0;
                egui::ScrollArea::vertical().show_rows(
                    ui,
                    row_h,
                    net::DMX_SLOTS,
                    |ui, range| {
                        for a in range {
                            ui.horizontal(|ui| {
                                ui.monospace(format!("{:4}", a + 1));

                                // Level bar.
                                let v = buf[a];
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(110.0, 11.0),
                                    Sense::hover(),
                                );
                                ui.painter().rect_filled(rect, 2.0, crate::ui::theme::WELL);
                                if v > 0 {
                                    let w = rect.width() * v as f32 / 255.0;
                                    let col = if overrides.contains_key(&a) {
                                        Color32::from_rgb(235, 160, 70)
                                    } else {
                                        Color32::from_rgb(90, 200, 130)
                                    };
                                    ui.painter().rect_filled(
                                        Rect::from_min_size(
                                            rect.min,
                                            egui::vec2(w.max(1.0), rect.height()),
                                        ),
                                        2.0,
                                        col,
                                    );
                                }
                                ui.monospace(format!("{v:3}"));

                                // Override: tick = force (defaults to full).
                                let mut on = overrides.contains_key(&a);
                                if ui
                                    .checkbox(&mut on, "")
                                    .on_hover_text("Force this channel (test)")
                                    .changed()
                                {
                                    if on {
                                        overrides.insert(a, 255);
                                    } else {
                                        overrides.remove(&a);
                                    }
                                }
                                if let Some(val) = overrides.get_mut(&a) {
                                    ui.add(egui::DragValue::new(val).speed(2));
                                }

                                if let Some(name) = owner[a] {
                                    ui.weak(name);
                                }
                            });
                        }
                    },
                );
            });
        self.show_dmx_test = open;
        // Keep the view fresh while it is open (values animate underneath).
        if self.show_dmx_test {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}
