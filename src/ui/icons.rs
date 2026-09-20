//! Hand-drawn vector icons for the toolbar and window chrome.
//!
//! Emoji render differently on every platform — different metrics, different
//! colours, sometimes a tofu box. These are painted from primitives instead,
//! so a control looks the same on macOS and Windows and picks up the theme's
//! colours like any other widget.

use eframe::egui::{self, Color32, Pos2, Rect, Rounding, Shape, Stroke, Vec2};

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
    /// Layers panel: stacked sheets seen edge-on.
    Layer,
    Scene,
    Audio,
    Palette,
    Gobo,
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
    Plugin,
    Settings,
    Freeze,
    Play,
    // --- panel chrome ---
    /// Open a panel in its own OS window.
    PopOut,
    /// Bring a popped-out panel back into the main window.
    Dock,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    ChevronDown,
    /// A curved arrow back: take the last change back.
    Undo,
    Redo,
    /// A grid of pads: the Phaser board.
    Board,
    /// A filled square: stop.
    Stop,
    // --- area A ---
    /// Presets tab: a star in a rounded square.
    Presets,
    /// Selection tab: marquee corner brackets around a dot.
    Selection,
    /// Stage tab (and the stage-box toggle): a floor with a bar over it.
    Stage,
    /// Build tab: a hammer.
    Build,
    /// A camera body with its lens.
    Camera,
    /// A floor stand: pole, crossbar, feet, hung lights.
    Tower,
    /// A straight box-truss run: two rails and a lattice.
    TrussStraight,
    /// A curved truss: two concentric arcs with spokes.
    TrussRadius,
    /// A par can seen from the front, beam fanning out.
    Light,
    /// An eye: shown.
    Eye,
    /// An eye with a slash: hidden.
    EyeOff,
    /// An arrow into a bracket: jump to.
    Jump,
    /// A cross: close, clear, cancel.
    Close,
    /// A tick: confirm.
    Check,
    /// A bin with a lid.
    Trash,
    /// A closed folder.
    Folder,
    /// A pin: keep at the top.
    Pin,
    /// A magnifier.
    Search,
    /// Two overlapping squares: copy.
    Copy,
    /// A 2×2 grid.
    Grid,
    /// An inverted cone: a beam.
    Beam,
    /// A flattened ellipse under a dot: a light's pool on the floor.
    Pool,
    /// A name tag.
    Label,
    /// A dot with a tilted ring: orbit the camera.
    Orbit,
    /// A paper plane: fly the camera.
    Fly,
    /// A question mark in a circle.
    Help,
    /// The power symbol: blackout.
    Blackout,
    /// A rack of three drawers: a ShowBuddy bank.
    Bank,
    /// A six-dot grip: drag to reorder.
    Drag,
    /// A circular arrow: reset.
    Reset,
    // (area A appends new variants above this line)
    // --- area B ---
    /// A cube seen corner-on: the three-quarter stage view.
    CamIso,
    /// A flat wall with the viewer below it: looking in from the house.
    CamFront,
    /// The same wall with the viewer above it: looking in from upstage.
    CamBack,
    /// The viewer stood off to the left, looking across the stage.
    CamLeft,
    /// The viewer stood off to the right, looking across the stage.
    CamRight,
    /// An eye over a slab: the plan view, straight down.
    CamTop,
    /// A head and shoulders in front of the stage: the FOH seat.
    CamFoh,
    /// Marquee brackets closing on one dot: frame the selection.
    FrameSel,
    /// The same brackets around a row of dots: frame the whole rig.
    FrameAll,
    /// A crosshair: aim at this, or keep it in shot.
    Target,
    /// A ribbon with a notched tail: save this camera.
    Bookmark,
    /// A ring with its mounting slots marked on it.
    SlotRing,
    /// Three axis arrows out of one origin: the move handle.
    Gizmo,
    /// A dark light with its aim drawn out to a dot.
    Tick,
    /// A shape riding over a wave: the stage overlays.
    Overlay,
    /// A rule with major and minor ticks: fit the view to the rig.
    Ruler,
    /// Stops around a circle with an arrow between them: the camera tour.
    Tour,
    /// A magnifier with a plus: move the camera in.
    ZoomIn,
    /// The same magnifier with a bar: pull the camera back.
    ZoomOut,
    // (area B appends new variants above this line)
    // --- area C ---
    /// Two uprights under a beam: the goalpost build.
    Goalpost,
    /// A square of truss with braced corners: the box build.
    BoxRig,
    /// Two rails tied together: the circular truss build.
    Ring,
    /// A pair of towers, each under its own short beam.
    TowerPair,
    /// A half circle on two legs: the arched truss build.
    Arch,
    /// A disk with a written label: save this rig.
    Save,
    /// Two arcs chasing each other: read the folder again.
    Refresh,
    // (area C appends new variants above this line)
    // --- area D ---
    /// An arrow dropping into a tray: store the look.
    Store,
    /// Bulleted rows: show the presets as a list.
    List,
    /// A ramp off a baseline: this preset carries its own fade.
    Fade,
    /// Rows getting shorter: choose the sort order.
    Sort,
    // (area D appends new variants above this line)
    // --- area E ---
    /// A ring three quarters round clockwise about a fixed centre.
    RotateCw,
    /// The same ring turning the other way.
    RotateCcw,
    /// Two bars flush to a line down the left.
    AlignMin,
    /// Two bars centred on a line down the middle.
    AlignMid,
    /// Two bars flush to a line down the right.
    AlignMax,
    /// Even steps between two rails: space the selection out.
    Distribute,
    /// A shape and its reflection across a dashed upright.
    MirrorX,
    /// The same reflection across a dashed horizon.
    MirrorZ,
    /// Dots strung along a line.
    ArrangeLine,
    /// Dots following the sweep of an arc.
    ArrangeArc,
    /// Dots spaced evenly round a circle.
    ArrangeCircle,
    /// Dots on a three-by-three grid.
    ArrangeGrid,
    /// A light pointing straight down, splashing on the floor.
    AimDown,
    /// The same light pointing straight up.
    AimUp,
    /// An arrow along the horizon: aim flat out.
    AimLevel,
    /// A mover on its base with the beam away to a point.
    AimMover,
    /// Rays splaying out of one point.
    Fan,
    /// Two beams crossing, each from its own light.
    CrossAim,
    /// A globe: aim onto a sphere around the rig.
    Sphere,
    /// A light dropping off its bar.
    Detach,
    /// A hung light swung back onto its slot.
    Reseat,
    /// A clipboard with two written lines.
    Paste,
    /// Two boxes with an equals between them.
    Match,
    /// Two stacked cards with a plus: make copies.
    Duplicate,
    /// A marquee with a tick in it: take everything.
    SelectAll,
    /// A marquee with a cross in it: take nothing.
    SelectNone,
    /// A disc half filled: swap what is picked for what is not.
    Invert,
    /// Two matching blocks, each ticked.
    SameKind,
    /// A light under a bar it is not hung on.
    Unmounted,
    /// Arrows pushing apart from the centre.
    Spread,
    /// Arrows pulling in towards the centre.
    Contract,
    /// A measured drop between two bars.
    Height,
    /// Dots thrown about anyhow.
    Scatter,
    /// Dots alternating high and low along a zigzag.
    Stagger,
    /// A dot in the middle of the stage box.
    CentreStage,
    // (area E appends new variants above this line)
    // --- area F ---
    /// A plug with a bolt beside it: patch it now.
    QuickPatch,
    /// An open book: the fixture library.
    LibraryBook,
    /// A par can head-on: the built-in profiles.
    ParCan,
    /// A clock face: the profiles patched most recently.
    Recent,
    /// An arrow running up to a wall: the next free address.
    NextFree,
    /// Two blocks with a hole between them: fill the first gap.
    Gap,
    /// A hook off a bar: hang this on an element.
    Hook,
    /// A plug with a cross beside it: take it out of the patch.
    Unplug,
    /// A circled plus: one more of these.
    AddOne,
    /// A row of blocks with one picked out: select this type.
    SelectType,
    /// Dots strung down a line.
    ArrangeColumn,
    /// An arrow dropping onto the floor line.
    Floor,
    // (area F appends new variants above this line)
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

/// Unit-coordinate points along a circular arc: `n + 1` samples from `a0` to
/// `a1` degrees about `(cx, cy)`, measured anticlockwise from three o'clock.
///
/// Sweep clockwise by giving an `a1` below `a0`. The result feeds straight
/// into `path`, and the last two points give the tangent an arrowhead needs —
/// which is the whole rotate / refresh / hook / arch family in one place.
fn arc(cx: f32, cy: f32, rad: f32, a0: f32, a1: f32, n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
            (cx + rad * a.cos(), cy - rad * a.sin())
        })
        .collect()
}

