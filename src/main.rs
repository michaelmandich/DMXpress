#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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

// When launched outside a show directory (e.g. a downloaded release binary),
// switch to a per-user data dir so show files aren't written to Downloads —
// or lost entirely inside a Gatekeeper-translocated .app on macOS.
fn resolve_data_dir() {
    if std::path::Path::new("settings.json").exists()
        || std::path::Path::new("stage_layout.json").exists()
    {
        return;
    }
    let Some(base) = dirs::data_dir() else { return };
    let dir = base.join("DMXpress");
    let _ = std::fs::create_dir_all(dir.join("configs"));
    let _ = std::fs::create_dir_all(dir.join("setups"));
    for (rel, data) in [
        ("configs/Great divide.json", include_str!("../configs/Great divide.json")),
        ("configs/My Rig.json", include_str!("../configs/My Rig.json")),
        ("setups/2d original.json", include_str!("../setups/2d original.json")),
        ("setups/Full setup.json", include_str!("../setups/Full setup.json")),
        ("setups/back towers done.json", include_str!("../setups/back towers done.json")),
    ] {
        let path = dir.join(rel);
        if !path.exists() {
            let _ = std::fs::write(path, data);
        }
    }
    let _ = std::env::set_current_dir(&dir);
}

fn main() -> eframe::Result<()> {
    resolve_data_dir();
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
