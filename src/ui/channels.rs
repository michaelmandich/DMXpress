//! Channel control: the programmer's channel list under the stage.
//!
//! One row per channel of the selected light (or per channel *type* across
//! several), under feature headings in console order — Dimmer, Position,
//! Color, Beam, Focus, Control, Other. Each row is a click-anywhere level
//! bar. Clicking a name *arms* the row: armed channels feed the Oscillator
//! window and the arrow keys, which nudge every armed channel by one (Shift
//! ten, PgUp/PgDn a band, Home/End the ends) while the pointer is over this
//! section and no text field has the keyboard.
//!
//! Selection tactics live in the toolbar and the row menus: All / None /
//! Invert over the rows shown, a filter box, Similar (same role, group,
//! cell, coarse+fine pair, or name as what is armed), Shift-click ranges,
//! Ctrl-click toggles, Alt-click same role, double-click a whole group,
//! and per-heading Arm / 0 / FL. The row model and these operations are
//! pure functions in `chanrows.rs`; the bar widget is `levelbar.rs`.

use std::collections::HashSet;

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Id, Key, Layout, Modifiers, Rect, RichText, Rounding,
    Sense, Shape, Stroke, TextWrapMode, Vec2,
};

use super::chanrows::{
    apply_arm, build_rows, is_pair, nudged, row_matches, ArmOp, ChanGroup, ChanSort, Sim,
};
use super::levelbar::{level_bar, wheel_nudge_key, BarEdit, BarStyle, WHEEL_HOLD_S};
use super::{icons, role_color, theme};
use crate::app::App;
use crate::encoder::step_bands;
use crate::net;
use crate::oscillator::Look;

/// One line of the list: a heading or a channel row (indices into `rows`).
enum Line {
    Head { group: ChanGroup, rows: Vec<usize>, armed: usize },
    Row(usize),
}

impl App {
    /// A manual edit settles the look. If a transition was mid-flight,
    /// capture the edited output as the new base; otherwise fold only the
    /// changed channels into `live.base` so oscillators keep animating
    /// around them. Every touched channel joins the programmer and stops
    /// tracking whatever palette it used to reference.
    pub(crate) fn commit_channel_edit(&mut self, buf: net::Frame, orig: &net::Frame) {
        if self.transition_run.take().is_some() {
            self.live = Look::from_frame(buf);
        } else {
            for i in 0..net::DMX_SLOTS {
                if buf[i] != orig[i] {
                    self.live.base[i] = buf[i];
                }
            }
        }
        for i in 0..net::DMX_SLOTS {
            if buf[i] != orig[i] {
                self.live_active.insert(i);
                self.live_refs.remove(&i);
            }
        }
        *self.net.dmx.lock() = buf;
    }

    /// The value a relative move (arrow key, Alt+wheel, fine drag) starts
    /// from: the programmer's own base when it owns the channel and nothing
    /// paints over it, else what the row shows. Keeps an arrow key from
    /// collapsing a dimmer while the grand master is below full.
    pub(crate) fn relative_source(&self, i: usize, shown: u8) -> u8 {
        if self.transition_run.is_none()
            && self.live_active.contains(&i)
            && !self.encoder_layer.contains_key(&i)
            && !self.test_overrides.contains_key(&i)
        {
            self.live.base[i]
        } else {
            shown
        }
    }

