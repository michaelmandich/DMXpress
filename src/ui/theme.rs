//! The DMXpress look: one dark console theme applied to every panel, window
//! and popup, independent of the operating system's light/dark preference.
//!
//! Dark for the room — a lighting desk sits next to the stage, so the
//! surfaces stay near-black and never throw light — but with real contrast:
//! bright type, clearly lit controls, a backdrop that drops away behind the
//! panels. Teal is the single interactive accent (selection, focus, active
//! state) and the warm colours are reserved for meaning — amber warns, red
//! is destructive, green confirms.
//!
//! Depth comes from light, not lines. Surfaces are layered by lightness
//! (backdrop, panel, raised control) and every raised control is lit from
//! above: a soft vertical gradient, a hairline rim on top and a one-pixel
//! shadow beneath. egui only knows flat fills, so that relief is added in a
//! single pass over the frame's shapes ([`relief_pass`]) rather than at each
//! of the several hundred call sites — see that function for the rules.
//!
//! Type is Inter (with a medium weight for controls and a semibold for
//! headings, since egui cannot synthesise weight) and JetBrains Mono for
//! addresses, logs and channel values. Both are embedded so the console
//! looks identical on every machine.

use std::sync::Arc;

use eframe::egui::{
    self, epaint, style::Selection, style::WidgetVisuals, style::Widgets, Color32, FontFamily,
    FontId, Mesh, Pos2, Rect, Rounding, Shadow, Shape, Stroke, TextStyle, Visuals,
};

// ---- palette ----

/// Deepest layer: the app backdrop and the 3D stage surround.
pub const BACKDROP: Color32 = Color32::from_rgb(0x0C, 0x0E, 0x11);
/// Panels, side bars and window bodies.
pub const SURFACE: Color32 = Color32::from_rgb(0x1A, 0x1D, 0x22);
/// The lit top of a header strip or the toolbar, fading down to [`SURFACE`].
pub const SURFACE_LIT: Color32 = Color32::from_rgb(0x27, 0x2C, 0x33);
/// Raised controls sitting on a panel (buttons, combo boxes, tiles).
pub const RAISED: Color32 = Color32::from_rgb(0x2E, 0x34, 0x3C);
/// Hovered control.
pub const HOVER: Color32 = Color32::from_rgb(0x3B, 0x43, 0x4D);
/// Slate used for dividers, inactive outlines and header underlines.
pub const EDGE: Color32 = Color32::from_rgb(0x33, 0x3A, 0x43);
/// The lit upper edge of a raised control.
pub const RIM: Color32 = Color32::from_rgb(0x48, 0x50, 0x5A);
/// Text-entry wells and scroll troughs: darker than the panel they sit in.
pub const WELL: Color32 = Color32::from_rgb(0x0F, 0x11, 0x14);

/// Interactive accent: selection, focus rings, active toggles.
pub const ACCENT: Color32 = Color32::from_rgb(0x1C, 0x8E, 0x98);
/// Lighter accent for text on charcoal, links and hovered accents.
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0x82, 0xDB, 0xD8);
/// Muted teal for secondary emphasis.
pub const ACCENT_MUTED: Color32 = Color32::from_rgb(0x3F, 0xA2, 0xAA);

/// Primary text.
pub const TEXT: Color32 = Color32::from_rgb(0xF2, 0xF6, 0xF7);
/// Secondary text: hints, units, inactive labels.
pub const TEXT_DIM: Color32 = Color32::from_rgb(0xA8, 0xB3, 0xB9);

/// Something is running / succeeded.
pub const OK: Color32 = Color32::from_rgb(0x8F, 0xD0, 0x7F);
/// Caution: unsaved, degraded, standing in for missing data.
pub const WARN: Color32 = Color32::from_rgb(0xF5, 0xC6, 0x3A);
/// Destructive or failed.
pub const DANGER: Color32 = Color32::from_rgb(0xE0, 0x39, 0x2B);

// ---- geometry ----

const R_WIDGET: f32 = 5.0;
const R_WINDOW: f32 = 10.0;

/// Inner margin of the docked side/top panels. Header strips extend back
/// out by this much so they run edge to edge.
pub const PANEL_MARGIN: egui::Margin = egui::Margin {
    left: 10.0,
    right: 10.0,
    top: 8.0,
    bottom: 8.0,
};

// ---- type ----

static INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
static INTER_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.ttf");
static INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
static JETBRAINS_MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");

/// Inter Medium: the weight controls and labels that must read at a glance
/// are set in.
pub fn medium() -> FontFamily {
    FontFamily::Name(Arc::from("inter-medium"))
}

/// Inter SemiBold: headings and window titles.
pub fn semibold() -> FontFamily {
    FontFamily::Name(Arc::from("inter-semibold"))
}

fn fonts() -> egui::FontDefinitions {
    let mut defs = egui::FontDefinitions::default();
    defs.font_data
        .insert("inter".to_owned(), egui::FontData::from_static(INTER));
    defs.font_data
        .insert("inter-medium".to_owned(), egui::FontData::from_static(INTER_MEDIUM));
    defs.font_data
        .insert("inter-semibold".to_owned(), egui::FontData::from_static(INTER_SEMIBOLD));
    defs.font_data
        .insert("jetbrains-mono".to_owned(), egui::FontData::from_static(JETBRAINS_MONO));

    // Inter first, then egui's bundled faces stay behind it as fallbacks for
    // the symbols it lacks (⇧ ⌘ ⧉ and emoji).
    defs.families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".to_owned());
    defs.families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "jetbrains-mono".to_owned());
    let fallbacks = defs.families[&FontFamily::Proportional].clone();
    for (family, face) in [(medium(), "inter-medium"), (semibold(), "inter-semibold")] {
        let mut chain = fallbacks.clone();
        chain[0] = face.to_owned();
        defs.families.insert(family, chain);
    }
    defs
}

// ---- visuals ----

/// Build the theme. Kept separate from [`install`] so panels can restyle a
/// sub-`Ui` from the same source of truth.
pub fn visuals() -> Visuals {
    let widget = |bg: Color32, weak: Color32, edge: Color32, fg: Color32, expansion: f32| {
        WidgetVisuals {
            bg_fill: bg,
            weak_bg_fill: weak,
            bg_stroke: Stroke::new(1.0, edge),
            rounding: Rounding::same(R_WIDGET),
            fg_stroke: Stroke::new(1.0, fg),
            expansion,
        }
    };

    Visuals {
        dark_mode: true,
        widgets: Widgets {
            // Non-interactive: labels, separators, panel frames.
            noninteractive: widget(SURFACE, SURFACE, EDGE, TEXT_DIM, 0.0),
            // Resting interactive control: lit rim on a raised fill. The
            // fill colours below double as the sentinels `relief_pass`
            // looks for, so keep them exactly these constants.
            inactive: widget(RAISED, RAISED, RIM, TEXT, 0.0),
            // Pointer over it: lift, and hint the accent on the outline.
            hovered: widget(HOVER, HOVER, ACCENT_MUTED, TEXT, 1.0),
            // Held down / toggled on: the accent takes over.
            active: widget(ACCENT, ACCENT, ACCENT_SOFT, TEXT, 1.0),
            // Keyboard focus / open combo.
            open: widget(RAISED, RAISED, ACCENT_MUTED, TEXT, 0.0),
        },
        selection: Selection {
            bg_fill: ACCENT,
            stroke: Stroke::new(1.0, TEXT),
        },
        hyperlink_color: ACCENT_SOFT,
        // Striped rows and hovered list entries.
        faint_bg_color: Color32::from_rgb(0x21, 0x25, 0x2B),
        // Text edits, sliders' troughs, plot backgrounds.
        extreme_bg_color: WELL,
        code_bg_color: Color32::from_rgb(0x14, 0x17, 0x1B),
        warn_fg_color: WARN,
        error_fg_color: DANGER,

        window_rounding: Rounding::same(R_WINDOW),
        window_fill: SURFACE,
        window_stroke: Stroke::new(1.0, Color32::from_rgb(0x3E, 0x46, 0x50)),
        window_shadow: Shadow {
            offset: egui::vec2(0.0, 12.0),
            blur: 32.0,
            spread: 0.0,
            color: Color32::from_black_alpha(170),
        },
        menu_rounding: Rounding::same(R_WIDGET + 2.0),
        popup_shadow: Shadow {
            offset: egui::vec2(0.0, 6.0),
            blur: 20.0,
            spread: 0.0,
            color: Color32::from_black_alpha(150),
        },
        panel_fill: SURFACE,

        // Slider fill up to the handle reads as "how much", like a meter.
        slider_trailing_fill: true,
        striped: false,
        indent_has_left_vline: true,
        ..Visuals::dark()
    }
}

