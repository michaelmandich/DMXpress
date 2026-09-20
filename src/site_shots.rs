//! The website's screenshots, rendered by the app's own headless renderer.
//!
//! [`site_shots_render`] drives the real `App::draw_ui` through
//! [`crate::stage::headless::render_frames`] and writes the manifest's PNGs
//! into `target/site-shots/`.
//!
//! They stop there on purpose. The site serves WebP — the same pictures as
//! PNG are five times the bytes, and a 23 MB page is a page nobody waits for —
//! and nothing in this crate can encode WebP. `tools/site_shots.py` does that
//! conversion and is the only thing that writes into `dist-netlify/images/`.
//! Rendering into the site directly would also mean a plain `cargo test` put
//! 19 MB of PNG back beside the WebP every time it ran.
//!
//! It is a screenshot job, not an assertion suite. Every scene is built in
//! memory — palettes, groups, cue stacks, routes, board pads, truss — and
//! nothing is ever stored: no `store_*`, no `capture_*`, no `save_*`, and
//! `stage.layout_path` is pointed at a scratch file before the first frame,
//! so a build or a hang can never reach the operator's `stage_layout.json`.

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Instant;

use crate::app::App;
use crate::audio::{Analysis, AudioTrigger, TriggerSource, BINS};
use crate::net::{DiscoveredNode, Frame, NetStats, Proto, UniverseStat, DMX_SLOTS};
use crate::order::{Order, OrderStep};
use crate::oscillator::Look;
use crate::palette::Feature;
use crate::preset_deck::preset_pads_per_page;
use crate::scene::{MergeMode, Scene};
use crate::showbuddy::{Channel, Fixture, Role};
use crate::stack::{Cue, CueVal, Stack};
use crate::stage::headless::render_frames;
use crate::stage::{
    classify, v3, Archetype, ElementRef, HangFill, OrderBy, PlacePattern, Placement, QuickView,
    RaidLook, Recipe, RecipeKind, TrussKind, V3,
};
use crate::ui::inspector::patch::{ElementChoice, PatchSource};
use crate::ui::inspector::presets::{PadSize, PadTarget, PresetView};
use crate::ui::inspector::InspectorTab;
use crate::ui::network::{NodeEntry, SourceEntry, Tab as NetTab};

/// Where the renders land. `tools/site_shots.py` turns these into the WebP
/// the site actually serves; see the module docs.
const OUT_DIR: &str = "target/site-shots";

/// Frames per shot. Frame 0 installs the theme (fonts registered mid-frame
/// only bind on the next one), frame 1 re-installs the plugin tint, and the
/// rest paint a settled console.
const FRAMES: usize = 6;

/// The house size for a shot the manifest does not size itself.
const WIDE: [u32; 2] = [1600, 1000];

// ---------------------------------------------------------------- output --

fn write_png(pixels: &[u8], size: [u32; 2], name: &str) {
    std::fs::create_dir_all(OUT_DIR).expect("make the images directory");
    // The renderer clears to opaque black, so every alpha byte is 255:
    // dropping the channel costs the picture nothing and saves the site
    // about a quarter of the bytes it has to ship.
    let rgb: Vec<u8> = pixels.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let img = image::RgbImage::from_raw(size[0], size[1], rgb).expect("pixels fit the size");
    img.save(format!("{OUT_DIR}/{name}.png")).expect("write the png");
}

/// `[x, y, w, h]` out of a frame, in pixels.
fn crop(pixels: &[u8], size: [u32; 2], at: [u32; 4]) -> (Vec<u8>, [u32; 2]) {
    let [x, y, w, h] = at;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * size[0] + x) * 4) as usize;
        out.extend_from_slice(&pixels[start..start + (w * 4) as usize]);
    }
    (out, [w, h])
}

/// Lay `tiles` out in one row on the console's own backdrop.
fn strip(tiles: &[(Vec<u8>, [u32; 2])], gap: u32, margin: u32) -> (Vec<u8>, [u32; 2]) {
    let h = tiles.iter().map(|(_, s)| s[1]).max().unwrap_or(0);
    let w: u32 = tiles.iter().map(|(_, s)| s[0]).sum::<u32>() + gap * (tiles.len() as u32 - 1);
    let (ow, oh) = (w + margin * 2, h + margin * 2);
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for p in out.chunks_mut(4) {
        p.copy_from_slice(&[13, 14, 17, 255]);
    }
    let mut x = margin;
    for (px, [tw, th]) in tiles {
        for row in 0..*th {
            let src = (row * tw * 4) as usize;
            let dst = (((row + margin) * ow + x) * 4) as usize;
            out[dst..dst + (*tw * 4) as usize].copy_from_slice(&px[src..src + (*tw * 4) as usize]);
        }
        x += tw + gap;
    }
    (out, [ow, oh])
}

// ------------------------------------------------------------- the light --

/// Deep blue → magenta → amber across the stage: the wash the static
/// lights carry.
fn wash(t: f32) -> [u8; 3] {
    const STOPS: [[f32; 3]; 3] =
        [[30.0, 80.0, 255.0], [255.0, 40.0, 165.0], [255.0, 150.0, 40.0]];
    let t = t.clamp(0.0, 1.0) * 2.0;
    let i = (t.floor() as usize).min(1);
    let k = t - i as f32;
    let (a, b) = (STOPS[i], STOPS[i + 1]);
    [
        (a[0] + (b[0] - a[0]) * k) as u8,
        (a[1] + (b[1] - a[1]) * k) as u8,
        (a[2] + (b[2] - a[2]) * k) as u8,
    ]
}

/// The punchier colours the moving heads throw into the air.
fn beam_colour(n: usize) -> [u8; 3] {
    const BEAMS: [[u8; 3]; 4] =
        [[60, 225, 255], [255, 55, 190], [130, 255, 150], [255, 205, 110]];
    BEAMS[n % BEAMS.len()]
}

