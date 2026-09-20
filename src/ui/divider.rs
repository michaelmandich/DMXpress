//! The seams between panes, and the styles they can be drawn in.
//!
//! egui gives every panel edge the same flat grey line. These replace it.
//! A seam is not a line — it is where two surfaces meet, and what you see
//! there is light: the near surface's edge dropping away into shadow, the
//! far one's edge turning back up toward the lamp. Every style here is
//! built on that one asymmetry, and they differ only in how deep the gap
//! is and what lives in it.
//!
//! The lamp is the one the rest of the console is lit by — above and a
//! little to the left — so "near" means the pane above or to the left of
//! the crack and "far" the one below or to the right, whichever way the
//! seam runs.
//!
//! The vertical seams are the side panels' resize handles and the one
//! under the stage drags the channel controls, so every style has a
//! hovered and a dragged state. Teal appears in those two states and
//! nowhere else, except in [`DividerStyle::LightSeam`], which is a lamp.
//!
//! Seams are collected as the panels lay themselves out and painted in one
//! pass at the end of the frame (see `App::draw_ui`), on top of the panels
//! rather than between them: egui draws its own line *under* the panel
//! contents and there is no way past it but to cover it. A left-hand panel
//! puts that line one pixel before the edge and a right-hand or bottom one
//! puts it on the edge, so every style below is opaque across both.

use eframe::egui::{self, Color32, Mesh, Pos2, Rect, Rounding, Shape, Stroke};
use serde::{Deserialize, Serialize};

use super::theme;

/// The bottom of the crack: below anything either pane paints, so the gap
/// reads as a gap and not as a line someone drew on a flat surface.
const CRACK: Color32 = Color32::from_rgb(0x05, 0x06, 0x08);

/// The near plate's wall falling away into the crack — one step under
/// [`theme::SURFACE`], not yet the dark.
const FALL: Color32 = Color32::from_rgb(0x14, 0x17, 0x1B);

/// The far plate's lit lip. Brighter than [`theme::RIM`], because it is
/// the single row carrying the whole "this is carved, not painted" read.
const LIP: Color32 = Color32::from_rgb(0x55, 0x5E, 0x69);

/// How thick the caller makes a seam rect, centred on the panel edge: six
/// pixels of ink at most, plus the pixel the snap can move it by.
pub(crate) const THICKNESS: f32 = 8.0;

/// One seam found during the frame: where it runs, which way, and whether
/// the pointer is on it or pulling it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Seam {
    /// A thin rect [`THICKNESS`] across, centred on the seam line and as
    /// long as the edge it covers.
    pub rect: Rect,
    /// True for a left/right panel edge.
    pub vertical: bool,
    pub hovered: bool,
    pub active: bool,
}

/// How the seams between panes are drawn. They all say the same thing —
/// two surfaces, a gap, a handle you can pull — in different materials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) enum DividerStyle {
    /// One pixel of shadow, one of light.
    Hairline,
    /// A milled channel between two plates.
    #[default]
    Groove,
    /// A raised bar with a knurled grip at its middle.
    Rail,
    /// One pane floating over the other, casting into the gap.
    ShadowGap,
    /// A thread of accent leaking out of a black crack.
    LightSeam,
    /// A rack panel: a shallow channel studded with rivets.
    Riveted,
}

impl DividerStyle {
    pub const ALL: [DividerStyle; 6] = [
        DividerStyle::Hairline,
        DividerStyle::Groove,
        DividerStyle::Rail,
        DividerStyle::ShadowGap,
        DividerStyle::LightSeam,
        DividerStyle::Riveted,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DividerStyle::Hairline => "Hairline",
            DividerStyle::Groove => "Groove",
            DividerStyle::Rail => "Rail",
            DividerStyle::ShadowGap => "Shadow gap",
            DividerStyle::LightSeam => "Light seam",
            DividerStyle::Riveted => "Riveted",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            DividerStyle::Hairline => {
                "Two pixels: the shadow under one pane, the lit edge of the next"
            }
            DividerStyle::Groove => {
                "A channel milled between two plates, lit lip on the far side"
            }
            DividerStyle::Rail => "A raised bar with a knurled grip where your hand goes",
            DividerStyle::ShadowGap => {
                "One pane floating over the other, its shadow falling across it"
            }
            DividerStyle::LightSeam => {
                "A black crack with a thread of accent leaking out of it"
            }
            DividerStyle::Riveted => "A rack panel: a shallow channel studded with rivets",
        }
    }
}

