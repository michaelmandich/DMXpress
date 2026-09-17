#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod artnet;
mod backup;
mod chase;
mod config;
mod encoder;
mod engine;
mod fixturedb;
mod gobo;
mod group;
mod net;
mod order;
mod oscillator;
mod palette;
mod phaser;
mod plugin;
mod preset;
mod preset_deck;
mod profiles;
mod sacn;
mod audio;
mod scene;
mod showbuddy;
mod stack;
mod stage;
mod streamdeck;
mod transition;
mod undo;
mod ui;
mod view;
mod wheels;

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
    let _ = std::fs::create_dir_all(dir.join("fixtures"));
    let _ = std::fs::create_dir_all(dir.join("plugins"));
    for (rel, data) in [
        ("configs/Great divide.json", include_str!("../configs/Great divide.json")),
        ("configs/My Rig.json", include_str!("../configs/My Rig.json")),
        ("setups/2d original.json", include_str!("../setups/2d original.json")),
        ("setups/Full setup.json", include_str!("../setups/Full setup.json")),
        ("setups/back towers done.json", include_str!("../setups/back towers done.json")),
        ("plugins/neon_night.rhai", include_str!("../plugins/neon_night.rhai")),
        ("plugins/breathing_floor.rhai", include_str!("../plugins/breathing_floor.rhai")),
        ("plugins/strobe_burst.rhai", include_str!("../plugins/strobe_burst.rhai")),
    ] {
        let path = dir.join(rel);
        if !path.exists() {
            let _ = std::fs::write(path, data);
        }
    }
    // Without this a downloaded build has no fixture library at all. It seeds
    // as bytes rather than text because it ships gzipped — an upgrade over a
    // data dir holding an older uncompressed `library.json` lands the new one
    // alongside it, and `fixturedb::read_library` prefers this one.
    let library = dir.join(fixturedb::LIBRARY_FILE);
    if !library.exists() {
        let _ = std::fs::write(library, include_bytes!("../fixtures/library.json.gz"));
    }
    // The gobo catalogue likewise — but unlike the library above, which is
    // left alone once present because a user may have swapped in their own,
    // the pack carries no user data (assignments live in `gobos_user.json`),
    // so a newer build's pack simply replaces an older one — told apart by
    // size, which is all a data dir can cheaply check.
    let pack: &[u8] = include_bytes!("../fixtures/gobos.tar.gz");
    let gobos = dir.join(gobo::PACK_FILE);
    let stale = std::fs::metadata(&gobos).map_or(true, |m| m.len() != pack.len() as u64);
    if stale {
        let _ = std::fs::write(gobos, pack);
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