/// Apply the theme to a context: fonts, colours, spacing and text sizes.
pub fn install(ctx: &egui::Context) {
    // The console has one look on every platform. Without this the app
    // inherits the OS preference, which left Windows with a white backdrop
    // under charcoal panels.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_fonts(fonts());

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals();

    // Inter sits larger on the line than egui's default Ubuntu Light, so
    // these are a touch under what the numbers suggest.
    style.text_styles = [
        (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.0, medium())),
        (TextStyle::Heading, FontId::new(16.0, semibold())),
        (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
    ]
    .into();

    let sp = &mut style.spacing;
    sp.item_spacing = egui::vec2(8.0, 6.0);
    sp.button_padding = egui::vec2(10.0, 5.0);
    sp.menu_margin = egui::Margin::same(8.0);
    sp.window_margin = egui::Margin::same(12.0);
    sp.indent = 18.0;
    sp.slider_width = 150.0;
    sp.slider_rail_height = 6.0;
    sp.combo_width = 110.0;
    // Bigger minimum hit targets: a desk gets driven by hand, fast.
    sp.interact_size = egui::vec2(40.0, 22.0);
    sp.icon_width = 16.0;
    sp.icon_width_inner = 10.0;
    sp.tooltip_width = 380.0;
    sp.scroll.bar_width = 10.0;
    sp.scroll.floating = false;
    // A raised thumb in a recessed trough, not a bar of text colour.
    sp.scroll.foreground_color = false;

    ctx.set_style(style.clone());
    // Pin it as *the* dark style too, so nothing re-derives egui's default
    // palette if the theme is re-evaluated.
    ctx.options_mut(|o| o.dark_style = std::sync::Arc::new(style));
}

/// The frame docked panels use, so their header strips can reach the edge.
pub fn panel_frame(style: &egui::Style) -> egui::Frame {
    egui::Frame::side_top_panel(style).inner_margin(PANEL_MARGIN)
}

/// [`install`], then patch a plugin's colour tint over the visuals. Only the
/// style is touched — the hand-painted `theme::*` constants (relief pass,
/// stage chrome) keep the stock look, so a tint restyles fills, accents and
/// text without breaking any painting that samples the palette directly.
pub fn install_with(ctx: &egui::Context, o: &crate::plugin::ThemeOverride) {
    install(ctx);
    if o.is_empty() {
        return;
    }
    let c = |rgb: [u8; 3]| Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    if let Some(a) = o.accent {
        let accent = c(a);
        let soft = accent.lerp_to_gamma(Color32::WHITE, 0.45);
        v.widgets.active.bg_fill = accent;
        v.widgets.active.weak_bg_fill = accent;
        v.widgets.active.bg_stroke = Stroke::new(1.0, soft);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, accent.lerp_to_gamma(Color32::WHITE, 0.2));
        v.widgets.open.bg_stroke = Stroke::new(1.0, accent);
        v.selection.bg_fill = accent;
        v.hyperlink_color = soft;
    }
    if let Some(sf) = o.surface {
        let surface = c(sf);
        let raised = surface.lerp_to_gamma(Color32::WHITE, 0.10);
        let hover = surface.lerp_to_gamma(Color32::WHITE, 0.18);
        v.panel_fill = surface;
        v.window_fill = surface;
        v.widgets.noninteractive.bg_fill = surface;
        v.widgets.noninteractive.weak_bg_fill = surface;
        v.widgets.inactive.bg_fill = raised;
        v.widgets.inactive.weak_bg_fill = raised;
        v.widgets.open.bg_fill = raised;
        v.widgets.open.weak_bg_fill = raised;
        v.widgets.hovered.bg_fill = hover;
        v.widgets.hovered.weak_bg_fill = hover;
        v.faint_bg_color = surface.lerp_to_gamma(Color32::WHITE, 0.05);
        v.extreme_bg_color = surface.lerp_to_gamma(Color32::BLACK, 0.45);
    }
    if let Some(t) = o.text {
        let text = c(t);
        let dim = text.lerp_to_gamma(Color32::BLACK, 0.3);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, dim);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, text);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, text);
        v.widgets.active.fg_stroke = Stroke::new(1.0, text);
        v.widgets.open.fg_stroke = Stroke::new(1.0, text);
        v.selection.stroke = Stroke::new(1.0, text);
    }
    ctx.set_style(style.clone());
    ctx.options_mut(|opt| opt.dark_style = Arc::new(style));
}

// ---- painting helpers ----

pub(crate) fn lighten(c: Color32, k: f32) -> Color32 {
    c.lerp_to_gamma(Color32::WHITE, k)
}

pub(crate) fn darken(c: Color32, k: f32) -> Color32 {
    c.lerp_to_gamma(Color32::BLACK, k)
}

/// A rounded rectangle filled with a vertical gradient, `top` to `bottom`.
///
/// The mesh has no anti-aliased feather; draw a stroke over it, or keep it
/// inside something that does.
pub fn vgradient_mesh(rect: Rect, rounding: Rounding, top: Color32, bottom: Color32) -> Mesh {
    let mut path = Vec::with_capacity(36);
    epaint::tessellator::path::rounded_rectangle(&mut path, rect, rounding);
    let mut mesh = Mesh::default();
    let h = rect.height().max(1.0);
    for p in &path {
        let t = ((p.y - rect.top()) / h).clamp(0.0, 1.0);
        mesh.colored_vertex(*p, top.lerp_to_gamma(bottom, t));
    }
    for i in 2..path.len() as u32 {
        mesh.add_triangle(0, i - 1, i);
    }
    mesh
}

/// A disc filled with a radial gradient, `inner` at the centre fading to
/// `outer` at the rim. Unfeathered like [`vgradient_mesh`].
pub fn radial_mesh(center: Pos2, radius: f32, inner: Color32, outer: Color32) -> Mesh {
    const N: u32 = 48;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, inner);
    for i in 0..N {
        let a = i as f32 / N as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(center + egui::vec2(a.cos(), a.sin()) * radius, outer);
    }
    for i in 0..N {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % N);
    }
    mesh
}

/// A ring between two radii whose colour changes around it. `color_at`
/// gets the angle in radians, 0 pointing right and increasing clockwise
/// on screen — so the upper left is at 5π/4 — which is what a bezel lit
/// from one side wants.
pub fn ring_mesh(
    center: Pos2,
    r_in: f32,
    r_out: f32,
    color_at: impl Fn(f32) -> Color32,
) -> Mesh {
    const N: u32 = 48;
    let mut mesh = Mesh::default();
    for i in 0..N {
        let a = i as f32 / N as f32 * std::f32::consts::TAU;
        let d = egui::vec2(a.cos(), a.sin());
        let c = color_at(a);
        mesh.colored_vertex(center + d * r_in, c);
        mesh.colored_vertex(center + d * r_out, c);
    }
    for i in 0..N {
        let j = (i + 1) % N;
        mesh.add_triangle(2 * i, 2 * i + 1, 2 * j);
        mesh.add_triangle(2 * i + 1, 2 * j + 1, 2 * j);
    }
    mesh
}

/// A convex polygon with a colour per vertex from `color_at(point)`; the
/// fill interpolates between them.
pub fn convex_mesh(points: &[Pos2], color_at: impl Fn(Pos2) -> Color32) -> Mesh {
    let mut mesh = Mesh::default();
    for p in points {
        mesh.colored_vertex(*p, color_at(*p));
    }
    for i in 2..points.len() as u32 {
        mesh.add_triangle(0, i - 1, i);
    }
    mesh
}

/// The toolbar's backdrop: lit at the top, falling away to the panel tone,
/// with a highlight along its top edge and a shadowed lower edge that the
/// panels below appear to sit under.
pub fn toolbar_backdrop(rect: Rect) -> Shape {
    let mut shapes = Vec::with_capacity(4);
    shapes.push(Shape::mesh(vgradient_mesh(
        rect,
        Rounding::ZERO,
        SURFACE_LIT,
        Color32::from_rgb(0x17, 0x1A, 0x1E),
    )));
    // Lit top edge.
    shapes.push(Shape::hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0, Color32::from_rgb(0x3E, 0x45, 0x4E)),
    ));
    // Shadow gathering at the bottom, then a hard dark edge.
    let shade = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 6.0), rect.max);
    shapes.push(Shape::mesh(vgradient_mesh(
        shade,
        Rounding::ZERO,
        Color32::TRANSPARENT,
        Color32::from_black_alpha(90),
    )));
    shapes.push(Shape::hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, Color32::from_rgb(0x07, 0x08, 0x0A)),
    ));
    Shape::Vec(shapes)
}

/// A raised control drawn with depth: shadow beneath, gradient body, rim.
fn lifted(rect: Rect, rounding: Rounding, fill: Color32, stroke: Stroke) -> Shape {
    let mut shapes = Vec::with_capacity(3);
    shapes.push(Shape::rect_filled(
        rect.translate(egui::vec2(0.0, 1.0)),
        rounding,
        Color32::from_black_alpha(110),
    ));
    shapes.push(Shape::mesh(vgradient_mesh(
        rect,
        rounding,
        lighten(fill, 0.09),
        darken(fill, 0.10),
    )));
    if stroke != Stroke::NONE {
        shapes.push(Shape::rect_stroke(rect, rounding, stroke));
    } else {
        // A hairline in the fill's own tone hides the unfeathered mesh edge.
        shapes.push(Shape::rect_stroke(rect, rounding, Stroke::new(1.0, fill)));
    }
    Shape::Vec(shapes)
}

/// Give a shape relief if it is a control body. See [`relief_pass`].
fn relieve(shape: &mut Shape) {
    match shape {
        Shape::Vec(inner) => inner.iter_mut().for_each(relieve),
        Shape::Rect(r) => {
            let is_control = r.fill == RAISED || r.fill == HOVER || r.fill == ACCENT;
            if !is_control
                || r.rounding == Rounding::ZERO
                || r.fill_texture_id != egui::TextureId::default()
                || r.rect.height() < 10.0
                || r.rect.width() < 10.0
            {
                return;
            }
            *shape = lifted(r.rect, r.rounding, r.fill, r.stroke);
        }
        _ => {}
    }
}

