//! Hand-drawn vector icons for the toolbar and window chrome.
//!
//! Emoji render differently on every platform — different metrics, different
//! colours, sometimes a tofu box. These are painted from primitives instead,
//! so a control looks the same on macOS and Windows and picks up the theme's
//! colours like any other widget.

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, Vec2};

use super::theme;

/// The DMXexpress mark, shared by the toolbar and the window icon.
static LOGO_PNG: &[u8] = include_bytes!("../../assets/logo.png");

/// The embedded DMXexpress mark, decoded once and uploaded to the GPU.
pub fn logo_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let decoded = image::load_from_memory(LOGO_PNG).ok()?.into_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    Some(ctx.load_texture("dmxpress-logo", image, egui::TextureOptions::LINEAR))
}

/// The title-bar / taskbar icon: the mark centred on a black square.
///
/// The window manager wants a square, opaque image, and the bare logo is both
/// wide and transparent — letterboxing it on black keeps it readable at the
/// 16px the taskbar actually draws.
pub fn window_icon() -> Option<egui::IconData> {
    const SIDE: u32 = 256;
    const PAD: f32 = 0.1;

    let logo = image::load_from_memory(LOGO_PNG).ok()?.into_rgba8();
    let budget = SIDE as f32 * (1.0 - PAD * 2.0);
    let scale = (budget / logo.width() as f32).min(budget / logo.height() as f32);
    let w = ((logo.width() as f32 * scale).round() as u32).max(1);
    let h = ((logo.height() as f32 * scale).round() as u32).max(1);
    let scaled = image::imageops::resize(&logo, w, h, image::imageops::FilterType::Lanczos3);

    let mut canvas = image::RgbaImage::from_pixel(SIDE, SIDE, image::Rgba([0, 0, 0, 255]));
    image::imageops::overlay(
        &mut canvas,
        &scaled,
        ((SIDE - w) / 2) as i64,
        ((SIDE - h) / 2) as i64,
    );
    Some(egui::IconData {
        rgba: canvas.into_raw(),
        width: SIDE,
        height: SIDE,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Artnet,
    Wave,
    Timer,
    Chase,
    Group,
    Order,
    Scene,
    Palette,
    Phaser,
    Beat,
    Stack,
    Deck,
    Command,
    Views,
    Log,
    Patch,
    Config,
    Test,
    Settings,
    Freeze,
    Play,
}

/// Map unit coordinates (0..1, y down) onto `r`.
fn p(r: Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(r.left() + r.width() * x, r.top() + r.height() * y)
}

fn path(painter: &egui::Painter, r: Rect, s: Stroke, pts: &[(f32, f32)]) {
    let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| p(r, x, y)).collect();
    painter.add(Shape::line(pts, s));
}

/// A sine-ish wave sampled across the icon, `phase` in turns.
fn wave(painter: &egui::Painter, r: Rect, s: Stroke, y: f32, amp: f32, phase: f32) {
    let pts: Vec<Pos2> = (0..=16)
        .map(|i| {
            let t = i as f32 / 16.0;
            let a = (t + phase) * std::f32::consts::TAU;
            p(r, 0.1 + t * 0.8, y - a.sin() * amp)
        })
        .collect();
    painter.add(Shape::line(pts, s));
}

/// Paint `icon` inside `rect` in `color`.
pub fn draw(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    // Keep the artwork square and slightly inset so strokes never clip.
    let side = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center(), Vec2::splat(side)).shrink(side * 0.08);
    let w = (side * 0.09).clamp(1.0, 2.0);
    let s = Stroke::new(w, color);

    match icon {
        // Broadcast: a source with two widening arcs.
        Icon::Artnet => {
            painter.circle_filled(p(r, 0.5, 0.72), side * 0.09, color);
            path(painter, r, s, &[(0.5, 0.72), (0.5, 0.42)]);
            for (dx, dy) in [(0.18, 0.20), (0.32, 0.06)] {
                path(
                    painter,
                    r,
                    s,
                    &[(0.5 - dx, dy + 0.10), (0.5, dy), (0.5 + dx, dy + 0.10)],
                );
            }
        }
        Icon::Wave => wave(painter, r, s, 0.5, 0.22, 0.0),
        // Clock face with two hands.
        Icon::Timer => {
            painter.circle_stroke(r.center(), side * 0.36, s);
            path(painter, r, s, &[(0.5, 0.5), (0.5, 0.28)]);
            path(painter, r, s, &[(0.5, 0.5), (0.68, 0.58)]);
        }
        // A run of dots trailing off, the way a chase reads on stage.
        Icon::Chase => {
            for (i, x) in [0.18f32, 0.4, 0.62, 0.84].iter().enumerate() {
                let fade = 1.0 - i as f32 * 0.22;
                painter.circle_filled(
                    p(r, *x, 0.5),
                    side * (0.13 - i as f32 * 0.02),
                    color.gamma_multiply(fade),
                );
            }
        }
        // A cluster of lights held together.
        Icon::Group => {
            for (x, y) in [(0.3, 0.32), (0.7, 0.32), (0.3, 0.68), (0.7, 0.68)] {
                painter.circle_filled(p(r, x, y), side * 0.12, color);
            }
        }
        // A numbered route threading between lights.
        Icon::Order => {
            path(painter, r, s, &[(0.2, 0.74), (0.44, 0.3), (0.68, 0.7), (0.86, 0.34)]);
            for (x, y) in [(0.2, 0.74), (0.44, 0.3), (0.68, 0.7), (0.86, 0.34)] {
                painter.circle_filled(p(r, x, y), side * 0.09, color);
            }
        }
        // Three offset cards: looks stacked on top of one another.
        Icon::Scene => {
            for (i, (x, y)) in [(0.36f32, 0.34f32), (0.5, 0.5), (0.64, 0.66)]
                .iter()
                .enumerate()
            {
                let fade = 0.5 + i as f32 * 0.25;
                painter.rect_stroke(
                    Rect::from_center_size(p(r, *x, *y), Vec2::splat(side * 0.34)),
                    2.0,
                    Stroke::new(w, color.gamma_multiply(fade)),
                );
            }
        }
        // Painter's palette: a rounded blob with wells.
        Icon::Palette => {            painter.circle_stroke(r.center(), side * 0.36, s);
            for (x, y) in [(0.36, 0.36), (0.62, 0.33), (0.68, 0.58)] {
                painter.circle_filled(p(r, x, y), side * 0.07, color);
            }
        }
        // Two waves running out of phase — the whole point of a phaser.
        Icon::Phaser => {
            wave(painter, r, s, 0.36, 0.16, 0.0);
            wave(
                painter,
                r,
                Stroke::new(w, color.gamma_multiply(0.6)),
                0.68,
                0.16,
                0.25,
            );
        }
        // A pulse spike on a baseline.
        Icon::Beat => {
            path(
                painter,
                r,
                s,
                &[
                    (0.1, 0.6),
                    (0.32, 0.6),
                    (0.44, 0.22),
                    (0.56, 0.78),
                    (0.68, 0.6),
                    (0.9, 0.6),
                ],
            );
        }
        // Stacked cues.
        Icon::Stack => {
            for (i, y) in [0.28f32, 0.5, 0.72].iter().enumerate() {
                let inset = i as f32 * 0.04;
                painter.rect_stroke(
                    Rect::from_min_max(p(r, 0.16 + inset, y - 0.07), p(r, 0.84 - inset, y + 0.07)),
                    side * 0.06,
                    s,
                );
            }
        }
        // Two faders.
        Icon::Deck => {
            for (x, knob) in [(0.36f32, 0.62f32), (0.64, 0.38)] {
                path(painter, r, s, &[(x, 0.14), (x, 0.86)]);
                painter.rect_filled(
                    Rect::from_center_size(p(r, x, knob), Vec2::new(side * 0.3, side * 0.13)),
                    side * 0.05,
                    color,
                );
            }
        }
        // A prompt caret in a frame.
        Icon::Command => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.1, 0.18), p(r, 0.9, 0.82)),
                side * 0.09,
                s,
            );
            path(painter, r, s, &[(0.28, 0.38), (0.44, 0.5), (0.28, 0.62)]);
            path(painter, r, s, &[(0.52, 0.64), (0.74, 0.64)]);
        }
        // Workspace panes.
        Icon::Views => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.12, 0.18), p(r, 0.88, 0.82)),
                side * 0.08,
                s,
            );
            path(painter, r, s, &[(0.45, 0.18), (0.45, 0.82)]);
            path(painter, r, s, &[(0.45, 0.5), (0.88, 0.5)]);
        }
        // Lines of text.
        Icon::Log => {
            for (y, right) in [(0.28f32, 0.86f32), (0.5, 0.72), (0.72, 0.8)] {
                path(painter, r, s, &[(0.14, y), (right, y)]);
            }
        }
        // A plug with two prongs.
        Icon::Patch => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.28, 0.4), p(r, 0.72, 0.8)),
                side * 0.08,
                s,
            );
            path(painter, r, s, &[(0.4, 0.4), (0.4, 0.16)]);
            path(painter, r, s, &[(0.6, 0.4), (0.6, 0.16)]);
        }
        // A save slot.
        Icon::Config => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.16, 0.18), p(r, 0.84, 0.82)),
                side * 0.08,
                s,
            );
            painter.rect_filled(
                Rect::from_min_max(p(r, 0.34, 0.18), p(r, 0.66, 0.42)),
                0.0,
                color,
            );
            path(painter, r, s, &[(0.32, 0.82), (0.32, 0.58), (0.68, 0.58), (0.68, 0.82)]);
        }
        // A level meter.
        Icon::Test => {
            for (x, h) in [(0.26f32, 0.34f32), (0.5, 0.6), (0.74, 0.46)] {
                path(painter, r, s, &[(x, 0.82), (x, 0.82 - h)]);
            }
            path(painter, r, s, &[(0.12, 0.86), (0.88, 0.86)]);
        }
        // A gear, drawn as a hub with spokes.
        Icon::Settings => {
            painter.circle_stroke(r.center(), side * 0.18, s);
            for i in 0..6 {
                let a = i as f32 / 6.0 * std::f32::consts::TAU;
                let (sn, cs) = a.sin_cos();
                let inner = Pos2::new(
                    r.center().x + cs * side * 0.26,
                    r.center().y + sn * side * 0.26,
                );
                let outer = Pos2::new(
                    r.center().x + cs * side * 0.42,
                    r.center().y + sn * side * 0.42,
                );
                painter.line_segment([inner, outer], s);
            }
        }
        // A snowflake: three crossing axes.
        Icon::Freeze => {
            for i in 0..3 {
                let a = i as f32 / 3.0 * std::f32::consts::PI;
                let (sn, cs) = a.sin_cos();
                let d = Vec2::new(cs * side * 0.4, sn * side * 0.4);
                painter.line_segment([r.center() - d, r.center() + d], s);
            }
        }
        Icon::Play => {
            painter.add(Shape::convex_polygon(
                vec![p(r, 0.28, 0.2), p(r, 0.8, 0.5), p(r, 0.28, 0.8)],
                color,
                Stroke::NONE,
            ));
        }
    }
}

