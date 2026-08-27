//! The DMXpress look: one charcoal dark theme applied to every panel, window
//! and popup, independent of the operating system's light/dark preference.
//!
//! Charcoal carries the whole surface; teal is the single interactive accent
//! (selection, focus, active state) and the warm colours are reserved for
//! meaning — amber warns, red is destructive, green confirms. Everything is
//! layered by lightness rather than by borders: backdrop, then panel, then
//! raised control, so depth reads without heavy outlines.

use eframe::egui::{
    self, style::Selection, style::WidgetVisuals, style::Widgets, Color32, Rounding, Shadow,
    Stroke, Visuals,
};

// ---- palette ----

/// Deepest layer: the app backdrop and the 3D stage surround.
pub const BACKDROP: Color32 = Color32::from_rgb(0x16, 0x18, 0x1B);
/// Panels, side bars and window bodies.
pub const SURFACE: Color32 = Color32::from_rgb(0x1E, 0x20, 0x24);
/// Raised controls sitting on a panel (buttons, combo boxes, tiles).
pub const RAISED: Color32 = Color32::from_rgb(0x2A, 0x2E, 0x33);
/// Hovered control.
pub const HOVER: Color32 = Color32::from_rgb(0x36, 0x3C, 0x42);
/// Slate used for dividers, inactive outlines and header underlines.
pub const EDGE: Color32 = Color32::from_rgb(0x2F, 0x3A, 0x3D);
/// Text-entry wells and scroll troughs: darker than the panel they sit in.
pub const WELL: Color32 = Color32::from_rgb(0x12, 0x14, 0x17);

/// Interactive accent: selection, focus rings, active toggles.
pub const ACCENT: Color32 = Color32::from_rgb(0x1F, 0x6F, 0x78);
/// Lighter accent for text on charcoal, links and hovered accents.
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0x6F, 0xB6, 0xB5);
/// Muted teal for secondary emphasis.
pub const ACCENT_MUTED: Color32 = Color32::from_rgb(0x4A, 0x8A, 0x8F);

/// Primary text.
pub const TEXT: Color32 = Color32::from_rgb(0xE6, 0xF4, 0xF3);
/// Secondary text: hints, units, inactive labels.
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x93, 0xA1, 0xA3);

/// Something is running / succeeded.
pub const OK: Color32 = Color32::from_rgb(0x8C, 0xC9, 0x7F);
/// Caution: unsaved, degraded, standing in for missing data.
pub const WARN: Color32 = Color32::from_rgb(0xF0, 0xC2, 0x30);
/// Destructive or failed.
pub const DANGER: Color32 = Color32::from_rgb(0xD4, 0x2A, 0x1F);

// ---- geometry ----

const R_WIDGET: f32 = 4.0;
const R_WINDOW: f32 = 8.0;

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
            // Resting interactive control.
            inactive: widget(RAISED, RAISED, Color32::TRANSPARENT, TEXT, 0.0),
            // Pointer over it: lift, and hint the accent on the outline.
            hovered: widget(HOVER, HOVER, Stroke::new(1.0, ACCENT_MUTED).color, TEXT, 1.0),
            // Held down / toggled on: the accent takes over.
            active: widget(ACCENT, ACCENT, ACCENT_SOFT, TEXT, 1.0),
            // Keyboard focus.
            open: widget(RAISED, RAISED, ACCENT_MUTED, TEXT, 0.0),
        },
        selection: Selection {
            bg_fill: ACCENT.gamma_multiply(0.75),
            stroke: Stroke::new(1.0, TEXT),
        },
        hyperlink_color: ACCENT_SOFT,
        // Striped rows and hovered list entries.
        faint_bg_color: Color32::from_rgb(0x24, 0x27, 0x2C),
        // Text edits, sliders' troughs, plot backgrounds.
        extreme_bg_color: WELL,
        code_bg_color: Color32::from_rgb(0x1A, 0x1D, 0x21),
        warn_fg_color: WARN,
        error_fg_color: DANGER,

        window_rounding: Rounding::same(R_WINDOW),
        window_fill: SURFACE,
        window_stroke: Stroke::new(1.0, EDGE),
        window_shadow: Shadow {
            offset: egui::vec2(0.0, 8.0),
            blur: 24.0,
            spread: 0.0,
            color: Color32::from_black_alpha(140),
        },
        menu_rounding: Rounding::same(R_WIDGET + 2.0),
        popup_shadow: Shadow {
            offset: egui::vec2(0.0, 4.0),
            blur: 16.0,
            spread: 0.0,
            color: Color32::from_black_alpha(120),
        },
        panel_fill: SURFACE,

        // Slider fill up to the handle reads as "how much", like a meter.
        slider_trailing_fill: true,
        striped: false,
        indent_has_left_vline: true,
        ..Visuals::dark()
    }
}

/// Apply the theme to a context: colours, spacing and text sizes.
pub fn install(ctx: &egui::Context) {
    // The console has one look on every platform. Without this the app
    // inherits the OS preference, which left Windows with a white backdrop
    // under charcoal panels.
    ctx.set_theme(egui::ThemePreference::Dark);

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals();

    let sp = &mut style.spacing;
    sp.item_spacing = egui::vec2(8.0, 6.0);
    sp.button_padding = egui::vec2(8.0, 4.0);
    sp.menu_margin = egui::Margin::same(6.0);
    sp.window_margin = egui::Margin::same(10.0);
    sp.indent = 18.0;
    sp.slider_width = 140.0;
    sp.scroll.bar_width = 10.0;
    sp.scroll.floating = false;

    if let Some(font) = style.text_styles.get_mut(&egui::TextStyle::Heading) {
        font.size = 17.0;
    }

    ctx.set_style(style.clone());
    // Pin it as *the* dark style too, so nothing re-derives egui's default
    // palette if the theme is re-evaluated.
    ctx.options_mut(|o| o.dark_style = std::sync::Arc::new(style));
}

/// Heading with a hairline rule under it — the standard section break inside
/// windows and side panels.
pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(title)
            .color(TEXT)
            .size(13.0)
            .strong(),
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
            ui.label(egui::RichText::new(text).color(color).size(11.0).strong());
        })
        .response
}
