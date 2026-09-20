//! The Layers window — the stack of programmer layers and what is on each.
//!
//! Layers read top-down here, highest priority first, because that is how an
//! operator thinks about a stack: the thing at the top is the thing you see.
//! `App::layers` stores them the other way up (bottom→top, the order the
//! mixer folds them), so this panel walks it in reverse.
//!
//! Each layer shows what has been applied to it as boxes, most far-reaching
//! first — preset, then palettes, then phasers. Clicking a box drops it and
//! hands its channels back to whatever sits underneath; shift-clicking marks
//! several to drop together.

use egui::{Color32, Sense, Stroke};

use super::{raid, theme};
use crate::app::App;
use crate::layer::BoxKind;
use crate::transition::TransitionTarget;

/// Effect boxes are raid tiles at well under full size: small enough that a
/// layer's whole makeup fits on one or two rows, big enough to still read.
const BOX_ZOOM: f32 = 0.88;

/// What a row in the panel asked for, applied after the loop so the stack is
/// never mutated while it is being drawn.
enum LayerAction {
    Select(usize),
    Raise(usize),
    Lower(usize),
    Remove(usize),
    Order(usize, Option<u32>),
    /// Drop these boxes from that layer and give their channels back.
    DropBoxes(usize, Vec<usize>),
    /// ⇧ click: mark a box to drop with others rather than dropping it now.
    MarkBox(usize, usize),
    /// Let go of a forced hold or flat add sitting above the whole stack.
    StopAbove(String),
}

impl App {
    /// Colour for a box, taken from the thing it stands for wherever that
    /// has one of its own — a phaser's pool colour and a palette's swatch are
    /// already how the operator recognises them in every other window, so a
    /// layer's makeup reads without a legend. Presets have no colour of
    /// their own and fall back to the accent.
    fn box_color(&self, kind: &BoxKind) -> Color32 {
        match kind {
            BoxKind::Preset(_) | BoxKind::BankPreset(..) => theme::ACCENT_SOFT,
            BoxKind::Palette(id) => self
                .palettes
                .iter()
                .find(|p| p.id == *id)
                .map(|p| self.palette_swatch(p))
                .unwrap_or(theme::OK),
            BoxKind::Phaser(name) => self
                .phasers
                .iter()
                .find(|p| p.name == *name)
                .map(|p| Color32::from_rgb(p.color[0], p.color[1], p.color[2]))
                .unwrap_or(theme::WARN),
        }
    }

