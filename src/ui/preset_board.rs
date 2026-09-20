//! The Presets board: the pads you recall looks from.
//!
//! The Presets tab is where looks are stored and organised; this is where
//! they are played. It is the same 4 × 9 page of pads the Stream Deck's
//! Presets page shows — the same `preset_deck` slots, the same reserved
//! corners for select-all, Clear and Tap — so what is laid out here is
//! exactly what is under your fingers on the device, and a show without a
//! deck gets the same board on screen.
//!
//! Arrange mode turns the board into its own editor: click a pad to point
//! it at a preset (or a ShowBuddy bank preset), drag pads to swap them,
//! right-click to clear, and Fill… lays a whole folder out in one go. A
//! preset dragged out of the Presets tab lands on the pad it is dropped on.
//!
//! Pads hold preset *ids*, so renaming, reordering or deleting in the pool
//! never re-points a pad at the wrong look; a pad whose preset has gone
//! shows a muted `?` and says so when pressed.

use eframe::egui::{self, Color32, Sense, Vec2};

use super::board::{reserved_pad, COLS, GAP, ROWS};
use super::inspector::presets::{symbol_of, PadDrag, PadTarget, PresetDrag};
use super::theme;
use super::icons::Icon;
use crate::app::App;
use crate::preset_deck::{
    first_free_slot, preset_deck_pages, preset_pads_per_page, save_preset_deck, PresetSlot,
    PRESET_DECK_SLOTS,
};
use crate::streamdeck::{phaser_page_key, preset_key_look, KeyIcon, PhaserKey};