/// Lift every raised control drawn this frame.
///
/// egui paints widgets as flat rounded rectangles. This walks the frame's
/// shape lists after the UI has been built and replaces each rectangle
/// whose fill is one of the three control tones ([`RAISED`], [`HOVER`],
/// [`ACCENT`]) with a lit version: a vertical gradient, the rim stroke the
/// widget asked for, and a one-pixel shadow beneath. Rectangles with square
/// corners are left alone — text selection, meters and rails use those —
/// as is anything too small to show a gradient.
///
/// Call it once, at the end of the frame's UI, before egui tessellates.
pub fn relief_pass(ctx: &egui::Context) {
    let mut layers: Vec<egui::LayerId> =
        ctx.memory(|m| m.areas().visible_layer_ids().into_iter().collect());
    layers.push(egui::LayerId::background());
    ctx.graphics_mut(|g| {
        for layer in layers {
            let list = g.entry(layer);
            let n = list.next_idx().0;
            for i in 0..n {
                list.mutate_shape(egui::layers::ShapeIdx(i), |cs| relieve(&mut cs.shape));
            }
        }
    });
}

// ---- composite widgets ----

/// The title strip of a docked panel: a lit gradient band running edge to
/// edge with the heading on the left and `right` (zoom steppers, buttons)
/// aligned to the right.
///
/// The band is painted behind the row once the row's height is known, so
/// it fits whatever `right` puts in it.
pub fn panel_header(ui: &mut egui::Ui, title: &str, right: impl FnOnce(&mut egui::Ui)) {
    let idx = ui.painter().add(Shape::Noop);
    let row = ui
        .horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // The controls claim their room first; the title gets what
                // is left and trails off rather than being painted over.
                ui.spacing_mut().item_spacing.x = 5.0;
                right(ui);
                let avail = ui.available_rect_before_wrap();
                let galley = egui::WidgetText::from(
                    egui::RichText::new(title).text_style(TextStyle::Heading).color(TEXT),
                )
                .into_galley(
                    ui,
                    Some(egui::TextWrapMode::Truncate),
                    (avail.width() - 4.0).max(0.0),
                    TextStyle::Heading,
                );
                let h = galley.size().y.max(ui.spacing().interact_size.y);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(avail.width(), h), egui::Sense::hover());
                ui.painter().galley(
                    Pos2::new(rect.left() + 2.0, rect.center().y - galley.size().y * 0.5),
                    galley,
                    TEXT,
                );
            });
        })
        .response
        .rect;
    let band = Rect::from_min_max(
        Pos2::new(ui.max_rect().left() - PANEL_MARGIN.left, row.top() - PANEL_MARGIN.top),
        Pos2::new(ui.max_rect().right() + PANEL_MARGIN.right, row.bottom() + 7.0),
    );
    ui.painter().set(
        idx,
        Shape::Vec(vec![
            Shape::mesh(vgradient_mesh(band, Rounding::ZERO, SURFACE_LIT, SURFACE)),
            Shape::hline(
                band.x_range(),
                band.bottom() - 0.5,
                Stroke::new(1.0, Color32::from_rgb(0x0A, 0x0C, 0x0E)),
            ),
        ]),
    );
    ui.add_space(8.0);
}

/// Heading with a hairline rule under it — the standard section break inside
/// windows and side panels.
pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(title)
            .color(TEXT)
            .family(semibold())
            .size(13.0),
    );
    let rect = ui.available_rect_before_wrap();
    let y = rect.top() + 2.0;
    ui.painter()
        .hline(rect.x_range(), y, Stroke::new(1.0, EDGE));
    ui.add_space(6.0);
}

/// Small dimmed caption used for hints under a control.
pub fn hint(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(
        egui::RichText::new(text.into())
            .color(TEXT_DIM)
            .size(11.0),
    );
}

/// A status pill: coloured text on a tinted rounded background.
pub fn pill(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.5)))
        .rounding(Rounding::same(R_WIDGET))
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .color(color)
                    .family(medium())
                    .size(11.0),
            );
        })
        .response
}

// ---- inspector kit ----

/// The vocabulary the Inspector's tabs are written in, so five areas produce
/// one panel: cards and wells, tool and toggle buttons, pills and chips,
/// list rows, folds, pads and the segmented control. Every hand-painted
/// pixel size is multiplied by [`zoom_of`] so custom paint tracks A−/A+.
///
/// Helpers that use a derived fill (`gamma_multiply`, `lerp_to_gamma`) lose
/// the automatic lift of [`relief_pass`], so they paint their own shadow,
/// gradient and stroke; helpers that go through stock widgets keep the
/// exact `RAISED` / `HOVER` / `ACCENT` fills the pass looks for.
mod kit {
    use std::hash::Hash;
    use std::sync::Arc;

    use eframe::egui::{
        self, Align, Align2, Color32, FontId, Galley, InnerResponse, Layout, Margin, Painter,
        Pos2, Rect, Response, RichText, Rounding, Sense, Shape, Stroke, TextStyle, TextWrapMode,
        Ui, Vec2, WidgetText,
    };

    use super::{
        darken, lighten, medium, pill, semibold, vgradient_mesh, ACCENT, ACCENT_MUTED,
        ACCENT_SOFT, DANGER, EDGE, HOVER, RAISED, RIM, SURFACE, TEXT, TEXT_DIM, WARN, WELL,
    };
    use crate::ui::icons::{self, Icon};

    /// Corner radius of a pad.
    pub const R_PAD: f32 = 6.0;
    /// Corner radius of a card.
    pub const R_CARD: f32 = 8.0;

    /// Text colour that reads on `bg`.
    pub fn readable_on(bg: Color32) -> Color32 {
        let lum = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
        if lum > 150.0 {
            Color32::from_rgb(0x12, 0x14, 0x18)
        } else {
            TEXT
        }
    }

    /// The panel's zoom, read back from the Body text size `apply_zoom`
    /// scaled (13 px at zoom 1).
    pub fn zoom_of(ui: &Ui) -> f32 {
        ui.style().text_styles.get(&TextStyle::Body).map_or(1.0, |f| f.size / 13.0)
    }

    /// Single-line galley truncated with an ellipsis to `max_w`.
    fn ellipsis(p: &Painter, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<Galley> {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
        job.wrap = egui::text::TextWrapping {
            max_width: max_w.max(8.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('\u{2026}'),
        };
        p.layout_job(job)
    }

    /// A raised group: `RAISED`, an `EDGE` hairline, 8 px corners, 10 px
    /// inner margin. Lifted by the relief pass. Cards do not nest — put a
    /// [`well`] inside one instead.
    pub fn card<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        egui::Frame::none()
            .fill(RAISED)
            .stroke(Stroke::new(1.0, EDGE))
            .rounding(Rounding::same(R_CARD))
            .inner_margin(Margin::same(10.0))
            .show(ui, body)
    }

    /// A flat recess for lists, drop targets and previews.
    pub fn well<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        egui::Frame::none()
            .fill(WELL)
            .stroke(Stroke::new(1.0, EDGE))
            .rounding(Rounding::same(6.0))
            .inner_margin(Margin::same(8.0))
            .show(ui, body)
    }