/// The Undo / Redo glyph in unit coordinates: the arc over the top, then
/// the three points of the arrowhead sitting on the end it returns to.
///
/// `mirror` reflects the whole glyph across x = 0.5 for Redo. The head is
/// built in unmirrored space and reflected with the arc, exactly once: the
/// arm used to mirror the arc when it was sampled and then mirror the head's
/// wings again off the already-mirrored apex, which threw them to the far
/// side of the icon and drew a full-width zigzag.
fn undo_glyph(mirror: bool) -> (Vec<(f32, f32)>, [(f32, f32); 3]) {
    let flip = |x: f32| if mirror { 1.0 - x } else { x };
    let (a0, a1) = (205f32.to_radians(), -20f32.to_radians());
    let arc: Vec<(f32, f32)> = (0..=12)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / 12.0;
            (flip(0.5 + 0.32 * a.cos()), 0.56 - 0.32 * a.sin())
        })
        .collect();
    let (hx, hy) = (0.5 + 0.32 * a0.cos(), 0.56 - 0.32 * a0.sin());
    (arc, [(flip(hx + 0.2), hy + 0.02), (flip(hx), hy), (flip(hx + 0.06), hy - 0.2)])
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
        // Three sheets stacked edge-on, the top one lit: a layer stack.
        Icon::Layer => {
            for (i, y) in [0.70f32, 0.52, 0.34].iter().enumerate() {
                let lit = Stroke::new(s.width, color.gamma_multiply(0.45 + i as f32 * 0.275));
                path(painter, r, lit, &[(0.18, *y), (0.5, *y - 0.14), (0.82, *y), (0.5, *y + 0.14), (0.18, *y)]);
            }
        }
        // Equaliser bars mid-dance.
        Icon::Audio => {
            let bar = Stroke::new((side * 0.13).clamp(1.5, 3.0), color);
            for (x, h) in [(0.24f32, 0.38f32), (0.44, 0.66), (0.64, 0.48), (0.84, 0.76)] {
                path(painter, r, bar, &[(x, 0.86), (x, 0.86 - h)]);
            }
        }
        // Three offset cards: looks stacked on top of one another.
        Icon::Scene => {            for (i, (x, y)) in [(0.36f32, 0.34f32), (0.5, 0.5), (0.64, 0.66)]
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
        // A gobo: a disc with a ring of apertures around a centre hole.
        Icon::Gobo => {
            painter.circle_stroke(r.center(), side * 0.38, s);
            painter.circle_filled(r.center(), side * 0.08, color);
            for i in 0..6 {
                let a = i as f32 / 6.0 * std::f32::consts::TAU;
                let (sn, cs) = a.sin_cos();
                painter.circle_filled(
                    Pos2::new(r.center().x + cs * side * 0.24, r.center().y + sn * side * 0.24),
                    side * 0.055,
                    color,
                );
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
        // A power plug: two prongs into a body, cable trailing out.
        Icon::Plugin => {
            path(painter, r, s, &[(0.38, 0.08), (0.38, 0.28)]);
            path(painter, r, s, &[(0.62, 0.08), (0.62, 0.28)]);
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.26, 0.28), p(r, 0.74, 0.56)),
                side * 0.06,
                s,
            );
            path(painter, r, s, &[(0.5, 0.56), (0.5, 0.7), (0.34, 0.88)]);
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
        // A window with an arrow leaving through its corner.
        Icon::PopOut => {
            path(painter, r, s, &[(0.45, 0.2), (0.15, 0.2), (0.15, 0.85), (0.8, 0.85), (0.8, 0.55)]);
            path(painter, r, s, &[(0.5, 0.5), (0.85, 0.15)]);
            path(painter, r, s, &[(0.6, 0.15), (0.85, 0.15), (0.85, 0.4)]);
        }
        // The same window, the arrow coming back in.
        Icon::Dock => {
            path(painter, r, s, &[(0.45, 0.2), (0.15, 0.2), (0.15, 0.85), (0.8, 0.85), (0.8, 0.55)]);
            path(painter, r, s, &[(0.85, 0.15), (0.5, 0.5)]);
            path(painter, r, s, &[(0.5, 0.25), (0.5, 0.5), (0.75, 0.5)]);
        }
        Icon::ChevronLeft => path(painter, r, s, &[(0.62, 0.22), (0.36, 0.5), (0.62, 0.78)]),
        Icon::ChevronRight => path(painter, r, s, &[(0.38, 0.22), (0.64, 0.5), (0.38, 0.78)]),
        Icon::ChevronUp => path(painter, r, s, &[(0.22, 0.62), (0.5, 0.36), (0.78, 0.62)]),
        Icon::ChevronDown => path(painter, r, s, &[(0.22, 0.38), (0.5, 0.64), (0.78, 0.38)]),
        Icon::Board => {
            for j in 0..3 {
                for i in 0..3 {
                    let cell = Rect::from_min_size(
                        p(r, 0.12 + 0.3 * i as f32, 0.12 + 0.3 * j as f32),
                        Vec2::new(r.width() * 0.2, r.height() * 0.2),
                    );
                    painter.rect_filled(cell, 1.0, color);
                }
            }
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_min_max(p(r, 0.27, 0.27), p(r, 0.73, 0.73)), 1.5, color);
        }
        // --- area A draw ---
        // A rounded square holding a filled five-point star.
        Icon::Presets => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.14, 0.14), p(r, 0.86, 0.86)),
                side * 0.1,
                s,
            );
            let pt = |i: usize| {
                let a = (-90.0 + 36.0 * i as f32).to_radians();
                let rad = if i % 2 == 0 { 0.24 } else { 0.1 };
                p(r, 0.5 + rad * a.cos(), 0.5 + rad * a.sin())
            };
            let inner: Vec<Pos2> = (0..5).map(|k| pt(2 * k + 1)).collect();
            painter.add(Shape::convex_polygon(inner, color, Stroke::NONE));
            for k in 0..5 {
                painter.add(Shape::convex_polygon(
                    vec![pt(2 * k), pt(2 * k + 1), pt((2 * k + 9) % 10)],
                    color,
                    Stroke::NONE,
                ));
            }
        }
        // Four marquee corner brackets around a dot.
        Icon::Selection => {
            let (a, b, arm) = (0.16, 0.84, 0.18);
            path(painter, r, s, &[(a, a + arm), (a, a), (a + arm, a)]);
            path(painter, r, s, &[(b - arm, a), (b, a), (b, a + arm)]);
            path(painter, r, s, &[(b, b - arm), (b, b), (b - arm, b)]);
            path(painter, r, s, &[(a + arm, b), (a, b), (a, b - arm)]);
            painter.circle_filled(r.center(), side * 0.07, color);
        }
        // A stage floor with a bar over it and two lights hanging.
        Icon::Stage => {
            path(painter, r, s, &[(0.1, 0.82), (0.3, 0.5), (0.7, 0.5), (0.9, 0.82), (0.1, 0.82)]);
            path(painter, r, s, &[(0.2, 0.22), (0.8, 0.22)]);
            for x in [0.35, 0.65] {
                painter.circle_filled(p(r, x, 0.3), side * 0.05, color);
            }
        }
        // A hammer.
        Icon::Build => {
            path(painter, r, Stroke::new(w * 1.6, color), &[(0.18, 0.82), (0.6, 0.4)]);
            painter.add(Shape::convex_polygon(
                vec![p(r, 0.48, 0.24), p(r, 0.76, 0.52), p(r, 0.66, 0.62), p(r, 0.38, 0.34)],
                color,
                Stroke::NONE,
            ));
        }
        // A camera body with its lens.
        Icon::Camera => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.12, 0.32), p(r, 0.64, 0.72)),
                side * 0.08,
                s,
            );
            path(painter, r, s, &[(0.64, 0.46), (0.88, 0.32), (0.88, 0.72), (0.64, 0.58), (0.64, 0.46)]);
        }
        // Pole, crossbar, feet, three lights under the bar.
        Icon::Tower => {
            path(painter, r, s, &[(0.5, 0.24), (0.5, 0.88)]);
            path(painter, r, s, &[(0.2, 0.32), (0.8, 0.32)]);
            path(painter, r, s, &[(0.32, 0.88), (0.68, 0.88)]);
            for x in [0.32, 0.5, 0.68] {
                painter.circle_filled(p(r, x, 0.42), side * 0.05, color);
            }
        }
        // Two rails laced with a zigzag.
        Icon::TrussStraight => {
            path(painter, r, s, &[(0.1, 0.36), (0.9, 0.36)]);
            path(painter, r, s, &[(0.1, 0.64), (0.9, 0.64)]);
            path(painter, r, s, &[(0.1, 0.64), (0.3, 0.36), (0.5, 0.64), (0.7, 0.36), (0.9, 0.64)]);
        }
        // Two concentric arcs bulging upward, joined by spokes.
        Icon::TrussRadius => {
            let (cx, cy) = (0.5, 0.98);
            let (a0, a1) = (210f32.to_radians(), 330f32.to_radians());
            for rad in [0.68, 0.48] {
                let arc: Vec<(f32, f32)> = (0..=12)
                    .map(|i| {
                        let a = a0 + (a1 - a0) * i as f32 / 12.0;
                        (cx + rad * a.cos(), cy + rad * a.sin())
                    })
                    .collect();
                path(painter, r, s, &arc);
            }
            for deg in [225.0f32, 255.0, 285.0, 315.0] {
                let a = deg.to_radians();
                path(
                    painter,
                    r,
                    s,
                    &[(cx + 0.48 * a.cos(), cy + 0.48 * a.sin()), (cx + 0.68 * a.cos(), cy + 0.68 * a.sin())],
                );
            }
        }
        // A par can seen from the front, its beam fanning down.
        Icon::Light => {
            painter.circle_stroke(p(r, 0.5, 0.36), side * 0.2, s);
            painter.circle_filled(p(r, 0.5, 0.36), side * 0.07, color);
            path(painter, r, s, &[(0.36, 0.54), (0.2, 0.88)]);
            path(painter, r, s, &[(0.64, 0.54), (0.8, 0.88)]);
        }
        // An almond eye with a pupil, and the same crossed out.
        Icon::Eye | Icon::EyeOff => {
            let mut pts: Vec<(f32, f32)> = Vec::with_capacity(17);
            for i in 0..=8 {
                let t = i as f32 / 8.0;
                pts.push((0.08 + 0.84 * t, 0.5 - 0.26 * (t * std::f32::consts::PI).sin()));
            }
            for i in 1..8 {
                let t = 1.0 - i as f32 / 8.0;
                pts.push((0.08 + 0.84 * t, 0.5 + 0.26 * (t * std::f32::consts::PI).sin()));
            }
            pts.push(pts[0]);
            path(painter, r, s, &pts);
            painter.circle_filled(r.center(), side * 0.1, color);
            if icon == Icon::EyeOff {
                path(painter, r, Stroke::new(w * 1.3, color), &[(0.18, 0.82), (0.82, 0.18)]);
            }
        }
        // An arrow into a bracket.
        Icon::Jump => {
            path(painter, r, s, &[(0.12, 0.5), (0.6, 0.5)]);
            path(painter, r, s, &[(0.46, 0.36), (0.6, 0.5), (0.46, 0.64)]);
            path(painter, r, s, &[(0.7, 0.2), (0.88, 0.2), (0.88, 0.8), (0.7, 0.8)]);
        }
        Icon::Close => {
            path(painter, r, s, &[(0.24, 0.24), (0.76, 0.76)]);
            path(painter, r, s, &[(0.76, 0.24), (0.24, 0.76)]);
        }
        Icon::Check => {
            path(painter, r, Stroke::new(w * 1.2, color), &[(0.2, 0.54), (0.42, 0.76), (0.8, 0.28)]);
        }
        // A bin: lid, handle, body, two inner lines.
        Icon::Trash => {
            path(painter, r, s, &[(0.2, 0.28), (0.8, 0.28)]);
            path(painter, r, s, &[(0.42, 0.2), (0.58, 0.2)]);
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.28, 0.28), p(r, 0.72, 0.84)),
                side * 0.05,
                s,
            );
            for x in [0.42, 0.58] {
                path(painter, r, s, &[(x, 0.4), (x, 0.72)]);
            }
        }
        Icon::Folder => {
            path(
                painter,
                r,
                s,
                &[(0.12, 0.3), (0.38, 0.3), (0.46, 0.4), (0.88, 0.4), (0.88, 0.78), (0.12, 0.78), (0.12, 0.3)],
            );
        }
        // A push pin: head, crossbar, needle.
        Icon::Pin => {
            painter.circle_filled(p(r, 0.5, 0.34), side * 0.13, color);
            path(painter, r, s, &[(0.34, 0.5), (0.66, 0.5)]);
            path(painter, r, s, &[(0.5, 0.5), (0.5, 0.88)]);
        }
        // A magnifier.
        Icon::Search => {
            painter.circle_stroke(p(r, 0.42, 0.42), side * 0.24, s);
            path(painter, r, Stroke::new(w * 1.4, color), &[(0.6, 0.6), (0.86, 0.86)]);
        }
        // Two overlapping squares, the back one faded.
        Icon::Copy => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.36, 0.14), p(r, 0.82, 0.66)),
                side * 0.06,
                Stroke::new(w, color.gamma_multiply(0.6)),
            );
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.18, 0.32), p(r, 0.64, 0.86)),
                side * 0.06,
                s,
            );
        }
        // A 2×2 grid.
        Icon::Grid => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.16, 0.16), p(r, 0.84, 0.84)),
                side * 0.04,
                s,
            );
            path(painter, r, s, &[(0.5, 0.16), (0.5, 0.84)]);
            path(painter, r, s, &[(0.16, 0.5), (0.84, 0.5)]);
        }
        // An inverted cone from a point.
        Icon::Beam => {
            painter.circle_filled(p(r, 0.5, 0.16), side * 0.07, color);
            path(painter, r, s, &[(0.16, 0.86), (0.5, 0.16), (0.84, 0.86)]);
            path(painter, r, Stroke::new(w, color.gamma_multiply(0.5)), &[(0.16, 0.86), (0.84, 0.86)]);
        }
        // A flattened ellipse on the floor under a light.
        Icon::Pool => {
            let ring: Vec<(f32, f32)> = (0..=24)
                .map(|i| {
                    let a = i as f32 / 24.0 * std::f32::consts::TAU;
                    (0.5 + 0.36 * a.cos(), 0.7 + 0.14 * a.sin())
                })
                .collect();
            path(painter, r, s, &ring);
            painter.circle_filled(p(r, 0.5, 0.24), side * 0.06, color);
            path(painter, r, Stroke::new(w, color.gamma_multiply(0.5)), &[(0.5, 0.3), (0.5, 0.56)]);
        }
        // A name tag with its hole.
        Icon::Label => {
            path(painter, r, s, &[(0.14, 0.3), (0.6, 0.3), (0.86, 0.5), (0.6, 0.7), (0.14, 0.7), (0.14, 0.3)]);
            painter.circle_filled(p(r, 0.3, 0.5), side * 0.05, color);
        }
        // A body with a tilted ring around it.
        Icon::Orbit => {
            painter.circle_filled(r.center(), side * 0.12, color);
            let (sn, cs) = (-25f32).to_radians().sin_cos();
            let ring: Vec<(f32, f32)> = (0..=32)
                .map(|i| {
                    let a = i as f32 / 32.0 * std::f32::consts::TAU;
                    let (x, y) = (0.44 * a.cos(), 0.16 * a.sin());
                    (0.5 + x * cs - y * sn, 0.5 + x * sn + y * cs)
                })
                .collect();
            path(painter, r, s, &ring);
        }
        // A paper plane.
        Icon::Fly => {
            path(painter, r, s, &[(0.12, 0.5), (0.88, 0.16), (0.62, 0.86), (0.5, 0.56), (0.12, 0.5)]);
            path(painter, r, s, &[(0.12, 0.5), (0.5, 0.56)]);
        }
        // A question mark in a circle.
        Icon::Help => {
            painter.circle_stroke(r.center(), side * 0.38, s);
            path(
                painter,
                r,
                s,
                &[(0.38, 0.4), (0.42, 0.31), (0.5, 0.28), (0.58, 0.31), (0.62, 0.4), (0.58, 0.48), (0.5, 0.53), (0.5, 0.6)],
            );
            painter.circle_filled(p(r, 0.5, 0.72), side * 0.045, color);
        }
        // The power symbol: a broken ring with a bar through the gap.
        Icon::Blackout => {
            let (a0, a1) = (300f32.to_radians(), 600f32.to_radians());
            let arc: Vec<(f32, f32)> = (0..=20)
                .map(|i| {
                    let a = a0 + (a1 - a0) * i as f32 / 20.0;
                    (0.5 + 0.32 * a.cos(), 0.54 + 0.32 * a.sin())
                })
                .collect();
            path(painter, r, s, &arc);
            path(painter, r, s, &[(0.5, 0.16), (0.5, 0.5)]);
        }
        // A rack of three drawers, each with its handle.
        Icon::Bank => {
            for (y0, y1) in [(0.18, 0.36), (0.41, 0.59), (0.64, 0.82)] {
                painter.rect_stroke(Rect::from_min_max(p(r, 0.14, y0), p(r, 0.86, y1)), side * 0.03, s);
                painter.circle_filled(p(r, 0.76, (y0 + y1) * 0.5), side * 0.04, color);
            }
        }
        // A six-dot grip.
        Icon::Drag => {
            for x in [0.4, 0.6] {
                for y in [0.28, 0.5, 0.72] {
                    painter.circle_filled(p(r, x, y), side * 0.06, color);
                }
            }
        }
        // A near-full ring with an arrowhead at its end.
        Icon::Reset => {
            let (a0, a1) = (60f32.to_radians(), 360f32.to_radians());
            let arc: Vec<(f32, f32)> = (0..=20)
                .map(|i| {
                    let a = a0 + (a1 - a0) * i as f32 / 20.0;
                    (0.5 + 0.32 * a.cos(), 0.5 - 0.32 * a.sin())
                })
                .collect();
            path(painter, r, s, &arc);
            let (hx, hy) = arc[arc.len() - 1];
            path(painter, r, s, &[(hx + 0.16, hy - 0.02), (hx, hy), (hx + 0.04, hy + 0.16)]);
        }
        // --- area B draw ---
        // A cube corner-on: the outline plus the three edges meeting at the middle.
        Icon::CamIso => {
            path(
                painter,
                r,
                s,
                &[
                    (0.5, 0.1),
                    (0.85, 0.3),
                    (0.85, 0.7),
                    (0.5, 0.9),
                    (0.15, 0.7),
                    (0.15, 0.3),
                    (0.5, 0.1),
                ],
            );
            for (x, y) in [(0.5, 0.9), (0.85, 0.3), (0.15, 0.3)] {
                path(painter, r, s, &[(0.5, 0.5), (x, y)]);
            }
        }
        // The stage box with the viewer stood below it (Front) or above it
        // (Back), looking its way. One arrow, not a cone of sight lines —
        // converging lines only make a lampshade at 16 px.
        Icon::CamFront | Icon::CamBack => {
            let f = |y: f32| if icon == Icon::CamBack { 1.0 - y } else { y };
            painter.rect_stroke(
                Rect::from_two_pos(p(r, 0.14, f(0.1)), p(r, 0.86, f(0.46))),
                side * 0.05,
                s,
            );
            painter.circle_filled(p(r, 0.5, f(0.92)), side * 0.1, color);
            path(painter, r, s, &[(0.5, f(0.82)), (0.5, f(0.58))]);
            path(painter, r, s, &[(0.36, f(0.72)), (0.5, f(0.58)), (0.64, f(0.72))]);
        }
        // The same box with the viewer off to one side.
        Icon::CamLeft | Icon::CamRight => {
            let f = |x: f32| if icon == Icon::CamRight { 1.0 - x } else { x };
            painter.rect_stroke(
                Rect::from_two_pos(p(r, f(0.54), 0.14), p(r, f(0.9), 0.86)),
                side * 0.05,
                s,
            );
            painter.circle_filled(p(r, f(0.08), 0.5), side * 0.1, color);
            path(painter, r, s, &[(f(0.18), 0.5), (f(0.42), 0.5)]);
            path(painter, r, s, &[(f(0.28), 0.36), (f(0.42), 0.5), (f(0.28), 0.64)]);
        }
        // An eye straight over the deck: the plan view.
        Icon::CamTop => {
            path(
                painter,
                r,
                s,
                &[(0.16, 0.34), (0.5, 0.12), (0.84, 0.34), (0.5, 0.56), (0.16, 0.34)],
            );
            painter.circle_filled(p(r, 0.5, 0.34), side * 0.095, color);
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.14, 0.78), p(r, 0.86, 0.94)),
                side * 0.04,
                s,
            );
        }
        // Head and shoulders in the house, the stage spread out beyond.
        Icon::CamFoh => {
            path(
                painter,
                r,
                s,
                &[(0.15, 0.4), (0.3, 0.15), (0.7, 0.15), (0.85, 0.4), (0.15, 0.4)],
            );
            painter.circle_filled(p(r, 0.5, 0.62), side * 0.09, color);
            path(painter, r, s, &[(0.32, 0.88), (0.5, 0.73), (0.68, 0.88)]);
        }
        // Marquee corners around one dot (selection) or a row of them (all).
        Icon::FrameSel | Icon::FrameAll => {
            for (cx, cy, dx, dy) in [
                (0.1f32, 0.1f32, 1.0f32, 1.0f32),
                (0.9, 0.1, -1.0, 1.0),
                (0.9, 0.9, -1.0, -1.0),
                (0.1, 0.9, 1.0, -1.0),
            ] {
                path(
                    painter,
                    r,
                    s,
                    &[(cx + dx * 0.22, cy), (cx, cy), (cx, cy + dy * 0.22)],
                );
            }
            if icon == Icon::FrameSel {
                // A solid block, not a dot: the Selection tab's icon is the same
                // brackets around a dot, and both are on screen together.
                painter.rect_filled(
                    Rect::from_center_size(r.center(), Vec2::splat(side * 0.26)),
                    side * 0.04,
                    color,
                );
            } else {
                for x in [0.24f32, 0.5, 0.76] {
                    painter.circle_filled(p(r, x, 0.5), side * 0.065, color);
                }
            }
        }
        // A crosshair: ring, four axis ticks through it, dot dead centre.
        Icon::Target => {
            painter.circle_stroke(r.center(), side * 0.32, s);
            for i in 0..4 {
                let a = i as f32 / 4.0 * std::f32::consts::TAU;
                let (sn, cs) = a.sin_cos();
                painter.line_segment(
                    [
                        Pos2::new(
                            r.center().x + cs * side * 0.24,
                            r.center().y + sn * side * 0.24,
                        ),
                        Pos2::new(
                            r.center().x + cs * side * 0.44,
                            r.center().y + sn * side * 0.44,
                        ),
                    ],
                    s,
                );
            }
            painter.circle_filled(r.center(), side * 0.08, color);
        }
        // A ribbon with the tail notched out of it.
        Icon::Bookmark => {
            path(
                painter,
                r,
                s,
                &[(0.28, 0.12), (0.72, 0.12), (0.72, 0.88), (0.5, 0.68), (0.28, 0.88), (0.28, 0.12)],
            );
        }
        // A bare ring with the four mounting slots sat on it — no centre mark,
        // which is what keeps it apart from Target.
        Icon::SlotRing => {
            painter.circle_stroke(r.center(), side * 0.32, s);
            for i in 0..4 {
                let a = (45.0 + 90.0 * i as f32).to_radians();
                painter.circle_filled(
                    Pos2::new(
                        r.center().x + a.cos() * side * 0.32,
                        r.center().y - a.sin() * side * 0.32,
                    ),
                    side * 0.065,
                    color,
                );
            }
        }
        // Three axis arrows out of a common origin.
        Icon::Gizmo => {
            for (tx, ty) in [(0.5f32, 0.12f32), (0.9, 0.55), (0.22, 0.85)] {
                let (dx, dy) = (tx - 0.5, ty - 0.55);
                let l = dx.hypot(dy);
                let (ux, uy) = (dx / l, dy / l);
                path(painter, r, s, &[(0.5, 0.55), (tx, ty)]);
                path(
                    painter,
                    r,
                    s,
                    &[
                        (tx - ux * 0.15 - uy * 0.09, ty - uy * 0.15 + ux * 0.09),
                        (tx, ty),
                        (tx - ux * 0.15 + uy * 0.09, ty - uy * 0.15 - ux * 0.09),
                    ],
                );
            }
            painter.circle_filled(p(r, 0.5, 0.55), side * 0.06, color);
        }
        // A dark head with its aim drawn out to where it lands.
        Icon::Tick => {
            painter.circle_stroke(p(r, 0.26, 0.74), side * 0.16, s);
            path(painter, r, s, &[(0.42, 0.6), (0.78, 0.24)]);
            painter.circle_filled(p(r, 0.82, 0.2), side * 0.09, color);
        }
        // A shape riding over a wave: the transition sphere and the traces.
        // Centred, so it reads as a shape sat over a trace rather than as
        // Tick's circle-with-a-tail.
        Icon::Overlay => {
            painter.circle_stroke(p(r, 0.5, 0.3), side * 0.22, s);
            wave(painter, r, s, 0.82, 0.1, 0.0);
        }
        // A rule with major and minor ticks hanging off its top edge.
        Icon::Ruler => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.08, 0.36), p(r, 0.92, 0.64)),
                side * 0.04,
                s,
            );
            for x in [0.24f32, 0.5, 0.76] {
                path(painter, r, s, &[(x, 0.36), (x, 0.5)]);
            }
            for x in [0.16f32, 0.37, 0.63, 0.84] {
                path(painter, r, s, &[(x, 0.36), (x, 0.44)]);
            }
        }
        // Three stops around a ring with one arrowhead showing the way round —
        // dots on the arc, so it never reads as a plain refresh.
        Icon::Tour => {
            painter.circle_stroke(r.center(), side * 0.32, s);
            for deg in [90.0f32, 210.0, 330.0] {
                let a = deg.to_radians();
                painter.circle_filled(
                    Pos2::new(
                        r.center().x + a.cos() * side * 0.32,
                        r.center().y - a.sin() * side * 0.32,
                    ),
                    side * 0.08,
                    color,
                );
            }
            // Clockwise tangent at 30°, so the head sits on the arc pointing on.
            let a = 30f32.to_radians();
            let tip = Pos2::new(
                r.center().x + a.cos() * side * 0.32,
                r.center().y - a.sin() * side * 0.32,
            );
            let (tx, ty) = (a.sin(), a.cos());
            let (nx, ny) = (a.cos(), -a.sin());
            painter.add(Shape::line(
                vec![
                    tip - Vec2::new(tx, ty) * side * 0.13 + Vec2::new(nx, ny) * side * 0.09,
                    tip,
                    tip - Vec2::new(tx, ty) * side * 0.13 - Vec2::new(nx, ny) * side * 0.09,
                ],
                s,
            ));
        }
        // A magnifier with a plus (in) or a bare bar (out).
        Icon::ZoomIn | Icon::ZoomOut => {
            painter.circle_stroke(p(r, 0.42, 0.42), side * 0.26, s);
            path(
                painter,
                r,
                Stroke::new(w * 1.4, color),
                &[(0.62, 0.62), (0.86, 0.86)],
            );
            path(painter, r, s, &[(0.3, 0.42), (0.54, 0.42)]);
            if icon == Icon::ZoomIn {
                path(painter, r, s, &[(0.42, 0.3), (0.42, 0.54)]);
            }
        }
                // (area B appends new draw arms above this line)
        // --- area C draw ---
        // Two uprights under a doubled beam, feet on the deck.
        Icon::Goalpost => {
            path(painter, r, Stroke::new(w * 1.4, color), &[(0.14, 0.28), (0.86, 0.28)]);
            path(painter, r, Stroke::new(w * 0.75, color), &[(0.18, 0.37), (0.82, 0.37)]);
            for x in [0.22f32, 0.78] {
                path(painter, r, s, &[(x, 0.28), (x, 0.86)]);
            }
            path(painter, r, s, &[(0.14, 0.86), (0.3, 0.86)]);
            path(painter, r, s, &[(0.7, 0.86), (0.86, 0.86)]);
        }
        // A square of truss with every corner braced off.
        Icon::BoxRig => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.16, 0.16), p(r, 0.84, 0.84)),
                side * 0.04,
                s,
            );
            for (a, b, c, d) in [
                (0.16f32, 0.3f32, 0.3f32, 0.16f32),
                (0.7, 0.16, 0.84, 0.3),
                (0.84, 0.7, 0.7, 0.84),
                (0.3, 0.84, 0.16, 0.7),
            ] {
                path(painter, r, s, &[(a, b), (c, d)]);
            }
        }
        // Two rails with ties between them: truss bent into a circle. The
        // rails need a fat gap or they close up into a donut at 16 px.
        Icon::Ring => {
            painter.circle_stroke(r.center(), side * 0.42, s);
            painter.circle_stroke(r.center(), side * 0.21, s);
            for i in 0..4 {
                let a = (45.0 + 90.0 * i as f32).to_radians();
                let (cs, sn) = (a.cos(), -a.sin());
                painter.line_segment(
                    [
                        Pos2::new(r.center().x + cs * side * 0.21, r.center().y + sn * side * 0.21),
                        Pos2::new(r.center().x + cs * side * 0.42, r.center().y + sn * side * 0.42),
                    ],
                    s,
                );
            }
        }
        // A pair of towers, each carrying its own short beam.
        Icon::TowerPair => {
            for x in [0.3f32, 0.7] {
                path(painter, r, s, &[(x, 0.3), (x, 0.86)]);
            }
            path(painter, r, s, &[(0.14, 0.3), (0.46, 0.3)]);
            path(painter, r, s, &[(0.54, 0.3), (0.86, 0.3)]);
            path(painter, r, s, &[(0.2, 0.86), (0.4, 0.86)]);
            path(painter, r, s, &[(0.6, 0.86), (0.8, 0.86)]);
        }
        // A half circle of truss stood on two legs.
        Icon::Arch => {
            path(painter, r, s, &arc(0.5, 0.6, 0.38, 180.0, 0.0, 12));
            path(painter, r, s, &arc(0.5, 0.6, 0.21, 180.0, 0.0, 8));
            path(painter, r, s, &[(0.12, 0.6), (0.12, 0.9)]);
            path(painter, r, s, &[(0.88, 0.6), (0.88, 0.9)]);
        }
        // A disk with the notch at the top and a written label below.
        Icon::Save => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.15, 0.15), p(r, 0.85, 0.85)),
                side * 0.08,
                s,
            );
            painter.rect_filled(
                Rect::from_min_max(p(r, 0.32, 0.15), p(r, 0.68, 0.38)),
                0.0,
                color,
            );
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.3, 0.6), p(r, 0.7, 0.85)),
                side * 0.03,
                s,
            );
        }
        // Two arcs chasing each other's tails: read it again from disk.
        Icon::Refresh => {
            for (a0, a1) in [(20.0f32, 170.0f32), (200.0, 350.0)] {
                let a = arc(0.5, 0.5, 0.34, a0, a1, 10);
                path(painter, r, s, &a);
                let (px, py) = a[a.len() - 1];
                let (qx, qy) = a[a.len() - 2];
                let l = (px - qx).hypot(py - qy);
                let (ux, uy) = ((px - qx) / l, (py - qy) / l);
                path(
                    painter,
                    r,
                    s,
                    &[
                        (px - ux * 0.13 - uy * 0.1, py - uy * 0.13 + ux * 0.1),
                        (px, py),
                        (px - ux * 0.13 + uy * 0.1, py - uy * 0.13 - ux * 0.1),
                    ],
                );
            }
        }
                // (area C appends new draw arms above this line)
        // --- area D draw ---
        // An arrow dropping into an open tray.
        Icon::Store => {
            path(painter, r, s, &[(0.16, 0.62), (0.16, 0.84), (0.84, 0.84), (0.84, 0.62)]);
            path(painter, r, s, &[(0.5, 0.14), (0.5, 0.62)]);
            path(painter, r, s, &[(0.32, 0.46), (0.5, 0.62), (0.68, 0.46)]);
        }
        // Bulleted rows.
        Icon::List => {
            for y in [0.28f32, 0.5, 0.72] {
                painter.circle_filled(p(r, 0.2, y), side * 0.07, color);
                path(painter, r, s, &[(0.35, y), (0.86, y)]);
            }
        }
        // A ramp lifting off a dim baseline: this one fades on its own.
        Icon::Fade => {
            path(
                painter,
                r,
                Stroke::new(w, color.gamma_multiply(0.55)),
                &[(0.12, 0.82), (0.88, 0.82)],
            );
            let ramp: Vec<(f32, f32)> = (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    (0.14 + t * 0.72, 0.8 - t * t * (3.0 - 2.0 * t) * 0.58)
                })
                .collect();
            path(painter, r, s, &ramp);
        }
        // Rows getting shorter down the page: the sort order.
        Icon::Sort => {
            for (y, len) in [(0.28f32, 0.7f32), (0.5, 0.5), (0.72, 0.3)] {
                path(painter, r, s, &[(0.15, y), (0.15 + len, y)]);
            }
        }
                // (area D appends new draw arms above this line)
        // --- area E draw ---
        // Three quarters of a ring with a chevron on the leading end and a
        // solid pivot in the middle — the pivot is what tells it from Reset.
        Icon::RotateCw | Icon::RotateCcw => {
            let flip = |x: f32| if icon == Icon::RotateCcw { 1.0 - x } else { x };
            let ring = arc(0.5, 0.52, 0.32, 0.0, -270.0, 24);
            let ring: Vec<(f32, f32)> = ring.iter().map(|&(x, y)| (flip(x), y)).collect();
            path(painter, r, s, &ring);
            let (px, py) = ring[ring.len() - 1];
            let (qx, qy) = ring[ring.len() - 2];
            let l = (px - qx).hypot(py - qy);
            let (ux, uy) = ((px - qx) / l, (py - qy) / l);
            path(
                painter,
                r,
                s,
                &[
                    (px - ux * 0.11 - uy * 0.11, py - uy * 0.11 + ux * 0.11),
                    (px, py),
                    (px - ux * 0.11 + uy * 0.11, py - uy * 0.11 - ux * 0.11),
                ],
            );
            painter.rect_filled(
                Rect::from_center_size(p(r, 0.5, 0.52), Vec2::splat(side * 0.16)),
                0.0,
                color,
            );
        }
        // Two bars brought up against a guide line.
        Icon::AlignMin | Icon::AlignMid | Icon::AlignMax => {
            let (line, top, bot) = match icon {
                Icon::AlignMin => (0.18f32, (0.18f32, 0.85f32), (0.18f32, 0.6f32)),
                Icon::AlignMax => (0.82, (0.15, 0.82), (0.4, 0.82)),
                _ => (0.5, (0.15, 0.85), (0.3, 0.7)),
            };
            path(painter, r, Stroke::new(w * 0.8, color), &[(line, 0.1), (line, 0.9)]);
            painter.rect_filled(
                Rect::from_min_max(p(r, top.0, 0.28), p(r, top.1, 0.42)),
                side * 0.03,
                color,
            );
            painter.rect_filled(
                Rect::from_min_max(p(r, bot.0, 0.58), p(r, bot.1, 0.72)),
                side * 0.03,
                color,
            );
        }
        // Four uprights at even centres — two rails and two movers. Three
        // inner marks would close up into a solid block at 16 px.
        Icon::Distribute => {
            for x in [0.14f32, 0.86] {
                path(painter, r, s, &[(x, 0.1), (x, 0.9)]);
            }
            for x in [0.38f32, 0.62] {
                path(painter, r, Stroke::new(w * 1.4, color), &[(x, 0.3), (x, 0.7)]);
            }
        }
        // A solid shape and its hollow reflection across a dashed axis.
        Icon::MirrorX | Icon::MirrorZ => {
            let t = |x: f32, y: f32| if icon == Icon::MirrorZ { (1.0 - y, x) } else { (x, y) };
            for y0 in [0.1f32, 0.31, 0.52, 0.73] {
                let (ax, ay) = t(0.5, y0);
                let (bx, by) = t(0.5, y0 + 0.12);
                path(painter, r, s, &[(ax, ay), (bx, by)]);
            }
            let solid: Vec<Pos2> = [(0.12f32, 0.3f32), (0.42, 0.5), (0.12, 0.7)]
                .iter()
                .map(|&(x, y)| {
                    let (x, y) = t(x, y);
                    p(r, x, y)
                })
                .collect();
            painter.add(Shape::convex_polygon(solid, color, Stroke::NONE));
            let ghost: Vec<(f32, f32)> = [(0.88f32, 0.3f32), (0.58, 0.5), (0.88, 0.7), (0.88, 0.3)]
                .iter()
                .map(|&(x, y)| t(x, y))
                .collect();
            path(painter, r, s, &ghost);
        }
        // Lights strung along a line across the stage. The rail is drawn thin
        // so the lights stand proud of it at 16 px.
        Icon::ArrangeLine => {
            path(painter, r, Stroke::new(w * 0.7, color), &[(0.1, 0.5), (0.9, 0.5)]);
            for x in [0.13f32, 0.38, 0.62, 0.87] {
                painter.circle_filled(p(r, x, 0.5), side * 0.095, color);
            }
        }
        // Lights following the sweep of an arc, marked at both ends and the bottom.
        Icon::ArrangeArc => {
            path(
                painter,
                r,
                Stroke::new(w * 0.7, color),
                &arc(0.5, 0.32, 0.38, 180.0, 360.0, 24),
            );
            for (x, y) in [(0.12, 0.32), (0.5, 0.7), (0.88, 0.32)] {
                painter.circle_filled(p(r, x, y), side * 0.095, color);
            }
        }
        // Lights spaced evenly round a circle, on the axes — fat lights on a
        // thin ring, which is what tells it from SlotRing's pips.
        Icon::ArrangeCircle => {
            painter.circle_stroke(r.center(), side * 0.34, Stroke::new(w * 0.7, color));
            for i in 0..4 {
                let a = i as f32 / 4.0 * std::f32::consts::TAU;
                let (sn, cs) = a.sin_cos();
                painter.circle_filled(
                    Pos2::new(
                        r.center().x + cs * side * 0.34,
                        r.center().y + sn * side * 0.34,
                    ),
                    side * 0.1,
                    color,
                );
            }
        }
        // A three-by-three block of lights.
        Icon::ArrangeGrid => {
            for y in [0.22f32, 0.5, 0.78] {
                for x in [0.22f32, 0.5, 0.78] {
                    painter.circle_filled(p(r, x, y), side * 0.07, color);
                }
            }
        }
        // A head with its aim thrown straight down (or straight up). The head
        // dot on the tail is what tells it from Floor and from ChevronDown;
        // the spec's splayed rays went to mush at 16 px and are dropped.
        Icon::AimDown | Icon::AimUp => {
            let f = |y: f32| if icon == Icon::AimUp { 1.0 - y } else { y };
            painter.circle_filled(p(r, 0.5, f(0.14)), side * 0.11, color);
            path(painter, r, s, &[(0.5, f(0.26)), (0.5, f(0.84))]);
            path(painter, r, s, &[(0.3, f(0.64)), (0.5, f(0.84)), (0.7, f(0.64))]);
        }
        // An arrow straight out along the horizon.
        Icon::AimLevel => {
            path(painter, r, s, &[(0.15, 0.5), (0.8, 0.5)]);
            path(painter, r, s, &[(0.66, 0.36), (0.8, 0.5), (0.66, 0.64)]);
            path(
                painter,
                r,
                Stroke::new(w, color.gamma_multiply(0.5)),
                &[(0.15, 0.76), (0.85, 0.76)],
            );
        }
        // A mover on its base, the beam away to where it lands. A yoke arc
        // this small only smears into the base, so the head is a solid disc.
        Icon::AimMover => {
            painter.rect_filled(
                Rect::from_min_max(p(r, 0.24, 0.82), p(r, 0.76, 0.94)),
                side * 0.03,
                color,
            );
            path(painter, r, s, &[(0.5, 0.82), (0.5, 0.66)]);
            painter.circle_filled(p(r, 0.5, 0.58), side * 0.13, color);
            path(painter, r, s, &[(0.58, 0.48), (0.86, 0.16)]);
            painter.circle_filled(p(r, 0.88, 0.13), side * 0.075, color);
        }
        // Rays splaying out of one point.
        Icon::Fan => {
            for (x, y) in [(0.15f32, 0.2f32), (0.5, 0.12), (0.85, 0.2)] {
                path(painter, r, s, &[(0.5, 0.85), (x, y)]);
            }
        }
        // Two beams crossing, a head at the top of each.
        Icon::CrossAim => {
            path(painter, r, s, &[(0.18, 0.2), (0.84, 0.9)]);
            path(painter, r, s, &[(0.82, 0.2), (0.16, 0.9)]);
            painter.circle_filled(p(r, 0.18, 0.2), side * 0.11, color);
            painter.circle_filled(p(r, 0.82, 0.2), side * 0.11, color);
        }
        // A globe: a ring with its equator drawn in.
        Icon::Sphere => {
            painter.circle_stroke(r.center(), side * 0.33, s);
            let ring: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = i as f32 / 24.0 * std::f32::consts::TAU;
                    Pos2::new(
                        r.center().x + a.cos() * side * 0.33,
                        r.center().y + a.sin() * side * 0.13,
                    )
                })
                .collect();
            painter.add(Shape::line(ring, s));
            painter.circle_filled(r.center(), side * 0.06, color);
        }
        // A light that has come off its bar and is on its way down. The three
        // bar-and-lamp glyphs are told apart by what sits in the gap: an
        // arrow here, a swing on Reseat, a broken bar on Unmounted.
        Icon::Detach => {
            path(painter, r, Stroke::new(w * 1.5, color), &[(0.1, 0.12), (0.9, 0.12)]);
            path(painter, r, s, &[(0.5, 0.28), (0.5, 0.56)]);
            path(painter, r, s, &[(0.38, 0.44), (0.5, 0.56), (0.62, 0.44)]);
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.32, 0.66), p(r, 0.68, 0.94)),
                side * 0.05,
                s,
            );
        }
        // A hung light with a swing under it: put it back on its slot.
        Icon::Reseat => {
            path(painter, r, Stroke::new(w * 1.5, color), &[(0.1, 0.1), (0.9, 0.1)]);
            path(painter, r, s, &[(0.5, 0.1), (0.5, 0.28)]);
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.36, 0.28), p(r, 0.64, 0.54)),
                side * 0.05,
                s,
            );
            let a = arc(0.5, 0.56, 0.3, 200.0, 340.0, 10);
            path(painter, r, s, &a);
            let (px, py) = a[a.len() - 1];
            let (qx, qy) = a[a.len() - 2];
            let l = (px - qx).hypot(py - qy);
            let (ux, uy) = ((px - qx) / l, (py - qy) / l);
            path(
                painter,
                r,
                s,
                &[
                    (px - ux * 0.13 - uy * 0.1, py - uy * 0.13 + ux * 0.1),
                    (px, py),
                    (px - ux * 0.13 + uy * 0.1, py - uy * 0.13 - ux * 0.1),
                ],
            );
        }
        // A clipboard with two lines written on it.
        Icon::Paste => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.22, 0.22), p(r, 0.78, 0.88)),
                side * 0.06,
                s,
            );
            painter.rect_filled(
                Rect::from_min_max(p(r, 0.4, 0.12), p(r, 0.6, 0.28)),
                side * 0.03,
                color,
            );
            for y in [0.52f32, 0.68] {
                path(painter, r, s, &[(0.34, y), (0.66, y)]);
            }
        }
        // Two square boxes with an equals between them. Square corners, or a
        // box this small rounds itself into a circle.
        Icon::Match => {
            for x in [0.04f32, 0.68] {
                painter.rect_stroke(
                    Rect::from_min_max(p(r, x, 0.28), p(r, x + 0.28, 0.72)),
                    0.0,
                    s,
                );
            }
            for y in [0.38f32, 0.62] {
                path(painter, r, s, &[(0.4, y), (0.6, y)]);
            }
        }
        // Two cards, one behind the other, and a plus in the free corner —
        // out of the card, where it survives 16 px.
        Icon::Duplicate => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.32, 0.1), p(r, 0.74, 0.56)),
                side * 0.06,
                Stroke::new(w, color.gamma_multiply(0.6)),
            );
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.1, 0.28), p(r, 0.52, 0.74)),
                side * 0.06,
                s,
            );
            path(painter, r, s, &[(0.62, 0.79), (0.92, 0.79)]);
            path(painter, r, s, &[(0.77, 0.64), (0.77, 0.94)]);
        }
        // A marquee with a tick in it (all) or a cross (none).
        Icon::SelectAll | Icon::SelectNone => {
            for (x0, y0, x1, y1) in [
                (0.15f32, 0.15f32, 0.85f32, 0.15f32),
                (0.85, 0.15, 0.85, 0.85),
                (0.85, 0.85, 0.15, 0.85),
                (0.15, 0.85, 0.15, 0.15),
            ] {
                let (dx, dy) = (x1 - x0, y1 - y0);
                path(painter, r, s, &[(x0, y0), (x0 + dx * 0.33, y0 + dy * 0.33)]);
                path(painter, r, s, &[(x0 + dx * 0.67, y0 + dy * 0.67), (x1, y1)]);
            }
            let mark = Stroke::new(w * 1.25, color);
            if icon == Icon::SelectAll {
                path(painter, r, mark, &[(0.3, 0.52), (0.45, 0.68), (0.72, 0.33)]);
            } else {
                path(painter, r, mark, &[(0.35, 0.35), (0.65, 0.65)]);
                path(painter, r, mark, &[(0.65, 0.35), (0.35, 0.65)]);
            }
        }
        // A disc with one half filled: swap the two halves over.
        Icon::Invert => {
            painter.circle_stroke(r.center(), side * 0.33, s);
            let half: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = (90.0 + 180.0 * i as f32 / 12.0).to_radians();
                    Pos2::new(
                        r.center().x + a.cos() * side * 0.33,
                        r.center().y - a.sin() * side * 0.33,
                    )
                })
                .collect();
            painter.add(Shape::convex_polygon(half, color, Stroke::NONE));
        }
        // Two blocks cut from the same cloth, each ticked off.
        Icon::SameKind => {
            for x in [0.12f32, 0.56] {
                painter.rect_filled(
                    Rect::from_min_max(p(r, x, 0.22), p(r, x + 0.32, 0.54)),
                    side * 0.04,
                    color,
                );
                path(
                    painter,
                    r,
                    Stroke::new(w * 1.3, color),
                    &[(x + 0.04, 0.76), (x + 0.28, 0.76)],
                );
            }
        }
        // A light under a bar, struck through: it is hung on nothing. The
        // slash carries it at 16 px, where an × in the gap read as Detach.
        Icon::Unmounted => {
            path(painter, r, Stroke::new(w * 1.5, color), &[(0.12, 0.14), (0.88, 0.14)]);
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.34, 0.48), p(r, 0.66, 0.8)),
                side * 0.05,
                s,
            );
            path(painter, r, Stroke::new(w * 1.3, color), &[(0.12, 0.92), (0.88, 0.08)]);
        }
        // Arrows pushing out from the centre, or pulling back into it.
        Icon::Spread | Icon::Contract => {
            path(painter, r, s, &[(0.45, 0.5), (0.12, 0.5)]);
            path(painter, r, s, &[(0.55, 0.5), (0.88, 0.5)]);
            let (l, rr) = if icon == Icon::Spread {
                (0.12f32, 0.88f32)
            } else {
                (0.45, 0.55)
            };
            let d = if icon == Icon::Spread { 0.14 } else { -0.14 };
            path(painter, r, s, &[(l + d, 0.36), (l, 0.5), (l + d, 0.64)]);
            path(painter, r, s, &[(rr - d, 0.36), (rr, 0.5), (rr - d, 0.64)]);
        }
        // A measured drop between two bars.
        Icon::Height => {
            for y in [0.1f32, 0.9] {
                path(painter, r, s, &[(0.35, y), (0.65, y)]);
            }
            path(painter, r, s, &[(0.5, 0.2), (0.5, 0.8)]);
            path(painter, r, s, &[(0.38, 0.32), (0.5, 0.2), (0.62, 0.32)]);
            path(painter, r, s, &[(0.38, 0.68), (0.5, 0.8), (0.62, 0.68)]);
        }
        // Dots thrown about anyhow.
        Icon::Scatter => {
            for (x, y) in [(0.2, 0.3), (0.65, 0.2), (0.4, 0.6), (0.8, 0.55), (0.3, 0.82)] {
                painter.circle_filled(p(r, x, y), side * 0.08, color);
            }
        }
        // Dots alternating high and low along a faint zigzag.
        Icon::Stagger => {
            path(
                painter,
                r,
                Stroke::new(w, color.gamma_multiply(0.5)),
                &[(0.2, 0.62), (0.4, 0.38), (0.6, 0.62), (0.8, 0.38)],
            );
            for (x, y) in [(0.2, 0.62), (0.4, 0.38), (0.6, 0.62), (0.8, 0.38)] {
                painter.circle_filled(p(r, x, y), side * 0.08, color);
            }
        }
        // A dot dead centre of the stage box, ticks pointing at it.
        Icon::CentreStage => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.12, 0.2), p(r, 0.88, 0.8)),
                side * 0.05,
                s,
            );
            path(painter, r, s, &[(0.12, 0.5), (0.24, 0.5)]);
            path(painter, r, s, &[(0.88, 0.5), (0.76, 0.5)]);
            path(painter, r, s, &[(0.5, 0.2), (0.5, 0.3)]);
            path(painter, r, s, &[(0.5, 0.8), (0.5, 0.7)]);
            painter.circle_filled(r.center(), side * 0.09, color);
        }
                // (area E appends new draw arms above this line)
        // --- area F draw ---
        // A plug with a bolt off its shoulder: patch it now. The bolt is two
        // filled wedges — a stroked zigzag only reads as a squiggle.
        Icon::QuickPatch => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.06, 0.52), p(r, 0.46, 0.88)),
                side * 0.06,
                s,
            );
            path(painter, r, s, &[(0.17, 0.32), (0.17, 0.52)]);
            path(painter, r, s, &[(0.35, 0.32), (0.35, 0.52)]);
            for tri in [
                [(0.84f32, 0.06f32), (0.58, 0.52), (0.84, 0.52)],
                [(0.62, 0.94), (0.88, 0.48), (0.62, 0.48)],
            ] {
                painter.add(Shape::convex_polygon(
                    tri.iter().map(|&(x, y)| p(r, x, y)).collect(),
                    color,
                    Stroke::NONE,
                ));
            }
        }
        // A book seen open, the pages falling away either side of the spine.
        Icon::LibraryBook => {
            path(
                painter,
                r,
                s,
                &[(0.1, 0.2), (0.5, 0.3), (0.9, 0.2), (0.9, 0.8), (0.5, 0.9), (0.1, 0.8), (0.1, 0.2)],
            );
            path(painter, r, s, &[(0.5, 0.3), (0.5, 0.9)]);
        }
        // A par can head-on: the body, the lens and the yoke.
        Icon::ParCan => {
            path(
                painter,
                r,
                s,
                &[(0.25, 0.2), (0.75, 0.2), (0.85, 0.7), (0.15, 0.7), (0.25, 0.2)],
            );
            painter.rect_filled(
                Rect::from_min_max(p(r, 0.15, 0.7), p(r, 0.85, 0.82)),
                side * 0.03,
                color,
            );
            path(painter, r, s, &[(0.5, 0.82), (0.5, 0.95)]);
        }
        // A clock face with its hours marked — a face, not the Timer stopwatch.
        Icon::Recent => {
            painter.circle_stroke(r.center(), side * 0.38, s);
            for i in 0..4 {
                let a = i as f32 / 4.0 * std::f32::consts::TAU;
                let (sn, cs) = a.sin_cos();
                painter.line_segment(
                    [
                        Pos2::new(
                            r.center().x + cs * side * 0.28,
                            r.center().y + sn * side * 0.28,
                        ),
                        Pos2::new(
                            r.center().x + cs * side * 0.38,
                            r.center().y + sn * side * 0.38,
                        ),
                    ],
                    s,
                );
            }
            path(painter, r, s, &[(0.5, 0.5), (0.5, 0.28)]);
            path(painter, r, s, &[(0.5, 0.5), (0.72, 0.5)]);
        }
        // An arrow running up to the wall: take the next address going.
        Icon::NextFree => {
            path(painter, r, s, &[(0.1, 0.5), (0.65, 0.5)]);
            path(painter, r, s, &[(0.5, 0.35), (0.65, 0.5), (0.5, 0.65)]);
            path(painter, r, Stroke::new(w * 1.3, color), &[(0.8, 0.22), (0.8, 0.78)]);
        }
        // Two blocks with a hole between them and an arrow into the hole. The
        // spec's dashed hairline was one thing too many at 16 px.
        Icon::Gap => {
            for x in [0.04f32, 0.68] {
                painter.rect_filled(
                    Rect::from_min_max(p(r, x, 0.5), p(r, x + 0.28, 0.9)),
                    side * 0.03,
                    color,
                );
            }
            path(painter, r, s, &[(0.5, 0.08), (0.5, 0.46)]);
            path(painter, r, s, &[(0.37, 0.33), (0.5, 0.46), (0.63, 0.33)]);
        }
        // A J hook hanging off a bar. Curled the other way (the spec's 240°
        // sweep from twelve o'clock) it closed into a balloon at 16 px.
        Icon::Hook => {
            path(painter, r, Stroke::new(w * 1.4, color), &[(0.1, 0.12), (0.9, 0.12)]);
            path(painter, r, s, &[(0.52, 0.12), (0.52, 0.56)]);
            path(painter, r, s, &arc(0.32, 0.56, 0.2, 0.0, -180.0, 10));
            path(painter, r, s, &[(0.12, 0.56), (0.12, 0.38)]);
        }
        // A plug pulled out, struck through: take it off the patch.
        Icon::Unplug => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.16, 0.54), p(r, 0.58, 0.9)),
                side * 0.06,
                s,
            );
            path(painter, r, s, &[(0.28, 0.34), (0.28, 0.54)]);
            path(painter, r, s, &[(0.46, 0.34), (0.46, 0.54)]);
            let mark = Stroke::new(w * 1.2, color);
            path(painter, r, mark, &[(0.64, 0.08), (0.94, 0.38)]);
            path(painter, r, mark, &[(0.94, 0.08), (0.64, 0.38)]);
        }
        // A circled plus: one more of these.
        Icon::AddOne => {
            painter.circle_stroke(r.center(), side * 0.4, s);
            path(painter, r, s, &[(0.3, 0.5), (0.7, 0.5)]);
            path(painter, r, s, &[(0.5, 0.3), (0.5, 0.7)]);
        }
        // A row of blocks with the matching one picked out.
        Icon::SelectType => {
            for (x0, x1) in [(0.06f32, 0.32f32), (0.37, 0.63), (0.68, 0.94)] {
                let cell = Rect::from_min_max(p(r, x0, 0.28), p(r, x1, 0.72));
                if x0 > 0.3 && x1 < 0.7 {
                    painter.rect_filled(cell, 0.0, color);
                } else {
                    painter.rect_stroke(cell, 0.0, s);
                }
            }
        }
        // Lights strung down a line into the stage.
        Icon::ArrangeColumn => {
            path(painter, r, Stroke::new(w * 0.7, color), &[(0.5, 0.1), (0.5, 0.9)]);
            for y in [0.13f32, 0.38, 0.62, 0.87] {
                painter.circle_filled(p(r, 0.5, y), side * 0.095, color);
            }
        }
        // An arrow dropping onto the deck.
        Icon::Floor => {
            path(painter, r, Stroke::new(w * 1.5, color), &[(0.15, 0.84), (0.85, 0.84)]);
            path(painter, r, s, &[(0.5, 0.14), (0.5, 0.72)]);
            path(painter, r, s, &[(0.36, 0.58), (0.5, 0.72), (0.64, 0.58)]);
        }
                // (area F appends new draw arms above this line)
        // An arc over the top, ending in an arrowhead on the side it returns to.
        Icon::Undo | Icon::Redo => {
            let (arc, head) = undo_glyph(icon == Icon::Redo);
            path(painter, r, s, &arc);
            path(painter, r, s, &head);
        }
    }
}