/// Middle of the first band whose label says the shutter is open; 255 when
/// the channel has no bands to read.
fn open_value(ch: &Channel) -> u8 {
    ch.bands
        .iter()
        .find(|b| {
            let l = b.label.to_lowercase();
            l.contains("open") || l.contains("no func")
        })
        .map(|b| (b.min / 2).saturating_add(b.max / 2))
        .unwrap_or(255)
}

/// Middle of a band carrying an actual gobo picture, for the gobo shot.
fn gobo_value(ch: &Channel) -> Option<u8> {
    ch.bands
        .iter()
        .find(|b| b.gobo.is_some() && !b.label.to_lowercase().contains("open"))
        .map(|b| (b.min / 2).saturating_add(b.max / 2))
}

/// Every colour-wheel slot the stage can tint a beam with, as DMX values.
fn colour_slots(ch: &Channel) -> Vec<u8> {
    const NAMES: [&str; 6] = ["blue", "cyan", "magenta", "pink", "green", "orange"];
    ch.bands
        .iter()
        .filter(|b| {
            let l = b.label.to_lowercase();
            NAMES.iter().any(|n| l.contains(n))
        })
        .map(|b| (b.min / 2).saturating_add(b.max / 2))
        .collect()
}

/// Paint one fixture into the buffer.
fn drive(
    buf: &mut [u8; DMX_SLOTS],
    f: &Fixture,
    rgb: [u8; 3],
    pan: u8,
    tilt: u8,
    zoom: u8,
    gobos: bool,
) {
    for (i, ch) in f.channels.iter().enumerate() {
        let addr = f.from as usize + i;
        if addr == 0 || addr > DMX_SLOTS {
            continue;
        }
        buf[addr - 1] = match ch.role() {
            Role::Dimmer => 255,
            Role::Red => rgb[0],
            Role::Green => rgb[1],
            Role::Blue => rgb[2],
            Role::Cyan => 255 - rgb[0],
            Role::Magenta => 255 - rgb[1],
            Role::Yellow => 255 - rgb[2],
            Role::Pan => pan,
            Role::Tilt => tilt,
            Role::Zoom => zoom,
            Role::Focus => 128,
            Role::Iris => 255,
            Role::Shutter | Role::Strobe => open_value(ch),
            Role::Gobo if gobos && ch.is_gobo_wheel() => gobo_value(ch).unwrap_or(0),
            _ => 0,
        };
    }
}

/// A designed look over the whole rig: a blue-to-amber wash on the static
/// lights, the moving heads fanned and lifted so their beams cross in the
/// air. `gobos` pushes every gobo wheel onto a slot that carries a picture.
fn stage_look(app: &mut App, gobos: bool) {
    // Colour by where each light actually stands, so the wash reads left to
    // right on screen rather than in patch order.
    let mut xs: Vec<f32> = vec![0.0; app.patch.fixtures.len()];
    for inst in &app.stage.instances {
        if let Some(x) = xs.get_mut(inst.fixture) {
            *x = inst.t.pos.x;
        }
    }
    let lo = xs.iter().copied().fold(f32::MAX, f32::min);
    let hi = xs.iter().copied().fold(f32::MIN, f32::max);
    let span = (hi - lo).max(0.001);

    let mut buf = [0u8; DMX_SLOTS];
    let mut movers = 0usize;
    for (i, f) in app.patch.fixtures.iter().enumerate() {
        let moving = matches!(classify(f), Archetype::Beam | Archetype::MovingPar);
        let (rgb, pan, tilt) = if moving {
            let phase = movers as f32 * 0.85;
            movers += 1;
            (
                beam_colour(movers),
                (128.0 + 48.0 * phase.sin()) as u8,
                (170.0 + 24.0 * (phase * 0.6).cos()) as u8,
            )
        } else {
            (wash((xs[i] - lo) / span), 128, 128)
        };
        drive(&mut buf, f, rgb, pan, tilt, 96, gobos);
    }
    let frame = Frame(buf);
    *app.net.dmx.lock() = frame;
    app.live = Look::from_frame(frame);
    app.live_active = (0..DMX_SLOTS).filter(|&a| buf[a] != 0).collect();
}

/// Push the lights whose wheels actually carry a picture to full, pale and
/// zoomed wide straight down onto the deck, and hold the rest of the rig
/// back to a wash: a gobo shows as the pattern in its floor pool, so that
/// pool has to be the brightest thing in the frame. Returns the lights lit.
fn gobo_look(app: &mut App) -> Vec<usize> {
    let mut buf = app.net.dmx.lock().0;
    let mut lit: Vec<usize> = Vec::new();
    for (fi, f) in app.patch.fixtures.iter().enumerate() {
        let carries = f
            .channels
            .iter()
            .any(|c| c.is_gobo_wheel() && c.bands.iter().any(|b| b.gobo.is_some()));
        // Keep the other heads throwing colour into the air for depth, but
        // take the static wash off the deck: a floor flood drowns a pattern.
        let backdrop = matches!(classify(f), Archetype::Beam | Archetype::MovingPar);
        for (i, ch) in f.channels.iter().enumerate() {
            let addr = f.from as usize + i;
            if addr == 0 || addr > DMX_SLOTS {
                continue;
            }
            let role = ch.role();
            if !carries {
                if role == Role::Dimmer {
                    buf[addr - 1] = if backdrop { 150 } else { 26 };
                }
                continue;
            }
            match role {
                Role::Dimmer | Role::Red | Role::Green | Role::Blue | Role::White => {
                    buf[addr - 1] = 255
                }
                Role::Cyan | Role::Magenta | Role::Yellow => buf[addr - 1] = 0,
                Role::Tilt => buf[addr - 1] = 128,
                Role::Pan => buf[addr - 1] = 128,
                Role::Zoom => buf[addr - 1] = 255,
                Role::Color => {
                    let slots = colour_slots(ch);
                    if !slots.is_empty() {
                        buf[addr - 1] = slots[lit.len() % slots.len()];
                    }
                }
                Role::Gobo if ch.is_gobo_wheel() => {
                    buf[addr - 1] = gobo_value(ch).unwrap_or(0);
                }
                _ => {}
            }
        }
        if carries {
            lit.push(fi);
        }
    }
    let frame = Frame(buf);
    *app.net.dmx.lock() = frame;
    app.live = Look::from_frame(frame);
    app.live_active = (0..DMX_SLOTS).filter(|&a| buf[a] != 0).collect();
    lit
}