/// The seam along the top edge of a bottom bar. Neither bar is resizable,
/// so it is a joint rather than a handle.
pub(crate) fn top_edge_seam(panel: Rect) -> Seam {
    let half = THICKNESS * 0.5;
    Seam {
        rect: Rect::from_x_y_ranges(
            panel.x_range(),
            (panel.top() - half)..=(panel.top() + half),
        ),
        vertical: false,
        hovered: false,
        active: false,
    }
}

/// Paint one seam. `seam` is a thin rect centred on the seam line and
/// [`THICKNESS`] across; `vertical` is true for a left/right panel edge.
/// Nothing is drawn outside `seam`.
pub(crate) fn paint(
    p: &egui::Painter,
    style: DividerStyle,
    seam: Rect,
    vertical: bool,
    hovered: bool,
    active: bool,
) {
    if seam.width() < 1.0 || seam.height() < 1.0 {
        return;
    }
    let mid = if vertical { seam.center().x } else { seam.center().y };
    let g = Geom {
        // Snap the crack to a pixel centre. Every rule below is snapped
        // again off it, so none of them lands half on a pixel and lets
        // egui's grey line show through the blur.
        c: p.round_to_pixel_center(mid),
        rect: seam,
        vertical,
    };
    match style {
        DividerStyle::Hairline => hairline(p, &g, hovered, active),
        DividerStyle::Groove => groove(p, &g, hovered, active),
        DividerStyle::Rail => rail(p, &g, hovered, active),
        DividerStyle::ShadowGap => shadow_gap(p, &g, hovered, active),
        DividerStyle::LightSeam => light_seam(p, &g, hovered, active),
        DividerStyle::Riveted => riveted(p, &g, hovered, active),
    }
}

// ---- the seam, reduced to what a style needs ----

/// A seam with its crack already snapped: `c` is the crack's x for a
/// vertical seam and its y for a horizontal one, and every offset below is
/// in pixels out of it — negative toward the near pane (above, or to the
/// left), positive toward the far one.
struct Geom {
    c: f32,
    rect: Rect,
    vertical: bool,
}

impl Geom {
    /// The strip from `a` to `b` pixels out of the crack, the whole length
    /// of the seam.
    fn band(&self, a: f32, b: f32) -> Rect {
        if self.vertical {
            Rect::from_min_max(
                Pos2::new(self.c + a, self.rect.top()),
                Pos2::new(self.c + b, self.rect.bottom()),
            )
        } else {
            Rect::from_min_max(
                Pos2::new(self.rect.left(), self.c + a),
                Pos2::new(self.rect.right(), self.c + b),
            )
        }
    }

    /// An opaque strip — what covers egui's own line.
    fn fill(&self, p: &egui::Painter, a: f32, b: f32, color: Color32) {
        p.rect_filled(self.band(a, b), Rounding::ZERO, color);
    }

    /// One pixel of colour, `off` out of the crack and snapped to its own
    /// pixel centre so it stays a hard edge on a scaled display too.
    fn rule(&self, p: &egui::Painter, off: f32, color: Color32) {
        let stroke = Stroke::new(1.0, color);
        let at = p.round_to_pixel_center(self.c + off);
        if self.vertical {
            p.vline(at, self.rect.y_range(), stroke);
        } else {
            p.hline(self.rect.x_range(), at, stroke);
        }
    }

    /// A strip whose colour runs *across* the seam, `near` on the upper or
    /// left side and `far` on the other. Four vertices, axis-aligned, so
    /// the unfeathered mesh has no diagonal to stair-step.
    fn wash(&self, p: &egui::Painter, a: f32, b: f32, near: Color32, far: Color32) {
        let r = self.band(a, b);
        let (n0, n1, f1, f0) = if self.vertical {
            (r.left_top(), r.left_bottom(), r.right_bottom(), r.right_top())
        } else {
            (r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom())
        };
        let mut mesh = Mesh::default();
        mesh.colored_vertex(n0, near);
        mesh.colored_vertex(n1, near);
        mesh.colored_vertex(f1, far);
        mesh.colored_vertex(f0, far);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        p.add(Shape::mesh(mesh));
    }

    /// How long the seam is.
    fn len(&self) -> f32 {
        if self.vertical {
            self.rect.height()
        } else {
            self.rect.width()
        }
    }

    /// The point `along` pixels from the seam's start and `off` out of the
    /// crack.
    fn at(&self, along: f32, off: f32) -> Pos2 {
        if self.vertical {
            Pos2::new(self.c + off, self.rect.top() + along)
        } else {
            Pos2::new(self.rect.left() + along, self.c + off)
        }
    }