    pub(crate) fn layers_window(&mut self, ctx: &egui::Context) {
        if !self.show_layers {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_layers;
        let mut popped = self.popped_out.contains("layers");
        let mut act: Option<LayerAction> = None;
        let mut do_add = false;

        let fade = self.transition.fade(TransitionTarget::LayerRelease);
        let sel = self.active_layer;
        let order_names: Vec<(u32, String)> = self
            .orders
            .iter()
            .map(|o| (o.id, o.name.clone()))
            .collect();
        // Drawn top-down, so the highest-priority layer is the first row.
        let rows: Vec<usize> = (0..self.layers.len()).rev().collect();
        let count = self.layers.len();

        // Resolved up front: the panel closure borrows the layer stack, and
        // both of these read other pools.
        let look = self.settings.raid_look;
        let swatches: Vec<Vec<Color32>> = self
            .layers
            .iter()
            .map(|l| l.boxes.iter().map(|b| self.box_color(&b.kind)).collect())
            .collect();
        let above: Vec<(String, String, usize, Color32)> = self
            .above_stack
            .iter()
            .map(|b| {
                (
                    b.label.clone(),
                    b.kind.short_tag().to_string(),
                    b.addrs.len(),
                    self.box_color(&b.kind),
                )
            })
            .collect();

        let mut zoom_level = self.zoom.layers;
        super::floating_panel(
            ctx,
            "layers",
            "Layers",
            &mut open,
            &mut popped,
            Some(&mut zoom_level),
            [420.0, 460.0],
            [screen.right() - 460.0, 120.0],
            |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .button("+ Layer")
                        .on_hover_text(
                            "Add an empty layer above the selected one and program into it",
                        )
                        .clicked()
                    {
                        do_add = true;
                    }
                    ui.separator();
                    theme::hint(
                        ui,
                        if fade > 0.01 {
                            format!("release fades over {fade:.1}s")
                        } else {
                            "release is instant".to_string()
                        },
                    );
                });
                theme::hint(
                    ui,
                    "Higher layers take the channels they assert; everything else falls \
                     through to the layer below. Phaser motion sums across layers.",
                );
                ui.separator();

                // Forced holds and flat adds are stamped on the flat frame
                // after the mixer and after the grand master, so no layer can
                // reach them. They get their own place above the stack rather
                // than a box inside a layer, which would imply otherwise.
                if !above.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        theme::pill(ui, "ABOVE EVERYTHING", theme::WARN);
                        theme::hint(ui, "forced on until stopped — no layer, blackout or master reaches these");
                    });
                    let tile = raid::tile_size(look, BOX_ZOOM);
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    let painter = ui.painter().clone();
                    ui.horizontal_wrapped(|ui| {
                        for (label, tag, chans, c) in &above {
                            let (rect, r) = ui.allocate_exact_size(tile, Sense::click());
                            raid::paint(
                                &painter,
                                look,
                                rect,
                                BOX_ZOOM,
                                &raid::Tile {
                                    name: label,
                                    addr: &format!("{tag} {chans}"),
                                    color: *c,
                                    level: 1.0,
                                    selected: false,
                                },
                            );
                            let r = r.on_hover_text(format!(
                                "{label} — forcing {chans} channel(s) over everything.\nClick to let go."
                            ));
                            if r.clicked() {
                                act = Some(LayerAction::StopAbove(label.clone()));
                            }
                        }
                    });
                    ui.separator();
                }

                for &i in &rows {
                    let l = &self.layers[i];
                    let is_sel = i == sel;
                    let tint = if is_sel { theme::ACCENT } else { theme::darken(theme::ACCENT, 0.6) };
                    egui::Frame::none()
                        .fill(if is_sel {
                            theme::ACCENT.gamma_multiply(0.10)
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(Stroke::new(if is_sel { 1.2 } else { 0.6 }, tint.gamma_multiply(0.7)))
                        .rounding(egui::Rounding::same(5.0))
                        .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                // The stack position, counted the way it reads.
                                theme::pill(ui, &format!("{}", i + 1), tint);
                                let name = egui::RichText::new(&l.name)
                                    .family(theme::semibold())
                                    .size(13.0);
                                if ui
                                    .add(egui::Label::new(name).sense(Sense::click()))
                                    .on_hover_text("Program into this layer")
                                    .clicked()
                                {
                                    act = Some(LayerAction::Select(i));
                                }
                                if is_sel {
                                    theme::pill(ui, "PROGRAMMING", theme::ACCENT_SOFT);
                                }
                                if l.leaving.is_some() {
                                    theme::pill(ui, "RELEASING", theme::WARN);
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add_enabled(count > 1, egui::Button::new("Out"))
                                            .on_hover_text("Take this layer out")
                                            .clicked()
                                        {
                                            act = Some(LayerAction::Remove(i));
                                        }
                                        if ui
                                            .add_enabled(i + 1 < count, egui::Button::new("▲"))
                                            .on_hover_text("Move up the stack")
                                            .clicked()
                                        {
                                            act = Some(LayerAction::Raise(i));
                                        }
                                        if ui
                                            .add_enabled(i > 0, egui::Button::new("▼"))
                                            .on_hover_text("Move down the stack")
                                            .clicked()
                                        {
                                            act = Some(LayerAction::Lower(i));
                                        }
                                    },
                                );
                            });

                            ui.horizontal(|ui| {
                                // The route effects on this layer fan along.
                                let cur = l
                                    .order
                                    .and_then(|id| {
                                        order_names.iter().find(|(oid, _)| *oid == id)
                                    })
                                    .map(|(_, n)| n.as_str())
                                    .unwrap_or("patch order");
                                egui::ComboBox::from_id_salt(("layer-order", l.id))
                                    .selected_text(cur)
                                    .width(150.0)
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(l.order.is_none(), "patch order")
                                            .clicked()
                                        {
                                            act = Some(LayerAction::Order(i, None));
                                        }
                                        for (id, name) in &order_names {
                                            if ui
                                                .selectable_label(l.order == Some(*id), name)
                                                .clicked()
                                            {
                                                act = Some(LayerAction::Order(i, Some(*id)));
                                            }
                                        }
                                    });
                                if !l.targets.is_empty() {
                                    theme::hint(ui, format!("{} light(s)", l.targets.len()));
                                }
                            });

                            if l.boxes.is_empty() {
                                theme::hint(ui, "nothing applied yet");
                            } else {
                                let marked: Vec<usize> = l
                                    .boxes
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, b)| b.selected)
                                    .map(|(k, _)| k)
                                    .collect();
                                let tile = raid::tile_size(look, BOX_ZOOM);
                                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                                let painter = ui.painter().clone();
                                ui.horizontal_wrapped(|ui| {
                                    for (k, b) in l.boxes.iter().enumerate() {
                                        let c = swatches
                                            .get(i)
                                            .and_then(|v| v.get(k))
                                            .copied()
                                            .unwrap_or(theme::ACCENT_SOFT);
                                        let (rect, r) =
                                            ui.allocate_exact_size(tile, Sense::click());
                                        // The same tile the Fixtures grid is
                                        // made of, in whatever material the
                                        // operator picked, so a layer's
                                        // contents read like the rest of the
                                        // desk instead of like a new widget.
                                        raid::paint(
                                            &painter,
                                            look,
                                            rect,
                                            BOX_ZOOM,
                                            &raid::Tile {
                                                name: &b.label,
                                                addr: &format!(
                                                    "{} {}",
                                                    b.kind.short_tag(),
                                                    b.addrs.len()
                                                ),
                                                color: c,
                                                level: if b.selected { 1.0 } else { 0.7 },
                                                selected: b.selected,
                                            },
                                        );
                                        let r = r.on_hover_text(format!(
                                            "{} · {} · {} channel(s)\nClick to drop it · ⇧ click to mark several",
                                            b.label,
                                            b.kind.tag(),
                                            b.addrs.len()
                                        ));
                                        if r.clicked() {
                                            act = Some(if ui.input(|i| i.modifiers.shift) {
                                                LayerAction::MarkBox(i, k)
                                            } else {
                                                LayerAction::DropBoxes(i, vec![k])
                                            });
                                        }
                                    }
                                });
                                if !marked.is_empty() {
                                    if ui
                                        .button(format!("Drop {} marked", marked.len()))
                                        .on_hover_text(
                                            "Release these and let the layers below show through",
                                        )
                                        .clicked()
                                    {
                                        act = Some(LayerAction::DropBoxes(i, marked));
                                    }
                                }
                            }
                        });
                    ui.add_space(4.0);
                }
            },
        );

        self.zoom.layers = zoom_level;
        self.show_layers = open;
        if popped {
            self.popped_out.insert("layers".into());
        } else {
            self.popped_out.remove("layers");
        }

        if do_add {
            self.add_layer(self.transition.fade(TransitionTarget::LayerGo));
        }
        match act {
            Some(LayerAction::Select(i)) => self.select_layer(i),
            Some(LayerAction::Raise(i)) => self.move_layer(i, i + 1),
            Some(LayerAction::Lower(i)) => self.move_layer(i, i.saturating_sub(1)),
            Some(LayerAction::Remove(i)) => self.remove_layer(i, fade),
            Some(LayerAction::Order(i, id)) => self.set_layer_order(i, id),
            Some(LayerAction::MarkBox(li, bi)) => {
                if let Some(b) = self.layers.get_mut(li).and_then(|l| l.boxes.get_mut(bi)) {
                    b.selected = !b.selected;
                }
            }
            Some(LayerAction::DropBoxes(i, picks)) => self.drop_layer_boxes(i, &picks),
            Some(LayerAction::StopAbove(name)) => self.stop_phaser(&name),
            None => {}
        }
    }
}