/// A compact button showing `icon`, with `label` beside it when given.
///
/// Sized to the Button text style so it lines up with the small buttons in
/// a header row. It paints the standard raised control, so it takes its
/// depth from the theme like every other button.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, label: Option<&str>) -> egui::Response {
    let galley = label.map(|l| {
        egui::WidgetText::from(l).into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Button,
        )
    });
    let pad = ui.spacing().button_padding;
    let icon_size = ui.text_style_height(&egui::TextStyle::Button) * 1.05;
    let gap = 5.0;
    // Square when it is only an icon.
    let px = if galley.is_some() { pad.x } else { pad.y + 2.0 };
    let text_w = galley.as_ref().map_or(0.0, |g| g.size().x + gap);
    let h = galley.as_ref().map_or(icon_size, |g| g.size().y.max(icon_size)) + pad.y * 2.0;
    let desired = Vec2::new(px * 2.0 + icon_size + text_w, h);
    let (rect, response) = ui.allocate_at_least(desired, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.style().interact(&response);
        ui.painter().rect(rect, v.rounding, v.weak_bg_fill, v.bg_stroke);
        let icon_rect = Rect::from_center_size(
            Pos2::new(rect.left() + px + icon_size * 0.5, rect.center().y),
            Vec2::splat(icon_size),
        );
        draw(ui.painter(), icon_rect, icon, v.fg_stroke.color);
        if let Some(g) = galley {
            ui.painter().galley(
                Pos2::new(icon_rect.right() + gap, rect.center().y - g.size().y * 0.5),
                g,
                v.fg_stroke.color,
            );
        }
    }
    response
}

