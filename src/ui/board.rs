//! The Phaser board: the pads you play a show from.
//!
//! The Phasers window is where phasers are designed; this is where they are
//! used. It is the same 4 × 9 page of pads the Stream Deck's Phaser page
//! shows — the same `phaser_deck` slots, the same reserved corners for
//! select-all, Clear and Tap — so what is arranged here is exactly what is
//! under your fingers on the device, and a show without a deck gets the
//! same board on screen. Pressing a pad starts or stops its phaser.
//!
//! Arrange mode turns the board into its own editor: click a pad to assign
//! a phaser, symbol and colour, drag pads to swap them, right-click to
//! clear. The Phasers window adds to the board from its library, so the
//! two can sit side by side — this one popped out — while a set of
//! channels is designed and dropped onto a pad.

use eframe::egui::{self, Color32, Rect, Rounding, Sense, Shape, Stroke};

use super::theme;
use super::theme::readable_on;
use crate::app::App;
use crate::phaser::{PathShape, Phaser};
use crate::streamdeck::{
    phaser_deck_pages, phaser_page_key, save_phaser_deck, KeyIcon, PhaserKey, PhaserSlot,
    PHASER_DECK_SLOTS,
};

pub(crate) const ROWS: usize = 4;
pub(crate) const COLS: usize = 9;
pub(crate) const GAP: f32 = 5.0;

/// The four-character symbol a pad shows for a phaser name.
pub(crate) fn pad_symbol(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).take(4).collect::<String>().to_uppercase()
}

/// The deck artwork that suits a phaser: arrows for pan and tilt, a figure
/// for a path, nothing for the rest.
pub(crate) fn pad_icon(ph: &Phaser) -> KeyIcon {
    match ph.path {
        Some(PathShape::Circle) => return KeyIcon::Ring,
        Some(PathShape::Eight | PathShape::Arc | PathShape::ZigZag) => return KeyIcon::Wave,
        Some(PathShape::Diamond | PathShape::Lissajous) => return KeyIcon::Bullseye,
        Some(PathShape::Line | PathShape::Diagonal) => return KeyIcon::LeftRight,
        Some(PathShape::Lift) => return KeyIcon::UpDown,
        _ => {}
    }
    let targets: Vec<String> = ph
        .components
        .iter()
        .map(|c| c.target.to_ascii_uppercase())
        .chain(ph.targets.iter().map(|t| t.to_ascii_uppercase()))
        .collect();
    let pan = targets.iter().any(|t| t == "PAN" || t == "PANF");
    let tilt = targets.iter().any(|t| t == "TILT" || t == "TILTF");
    match (pan, tilt) {
        (true, false) => KeyIcon::LeftRight,
        (false, true) => KeyIcon::UpDown,
        _ => KeyIcon::None,
    }
}

/// Paint one pad: a lit tile in its colour (muted while idle), the symbol
/// large, the phaser's name small beneath, and a ring while it runs.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_pad(
    ui: &egui::Ui,
    rect: Rect,
    color: Option<[u8; 3]>,
    symbol: &str,
    name: &str,
    running: bool,
    editing: bool,
    hovered: bool,
    drop_target: bool,
) {
    theme::paint_pad(
        ui.painter(),
        rect,
        theme::PadState {
            fill: color.map(|[r, g, b]| Color32::from_rgb(r, g, b)),
            active: running,
            editing,
            hovered,
            drop_target,
            dim: !running,
        },
        theme::PadFace::Symbol(symbol),
        name,
    );
}

/// A reserved corner key, drawn as a working button.
pub(crate) fn reserved_pad(
    ui: &mut egui::Ui,
    rect: Rect,
    salt: impl std::hash::Hash,
    label: &str,
    tint: Color32,
    active: bool,
) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with(("reserved", salt)), Sense::click());
    let fill = if active { tint } else { tint.lerp_to_gamma(theme::SURFACE, 0.55) };
    let fill = if resp.hovered() { fill.lerp_to_gamma(Color32::WHITE, 0.08) } else { fill };
    let p = ui.painter();
    let rounding = Rounding::same(6.0);
    p.rect_filled(rect.translate(egui::vec2(0.0, 1.0)), rounding, Color32::from_black_alpha(120));
    p.add(Shape::mesh(theme::vgradient_mesh(
        rect,
        rounding,
        fill.lerp_to_gamma(Color32::WHITE, 0.10),
        fill.lerp_to_gamma(Color32::BLACK, 0.14),
    )));
    p.rect_stroke(rect, rounding, Stroke::new(1.0, theme::RIM));
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::new((rect.height() * 0.3).clamp(10.0, 16.0), theme::semibold()),
        readable_on(fill),
    );
    resp
}