    /// The title row of a [`card`]: semibold title on the left, `right` laid
    /// out right-to-left. No rule — the card already has an edge.
    pub fn card_title(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).family(semibold()).size(13.0).color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                right(ui);
            });
        });
        ui.add_space(4.0);
    }

    /// [`super::section`] with a right-hand slot on the title row (count
    /// pills, a tool button, a help glyph). Vertical flow only.
    pub fn section_with(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).color(TEXT).family(semibold()).size(13.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                right(ui);
            });
        });
        let rect = ui.available_rect_before_wrap();
        let y = rect.top() + 2.0;
        ui.painter().hline(rect.x_range(), y, Stroke::new(1.0, EDGE));
        ui.add_space(6.0);
    }

    /// A wrapping row of square icon buttons, packed tight.
    /// Controls built from the kit claim their own space, so the row breaks
    /// at the panel edge instead of running off it. A raw `ui.scope` or
    /// `add_enabled_ui` inside one will not — see [`wrapped_child`].
    ///
    /// To decide something yourself part-way along a row — whether a caption
    /// still fits, say — do not ask `Ui::available_width`: inside a wrapping
    /// layout it reports the whole row, meaning "you may have this much, I
    /// will fold you onto the next line". Measure from the cursor instead,
    /// `ui.max_rect().right() - ui.cursor().min.x`.
    pub fn toolbar<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            ui.spacing_mut().button_padding = egui::vec2(6.0, 4.0);
            body(ui)
        })
        .inner
    }

    /// The size [`icons::icon_button`] will take in this `Ui`.
    ///
    /// Public because a tab that lays a row out by hand needs the same number.
    ///
    /// It mirrors that function's arithmetic so a wrapping row can claim the
    /// space before the button is built; `tool_size_matches_the_button` holds
    /// the two together.
    pub fn tool_size(ui: &Ui, label: Option<&str>) -> Vec2 {
        let galley = label.map(|l| {
            WidgetText::from(l).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Button)
        });
        let pad = ui.spacing().button_padding;
        let icon = ui.text_style_height(&TextStyle::Button) * 1.05;
        let px = if galley.is_some() { pad.x } else { pad.y + 2.0 };
        let text_w = galley.as_ref().map_or(0.0, |g| g.size().x + 5.0);
        let h = galley.as_ref().map_or(icon, |g| g.size().y.max(icon)) + pad.y * 2.0;
        Vec2::new(px * 2.0 + icon + text_w, h)
    }

    /// Build `add` inside a child `Ui` of exactly `size`, claiming that space
    /// from the parent first.
    ///
    /// egui only wraps a row at `allocate_space`. A child `Ui` — which is what
    /// `scope` and `add_enabled_ui` create — is placed afterwards with
    /// `allocate_rect`, and that never wraps: a toolbar built from scoped
    /// controls runs off the panel instead, and a side panel sized by its
    /// contents then grows every frame to chase it. Claiming the space up
    /// front puts the break back where the layout can see it.
    fn wrapped_child<R>(ui: &mut Ui, size: Vec2, add: impl FnOnce(&mut Ui) -> R) -> R {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        add(&mut child)
    }

    /// A square icon button with a tooltip; `Err(why)` disables it and
    /// shows `why` instead.
    pub fn tool_button(
        ui: &mut Ui,
        icon: Icon,
        label: Option<&str>,
        hint: &str,
        enabled: Result<(), &str>,
    ) -> Response {
        let size = tool_size(ui, label);
        let r = wrapped_child(ui, size, |ui| {
            ui.add_enabled_ui(enabled.is_ok(), |ui| icons::icon_button(ui, icon, label)).inner
        });
        r.on_hover_text(hint).on_disabled_hover_text(enabled.err().unwrap_or(hint))
    }

    /// An icon button whose body turns `ACCENT` while `on` (the Freeze
    /// pattern in the toolbar), so the relief pass still lifts it.
    pub fn toggle_icon(ui: &mut Ui, icon: Icon, label: Option<&str>, on: bool, hint: &str) -> Response {
        // Swapped in place rather than in a `scope`, so the button still does
        // its own allocation and a wrapping row can break before it — see
        // `wrapped_child` for why a child `Ui` cannot.
        let saved = on.then(|| ui.visuals().widgets.clone());
        if on {
            let v = &mut ui.visuals_mut().widgets;
            v.inactive.weak_bg_fill = ACCENT;
            v.inactive.bg_stroke = Stroke::new(1.0, ACCENT_SOFT);
            v.hovered.weak_bg_fill = ACCENT;
            v.hovered.bg_stroke = Stroke::new(1.0, TEXT);
        }
        let r = icons::icon_button(ui, icon, label);
        if let Some(widgets) = saved {
            ui.visuals_mut().widgets = widgets;
        }
        r.on_hover_text(hint)
    }

    /// Disable any control with a full-sentence reason shown on hover.
    ///
    /// It wraps the control in a child `Ui`, which a wrapping row cannot break
    /// before (see [`wrapped_child`]), so inside a [`toolbar`] reach for
    /// [`tool_button`]'s own `enabled` argument instead.
    pub fn gated(ui: &mut Ui, enabled: Result<(), &str>, add: impl FnOnce(&mut Ui) -> Response) -> Response {
        let r = ui.add_enabled_ui(enabled.is_ok(), add).inner;
        match enabled {
            Ok(()) => r,
            Err(why) => r.on_disabled_hover_text(why),
        }
    }

    /// A stock button on the accent fill.
    pub fn accent_button(ui: &mut Ui, text: &str) -> Response {
        let saved = ui.visuals().widgets.clone();
        {
            let v = &mut ui.visuals_mut().widgets;
            v.inactive.weak_bg_fill = ACCENT;
            v.inactive.bg_stroke = Stroke::new(1.0, ACCENT_SOFT);
            v.hovered.weak_bg_fill = ACCENT;
            v.hovered.bg_stroke = Stroke::new(1.0, TEXT);
        }
        let r = ui.add(egui::Button::new(RichText::new(text).color(TEXT)));
        ui.visuals_mut().widgets = saved;
        r
    }

    /// A stock button outlined and lettered in `DANGER`; the fill stays
    /// `RAISED` so it is still lifted.
    pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
        let saved = ui.visuals().widgets.clone();
        {
            let v = &mut ui.visuals_mut().widgets;
            v.inactive.bg_stroke = Stroke::new(1.0, DANGER.gamma_multiply(0.6));
            v.hovered.bg_stroke = Stroke::new(1.0, DANGER);
        }
        let r = ui.add(egui::Button::new(RichText::new(text).color(DANGER)));
        ui.visuals_mut().widgets = saved;
        r
    }

    /// The colour a [`wide_button`] wears.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Tone {
        Accent,
        Danger,
    }

    /// The one hero action of a tab: full width, 1.45× the interact height,
    /// self-painted with shadow, gradient and rim.
    pub fn wide_button(ui: &mut Ui, icon: Option<Icon>, label: &str, tone: Tone) -> Response {
        let z = zoom_of(ui);
        let h = ui.spacing().interact_size.y * 1.45;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
        if ui.is_rect_visible(rect) {
            let (fill, stroke) = match tone {
                Tone::Accent => (ACCENT.lerp_to_gamma(SURFACE, 0.25), ACCENT_SOFT),
                Tone::Danger => (RAISED, DANGER.gamma_multiply(0.7)),
            };
            let fill = if resp.is_pointer_button_down_on() {
                darken(fill, 0.08)
            } else if resp.hovered() {
                lighten(fill, 0.08)
            } else {
                fill
            };
            let rounding = Rounding::same(6.0);
            let p = ui.painter();
            p.rect_filled(rect.translate(Vec2::new(0.0, 1.0)), rounding, Color32::from_black_alpha(120));
            p.add(Shape::mesh(vgradient_mesh(rect, rounding, lighten(fill, 0.10), darken(fill, 0.14))));
            p.rect_stroke(rect, rounding, Stroke::new(1.0, stroke));
            let ink = match tone {
                Tone::Danger => DANGER,
                Tone::Accent => readable_on(fill),
            };
            let galley = p.layout_no_wrap(label.to_owned(), FontId::new(13.0 * z, medium()), ink);
            let icon_size = ui.text_style_height(&TextStyle::Button) * 1.2;
            let gap = 6.0 * z;
            let total = galley.size().x + icon.map_or(0.0, |_| icon_size + gap);
            let mut x = rect.center().x - total * 0.5;
            if let Some(i) = icon {
                let ir = Rect::from_center_size(
                    Pos2::new(x + icon_size * 0.5, rect.center().y),
                    Vec2::splat(icon_size),
                );
                icons::draw(p, ir, i, ink);
                x += icon_size + gap;
            }
            p.galley(Pos2::new(x, rect.center().y - galley.size().y * 0.5), galley, ink);
        }
        resp
    }

    /// A two-column label / value grid. Values are `DragValue`s no wider
    /// than 64 px, or sliders sized to the row.
    pub fn kv_grid<R>(ui: &mut Ui, salt: impl Hash, body: impl FnOnce(&mut Ui) -> R) -> R {
        egui::Grid::new(salt)
            .num_columns(2)
            .spacing([10.0, 4.0])
            .min_col_width(64.0)
            .show(ui, body)
            .inner
    }

    /// A dimmed 12 px label, for the left column of a row.
    pub fn label_dim(ui: &mut Ui, text: &str) {
        ui.label(RichText::new(text).color(TEXT_DIM).size(12.0));
    }

    /// "3 lights" as a pill: accent while there are some, dim at zero.
    pub fn count_pill(ui: &mut Ui, n: usize, noun: &str) -> Response {
        let text = format!("{n} {noun}{}", if n == 1 { "" } else { "s" });
        pill(ui, &text, if n > 0 { ACCENT_SOFT } else { TEXT_DIM })
    }

    /// What an empty list shows: a dim icon, a title and one line of advice.
    pub fn empty_state(ui: &mut Ui, icon: Icon, title: &str, hint: &str) {
        let z = zoom_of(ui);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0 * z), Sense::hover());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let p = ui.painter();
        let cx = rect.center().x;
        let icon_rect = Rect::from_center_size(Pos2::new(cx, rect.top() + 12.0 * z), Vec2::splat(22.0 * z));
        icons::draw(p, icon_rect, icon, TEXT_DIM.gamma_multiply(0.7));
        let title_g = ellipsis(p, title, FontId::new(13.0 * z, medium()), TEXT_DIM, rect.width() - 8.0);
        p.galley(
            Pos2::new(cx - title_g.size().x * 0.5, rect.top() + 26.0 * z),
            title_g,
            TEXT_DIM,
        );
        let hint_g = ellipsis(
            p,
            hint,
            FontId::new(11.0 * z, egui::FontFamily::Proportional),
            TEXT_DIM.gamma_multiply(0.8),
            rect.width() - 8.0,
        );
        p.galley(
            Pos2::new(cx - hint_g.size().x * 0.5, rect.top() + 42.0 * z),
            hint_g,
            TEXT_DIM.gamma_multiply(0.8),
        );
    }

    /// A lit colour chip: shadow, gradient, rim (an `EDGE` rim when the
    /// colour is near black so it keeps an outline).
    pub fn paint_swatch(p: &Painter, rect: Rect, color: Color32, rounding: f32) {
        let rounding = Rounding::same(rounding);
        p.rect_filled(rect.translate(Vec2::new(0.0, 1.0)), rounding, Color32::from_black_alpha(120));
        p.add(Shape::mesh(vgradient_mesh(rect, rounding, lighten(color, 0.12), darken(color, 0.16))));
        let lum = 0.299 * color.r() as f32 + 0.587 * color.g() as f32 + 0.114 * color.b() as f32;
        let rim = if lum < 40.0 { EDGE } else { RIM };
        p.rect_stroke(rect, rounding, Stroke::new(1.0, rim));
    }

    /// A swatch as a widget, `size` px square at zoom 1.
    pub fn swatch(ui: &mut Ui, color: Color32, size: f32) -> Response {
        let z = zoom_of(ui);
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size * z), Sense::hover());
        if ui.is_rect_visible(rect) {
            paint_swatch(ui.painter(), rect, color, 3.0 * z);
        }
        resp
    }

    /// A clickable filter pill. Its id comes from `text`: wrap duplicate
    /// texts in `push_id`.
    /// Two chips with the same text in one `Ui` collide, because the id comes
    /// from the text. Wrap the WHOLE row in one `ui.push_id`, never each chip:
    /// a scope per chip stops the row wrapping (see [`wrapped_child`]), and one
    /// scope around the row already makes the pair unique.
    pub fn chip(ui: &mut Ui, text: &str, color: Color32, selected: bool) -> Response {
        let z = zoom_of(ui);
        let galley = WidgetText::from(RichText::new(text).family(medium()).size(11.0 * z)).into_galley(
            ui,
            Some(TextWrapMode::Extend),
            f32::INFINITY,
            TextStyle::Body,
        );
        let pad = Vec2::new(6.0 * z, 2.0 * z);
        let (rect, _) = ui.allocate_exact_size(galley.size() + pad * 2.0, Sense::hover());
        let resp = ui.interact(rect, ui.id().with(("chip", text)), Sense::click());
        if ui.is_rect_visible(rect) {
            let rounding = Rounding::same(5.0);
            let p = ui.painter();
            if selected {
                let fill = if resp.hovered() { lighten(color, 0.08) } else { color };
                p.rect_filled(rect.translate(Vec2::new(0.0, 1.0)), rounding, Color32::from_black_alpha(120));
                p.add(Shape::mesh(vgradient_mesh(rect, rounding, lighten(fill, 0.10), darken(fill, 0.14))));
                p.rect_stroke(rect, rounding, Stroke::new(1.0, lighten(color, 0.3)));
                p.galley(rect.min + pad, galley, readable_on(fill));
            } else {
                let fill = color.gamma_multiply(if resp.hovered() { 0.26 } else { 0.18 });
                p.rect(rect, rounding, fill, Stroke::new(1.0, color.gamma_multiply(0.5)));
                p.galley(rect.min + pad, galley, color);
            }
        }
        resp
    }

    /// A lit key cap for legends: monospace on `RAISED` with a `RIM` edge.
    pub fn keycap(ui: &mut Ui, key: &str) {
        let z = zoom_of(ui);
        egui::Frame::none()
            .fill(RAISED)
            .stroke(Stroke::new(1.0, RIM))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(5.0, 1.0))
            .show(ui, |ui| {
                ui.set_min_width(22.0 * z - 10.0);
                ui.label(RichText::new(key).monospace().size(11.0).color(TEXT));
            });
    }

    /// A small question-mark glyph carrying longer help as its tooltip.
    pub fn help(ui: &mut Ui, text: &str) -> Response {
        let z = zoom_of(ui);
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(16.0 * z), Sense::hover());
        if ui.is_rect_visible(rect) {
            let color = if resp.hovered() { ACCENT_SOFT } else { TEXT_DIM };
            let ir = Rect::from_center_size(rect.center(), Vec2::splat(14.0 * z));
            icons::draw(ui.painter(), ir, Icon::Help, color);
        }
        resp.on_hover_text(text)
    }

    /// A keycap → effect table for gesture legends.
    pub fn help_table(ui: &mut Ui, salt: impl Hash, rows: &[(&str, &str)]) {
        egui::Grid::new(salt).num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
            for (gesture, effect) in rows {
                keycap(ui, gesture);
                ui.add(egui::Label::new(RichText::new(*effect).size(11.0).color(TEXT_DIM)).wrap());
                ui.end_row();
            }
        });
    }

    /// One row of a [`list_row`] list.
    pub struct Row<'a> {
        pub swatch: Option<Color32>,
        pub icon: Option<Icon>,
        pub title: &'a str,
        pub subtitle: Option<&'a str>,
        pub badges: &'a [(&'a str, Color32)],
        pub selected: bool,
        pub active: bool,
        pub indent: f32,
    }

    /// A full-width selectable row: swatch or icon, title, optional
    /// subtitle, right-aligned badges (only with room), a play mark when
    /// `active`. Returns the click response, so drag-and-drop and context
    /// menus hang off it as they do off a `selectable_label`.
    pub fn list_row(ui: &mut Ui, salt: impl Hash, row: &Row) -> Response {
        ui.push_id(salt, |ui| {
            let z = zoom_of(ui);
            let h = ui.spacing().interact_size.y
                + 4.0
                + if row.subtitle.is_some() { 12.0 * z } else { 0.0 };
            // Click *and drag*: rows are drag sources (the Presets pool
            // files, unfiles and reorders by dragging them), and egui only
            // ever reports `drag_started` for a sense that includes drag.
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click_and_drag());
            if ui.is_rect_visible(rect) {
                let p = ui.painter();
                let rounding = Rounding::same(5.0);
                if row.selected {
                    p.rect_filled(rect.translate(Vec2::new(0.0, 1.0)), rounding, Color32::from_black_alpha(110));
                    p.add(Shape::mesh(vgradient_mesh(
                        rect,
                        rounding,
                        ACCENT.lerp_to_gamma(RAISED, 0.45),
                        ACCENT.lerp_to_gamma(SURFACE, 0.72),
                    )));
                    p.rect_stroke(rect, rounding, Stroke::new(1.0, ACCENT_MUTED));
                } else if resp.hovered() {
                    p.rect_filled(rect.translate(Vec2::new(0.0, 1.0)), rounding, Color32::from_black_alpha(90));
                    p.add(Shape::mesh(vgradient_mesh(
                        rect,
                        rounding,
                        HOVER.lerp_to_gamma(Color32::WHITE, 0.06),
                        HOVER.lerp_to_gamma(Color32::BLACK, 0.10),
                    )));
                    p.rect_stroke(rect, rounding, Stroke::new(1.0, RIM));
                }
                let mark = 14.0 * z;
                let mut x = rect.left() + 6.0 * z + row.indent;
                if let Some(c) = row.swatch {
                    let sr = Rect::from_center_size(Pos2::new(x + mark * 0.5, rect.center().y), Vec2::splat(mark));
                    paint_swatch(p, sr, c, 3.0 * z);
                    x += mark + 6.0 * z;
                } else if let Some(i) = row.icon {
                    let ir = Rect::from_center_size(Pos2::new(x + mark * 0.5, rect.center().y), Vec2::splat(mark));
                    icons::draw(p, ir, i, if row.selected { ACCENT_SOFT } else { TEXT_DIM });
                    x += mark + 6.0 * z;
                }
                let mut right = rect.right() - 6.0 * z;
                if row.active {
                    let s = 9.0 * z;
                    let pr = Rect::from_center_size(Pos2::new(right - s * 0.5, rect.center().y), Vec2::splat(s));
                    icons::draw(p, pr, Icon::Play, ACCENT_SOFT);
                    right -= s + 6.0 * z;
                }
                let title_font = FontId::new(13.0 * z, medium());
                let natural = p.layout_no_wrap(row.title.to_owned(), title_font.clone(), TEXT);
                let spare = right - x - natural.size().x;
                if spare >= 150.0 {
                    for (text, color) in row.badges.iter().rev() {
                        let g = p.layout_no_wrap(text.to_string(), FontId::new(10.0 * z, medium()), *color);
                        let bw = g.size().x + 8.0 * z;
                        let bh = g.size().y + 2.0 * z;
                        let br = Rect::from_min_max(
                            Pos2::new(right - bw, rect.center().y - bh * 0.5),
                            Pos2::new(right, rect.center().y + bh * 0.5),
                        );
                        p.rect_filled(br, Rounding::same(4.0 * z), color.gamma_multiply(0.18));
                        p.galley(Pos2::new(br.left() + 4.0 * z, br.center().y - g.size().y * 0.5), g, *color);
                        right -= bw + 4.0 * z;
                    }
                }
                let text_w = (right - x - 2.0).max(8.0);
                let title = ellipsis(p, row.title, title_font, TEXT, text_w);
                match row.subtitle {
                    Some(sub) => {
                        let ty = rect.top() + 3.0 * z;
                        let th = title.size().y;
                        p.galley(Pos2::new(x, ty), title, TEXT);
                        let sg = ellipsis(p, sub, FontId::new(11.0 * z, egui::FontFamily::Proportional), TEXT_DIM, text_w);
                        p.galley(Pos2::new(x, ty + th + 1.0 * z), sg, TEXT_DIM);
                    }
                    None => {
                        p.galley(Pos2::new(x, rect.center().y - title.size().y * 0.5), title, TEXT);
                    }
                }
            }
            resp
        })
        .inner
    }

    /// A section heading with a chevron that collapses its body. Returns
    /// true when the header was clicked this frame (`open` is toggled).
    pub fn fold(
        ui: &mut Ui,
        salt: impl Hash,
        title: &str,
        badge: Option<(&str, Color32)>,
        open: &mut bool,
        body: impl FnOnce(&mut Ui),
    ) -> bool {
        let mut toggled = false;
        ui.push_id(salt, |ui| {
            let z = zoom_of(ui);
            let h = ui.spacing().interact_size.y;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
            let resp = resp.on_hover_text("Show or hide this section.");
            if ui.is_rect_visible(rect) {
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(rect, Rounding::same(4.0), HOVER);
                }
                let chevron = if *open { Icon::ChevronDown } else { Icon::ChevronRight };
                let cs = 12.0 * z;
                let cr = Rect::from_center_size(Pos2::new(rect.left() + 4.0 * z + cs * 0.5, rect.center().y), Vec2::splat(cs));
                icons::draw(p, cr, chevron, TEXT_DIM);
                let tx = cr.right() + 6.0 * z;
                let mut right = rect.right() - 4.0 * z;
                if let Some((text, color)) = badge {
                    let g = p.layout_no_wrap(text.to_owned(), FontId::new(10.0 * z, medium()), color);
                    let bw = g.size().x + 10.0 * z;
                    let bh = g.size().y + 4.0 * z;
                    let br = Rect::from_min_max(
                        Pos2::new(right - bw, rect.center().y - bh * 0.5),
                        Pos2::new(right, rect.center().y + bh * 0.5),
                    );
                    p.rect(br, Rounding::same(5.0 * z), color.gamma_multiply(0.18), Stroke::new(1.0, color.gamma_multiply(0.5)));
                    p.galley(Pos2::new(br.left() + 5.0 * z, br.center().y - g.size().y * 0.5), g, color);
                    right -= bw + 6.0 * z;
                }
                let g = ellipsis(p, title, FontId::new(13.0 * z, semibold()), TEXT, (right - tx).max(8.0));
                p.galley(Pos2::new(tx, rect.center().y - g.size().y * 0.5), g, TEXT);
                p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, EDGE));
            }
            if resp.clicked() {
                *open = !*open;
                toggled = true;
            }
            if *open {
                ui.add_space(4.0);
                body(ui);
                ui.add_space(4.0);
            }
        });
        toggled
    }

    /// How a pad is drawn.
    #[derive(Clone, Copy, Default)]
    pub struct PadState {
        /// The pad's own colour; `None` = a plain raised pad.
        pub fill: Option<Color32>,
        /// Running / applied: white ring and a play mark.
        pub active: bool,
        /// Being edited in arrange mode: amber ring.
        pub editing: bool,
        pub hovered: bool,
        /// A drag is hovering it: accent ring.
        pub drop_target: bool,
        /// Idle: the colour is muted toward the panel.
        pub dim: bool,
    }

    /// What a pad shows large in its middle.
    pub enum PadFace<'a> {
        Symbol(&'a str),
        Icon(Icon),
    }

    /// The Phaser-board pad recipe: a lit tile in its colour, the face
    /// large, the name small beneath (when the pad is tall enough) and a
    /// ring while it runs.
    pub fn paint_pad(p: &Painter, rect: Rect, st: PadState, face: PadFace, name: &str) {
        let rounding = Rounding::same(R_PAD);
        let coloured = st.fill.is_some();
        let fill = match st.fill {
            None => RAISED,
            Some(c) => {
                if st.dim {
                    c.lerp_to_gamma(SURFACE, 0.6)
                } else {
                    c
                }
            }
        };
        let fill = if st.hovered { lighten(fill, 0.08) } else { fill };
        p.rect_filled(rect.translate(Vec2::new(0.0, 1.0)), rounding, Color32::from_black_alpha(120));
        p.add(Shape::mesh(vgradient_mesh(rect, rounding, lighten(fill, 0.10), darken(fill, 0.14))));
        let stroke = if st.drop_target {
            Stroke::new(2.0, ACCENT_SOFT)
        } else if st.editing {
            Stroke::new(2.0, WARN)
        } else if st.active {
            Stroke::new(2.0, Color32::WHITE)
        } else if coloured {
            Stroke::new(1.0, RIM)
        } else {
            Stroke::new(1.0, EDGE)
        };
        p.rect_stroke(rect, rounding, stroke);
        let ink = if coloured { readable_on(fill) } else { TEXT_DIM };
        let h = rect.height();
        let has_name = !name.is_empty() && h >= 44.0;
        let cy = if has_name { rect.center().y - h * 0.12 } else { rect.center().y };
        match face {
            PadFace::Symbol(symbol) => {
                let big = (h * 0.36).clamp(11.0, 22.0);
                p.text(
                    Pos2::new(rect.center().x, cy),
                    Align2::CENTER_CENTER,
                    if symbol.is_empty() && coloured { "•" } else { symbol },
                    FontId::new(big, semibold()),
                    ink,
                );
            }
            PadFace::Icon(icon) => {
                let ir = Rect::from_center_size(Pos2::new(rect.center().x, cy), Vec2::splat(h * 0.5));
                icons::draw(p, ir, icon, ink);
            }
        }
        if has_name {
            let galley = p.layout(
                name.to_string(),
                FontId::new(10.0, egui::FontFamily::Proportional),
                ink.gamma_multiply(0.85),
                rect.width() - 8.0,
            );
            // One line only: the pad is a button, not a label.
            let line = galley.rows.first().map(|r| r.rect.height()).unwrap_or(10.0);
            let clip = Rect::from_min_max(
                Pos2::new(rect.left() + 4.0, rect.bottom() - line - 5.0),
                Pos2::new(rect.right() - 4.0, rect.bottom() - 3.0),
            );
            p.with_clip_rect(clip).galley(
                Pos2::new(rect.center().x - galley.size().x.min(rect.width() - 8.0) * 0.5, clip.top()),
                galley,
                ink,
            );
        }
        if st.active {
            // A small play mark in the corner, so a running pad reads at a glance.
            let r = Rect::from_min_size(rect.left_top() + Vec2::new(5.0, 5.0), Vec2::new(9.0, 9.0));
            icons::draw(p, r, Icon::Play, ink);
        }
    }

    /// A pad as a widget. Callers add tooltips, context menus and drag
    /// sources on the response as the Phaser board does.
    pub fn pad_button(ui: &mut Ui, salt: impl Hash, size: Vec2, st: PadState, face: PadFace, name: &str) -> Response {
        // Allocated straight on `ui` rather than inside a `push_id` scope: a
        // child `Ui` is placed with `allocate_rect`, which no wrapping row can
        // break before (see `wrapped_child`), and a grid of pads laid out that
        // way runs off the panel. The salt keeps the id stable instead.
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let resp = ui.interact(rect, ui.id().with(("pad", salt)), Sense::click());
        if ui.is_rect_visible(rect) {
            let st = PadState { hovered: st.hovered || resp.hovered(), ..st };
            paint_pad(ui.painter(), rect, st, face, name);
        }
        resp
    }

    /// How many pads of at least `min_w` fit across `avail` with `gap`
    /// between them — never fewer than two.
    pub fn pad_columns(avail: f32, min_w: f32, gap: f32) -> usize {
        (((avail + gap) / (min_w + gap)).floor() as usize).max(2)
    }

    /// A status pill that wraps, and scales with the panel's zoom.
    ///
    /// [`pill`] is an `egui::Frame`, and a frame allocates its rect outright,
    /// so a row of them never breaks onto a second line — it just pushes a
    /// content-sized panel wider. Any row that can carry several at once
    /// wants this one; a lone pill in a vertical flow can stay a [`pill`].
    pub fn tag(ui: &mut Ui, text: &str, color: Color32) -> Response {
        let z = zoom_of(ui);
        let galley =
            ui.painter()
                .layout_no_wrap(text.to_owned(), FontId::new(11.0 * z, medium()), color);
        let pad = Vec2::new(6.0 * z, 2.0 * z);
        let (rect, resp) = ui.allocate_exact_size(galley.size() + pad * 2.0, Sense::hover());
        if ui.is_rect_visible(rect) {
            let p = ui.painter();
            p.rect(
                rect,
                Rounding::same(5.0 * z),
                color.gamma_multiply(0.18),
                Stroke::new(1.0, color.gamma_multiply(0.5)),
            );
            p.galley(rect.min + pad, galley, color);
        }
        resp
    }

    /// One segment of a [`segmented`] control.
    pub struct Segment<'a> {
        pub icon: Option<Icon>,
        pub label: &'a str,
        pub hint: &'a str,
        pub badge: Option<(String, Color32)>,
    }

    /// Whether labelled segments of these widths fit side by side.
    pub fn segments_fit(avail: f32, widths: &[f32]) -> bool {
        widths.iter().sum::<f32>() <= avail
    }

    /// A lit segmented control on a recessed track. Labels show when the
    /// labelled row fits, else the segments are icon-only squares with the
    /// active one's label painted under its icon and the names in the
    /// tooltips. Returns `Some(i)` when a segment other than `selected` is
    /// clicked; the mouse wheel over the track cycles.
    pub fn segmented(ui: &mut Ui, salt: impl Hash, items: &[Segment], selected: usize) -> Option<usize> {
        let n = items.len();
        if n == 0 {
            return None;
        }
        let z = zoom_of(ui);
        let pad = ui.spacing().button_padding;
        let icon_size = ui.text_style_height(&TextStyle::Button) * 1.1;
        let gap = 6.0 * z;
        let inner_pad = 2.0;
        let avail = ui.available_width();
        let inner_w = avail - inner_pad * 2.0;
        let galleys: Vec<Arc<Galley>> = items
            .iter()
            .map(|it| {
                WidgetText::from(it.label).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Button)
            })
            .collect();
        let widths: Vec<f32> = items
            .iter()
            .zip(&galleys)
            .map(|(it, g)| pad.x * 2.0 + it.icon.map_or(0.0, |_| icon_size + gap) + g.size().x)
            .collect();
        let fits = segments_fit(inner_w, &widths);
        let forced = items.iter().any(|it| it.icon.is_none());
        let labelled = forced || fits;
        let row_h = ui.spacing().interact_size.y + 8.0;
        // Icon-only: a band under the icons for the active label, leaving
        // room for the glow underline beneath it.
        let height = row_h + if labelled { 0.0 } else { 12.0 * z + 8.0 };
        let (track, _) = ui.allocate_exact_size(Vec2::new(avail, height), Sense::hover());
        let visible = ui.is_rect_visible(track);
        if visible {
            let p = ui.painter();
            let tr = Rounding::same(7.0);
            p.rect_filled(track.translate(Vec2::new(0.0, 1.0)), tr, Color32::from_black_alpha(110));
            p.rect(track, tr, WELL, Stroke::new(1.0, EDGE));
        }
        let inner = track.shrink(inner_pad);
        let equal_w = inner.width() / n as f32;
        let extra = if labelled && fits { (inner.width() - widths.iter().sum::<f32>()) / n as f32 } else { 0.0 };
        let mut out: Option<usize> = None;
        let mut x = inner.left();
        for (i, it) in items.iter().enumerate() {
            let w = if labelled && fits { widths[i] + extra } else { equal_w };
            let seg_rect = Rect::from_min_size(Pos2::new(x, inner.top()), Vec2::new(w, inner.height()));
            x += w;
            let resp = ui.interact(seg_rect, ui.id().with((&salt, i)), Sense::click());
            let is_sel = i == selected;
            if visible {
                let body = seg_rect.shrink(2.0);
                let rounding = Rounding::same(5.0);
                let p = ui.painter();
                if is_sel {
                    p.rect_filled(body.translate(Vec2::new(0.0, 1.0)), rounding, Color32::from_black_alpha(110));
                    p.add(Shape::mesh(vgradient_mesh(
                        body,
                        rounding,
                        ACCENT.lerp_to_gamma(RAISED, 0.45),
                        ACCENT.lerp_to_gamma(SURFACE, 0.72),
                    )));
                    p.rect_stroke(body, rounding, Stroke::new(1.0, ACCENT_MUTED));
                } else if resp.hovered() {
                    p.rect_filled(body.translate(Vec2::new(0.0, 1.0)), rounding, Color32::from_black_alpha(90));
                    p.add(Shape::mesh(vgradient_mesh(
                        body,
                        rounding,
                        HOVER.lerp_to_gamma(Color32::WHITE, 0.06),
                        HOVER.lerp_to_gamma(Color32::BLACK, 0.10),
                    )));
                    p.rect_stroke(body, rounding, Stroke::new(1.0, RIM));
                }
                let (icon_color, text_color) = if is_sel {
                    (ACCENT_SOFT, TEXT)
                } else if resp.hovered() {
                    (TEXT, TEXT)
                } else {
                    (TEXT_DIM.lerp_to_gamma(TEXT, 0.35), TEXT_DIM.lerp_to_gamma(TEXT, 0.55))
                };
                if labelled {
                    let text_w = if fits { f32::INFINITY } else { (body.width() - pad.x * 2.0 - it.icon.map_or(0.0, |_| icon_size + gap)).max(8.0) };
                    let g = if fits {
                        galleys[i].clone()
                    } else {
                        WidgetText::from(it.label).into_galley(ui, Some(TextWrapMode::Truncate), text_w, TextStyle::Button)
                    };
                    let content_w = it.icon.map_or(0.0, |_| icon_size + gap) + g.size().x;
                    let mut cx = body.center().x - content_w * 0.5;
                    if let Some(icon) = it.icon {
                        let ir = Rect::from_center_size(Pos2::new(cx + icon_size * 0.5, body.center().y), Vec2::splat(icon_size));
                        icons::draw(p, ir, icon, icon_color);
                        cx += icon_size + gap;
                    }
                    p.galley(Pos2::new(cx, body.center().y - g.size().y * 0.5), g, text_color);
                } else {
                    // Icon-only: the icon in the top band, the active label under it.
                    let isz = icon_size.min(body.width() - 4.0).min(row_h - 12.0).max(6.0);
                    let ir = Rect::from_center_size(
                        Pos2::new(body.center().x, track.top() + (row_h - 4.0) * 0.5),
                        Vec2::splat(isz),
                    );
                    if let Some(icon) = it.icon {
                        icons::draw(p, ir, icon, icon_color);
                    }
                    if is_sel {
                        // The label may run past its own segment (the
                        // neighbours' bands are empty) but stays on the track.
                        let g = ellipsis(p, it.label, FontId::new(10.0 * z, medium()), ACCENT_SOFT, track.width() - 8.0);
                        let x = (body.center().x - g.size().x * 0.5)
                            .clamp(track.left() + 4.0, (track.right() - 4.0 - g.size().x).max(track.left() + 4.0));
                        p.galley(Pos2::new(x, track.top() + row_h - 4.0), g, ACCENT_SOFT);
                    }
                }
                if let Some((text, color)) = &it.badge {
                    let g = p.layout_no_wrap(text.clone(), FontId::new(9.0 * z, medium()), *color);
                    let bh = 12.0 * z;
                    let bw = (g.size().x + 6.0 * z).max(12.0 * z);
                    let anchor = seg_rect.right_top() + Vec2::new(-2.0, 2.0);
                    let br = Rect::from_min_max(Pos2::new(anchor.x - bw, anchor.y), Pos2::new(anchor.x, anchor.y + bh));
                    p.rect(br, Rounding::same(6.0 * z), color.gamma_multiply(0.22), Stroke::new(1.0, color.gamma_multiply(0.6)));
                    p.galley(Pos2::new(br.center().x - g.size().x * 0.5, br.center().y - g.size().y * 0.5), g, *color);
                }
                if is_sel {
                    // Underline with a faint glow above it.
                    let glow = Rect::from_min_max(
                        Pos2::new(body.left() + 4.0, body.bottom() - 5.0),
                        Pos2::new(body.right() - 4.0, body.bottom() - 1.0),
                    );
                    p.add(Shape::mesh(vgradient_mesh(
                        glow,
                        Rounding::ZERO,
                        Color32::TRANSPARENT,
                        ACCENT_SOFT.gamma_multiply(0.35),
                    )));
                    p.rect_filled(
                        Rect::from_min_max(
                            Pos2::new(body.left() + 4.0, body.bottom() - 2.0),
                            Pos2::new(body.right() - 4.0, body.bottom() - 0.5),
                        ),
                        1.0,
                        ACCENT_SOFT,
                    );
                }
            }
            let tip = if labelled { it.hint.to_owned() } else { format!("{} — {}", it.label, it.hint) };
            if resp.on_hover_text(tip).clicked() && !is_sel {
                out = Some(i);
            }
        }
        // The wheel belongs to the strip while the pointer is on it: cycle
        // the selection and swallow the scroll, so a strip sitting inside a
        // ScrollArea does not scroll the panel on the same notch. egui bleeds
        // one notch out over several frames (`unprocessed_scroll_delta` is
        // private), so the smooth delta is zeroed on every hovered frame and
        // not only on the one the notch lands in. Off the track nothing is
        // touched and the ScrollArea scrolls as usual.
        if ui.rect_contains_pointer(track) {
            let dy = ui.input_mut(|i| {
                let dy = i.raw_scroll_delta.y;
                i.raw_scroll_delta = Vec2::ZERO;
                i.smooth_scroll_delta = Vec2::ZERO;
                dy
            });
            if out.is_none() {
                if dy <= -8.0 {
                    out = Some((selected + 1) % n);
                } else if dy >= 8.0 {
                    out = Some((selected + n - 1) % n);
                }
            }
        }
        out
    }
}
pub use kit::*;