/// Where `fixtures` stand, projected onto the floor.
fn floor_centre(app: &App, fixtures: &[usize]) -> V3 {
    let mut sum = v3(0.0, 0.0, 0.0);
    let mut n = 0.0f32;
    for inst in &app.stage.instances {
        if fixtures.contains(&inst.fixture) {
            sum = sum + v3(inst.t.pos.x, 0.0, inst.t.pos.z);
            n += 1.0;
        }
    }
    if n > 0.0 {
        sum * (1.0 / n)
    } else {
        v3(0.0, 0.0, 0.0)
    }
}

/// Re-aim the moving heads and pull the static wash down, so the thing a
/// shot is about — a gobo pattern, a truss — is the brightest thing on it.
/// `only_wheels` narrows the re-aim to the lights that carry gobos.
fn aim_heads(app: &mut App, tilt: u8, zoom: u8, wash: u8, only_wheels: bool) {
    let mut buf = app.net.dmx.lock().0;
    for f in &app.patch.fixtures {
        let lit = if only_wheels {
            f.channels.iter().any(|c| c.is_gobo_wheel())
        } else {
            matches!(classify(f), Archetype::Beam | Archetype::MovingPar)
        };
        for (i, ch) in f.channels.iter().enumerate() {
            let addr = f.from as usize + i;
            if addr == 0 || addr > DMX_SLOTS {
                continue;
            }
            match ch.role() {
                Role::Dimmer if !lit => buf[addr - 1] = wash,
                Role::Tilt if lit => buf[addr - 1] = tilt,
                Role::Zoom if lit => buf[addr - 1] = zoom,
                _ => {}
            }
        }
    }
    let frame = Frame(buf);
    *app.net.dmx.lock() = frame;
    app.live = Look::from_frame(frame);
}

// ------------------------------------------------------------- the shell --

/// Everything shut: the console with no window on top and no bottom bars.
fn quiet(app: &mut App) {
    app.show_settings = false;
    app.show_artnet = false;
    app.show_transition = false;
    app.show_chases = false;
    app.show_groups = false;
    app.show_orders = false;
    app.show_layers = false;
    app.show_scenes = false;
    app.show_audio = false;
    app.show_beat = false;
    app.show_palettes = false;
    app.show_phasers = false;
    app.show_stacks = false;
    app.show_decks = false;
    app.show_command = false;
    app.show_views = false;
    app.show_log = false;
    app.show_osc = false;
    app.show_phaser_board = false;
    app.show_preset_board = false;
    app.show_patch = false;
    app.show_gobos = false;
    app.show_configs = false;
    app.show_dmx_test = false;
    app.show_plugins = false;
    app.network.open = false;
    app.collapsed.clear();
}

/// Just the stage: both rails and the channel controls folded away.
fn stage_only(app: &mut App) {
    quiet(app);
    for key in ["fixtures", "inspector", "channels"] {
        app.collapsed.insert(key);
    }
}

/// Draw the frames, letting the caller change state between them.
fn draw_with(
    app: &mut App,
    size: [u32; 2],
    mut per_frame: impl FnMut(&mut App, usize),
) -> Option<Vec<u8>> {
    render_frames(FRAMES, size, |ctx, frame| {
        if frame == 0 {
            crate::ui::install_theme(ctx);
            return;
        }
        per_frame(app, frame);
        app.draw_ui(ctx);
    })
}

fn draw(app: &mut App, size: [u32; 2]) -> Option<Vec<u8>> {
    draw_with(app, size, |_, _| {})
}

/// How much bigger a window is drawn on the opening frames. egui's `Resize`
/// only ever *grows* a floating window to fit its content and never shrinks
/// it back on its own, so two oversized frames hand the window the room its
/// scroll areas then fill at normal size — otherwise a pool or a cue list
/// opens clipped to one row.
const ROOM: f32 = 1.35;

/// The pictures written so far, so the run reports what it made.
struct Sheet {
    made: Vec<(String, [u32; 2])>,
}

impl Sheet {
    fn keep(&mut self, pixels: &[u8], size: [u32; 2], name: &str) {
        let lit = pixels
            .chunks(4)
            .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120)
            .count();
        assert!(lit > 2000, "{name} came out dark ({lit} lit pixels)");
        write_png(pixels, size, name);
        self.made.push((name.to_owned(), size));
    }
}

/// One window over the lit console, at the house size. `room` is handed the
/// panel zoom to use (see [`ROOM`]); pass `|_, _| {}` when the window needs
/// no extra room.
fn window_shot(
    app: &mut App,
    sheet: &mut Sheet,
    name: &str,
    open: impl FnOnce(&mut App),
    room: impl Fn(&mut App, f32),
) {
    quiet(app);
    app.insp.prefs.tab = InspectorTab::Presets;
    select_some(app, 4);
    open(app);
    let px = draw_with(app, WIDE, |a, frame| {
        room(a, if frame <= 2 { ROOM } else { 1.0 });
    })
    .expect("the GPU went away mid-run");
    room(app, 1.0);
    sheet.keep(&px, WIDE, name);
}

/// Select the first `n` lights of the rig.
fn select_some(app: &mut App, n: usize) {
    let all: Vec<usize> = (0..n.min(app.patch.fixtures.len())).collect();
    select_these(app, &all);
}

/// Replace the stage selection with exactly these patch fixtures.
fn select_these(app: &mut App, fixtures: &[usize]) {
    app.stage.clear_selection();
    for &i in fixtures {
        app.stage.select_fixture(i, true);
    }
    app.sel_fixture = app.stage.last_selected;
    app.sync_selection_units();
}

