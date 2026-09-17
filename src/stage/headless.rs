//! Headless rendering of the real UI through egui's own wgpu renderer, on
//! whatever GPU the test machine has. This is the closest a test gets to
//! the app: the same `StageView::ui`, the same paint callbacks, the same
//! buffer uploads egui does per frame — so a GPU fault or a validation error
//! shows up here instead of as a crash on someone's desk.

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};

use crate::gobo::{assign_patch, Catalogue, UserGobos};
use crate::net::DMX_SLOTS;
use crate::showbuddy::Patch;

/// Poll a wgpu future to completion on this thread.
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    use std::task::{Context, Poll, Waker};
    let mut fut = std::pin::pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

/// One renderer at a time.
///
/// Every call here builds its own instance, adapter and device, and the test
/// harness runs tests on as many threads as the machine has cores: a dozen
/// devices coming up at once took the whole process down (`cargo test` exited
/// 0xffffffff while each test passed on its own). Rendering is a fraction of
/// the suite's time, so serialising it costs little and makes the run
/// deterministic. A panicking test poisons the lock, which we ignore — the
/// failure it already reported is the interesting one.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Run `ui` for `frames` frames at `size` pixels and return the last
/// frame's RGBA pixels, or `None` when this machine has no GPU adapter.
/// Any wgpu error panics with its message.
pub(crate) fn render_frames(
    frames: usize,
    size: [u32; 2],
    mut ui: impl FnMut(&egui::Context, usize),
) -> Option<Vec<u8>> {
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let instance = wgpu::Instance::default();
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    eprintln!("headless adapter: {:?}", adapter.get_info().name);
    let (device, queue) =
        block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None)).ok()?;
    device.on_uncaptured_error(Box::new(|e| panic!("wgpu error: {e}")));

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = egui_wgpu::Renderer::new(&device, format, None, 1, false);
    renderer
        .callback_resources
        .insert(super::volumetric::VolumetricResources::new(&device, format));
    let ctx = egui::Context::default();
    let [w, h] = size;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless target"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: size, pixels_per_point: 1.0 };
    let bytes_per_row = (w * 4).div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("headless readback"),
        size: (bytes_per_row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    for frame in 0..frames {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w as f32, h as f32),
            )),
            time: Some(frame as f64 / 30.0),
            ..Default::default()
        };
        let out = ctx.run(raw, |ctx| ui(ctx, frame));
        let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
        for (id, delta) in &out.textures_delta.set {
            renderer.update_texture(&device, &queue, *id, delta);
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        let commands = renderer.update_buffers(&device, &queue, &mut encoder, &primitives, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("headless pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            renderer.render(&mut pass, &primitives, &screen);
        }
        for id in &out.textures_delta.free {
            renderer.free_texture(id);
        }
        if frame + 1 == frames {
            encoder.copy_texture_to_buffer(
                wgpu::ImageCopyTexture {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::ImageCopyBuffer {
                    buffer: &readback,
                    layout: wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(bytes_per_row),
                        rows_per_image: Some(h),
                    },
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
        queue.submit(commands.into_iter().chain([encoder.finish()]));
        device.poll(wgpu::Maintain::Wait);
    }

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback"));
    device.poll(wgpu::Maintain::Wait);
    let padded = slice.get_mapped_range().to_vec();
    readback.unmap();
    // Strip the row padding.
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        let start = row * bytes_per_row as usize;
        pixels.extend_from_slice(&padded[start..start + (w * 4) as usize]);
    }
    Some(pixels)
}

pub(crate) fn save(pixels: &[u8], size: [u32; 2], name: &str) {
    let _ = std::fs::create_dir_all("target");
    if let Some(img) = image::RgbaImage::from_raw(size[0], size[1], pixels.to_vec()) {
        let _ = img.save(format!("target/{name}.png"));
    }
}

fn lit_pixels(pixels: &[u8]) -> usize {
    pixels.chunks(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120).count()
}

/// The rig on this machine, patched the way `App::new` patches it (minus
/// ShowBuddy's live import), with gobos assigned.
fn local_rig() -> Option<(Patch, crate::fixturedb::Library, Catalogue, UserGobos)> {
    let user = crate::profiles::load_user_patch();
    let mut patch = Patch {
        fixtures: if user.include_showbuddy { crate::showbuddy::load_cache() } else { Vec::new() },
        warnings: Vec::new(),
    };
    let library = crate::fixturedb::Library::load();
    crate::profiles::extend_patch(&mut patch, &user, &library);
    if patch.fixtures.is_empty() {
        return None;
    }
    let gobos = Catalogue::load();
    let user_gobos = UserGobos::load();
    assign_patch(&mut patch, &library, &gobos, &user_gobos);
    Some((patch, library, gobos, user_gobos))
}

/// What the join found for this rig — printed so a run with `--nocapture`
/// says which lights got pictures.
fn report(patch: &Patch) -> usize {
    let mut keys = std::collections::BTreeSet::new();
    for f in &patch.fixtures {
        let wheels: Vec<String> = f
            .channels
            .iter()
            .filter(|c| c.is_gobo_wheel())
            .map(|c| {
                let known = c.bands.iter().filter(|b| b.gobo.is_some()).count();
                for b in &c.bands {
                    if let Some(k) = &b.gobo {
                        keys.insert(k.clone());
                    }
                }
                format!("{}: {known}/{} slots", c.name, c.bands.len())
            })
            .collect();
        if !wheels.is_empty() {
            eprintln!("  {} [{}] — {}", f.display, f.file.display(), wheels.join("; "));
        }
    }
    eprintln!("{} distinct gobo keys across the rig", keys.len());
    keys.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::{Settings, StageView};

    /// The stage widget, on the real rig, with every channel driven so the
    /// beams (and any gobos the join found) are lit. Three frames, so the
    /// atlas upload on the first frame is followed by frames that use it.
    #[test]
    fn stage_renders_the_local_rig_headless() {
        let Some((patch, _library, gobos, _user)) = local_rig() else {
            eprintln!("no local rig — skipping");
            return;
        };
        report(&patch);
        let mut settings = Settings::load();
        let mut stage = StageView::new();
        stage.sync(&patch, &settings);
        let buf = [170u8; DMX_SLOTS];
        let size = [1400, 800];
        let Some(pixels) = render_frames(3, size, |ctx, _| {
            egui::CentralPanel::default().show(ctx, |ui| {
                stage.ui(ui, &patch, &buf, &gobos, 700.0, &mut settings, None, None, None);
            });
        }) else {
            eprintln!("no GPU adapter — skipping");
            return;
        };
        save(&pixels, size, "stage_headless");
        assert!(lit_pixels(&pixels) > 1000, "the stage came out black");
    }

    /// The whole app's per-frame UI with the Gobos window open, the picker
    /// showing, and every light selected — the paths a person clicks
    /// through — rendered through egui's renderer for several frames.
    #[test]
    fn gobos_window_renders_headless() {
        let mut app = crate::app::App::new();
        let has_wheels: Vec<usize> = app
            .patch
            .fixtures
            .iter()
            .enumerate()
            .filter(|(_, f)| f.channels.iter().any(|c| c.is_gobo_wheel()))
            .map(|(i, _)| i)
            .collect();
        eprintln!("{} fixtures with gobo wheels", has_wheels.len());
        // Light everything so the stage has beams to draw under the window.
        *app.net.dmx.lock() = crate::net::Frame([170u8; DMX_SLOTS]);
        app.show_gobos = true;
        let size = [1400, 900];
        let Some(pixels) = render_frames(6, size, |ctx, frame| {
            // The console's theme binds the font families the panels use;
            // it takes effect the frame after it is installed.
            if frame == 0 {
                crate::ui::install_theme(ctx);
                return;
            }
            if frame == 1 {
                for &fi in &has_wheels {
                    app.stage.select_fixture(fi, true);
                }
                app.gobo_assign_mode = true;
            }
            if frame == 3 {
                // Open the picker on the first slot of the first wheel.
                if let Some(&fi) = has_wheels.first() {
                    let f = &app.patch.fixtures[fi];
                    let ci = f.channels.iter().position(|c| c.is_gobo_wheel()).unwrap();
                    app.gobo_pick = Some(crate::gobo::GoboPick {
                        profile: crate::gobo::profile_of(f),
                        channel: ci,
                        band: 1,
                        title: "test slot".into(),
                    });
                }
                app.gobo_search = "star".into();
            }
            app.central_panel(ctx);
            app.gobos_window(ctx);
        }) else {
            eprintln!("no GPU adapter — skipping");
            return;
        };
        save(&pixels, size, "gobos_window_headless");
        assert!(lit_pixels(&pixels) > 1000, "the window came out black");
        // The picker's thumbnails were uploaded.
        assert!(!app.gobo_thumbs.is_empty() || app.gobos.gobos.is_empty(), "no thumbnails uploaded");
    }
}
