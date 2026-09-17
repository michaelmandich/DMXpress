//! Floating "Gobos" window: what the selected lights can project, shown as
//! the pictures of the gobos themselves. Click a slot to send those lights
//! to it. Turn on Assign to pick a picture from the catalogue for any slot
//! the catalogue doesn't know (or has wrong); the choice is kept per fixture
//! type, so every light of that model picks it up.

use std::collections::BTreeMap;

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, TextureId, Vec2};

use crate::app::App;
use crate::fixturedb;
use crate::gobo::{self, GoboPick};
use crate::net::DMX_SLOTS;
use crate::showbuddy::Fixture;

/// One slot on a wheel, ready to draw.
struct SlotRow {
    band: usize,
    label: String,
    /// The DMX value that lands in the middle of the slot.
    value: u8,
    key: Option<String>,
    /// The first light of the type is on this slot right now.
    current: bool,
    /// Chosen by hand in this window rather than by the catalogue.
    pinned: bool,
}

struct WheelRow {
    channel: usize,
    name: String,
    slots: Vec<SlotRow>,
}

/// One fixture type: every light of it in play, and its wheels.
struct TypeRow {
    profile: String,
    title: String,
    fixtures: Vec<usize>,
    wheels: Vec<WheelRow>,
}

/// Thumbnail side, in points before zoom.
const TILE: f32 = 56.0;
/// Thumbnails are downsampled to this many pixels a side before upload.
const THUMB_PX: usize = 64;

impl App {
    /// Re-run gobo assignment over the patch (after an assignment changes).
    pub(crate) fn assign_gobos(&mut self) {
        gobo::assign_patch(&mut self.patch, &self.library, &self.gobos, &self.user_gobos);
    }

    /// The thumbnail texture for a gobo, uploaded on first use.
    pub(crate) fn gobo_thumb(&mut self, ctx: &egui::Context, key: &str) -> Option<TextureId> {
        if let Some(cached) = self.gobo_thumbs.get(key) {
            return cached.as_ref().map(|t| t.id());
        }
        let handle = self.gobos.mask(key).map(|mask| {
            let n = mask.size as usize;
            let step = (n / THUMB_PX).max(1);
            let side = n / step;
            let mut px = Vec::with_capacity(side * side);
            for y in 0..side {
                for x in 0..side {
                    // Box-filter each step×step block.
                    let mut sum = 0u32;
                    for dy in 0..step {
                        for dx in 0..step {
                            sum += mask.data[(y * step + dy) * n + x * step + dx] as u32;
                        }
                    }
                    px.push((sum / (step * step) as u32) as u8);
                }
            }
            let image = egui::ColorImage::from_gray([side, side], &px);
            ctx.load_texture(format!("gobo:{key}"), image, egui::TextureOptions::LINEAR)
        });
        let id = handle.as_ref().map(|t| t.id());
        self.gobo_thumbs.insert(key.to_string(), handle);
        id
    }