    /// A one-pixel tick across the seam at `along`, from `a` to `b`, on a
    /// pixel centre so a run of them stays crisp.
    fn tick(&self, p: &egui::Painter, along: f32, a: f32, b: f32, color: Color32) {
        let (mut from, mut to) = (self.at(along, a), self.at(along, b));
        if self.vertical {
            from.y = p.round_to_pixel_center(from.y);
            to.y = from.y;
        } else {
            from.x = p.round_to_pixel_center(from.x);
            to.x = from.x;
        }
        p.line_segment([from, to], Stroke::new(1.0, color));
    }

    /// Distances along the seam at a fixed `pitch`, the run centred so
    /// both ends stay clear. The pitch is the same on every seam in the
    /// app, which is what makes a row of anything read as machined rather
    /// than as ornament stretched to fit.
    fn pitched(&self, pitch: f32, max: usize) -> impl Iterator<Item = f32> {
        let len = self.len();
        let n = ((len / pitch).floor() as usize).min(max);
        let start = (len - n.saturating_sub(1) as f32 * pitch) * 0.5;
        (0..n).map(move |i| start + i as f32 * pitch)
    }
}

/// What a seam's lit edge turns: its own slate at rest, teal under the
/// pointer, the soft accent while it is being dragged.
fn lit(rest: Color32, hovered: bool, active: bool) -> Color32 {
    if active {
        theme::ACCENT_SOFT
    } else if hovered {
        theme::ACCENT_MUTED
    } else {
        rest
    }
}

/// How hard a style leans on the accent: nothing at rest, half under the
/// pointer, all of it while dragged.
fn heat(hovered: bool, active: bool) -> f32 {
    if active {
        1.0
    } else if hovered {
        0.5
    } else {
        0.0
    }
}

// ---- styles ----

/// One pixel of shadow, one of light — egui's flat grey line replaced by
/// the edge the rest of the console draws, and nothing more.
fn hairline(p: &egui::Painter, g: &Geom, hovered: bool, active: bool) {
    g.fill(p, -1.5, -0.5, CRACK);
    g.fill(p, -0.5, 0.5, lit(theme::EDGE, hovered, active));
}

/// Two milled plates with a channel between them: the near plate takes a
/// chamfer of light, falls away into the dark, and the far plate turns a
/// lit lip back up at the lamp.
fn groove(p: &egui::Painter, g: &Geom, hovered: bool, active: bool) {
    // The floor first — everything else is laid on top of it, so the
    // channel stays opaque whatever happens above.
    g.fill(p, -2.5, 1.5, CRACK);
    // Under the pointer the floor picks up the lamp. Flat, not a gradient:
    // three pixels of gradient is three pixels of one colour.
    let k = heat(hovered, active);
    if k > 0.0 {
        g.fill(p, -1.5, 1.5, theme::ACCENT.gamma_multiply(0.16 + 0.22 * k));
    }
    // The near plate catches a little light on its chamfer before it drops.
    g.rule(p, -3.0, Color32::from_white_alpha(22));
    g.rule(p, -2.0, FALL);
    // The lip last, so nothing above tints it.
    g.rule(p, 1.0, lit(LIP, hovered, active));
}

/// A raised extrusion capping the joint, knurled across its middle where
/// your hand goes. Lit on the near side and shadowed on the far one — the
/// groove's order reversed, which is the whole of why it stands proud.
fn rail(p: &egui::Painter, g: &Geom, hovered: bool, active: bool) {
    let body = if hovered || active { theme::HOVER } else { theme::RAISED };
    g.wash(p, -2.5, 2.5, theme::lighten(body, 0.18), theme::darken(body, 0.45));
    g.rule(p, -2.0, Color32::from_white_alpha(30));
    // The shadow the bar throws on the pane behind it: hard, then soft.
    g.rule(p, 2.0, CRACK);
    g.rule(p, 3.0, Color32::from_black_alpha(90));
    // The grip: ticks across the crown, each a lit edge with its own
    // shadow beside it, the way a machined knurl takes the light.
    let len = g.len();
    let run = (len * 0.3).min(36.0);
    if run < 12.0 {
        return;
    }
    let n = (run / 3.0) as usize;
    let start = (len - n as f32 * 3.0) * 0.5;
    let hi = lit(Color32::from_white_alpha(80), hovered, active);
    let lo = Color32::from_black_alpha(110);
    for i in 0..n {
        let along = start + i as f32 * 3.0 + 0.5;
        g.tick(p, along, -1.5, 1.5, hi);
        g.tick(p, along + 1.0, -1.5, 1.5, lo);
    }
}