    /// The whole section: encoder readout, title, toolbar, keyboard, list.
    pub(crate) fn channel_controls(&mut self, ui: &mut egui::Ui) {
        // Arrow keys belong to this section while the pointer is over it
        // (a tooltip over it counts) and nothing owns the keyboard.
        let area = ui.available_rect_before_wrap();
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over_tooltip = pointer.is_some_and(|p| {
            area.contains(p)
                && ui.ctx().layer_id_at(p).is_some_and(|l| l.order == egui::Order::Tooltip)
        });
        let hot = ui.rect_contains_pointer(area) || over_tooltip;
        let free = ui.memory(|m| m.focused()).is_none();
        let active = hot && free;

        let mut buf = *self.net.dmx.lock();
        let orig = buf;
        let mut changed = false;
        // Relative edits (arrows, Alt+wheel, fine drags) move the shown
        // value by the step for the wire, and the programmer's own base by
        // the same step — two different numbers under a grand master or
        // an oscillator, so the base result is carried separately.
        let mut base_edits: Vec<(usize, u8)> = Vec::new();
        let z = ui.spacing().interact_size.y / 22.0;
        let row_h = ui.spacing().interact_size.y;

        // The Stream Deck's programmer knob, when it is on something.
        let encoder = self.encoder_readout();
        let encoder_held = self.encoder_layer.len();
        let mut clear_encoders = false;
        if encoder.is_some() || encoder_held > 0 {
            ui.horizontal(|ui| {
                if let Some((name, value, n)) = &encoder {
                    ui.weak(format!("Knob: {name} = {value} ({n})")).on_hover_text(
                        "The Stream Deck's programmer knob: twist to nudge this channel \
                         on every selected light, press to move to the next channel",
                    );
                }
                if encoder_held > 0 {
                    if encoder.is_some() {
                        ui.separator();
                    }
                    ui.label(format!("{encoder_held} encoder ch")).on_hover_text(
                        "Channels held by the encoder layer — painted over the mix \
                         until cleared (Clear on the deck, or here)",
                    );
                    if ui.small_button("clear").clicked() {
                        clear_encoders = true;
                    }
                }
            });
        }
        if clear_encoders {
            self.clear_encoders();
        }

        // The lights: every stage-selected fixture, else the one list-selected.
        let targets: Vec<usize> = {
            let sf = self.stage.selected_fixtures();
            if sf.len() > 1 {
                sf
            } else if let Some(i) = self.sel_fixture {
                vec![i]
            } else {
                Vec::new()
            }
        };
        if targets.is_empty() {
            ui.label("Select a fixture from the list or stage view.");
            if !self.sel_channels.is_empty() {
                let n = self.sel_channels.len();
                ui.horizontal(|ui| {
                    theme::pill(ui, &format!("{n} armed"), theme::ACCENT_SOFT);
                    if ui.small_button("clear").clicked() {
                        self.sel_channels.clear();
                    }
                });
            }
            return;
        }
        let single = targets.len() == 1;
        self.sel_channels.retain(|&i| i < net::DMX_SLOTS);

        let rows = build_rows(&self.patch.fixtures, &targets, self.chan_ui.sort);
        let displayed: HashSet<usize> = rows.iter().flat_map(|r| r.idxs.iter().copied()).collect();
        let armed: Vec<bool> = rows
            .iter()
            .map(|r| r.idxs.iter().all(|i| self.sel_channels.contains(i)))
            .collect();
        let bands: Vec<Option<String>> = rows
            .iter()
            .map(|r| {
                self.patch
                    .fixtures
                    .get(r.repr.0)
                    .and_then(|f| f.channels.get(r.repr.1))
                    .and_then(|c| c.band_label(buf[r.idxs[0]]))
                    .map(str::to_string)
            })
            .collect();
        let filter_lower = self.chan_ui.filter.trim().to_lowercase();
        let (hide_fine, only_armed, sort) =
            (self.chan_ui.hide_fine, self.chan_ui.only_armed, self.chan_ui.sort);
        let visible_flag: Vec<bool> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                !(hide_fine && r.fine)
                    && !(only_armed && !armed[i])
                    && row_matches(r, &filter_lower, bands[i].as_deref(), single)
            })
            .collect();
        let mut lines: Vec<Line> = Vec::new();
        let mut visible: Vec<usize> = Vec::new();
        match sort {
            ChanSort::Feature => {
                for g in ChanGroup::ALL {
                    let idxs: Vec<usize> =
                        (0..rows.len()).filter(|&i| visible_flag[i] && rows[i].group == g).collect();
                    if idxs.is_empty() {
                        continue;
                    }
                    let armed_n = idxs.iter().filter(|&&i| armed[i]).count();
                    visible.extend(&idxs);
                    let folded = self.chan_ui.collapsed.contains(&g);
                    lines.push(Line::Head { group: g, rows: idxs.clone(), armed: armed_n });
                    if !folded {
                        lines.extend(idxs.into_iter().map(Line::Row));
                    }
                }
            }
            ChanSort::Address => {
                for i in 0..rows.len() {
                    if visible_flag[i] {
                        visible.push(i);
                        lines.push(Line::Row(i));
                    }
                }
            }
        }
        let candidates: Vec<usize> = (0..rows.len())
            .filter(|&i| row_matches(&rows[i], &filter_lower, bands[i].as_deref(), single))
            .collect();
        let n_armed = self.sel_channels.len();
        let k_here = self.sel_channels.iter().filter(|i| displayed.contains(i)).count();
        let idxs_of = |which: &[usize]| -> Vec<usize> {
            which.iter().flat_map(|&r| rows[r].idxs.iter().copied()).collect()
        };

        let mut ops: Vec<ArmOp> = Vec::new();
        let mut level_ops: Vec<(Vec<usize>, u8)> = Vec::new();
        let cmd = ui.input(|i| i.modifiers.command);

        // Title, with the armed count at the right.
        ui.horizontal(|ui| {
            if single {
                if let Some(f) = self.patch.fixtures.get(targets[0]) {
                    ui.label(
                        RichText::new(format!(
                            "{} · {}ch @ DMX {}..{}",
                            f.display,
                            f.channel_count(),
                            f.from,
                            f.to
                        ))
                        .family(theme::semibold())
                        .size(13.0 * z),
                    );
                }
            } else {
                ui.label(
                    RichText::new(format!("Channel types · {} fixtures", targets.len()))
                        .family(theme::semibold())
                        .size(13.0 * z),
                );
                ui.weak("Each row drives that type across every selected fixture.");
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if n_armed > 0 {
                    if ui.small_button("clear").on_hover_text("Disarm everything").clicked() {
                        ops.push(ArmOp::None);
                    }
                    let txt = if k_here == n_armed {
                        format!("{n_armed} armed")
                    } else {
                        format!("{n_armed} armed · {k_here} here")
                    };
                    theme::pill(ui, &txt, theme::ACCENT_SOFT).on_hover_text(
                        "Armed channels feed the Oscillator window and the arrow keys \
                         (while the pointer is over this list). 'here' = armed channels \
                         of the lights shown.",
                    );
                }
            });
        });

        // Toolbar: filter, arm tactics, view toggles, sort, whole-fixture levels.
        ui.horizontal_wrapped(|ui| {
            let search = ui
                .add(
                    egui::TextEdit::singleline(&mut self.chan_ui.filter)
                        .id(Id::new("chan_filter"))
                        .hint_text("Filter: name, tag, group, address")
                        .desired_width(190.0 * z),
                )
                .on_hover_text("Ctrl+F · matches name, role tag, group, address or slot name · Enter arms the matches");
            if !self.chan_ui.filter.is_empty()
                && ui.small_button("×").on_hover_text("Clear the filter").clicked()
            {
                self.chan_ui.filter.clear();
            }
            if search.lost_focus() {
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    let idxs = idxs_of(&visible);
                    ops.push(if cmd { ArmOp::Add(idxs) } else { ArmOp::Replace(idxs) });
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.chan_ui.filter.clear();
                }
            }
            ui.separator();
            if ui.small_button("All").on_hover_text("Arm every row shown · Ctrl+A").clicked() {
                ops.push(ArmOp::All);
            }
            if ui.small_button("None").on_hover_text("Disarm everything · Ctrl+Shift+A").clicked() {
                ops.push(ArmOp::None);
            }
            if ui
                .small_button("Invert")
                .on_hover_text("Swap armed and not, over the rows shown · Ctrl+I")
                .clicked()
            {
                ops.push(ArmOp::Invert);
            }
            ui.add_enabled_ui(n_armed > 0, |ui| {
                ui.menu_button("Similar", |ui| {
                    let items = [
                        (Sim::Role, "Same role as armed", "Every row with a role that is armed — all reds, say"),
                        (Sim::Group, "Same feature group", "Every row in a group that has something armed"),
                        (Sim::Cell, "Same cell / pixel", "Every channel of a pixel or head that has something armed"),
                        (Sim::Pair, "Coarse + fine partners", "The fine byte of each armed channel, and vice versa"),
                        (Sim::Name, "Same name", "Every row with the same base name as an armed one"),
                    ];
                    for (sim, label, hint) in items {
                        if ui.button(label).on_hover_text(hint).clicked() {
                            ops.push(ArmOp::Similar(sim));
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_hover_text("Add every row like the ones armed");
            });
            ui.separator();
            ui.toggle_value(&mut self.chan_ui.hide_fine, "hide fine")
                .on_hover_text("Hide the fine bytes of pan, tilt and dimmers");
            ui.toggle_value(&mut self.chan_ui.only_armed, "armed only")
                .on_hover_text("Show only the armed rows");
            ui.separator();
            ui.selectable_value(&mut self.chan_ui.sort, ChanSort::Feature, "By feature")
                .on_hover_text("Under headings, in console order");
            ui.selectable_value(&mut self.chan_ui.sort, ChanSort::Address, "By address")
                .on_hover_text("Flat, in DMX order");
            ui.separator();
            let every_channel = || -> Vec<usize> {
                targets
                    .iter()
                    .filter_map(|&fi| self.patch.fixtures.get(fi))
                    .flat_map(|f| {
                        (0..f.channel_count())
                            .map(move |ci| f.from as usize + ci)
                            .filter(|addr| (1..=net::DMX_SLOTS).contains(addr))
                            .map(|addr| addr - 1)
                    })
                    .collect()
            };
            if ui
                .small_button("Blackout")
                .on_hover_text("Every channel of the selected lights to 0")
                .clicked()
            {
                level_ops.push((every_channel(), 0));
            }
            if ui.small_button("Full").on_hover_text("Every channel of the selected lights to 255").clicked() {
                level_ops.push((every_channel(), 255));
            }
        });
        theme::hint(
            ui,
            "Click a name to arm · Shift range · Ctrl toggle · Alt same role · double-click a group · \
             with the pointer here: ↑↓ ±1 on armed, Shift ±10, PgUp/PgDn band, Home/End · Alt+wheel ±1 on a bar",
        );

        // Keyboard, only while this section is hot and no text field has the keys.
        if active {
            let (all, none, inv, find) = ui.input_mut(|i| {
                let none = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::A);
                let all = i.consume_key(Modifiers::COMMAND, Key::A);
                let inv = i.consume_key(Modifiers::COMMAND, Key::I);
                let find = i.consume_key(Modifiers::COMMAND, Key::F);
                (all, none, inv, find)
            });
            if all {
                ops.push(ArmOp::All);
            }
            if none {
                ops.push(ArmOp::None);
            }
            if inv {
                ops.push(ArmOp::Invert);
            }
            if find {
                ui.memory_mut(|m| m.request_focus(Id::new("chan_filter")));
            }
            let keys: HashSet<usize> =
                self.sel_channels.iter().copied().filter(|i| displayed.contains(i)).collect();
            if !keys.is_empty() {
                let (delta, page, home, end) = ui.input_mut(|i| {
                    let c = |i: &mut egui::InputState, m: Modifiers, k: Key| {
                        i.count_and_consume_key(m, k) as i32
                    };
                    // Shift chords first, so a plain check never eats them.
                    let big = c(i, Modifiers::SHIFT, Key::ArrowUp)
                        + c(i, Modifiers::SHIFT, Key::ArrowRight)
                        - c(i, Modifiers::SHIFT, Key::ArrowDown)
                        - c(i, Modifiers::SHIFT, Key::ArrowLeft);
                    let one = c(i, Modifiers::NONE, Key::ArrowUp)
                        + c(i, Modifiers::NONE, Key::ArrowRight)
                        - c(i, Modifiers::NONE, Key::ArrowDown)
                        - c(i, Modifiers::NONE, Key::ArrowLeft);
                    let page = c(i, Modifiers::NONE, Key::PageUp) - c(i, Modifiers::NONE, Key::PageDown);
                    let home = i.consume_key(Modifiers::NONE, Key::Home);
                    let end = i.consume_key(Modifiers::NONE, Key::End);
                    (big * 10 + one, page, home, end)
                });
                if delta != 0 {
                    for &i in &keys {
                        base_edits.push((i, nudged(self.relative_source(i, buf[i]), delta)));
                        buf[i] = nudged(buf[i], delta);
                    }
                    changed = true;
                }
                if page != 0 {
                    for r in &rows {
                        for &i in &r.idxs {
                            if keys.contains(&i) {
                                let cur = self.relative_source(i, buf[i]);
                                let (base, shown) = if r.stepped {
                                    (step_bands(&r.bands, cur, page), step_bands(&r.bands, buf[i], page))
                                } else {
                                    (nudged(cur, page * 16), nudged(buf[i], page * 16))
                                };
                                base_edits.push((i, base));
                                buf[i] = shown;
                            }
                        }
                    }
                    changed = true;
                }
                if home || end {
                    let v = if end { 255 } else { 0 };
                    for &i in &keys {
                        buf[i] = v;
                    }
                    changed = true;
                }
            }
        }

        // The list: only the lines on screen are laid out.
        let actions_w = 126.0 * z;
        let encoder_key = self.encoder_key.clone();
        egui::ScrollArea::vertical()
            .id_salt("chan_ctrl")
            .auto_shrink([false, false])
            .show_rows(ui, row_h, lines.len(), |ui, range| {
                for line in &lines[range] {
                    match line {
                        Line::Head { group, rows: grp, armed: armed_n } => {
                            let group = *group;
                            let grp_idxs = idxs_of(grp);
                            ui.horizontal(|ui| {
                                ui.set_height(row_h);
                                ui.spacing_mut().item_spacing.x = 6.0 * z;
                                let (rect, resp) = ui.allocate_exact_size(
                                    Vec2::new((ui.available_width() - actions_w).max(row_h), row_h),
                                    Sense::click(),
                                );
                                let folded = self.chan_ui.collapsed.contains(&group);
                                let p = ui.painter();
                                if resp.hovered() {
                                    p.rect_filled(rect, Rounding::ZERO, ui.visuals().faint_bg_color);
                                }
                                let icon_rect = Rect::from_min_size(rect.left_top(), Vec2::splat(row_h))
                                    .shrink(row_h * 0.3);
                                let chevron = if folded {
                                    icons::Icon::ChevronRight
                                } else {
                                    icons::Icon::ChevronDown
                                };
                                icons::draw(p, icon_rect, chevron, theme::TEXT_DIM);
                                let swatch = Rect::from_min_size(
                                    egui::pos2(rect.left() + row_h, rect.center().y - 7.0 * z),
                                    Vec2::new(3.0 * z, 14.0 * z),
                                );
                                p.rect_filled(swatch, Rounding::ZERO, group.tint());
                                let galley = p.layout_no_wrap(
                                    group.label().to_string(),
                                    FontId::new(12.0 * z, theme::semibold()),
                                    theme::TEXT,
                                );
                                let text_x = rect.left() + row_h + 8.0 * z;
                                let text_w = galley.size().x;
                                p.galley(
                                    egui::pos2(text_x, rect.center().y - galley.size().y * 0.5),
                                    galley,
                                    theme::TEXT,
                                );
                                let mut count = format!("({})", grp.len());
                                if folded && *armed_n > 0 {
                                    count.push_str(&format!(" · {armed_n} armed"));
                                }
                                p.text(
                                    egui::pos2(text_x + text_w + 6.0 * z, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    count,
                                    FontId::new(11.0 * z, egui::FontFamily::Proportional),
                                    theme::TEXT_DIM,
                                );
                                p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, theme::EDGE));
                                if resp.clicked() {
                                    if cmd {
                                        ops.push(ArmOp::Add(grp_idxs.clone()));
                                    } else if folded {
                                        self.chan_ui.collapsed.remove(&group);
                                    } else {
                                        self.chan_ui.collapsed.insert(group);
                                    }
                                }
                                if ui
                                    .small_button("Arm")
                                    .on_hover_text("Arm the whole group (Ctrl adds)")
                                    .clicked()
                                {
                                    ops.push(if cmd {
                                        ArmOp::Add(grp_idxs.clone())
                                    } else {
                                        ArmOp::Replace(grp_idxs.clone())
                                    });
                                }
                                if ui.small_button("0").on_hover_text("Every visible channel in this group to 0").clicked() {
                                    level_ops.push((grp_idxs.clone(), 0));
                                }
                                if ui.small_button("FL").on_hover_text("Every visible channel in this group to 255").clicked() {
                                    level_ops.push((grp_idxs.clone(), 255));
                                }
                            });
                        }
                        Line::Row(ri) => {
                            let ri = *ri;
                            let row = &rows[ri];
                            let is_armed = armed[ri];
                            let idx0 = row.idxs[0];
                            let bg = ui.painter().add(Shape::Noop);
                            let inner = ui.horizontal(|ui| {
                                ui.set_height(row_h);
                                ui.spacing_mut().item_spacing.x = 6.0 * z;
                                // The name is a SelectableLabel, which sizes itself to the
                                // text plus the button padding — taller than the row the
                                // list is virtualised on. No vertical padding keeps every
                                // line exactly row_h, so show_rows' arithmetic holds.
                                ui.spacing_mut().button_padding.y = 0.0;
                                ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
                                // The knob's row carries a thin accent bar at its left edge.
                                if encoder_key.as_deref() == Some(row.enc_key.as_str()) {
                                    let r = ui.available_rect_before_wrap();
                                    ui.painter().rect_filled(
                                        Rect::from_min_size(r.left_top(), Vec2::new(2.0, row_h)),
                                        Rounding::ZERO,
                                        theme::ACCENT_MUTED,
                                    );
                                }
                                // Role badge.
                                let tag = row.role.tag();
                                let badge = if tag.is_empty() {
                                    "    ·".to_string()
                                } else if row.fine && !tag.ends_with('f') {
                                    format!("{:>4}f", tag)
                                } else {
                                    format!("{:>5}", tag)
                                };
                                let badge_color = if tag.is_empty() { theme::TEXT_DIM } else { role_color(row.role) };
                                let badge_resp = ui.add_sized(
                                    [44.0 * z, row_h],
                                    egui::Label::new(RichText::new(badge).monospace().size(10.0 * z).color(badge_color)),
                                );
                                let osc_on = self.live.oscs.get(&idx0).is_some_and(|o| o.enabled);
                                if osc_on {
                                    ui.painter().circle_filled(
                                        badge_resp.rect.right_top() + Vec2::new(-3.0, 4.0),
                                        2.5 * z,
                                        theme::ACCENT_SOFT,
                                    );
                                    badge_resp.on_hover_text("Oscillator running — edit it in the Oscillator window");
                                }
                                // Address, or how many lights the row drives.
                                let addr_txt = if single {
                                    format!("{:>4}", row.addr)
                                } else {
                                    format!("×{}", row.idxs.len())
                                };
                                ui.add_sized(
                                    [34.0 * z, row_h],
                                    egui::Label::new(RichText::new(addr_txt).monospace().size(10.0 * z).color(theme::TEXT_DIM)),
                                );
                                // The name is the arm control.
                                // The name takes the slack once the bar has hit its widest.
                                let slack = (ui.available_width() - 150.0 * z - 200.0 * z - 480.0 * z).max(0.0);
                                let mut name_w = 150.0 * z + slack.min(220.0 * z);
                                if row.fine {
                                    ui.add_space(ui.spacing().indent);
                                    name_w -= ui.spacing().indent;
                                }
                                let name_resp = ui
                                    .allocate_ui_with_layout(
                                        Vec2::new(name_w, row_h),
                                        Layout::left_to_right(Align::Center)
                                            .with_main_align(Align::Min)
                                            .with_main_justify(true)
                                            .with_cross_justify(true),
                                        |ui| {
                                            ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
                                            ui.selectable_label(
                                                is_armed,
                                                RichText::new(&row.name).family(theme::medium()),
                                            )
                                        },
                                    )
                                    .inner;
                                let where_ = if single {
                                    format!("DMX {}", row.addr)
                                } else {
                                    format!("×{} lights", row.idxs.len())
                                };
                                let hover = format!(
                                    "{}\n{}{}\nClick to arm · Shift range · Ctrl toggle · Alt same role · double-click the group",
                                    row.name,
                                    where_,
                                    if row.stepped { " · stepped (PgUp/PgDn hops bands)" } else { "" }
                                );
                                let name_resp = name_resp.on_hover_text(hover);
                                let mods = ui.input(|i| i.modifiers);
                                let same_role = || -> Vec<usize> {
                                    visible
                                        .iter()
                                        .filter(|&&r| rows[r].role == row.role)
                                        .flat_map(|&r| rows[r].idxs.iter().copied())
                                        .collect()
                                };
                                let same_group = || -> Vec<usize> {
                                    visible
                                        .iter()
                                        .filter(|&&r| rows[r].group == row.group)
                                        .flat_map(|&r| rows[r].idxs.iter().copied())
                                        .collect()
                                };
                                // egui counts a double-click by time alone, so two quick
                                // clicks on different rows would read as one: only a
                                // second click on the same row within the delay counts.
                                let now = ui.input(|i| i.time);
                                let delay = ui.ctx().options(|o| o.input_options.max_double_click_delay);
                                let same_row_twice = self
                                    .chan_ui
                                    .last_click
                                    .as_ref()
                                    .is_some_and(|(k, t)| *k == row.key && now - t < delay);
                                if name_resp.double_clicked() && same_row_twice {
                                    ops.push(ArmOp::Replace(same_group()));
                                    self.chan_ui.last_click = None;
                                } else if name_resp.clicked() {
                                    self.chan_ui.last_click = Some((row.key.clone(), now));
                                    if mods.shift {
                                        ops.push(ArmOp::Range { to_key: row.key.clone() });
                                    } else if mods.alt {
                                        let idxs = same_role();
                                        ops.push(if mods.command { ArmOp::Add(idxs) } else { ArmOp::Replace(idxs) });
                                    } else if mods.command {
                                        ops.push(ArmOp::Toggle(row.idxs.clone()));
                                    } else {
                                        // A row armed on its own, clicked again, disarms.
                                        ops.push(if is_armed && n_armed == row.idxs.len() {
                                            ArmOp::None
                                        } else {
                                            ArmOp::Replace(row.idxs.clone())
                                        });
                                        self.chan_ui.anchor = Some(row.key.clone());
                                    }
                                }
                                name_resp.context_menu(|ui| {
                                    let cmd = ui.input(|i| i.modifiers.command);
                                    let pick = |idxs: Vec<usize>| if cmd { ArmOp::Add(idxs) } else { ArmOp::Replace(idxs) };
                                    if ui.button("Arm only this").clicked() {
                                        ops.push(ArmOp::Replace(row.idxs.clone()));
                                        ui.close_menu();
                                    }
                                    if ui.button("Add to armed").clicked() {
                                        ops.push(ArmOp::Add(row.idxs.clone()));
                                        ui.close_menu();
                                    }
                                    if ui.button("Remove from armed").clicked() {
                                        ops.push(ArmOp::Remove(row.idxs.clone()));
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                    let n_role = visible.iter().filter(|&&r| rows[r].role == row.role).count();
                                    let role_name = if tag.is_empty() { "unclassified".to_string() } else { tag.to_string() };
                                    if ui.button(format!("Same role ({role_name}) — {n_role} rows")).clicked() {
                                        ops.push(pick(same_role()));
                                        ui.close_menu();
                                    }
                                    if ui.button(format!("Same group ({})", row.group.label())).clicked() {
                                        ops.push(pick(same_group()));
                                        ui.close_menu();
                                    }
                                    if let Some(c) = &row.cell {
                                        if ui.button(format!("Same cell {c}")).clicked() {
                                            let idxs = visible
                                                .iter()
                                                .filter(|&&r| rows[r].cell.as_deref() == Some(c.as_str()))
                                                .flat_map(|&r| rows[r].idxs.iter().copied())
                                                .collect();
                                            ops.push(pick(idxs));
                                            ui.close_menu();
                                        }
                                    }
                                    if let Some(&partner) = visible.iter().find(|&&r| r != ri && is_pair(row, &rows[r])) {
                                        if ui.button("Coarse + fine partner").clicked() {
                                            let mut idxs = row.idxs.clone();
                                            idxs.extend(rows[partner].idxs.iter().copied());
                                            ops.push(pick(idxs));
                                            ui.close_menu();
                                        }
                                    }
                                    if ui.button(format!("Same name «{}»", row.base)).clicked() {
                                        let idxs = visible
                                            .iter()
                                            .filter(|&&r| rows[r].base == row.base)
                                            .flat_map(|&r| rows[r].idxs.iter().copied())
                                            .collect();
                                        ops.push(pick(idxs));
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                    if ui.button("Set 0").clicked() {
                                        level_ops.push((row.idxs.clone(), 0));
                                        ui.close_menu();
                                    }
                                    if ui.button("Set 255").clicked() {
                                        level_ops.push((row.idxs.clone(), 255));
                                        ui.close_menu();
                                    }
                                });
                                // The level bar.
                                let bar_w = (ui.available_width() - 200.0 * z).clamp(100.0 * z, 480.0 * z);
                                let lo = row.idxs.iter().map(|&i| buf[i]).min().unwrap_or(0);
                                let hi = row.idxs.iter().map(|&i| buf[i]).max().unwrap_or(0);
                                let held = self.encoder_layer.contains_key(&idx0);
                                let style = BarStyle {
                                    tint: role_color(row.role),
                                    armed: is_armed,
                                    held,
                                    spread: (lo != hi).then_some((lo, hi)),
                                    base_tick: osc_on.then(|| self.live.base[idx0]),
                                    bands: &row.bands,
                                };
                                let (_, edit) = level_bar(
                                    ui,
                                    Id::new(("chan_lvl", idx0)),
                                    Vec2::new(bar_w, row_h - 4.0 * z),
                                    buf[idx0],
                                    &style,
                                );
                                match edit {
                                    Some(BarEdit::Set(n)) => {
                                        for &i in &row.idxs {
                                            buf[i] = n;
                                        }
                                        changed = true;
                                    }
                                    Some(BarEdit::Nudge(d)) => {
                                        for &i in &row.idxs {
                                            base_edits.push((i, nudged(self.relative_source(i, buf[i]), d)));
                                            buf[i] = nudged(buf[i], d);
                                        }
                                        changed = true;
                                    }
                                    None => {}
                                }
                                if ui.small_button("0").clicked() {
                                    level_ops.push((row.idxs.clone(), 0));
                                }
                                if ui.small_button("255").clicked() {
                                    level_ops.push((row.idxs.clone(), 255));
                                }
                                if let Some(lbl) = &bands[ri] {
                                    ui.add(egui::Label::new(RichText::new(lbl).color(theme::TEXT_DIM).size(11.0 * z)).truncate());
                                }
                            });
                            let row_rect = inner.response.rect;
                            let fill = if is_armed {
                                theme::ACCENT.gamma_multiply(0.10)
                            } else if ui.rect_contains_pointer(row_rect) {
                                ui.visuals().faint_bg_color
                            } else {
                                Color32::TRANSPARENT
                            };
                            ui.painter().set(bg, Shape::rect_filled(row_rect, Rounding::ZERO, fill));
                        }
                    }
                }
                // egui pays a wheel notch out over a few frames; after an
                // Alt+wheel nudge the remainder must not scroll the list.
                let now = ui.input(|i| i.time);
                let nudged_at: Option<f64> = ui.data(|d| d.get_temp(wheel_nudge_key()));
                if nudged_at.is_some_and(|t| now - t < WHEEL_HOLD_S) {
                    ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
                }
            });

        for (idxs, v) in level_ops {
            for i in idxs {
                if i < net::DMX_SLOTS {
                    buf[i] = v;
                }
            }
            changed = true;
        }
        let anchor = self.chan_ui.anchor.clone();
        for op in ops {
            apply_arm(&mut self.sel_channels, anchor.as_deref(), op, &rows, &visible, &candidates);
        }
        if changed {
            self.commit_channel_edit(buf, &orig);
            for (i, v) in base_edits {
                self.live.base[i] = v;
                self.live_active.insert(i);
                self.live_refs.remove(&i);
            }
            // A held arrow is one edit, not one per repeat.
            self.undo.note_input();
        }
    }
}