#[cfg(test)]
mod tests {
    use eframe::egui::Vec2;

    /// A row of kit controls has to break at the panel edge. It did not once:
    /// every scoped control is placed with `allocate_rect`, which no wrapping
    /// layout can break before, so the row ran off a 230 px Inspector and the
    /// content-sized side panel chased it out to 585 px.
    #[test]
    fn a_toolbar_of_kit_controls_stays_inside_its_panel() {
        use crate::ui::icons::Icon;
        const W: f32 = 214.0;
        let ctx = egui::Context::default();
        install(&ctx);
        let mut used = 0.0f32;
        // Two passes: the fonts `install` registers only take effect on the next frame.
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = Rect::from_min_size(ui.max_rect().min, Vec2::new(W, 600.0));
                    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    toolbar(&mut row, |ui| {
                        for i in 0..12 {
                            let gate = if i % 4 == 3 { Err("Select some lights first.") } else { Ok(()) };
                            tool_button(ui, Icon::Camera, None, "A tool.", gate);
                        }
                        toggle_icon(ui, Icon::Grid, None, true, "A toggle.");
                        toggle_icon(ui, Icon::Beam, None, false, "Another toggle.");
                        tool_button(ui, Icon::Reset, Some("Reset"), "A labelled tool.", Ok(()));
                    });
                    // Pads wrap on the same rules; a grid of them blew a panel
                    // from 210 px to 568 px before `pad_button` stopped scoping.
                    toolbar(&mut row, |ui| {
                        for i in 0..9 {
                            pad_button(
                                ui,
                                i,
                                Vec2::new(64.0, 44.0),
                                PadState::default(),
                                PadFace::Symbol("PAD"),
                                "pad",
                            );
                        }
                    });
                    used = row.min_rect().width();
                });
            });
        }
        assert!(used <= W + 0.5, "the row ran {used} px wide inside a {W} px panel");
    }

    /// `tool_size` copies `icon_button`'s arithmetic so a row can claim the
    /// space before the button exists. If the button's padding ever changes,
    /// this is what notices.
    #[test]
    fn tool_size_matches_the_button_it_measures() {
        use crate::ui::icons::{self, Icon};
        let ctx = egui::Context::default();
        install(&ctx);
        let mut pairs: Vec<(Vec2, Vec2)> = Vec::new();
        for _ in 0..2 {
            pairs.clear();
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for label in [None, Some("Frame all"), Some("W")] {
                        let want = tool_size(ui, label);
                        let got = icons::icon_button(ui, Icon::Camera, label).rect.size();
                        pairs.push((want, got));
                    }
                });
            });
        }
        for (want, got) in pairs {
            assert!(
                (want.x - got.x).abs() < 0.5 && (want.y - got.y).abs() < 0.5,
                "measured {want:?} but the button took {got:?}"
            );
        }
    }
    use super::*;
    use crate::net::{Frame, DMX_SLOTS};
    use crate::stage::headless::{render_frames, save};

    /// `list_row` is a drag source: the Presets pool files, unfiles and
    /// reorders presets by dragging rows onto folders, onto the heading and
    /// onto each other. egui only ever reports `drag_started` for a widget
    /// whose sense includes drag, so with `Sense::click()` the whole gesture
    /// was dead in List view. Every other caller only asks for clicks, and a
    /// `click_and_drag` response still reports those.
    #[test]
    fn a_list_row_senses_a_drag_and_still_reports_a_plain_click() {
        let ctx = egui::Context::default();
        install(&ctx);
        let press = |pos: Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut senses_drag = false;
        let mut clicked = false;
        let mut drag_started = false;
        // The row's own centre, learned on the warm-up frame.
        let mut at = Pos2::ZERO;
        // Warm-up, press, release in place, press again, then pull away.
        for frame in 0..5 {
            let events = match frame {
                1 => vec![egui::Event::PointerMoved(at), press(at, true)],
                2 => vec![press(at, false)],
                3 => vec![egui::Event::PointerMoved(at), press(at, true)],
                4 => vec![egui::Event::PointerMoved(at + Vec2::new(0.0, 70.0))],
                _ => Vec::new(),
            };
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let row = Row {
                        swatch: None,
                        icon: None,
                        title: "Verse blue",
                        subtitle: None,
                        badges: &[],
                        selected: false,
                        active: false,
                        indent: 0.0,
                    };
                    let resp = list_row(ui, "one", &row);
                    at = resp.rect.center();
                    senses_drag = resp.sense.drag;
                    clicked |= frame == 2 && resp.clicked();
                    drag_started |= frame == 4 && resp.drag_started();
                });
            });
        }
        assert!(senses_drag, "the row is allocated click-only, so it can never be dragged");
        assert!(clicked, "a plain click on a row stopped being reported");
        assert!(drag_started, "pulling a row away never starts a drag");
    }

    /// One wheel notch over a segmented strip cycles the strip and stops
    /// there. `segmented` used to read `raw_scroll_delta` without consuming
    /// it, and a `ScrollArea` only ever zeroes `smooth_scroll_delta`, so the
    /// same notch flipped the control *and* scrolled the panel it sits in —
    /// on the Stage tab that silently threw the camera into Fly mode. Off
    /// the strip the panel must still scroll as usual.
    #[test]
    fn a_wheel_over_a_segmented_strip_cycles_it_without_scrolling_the_panel() {
        let segs = [
            Segment { icon: None, label: "Orbit", hint: "orbit the stage", badge: None },
            Segment { icon: None, label: "Fly", hint: "fly the camera", badge: None },
        ];
        // (what the strip picked, how far the panel scrolled)
        let run = |over_strip: bool| -> (Option<usize>, f32) {
            let ctx = egui::Context::default();
            install(&ctx);
            let mut strip = (0.0f32, 0.0f32);
            let mut picked = None;
            let mut offset = 0.0;
            for frame in 0..6 {
                let y = if over_strip {
                    (strip.0 + strip.1) * 0.5
                } else {
                    strip.1 + 40.0
                };
                let at = Pos2::new(120.0, y);
                let mut events = vec![egui::Event::PointerMoved(at)];
                if frame == 2 {
                    events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        delta: Vec2::new(0.0, -1.0),
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 400.0))),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let out = egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                            let top = ui.cursor().min.y;
                            let got = segmented(ui, "seg", &segs, 0);
                            strip = (top, ui.cursor().min.y);
                            for i in 0..40 {
                                ui.label(format!("row {i}"));
                            }
                            got
                        });
                        picked = picked.or(out.inner);
                        offset = out.state.offset.y;
                    });
                });
            }
            (picked, offset)
        };
        let (picked, offset) = run(true);
        assert_eq!(picked, Some(1), "the wheel did not cycle the strip under the pointer");
        assert_eq!(offset, 0.0, "the panel scrolled on the same notch");
        let (picked, offset) = run(false);
        assert_eq!(picked, None, "a strip nowhere near the pointer cycled");
        assert!(offset > 0.0, "the panel stopped scrolling off the strip");
    }

    #[test]
    fn segments_fit_is_a_plain_sum() {
        assert!(segments_fit(200.0, &[60.0, 60.0, 60.0]));
        assert!(!segments_fit(200.0, &[70.0; 3]));
        assert!(segments_fit(180.0, &[60.0, 60.0, 60.0]));
        assert!(segments_fit(0.0, &[]));
    }

    #[test]
    fn readable_on_flips_at_150_luminance() {
        let dark = Color32::from_rgb(0x12, 0x14, 0x18);
        assert_eq!(readable_on(Color32::WHITE), dark);
        assert_eq!(readable_on(Color32::BLACK), TEXT);
        assert_eq!(readable_on(Color32::from_rgb(150, 150, 150)), TEXT);
        assert_eq!(readable_on(Color32::from_rgb(151, 151, 151)), dark);
        assert_eq!(readable_on(RAISED), TEXT);
    }

    #[test]
    fn pad_columns_never_below_two() {
        assert_eq!(pad_columns(210.0, 60.0, 5.0), 3);
        assert_eq!(pad_columns(120.0, 60.0, 5.0), 2);
        assert_eq!(pad_columns(20.0, 60.0, 5.0), 2);
        assert_eq!(pad_columns(600.0, 60.0, 5.0), 9);
    }

    /// The whole console — toolbar, both side panels, the stage and a few
    /// of the busier windows — rendered through egui's renderer with the
    /// theme installed, and written to `target/ui_theme_headless.png` so a
    /// restyle can be looked at without launching the app.
    #[test]
    fn console_renders_headless() {
        let mut app = crate::app::App::new();
        *app.net.dmx.lock() = Frame([170u8; DMX_SLOTS]);
        // A light in the channel controls, with its first three channels armed.
        if let Some(f) = app.patch.fixtures.first() {
            app.sel_fixture = Some(0);
            for k in 0..3 {
                app.sel_channels.insert(f.from as usize - 1 + k);
            }
        }
        let size = [1600, 1000];
        // Two layouts, so every kind of surface gets a look.
        let scenes: [(&str, &[&str]); 4] = [
            ("ui_theme_headless", &["phasers", "settings", "configs"]),
            ("ui_theme_headless_2", &["palettes", "chases", "beat", "audio", "artnet", "board"]),
            // Every docked panel folded: just the stage and the rails.
            ("ui_theme_headless_3", &["stage-only"]),
            // Nothing open: the channel controls for three selected lights.
            ("ui_theme_headless_4", &["quiet"]),
        ];
        for (name, windows) in scenes {
            for key in ["fixtures", "inspector", "channels"] {
                if windows.contains(&"stage-only") {
                    app.collapsed.insert(key);
                } else {
                    app.collapsed.remove(key);
                }
            }
            app.show_phasers = windows.contains(&"phasers");
            app.show_settings = windows.contains(&"settings");
            app.show_palettes = windows.contains(&"palettes");
            app.show_chases = windows.contains(&"chases");
            app.show_beat = windows.contains(&"beat");
            app.show_audio = windows.contains(&"audio");
            app.show_artnet = windows.contains(&"artnet");
            app.show_configs = windows.contains(&"configs");
            app.show_phaser_board = windows.contains(&"board");
            app.show_log = !windows.contains(&"quiet");
            if windows.contains(&"quiet") {
                for i in 0..3.min(app.patch.fixtures.len()) {
                    app.stage.select_fixture(i, true);
                }
            }
            let Some(pixels) = render_frames(5, size, |ctx, frame| {
                // Fonts registered mid-frame take effect the next frame,
                // while the style does at once — so frame 0 only installs.
                if frame == 0 {
                    install(ctx);
                } else {
                    app.draw_ui(ctx);
                }
            }) else {
                eprintln!("no GPU adapter — skipping");
                return;
            };
            save(&pixels, size, name);
            let lit = pixels
                .chunks(4)
                .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120)
                .count();
            assert!(lit > 1000, "{name} came out black");
        }
    }
}
