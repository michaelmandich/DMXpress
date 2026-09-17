//! The level bar: a fat, click-anywhere fader for one DMX byte.
//!
//! Press anywhere on the bar and the level jumps there; keep the button
//! down and it follows the pointer. Hold Shift (at the press or during it)
//! and the drag turns fine — six pixels per step from wherever the level
//! was — and stays fine for the rest of that press. Alt + wheel nudges by
//! one (Alt + Shift + wheel by ten); a plain wheel scrolls the list as it
//! always did. The bar never takes keyboard focus, so the arrow keys keep
//! belonging to the armed channels and Space to the beat tap.

use eframe::egui::{self, Align2, Color32, CursorIcon, FontFamily, FontId, Rangef, Rect, Rounding, Sense, Shape, Stroke, Vec2};

use super::theme;

/// Rail inset from the bar's edge.
pub(crate) const PAD: f32 = 2.0;
/// Screen pixels per DMX step in fine mode, at zoom 1.
pub(crate) const FINE_PX: f32 = 6.0;
/// Temp-data key: `InputState::time` of the last Alt+wheel nudge. egui pays
/// a wheel notch out over a few frames; the list stays still that long.
pub(crate) fn wheel_nudge_key() -> egui::Id {
    egui::Id::new("chan_alt_wheel_t")
}
/// How long after an Alt+wheel nudge the list ignores the wheel.
pub(crate) const WHEEL_HOLD_S: f64 = 0.3;

/// How to paint the bar.
pub(crate) struct BarStyle<'a> {
    pub tint: Color32,
    pub armed: bool,
    /// The Stream Deck knob holds this channel: edits land under it.
    pub held: bool,
    /// A multi-fixture row whose lights disagree: the range they span.
    pub spread: Option<(u8, u8)>,
    /// The programmer's base while an oscillator moves the shown value.
    pub base_tick: Option<u8>,
    /// Band starts, for stepped channels.
    pub bands: &'a [(u8, u8)],
}

/// What a bar asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BarEdit {
    Set(u8),
    Nudge(i32),
}

/// One press in progress, kept in egui's temp storage under the bar's id.
#[derive(Clone, Copy, Debug)]
struct Press {
    fine: bool,
    last_x: f32,
    acc: f32,
}

/// The value at pointer x on `rail`, clamped to the ends.
pub(crate) fn value_at(x: f32, rail: Rangef) -> u8 {
    let span = rail.span();
    let t = if span <= 0.0 { 0.0 } else { ((x - rail.min) / span).clamp(0.0, 1.0) };
    (t * 255.0).round() as u8
}

/// Where value `v` sits on `rail`.
pub(crate) fn x_of(v: u8, rail: Rangef) -> f32 {
    rail.min + rail.span() * (v as f32 / 255.0)
}

/// Turn pointer motion into whole steps, carrying the remainder in `acc`.
pub(crate) fn fine_steps(acc: &mut f32, dx: f32, px_per_step: f32) -> i32 {
    let px = px_per_step.max(0.5);
    *acc += dx;
    let steps = (*acc / px).trunc();
    *acc -= steps * px;
    steps as i32
}