impl App {
    pub(crate) fn phaser_board_window(&mut self, ctx: &egui::Context) {
        if !self.show_phaser_board {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_phaser_board;
        let mut popped = self.popped_out.contains("phaser_board");
        let mut zoom_level = self.zoom.phasers;
        super::floating_panel(
            ctx,
            "phaser_board",
            "Phaser board",
            &mut open,
            &mut popped,
            Some(&mut zoom_level),
            [600.0, 430.0],
            [screen.right() - 640.0, screen.bottom() - 470.0],
            |ui| self.phaser_board_ui(ui),
        );
        self.zoom.phasers = zoom_level;
        self.show_phaser_board = open;
        if popped {
            self.popped_out.insert("phaser_board");
        } else {
            self.popped_out.remove("phaser_board");
        }
    }

    /// The board itself: page bar, the pad grid, and in arrange mode the
    /// slot editor under it.
    pub(crate) fn phaser_board_ui(&mut self, ui: &mut egui::Ui) {
        let pages = phaser_deck_pages(&self.phaser_deck);
        self.deck_phaser_page = self.deck_phaser_page.min(pages - 1);

        ui.horizontal(|ui| {
            if ui
                .add_enabled(pages > 1, egui::Button::new("◀"))
                .on_hover_text("Previous page (the deck's Page knob does the same)")
                .clicked()
            {
                self.step_phaser_deck_page(-1);
            }
            ui.label(
                egui::RichText::new(format!("Page {} / {}", self.deck_phaser_page + 1, pages))
                    .family(theme::medium()),
            );
            if ui.add_enabled(pages > 1, egui::Button::new("▶")).on_hover_text("Next page").clicked() {
                self.step_phaser_deck_page(1);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .selectable_label(self.board_arrange, "Arrange")
                    .on_hover_text(
                        "Lay the board out: click a pad to assign a phaser, drag pads to \
                         swap them, right-click to clear",
                    )
                    .clicked()
                {
                    self.board_arrange = !self.board_arrange;
                    if !self.board_arrange {
                        self.deck_slot_edit = None;
                    }
                }
                if self.board_arrange {
                    ui.menu_button("Fill…", |ui| {
                        if ui
                            .button("Rows 1–2: movement presets")
                            .on_hover_text(
                                "Pan and tilt phasers at three speeds × three sizes, added \
                                 to the library if missing and laid into rows 1 and 2",
                            )
                            .clicked()
                        {
                            self.fill_movement_deck_page();
                            ui.close_menu();
                        }
                        if ui
                            .button("Row 3: path shapes")
                            .on_hover_text(
                                "One phaser per pan/tilt figure — circle, eight, arc, \
                                 square… — from row 3 on",
                            )
                            .clicked()
                        {
                            self.fill_path_deck_page();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Add a page").clicked() {
                            self.add_phaser_deck_page();
                            ui.close_menu();
                        }
                        let base = self.deck_phaser_page * PHASER_DECK_SLOTS;
                        let page_empty = self.phaser_deck[base..base + PHASER_DECK_SLOTS]
                            .iter()
                            .all(|s| s.is_none());
                        if ui
                            .add_enabled(pages > 1 && page_empty, egui::Button::new("Remove this page"))
                            .on_hover_text("Only while the page is empty")
                            .clicked()
                        {
                            self.remove_phaser_deck_page();
                            ui.close_menu();
                        }
                    });
                }
            });
        });
        theme::hint(
            ui,
            if self.board_arrange {
                "Click a pad to assign · drag a pad onto another to swap · right-click to clear"
            } else {
                "Press a pad to start or stop it · ALL, CLR and TAP work as on the deck"
            },
        );
        ui.add_space(4.0);

        // The page bar may have moved us; index the grid off the page it
        // settled on.
        let base = self.deck_phaser_page * PHASER_DECK_SLOTS;
        let avail = ui.available_width();
        let w = ((avail - GAP * (COLS as f32 - 1.0)) / COLS as f32).clamp(40.0, 120.0);
        let h = (w * 0.74).clamp(34.0, 92.0);

        let mut press: Option<usize> = None;
        let mut open_edit: Option<usize> = None;
        let mut clear_slot: Option<usize> = None;
        let mut swap: Option<(usize, usize)> = None;
        let mut reserved: Option<PhaserKey> = None;
        let arrange = self.board_arrange;
        let dragging = egui::DragAndDrop::payload::<usize>(ui.ctx()).map(|p| *p);

        for row in 0..ROWS {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for col in 0..COLS {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
                    let k = row * COLS + col;
                    match phaser_page_key(k, ROWS, COLS) {
                        PhaserKey::Pad(slot) => {
                            let idx = base + slot;
                            let entry = self.phaser_deck.get(idx).cloned().flatten();
                            let id = ui.id().with(("pad", idx));
                            let sense = if arrange { Sense::click_and_drag() } else { Sense::click() };
                            let resp = ui.interact(rect, id, sense);
                            let running = entry
                                .as_ref()
                                .is_some_and(|s| self.active_phasers.contains_key(&s.phaser));
                            let editing = arrange && self.deck_slot_edit == Some(idx);
                            let drop_here = arrange
                                && dragging.is_some_and(|d| d != idx)
                                && resp.hovered();
                            let (color, symbol, name) = match &entry {
                                Some(s) => (Some(s.color), s.label.clone(), s.phaser.clone()),
                                None => (None, if arrange { "+".to_string() } else { String::new() }, String::new()),
                            };
                            let hovered = resp.hovered() && (arrange || entry.is_some());
                            paint_pad(ui, rect, color, &symbol, &name, running, editing, hovered, drop_here);
                            if arrange {
                                if entry.is_some() && resp.drag_started() {
                                    egui::DragAndDrop::set_payload(ui.ctx(), idx);
                                }
                                if let Some(from) = resp.dnd_release_payload::<usize>() {
                                    if *from != idx {
                                        swap = Some((*from, idx));
                                    }
                                }
                                if resp.clicked() {
                                    open_edit = Some(idx);
                                }
                                if entry.is_some() {
                                    resp.context_menu(|ui| {
                                        if ui.button("Clear pad").clicked() {
                                            clear_slot = Some(idx);
                                            ui.close_menu();
                                        }
                                    });
                                }
                            } else if entry.is_some() {
                                let resp = resp.on_hover_text(if running {
                                    format!("\"{name}\" — running · press to stop")
                                } else {
                                    format!("\"{name}\" — press to start")
                                });
                                if resp.clicked() {
                                    press = Some(idx);
                                }
                            }
                        }
                        key => {
                            let (label, tint, active, hint) = match key {
                                PhaserKey::Tap => (
                                    "TAP",
                                    Color32::from_rgb(31, 111, 120),
                                    self.beat_taps.last().is_some_and(|t| t.elapsed().as_secs_f32() < 0.15),
                                    "Tap the beat",
                                ),
                                PhaserKey::Clear => (
                                    "CLR",
                                    Color32::from_rgb(160, 105, 40),
                                    false,
                                    "Clear: encoders, then effects, then blackout — one stage per press",
                                ),
                                _ => (
                                    "ALL",
                                    Color32::from_rgb(150, 128, 42),
                                    self.stage.all_selected(),
                                    "Select every light, or let go of them all",
                                ),
                            };
                            if reserved_pad(ui, rect, k, label, tint, active).on_hover_text(hint).clicked() {
                                reserved = Some(key);
                            }
                        }
                    }
                }
            });
            ui.add_space(GAP);
        }

        if let Some((a, b)) = swap {
            if a < self.phaser_deck.len() && b < self.phaser_deck.len() {
                self.phaser_deck.swap(a, b);
                save_phaser_deck(&self.phaser_deck);
                if self.deck_slot_edit == Some(a) {
                    self.deck_slot_edit = Some(b);
                }
            }
        }
        if let Some(idx) = clear_slot {
            if idx < self.phaser_deck.len() {
                self.phaser_deck[idx] = None;
                save_phaser_deck(&self.phaser_deck);
            }
            if self.deck_slot_edit == Some(idx) {
                self.deck_slot_edit = None;
            }
        }
        if let Some(idx) = press {
            if let Some(slot) = self.phaser_deck.get(idx).cloned().flatten() {
                if self.active_phasers.contains_key(&slot.phaser) {
                    self.stop_phaser(&slot.phaser);
                } else if let Some(ph) = self.phasers.iter().find(|p| p.name == slot.phaser).cloned() {
                    self.apply_phaser(ph, false);
                } else {
                    self.log.push(format!(
                        "Pad \"{}\": no phaser of that name in the library any more",
                        slot.phaser
                    ));
                }
            }
        }
        match reserved {
            Some(PhaserKey::Tap) => self.beat_tap(),
            Some(PhaserKey::Clear) => self.clear_stage(),
            Some(PhaserKey::SelectAll) => {
                let on = self.stage.select_all_fixtures();
                self.sync_selection_units();
                self.log.push(if on {
                    format!("Selected all {} lights", self.patch.fixtures.len())
                } else {
                    "Selection cleared".into()
                });
            }
            _ => {}
        }
        if let Some(idx) = open_edit {
            self.deck_slot_edit = Some(idx);
            match self.phaser_deck.get(idx).cloned().flatten() {
                Some(s) => {
                    self.deck_slot_phaser = s.phaser;
                    self.deck_slot_color = s.color;
                    self.deck_slot_label = s.label;
                    self.deck_slot_icon = s.icon;
                }
                None => {
                    self.deck_slot_icon = KeyIcon::None;
                    if let Some(first) = self.phasers.first() {
                        self.deck_slot_phaser = first.name.clone();
                        self.deck_slot_color = first.color;
                        self.deck_slot_label = pad_symbol(&first.name);
                    } else {
                        self.deck_slot_phaser.clear();
                        self.deck_slot_label.clear();
                    }
                }
            }
        }

        if let Some(idx) = self.deck_slot_edit.filter(|_| self.board_arrange) {
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("Pad {}", idx - base + 1)).family(theme::medium()),
                );
                egui::ComboBox::from_id_salt("deck_slot_phaser")
                    .selected_text(if self.deck_slot_phaser.is_empty() {
                        "(choose a phaser)"
                    } else {
                        &self.deck_slot_phaser
                    })
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for p in &self.phasers {
                            if ui.selectable_label(self.deck_slot_phaser == p.name, &p.name).clicked() {
                                self.deck_slot_phaser = p.name.clone();
                                self.deck_slot_color = p.color;
                                self.deck_slot_label = pad_symbol(&p.name);
                                self.deck_slot_icon = pad_icon(p);
                            }
                        }
                    });
                ui.color_edit_button_srgb(&mut self.deck_slot_color).on_hover_text("Pad colour");
                ui.add(
                    egui::TextEdit::singleline(&mut self.deck_slot_label)
                        .desired_width(52.0)
                        .char_limit(4)
                        .hint_text("symbol"),
                )
                .on_hover_text("Up to four characters, shown large on the pad and the key");
                egui::ComboBox::from_id_salt("deck_slot_icon")
                    .selected_text(self.deck_slot_icon.label())
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for icon in KeyIcon::ALL {
                            ui.selectable_value(&mut self.deck_slot_icon, icon, icon.label());
                        }
                    })
                    .response
                    .on_hover_text("Artwork drawn behind the symbol on the deck key");
                let can_save = !self.deck_slot_phaser.is_empty();
                if ui.add_enabled(can_save, egui::Button::new("Save")).clicked() {
                    if idx < self.phaser_deck.len() {
                        self.phaser_deck[idx] = Some(PhaserSlot {
                            phaser: self.deck_slot_phaser.clone(),
                            color: self.deck_slot_color,
                            label: self.deck_slot_label.clone(),
                            icon: self.deck_slot_icon,
                        });
                        save_phaser_deck(&self.phaser_deck);
                    }
                    self.deck_slot_edit = None;
                }
                if ui.button("Cancel").clicked() {
                    self.deck_slot_edit = None;
                }
            });
        }
    }

    /// Put library phaser `i` on the first empty pad — from the current page
    /// on, then earlier pages, then a new page — and show the board.
    pub(crate) fn board_add(&mut self, i: usize) {
        let Some(ph) = self.phasers.get(i).cloned() else { return };
        let start = self.deck_phaser_page * PHASER_DECK_SLOTS;
        let free = (start..self.phaser_deck.len())
            .chain(0..start)
            .find(|&k| self.phaser_deck[k].is_none());
        let k = match free {
            Some(k) => k,
            None => {
                self.add_phaser_deck_page();
                self.deck_phaser_page * PHASER_DECK_SLOTS
            }
        };
        self.phaser_deck[k] = Some(PhaserSlot {
            phaser: ph.name.clone(),
            color: ph.color,
            label: pad_symbol(&ph.name),
            icon: pad_icon(&ph),
        });
        save_phaser_deck(&self.phaser_deck);
        self.deck_phaser_page = k / PHASER_DECK_SLOTS;
        self.show_phaser_board = true;
        self.log.push(format!(
            "\"{}\" → board page {}, pad {}",
            ph.name,
            self.deck_phaser_page + 1,
            k % PHASER_DECK_SLOTS + 1
        ));
    }

    /// Whether library phaser `i` already has a pad.
    pub(crate) fn on_board(&self, i: usize) -> bool {
        self.phasers
            .get(i)
            .is_some_and(|ph| self.phaser_deck.iter().flatten().any(|s| s.phaser == ph.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_and_icons_come_from_the_name_and_channels() {
        assert_eq!(pad_symbol("Pan Slow 2"), "PANS");
        assert_eq!(pad_symbol("fx!"), "FX");
        let mut ph = Phaser::default();
        ph.targets = vec!["TILT".into()];
        assert_eq!(pad_icon(&ph), KeyIcon::UpDown);
        ph.targets = vec!["PAN".into(), "TILT".into()];
        assert_eq!(pad_icon(&ph), KeyIcon::None);
        ph.path = Some(PathShape::Circle);
        assert_eq!(pad_icon(&ph), KeyIcon::Ring);
    }

    /// Adding to the board takes the first free pad from the current page
    /// on, and grows the board by a page when every pad is taken.
    #[test]
    fn board_add_fills_free_pads_then_adds_a_page() {
        let mut app = crate::app::App::new();
        if app.phasers.is_empty() {
            app.phasers.push(Phaser { name: "Test wave".into(), ..Phaser::default() });
        }
        let name = app.phasers[0].name.clone();
        // Start from a board with exactly one page, one pad free.
        app.phaser_deck = vec![None; PHASER_DECK_SLOTS];
        for k in 1..PHASER_DECK_SLOTS {
            app.phaser_deck[k] = Some(PhaserSlot {
                phaser: "other".into(),
                color: [1, 2, 3],
                label: "OTHR".into(),
                icon: KeyIcon::None,
            });
        }
        app.deck_phaser_page = 0;
        app.board_add(0);
        assert_eq!(app.phaser_deck[0].as_ref().map(|s| s.phaser.as_str()), Some(name.as_str()));
        assert!(app.on_board(0));
        assert!(app.show_phaser_board);
        // Full now: the next add opens page 2 and lands on its first pad.
        app.board_add(0);
        assert_eq!(phaser_deck_pages(&app.phaser_deck), 2);
        assert_eq!(app.deck_phaser_page, 1);
        assert_eq!(
            app.phaser_deck[PHASER_DECK_SLOTS].as_ref().map(|s| s.phaser.as_str()),
            Some(name.as_str())
        );
    }
}