    /// The fixture types (and their gobo wheels) the window lists: the
    /// selected lights' types, or every type in the rig when nothing is
    /// selected. Only types with a gobo wheel show up.
    fn gobo_rows(&self, buf: &[u8; DMX_SLOTS]) -> Vec<TypeRow> {
        let selected = self.stage.selected_fixtures();
        let pool: Vec<usize> = if selected.is_empty() {
            (0..self.patch.fixtures.len()).collect()
        } else {
            selected
        };
        let mut by_profile: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for fi in pool {
            let Some(f) = self.patch.fixtures.get(fi) else { continue };
            if f.channels.iter().any(|c| c.is_gobo_wheel()) {
                by_profile.entry(gobo::profile_of(f)).or_default().push(fi);
            }
        }
        by_profile
            .into_iter()
            .map(|(profile, fixtures)| {
                let first = &self.patch.fixtures[fixtures[0]];
                let title = self.profile_title(&profile, first);
                let wheels = first
                    .channels
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.is_gobo_wheel())
                    .map(|(ci, ch)| {
                        let addr = first.from as usize + ci;
                        let live = if (1..=DMX_SLOTS).contains(&addr) { buf[addr - 1] } else { 0 };
                        let slots = ch
                            .bands
                            .iter()
                            .enumerate()
                            .filter(|(_, b)| b.kind == 'S' && !(b.min == 0 && b.max == 255))
                            .map(|(bi, b)| SlotRow {
                                band: bi,
                                label: if b.label.trim().is_empty() {
                                    format!("{}–{}", b.min, b.max)
                                } else {
                                    b.label.trim().to_string()
                                },
                                value: ((b.min as u16 + b.max as u16) / 2) as u8,
                                key: b.gobo.clone(),
                                current: live >= b.min && live <= b.max,
                                pinned: self.user_gobos.is_pinned(&profile, ci, bi),
                            })
                            .collect();
                        WheelRow { channel: ci, name: ch.name.clone(), slots }
                    })
                    .collect();
                TypeRow { profile, title, fixtures, wheels }
            })
            .collect()
    }

    /// A readable name for a fixture type: the library's, the built-in
    /// profile's, or the ShowBuddy light's own.
    fn profile_title(&self, profile: &str, first: &Fixture) -> String {
        if let Some(id) = fixturedb::library_id(profile) {
            return match self.library.find(id) {
                Some(e) => format!("{} · {}", e.title(), e.subtitle()),
                None => id.to_string(),
            };
        }
        if let Some(name) = profile.strip_prefix("builtin:") {
            return name.to_string();
        }
        first.display.clone()
    }

    /// Send lights to a gobo slot: the programmer takes their wheel channel.
    /// Returns how many lights it reached.
    pub(crate) fn set_gobo_slot(&mut self, fixtures: &[usize], channel: usize, value: u8) -> usize {
        // Settle any running fade so the change is visible immediately.
        if self.transition_run.take().is_some() {
            self.live = crate::oscillator::Look::from_frame(*self.net.dmx.lock());
        }
        let mut reached = 0;
        for &fi in fixtures {
            let Some(f) = self.patch.fixtures.get(fi) else { continue };
            let addr = f.from as usize + channel;
            if !(1..=DMX_SLOTS).contains(&addr) {
                continue;
            }
            let addr0 = addr - 1;
            self.base_fades.remove(&addr0);
            self.live.base[addr0] = value;
            self.live_active.insert(addr0);
            self.live_refs.remove(&addr0);
            reached += 1;
        }
        reached
    }

    pub(crate) fn gobos_window(&mut self, ctx: &egui::Context) {
        if !self.show_gobos {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_gobos;
        let mut popped = self.popped_out.contains("gobos");
        let buf = *self.net.dmx.lock();
        let rows = self.gobo_rows(&buf);
        let selected_count = self.stage.selected_fixtures().len();
        // Deferred: (lights, wheel channel, value, slot label).
        let mut do_set: Option<(Vec<usize>, usize, u8, String)> = None;
        // Deferred: the slot, and Some(key) to pin, Some("") to blank, None for auto.
        let mut do_assign: Option<(GoboPick, Option<String>)> = None;

        let mut zoom_level = self.zoom.gobos;
        super::floating_panel(
            ctx,
            "gobos",
            "Gobos",
            &mut open,
            &mut popped,
            Some(&mut zoom_level),
            [560.0, 440.0],
            [screen.right() - 600.0, 140.0],
            |ui| {
                ui.horizontal(|ui| {
                    if selected_count == 0 {
                        ui.weak("Nothing selected: every type in the rig, clicks reach all lights of a type");
                    } else {
                        ui.weak(format!("{selected_count} selected"));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.toggle_value(&mut self.gobo_assign_mode, "✎ Assign pictures")
                            .on_hover_text(
                                "Click a slot to choose the picture it projects. Kept per \
                                 fixture type, so every light of that model gets it.",
                            );
                    });
                });
                if let Some(e) = &self.gobos.error {
                    ui.colored_label(
                        Color32::from_rgb(240, 180, 90),
                        format!("Gobo catalogue: {e}"),
                    );
                }
                ui.separator();

                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if let Some(pick) = self.gobo_pick.clone() {
                        self.gobo_picker(ui, ctx, &pick, &mut do_assign);
                        ui.separator();
                    }
                    if rows.is_empty() {
                        ui.weak("None of these lights has a gobo wheel.");
                    }
                    for row in &rows {
                        ui.horizontal(|ui| {
                            ui.strong(&row.title);
                            ui.weak(format!("×{}", row.fixtures.len()));
                        });
                        for wheel in &row.wheels {
                            ui.label(&wheel.name);
                            ui.horizontal_wrapped(|ui| {
                                for slot in &wheel.slots {
                                    let tex = slot.key.as_deref().and_then(|k| self.gobo_thumb(ctx, k));
                                    let hover = match (&slot.key, slot.pinned) {
                                        (Some(k), true) => format!("{} · {k} (your pick)", slot.label),
                                        (Some(k), false) => format!("{} · {k}", slot.label),
                                        (None, true) => format!("{} · no gobo (your pick)", slot.label),
                                        (None, false) => format!(
                                            "{} · no picture yet — turn on Assign pictures to choose one",
                                            slot.label
                                        ),
                                    };
                                    let resp =
                                        gobo_tile(ui, tex, &slot.label, TILE, slot.current, slot.pinned)
                                            .on_hover_text(hover);
                                    if resp.clicked() {
                                        if self.gobo_assign_mode {
                                            self.gobo_pick = Some(GoboPick {
                                                profile: row.profile.clone(),
                                                channel: wheel.channel,
                                                band: slot.band,
                                                title: format!(
                                                    "{} · {} · {}",
                                                    row.title, wheel.name, slot.label
                                                ),
                                            });
                                        } else {
                                            do_set = Some((
                                                row.fixtures.clone(),
                                                wheel.channel,
                                                slot.value,
                                                slot.label.clone(),
                                            ));
                                        }
                                    }
                                }
                            });
                        }
                        ui.add_space(6.0);
                    }
                });
            },
        );
        self.zoom.gobos = zoom_level;
        self.show_gobos = open;
        if popped {
            self.popped_out.insert("gobos");
        } else {
            self.popped_out.remove("gobos");
        }

        if let Some((fixtures, channel, value, label)) = do_set {
            let reached = self.set_gobo_slot(&fixtures, channel, value);
            self.log.push(format!("Gobo \"{label}\" on {reached} light(s)"));
        }
        if let Some((pick, key)) = do_assign {
            self.user_gobos.set(&pick.profile, pick.channel, pick.band, key.as_deref());
            self.user_gobos.save();
            self.assign_gobos();
            self.gobo_pick = None;
            self.log.push(match key.as_deref() {
                Some("") => format!("{}: no gobo", pick.title),
                Some(k) => format!("{}: {k}", pick.title),
                None => format!("{}: back to the catalogue's pick", pick.title),
            });
        }
    }

    /// The picture picker for one slot: search the catalogue, click a tile.
    fn gobo_picker(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        pick: &GoboPick,
        out: &mut Option<(GoboPick, Option<String>)>,
    ) {
        ui.strong(format!("Picture for {}", pick.title));
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.gobo_search)
                    .hint_text("search: stars, breakup, dots…")
                    .desired_width(170.0),
            );
            let makers: Vec<String> = self.gobos.manufacturers().iter().map(|m| m.to_string()).collect();
            egui::ComboBox::from_id_salt("gobo_maker")
                .selected_text(self.gobo_maker.as_deref().unwrap_or("All makers"))
                .width(150.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.gobo_maker, None, "All makers");
                    for m in makers {
                        let label = m.clone();
                        ui.selectable_value(&mut self.gobo_maker, Some(m), label);
                    }
                });
            if ui
                .button("No gobo")
                .on_hover_text("This slot projects a plain beam")
                .clicked()
            {
                *out = Some((pick.clone(), Some(String::new())));
            }
            if ui
                .button("Auto")
                .on_hover_text("Forget the hand-picked picture; use what the catalogue knows")
                .clicked()
            {
                *out = Some((pick.clone(), None));
            }
            if ui.button("Cancel").clicked() {
                self.gobo_pick = None;
            }
        });
        let hits = self.gobos.search(&self.gobo_search, self.gobo_maker.as_deref(), 120);
        let keys: Vec<(String, String)> = hits
            .iter()
            .map(|&i| (self.gobos.gobos[i].key.clone(), self.gobos.gobos[i].name.clone()))
            .collect();
        if keys.is_empty() {
            ui.weak("Nothing matches. Drop a PNG into fixtures/gobos/ to add your own.");
        }
        ui.horizontal_wrapped(|ui| {
            for (key, name) in &keys {
                let tex = self.gobo_thumb(ctx, key);
                let resp = gobo_tile(ui, tex, name, TILE, false, false).on_hover_text(key);
                if resp.clicked() {
                    *out = Some((pick.clone(), Some(key.clone())));
                }
            }
        });
        ui.weak(format!("{} of {} gobos", keys.len(), self.gobos.gobos.len()));
    }
}