impl App {
    /// Move a layer to a new position in the stack, keeping the programmer
    /// pointed at whichever layer it was on.
    pub(crate) fn move_layer(&mut self, from: usize, to: usize) {
        if from >= self.layers.len() || to >= self.layers.len() || from == to {
            return;
        }
        let l = self.layers.remove(from);
        self.layers.insert(to, l);
        // `active_layer` indexes the same Vec, and the live programmer state
        // belongs to whatever sits there — so it has to follow the move.
        self.active_layer = if self.active_layer == from {
            to
        } else {
            let mut a = self.active_layer;
            if from < a {
                a -= 1;
            }
            if to <= a {
                a += 1;
            }
            a
        };
    }

    /// Point a layer at an order. The selected layer's route is the live
    /// `active_order`, so setting that one updates both.
    pub(crate) fn set_layer_order(&mut self, idx: usize, id: Option<u32>) {
        let Some(l) = self.layers.get_mut(idx) else {
            return;
        };
        l.order = id;
        if idx == self.active_layer {
            self.active_order = id.and_then(|i| self.orders.iter().position(|o| o.id == i));
        }
    }

    /// Keep the selected layer's stored route in step with the live one, so
    /// picking an order in the Orders window sticks to the layer it was
    /// picked on.
    pub(crate) fn sync_layer_order(&mut self) {
        let id = self
            .active_order
            .and_then(|i| self.orders.get(i))
            .map(|o| o.id);
        if let Some(l) = self.layers.get_mut(self.active_layer) {
            l.order = id;
        }
    }
}
