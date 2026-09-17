//! All egui panel/window rendering for the app, split by area:
//! - `top`     — top toolbar, Art-Net panel, floating Log window
//! - `side`      — Fixtures (left) side panel and the dockable-panel shell
//! - `inspector` — the right-hand Inspector: tab strip, context strip, one file per tab
//! - `central` — 3D stage view and the channel-type control grid
//! - `windows` — Settings, reset confirmation, floating Oscillator window

mod central;
mod channels;
pub(crate) mod chanrows;
mod levelbar;
mod audio;
mod beat;
pub(crate) mod board;
mod chases;
mod command;
mod decks;
mod dmxtest;
mod gobos;
mod groups;
mod icons;
pub(crate) mod inspector;
pub(crate) mod network;
mod orders;
mod palettes;
mod patchcfg;
mod phasers;
mod plugins;
mod preset_board;
mod raid;
mod safety;
mod scenes;
mod side;
mod stacks;
mod theme;
mod top;
mod views;
mod windows;

use eframe::egui;

pub(crate) use theme::install as install_theme;
pub(crate) use theme::install_with;
pub(crate) use theme::relief_pass;
pub(crate) use icons::window_icon;
pub(crate) use theme::BACKDROP;

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

/// Tiny `A− / A+` zoom stepper drawn in a panel header. Reads left to
/// right whichever way the header row lays out.
pub(crate) fn zoom_controls(ui: &mut egui::Ui, z: &mut f32) {
    const SHRINK: (&str, &str, f32) = ("A−", "Shrink this panel", -0.1);
    const GROW: (&str, &str, f32) = ("A+", "Enlarge this panel", 0.1);
    let rtl = ui.layout().main_dir() == egui::Direction::RightToLeft;
    let order = if rtl { [GROW, SHRINK] } else { [SHRINK, GROW] };
    // Tight, so the pair fits a narrow panel's header beside its title.
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        for (label, hint, step) in order {
            if ui.small_button(label).on_hover_text(hint).clicked() {
                *z = (*z + step).clamp(0.5, 2.0);
            }
        }
    });
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
        Role::Amber => C::from_rgb(255, 170, 60),
        Role::Uv => C::from_rgb(150, 90, 240),
        Role::Cyan => C::from_rgb(90, 220, 225),
        Role::Magenta => C::from_rgb(240, 100, 200),
        Role::Yellow => C::from_rgb(240, 225, 100),
        Role::Color => C::from_rgb(240, 120, 230),
        Role::Strobe => C::from_rgb(250, 250, 150),
        Role::Shutter => C::from_rgb(215, 215, 175),
        Role::Pan | Role::PanFine => C::from_rgb(120, 220, 240),
        Role::Tilt | Role::TiltFine => C::from_rgb(120, 190, 240),
        Role::Zoom => C::from_rgb(180, 160, 255),
        Role::Focus => C::from_rgb(160, 150, 235),
        Role::Iris => C::from_rgb(200, 175, 245),
        Role::Gobo => C::from_rgb(190, 215, 150),
        Role::Prism => C::from_rgb(150, 215, 195),
        Role::Frost => C::from_rgb(205, 225, 230),
        Role::Speed => C::from_rgb(170, 170, 170),
        Role::Other => C::from_gray(120),
    }
}

/// What happened to a popped-out panel this frame.
#[derive(Default)]
pub(crate) struct PoppedOutcome {
    /// The person asked for it back in the main window.
    pub dock: bool,
    /// The OS window was closed.
    pub closed: bool,
}

/// Show a panel as its own native OS window via egui's multi-viewport
/// support, with the standard header strip (title, Dock, zoom stepper).
///
/// It lives in the same process/app, so it does not get its own Dock icon
/// (macOS) or taskbar entry (Windows/Linux) — it's just another window of
/// DMXpress — but it can be dragged anywhere on screen, including off the
/// main window and onto another monitor.
///
/// `key` must be a short, stable, unique identifier for the panel (it is
/// the viewport's identity across frames).
pub(crate) fn popped_viewport(
    ctx: &egui::Context,
    key: &str,
    title: &str,
    mut zoom: Option<&mut f32>,
    default_size: [f32; 2],
    add_contents: impl FnOnce(&mut egui::Ui),
) -> PoppedOutcome {
    let mut add_contents = Some(add_contents);
    let mut outcome = PoppedOutcome::default();
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of(key),
        egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(default_size)
            .with_min_inner_size([220.0, 160.0]),
        |ctx, class| {
            if class == egui::ViewportClass::Embedded {
                // This backend doesn't support real multi-viewport windows
                // (e.g. some Wayland setups) — fall back to docking rather
                // than silently losing the panel.
                outcome.dock = true;
            }
            let frame = theme::panel_frame(&ctx.style());
            egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
                theme::panel_header(ui, title, |ui| {
                    if icons::icon_button(ui, icons::Icon::Dock, Some("Dock"))
                        .on_hover_text("Return to the main window")
                        .clicked()
                    {
                        outcome.dock = true;
                    }
                    if let Some(z) = zoom.as_deref_mut() {
                        ui.add_space(6.0);
                        zoom_controls(ui, z);
                    }
                });
                if let Some(z) = zoom.as_deref() {
                    apply_zoom(ui, *z);
                }
                if let Some(f) = add_contents.take() {
                    f(ui);
                }
            });
            if ctx.input(|i| i.viewport().close_requested()) {
                outcome.closed = true;
            }
        },
    );
    outcome
}

/// Show a panel either docked in the main window as a floating
/// [`egui::Window`] (the default), or — once popped out — as its own
/// native OS window (see [`popped_viewport`]).
///
/// `key` must be a short, stable, unique identifier for the panel (used to
/// derive both the egui `Id` and the viewport's identity across frames).
/// `zoom`, when given, puts the panel's `A− / A+` stepper in the header row
/// and scales the contents by it.
pub(crate) fn floating_panel(
    ctx: &egui::Context,
    key: &str,
    title: &str,
    open: &mut bool,
    popped: &mut bool,
    mut zoom: Option<&mut f32>,
    default_size: [f32; 2],
    default_pos: [f32; 2],
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    if !*open {
        return;
    }
    if *popped {
        let outcome = popped_viewport(ctx, key, title, zoom, default_size, add_contents);
        if outcome.dock {
            *popped = false;
        }
        if outcome.closed {
            *open = false;
            *popped = false;
        }
    } else {
        egui::Window::new(title)
            .id(egui::Id::new(key))
            .open(open)
            .collapsible(true)
            .resizable(true)
            .default_size(default_size)
            .default_pos(default_pos)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if icons::icon_button(ui, icons::Icon::PopOut, Some("Pop out"))
                        .on_hover_text(
                            "Open this panel in its own window, draggable outside the main window",
                        )
                        .clicked()
                    {
                        *popped = true;
                    }
                    if let Some(z) = zoom.as_deref_mut() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            zoom_controls(ui, z);
                        });
                    }
                });
                ui.separator();
                if let Some(z) = zoom.as_deref() {
                    apply_zoom(ui, *z);
                }
                add_contents(ui);
            });
    }
}