impl App {
    pub(crate) fn preset_board_window(&mut self, ctx: &egui::Context) {
        if !self.show_preset_board {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_preset_board;
        let mut popped = self.popped_out.contains("preset_board");
        let mut zoom_level = self.zoom.preset_board;
        super::floating_panel(
            ctx,
            "preset_board",
            "Presets board",
            &mut open,
            &mut popped,
            Some(&mut zoom_level),
            [600.0, 430.0],
            [screen.right() - 640.0, screen.bottom() - 470.0],
            |ui| self.preset_board_ui(ui),
        );
        self.zoom.preset_board = zoom_level;
        self.show_preset_board = open;
        if popped {
            self.popped_out.insert("preset_board");
        } else {
            self.popped_out.remove("preset_board");
        }
    }

    /// Which page of pads the window shows. The deck may be sitting on its
    /// extra ShowBuddy page; the window then shows the last board page.
    pub(crate) fn board_page_index(&self) -> usize {
        self.deck_preset_page.min(preset_deck_pages(&self.preset_deck) - 1)
    }

    /// The board: page bar, the pad grid, and in arrange mode the slot
    /// editor under it.
    pub(crate) fn preset_board_ui(&mut self, ui: &mut egui::Ui) {
        ui.push_id("preset_board", |ui| self.preset_board_body(ui));
    }

    fn preset_board_body(&mut self, ui: &mut egui::Ui) {
        self.refresh_preset_caches();
        let pages = preset_deck_pages(&self.preset_deck);
        let page = self.board_page_index();
        let base = page * PRESET_DECK_SLOTS;
        let arrange = self.insp.presets.board.arrange;
        let banked = !self.banks.is_empty();
        let on_bank_page = banked && self.deck_preset_page >= pages;

        let mut step = 0i32;
        let mut fill_folder: Option<String> = None;
        let mut fill_all = false;
        let mut clear_page = false;
        let mut add_page = false;
        let mut remove_page = false;
        // (name, how many presets are in it) — "" is the top level.
        let mut folders: Vec<(String, usize)> = vec![(
            String::new(),
            self.user_presets.iter().filter(|p| p.folder.is_empty()).count(),
        )];
        folders.extend(self.preset_folders.iter().map(|f| {
            (f.clone(), self.user_presets.iter().filter(|p| p.folder == *f).count())
        }));
        let page_gate = if pages > 1 { Ok(()) } else { Err("There is only one page.") };
        // Only the pads a key can reach count: a page whose hidden tail was
        // filled by an older build still reads as empty and can be removed.
        let page_empty = self.preset_deck[base..base + preset_pads_per_page()]
            .iter()
            .all(Option::is_none);
        theme::toolbar(ui, |ui| {
            if theme::tool_button(
                ui,
                Icon::ChevronLeft,
                None,
                "Show the previous page — the deck's Page knob does the same.",
                page_gate,
            )
            .clicked()
            {
                step = -1;
            }
            ui.label(
                egui::RichText::new(format!("Page {} / {pages}", page + 1))
                    .family(theme::medium()),
            );
            if theme::tool_button(
                ui,
                Icon::ChevronRight,
                None,
                "Show the next page — the deck's Page knob does the same.",
                page_gate,
            )
            .clicked()
            {
                step = 1;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::toggle_icon(
                    ui,
                    Icon::Drag,
                    Some("Arrange"),
                    arrange,
                    "Lay the board out: click a pad to point it at a preset, drag pads to swap \
                     them, right-click to clear one.",
                )
                .clicked()
                {
                    self.insp.presets.board.arrange = !arrange;
                    if arrange {
                        self.insp.presets.board.slot = None;
                    }
                }
                if arrange {
                    ui.menu_button("Fill…", |ui| {
                        ui.menu_button("Fill with a folder", |ui| {
                            for (name, n) in &folders {
                                let label = if name.is_empty() {
                                    format!("(top level)   {n}")
                                } else {
                                    format!("{name}   {n}")
                                };
                                if ui
                                    .add_enabled(*n > 0, egui::Button::new(label))
                                    .on_disabled_hover_text("Nothing is filed there yet.")
                                    .clicked()
                                {
                                    fill_folder = Some(name.clone());
                                    ui.close_menu();
                                }
                            }
                        });
                        if ui
                            .button("Fill with every preset")
                            .on_hover_text("Lay the whole pool out, skipping anything already on a pad.")
                            .clicked()
                        {
                            fill_all = true;
                            ui.close_menu();
                        }
                        if ui.button("Clear this page").clicked() {
                            clear_page = true;
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Add a page").clicked() {
                            add_page = true;
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(pages > 1 && page_empty, egui::Button::new("Remove this page"))
                            .on_hover_text("Only while the page is empty.")
                            .clicked()
                        {
                            remove_page = true;
                            ui.close_menu();
                        }
                    });
                }
            });
        });
        theme::hint(
            ui,
            if arrange {
                "Click a pad to point it at a preset · drag a pad onto another to swap them · \
                 right-click to clear · drop a preset from the Presets tab onto a pad"
            } else {
                "Press a pad to recall its look · ALL, CLR and TAP work as on the deck · 1–9 \
                 press the first pads"
            },
        );
        if banked {
            theme::hint(
                ui,
                if on_bank_page {
                    "The deck is on the ShowBuddy page — turn the Page knob or press ◀ to come back."
                } else {
                    "The deck shows the ShowBuddy banks as its last page."
                },
            );
        }
        ui.add_space(4.0);

        let avail = ui.available_width();
        let w = ((avail - GAP * (COLS as f32 - 1.0)) / COLS as f32).clamp(40.0, 120.0);
        let h = (w * 0.74).clamp(34.0, 92.0);
        let mut press: Option<usize> = None;
        let mut open_edit: Option<usize> = None;
        let mut clear_slot: Option<usize> = None;
        let mut swap: Option<(usize, usize)> = None;
        let mut drop_preset: Option<(usize, u32)> = None;
        let mut reserved: Option<PhaserKey> = None;
        let dragging = egui::DragAndDrop::payload::<PadDrag>(ui.ctx()).map(|p| p.0);
        let editing = self.insp.presets.board.slot;

        for row in 0..ROWS {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for col in 0..COLS {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
                    let k = row * COLS + col;
                    match phaser_page_key(k, ROWS, COLS) {
                        PhaserKey::Pad(slot) => {
                            let idx = base + slot;
                            let entry = self.preset_deck.get(idx).cloned().flatten();
                            let sense =
                                if arrange { Sense::click_and_drag() } else { Sense::click() };
                            let resp = ui.interact(rect, ui.id().with(("preset_pad", idx)), sense);
                            let lit = entry.as_ref().is_some_and(|s| self.preset_pad_lit(s));
                            let resolved =
                                entry.as_ref().is_some_and(|s| self.slot_resolves(s));
                            let drop_here = resp.dnd_hover_payload::<PresetDrag>().is_some()
                                || (arrange && dragging.is_some_and(|d| d != idx) && resp.hovered());
                            let (fill, symbol, name) = match &entry {
                                Some(s) if resolved => (
                                    Some(Color32::from_rgb(s.color[0], s.color[1], s.color[2])),
                                    s.label.clone(),
                                    s.name.clone(),
                                ),
                                Some(s) => (
                                    Some(
                                        Color32::from_rgb(s.color[0], s.color[1], s.color[2])
                                            .lerp_to_gamma(theme::SURFACE, 0.6),
                                    ),
                                    "?".to_owned(),
                                    s.name.clone(),
                                ),
                                None => (
                                    None,
                                    if arrange { "+".to_owned() } else { String::new() },
                                    String::new(),
                                ),
                            };
                            theme::paint_pad(
                                ui.painter(),
                                rect,
                                theme::PadState {
                                    fill,
                                    active: lit,
                                    editing: arrange && editing == Some(idx),
                                    hovered: resp.hovered() && (arrange || entry.is_some()),
                                    drop_target: drop_here,
                                    dim: !lit,
                                },
                                theme::PadFace::Symbol(&symbol),
                                &name,
                            );
                            if let Some(src) = resp.dnd_release_payload::<PresetDrag>() {
                                drop_preset = Some((idx, src.0));
                            }
                            if arrange {
                                if entry.is_some() && resp.drag_started() {
                                    egui::DragAndDrop::set_payload(ui.ctx(), PadDrag(idx));
                                }
                                if let Some(from) = resp.dnd_release_payload::<PadDrag>() {
                                    if from.0 != idx {
                                        swap = Some((from.0, idx));
                                    }
                                }
                                if resp.clicked() {
                                    open_edit = Some(idx);
                                }
                                let resp = resp.on_hover_text(match &entry {
                                    Some(s) => format!(
                                        "Pad {} — \"{}\". Click to change it, drag it onto \
                                         another pad to swap.",
                                        slot + 1,
                                        s.name
                                    ),
                                    None => format!(
                                        "Pad {} is empty. Click to put a preset on it.",
                                        slot + 1
                                    ),
                                });
                                if entry.is_some() {
                                    resp.context_menu(|ui| {
                                        if ui.button("Clear pad").clicked() {
                                            clear_slot = Some(idx);
                                            ui.close_menu();
                                        }
                                    });
                                }
                            } else if let Some(s) = &entry {
                                let hint = if !resolved {
                                    format!("Preset \"{}\" is no longer in the pool.", s.name)
                                } else if lit {
                                    format!("\"{}\" — lit · press to recall it again", s.name)
                                } else {
                                    format!("\"{}\" — press to recall it", s.name)
                                };
                                if resp.on_hover_text(hint).clicked() {
                                    press = Some(idx);
                                }
                            }
                        }
                        key => {
                            let (label, tint, active, hint) = match key {
                                PhaserKey::Tap => (
                                    "TAP",
                                    Color32::from_rgb(31, 111, 120),
                                    self.beat_taps
                                        .last()
                                        .is_some_and(|t| t.elapsed().as_secs_f32() < 0.15),
                                    "Tap the beat.",
                                ),
                                PhaserKey::Clear => (
                                    "CLR",
                                    Color32::from_rgb(160, 105, 40),
                                    false,
                                    "Clear: encoders, then effects, then blackout — one stage \
                                     per press.",
                                ),
                                _ => (
                                    "ALL",
                                    Color32::from_rgb(150, 128, 42),
                                    self.stage.all_selected(),
                                    "Select every light, or let go of them all.",
                                ),
                            };
                            if reserved_pad(ui, rect, k, label, tint, active)
                                .on_hover_text(hint)
                                .clicked()
                            {
                                reserved = Some(key);
                            }
                        }
                    }
                }
            });
            ui.add_space(GAP);
        }