/// A gobo slot tile: the mask as the light would throw it (white pattern
/// on black), the slot's label under it, a highlight when it is live, and a
/// corner mark when the picture was chosen by hand.
fn gobo_tile(
    ui: &mut egui::Ui,
    tex: Option<TextureId>,
    label: &str,
    size: f32,
    current: bool,
    pinned: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(size, size + 16.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let painter = ui.painter();
    let img = Rect::from_min_size(rect.min, Vec2::splat(size));
    painter.rect_filled(img, 4.0, Color32::from_gray(12));
    match tex {
        Some(tex) => {
            painter.image(
                tex,
                img.shrink(3.0),
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            painter.text(
                img.center(),
                Align2::CENTER_CENTER,
                "?",
                FontId::proportional(size * 0.4),
                Color32::from_gray(80),
            );
        }
    }
    let stroke = if current {
        Stroke::new(2.0, Color32::YELLOW)
    } else if resp.hovered() {
        Stroke::new(1.5, Color32::from_gray(170))
    } else {
        Stroke::new(1.0, Color32::from_gray(60))
    };
    painter.rect_stroke(img, 4.0, stroke);
    if pinned {
        painter.circle_filled(
            img.right_top() + Vec2::new(-6.0, 6.0),
            3.0,
            Color32::from_rgb(120, 200, 255),
        );
    }
    let text: String = label.chars().take(12).collect();
    painter.text(
        egui::pos2(rect.center().x, img.bottom() + 2.0),
        Align2::CENTER_TOP,
        text,
        FontId::proportional(9.5),
        Color32::from_gray(200),
    );
    resp
}