/// Every moving head in the rig, in patch order.
fn movers(app: &App) -> Vec<usize> {
    app.patch
        .fixtures
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(classify(f), Archetype::Beam | Archetype::MovingPar))
        .map(|(i, _)| i)
        .collect()
}

/// Arm the first `per` channels of everything selected.
fn arm_selection(app: &mut App, per: usize) {
    let sel = app.stage.selected_fixtures();
    let mut armed = HashSet::new();
    for fi in sel {
        let Some(f) = app.patch.fixtures.get(fi) else { continue };
        for k in 0..per.min(f.channel_count()) {
            let addr = f.from as usize + k;
            if addr >= 1 && addr <= DMX_SLOTS {
                armed.insert(addr - 1);
            }
        }
    }
    app.sel_channels = armed;
}

/// Fill the programming tools with something worth photographing: routes,
/// cue stacks, scenes, board pads, a palette cycle, net tables and audio —
/// all in memory, none of it saved.
fn furnish(app: &mut App) {
    // A route through the operator's own groups.
    let mut steps: Vec<OrderStep> = app
        .groups
        .iter()
        .filter(|g| !g.fixtures.is_empty())
        .take(5)
        .map(OrderStep::from_group)
        .collect();
    if steps.is_empty() {
        steps = app
            .patch
            .fixtures
            .iter()
            .enumerate()
            .take(5)
            .map(|(i, f)| OrderStep::from_fixtures(f.display.clone(), vec![i]))
            .collect();
    }
    let reversed: Vec<OrderStep> = steps.iter().rev().cloned().collect();
    app.orders = vec![
        Order { id: 1, name: "Back to front".into(), steps },
        Order { id: 2, name: "Outside in".into(), steps: reversed },
    ];
    app.active_order = Some(0);
    app.order_edit = Some(0);

    // A layer stack mid-show: the base carrying the look, a movement layer
    // over it, and a blackout chop on top — the shape of the thing layers
    // exist for. Only *live state* is set here; the boxes come out of
    // `refresh_layer_boxes`, so the shot proves the derivation rather than
    // a hand-written list.
    {
        use crate::layer::ProgLayer;
        let fx: Vec<String> = app.phasers.iter().take(3).map(|p| p.name.clone()).collect();

        let mut base = ProgLayer::new(1, "Base".into());
        base.preset_id = app.user_presets.first().map(|p| p.id);
        base.active = (0..96).collect();
        for (k, pal) in app.palettes.iter().take(2).enumerate() {
            for a in (24 + k * 8)..(32 + k * 8) {
                base.refs.insert(a, pal.reference());
            }
        }

        let mut sweep = ProgLayer::new(2, "Movement".into());
        sweep.order = app.orders.first().map(|o| o.id);
        sweep.targets = (0..12).collect();
        sweep.active = (100..142).collect();

        let mut chop = ProgLayer::new(3, "Blackout chop".into());
        chop.order = app.orders.get(1).map(|o| o.id);
        chop.targets = (0..24).collect();
        chop.active = (200..240).collect();

        for (k, name) in fx.iter().take(2).enumerate() {
            app.active_phasers
                .insert(name.clone(), (100 + k * 18..118 + k * 18).collect());
        }
        if let Some(name) = fx.get(2) {
            app.active_phasers.insert(name.clone(), (200..240).collect());
        }

        app.layers = vec![base, sweep, chop];
        app.active_layer = 1;
        app.next_layer_id = 4;
        // The selected layer reads its state from the live programmer.
        app.live_active = std::mem::take(&mut app.layers[1].active);
        app.refresh_layer_boxes();
    }

    app.active_order = Some(0);
    app.order_edit = Some(0);

    // A layer stack mid-show: the base carrying the look, a movement layer
    // over it, and a blackout chop on top — the shape of the thing layers
    // exist for.
    {
        use crate::layer::{BoxKind, ProgLayer};
        let mut base = ProgLayer::new(1, "Base".into());
        base.boxes.push(crate::layer::LayerBox::new(
            BoxKind::Preset(1),
            "Blue 50".into(),
            (0..96).collect(),
        ));
        for (k, pal) in app.palettes.iter().take(2).enumerate() {
            base.boxes.push(crate::layer::LayerBox::new(
                BoxKind::Palette(pal.id),
                pal.name.clone(),
                (0..24 + k * 8).collect(),
            ));
        }
        // Real pool phasers, so the tiles carry their real pool colours.
        let fx: Vec<String> = app.phasers.iter().take(3).map(|p| p.name.clone()).collect();
        let mut sweep = ProgLayer::new(2, "Movement".into());
        sweep.order = app.orders.first().map(|o| o.id);
        sweep.targets = (0..12).collect();
        for (k, name) in fx.iter().take(2).enumerate() {
            sweep.boxes.push(crate::layer::LayerBox::new(
                BoxKind::Phaser(name.clone()),
                name.clone(),
                (0..18 + k * 6).collect(),
            ));
        }
        let mut chop = ProgLayer::new(3, "Blackout chop".into());
        chop.order = app.orders.get(1).map(|o| o.id);
        chop.targets = (0..24).collect();
        if let Some(name) = fx.get(2) {
            chop.boxes.push(crate::layer::LayerBox::new(
                BoxKind::Phaser(name.clone()),
                name.clone(),
                (0..40).collect(),
            ));
        }
        app.layers = vec![base, sweep, chop];
        app.active_layer = 1;
        app.next_layer_id = 4;
    }

    // Cue lists on three stacks, recorded off the look on stage.
    let buf = app.net.dmx.lock().0;
    let active: Vec<usize> = (0..DMX_SLOTS).filter(|&a| buf[a] != 0).take(240).collect();
    let mut main = Stack::new("Main".into());
    for (n, (name, fade)) in
        [("House in", 3.0), ("Verse", 2.0), ("Chorus", 0.5), ("Break", 6.0), ("Bows", 4.0)]
            .into_iter()
            .enumerate()
    {
        main.cues.push(Cue {
            number: n as f32 + 1.0,
            name: name.into(),
            fade,
            values: active
                .iter()
                .map(|&a| {
                    let v = (buf[a] as u32 * (55 + n as u32 * 40) / 255).min(255) as u8;
                    (a, CueVal::Absolute(v))
                })
                .collect(),
        });
    }
    main.current = Some(1);
    main.level = 0.86;
    app.cur_stack = Some(0);
    let mut fx = Stack::new("Specials".into());
    for (n, name) in ["Strobe hit", "Blinders"].into_iter().enumerate() {
        fx.cues.push(Cue {
            number: n as f32 + 1.0,
            name: name.into(),
            fade: 0.0,
            values: active.iter().take(60).map(|&a| (a, CueVal::Absolute(255))).collect(),
        });
    }
    fx.level = 0.42;
    let mut haze = Stack::new("Haze".into());
    haze.cues.push(Cue {
        number: 1.0,
        name: "On".into(),
        fade: 1.0,
        values: active.iter().take(12).map(|&a| (a, CueVal::Absolute(180))).collect(),
    });
    haze.level = 0.6;
    app.stacks = vec![main, fx, haze];
    app.grand_master = 0.88;
    app.transition.duration = 2.5;

    // Captured scenes.
    let scene = |name: &str, color: [u8; 3], level: f32, hold: f32| Scene {
        name: name.into(),
        color,
        values: active.iter().map(|&a| (a, buf[a])).collect(),
        oscs: Vec::new(),
        active: active.clone(),
        speed: 0.35,
        tempo: 124.0,
        master_speed: 1.0,
        merge: MergeMode::Highest,
        level,
        fade: 2.0,
        hold,
        order: Some("Back to front".into()),
        run: None,
        run_fade: 0.0,
        leaving: None,
    };
    app.scenes = vec![
        scene("Deep blue sweep", [70, 110, 235], 1.0, 8.0),
        scene("Amber pulse", [235, 150, 60], 0.75, 6.0),
        scene("Magenta chase", [225, 70, 180], 0.9, 4.0),
    ];

    // A page of board pads, from the operator's own presets.
    let pads = preset_pads_per_page().min(app.user_presets.len());
    for i in 0..pads {
        app.preset_deck[i] = app.preset_slot_for_user(i);
    }

    // Presets as pads, a pinned pair and a recall history.
    app.insp.prefs.presets.view = PresetView::Grid;
    app.insp.prefs.presets.pad = PadSize::M;
    for i in [1usize, 4, 6] {
        if let Some(p) = app.user_presets.get_mut(i) {
            p.pinned = true;
        }
    }
    app.insp.presets.recent =
        app.user_presets.iter().rev().take(4).map(|p| p.id).collect();

    // Saved cameras with their number keys.
    for (name, view, key, color) in [
        ("Front of house", QuickView::Foh, 1u8, [90, 170, 235]),
        ("Wide iso", QuickView::Iso, 2, [120, 210, 170]),
        ("Over the band", QuickView::Top, 3, [235, 165, 90]),
        ("Stage left", QuickView::Left, 4, [205, 120, 220]),
    ] {
        app.stage.quick_view(view, &app.settings, false, false);
        let mut mark = app.stage.bookmark(name.into());
        mark.hotkey = Some(key);
        mark.color = Some(color);
        app.cameras.push(mark);
    }
    app.insp.prefs.stage.show_camera_numbers = true;

    // Two Art-Net nodes for the output panel's target list.
    app.nodes = vec![
        DiscoveredNode {
            ip: Ipv4Addr::new(2, 0, 0, 10),
            short_name: "Node1".into(),
            long_name: "Stage left 4-port node".into(),
        },
        DiscoveredNode {
            ip: Ipv4Addr::new(2, 0, 0, 11),
            short_name: "Stage SR".into(),
            long_name: "Stage right 8-port node".into(),
        },
    ];
    app.selected = Some(Ipv4Addr::new(2, 0, 0, 10));

    // A palette cycle, ready to run.
    let colours: Vec<u32> = app
        .palettes
        .iter()
        .filter(|p| p.feature == Feature::Color)
        .take(6)
        .map(|p| p.id)
        .collect();
    if !colours.is_empty() {
        app.cycle_weights = vec![1.0; colours.len()];
        app.cycle_ids = colours;
        app.cycle_on = true;
        app.cycle_beats = 2.6;
        app.cycle_spread = 0.45;
    }
    app.master_bpm = 124.0;
    app.master_bpm_on = true;

    // Audio: a spectrum planted behind two trigger bands.
    let mut spectrum = [0.0f32; BINS];
    for (i, s) in spectrum.iter_mut().enumerate() {
        let t = i as f32 / BINS as f32;
        *s = ((1.0 - t).powf(1.5) * (0.6 + 0.4 * (t * 26.0).sin())).clamp(0.03, 1.0);
    }
    app.audio.plant(Analysis { spectrum, level: 0.62, bpm: 124.4, confidence: 0.78 }, true);
    app.audio_follow_beat = true;
    let palette_source = app.palettes.first().map(|p| TriggerSource::Palette(p.id));
    let mut bass = AudioTrigger::new(1);
    bass.name = "Bass".into();
    bass.env = 0.82;
    bass.source = palette_source;
    let mut air = AudioTrigger::new(2);
    air.name = "Snare".into();
    air.lo_hz = 1600.0;
    air.hi_hz = 5200.0;
    air.threshold = 0.45;
    air.env = 0.21;
    app.audio_triggers = vec![bass, air];
    app.audio_sel = Some(0);

    // Net tables, so the Network window has nodes to list.
    let now = Instant::now();
    if let Some(mut reply) = crate::artnet::parse_poll_reply(&crate::artnet::sample_reply()) {
        app.network.nodes.push(NodeEntry {
            reply: reply.clone(),
            from: reply.ip,
            first_seen: now,
            last_seen: now,
            replies: 24,
        });
        reply.ip = Ipv4Addr::new(2, 0, 0, 11);
        reply.short_name = "Stage SR".into();
        reply.long_name = "Stage right 8-port node".into();
        reply.sw_out = [0, 1, 2, 3];
        reply.node_report = "#0001 [0042] Power On Tests successful".into();
        app.network.nodes.push(NodeEntry {
            reply,
            from: Ipv4Addr::new(2, 0, 0, 11),
            first_seen: now,
            last_seen: now,
            replies: 17,
        });
    }
    app.network.sources.push(SourceEntry {
        cid: [7; 16],
        name: "Front of house desk".into(),
        from: Ipv4Addr::new(2, 0, 0, 50),
        universes: vec![1, 2, 3],
        data: vec![(1, 150, 9, 512, false, now)],
        last_seen: now,
    });
    app.network.stats = NetStats {
        universes: (0..3)
            .map(|u| UniverseStat {
                proto: if u == 2 { Proto::Sacn } else { Proto::ArtNet },
                page: u as usize,
                universe: u,
                fps: 40.0,
                bytes_per_s: 21_200.0,
                last_send: Some(now),
                total_frames: 48_000 + u as u64 * 200,
                destinations: vec![SocketAddrV4::new(Ipv4Addr::new(2, 255, 255, 255), 6454)],
            })
            .collect(),
        sockets: vec![
            "Art-Net listener 0.0.0.0:6454".into(),
            "sACN listener 0.0.0.0:5568".into(),
        ],
        last_error: None,
        error_count: 0,
        artnet_listening: true,
        sacn_ready: true,
        sacn_listening: true,
        multicast_if: None,
        uptime_s: 1840.0,
        test_active: false,
    };
    app.network.last_poll = Some(now);

    // The phaser on the bench: one of the operator's own.
    let built = app
        .phasers
        .iter()
        .find(|p| p.components.len() > 1)
        .or_else(|| app.phasers.first())
        .cloned();
    if let Some(p) = built {
        app.phaser_name = p.name.clone();
        app.phaser_edit = p;
    }

    // A chase pointed at a real look.
    if !app.user_presets.is_empty() {
        app.chase.source = Some(crate::chase::ChaseSource::User(0));
    }
    app.chase.enabled = true;
    // The sphere editor swallows the whole stage behind the window.
    app.chase.expanded = false;
    app.chase.speed = 0.6;
    app.chase.band_deg = 42.0;

    // Marketing house style: no name tags stacked over the rig, no
    // keyboard crib sheet across the foot of the stage.
    app.insp.prefs.display.labels = false;
    app.insp.prefs.display.help = false;
}