/// One pane floating over the other: a black gap, a lit chamfer on the
/// floating edge, and the shadow that edge throws across the pane beneath.
fn shadow_gap(p: &egui::Painter, g: &Geom, hovered: bool, active: bool) {
    g.fill(p, -1.5, 0.5, CRACK);
    // The last of the lamp on the floating plate's edge.
    g.rule(p, -2.0, Color32::from_white_alpha(22));
    // Its shadow across the plate below: hard at the crack and gone before
    // that plate's own surface starts. This one mesh does all the work.
    g.wash(p, 0.5, 3.5, Color32::from_black_alpha(185), Color32::TRANSPARENT);
    // Light finding its way under the overhang.
    let k = heat(hovered, active);
    if k > 0.0 {
        g.rule(p, 0.0, theme::ACCENT_SOFT.gamma_multiply(0.45 + 0.45 * k));
    }
}

/// A crack with a lamp behind it: black at the edges, a thread of accent
/// down the middle, and the bloom of it spreading a couple of pixels onto
/// both pane faces. The bloom is the cue — without it the teal is a line
/// someone painted; with it the crack is lit from inside.
fn light_seam(p: &egui::Painter, g: &Geom, hovered: bool, active: bool) {
    let k = 0.30 + 0.70 * heat(hovered, active);
    g.fill(p, -1.5, 1.5, CRACK);
    let glow = theme::ACCENT.gamma_multiply(k * 0.34);
    g.wash(p, -3.0, -1.5, Color32::TRANSPARENT, glow);
    g.wash(p, 1.5, 3.0, glow, Color32::TRANSPARENT);
    // Inside the crack: a halo either side of the filament, then the
    // filament, so the core is the brightest pixel anywhere near it.
    let halo = theme::ACCENT.gamma_multiply(0.35 + 0.55 * k);
    g.rule(p, -1.0, halo);
    g.rule(p, 1.0, halo);
    g.rule(p, 0.0, theme::ACCENT_SOFT.gamma_multiply(0.5 + 0.5 * k));
}

/// A rack panel: a shallow channel studded with rivets at a fixed pitch,
/// each one a little dome, lit on its upper left with its own shadow
/// crescent on the lower right.
fn riveted(p: &egui::Painter, g: &Geom, hovered: bool, active: bool) {
    g.fill(p, -1.5, 1.5, CRACK);
    g.rule(p, -2.0, FALL);
    g.rule(p, 2.0, lit(theme::RIM, hovered, active));
    if g.len() < 24.0 {
        return;
    }
    let shade = theme::darken(theme::RIM, 0.62);
    let crown = lit(theme::lighten(theme::RIM, 0.22), hovered, active);
    for along in g.pitched(18.0, 160) {
        let c = g.at(along, 0.0);
        // Two discs offset along the light: the shadowed one down and to
        // the right, the lit one up and to the left. A dome, in two shapes.
        p.circle_filled(c + egui::vec2(0.5, 0.5), 1.9, shade);
        p.circle_filled(c - egui::vec2(0.2, 0.2), 1.6, crown);
    }
}
#[cfg(test)]
mod tests {
    use crate::net::{Frame, DMX_SLOTS};
    use crate::stage::headless::{render_frames, save};

    use super::DividerStyle;

    /// The whole console in every divider style, written to
    /// `target/divider_<name>.png` so a restyle can be checked by eye. All
    /// four seams are in shot: both side panels, the stage/controls joint
    /// and the executor bar's top edge.
    #[test]
    fn divider_styles_render_headless() {
        let mut app = crate::app::App::new();
        let mut frame = [0u8; DMX_SLOTS];
        for (i, v) in frame.iter_mut().enumerate() {
            *v = ((i * 53) % 256) as u8;
        }
        *app.net.dmx.lock() = Frame(frame);
        app.show_log = false;
        // Nothing floating over the seams: the shot is of the joints.
        app.show_osc = false;
        app.show_decks = true;
        let size = [1100, 800];
        for style in DividerStyle::ALL {
            app.settings.divider = style;
            let Some(pixels) = render_frames(5, size, |ctx, frame| {
                if frame == 0 {
                    super::super::theme::install(ctx);
                } else {
                    app.draw_ui(ctx);
                }
            }) else {
                eprintln!("no GPU adapter — skipping");
                return;
            };
            save(
                &pixels,
                size,
                &format!("divider_{}", style.label().to_lowercase().replace(' ', "_")),
            );
        }
    }
}
