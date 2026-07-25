mod app;
mod artnet;
mod chase;
mod config;
mod engine;
mod group;
mod net;
mod order;
mod oscillator;
mod palette;
mod phaser;
mod preset;
mod profiles;
mod audio;
mod scene;
mod showbuddy;
mod stack;
mod stage;
mod transition;
mod ui;
mod view;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([960.0, 640.0])
        .with_title("DMXpress");
    if let Some(icon) = ui::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "DMXpress",
        native_options,
        Box::new(|cc| {
            ui::install_theme(&cc.egui_ctx);
            if let Some(render_state) = &cc.wgpu_render_state {
                stage::initialize_volumetric(render_state);
            }
            Ok(Box::new(app::App::new()))
        }),
    )
}