        // 1–9 and 0 press the first ten pads of the page on show.
        if ui.ui_contains_pointer() && ui.ctx().memory(|m| m.focused().is_none()) {
            let keys = [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
                egui::Key::Num8,
                egui::Key::Num9,
                egui::Key::Num0,
            ];
            for (n, key) in keys.into_iter().enumerate() {
                if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
                    press = Some(base + n);
                }
            }
        }

        if step != 0 {
            self.step_preset_deck_page(step);
        }
        if let Some((a, b)) = swap {
            if a < self.preset_deck.len() && b < self.preset_deck.len() {
                self.preset_deck.swap(a, b);
                save_preset_deck(&self.preset_deck);
                if self.insp.presets.board.slot == Some(a) {
                    self.insp.presets.board.slot = Some(b);
                }
            }
        }
        if let Some((idx, id)) = drop_preset {
            if let Some(pi) = self.preset_index_by_id(id) {
                if let Some(slot) = self.preset_slot_for_user(pi) {
                    let name = slot.name.clone();
                    if idx < self.preset_deck.len() {
                        self.preset_deck[idx] = Some(slot);
                        save_preset_deck(&self.preset_deck);
                        self.log.push(format!(
                            "\"{name}\" → Presets board page {}, pad {}",
                            idx / PRESET_DECK_SLOTS + 1,
                            idx % PRESET_DECK_SLOTS + 1
                        ));
                    }
                }
            }
        }
        if let Some(idx) = clear_slot {
            if idx < self.preset_deck.len() && self.preset_deck[idx].is_some() {
                self.preset_deck[idx] = None;
                save_preset_deck(&self.preset_deck);
                self.log.push(format!("Cleared board pad {}", idx % PRESET_DECK_SLOTS + 1));
            }
            if self.insp.presets.board.slot == Some(idx) {
                self.insp.presets.board.slot = None;
            }
        }
        if let Some(idx) = press {
            self.press_preset_pad(idx);
        }
        match reserved {
            Some(PhaserKey::Tap) => self.beat_tap(),
            Some(PhaserKey::Clear) => self.clear_stage(),
            Some(PhaserKey::SelectAll) => {
                let on = self.stage.select_all_fixtures();
                self.sync_selection_units();
                self.sel_fixture = self.stage.last_selected;
                self.log.push(if on {
                    format!("Selected all {} lights", self.patch.fixtures.len())
                } else {
                    "Selection cleared".into()
                });
            }
            _ => {}
        }
        if let Some(idx) = open_edit {
            self.open_slot_editor(idx);
        }
        if fill_all {
            self.fill_preset_page_all();
        }
        if let Some(f) = fill_folder {
            self.fill_preset_page_with_folder(&f);
        }
        if clear_page {
            self.clear_preset_page();
        }
        if add_page {
            self.add_preset_deck_page();
        }
        if remove_page {
            self.remove_preset_deck_page();
        }
        self.preset_slot_editor(ui, base);
    }

    /// Open the slot editor on `idx`, pre-filled from the pad (or from the
    /// first preset when the pad is empty).
    fn open_slot_editor(&mut self, idx: usize) {
        let slot = self.preset_deck.get(idx).cloned().flatten();
        let first = self.preset_slot_for_user(0);
        let board = &mut self.insp.presets.board;
        board.slot = Some(idx);
        match slot {
            Some(s) => {
                board.target = match s.bank {
                    Some((b, n)) => PadTarget::Bank(b, n),
                    None => PadTarget::User(s.preset),
                };
                board.color = s.color;
                board.label = s.label;
                board.icon = s.icon;
            }
            None => match first {
                Some(s) => {
                    board.target = PadTarget::User(s.preset);
                    board.color = s.color;
                    board.label = s.label;
                    board.icon = s.icon;
                }
                None => {
                    board.target = PadTarget::None;
                    board.color = [60, 60, 70];
                    board.label.clear();
                    board.icon = KeyIcon::None;
                }
            },
        }
    }

    /// The row under the grid that points one pad at a look.
    fn preset_slot_editor(&mut self, ui: &mut egui::Ui, base: usize) {
        // The pad is an absolute deck index, so an editor left open while the
        // view moves to another page belongs to neither this page's grid nor
        // its "Pad n" label — `idx - base` would run off the bottom of usize.
        let Some(idx) = self
            .insp
            .presets
            .board
            .slot
            .filter(|_| self.insp.presets.board.arrange)
            .filter(|idx| (base..base + PRESET_DECK_SLOTS).contains(idx))
        else {
            return;
        };
        let z = theme::zoom_of(ui);
        // Everything the combo lists, read before the closures.
        let natives: Vec<(u32, String, String, [u8; 3], String, KeyIcon)> = self
            .user_presets
            .iter()
            .map(|p| {
                (p.id, p.name.clone(), p.folder.clone(), self.preset_face(p), symbol_of(p), p.icon)
            })
            .collect();
        let banked: Vec<(usize, usize, String, String)> = self
            .banks
            .iter()
            .enumerate()
            .flat_map(|(bi, b)| {
                b.presets
                    .iter()
                    .enumerate()
                    .map(move |(pi, p)| (bi, pi, b.name.clone(), p.name.clone()))
            })
            .collect();
        let selected = match &self.insp.presets.board.target {
            PadTarget::None => "(choose a preset)".to_owned(),
            PadTarget::User(id) => natives
                .iter()
                .find(|(i, ..)| i == id)
                .map_or_else(|| "(gone)".to_owned(), |(_, n, ..)| n.clone()),
            PadTarget::Bank(b, n) => format!("{b} · {n}"),
        };
        let mut save = false;
        let mut cancel = false;
        let mut pick_bank: Option<(usize, usize)> = None;
        ui.add_space(4.0);
        theme::card(ui, |ui| {
            let board = &mut self.insp.presets.board;
            theme::card_title(ui, &format!("Pad {}", idx % PRESET_DECK_SLOTS + 1), |ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(52.0 * z, 34.0 * z), Sense::hover());
                theme::paint_pad(
                    ui.painter(),
                    r,
                    theme::PadState {
                        fill: Some(Color32::from_rgb(
                            board.color[0],
                            board.color[1],
                            board.color[2],
                        )),
                        ..theme::PadState::default()
                    },
                    theme::PadFace::Symbol(&board.label),
                    "",
                );
            });
            theme::toolbar(ui, |ui| {
                egui::ComboBox::from_id_salt("preset_slot_target")
                    .selected_text(selected)
                    .width(190.0 * z)
                    .show_ui(ui, |ui| {
                        for (id, name, folder, face, symbol, icon) in &natives {
                            let on = board.target == PadTarget::User(*id);
                            let text = if folder.is_empty() {
                                name.clone()
                            } else {
                                format!("{name}   ({folder})")
                            };
                            if ui.selectable_label(on, text).clicked() {
                                board.target = PadTarget::User(*id);
                                board.color = *face;
                                board.label = symbol.clone();
                                board.icon = *icon;
                            }
                        }
                        if !banked.is_empty() {
                            ui.separator();
                            for (bi, pi, bank, name) in &banked {
                                let on = board.target
                                    == PadTarget::Bank(bank.clone(), name.clone());
                                if ui.selectable_label(on, format!("{bank} · {name}")).clicked() {
                                    board.target = PadTarget::Bank(bank.clone(), name.clone());
                                    pick_bank = Some((*bi, *pi));
                                }
                            }
                        }
                    })
                    .response
                    .on_hover_text("The look this pad recalls.");
                ui.color_edit_button_srgb(&mut board.color).on_hover_text("The pad's colour.");
                ui.add(
                    egui::TextEdit::singleline(&mut board.label)
                        .id_salt("preset_slot_label")
                        .desired_width(52.0 * z)
                        .char_limit(4)
                        .hint_text("symbol"),
                )
                .on_hover_text("Up to four characters, shown large on the pad and the deck key.");
                egui::ComboBox::from_id_salt("preset_slot_icon")
                    .selected_text(board.icon.label())
                    .width(120.0 * z)
                    .show_ui(ui, |ui| {
                        for icon in KeyIcon::ALL {
                            ui.selectable_value(&mut board.icon, icon, icon.label());
                        }
                    })
                    .response
                    .on_hover_text("Artwork drawn behind the symbol on the deck key.");
                let gate = if board.target == PadTarget::None {
                    Err("Choose a preset for this pad first.")
                } else {
                    Ok(())
                };
                // `gated` builds its child in a `Ui`, which cannot wrap; in a
                // toolbar the gate belongs to the button itself.
                if theme::tool_button(
                    ui,
                    Icon::Check,
                    Some("Save"),
                    "Write this pad to the board and the deck.",
                    gate,
                )
                .clicked()
                {
                    save = true;
                }
                if ui.button("Cancel").on_hover_text("Leave the pad as it was.").clicked() {
                    cancel = true;
                }
            });
        });
        if let Some((bi, pi)) = pick_bank {
            let face = self.bank_face(bi, pi);
            let name = self.banks[bi].presets[pi].name.clone();
            let (_, label, icon) = preset_key_look(&name);
            let board = &mut self.insp.presets.board;
            board.color = face;
            board.label = label;
            board.icon = icon;
        }
        if save {
            self.save_slot_editor(idx);
        }
        if save || cancel {
            self.insp.presets.board.slot = None;
        }
    }

    /// Write the slot editor's pad.
    fn save_slot_editor(&mut self, idx: usize) {
        if idx >= self.preset_deck.len() {
            return;
        }
        let board = &self.insp.presets.board;
        let slot = match &board.target {
            PadTarget::None => return,
            PadTarget::User(id) => {
                let name = self
                    .user_presets
                    .iter()
                    .find(|p| p.id == *id)
                    .map_or_else(String::new, |p| p.name.clone());
                PresetSlot {
                    preset: *id,
                    bank: None,
                    name,
                    color: board.color,
                    label: board.label.clone(),
                    icon: board.icon,
                }
            }
            PadTarget::Bank(bank, name) => PresetSlot {
                preset: 0,
                bank: Some((bank.clone(), name.clone())),
                name: name.clone(),
                color: board.color,
                label: board.label.clone(),
                icon: board.icon,
            },
        };
        let name = slot.name.clone();
        self.preset_deck[idx] = Some(slot);
        save_preset_deck(&self.preset_deck);
        self.log.push(format!(
            "\"{name}\" → Presets board page {}, pad {}",
            idx / PRESET_DECK_SLOTS + 1,
            idx % PRESET_DECK_SLOTS + 1
        ));
    }

    /// A pad for native preset `idx`, as it would be placed today.
    pub(crate) fn preset_slot_for_user(&self, idx: usize) -> Option<PresetSlot> {
        let p = self.user_presets.get(idx)?;
        Some(PresetSlot {
            preset: p.id,
            bank: None,
            name: p.name.clone(),
            color: self.preset_face(p),
            label: symbol_of(p),
            icon: p.icon,
        })
    }

    /// A pad for a ShowBuddy bank preset.
    pub(crate) fn preset_slot_for_bank(&self, bi: usize, pi: usize) -> Option<PresetSlot> {
        let bank = self.banks.get(bi)?;
        let p = bank.presets.get(pi)?;
        let (_, label, icon) = preset_key_look(&p.name);
        Some(PresetSlot {
            preset: 0,
            bank: Some((bank.name.clone(), p.name.clone())),
            name: p.name.clone(),
            color: self.bank_face(bi, pi),
            label,
            icon,
        })
    }

    /// Whether a pad still points at something that exists.
    fn slot_resolves(&self, slot: &PresetSlot) -> bool {
        match &slot.bank {
            Some((bank, name)) => self.find_bank_preset(bank, name).is_some(),
            None => self.preset_index_by_id(slot.preset).is_some(),
        }
    }

    /// A ShowBuddy preset by the names a pad stored.
    fn find_bank_preset(&self, bank: &str, name: &str) -> Option<(usize, usize)> {
        let bi = self.banks.iter().position(|b| b.name == bank)?;
        let pi = self.banks[bi].presets.iter().position(|p| p.name == name)?;
        Some((bi, pi))
    }

    /// Whether the look a pad points at is the one that is lit.
    pub(crate) fn preset_pad_lit(&self, slot: &PresetSlot) -> bool {
        match &slot.bank {
            Some((bank, name)) => {
                self.find_bank_preset(bank, name).is_some_and(|k| self.active_preset == Some(k))
            }
            None => self
                .active_user_preset
                .and_then(|i| self.user_presets.get(i))
                .is_some_and(|p| p.id == slot.preset),
        }
    }

    /// Press a board pad: recall what it points at, or say why it cannot.
    pub(crate) fn press_preset_pad(&mut self, idx: usize) {
        let Some(slot) = self.preset_deck.get(idx).cloned().flatten() else { return };
        match &slot.bank {
            Some((bank, name)) => match self.find_bank_preset(bank, name) {
                Some((bi, pi)) => self.recall_bank_preset(bi, pi, None),
                None => self.log.push(format!(
                    "Pad \"{}\": ShowBuddy preset \"{name}\" is not in the banks any more",
                    slot.label
                )),
            },
            None => match self.preset_index_by_id(slot.preset) {
                Some(i) => self.recall_user_preset(i, None),
                None => self.log.push(format!(
                    "Pad \"{}\": preset \"{}\" is no longer in the pool",
                    slot.label, slot.name
                )),
            },
        }
    }

    /// Put native preset `idx` on the first free pad — from the page on
    /// show, then earlier pages, then a new page — and show the board.
    pub(crate) fn preset_board_add(&mut self, idx: usize) {
        let Some(slot) = self.preset_slot_for_user(idx) else { return };
        self.place_on_board(slot);
    }

    /// The same for a ShowBuddy bank preset.
    pub(crate) fn preset_board_add_bank(&mut self, bi: usize, pi: usize) {
        let Some(slot) = self.preset_slot_for_bank(bi, pi) else { return };
        self.place_on_board(slot);
    }

    fn place_on_board(&mut self, slot: PresetSlot) {
        let page = self.board_page_index();
        let k = match first_free_slot(&self.preset_deck, page) {
            Some(k) => k,
            None => {
                self.add_preset_deck_page();
                self.board_page_index() * PRESET_DECK_SLOTS
            }
        };
        let name = slot.name.clone();
        self.preset_deck[k] = Some(slot);
        save_preset_deck(&self.preset_deck);
        self.deck_preset_page = k / PRESET_DECK_SLOTS;
        self.show_preset_board = true;
        self.log.push(format!(
            "\"{name}\" → Presets board page {}, pad {}",
            self.deck_preset_page + 1,
            k % PRESET_DECK_SLOTS + 1
        ));
    }

    /// Whether native preset `id` already has a pad you can press. A slot
    /// with no key on it does not count, or the preset would be reported as
    /// on the board while being nowhere on it.
    pub(crate) fn preset_on_board(&self, id: u32) -> bool {
        let pads = preset_pads_per_page();
        self.preset_deck.iter().enumerate().any(|(k, s)| {
            k % PRESET_DECK_SLOTS < pads
                && s.as_ref().is_some_and(|s| s.bank.is_none() && s.preset == id)
        })
    }

    /// How many pages the deck's Presets page steps through: the board's,
    /// plus one for the ShowBuddy banks when there are any.
    pub(crate) fn preset_deck_page_count(&self) -> usize {
        preset_deck_pages(&self.preset_deck) + usize::from(!self.banks.is_empty())
    }

    /// Scroll the Presets page by `d` pages, wrapping at either end.
    pub(crate) fn step_preset_deck_page(&mut self, d: i32) {
        let count = self.preset_deck_page_count() as i32;
        self.deck_preset_page = (self.deck_preset_page as i32 + d).rem_euclid(count) as usize;
    }

    /// Grow the board by one page and show it.
    pub(crate) fn add_preset_deck_page(&mut self) {
        self.preset_deck.resize(self.preset_deck.len() + PRESET_DECK_SLOTS, None);
        save_preset_deck(&self.preset_deck);
        self.deck_preset_page = preset_deck_pages(&self.preset_deck) - 1;
        self.log.push(format!("Presets board: added page {}", self.deck_preset_page + 1));
    }

    /// Drop the page on show, which has to be empty.
    pub(crate) fn remove_preset_deck_page(&mut self) {
        let pages = preset_deck_pages(&self.preset_deck);
        if pages < 2 {
            return;
        }
        let page = self.board_page_index();
        let base = page * PRESET_DECK_SLOTS;
        if self.preset_deck[base..base + preset_pads_per_page()].iter().any(Option::is_some) {
            return;
        }
        self.preset_deck.drain(base..base + PRESET_DECK_SLOTS);
        save_preset_deck(&self.preset_deck);
        self.deck_preset_page = page.min(preset_deck_pages(&self.preset_deck) - 1);
        self.log.push(format!("Presets board: removed page {}", page + 1));
    }

    /// Empty every pad of the page on show.
    pub(crate) fn clear_preset_page(&mut self) {
        let base = self.board_page_index() * PRESET_DECK_SLOTS;
        if self.preset_deck[base..base + PRESET_DECK_SLOTS].iter().all(Option::is_none) {
            return;
        }
        for slot in &mut self.preset_deck[base..base + PRESET_DECK_SLOTS] {
            *slot = None;
        }
        save_preset_deck(&self.preset_deck);
        self.log
            .push(format!("Presets board: cleared page {}", self.board_page_index() + 1));
    }

    /// Lay a folder's presets out on the page on show, skipping any that
    /// already have a pad and growing the board when it runs out of room.
    pub(crate) fn fill_preset_page_with_folder(&mut self, folder: &str) {
        let idxs: Vec<usize> = (0..self.user_presets.len())
            .filter(|&i| self.user_presets[i].folder == folder)
            .collect();
        let n = self.fill_page_from(&idxs);
        let where_from =
            if folder.is_empty() { "the top level".to_owned() } else { format!("folder \"{folder}\"") };
        self.log.push(format!(
            "Presets board: filled page {} with {where_from} ({n} pads)",
            self.board_page_index() + 1
        ));
    }

    /// The same over the whole pool.
    pub(crate) fn fill_preset_page_all(&mut self) {
        let idxs: Vec<usize> = (0..self.user_presets.len()).collect();
        let n = self.fill_page_from(&idxs);
        self.log.push(format!(
            "Presets board: filled page {} with {n} presets",
            self.board_page_index() + 1
        ));
    }

    /// Place every one of `idxs` that is not on the board yet, from the
    /// page on show onwards. Returns how many pads were written.
    fn fill_page_from(&mut self, idxs: &[usize]) -> usize {
        let page = self.board_page_index();
        let pads = preset_pads_per_page();
        let mut placed = 0;
        let mut k = page * PRESET_DECK_SLOTS;
        for &i in idxs {
            let Some(slot) = self.preset_slot_for_user(i) else { continue };
            if self.preset_on_board(slot.preset) {
                continue;
            }
            // Step over taken pads and over the slots no key reaches.
            while k < self.preset_deck.len()
                && (self.preset_deck[k].is_some() || k % PRESET_DECK_SLOTS >= pads)
            {
                k += 1;
            }
            if k >= self.preset_deck.len() {
                break;
            }
            self.preset_deck[k] = Some(slot);
            placed += 1;
        }
        if placed > 0 {
            save_preset_deck(&self.preset_deck);
        }
        placed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset_deck::load_preset_deck;
    use crate::showbuddy::{PresetBank, PresetData, PresetRef};

    /// The tests here write `preset_deck.json` (as the Phaser board's do
    /// with `phaser_deck.json`), so each one puts the file back. It is the
    /// copy in the run's scratch directory — see `crate::paths`.
    struct DeckFile(Option<Vec<u8>>);

    impl DeckFile {
        fn path() -> std::path::PathBuf {
            crate::paths::data_path("preset_deck.json")
        }

        fn snapshot() -> Self {
            Self(std::fs::read(Self::path()).ok())
        }
    }

    impl Drop for DeckFile {
        fn drop(&mut self) {
            match &self.0 {
                Some(bytes) => {
                    let _ = std::fs::write(Self::path(), bytes);
                }
                None => {
                    let _ = std::fs::remove_file(Self::path());
                }
            }
        }
    }

    fn app_with_one_preset() -> App {
        let mut app = App::new();
        app.user_presets.clear();
        app.next_preset_id = 1;
        app.user_presets.push(crate::preset::UserPreset {
            id: 1,
            name: "Verse blue".into(),
            folder: String::new(),
            values: vec![(0, 200)],
            oscs: Vec::new(),
            speed: 0.3,
            tempo: 120.0,
            master_speed: 1.0,
            color: Some([10, 90, 200]),
            symbol: String::new(),
            icon: KeyIcon::None,
            fade: None,
            pinned: false,
        });
        app.next_preset_id = 2;
        app.preset_deck = vec![None; PRESET_DECK_SLOTS];
        app.deck_preset_page = 0;
        app
    }

    /// Adding to the board takes the first free pad from the page on show,
    /// and grows the board by a page when every pad is taken.
    #[test]
    fn preset_board_add_fills_free_pads_then_adds_a_page() {
        let _guard = DeckFile::snapshot();
        let mut app = app_with_one_preset();
        let filler = PresetSlot {
            preset: 999,
            bank: None,
            name: "other".into(),
            color: [1, 2, 3],
            label: "OTHR".into(),
            icon: KeyIcon::None,
        };
        for k in 1..PRESET_DECK_SLOTS {
            app.preset_deck[k] = Some(filler.clone());
        }
        app.preset_board_add(0);
        assert_eq!(app.preset_deck[0].as_ref().map(|s| s.preset), Some(1));
        assert!(app.preset_on_board(1));
        assert!(app.show_preset_board);
        // Full now: the next add opens page 2 and lands on its first pad.
        app.user_presets[0].id = 2;
        app.preset_board_add(0);
        assert_eq!(preset_deck_pages(&app.preset_deck), 2);
        assert_eq!(app.deck_preset_page, 1);
        assert_eq!(app.preset_deck[PRESET_DECK_SLOTS].as_ref().map(|s| s.preset), Some(2));
        // What was written is what a reload reads back.
        assert_eq!(load_preset_deck().len(), app.preset_deck.len());
    }

    /// A page stores 36 slots and shows `preset_pads_per_page` of them. Add
    /// used to walk into the three with no key on them: the log said "pad
    /// 34", the board showed nothing, and the preset was then reported as
    /// already on the board and could never be placed.
    #[test]
    fn add_to_board_skips_the_slots_no_key_reaches() {
        let _guard = DeckFile::snapshot();
        let mut app = app_with_one_preset();
        let pads = preset_pads_per_page();
        let filler = PresetSlot {
            preset: 999,
            bank: None,
            name: "other".into(),
            color: [1, 2, 3],
            label: "OTHR".into(),
            icon: KeyIcon::None,
        };
        for k in 0..pads {
            app.preset_deck[k] = Some(filler.clone());
        }
        app.preset_board_add(0);
        assert!(
            app.preset_deck[pads..PRESET_DECK_SLOTS].iter().all(Option::is_none),
            "a pad landed on a slot with no key on it"
        );
        assert_eq!(preset_deck_pages(&app.preset_deck), 2);
        assert_eq!(app.deck_preset_page, 1);
        assert_eq!(app.preset_deck[PRESET_DECK_SLOTS].as_ref().map(|s| s.preset), Some(1));
        assert!(app.preset_on_board(1));
        assert!(app.log.last().is_some_and(|l| l.contains("page 2, pad 1")), "{:?}", app.log.last());
        // A pad an older build parked on an unreachable slot is not on the
        // board as far as anything the operator can press is concerned.
        app.preset_deck[PRESET_DECK_SLOTS] = None;
        app.preset_deck[pads] = app.preset_slot_for_user(0);
        assert!(!app.preset_on_board(1));
    }

    /// The slot editor holds an absolute deck index, so a page change under
    /// it (◀ / ▶, or Fill… → "Add a page") left it pointing at a pad on
    /// another page: `idx - base` underflowed `usize` and the debug build
    /// died. It now simply stays out of the way until its page is back.
    #[test]
    fn the_slot_editor_stands_down_when_its_page_is_not_the_one_on_show() {
        let _guard = DeckFile::snapshot();
        let mut app = app_with_one_preset();
        app.preset_deck = vec![None; 2 * PRESET_DECK_SLOTS];
        app.insp.presets.board.arrange = true;
        app.insp.presets.board.slot = Some(3);
        app.insp.presets.board.target = PadTarget::User(1);
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut heights = Vec::new();
        // The editor's page first, then the one after it; a warm-up frame
        // each, since the console's fonts bind a frame late.
        for page in [0usize, 0, 1, 1] {
            app.deck_preset_page = page;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(900.0, 800.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let top = ui.cursor().min.y;
                    app.preset_board_ui(ui);
                    heights.push(ui.cursor().min.y - top);
                });
            });
        }
        assert!(heights[1] > heights[3], "the editor drew on the wrong page: {heights:?}");
        // It is only hidden, not thrown away: page back and it is there.
        assert_eq!(app.insp.presets.board.slot, Some(3));
    }

    /// A pad whose preset has gone says so and changes nothing.
    #[test]
    fn press_missing_preset_only_logs() {
        let _guard = DeckFile::snapshot();
        let mut app = app_with_one_preset();
        app.active_user_preset = None;
        app.preset_deck[0] = Some(PresetSlot {
            preset: 99,
            bank: None,
            name: "Gone".into(),
            color: [1, 2, 3],
            label: "GONE".into(),
            icon: KeyIcon::None,
        });
        app.press_preset_pad(0);
        assert_eq!(app.active_user_preset, None);
        assert!(app.log.last().is_some_and(|l| l.contains("no longer in the pool")), "{:?}", app.log.last());
    }

    /// A bank pad finds its preset by name, wherever the bank has moved to.
    #[test]
    fn bank_pad_resolves_by_name() {
        let _guard = DeckFile::snapshot();
        let mut app = app_with_one_preset();
        app.banks = vec![PresetBank {
            name: "B".into(),
            order: 0,
            presets: vec![PresetRef {
                name: "P".into(),
                path: std::path::PathBuf::new(),
                data: Some(PresetData {
                    values: vec![(1, 255)],
                    mods: Vec::new(),
                    master_speed: 1.0,
                    speed: 0.3,
                    shape: 0.0,
                    tempo: 120.0,
                }),
            }],
        }];
        app.transition.duration = 0.0;
        app.preset_deck[0] = app.preset_slot_for_bank(0, 0);
        assert!(app.preset_deck[0].as_ref().is_some_and(|s| s.bank.is_some()));
        app.press_preset_pad(0);
        assert_eq!(app.active_preset, Some((0, 0)));
        assert!(app.preset_pad_lit(app.preset_deck[0].as_ref().unwrap()));
    }

    /// Fill skips what is already on the board and stops when the page is
    /// full; a second fill finds nothing left to do.
    #[test]
    fn fill_folder_skips_pads_already_on_board_and_stops_when_full() {
        let _guard = DeckFile::snapshot();
        let mut app = app_with_one_preset();
        let base = app.user_presets[0].clone();
        app.user_presets.clear();
        for i in 0..40 {
            let mut p = base.clone();
            p.id = i + 1;
            p.name = format!("P{i}");
            p.folder = "X".into();
            app.user_presets.push(p);
        }
        app.preset_folders = vec!["X".into()];
        app.next_preset_id = 41;
        app.fill_preset_page_with_folder("X");
        let filled = app.preset_deck.iter().filter(|s| s.is_some()).count();
        // A page stores 36 slots but only shows `preset_pads_per_page` of
        // them, and a fill stops at the last one a key can reach.
        assert_eq!(filled, preset_pads_per_page());
        let want = format!("{} pads", preset_pads_per_page());
        assert!(app.log.last().is_some_and(|l| l.contains(&want)), "{:?}", app.log.last());
        app.fill_preset_page_with_folder("X");
        assert_eq!(app.preset_deck.iter().filter(|s| s.is_some()).count(), preset_pads_per_page());
        assert!(app.log.last().is_some_and(|l| l.contains("0 pads")));
    }

    /// The Page knob wraps over the board's pages and the banks' one.
    #[test]
    fn page_knob_wraps_over_board_and_bank_pages() {
        let _guard = DeckFile::snapshot();
        let mut app = app_with_one_preset();
        app.banks.clear();
        app.preset_deck = vec![None; 2 * PRESET_DECK_SLOTS];
        app.deck_preset_page = 0;
        assert_eq!(app.preset_deck_page_count(), 2);
        app.step_preset_deck_page(1);
        assert_eq!(app.deck_preset_page, 1);
        app.step_preset_deck_page(1);
        assert_eq!(app.deck_preset_page, 0);
        app.banks = vec![PresetBank { name: "B".into(), order: 0, presets: Vec::new() }];
        assert_eq!(app.preset_deck_page_count(), 3);
        app.deck_preset_page = 2;
        // The window falls back to the last board page while the deck sits
        // on the banks.
        assert_eq!(app.board_page_index(), 1);
        app.step_preset_deck_page(1);
        assert_eq!(app.deck_preset_page, 0);
        app.step_preset_deck_page(-1);
        assert_eq!(app.deck_preset_page, 2);
    }
}