/// A toolbar tab: icon and label, sitting flat until it is showing, when it
/// lifts on an accent-tinted body with a glowing underline.
pub fn tab(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> egui::Response {
    let galley = egui::WidgetText::from(label).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let pad = ui.spacing().button_padding;
    let icon_size = ui.text_style_height(&egui::TextStyle::Button) * 1.1;
    let gap = 6.0;
    let desired = Vec2::new(
        pad.x * 2.0 + icon_size + gap + galley.size().x,
        galley.size().y.max(icon_size) + pad.y * 2.0,
    );
    let (rect, response) = ui.allocate_at_least(desired, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.style().interact_selectable(&response, selected);
        let rounding = v.rounding;
        let painter = ui.painter();
        if selected {
            // Teal-tinted body, lit from above, on a soft shadow.
            painter.rect_filled(
                rect.translate(Vec2::new(0.0, 1.0)),
                rounding,
                Color32::from_black_alpha(110),
            );
            painter.add(Shape::mesh(theme::vgradient_mesh(
                rect,
                rounding,
                theme::ACCENT.lerp_to_gamma(theme::RAISED, 0.45),
                theme::ACCENT.lerp_to_gamma(theme::SURFACE, 0.72),
            )));
            painter.rect_stroke(rect, rounding, Stroke::new(1.0, theme::ACCENT_MUTED));
        } else if response.hovered() || response.has_focus() {
            painter.rect_filled(
                rect.translate(Vec2::new(0.0, 1.0)),
                rounding,
                Color32::from_black_alpha(90),
            );
            painter.add(Shape::mesh(theme::vgradient_mesh(
                rect,
                rounding,
                theme::HOVER.lerp_to_gamma(Color32::WHITE, 0.06),
                theme::HOVER.lerp_to_gamma(Color32::BLACK, 0.10),
            )));
            painter.rect_stroke(rect, rounding, Stroke::new(1.0, theme::RIM));
        }
        let (icon_color, text_color) = if selected {
            (theme::ACCENT_SOFT, theme::TEXT)
        } else if response.hovered() {
            (theme::TEXT, theme::TEXT)
        } else {
            (theme::TEXT_DIM.lerp_to_gamma(theme::TEXT, 0.35), theme::TEXT_DIM.lerp_to_gamma(theme::TEXT, 0.55))
        };
        let icon_rect = Rect::from_center_size(
            Pos2::new(rect.left() + pad.x + icon_size * 0.5, rect.center().y),
            Vec2::splat(icon_size),
        );
        draw(painter, icon_rect, icon, icon_color);
        painter.galley(
            Pos2::new(
                icon_rect.right() + gap,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            text_color,
        );
        if selected {
            // Underline with a faint glow above it.
            let glow = Rect::from_min_max(
                Pos2::new(rect.left() + 4.0, rect.bottom() - 5.0),
                Pos2::new(rect.right() - 4.0, rect.bottom() - 1.0),
            );
            painter.add(Shape::mesh(theme::vgradient_mesh(
                glow,
                Rounding::ZERO,
                Color32::TRANSPARENT,
                theme::ACCENT_SOFT.gamma_multiply(0.35),
            )));
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(rect.left() + 4.0, rect.bottom() - 2.0),
                    Pos2::new(rect.right() - 4.0, rect.bottom() - 0.5),
                ),
                1.0,
                theme::ACCENT_SOFT,
            );
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::headless::{render_frames, save};
    use eframe::egui::{Align2, FontId};

    /// Redo is Undo mirrored, once. The arm used to mirror the arc as it was
    /// sampled and then mirror the arrowhead's wings again off the already
    /// mirrored apex, throwing them to the far side of the glyph: the head
    /// stopped being a head and the icon drew a zigzag right across itself.
    #[test]
    fn the_redo_arrowhead_is_the_undo_one_mirrored_once() {
        let (undo_arc, undo_head) = undo_glyph(false);
        let (redo_arc, redo_head) = undo_glyph(true);
        // The head sits on the end of the arc it is drawn for.
        for (arc, head) in [(&undo_arc, &undo_head), (&redo_arc, &redo_head)] {
            assert_eq!(head[1], arc[0], "the apex left the arc");
            // Both wings stay within a chevron's reach of the apex, i.e. the
            // head is a mark on the end of the arc and not a stroke across
            // the whole 0..1 icon.
            for (wx, wy) in head {
                let d = (wx - head[1].0).hypot(wy - head[1].1);
                assert!(d < 0.25, "a wing landed {d} away at {wx},{wy}");
            }
        }
        // And the whole thing is the mirror image of the other.
        for (u, r) in undo_arc.iter().zip(&redo_arc) {
            assert!((u.0 - (1.0 - r.0)).abs() < 1e-5 && (u.1 - r.1).abs() < 1e-5);
        }
        for (u, r) in undo_head.iter().zip(&redo_head) {
            assert!((u.0 - (1.0 - r.0)).abs() < 1e-5 && (u.1 - r.1).abs() < 1e-5);
        }
    }

    /// Every Inspector glyph at 16 px and again at 28 px on the console's own
    /// surface, written to `target/icon_sheet.png` so a redraw can be judged
    /// by eye at the sizes the panels actually use.
    #[test]
    fn icon_sheet_renders_headless() {
        const SHEET: &[(Icon, &str)] = &[
            (Icon::Presets, "Presets"),
            (Icon::Selection, "Selection"),
            (Icon::Stage, "Stage"),
            (Icon::Build, "Build"),
            (Icon::Camera, "Camera"),
            (Icon::Tower, "Tower"),
            (Icon::TrussStraight, "TrussStraight"),
            (Icon::TrussRadius, "TrussRadius"),
            (Icon::Light, "Light"),
            (Icon::Eye, "Eye"),
            (Icon::EyeOff, "EyeOff"),
            (Icon::Jump, "Jump"),
            (Icon::Close, "Close"),
            (Icon::Check, "Check"),
            (Icon::Trash, "Trash"),
            (Icon::Folder, "Folder"),
            (Icon::Pin, "Pin"),
            (Icon::Search, "Search"),
            (Icon::Copy, "Copy"),
            (Icon::Grid, "Grid"),
            (Icon::Beam, "Beam"),
            (Icon::Pool, "Pool"),
            (Icon::Label, "Label"),
            (Icon::Orbit, "Orbit"),
            (Icon::Fly, "Fly"),
            (Icon::Help, "Help"),
            (Icon::Blackout, "Blackout"),
            (Icon::Bank, "Bank"),
            (Icon::Drag, "Drag"),
            (Icon::Reset, "Reset"),
            (Icon::CamIso, "CamIso"),
            (Icon::CamFront, "CamFront"),
            (Icon::CamBack, "CamBack"),
            (Icon::CamLeft, "CamLeft"),
            (Icon::CamRight, "CamRight"),
            (Icon::CamTop, "CamTop"),
            (Icon::CamFoh, "CamFoh"),
            (Icon::FrameSel, "FrameSel"),
            (Icon::FrameAll, "FrameAll"),
            (Icon::Target, "Target"),
            (Icon::Bookmark, "Bookmark"),
            (Icon::SlotRing, "SlotRing"),
            (Icon::Gizmo, "Gizmo"),
            (Icon::Tick, "Tick"),
            (Icon::Overlay, "Overlay"),
            (Icon::Ruler, "Ruler"),
            (Icon::Tour, "Tour"),
            (Icon::ZoomIn, "ZoomIn"),
            (Icon::ZoomOut, "ZoomOut"),
            (Icon::Goalpost, "Goalpost"),
            (Icon::BoxRig, "BoxRig"),
            (Icon::Ring, "Ring"),
            (Icon::TowerPair, "TowerPair"),
            (Icon::Arch, "Arch"),
            (Icon::Save, "Save"),
            (Icon::Refresh, "Refresh"),
            (Icon::Store, "Store"),
            (Icon::List, "List"),
            (Icon::Fade, "Fade"),
            (Icon::Sort, "Sort"),
            (Icon::RotateCw, "RotateCw"),
            (Icon::RotateCcw, "RotateCcw"),
            (Icon::AlignMin, "AlignMin"),
            (Icon::AlignMid, "AlignMid"),
            (Icon::AlignMax, "AlignMax"),
            (Icon::Distribute, "Distribute"),
            (Icon::MirrorX, "MirrorX"),
            (Icon::MirrorZ, "MirrorZ"),
            (Icon::ArrangeLine, "ArrangeLine"),
            (Icon::ArrangeArc, "ArrangeArc"),
            (Icon::ArrangeCircle, "ArrangeCircle"),
            (Icon::ArrangeGrid, "ArrangeGrid"),
            (Icon::AimDown, "AimDown"),
            (Icon::AimUp, "AimUp"),
            (Icon::AimLevel, "AimLevel"),
            (Icon::AimMover, "AimMover"),
            (Icon::Fan, "Fan"),
            (Icon::CrossAim, "CrossAim"),
            (Icon::Sphere, "Sphere"),
            (Icon::Detach, "Detach"),
            (Icon::Reseat, "Reseat"),
            (Icon::Paste, "Paste"),
            (Icon::Match, "Match"),
            (Icon::Duplicate, "Duplicate"),
            (Icon::SelectAll, "SelectAll"),
            (Icon::SelectNone, "SelectNone"),
            (Icon::Invert, "Invert"),
            (Icon::SameKind, "SameKind"),
            (Icon::Unmounted, "Unmounted"),
            (Icon::Spread, "Spread"),
            (Icon::Contract, "Contract"),
            (Icon::Height, "Height"),
            (Icon::Scatter, "Scatter"),
            (Icon::Stagger, "Stagger"),
            (Icon::CentreStage, "CentreStage"),
            (Icon::QuickPatch, "QuickPatch"),
            (Icon::LibraryBook, "LibraryBook"),
            (Icon::ParCan, "ParCan"),
            (Icon::Recent, "Recent"),
            (Icon::NextFree, "NextFree"),
            (Icon::Gap, "Gap"),
            (Icon::Hook, "Hook"),
            (Icon::Unplug, "Unplug"),
            (Icon::AddOne, "AddOne"),
            (Icon::SelectType, "SelectType"),
            (Icon::ArrangeColumn, "ArrangeColumn"),
            (Icon::Floor, "Floor"),
            // The neighbours a new glyph must not be mistaken for.
            (Icon::Timer, "Timer (old)"),
            (Icon::Config, "Config (old)"),
            (Icon::Log, "Log (old)"),
            (Icon::Patch, "Patch (old)"),
            (Icon::Gobo, "Gobo (old)"),
            (Icon::Undo, "Undo (old)"),
            (Icon::Redo, "Redo"),
        ];
        const COLS: usize = 8;
        const CW: f32 = 150.0;
        const CH: f32 = 76.0;

        let rows = SHEET.len().div_ceil(COLS);
        let size = [
            (COLS as f32 * CW) as u32 + 24,
            (rows as f32 * CH) as u32 + 24,
        ];
        let Some(pixels) = render_frames(2, size, |ctx, frame| {
            // The console's fonts bind the frame after the theme goes in.
            if frame == 0 {
                crate::ui::install_theme(ctx);
                return;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(theme::SURFACE))
                .show(ctx, |ui| {
                    let painter = ui.painter().clone();
                    let origin = ui.max_rect().left_top() + Vec2::splat(12.0);
                    for (i, (icon, name)) in SHEET.iter().enumerate() {
                        let cell = Rect::from_min_size(
                            origin + Vec2::new((i % COLS) as f32 * CW, (i / COLS) as f32 * CH),
                            Vec2::new(CW, CH),
                        );
                        painter.rect_stroke(cell.shrink(2.0), 4.0, Stroke::new(1.0, theme::EDGE));
                        let art = cell.center() - Vec2::new(0.0, 12.0);
                        draw(
                            &painter,
                            Rect::from_center_size(art - Vec2::new(26.0, 0.0), Vec2::splat(16.0)),
                            *icon,
                            theme::TEXT,
                        );
                        draw(
                            &painter,
                            Rect::from_center_size(art + Vec2::new(20.0, 0.0), Vec2::splat(28.0)),
                            *icon,
                            theme::TEXT,
                        );
                        painter.text(
                            cell.center_bottom() - Vec2::new(0.0, 6.0),
                            Align2::CENTER_BOTTOM,
                            name,
                            FontId::proportional(11.0),
                            theme::TEXT_DIM,
                        );
                    }
                });
        }) else {
            eprintln!("no GPU adapter — skipping");
            return;
        };
        save(&pixels, size, "icon_sheet");
        let lit = pixels
            .chunks(4)
            .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120)
            .count();
        assert!(lit > 1000, "the icon sheet came out black");
    }
}