/// Every picture the site's manifest asks for, in one GPU session.
/// Development preview of the Transition window, simple and advanced, plus
/// the Palettes window's fade readout. Not part of the site; run on demand
/// with `cargo test transition_window_preview -- --ignored`.
#[test]
#[ignore]
fn transition_window_preview() {
    let mut app = App::new();
    app.stage.layout_path = std::env::temp_dir().join("dmxpress_transition_preview_layout.json");
    let _ = std::fs::remove_file(&app.stage.layout_path);
    let mut sheet = Sheet { made: Vec::new() };
    stage_look(&mut app, false);
    furnish(&mut app);
    if draw(&mut app, WIDE).is_none() {
        eprintln!("no GPU adapter — skipping the preview");
        return;
    }
    app.transition.duration = 2.5;
    window_shot(
        &mut app,
        &mut sheet,
        "preview-transition-simple",
        |a| a.show_transition = true,
        |_, _| {},
    );
    app.transition.advanced = true;
    {
        use crate::transition::{TransitionBinding, TransitionTarget};
        let slot = app.transition.slot_mut(TransitionTarget::PhaserEdit);
        slot.binding = TransitionBinding::Custom;
        slot.custom = 0.3;
        app.transition.slot_mut(TransitionTarget::GoboChange).binding = TransitionBinding::None;
    }
    window_shot(
        &mut app,
        &mut sheet,
        "preview-transition-advanced",
        |a| a.show_transition = true,
        |a, z| a.zoom.transition = z,
    );
    window_shot(
        &mut app,
        &mut sheet,
        "preview-palettes-readout",
        |a| a.show_palettes = true,
        |a, z| a.zoom.palettes = z,
    );
    for (name, size) in &sheet.made {
        eprintln!("wrote {name} at {}x{}", size[0], size[1]);
    }
}