/// A toolbar tab: icon, label, and an accent underline when it is showing.
pub fn tab(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> egui::Response {
    let galley = egui::WidgetText::from(label).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let pad = ui.spacing().button_padding;
    let icon_size = ui.text_style_height(&egui::TextStyle::Button) * 1.1;
    let gap = 5.0;
    let desired = Vec2::new(
        pad.x * 2.0 + icon_size + gap + galley.size().x,
        galley.size().y.max(icon_size) + pad.y * 2.0,
    );
    let (rect, response) = ui.allocate_at_least(desired, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.style().interact_selectable(&response, selected);
        if selected || response.hovered() {
            ui.painter()
                .rect(rect, v.rounding, v.weak_bg_fill, Stroke::NONE);
        }
        let color = if selected {
            theme::ACCENT_SOFT
        } else {
            v.fg_stroke.color
        };
        let icon_rect = Rect::from_center_size(
            Pos2::new(rect.left() + pad.x + icon_size * 0.5, rect.center().y),
            Vec2::splat(icon_size),
        );
        draw(ui.painter(), icon_rect, icon, color);
        ui.painter().galley(
            Pos2::new(
                icon_rect.right() + gap,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            color,
        );
        if selected {
            ui.painter().rect_filled(
                Rect::from_min_max(
                    Pos2::new(rect.left() + 3.0, rect.bottom() - 2.0),
                    Pos2::new(rect.right() - 3.0, rect.bottom()),
                ),
                1.0,
                theme::ACCENT,
            );
        }
    }
    response
}
