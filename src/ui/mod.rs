//! All egui panel/window rendering for the app, split by area:
//! - `top`     — top toolbar, Art-Net panel, floating Log window
//! - `side`    — Fixtures (left) and Inspector (right) side panels
//! - `central` — 3D stage view and the channel-type control grid
//! - `windows` — Settings, reset confirmation, floating Oscillator window

mod central;
mod beat;
mod chases;
mod command;
mod decks;
mod dmxtest;
mod groups;
mod palettes;
mod patchcfg;
mod phasers;
mod side;
mod stacks;
mod top;
mod views;
mod windows;

use eframe::egui;

use crate::showbuddy::Role;

/// Scale the fonts and spacing of a `Ui` (and its children) by `z`, giving
/// each panel its own independent zoom level.
pub(crate) fn apply_zoom(ui: &mut egui::Ui, z: f32) {
    if (z - 1.0).abs() < 0.001 {
        return;
    }
    let style = ui.style_mut();
    for font in style.text_styles.values_mut() {
        font.size *= z;
    }
    let sp = &mut style.spacing;
    sp.item_spacing *= z;
    sp.button_padding *= z;
    sp.indent *= z;
    sp.interact_size *= z;
    sp.icon_width *= z;
    sp.icon_width_inner *= z;
    sp.icon_spacing *= z;
}

/// Tiny `A− / A+` zoom stepper drawn in a panel header.
pub(crate) fn zoom_controls(ui: &mut egui::Ui, z: &mut f32) {
    if ui.small_button("A−").on_hover_text("Shrink this panel").clicked() {
        *z = (*z - 0.1).max(0.5);
    }
    if ui.small_button("A+").on_hover_text("Enlarge this panel").clicked() {
        *z = (*z + 0.1).min(2.0);
    }
}

/// UI tint for channel role badges.
pub(crate) fn role_color(r: Role) -> egui::Color32 {
    use egui::Color32 as C;
    match r {
        Role::Dimmer => C::from_rgb(255, 210, 120),
        Role::Red => C::from_rgb(255, 90, 80),
        Role::Green => C::from_rgb(90, 230, 100),
        Role::Blue => C::from_rgb(110, 140, 255),
        Role::White => C::from_rgb(235, 235, 235),
        Role::Color => C::from_rgb(240, 120, 230),
        Role::Strobe => C::from_rgb(250, 250, 150),
        Role::Pan | Role::PanFine => C::from_rgb(120, 220, 240),
        Role::Tilt | Role::TiltFine => C::from_rgb(120, 190, 240),
        Role::Zoom => C::from_rgb(180, 160, 255),
        Role::Speed => C::from_rgb(170, 170, 170),
        Role::Other => C::from_gray(120),
    }
}