#[test]
fn site_shots_render() {
    let mut app = App::new();
    // Nothing here may reach the operator's own arrangement.
    app.stage.layout_path = std::env::temp_dir().join("dmxpress_site_shots_layout.json");
    let _ = std::fs::remove_file(&app.stage.layout_path);
    let mut sheet = Sheet { made: Vec::new() };
    eprintln!(
        "rig: {} fixtures, {} palettes, {} presets, {} phasers, {} groups",
        app.patch.fixtures.len(),
        app.palettes.len(),
        app.user_presets.len(),
        app.phasers.len(),
        app.groups.len()
    );

    stage_look(&mut app, false);
    furnish(&mut app);

    // ---- the console and the stage ----
    quiet(&mut app);
    app.insp.prefs.tab = InspectorTab::Selection;
    app.insp.prefs.width = Some(330.0);
    select_some(&mut app, 6);
    arm_selection(&mut app, 4);
    app.stage.quick_view(QuickView::Iso, &app.settings, false, false);
    app.stage.cam.pitch = 0.30;
    app.stage.cam.dist *= 0.78;
    let Some(px) = draw(&mut app, WIDE) else {
        eprintln!("no GPU adapter — skipping the site shots");
        return;
    };
    sheet.keep(&px, WIDE, "shot-console");

    // The stage on its own.
    stage_only(&mut app);
    app.stage.clear_selection();
    app.stage.quick_view(QuickView::Iso, &app.settings, false, false);
    app.stage.cam.pitch = 0.26;
    app.stage.cam.dist *= 0.74;
    let px = draw(&mut app, WIDE).expect("gpu");
    sheet.keep(&px, WIDE, "shot-stage-beams");

    // Gobo breakup: only the wheels are lit, aimed down onto the deck, with
    // the wash pulled back so the patterns are the brightest thing on stage.
    let with_pictures = gobo_look(&mut app);
    eprintln!("{} lights carry a gobo the catalogue knows", with_pictures.len());
    assert!(!with_pictures.is_empty(), "no light in this rig projects a gobo the atlas holds");
    app.stage.clear_selection();
    // A spot's cone is narrow, so the pattern only reads from close in —
    // and with the haze turned up, so the breakup shows in the air as well
    // as on the deck.
    let haze = app.settings.beam_opacity;
    let fov = app.stage.cam.fov_y;
    app.settings.beam_opacity = 2.4;
    // A long lens rather than a short throw: the patterns come up to size
    // without the camera having to stand inside the beams.
    app.stage.cam.fov_y = 34.0_f32.to_radians();
    app.stage.cam.target = floor_centre(&app, &with_pictures[..1]) + v3(0.0, 0.7, 0.0);
    app.stage.cam.yaw = 0.75;
    app.stage.cam.pitch = 0.30;
    app.stage.cam.dist = 7.0;
    let px = draw(&mut app, WIDE).expect("gpu");
    sheet.keep(&px, WIDE, "shot-stage-gobos");
    app.settings.beam_opacity = haze;
    app.stage.cam.fov_y = fov;
    stage_look(&mut app, false);

    // Truss and towers, with lights hung on them.
    let heads = movers(&app);
    let box_truss = app.stage.build(
        &app.patch,
        Recipe::Box { width: 8.0, depth: 5.0, height: 4.6, yaw: 0.0, grounded: true },
        Placement::At(v3(0.0, 0.0, 0.0)),
    );
    let goalpost = app.stage.build(
        &app.patch,
        Recipe::Goalpost { span: 6.0, height: 3.6, yaw: 0.0 },
        Placement::At(v3(0.0, 0.0, -5.5)),
    );
    let runs: Vec<ElementRef> = box_truss
        .added
        .iter()
        .chain(goalpost.added.iter())
        .copied()
        .filter(|r| matches!(r, ElementRef::Truss(_)))
        .collect();
    for (n, &run) in runs.iter().enumerate() {
        let batch: Vec<usize> = heads.iter().skip(n * 4).take(4).copied().collect();
        if batch.is_empty() {
            break;
        }
        select_these(&mut app, &batch);
        let _ = app
            .stage
            .hang_selection(&app.patch, run, 1, HangFill::Spread, OrderBy::Address);
    }
    app.stage.clear_selection();
    stage_look(&mut app, false);
    // Heads pointed down off the bars and the wash held back, so the steel
    // reads as steel instead of disappearing into the haze.
    aim_heads(&mut app, 142, 150, 130, false);
    app.stage.frame_all(&app.settings, false);
    app.stage.cam.yaw = 0.60;
    app.stage.cam.pitch = 0.19;
    app.stage.cam.dist *= 0.60;
    let px = draw(&mut app, WIDE).expect("gpu");
    sheet.keep(&px, WIDE, "shot-stage-truss");
    stage_look(&mut app, false);

    // ---- the Inspector's five tabs ----
    quiet(&mut app);
    app.collapsed.insert("fixtures");
    app.collapsed.insert("channels");
    app.insp.prefs.width = Some(540.0);
    select_some(&mut app, 5);
    app.insp.patch.source = PatchSource::Library;
    app.patch_search = "chauvet spot".into();
    app.insp.patch.element = ElementChoice::NewTruss(TrussKind::Straight);
    app.insp.prefs.patch.pattern = PlacePattern::Truss;
    app.insp.build.kind = RecipeKind::Goalpost;
    app.insp.prefs.build.show_lights = true;
    for (tab, name) in [
        (InspectorTab::Presets, "shot-inspector-presets"),
        (InspectorTab::Selection, "shot-inspector-selection"),
        (InspectorTab::Stage, "shot-inspector-stage"),
        (InspectorTab::Build, "shot-inspector-build"),
        (InspectorTab::Patch, "shot-inspector-patch"),
    ] {
        app.insp.prefs.tab = tab;
        let px = draw(&mut app, WIDE).expect("gpu");
        sheet.keep(&px, WIDE, name);
    }

    // The tab strip on its own, caption sized.
    app.insp.prefs.tab = InspectorTab::Selection;
    app.insp.prefs.width = Some(900.0);
    let band = [960u32, 420];
    let px = draw(&mut app, band).expect("gpu");
    let (px, size) = crop(&px, band, [58, 44, 900, 260]);
    sheet.keep(&px, size, "shot-inspector-tabs");
    app.insp.prefs.width = Some(330.0);

    // ---- programming ----
    window_shot(
        &mut app,
        &mut sheet,
        "shot-palettes",
        |a| {
            a.show_palettes = true;
            a.palette_tab = Feature::Color;
        },
        |a, z| a.zoom.palettes = z,
    );
    window_shot(&mut app, &mut sheet, "shot-phasers", |a| a.show_phasers = true, |_, _| {});
    window_shot(
        &mut app,
        &mut sheet,
        "shot-phaser-board",
        |a| a.show_phaser_board = true,
        |_, _| {},
    );
    window_shot(
        &mut app,
        &mut sheet,
        "shot-preset-board",
        |a| {
            a.show_preset_board = true;
            a.insp.presets.board.arrange = true;
            a.insp.presets.board.slot = Some(4);
            if let Some(p) = a.user_presets.get(4) {
                let (id, name) = (p.id, p.name.clone());
                a.insp.presets.board.target = PadTarget::User(id);
                a.insp.presets.board.label = name;
            }
        },
        |_, _| {},
    );
    window_shot(&mut app, &mut sheet, "shot-chases", |a| a.show_chases = true, |_, _| {});
    window_shot(
        &mut app,
        &mut sheet,
        "shot-groups",
        |a| a.show_groups = true,
        |a, z| a.zoom.groups = z,
    );
    window_shot(
        &mut app,
        &mut sheet,
        "shot-orders",
        |a| a.show_orders = true,
        |a, z| a.zoom.orders = z,
    );
    window_shot(
        &mut app,
        &mut sheet,
        "shot-layers",
        |a| a.show_layers = true,
        |a, z| a.zoom.layers = z,
    );
    window_shot(
        &mut app,
        &mut sheet,
        "shot-scenes",
        |a| a.show_scenes = true,
        |a, z| a.zoom.scenes = z,
    );
    window_shot(
        &mut app,
        &mut sheet,
        "shot-stacks",
        |a| a.show_stacks = true,
        |a, z| a.zoom.stacks = z,
    );

    // The executor bar, wide and short: a letterboxed console, so the bar is
    // the subject rather than a strip of empty floor.
    stage_only(&mut app);
    app.stage.clear_selection();
    app.show_decks = true;
    // Rendered one toolbar taller and cropped back to it, so the picture
    // reads as a band lifted out of the console rather than a whole window.
    let deck_frame = [1600u32, 466];
    let px = draw_with(&mut app, deck_frame, |a, frame| {
        // Frame the rig once the stage knows how wide and short it is.
        if frame == 2 {
            a.stage.frame_all(&a.settings, false);
            a.stage.cam.dist *= 0.55;
        }
    })
    .expect("gpu");
    let (deck, size) = crop(&px, deck_frame, [0, 46, 1600, 420]);
    sheet.keep(&deck, size, "shot-decks");

    // The command line, the same treatment.
    stage_only(&mut app);
    app.stage.clear_selection();
    app.show_command = true;
    // A command the bar can actually parse: `run_command` takes a verb and at
    // most one argument, so a screenshot showing `group 3 + group 7 @ 80`
    // would advertise a grammar the app does not have.
    app.command = "group 3".into();
    let line = [1600u32, 300];
    let px = draw_with(&mut app, line, |a, frame| {
        if frame == 2 {
            a.stage.frame_all(&a.settings, false);
            a.stage.cam.dist *= 0.55;
        }
    })
    .expect("gpu");
    sheet.keep(&px, line, "shot-command");
    app.command.clear();

    // Channel control, with rows armed.
    quiet(&mut app);
    app.collapsed.insert("fixtures");
    app.collapsed.insert("inspector");
    select_some(&mut app, 4);
    arm_selection(&mut app, 6);
    let px = draw(&mut app, WIDE).expect("gpu");
    sheet.keep(&px, WIDE, "shot-channels");
    app.sel_channels = HashSet::new();

    // ---- rig and output ----
    window_shot(
        &mut app,
        &mut sheet,
        "shot-patch-window",
        |a| {
            a.show_patch = true;
            a.patch_search = "moving head".into();
            a.patch_count = 6;
            a.patch_name = "Spot".into();
        },
        |_, _| {},
    );
    // One fixture type, and one whose wheel the catalogue actually has
    // pictures for — a profile with nothing but "?" slots is a poor advert.
    let one_wheel: Vec<usize> = {
        let first = app
            .patch
            .fixtures
            .iter()
            .find(|f| {
                f.channels
                    .iter()
                    .any(|c| c.is_gobo_wheel() && c.bands.iter().any(|b| b.gobo.is_some()))
            })
            .map(|f| f.file.clone());
        app.patch
            .fixtures
            .iter()
            .enumerate()
            .filter(|(_, f)| Some(&f.file) == first.as_ref())
            .map(|(i, _)| i)
            .collect()
    };
    window_shot(
        &mut app,
        &mut sheet,
        "shot-gobos",
        |a| {
            a.show_gobos = true;
            select_these(a, &one_wheel);
        },
        |a, z| a.zoom.gobos = z,
    );
    window_shot(
        &mut app,
        &mut sheet,
        "shot-network",
        |a| {
            a.network.open = true;
            a.network.tab = NetTab::Nodes;
        },
        |_, _| {},
    );
    window_shot(&mut app, &mut sheet, "shot-artnet", |a| a.show_artnet = true, |_, _| {});
    window_shot(&mut app, &mut sheet, "shot-settings", |a| a.show_settings = true, |_, _| {});
    window_shot(
        &mut app,
        &mut sheet,
        "shot-audio",
        |a| a.show_audio = true,
        |a, z| a.zoom.audio = z,
    );
    window_shot(&mut app, &mut sheet, "shot-beat", |a| a.show_beat = true, |_, _| {});
    window_shot(
        &mut app,
        &mut sheet,
        "shot-plugins",
        |a| a.show_plugins = true,
        |a, z| a.zoom.plugins = z,
    );

    // Every Fixtures-panel look, side by side — each column keeps the
    // panel's own look picker above it, so the strip labels itself.
    quiet(&mut app);
    app.collapsed.insert("inspector");
    app.collapsed.insert("channels");
    select_some(&mut app, 3);
    let panel = [900u32, 700];
    let mut tiles = Vec::new();
    for look in RaidLook::ALL {
        app.settings.raid_look = look;
        let px = draw(&mut app, panel).expect("gpu");
        tiles.push(crop(&px, panel, [6, 86, 248, 500]));
    }
    app.settings.raid_look = RaidLook::Jewel;
    let (row, size) = strip(&tiles, 14, 18);
    sheet.keep(&row, size, "shot-raid-looks");

    eprintln!("--- {} shots written to {OUT_DIR} ---", sheet.made.len());
    for (name, [w, h]) in &sheet.made {
        eprintln!("  {name}.png  {w}x{h}");
    }
    assert!(!app.insp.dirty, "the shots must not mark Inspector prefs dirty");
}