/// Show a level bar of `size` for value `v`. Returns the response (marked
/// changed when it asks for an edit) and the edit, if any.
pub(crate) fn level_bar(
    ui: &mut egui::Ui,
    id: egui::Id,
    size: Vec2,
    v: u8,
    style: &BarStyle,
) -> (egui::Response, Option<BarEdit>) {
    let (_, rect) = ui.allocate_space(size);
    let resp = ui.interact(rect, id, Sense { click: true, drag: true, focusable: false });
    let z = ui.spacing().interact_size.y / 22.0;
    let rail = rect.x_range().shrink(PAD);
    let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
    let mut out: Option<BarEdit> = None;

    let down = resp.is_pointer_button_down_on();
    if !down && resp.clicked() {
        // Pressed and released inside one frame (a long frame, a quick
        // tap): still an absolute set.
        if let Some(p) = resp.interact_pointer_pos() {
            out = Some(BarEdit::Set(value_at(p.x, rail)));
        }
    }
    if down {
        if let Some(p) = resp.interact_pointer_pos() {
            // A press that ended while this bar was scrolled away leaves a
            // stale record; the press frame always starts fresh.
            let fresh = ui.input(|i| i.pointer.any_pressed());
            let stored = if fresh { None } else { ui.data(|d| d.get_temp::<Press>(id)) };
            let mut press: Press = stored.unwrap_or(Press { fine: false, last_x: p.x, acc: 0.0 });
            if shift && !press.fine {
                // Latched for the rest of the press; from here on the level
                // moves with the pointer, not to it.
                press.fine = true;
                press.last_x = p.x;
                press.acc = 0.0;
            }
            if press.fine {
                let n = fine_steps(&mut press.acc, p.x - press.last_x, FINE_PX * z);
                if n != 0 {
                    out = Some(BarEdit::Nudge(n));
                }
            } else {
                out = Some(BarEdit::Set(value_at(p.x, rail)));
            }
            press.last_x = p.x;
            ui.data_mut(|d| d.insert_temp(id, press));
        }
    } else {
        ui.data_mut(|d| d.remove::<Press>(id));
    }

    if resp.hovered() && alt {
        let raw = ui.input(|i| i.raw_scroll_delta);
        // egui folds Shift+wheel into the horizontal axis.
        let d = raw.y + raw.x;
        if d != 0.0 {
            let step = if shift { 10 } else { 1 };
            out = Some(BarEdit::Nudge(if d > 0.0 { step } else { -step }));
            let now = ui.input(|i| i.time);
            ui.data_mut(|d| d.insert_temp(wheel_nudge_key(), now));
        }
        // The list must not scroll under an Alt+wheel nudge.
        ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let r = 3.0;
        let tint = if style.held { theme::WARN } else { style.tint };
        p.rect_filled(rect.translate(Vec2::new(0.0, 1.0)), r, Color32::from_black_alpha(110));
        p.rect_filled(rect, r, theme::WELL);
        p.hline(
            rect.x_range().shrink(1.0),
            rect.top() + 0.5,
            Stroke::new(1.0, Color32::from_black_alpha(120)),
        );
        if let Some((lo, hi)) = style.spread {
            if lo != hi {
                let band = Rect::from_x_y_ranges(
                    Rangef::new(x_of(lo, rail), x_of(hi, rail)),
                    rect.y_range().shrink(2.0),
                );
                p.rect_filled(band, Rounding::ZERO, tint.gamma_multiply(0.25));
            }
        }
        if v > 0 {
            let right = x_of(v, rail).max(rect.left() + 2.0 * r);
            let fill = Rect::from_min_max(rect.min, egui::pos2(right, rect.max.y))
                .shrink2(Vec2::new(0.0, 1.0));
            p.add(Shape::mesh(theme::vgradient_mesh(
                fill,
                Rounding::same(r),
                theme::lighten(tint, 0.15),
                theme::darken(tint, 0.30),
            )));
            // The mesh is unfeathered; a hairline in the tint hides its edge.
            p.rect_stroke(fill, r, Stroke::new(1.0, tint));
        }
        for &(min, _) in style.bands {
            if min > 0 {
                p.vline(x_of(min, rail), rect.y_range().shrink(3.0), Stroke::new(1.0, theme::EDGE));
            }
        }
        if let Some(b) = style.base_tick {
            p.vline(x_of(b, rail), rect.y_range().shrink(1.0), Stroke::new(2.0, theme::TEXT));
        }
        let rim = if down || resp.hovered() {
            theme::RIM
        } else if style.armed {
            theme::ACCENT_MUTED
        } else {
            theme::EDGE
        };
        p.rect_stroke(rect, r, Stroke::new(1.0, rim));
        let txt = match style.spread {
            Some((lo, hi)) if lo != hi => format!("{lo}–{hi}"),
            _ => format!("{v:>3}"),
        };
        let font = FontId::new(11.0 * z, FontFamily::Monospace);
        let pos = egui::pos2(rect.right() - 6.0, rect.center().y);
        // Twice, shadow then ink, so it reads over a light tint.
        p.text(pos + Vec2::new(0.0, 1.0), Align2::RIGHT_CENTER, &txt, font.clone(), Color32::from_black_alpha(170));
        p.text(pos, Align2::RIGHT_CENTER, &txt, font, theme::TEXT);
    }

    let mut resp = resp.on_hover_and_drag_cursor(CursorIcon::ResizeHorizontal);
    if out.is_some() {
        resp.mark_changed();
    }
    let enabled = ui.is_enabled();
    resp.widget_info(|| egui::WidgetInfo::slider(enabled, v as f64, ""));
    if style.held {
        resp = resp.on_hover_text("Held by the Stream Deck knob — edits land under it until Clear");
    }
    (resp, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_maps_onto_the_rail_and_back() {
        let rail = Rangef::new(10.0, 310.0);
        assert_eq!(value_at(10.0, rail), 0);
        assert_eq!(value_at(310.0, rail), 255);
        assert_eq!(value_at(160.0, rail), 128);
        assert_eq!(value_at(-50.0, rail), 0);
        assert_eq!(value_at(999.0, rail), 255);
        for x in (10..=310).step_by(7) {
            let v = value_at(x as f32, rail);
            assert!((x_of(v, rail) - x as f32).abs() <= 1.0, "x {x} → {v} → {}", x_of(v, rail));
        }
        // A degenerate rail never panics.
        assert_eq!(value_at(5.0, Rangef::new(5.0, 5.0)), 0);
    }

    #[test]
    fn fine_motion_accumulates_whole_steps() {
        let mut acc = 0.0;
        assert_eq!(fine_steps(&mut acc, 6.0, 6.0), 1);
        assert_eq!(acc, 0.0);
        assert_eq!(fine_steps(&mut acc, 3.0, 6.0), 0);
        assert_eq!(fine_steps(&mut acc, 3.0, 6.0), 1);
        assert_eq!(fine_steps(&mut acc, -6.0, 6.0), -1);
        assert_eq!(fine_steps(&mut acc, 15.0, 6.0), 2);
        assert!((acc - 3.0).abs() < 1e-4);
    }
}
