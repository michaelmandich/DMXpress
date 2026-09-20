//! The Fixtures panel's raid-grid tiles, drawn in each [`RaidLook`].
//!
//! Every look shows the same three things — the fixture's live colour, its
//! name and its DMX address — and answers the same selection outline. They
//! differ in the material the tile is made of: a moulded colour cap, an amp
//! faceplate with a pilot jewel screwed into it, a chamfered sci-fi slab,
//! a slab of cut glass, a lens behind a machined chrome bezel, a backlit
//! silicone pad, an LED batten, a colour gel in its frame, a strip of
//! gaffer tape, a neon tube on a board. All the depth is faked with
//! per-vertex gradient meshes (see the helpers in [`theme`]), so the grid
//! stays as cheap as the flat one it replaced: no textures, no shaders.

use std::f32::consts::PI;
use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, FontId, Galley, Pos2, Rect, Rounding, Shape, Stroke};

use super::theme::{self, convex_mesh, darken, lighten, radial_mesh, ring_mesh, vgradient_mesh};
use crate::stage::RaidLook;

/// What one tile has to say.
pub(crate) struct Tile<'a> {
    pub name: &'a str,
    /// Already formatted, e.g. `@17`.
    pub addr: &'a str,
    /// Live colour, normalised to full hue (dark grey when off).
    pub color: Color32,
    /// Live output level, 0..1.
    pub level: f32,
    pub selected: bool,
}

const SELECT: Color32 = Color32::from_rgb(120, 180, 255);

/// Tile size for a look at panel zoom `z`. The ones that frame their
/// subject — the jewel on its faceplate, the gel in its frame, the tape on
/// the fixture's face — are bigger, because the frame is the look.
pub(crate) fn tile_size(look: RaidLook, z: f32) -> egui::Vec2 {
    let (w, h) = match look {
        RaidLook::Lit => (58.0, 38.0),
        RaidLook::Jewel => (60.0, 54.0),
        RaidLook::Future => (62.0, 42.0),
        RaidLook::Glass => (62.0, 44.0),
        RaidLook::Chrome => (62.0, 44.0),
        RaidLook::Pillow => (64.0, 46.0),
        RaidLook::Pixel => (62.0, 46.0),
        RaidLook::Gel => (58.0, 48.0),
        RaidLook::Tape => (66.0, 46.0),
        RaidLook::Neon => (64.0, 48.0),
    };
    egui::vec2(w * z, h * z)
}

/// Paint one tile into `rect`.
pub(crate) fn paint(painter: &egui::Painter, look: RaidLook, rect: Rect, z: f32, t: &Tile) {
    // Glows and shadows stop at the tile's edge so neighbours never overlap.
    let p = painter.with_clip_rect(rect.expand(1.0));
    match look {
        RaidLook::Lit => lit(&p, rect, z, t),
        RaidLook::Jewel => jewel(&p, rect, z, t),
        RaidLook::Future => future(&p, rect, z, t),
        RaidLook::Glass => glass(&p, rect, z, t),
        RaidLook::Chrome => chrome(&p, rect, z, t),
        RaidLook::Pillow => pillow(&p, rect, z, t),
        RaidLook::Pixel => pixel(&p, rect, z, t),
        RaidLook::Gel => gel(&p, rect, z, t),
        RaidLook::Tape => tape(&p, rect, z, t),
        RaidLook::Neon => neon(&p, rect, z, t),
    }
}

// ---- shared bits ----

/// Perceptual level: the eye sees a dim lamp as brighter than its output.
fn vis(level: f32) -> f32 {
    level.clamp(0.0, 1.0).sqrt()
}

/// Text colours that read on a fill of `bg`: (primary, dimmed).
fn ink(bg: Color32) -> (Color32, Color32) {
    let lum = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    let txt = if lum > 140.0 {
        Color32::BLACK
    } else {
        Color32::from_gray(235)
    };
    (txt, txt.gamma_multiply(0.75))
}

fn galley(p: &egui::Painter, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping {
        max_width: max_w.max(8.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    p.layout_job(job)
}

/// Single-line text anchored at `pos`, truncated with an ellipsis to
/// `max_w`.
fn text(
    p: &egui::Painter,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    font: FontId,
    color: Color32,
    max_w: f32,
) {
    let g = galley(p, text, font, color, max_w);
    let r = anchor.anchor_size(pos, g.size());
    p.galley(r.min, g, color);
}

/// Text with a one-pixel drop shadow, for sitting on a busy or translucent
/// fill.
#[allow(clippy::too_many_arguments)]
fn shadowed_text(
    p: &egui::Painter,
    pos: Pos2,
    anchor: Align2,
    s: &str,
    font: FontId,
    color: Color32,
    shadow: Color32,
    max_w: f32,
    z: f32,
) {
    let g = galley(p, s, font, color, max_w);
    let r = anchor.anchor_size(pos, g.size());
    p.galley(r.min + egui::vec2(0.0, 1.0 * z), g.clone(), shadow);
    p.galley(r.min, g, color);
}

/// A number standing in for how this one fixture's tile was cut, hung and
/// creased.
///
/// The looks that vary tile to tile — the crooked tape, the bow in a gel —
/// have to look hand-made and yet be identical on every frame, including
/// while the grid scrolls and rewraps. So the variation is hashed from the
/// fixture itself rather than from where its tile happens to land.
fn seed(t: &Tile) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for b in t.name.bytes().chain(t.addr.bytes()).take(24) {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    h
}

fn select_ring(p: &egui::Painter, rect: Rect, rounding: f32, z: f32, t: &Tile) {
    if t.selected {
        p.rect_stroke(rect, rounding, Stroke::new(2.0 * z, SELECT));
    }
}

/// Brightness of a metal surface at angle `a`, lit from the upper left
/// with a softer second reflection from the lower right.
fn metal(a: f32) -> f32 {
    let k = 0.5 + 0.5 * (a - 1.25 * PI).cos();
    let k2 = 0.5 + 0.5 * (a - 0.25 * PI).cos();
    (0.26 + 0.64 * k * k + 0.22 * k2 * k2).min(1.0)
}

fn steel(v: f32) -> Color32 {
    let g = (v * 255.0) as u8;
    Color32::from_rgb(g, g, g.saturating_add(5))
}

fn top_rounding(r: f32) -> Rounding {
    Rounding {
        nw: r,
        ne: r,
        sw: 0.0,
        se: 0.0,
    }
}

fn bottom_rounding(r: f32) -> Rounding {
    Rounding {
        nw: 0.0,
        ne: 0.0,
        sw: r,
        se: r,
    }
}

// ---- looks ----

/// The black bed a [`lit`] chip is seated in, so the cap has an edge to
/// sit against instead of floating on the panel.
const SOCKET: Color32 = Color32::from_rgb(0x08, 0x09, 0x0B);

/// Bare moulding grey. A fixture with no output has no hue to show, so the
/// cap goes to this and the colour mixes back in as the level comes up.
const RESIN: Color32 = Color32::from_rgb(0x25, 0x29, 0x2F);

/// A rounded rectangle shaded per point of its outline, the fan filling in
/// between.
///
/// [`vgradient_mesh`] with the direction left open, so the light can run
/// across as well as down. Only the outline is sampled and the inside is a
/// fan from one corner, so keep `shade` close to affine in the point — a
/// ramp, not a hot spot — and put a stroke over the silhouette, which has
/// no feather.
fn shaded_mesh(rect: Rect, rounding: Rounding, shade: impl Fn(Pos2) -> Color32) -> egui::Mesh {
    let mut path = Vec::with_capacity(36);
    egui::epaint::tessellator::path::rounded_rectangle(&mut path, rect, rounding);
    let mut mesh = egui::Mesh::default();
    for p in &path {
        mesh.colored_vertex(*p, shade(*p));
    }
    for i in 2..path.len() as u32 {
        mesh.add_triangle(0, i - 1, i);
    }
    mesh
}

/// The default: a moulded colour cap seated in a black socket, lit from up
/// and to the left.
///
/// The fall of light runs corner to corner rather than straight down, so
/// the cap has a near side and a far side and the eye puts a lamp
/// somewhere. Crown, left flank, foot and right flank each carry their own
/// hairline, which is what sells a radiused moulding at 35 px tall.
///
/// Output is a material change, not only a brightness one. At zero the cap
/// is bare resin; as the fixture is driven the hue mixes into the
/// moulding, the crown gloss tightens and brightens, and a little of the
/// colour spills onto the socket around it. So a dark fixture reads as
/// unlit plastic, and a half-lit one reads as half lit — which a flat
/// swatch of `t.color` never did.
fn lit(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let on = t.level > 0.02;
    // Enough hue stays in at the bottom of the range that a red at 10% is
    // still a red and not a grey.
    let base = if on {
        RESIN.lerp_to_gamma(t.color, 0.34 + 0.66 * lv)
    } else {
        RESIN
    };

    // The socket. Light spills round the edge of a driven cap, so it takes
    // a little of the colour; a selected one is bedded in blue instead.
    let ro = 5.0 * z;
    let bed = if t.selected {
        SELECT.lerp_to_gamma(Color32::BLACK, 0.34)
    } else if on {
        SOCKET.lerp_to_gamma(t.color, 0.14 * lv)
    } else {
        SOCKET
    };
    p.rect_filled(rect, ro, bed);

    // The cap, proud of the socket by a hairline all round. 5.0 - 1.5 =
    // 3.5, so the two roundings are concentric.
    let chip = rect.shrink(1.5 * z);
    let r = 3.5 * z;
    let w = chip.width().max(1.0);
    let h = chip.height().max(1.0);
    let crest = lighten(base, 0.17 + 0.12 * lv);
    let trough = darken(base, 0.32);
    p.add(Shape::mesh(shaded_mesh(chip, Rounding::same(r), |q| {
        // Mostly down, like everything else on the desk, but far enough
        // across that the lamp has a position. Affine in the point, so the
        // fan reproduces it exactly.
        let k = 0.34 * (q.x - chip.left()) / w + 0.66 * (q.y - chip.top()) / h;
        crest.lerp_to_gamma(trough, k.clamp(0.0, 1.0))
    })));

    // Gloss on the crown radius: a tight band rather than a wash over the
    // top third, so it never lifts the ground under the name.
    let band = (h * 0.17).max(2.0);
    let sheen = Rect::from_min_max(chip.min, Pos2::new(chip.right(), chip.top() + band));
    let peak = if on { 26.0 + 44.0 * lv } else { 30.0 };
    p.add(Shape::mesh(shaded_mesh(sheen, top_rounding(r), |q| {
        let down = 1.0 - ((q.y - sheen.top()) / band).clamp(0.0, 1.0);
        let across = 1.0 - 0.45 * ((q.x - chip.left()) / w).clamp(0.0, 1.0);
        Color32::from_white_alpha((peak * down * across) as u8)
    })));

    // The cap rolls away at the foot: the last few pixels lose the light.
    // It stops above the address's x-height.
    let foot = Rect::from_min_max(Pos2::new(chip.left(), chip.bottom() - h * 0.18), chip.max);
    p.add(Shape::mesh(vgradient_mesh(
        foot,
        bottom_rounding(r),
        Color32::TRANSPARENT,
        Color32::from_black_alpha(58),
    )));

    // Rim first, to cover the unfeathered silhouette of the three meshes,
    // then the four edges of the moulding. Those grow with zoom or the cap
    // reads flatter the closer you get; the rim stays a 1 px seam.
    p.rect_stroke(chip, r, Stroke::new(1.0, darken(base, 0.52)));
    let hair = z.clamp(1.0, 2.0);
    let flat_x = (chip.left() + r)..=(chip.right() - r);
    let flat_y = (chip.top() + r)..=(chip.bottom() - r);
    let crown = if on {
        lighten(base, 0.55).gamma_multiply(0.40 + 0.45 * lv)
    } else {
        Color32::from_white_alpha(34)
    };
    p.hline(flat_x.clone(), chip.top() + hair, Stroke::new(hair, crown));
    p.vline(
        chip.left() + hair,
        flat_y.clone(),
        Stroke::new(hair, Color32::from_white_alpha(38)),
    );
    p.hline(
        flat_x,
        chip.bottom() - hair,
        Stroke::new(hair, Color32::from_black_alpha(105)),
    );
    p.vline(
        chip.right() - hair,
        flat_y,
        Stroke::new(hair, Color32::from_black_alpha(70)),
    );

    // One ink for both labels, read off the middle of the fall so neither
    // end of it can flip them apart, with a hairline of the opposite tone
    // under each for the colours that land near the threshold.
    let (txt, dim) = ink(crest.lerp_to_gamma(trough, 0.42));
    let shadow = if txt == Color32::BLACK {
        Color32::from_white_alpha(90)
    } else {
        Color32::from_black_alpha(140)
    };
    let lw = w - 7.0 * z;
    shadowed_text(
        p,
        chip.left_top() + egui::vec2(3.5 * z, 2.5 * z),
        Align2::LEFT_TOP,
        t.name,
        FontId::proportional(11.0 * z),
        txt,
        shadow,
        lw,
        z,
    );
    shadowed_text(
        p,
        chip.left_bottom() + egui::vec2(3.5 * z, -2.5 * z),
        Align2::LEFT_BOTTOM,
        t.addr,
        FontId::monospace(9.0 * z),
        dim,
        shadow,
        lw,
        z,
    );

    // Selection sits on the blue bed, drawn inside the tile rather than
    // straddling its edge, so the clip never eats half of it at high zoom.
    if t.selected {
        let sw = 2.0 * z;
        p.rect_stroke(rect.shrink(sw * 0.5), ro - sw * 0.5, Stroke::new(sw, SELECT));
    }
}

/// Anodised brass at brightness `v`: the amp plate's metal. One hue across
/// the whole range, so the brush grain reads as a single surface lit
/// unevenly rather than as stripes of different alloys.
fn brass(v: f32) -> Color32 {
    let f = |c: f32| (c * v).clamp(0.0, 255.0) as u8;
    Color32::from_rgb(f(252.0), f(226.0), f(166.0))
}

/// How bright the plate is `k` of the way down it: a dark lip along the
/// top, a broad highlight a third of the way down where the room light
/// rakes across the grain, a long fall into shadow, and a little bounce
/// off the desk at the bottom.
///
/// Brushed metal scatters light along the grain, so it shows a band and
/// not a ramp. A plain top-to-bottom ramp is what makes gold paint look
/// like plastic — the band is the whole cue.
fn plate_tone(k: f32) -> f32 {
    let bump = |c: f32, w: f32| {
        let d = (k - c) / w;
        (-(d * d)).exp()
    };
    0.17 + 0.30 * bump(0.30, 0.26) + 0.07 * bump(0.97, 0.10) - 0.05 * k
}

/// A rectangle filled with a vertical gradient of more than two stops:
/// `tone(k)` is sampled on `rows` evenly spaced lines from the top down and
/// each strip interpolates between the pair that bounds it.
///
/// Corners are square. Whatever uses it has either no rounding worth
/// speaking of or a stroke drawn over the edge — like the other meshes,
/// this one has no feather.
fn vband_mesh(rect: Rect, rows: u32, tone: impl Fn(f32) -> Color32) -> egui::Mesh {
    let rows = rows.max(2);
    let mut mesh = egui::Mesh::default();
    for i in 0..rows {
        let k = i as f32 / (rows - 1) as f32;
        let c = tone(k);
        let y = rect.top() + rect.height() * k;
        mesh.colored_vertex(Pos2::new(rect.left(), y), c);
        mesh.colored_vertex(Pos2::new(rect.right(), y), c);
    }
    for i in 0..rows - 1 {
        mesh.add_triangle(2 * i, 2 * i + 1, 2 * i + 2);
        mesh.add_triangle(2 * i + 1, 2 * i + 3, 2 * i + 2);
    }
    mesh
}

/// A cheap integer hash. The brush grain has to look scattered and be the
/// same on every frame, so each hairline's tone and its offset come from
/// the line's index — never from `rand`, never from the clock.
fn hash(i: u32) -> u32 {
    let mut h = i.wrapping_add(1).wrapping_mul(0x9E37_79B9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^ (h >> 13)
}

/// A slot-head screw sunk into a plate, its slot lying at `slot` radians.
///
/// What sells it at five pixels across is the pair of edges in the slot —
/// a lit lip along one side, a hard black line down the other — and the
/// shadow the head sits in. The head itself is two circles.
fn screw(p: &egui::Painter, c: Pos2, r: f32, slot: f32) {
    p.circle_filled(
        c + egui::vec2(0.0, 0.5 * r),
        r * 1.2,
        Color32::from_black_alpha(110),
    );
    p.circle_filled(c, r, steel(0.30));
    p.circle_filled(c + egui::vec2(-0.20 * r, -0.24 * r), r * 0.78, steel(0.70));
    let d = egui::vec2(slot.cos(), slot.sin());
    let lip = egui::vec2(-d.y, d.x) * (r * 0.30);
    p.line_segment(
        [c - d * r * 0.82 + lip, c + d * r * 0.82 + lip],
        Stroke::new(1.0, Color32::from_white_alpha(100)),
    );
    p.line_segment(
        [c - d * r * 0.82, c + d * r * 0.82],
        Stroke::new(1.2, Color32::from_black_alpha(210)),
    );
    p.circle_stroke(c, r, Stroke::new(1.0, Color32::from_black_alpha(160)));
}

/// A stroked arc from `a0` to `a1` radians, drawn as a polyline rather
/// than a mesh: a stroke is anti-aliased and a mesh edge is not, and a
/// glint on a lens is all edge.
fn arc(p: &egui::Painter, c: Pos2, radius: f32, a0: f32, a1: f32, stroke: Stroke) {
    const N: usize = 10;
    let pts: Vec<Pos2> = (0..N)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / (N - 1) as f32;
            c + egui::vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    p.add(Shape::line(pts, stroke));
}

/// A pilot lamp on an amplifier's faceplate.
///
/// Brushed brass with the grain running across it, two slot screws
/// flanking the lamp, and a faceted jewel screwed into a countersunk hole.
/// The bulb blooms through the glass in the fixture's colour and throws
/// its light back onto the plate; the legend is silkscreened underneath in
/// the panel's own cream ink, which warms as the lamp comes up. Unlit, the
/// glass goes cold and dark with the bulb still sitting visible inside it.
///
/// The plate is lit in a band rather than a ramp, because that is what
/// separates brushed metal from painted plastic, and the grain is strongest
/// inside the band where the light rakes it.
///
/// It is the one look that does not try to match the console. That is the
/// point — it is a piece of gear that wandered in, not a card.
fn jewel(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let on = t.level > 0.02;
    let rr = 1.5 * z;
    // Below about two-thirds size none of the fine cutting survives a
    // pixel, so it is dropped rather than smeared into grey.
    let fine = z >= 0.8;

    // ---- the plate ----
    p.add(Shape::mesh(vband_mesh(rect, 9, |k| brass(plate_tone(k)))));

    // Brush grain. The spacing is in screen pixels rather than zoomed ones,
    // so zooming up gives more, finer lines instead of turning the brushing
    // into grooves, and each line carries a sub-pixel offset from its hash
    // so the run never beats against the pixel grid into a moiré. Alpha
    // follows the plate's own light: brushing shows in the band and all but
    // vanishes in the shadow.
    let lines = ((rect.height() / 2.8) as u32).clamp(8, 24);
    let span = egui::Rangef::new(rect.left() + 2.0 * z, rect.right() - 2.0 * z);
    let step = (rect.height() - 3.0) / lines as f32;
    let h_all = rect.height().max(1.0);
    for i in 0..lines {
        let h = hash(i);
        let y = rect.top() + 1.5 + (i as f32 + 0.5) * step + (((h >> 8) & 7) as f32 - 3.5) * 0.18;
        let k = ((y - rect.top()) / h_all).clamp(0.0, 1.0);
        let amp = 0.30 + 0.70 * (plate_tone(k) / 0.46).clamp(0.0, 1.0);
        let a = ((6 + ((h >> 4) & 0x1F)) as f32 * amp) as u8;
        let tone = if h & 1 == 0 {
            Color32::from_white_alpha(a)
        } else {
            Color32::from_black_alpha(a.saturating_add(8))
        };
        p.hline(span, y, Stroke::new(1.0, tone));
    }

    let c = Pos2::new(rect.center().x, rect.top() + 15.0 * z);
    let r = 8.0 * z;
    let bez = 2.4 * z;
    let ch = 1.6 * z;

    // The lamp's light landing on the brass. Lightened rather than laid on
    // flat, so a blue fixture brightens the plate instead of darkening it,
    // and clipped to the plate so it runs off the edge the way light on a
    // real panel does rather than showing the mesh's cut. It dies before
    // the legend, which keeps the bottom third dark for the ink.
    if on {
        let pc = p.with_clip_rect(rect);
        pc.add(Shape::mesh(radial_mesh(
            c,
            (r + bez + ch) * 2.6,
            lighten(t.color, 0.30).gamma_multiply(0.34 * lv),
            Color32::TRANSPARENT,
        )));
    }

    // Machined edges: a lit chamfer along the top, shadow under the bottom,
    // the sides catching and losing the same light, and a near-black seam
    // all round so the plate seats on the console. They go on after the
    // spill so they close its clip.
    let edge = egui::Rangef::new(rect.left() + rr, rect.right() - rr);
    p.hline(edge, rect.top() + 1.2, Stroke::new(1.0, Color32::from_white_alpha(80)));
    p.hline(
        edge,
        rect.bottom() - 1.2,
        Stroke::new(1.0, Color32::from_black_alpha(170)),
    );
    let side = egui::Rangef::new(rect.top() + rr, rect.bottom() - rr);
    p.vline(rect.left() + 1.0, side, Stroke::new(1.0, Color32::from_white_alpha(26)));
    p.vline(
        rect.right() - 1.0,
        side,
        Stroke::new(1.0, Color32::from_black_alpha(120)),
    );
    p.rect_stroke(rect, rr, Stroke::new(1.0, Color32::from_rgb(0x14, 0x11, 0x0B)));

    // ---- hardware ----
    let sr = 2.7 * z;
    screw(p, Pos2::new(rect.left() + 6.8 * z, c.y), sr, 0.37 * PI);
    screw(p, Pos2::new(rect.right() - 6.8 * z, c.y), sr, -0.11 * PI);

    // The countersunk hole, lit opposite to everything else on the plate: a
    // cone shadows the wall nearest the light and brightens the far one.
    // That inversion is what makes it read as a hole and not a raised boss,
    // and the dark near wall keeps an amber jewel off warm brass.
    p.add(Shape::mesh(ring_mesh(c, r + bez, r + bez + ch, |a| {
        brass(0.10 + 0.40 * metal(a + PI))
    })));
    p.circle_stroke(
        c,
        r + bez + ch,
        Stroke::new(1.0, Color32::from_black_alpha(120)),
    );

    // Chrome bezel: cool metal against the warm plate. Selected, it wears
    // the selection blue.
    let sel = t.selected;
    p.add(Shape::mesh(ring_mesh(c, r, r + bez, |a| {
        let v = metal(a);
        if sel {
            SELECT
                .lerp_to_gamma(Color32::WHITE, v * 0.70)
                .lerp_to_gamma(Color32::BLACK, (1.0 - v) * 0.55)
        } else {
            steel(v)
        }
    })));
    p.circle_stroke(c, r + bez, Stroke::new(1.0, Color32::from_black_alpha(180)));
    p.circle_stroke(c, r, Stroke::new(1.0, Color32::from_black_alpha(160)));

    // ---- the jewel ----
    // Unlit glass is cold and dark in its own right, not a grey disc.
    let base = if on {
        t.color
    } else {
        Color32::from_rgb(0x1B, 0x1E, 0x24)
    };
    let core_r = r * 0.60;
    // The cut bevel: bright where it faces the light, near-black opposite.
    p.add(Shape::mesh(ring_mesh(c, core_r, r, |a| {
        let k = 0.5 + 0.5 * (a - 1.25 * PI).cos();
        let hot = k * k;
        lighten(base, hot * (0.20 + 0.50 * lv))
            .lerp_to_gamma(darken(base, 0.70), (1.0 - k) * 0.85)
    })));
    // The dome inside it.
    p.add(Shape::mesh(radial_mesh(
        c,
        core_r,
        lighten(base, 0.08 + 0.42 * lv),
        darken(base, 0.34),
    )));
    // Eight cuts across the bevel, each lit or shaded by which way it
    // faces. Eight cuts at one brightness is what makes drawn cut glass
    // look drawn.
    if fine {
        for i in 0..8u32 {
            let a = i as f32 / 8.0 * 2.0 * PI + PI / 8.0;
            let d = egui::vec2(a.cos(), a.sin());
            let k = 0.5 + 0.5 * (a - 1.25 * PI).cos();
            let cut = if k > 0.5 {
                Color32::from_white_alpha((25.0 + 165.0 * (k - 0.5) * 2.0 * (0.6 + 0.4 * lv)) as u8)
            } else {
                Color32::from_black_alpha((20.0 + 150.0 * (0.5 - k) * 2.0) as u8)
            };
            p.line_segment([c + d * core_r, c + d * r], Stroke::new(1.0, cut));
        }
    }
    // The bulb, sitting behind the glass rather than being the glass. The
    // filament fades up with the level instead of switching on at a
    // threshold, so a fixture coming up strikes rather than snaps.
    let bc = c + egui::vec2(0.0, 0.14 * r);
    if on {
        p.add(Shape::mesh(radial_mesh(
            bc,
            core_r * 0.92,
            lighten(t.color, 0.28 + 0.45 * lv).gamma_multiply(0.45 + 0.55 * lv),
            Color32::TRANSPARENT,
        )));
        let fade = ((lv - 0.12) / 0.25).clamp(0.0, 1.0);
        if fade > 0.0 {
            p.line_segment(
                [bc - egui::vec2(1.6 * z, 0.0), bc + egui::vec2(1.6 * z, 0.0)],
                Stroke::new(1.3 * z, lighten(t.color, 0.75).gamma_multiply(fade)),
            );
        }
    } else {
        p.add(Shape::mesh(radial_mesh(
            bc,
            core_r * 0.60,
            Color32::from_rgb(0x4C, 0x4E, 0x54),
            Color32::TRANSPARENT,
        )));
        p.line_segment(
            [bc - egui::vec2(1.4 * z, 0.0), bc + egui::vec2(1.4 * z, 0.0)],
            Stroke::new(1.0, Color32::from_rgb(0x16, 0x17, 0x1A)),
        );
    }
    // The bevel catching the room: a hard arc on the upper left with a
    // softer pass under it so it dies away at the ends, and a weak bounce
    // back at the lower right.
    let gr = r - 0.8 * z;
    if fine {
        arc(p, c, gr, 1.02 * PI, 1.52 * PI, Stroke::new(1.0 * z, Color32::from_white_alpha(110)));
        arc(p, c, gr, 0.10 * PI, 0.44 * PI, Stroke::new(1.0 * z, Color32::from_white_alpha(60)));
    }
    arc(p, c, gr, 1.13 * PI, 1.39 * PI, Stroke::new(1.2 * z, Color32::from_white_alpha(225)));
    // One small, hard specular. Small and hard reads as glass; big and soft
    // reads as plastic.
    let sp = c + egui::vec2(-0.34 * r, -0.40 * r);
    p.add(Shape::mesh(radial_mesh(
        sp,
        0.30 * r,
        Color32::from_white_alpha(if on { 170 } else { 120 }),
        Color32::TRANSPARENT,
    )));
    p.circle_filled(sp, 0.13 * r, Color32::from_white_alpha(if on { 245 } else { 195 }));

    // ---- silkscreen ----
    // The plate's own cream ink, picking up the lamp as it lights. Both
    // lines hang off the bottom edge rather than being placed from the top,
    // so they cannot run past it whatever height the font turns out to be.
    let silk = Color32::from_rgb(0xF0, 0xE4, 0xC4);
    let legend = if on {
        silk.lerp_to_gamma(lighten(t.color, 0.55), 0.24 * lv)
    } else {
        silk
    };
    let sh = Color32::from_black_alpha(140);
    let w = rect.width() - 8.0 * z;
    shadowed_text(
        p,
        Pos2::new(c.x, rect.bottom() - 15.0 * z),
        Align2::CENTER_BOTTOM,
        t.name,
        FontId::proportional(10.0 * z),
        legend,
        sh,
        w,
        z,
    );
    shadowed_text(
        p,
        Pos2::new(c.x, rect.bottom() - 3.0 * z),
        Align2::CENTER_BOTTOM,
        t.addr,
        FontId::monospace(8.5 * z),
        legend.gamma_multiply(0.74),
        sh,
        w,
        z,
    );
    select_ring(p, rect, rr, z, t);
}

/// A coordinate pushed onto a pixel centre, so a one-pixel stroke lands on
/// one lit row of pixels instead of two half-lit ones. `ppp` is the
/// painter's pixels-per-point, read once per tile rather than once per
/// line.
fn snap(v: f32, ppp: f32) -> f32 {
    ((v * ppp - 0.5).round() + 0.5) / ppp
}

/// One line of the readout face: monospace, the letters pushed `track`
/// apart, laid into `row` and cut with an ellipsis where the row ends. The
/// air between the characters is most of what separates a readout from a
/// label.
fn hud_text(
    p: &egui::Painter,
    row: Rect,
    anchor: Align2,
    s: &str,
    size: f32,
    track: f32,
    color: Color32,
) {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        s,
        0.0,
        egui::text::TextFormat {
            font_id: FontId::monospace(size),
            extra_letter_spacing: track,
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping {
        max_width: row.width().max(6.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    let g = p.layout_job(job);
    p.galley(anchor.align_size_within_rect(g.size(), row).min, g, color);
}

/// An instrument face: one machined corner off the top right, and every
/// scrap of light in the tile coming out of the meter along the bottom.
///
/// No room lights this one. The slab is near-black under the shadow of the
/// lip it is recessed behind, and the fixture's colour only turns up where
/// the meter puts it — a haze over the lower face, a column standing over
/// each lit cell, and a spill along the bottom edge, which is what lets a
/// wall of a hundred tiles be read for colour at a glance. The meter is
/// eight cells, so a level is a count before it is a length: a fixture at
/// a third lights its left third and nothing above it. Selection clamps
/// the two edges that give the shape its character, a spine down the left
/// and a bracket round the cut corner, and nothing else in the tile is
/// ever that blue.
fn future(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lvl = t.level.clamp(0.0, 1.0);
    let lv = vis(t.level);
    let on = lvl > 0.02;
    // Below about three-quarter zoom the index scale and the numeric
    // readout are a smudge, and eight cells come out narrower than the
    // gaps between them. Both go, and the meter halves its resolution.
    let detail = z >= 0.85;
    let ppp = p.ctx().pixels_per_point();

    let (x0, x1) = (rect.left(), rect.right());
    let (y0, y1) = (rect.top(), rect.bottom());
    let h = rect.height().max(1.0);
    // One corner cut, top right. Four square corners and a fifth that is
    // not is a machined part; four cut corners is a decoration.
    let ch = 9.0 * z;
    let face = [
        Pos2::new(x0, y0),
        Pos2::new(x1 - ch, y0),
        Pos2::new(x1, y0 + ch),
        Pos2::new(x1, y1),
        Pos2::new(x0, y1),
    ];

    // The slab, and the shadow of the lip it is recessed behind. The
    // shadow's right edge runs down the chamfer itself, so the frame
    // stroke later covers the steps both unfeathered meshes leave there.
    p.add(Shape::mesh(convex_mesh(&face, |q| {
        Color32::from_rgb(0x07, 0x09, 0x0C)
            .lerp_to_gamma(Color32::from_rgb(0x16, 0x1B, 0x24), (q.y - y0) / h)
    })));
    let sd = 5.0 * z;
    let lip = [
        Pos2::new(x0, y0),
        Pos2::new(x1 - ch, y0),
        Pos2::new(x1 - ch + sd, y0 + sd),
        Pos2::new(x0, y0 + sd),
    ];
    p.add(Shape::mesh(convex_mesh(&lip, |q| {
        Color32::from_black_alpha((110.0 * (1.0 - (q.y - y0) / sd)) as u8)
    })));

    // The selected wash goes down this early so it sits under the type
    // instead of greying it.
    if t.selected {
        let w = 12.0 * z;
        let wash = [
            Pos2::new(x0, y0),
            Pos2::new(x0 + w, y0),
            Pos2::new(x0 + w, y1),
            Pos2::new(x0, y1),
        ];
        p.add(Shape::mesh(convex_mesh(&wash, |q| {
            if q.x <= x0 {
                SELECT.gamma_multiply(0.24)
            } else {
                Color32::TRANSPARENT
            }
        })));
    }

    // The meter's milled channel, and the cells lying in it.
    let trough = Rect::from_min_max(
        Pos2::new(x0 + 3.0 * z, y1 - 10.0 * z),
        Pos2::new(x1 - 3.0 * z, y1 - 3.0 * z),
    );
    let cells = trough.shrink(1.0 * z);
    let segs: usize = if detail { 8 } else { 4 };
    let gap = (1.0 * z).max(1.0);
    let sw = ((cells.width() - gap * (segs - 1) as f32) / segs as f32).max(1.0);
    let f = lvl * segs as f32;
    let cell_x = |i: usize| cells.left() + i as f32 * (sw + gap);

    if on {
        // Haze over the lower face: the light that made it up the slab.
        p.add(Shape::mesh(vgradient_mesh(
            Rect::from_min_max(Pos2::new(x0, y1 - h * 0.62), Pos2::new(x1, y1)),
            Rounding::ZERO,
            Color32::TRANSPARENT,
            t.color.gamma_multiply(0.20 * lv),
        )));
        // Spill under the channel, out along the bottom edge. Three pixels
        // of hue is what makes the colour of a whole grid readable.
        p.add(Shape::mesh(vgradient_mesh(
            Rect::from_min_max(Pos2::new(x0, trough.bottom()), Pos2::new(x1, y1)),
            Rounding::ZERO,
            t.color.gamma_multiply(0.52 * lv),
            t.color.gamma_multiply(0.14 * lv),
        )));
        // Light over the lit run and nothing over the dead one, so the
        // glow counts the same count the meter does: a sheet as far as
        // the run reaches, ribbed by a column standing over each cell.
        let rise = (trough.top() - 10.0 * z).max(y0);
        let lit = (f.ceil() as usize).min(segs);
        p.add(Shape::mesh(vgradient_mesh(
            Rect::from_min_max(
                Pos2::new(cells.left(), rise),
                Pos2::new(cell_x(lit.max(1) - 1) + sw, trough.top()),
            ),
            Rounding::ZERO,
            Color32::TRANSPARENT,
            t.color.gamma_multiply(0.22 * lv),
        )));
        for i in 0..lit {
            let k = (f - i as f32).clamp(0.0, 1.0);
            p.add(Shape::mesh(vgradient_mesh(
                Rect::from_min_max(
                    Pos2::new(cell_x(i), rise),
                    Pos2::new(cell_x(i) + sw, trough.top()),
                ),
                Rounding::ZERO,
                Color32::TRANSPARENT,
                t.color.gamma_multiply(0.30 * lv * k),
            )));
        }
    }

    p.rect_filled(
        trough,
        0.0,
        if on {
            Color32::from_rgb(0x04, 0x05, 0x0A).lerp_to_gamma(t.color, 0.10 * lv)
        } else {
            Color32::from_rgb(0x05, 0x06, 0x0A)
        },
    );
    // The seam along the bottom of the channel: where the light gets out.
    p.hline(
        trough.x_range(),
        snap(trough.bottom(), ppp),
        Stroke::new(
            1.0,
            if on {
                lighten(t.color, 0.12).gamma_multiply(0.55 + 0.45 * lv)
            } else {
                Color32::from_rgb(0x17, 0x1C, 0x24)
            },
        ),
    );
    let dead = Color32::from_rgb(0x1B, 0x21, 0x2B);
    let cap = lighten(t.color, 0.75);
    let cap_y = snap(cells.top() + 0.5, ppp);
    for i in 0..segs {
        // The cell the level lands in is part lit, so the meter still
        // moves smoothly between its steps; and the run brightens toward
        // its head, so a full meter is not just a bar.
        let k = (f - i as f32).clamp(0.0, 1.0);
        let ramp = i as f32 / (segs - 1) as f32;
        let live = lighten(t.color, 0.06 + 0.10 * ramp + 0.26 * lv);
        let cell = Rect::from_min_max(
            Pos2::new(cell_x(i), cells.top()),
            Pos2::new(cell_x(i) + sw, cells.bottom()),
        );
        p.rect_filled(cell, 0.0, dead.lerp_to_gamma(live, k));
        if k > 0.25 {
            p.hline(cell.x_range(), cap_y, Stroke::new(1.0, cap.gamma_multiply(k)));
        }
    }

    // Index scale over the meter: a faint rule, a long mark at the half
    // and short ones at the quarters, the ones the level has passed
    // taking its colour.
    let slate = Color32::from_rgb(0x26, 0x2D, 0x38);
    if detail {
        let rule = snap(trough.top() - 3.0 * z, ppp);
        p.hline(
            trough.x_range(),
            rule,
            Stroke::new(1.0, slate.gamma_multiply(0.7)),
        );
        let reach = cells.left() + cells.width() * lvl;
        for i in 1..4 {
            let x = snap(cells.left() + cells.width() * i as f32 / 4.0, ppp);
            let c = if on && x <= reach {
                lighten(t.color, 0.40)
            } else {
                slate
            };
            let drop = if i == 2 { 2.5 * z } else { 1.5 * z };
            p.vline(x, rule..=(rule + drop), Stroke::new(1.0, c));
        }
    }

    // Frame: dead slate all round, because nothing lights the top of this
    // tile. Only the bottom edge and the last stretch of each side are
    // near enough the meter to take its colour.
    p.add(Shape::closed_line(face.to_vec(), Stroke::new(1.0, slate)));
    if on {
        let y = y1 - h * 0.42;
        p.add(Shape::line(
            vec![
                Pos2::new(x0, y),
                Pos2::new(x0, y1),
                Pos2::new(x1, y1),
                Pos2::new(x1, y),
            ],
            Stroke::new(1.0, slate.lerp_to_gamma(t.color, 0.55 * lv)),
        ));
    }

    // Two rows of monospace caps: the name untracked, then the address
    // tracked out with the level beside it as a three-digit percent. The
    // name stops 5px short of the right edge, which is where the chamfer
    // would start cutting its ascenders.
    let tx = x0 + 4.0 * z;
    hud_text(
        p,
        Rect::from_min_max(
            Pos2::new(tx, y0 + 3.5 * z),
            Pos2::new(x1 - 5.0 * z, y0 + 14.5 * z),
        ),
        Align2::LEFT_TOP,
        &t.name.to_uppercase(),
        9.0 * z,
        0.0,
        if on {
            theme::TEXT
        } else {
            theme::TEXT.gamma_multiply(0.58)
        },
    );
    let data = Rect::from_min_max(
        Pos2::new(tx, y0 + 15.5 * z),
        Pos2::new(x1 - 3.5 * z, y0 + 25.5 * z),
    );
    let pct_w = if detail { 21.0 * z } else { 0.0 };
    hud_text(
        p,
        Rect::from_min_max(data.min, Pos2::new(data.right() - pct_w, data.bottom())),
        Align2::LEFT_TOP,
        t.addr,
        8.0 * z,
        0.7 * z,
        if on {
            lighten(t.color, 0.55)
        } else {
            theme::TEXT_DIM.gamma_multiply(0.6)
        },
    );
    if detail {
        hud_text(
            p,
            data,
            Align2::RIGHT_TOP,
            &format!("{:03}", (lvl * 100.0).round() as i32),
            8.0 * z,
            0.7 * z,
            if on {
                lighten(t.color, 0.28)
            } else {
                Color32::from_gray(66)
            },
        );
    }

    // Selected: a spine down the left edge and a bracket clamping the cut
    // corner. The bracket is clipped to the tile so it sits flush with the
    // two edges it grips instead of hanging over them, and the spine holds
    // at half zoom where a hairline outline would not.
    if t.selected {
        p.rect_filled(
            Rect::from_min_max(rect.left_top(), Pos2::new(x0 + (2.5 * z).max(2.0), y1)),
            0.0,
            SELECT,
        );
        let pc = p.with_clip_rect(rect);
        let stub = 4.5 * z;
        let brk = Stroke::new((1.8 * z).max(1.4), SELECT);
        pc.line_segment([Pos2::new(x1 - ch - stub, y0), Pos2::new(x1 - ch, y0)], brk);
        pc.line_segment([Pos2::new(x1 - ch, y0), Pos2::new(x1, y0 + ch)], brk);
        pc.line_segment([Pos2::new(x1, y0 + ch), Pos2::new(x1, y0 + ch + stub)], brk);
    }
}

/// One run of a chamfer: a thin axis-aligned band shading from `a` at one
/// end to `b` at the other, so the light slides along the cut edge rather
/// than sitting on the face as a blob. `along_x` shades left to right,
/// otherwise top to bottom.
fn bevel_run(p: &egui::Painter, band: Rect, along_x: bool, a: Color32, b: Color32) {
    let pts = [
        band.left_top(),
        band.right_top(),
        band.right_bottom(),
        band.left_bottom(),
    ];
    p.add(Shape::mesh(convex_mesh(&pts, |q| {
        let k = if along_x {
            (q.x - band.left()) / band.width().max(1.0)
        } else {
            (q.y - band.top()) / band.height().max(1.0)
        };
        a.lerp_to_gamma(b, k.clamp(0.0, 1.0))
    })));
}

/// A slab of cut glass lying in a pocket milled into the desk: a black rim
/// with the lit chamfer trapped behind it, a window standing in the
/// polish, the caustic doubled along the bottom where the far face shows
/// through the near one, and the fixture's colour thrown down onto the
/// floor underneath.
fn glass(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let on = t.level > 0.02;
    let r = 3.0 * z;
    let rr = Rounding::same(r);
    let c = 1.8 * z;
    // The edge stack is the material, so its hairlines never thin below a
    // real pixel: at z = 0.5 a half-pixel stroke greys out into nothing.
    let hw = (1.0 * z).max(1.0);

    // The pocket the slab lies in: floor in shadow under the near wall,
    // coming back up to the light at the front. The stroke is only there
    // to cover the mesh's unfeathered corners.
    p.add(Shape::mesh(vgradient_mesh(
        rect,
        rr,
        darken(theme::WELL, 0.45),
        lighten(theme::WELL, 0.18),
    )));
    p.rect_stroke(rect, rr, Stroke::new(1.0, darken(theme::WELL, 0.25)));

    // The slab. Square-cut — glass is sawn and polished, not moulded, and
    // a generous radius was most of what made the old tile read as a gel
    // button — and set in far enough that its own shadow clears it.
    let pane = Rect::from_min_max(
        rect.min + egui::vec2(3.0 * z, 2.5 * z),
        rect.max - egui::vec2(3.0 * z, 5.5 * z),
    );
    let hue = if on {
        t.color
    } else {
        Color32::from_rgb(0x1E, 0x24, 0x2E)
    };

    // A transparent thing casts a coloured shadow. Nothing else on the
    // grid does, and it is the cheapest proof that the light gets through.
    p.rect_filled(
        pane.translate(egui::vec2(1.6 * z, 1.8 * z)),
        Rounding::ZERO,
        if on {
            darken(hue, 0.76 - 0.18 * lv)
        } else {
            Color32::from_black_alpha(165)
        },
    );
    // And the pool it throws on the floor under its bottom edge.
    if on {
        let spill = Rect::from_min_max(
            Pos2::new(pane.left() + 1.0 * z, pane.bottom()),
            Pos2::new(pane.right() + 1.6 * z, pane.bottom() + 4.4 * z),
        );
        p.add(Shape::mesh(vgradient_mesh(
            spill,
            Rounding::ZERO,
            lighten(hue, 0.18).gamma_multiply(0.24 + 0.36 * lv),
            Color32::TRANSPARENT,
        )));
    }

    // Transmission: the tint goes over the pocket rather than instead of
    // it, and thins towards the bottom so the floor's own gradient comes
    // back through exactly where the light is leaving the slab.
    let (a_top, a_bot) = if on {
        (0.26 + 0.52 * lv, 0.18 + 0.40 * lv)
    } else {
        (0.66, 0.52)
    };
    p.add(Shape::mesh(vgradient_mesh(
        pane,
        Rounding::ZERO,
        hue.gamma_multiply(a_top),
        hue.gamma_multiply(a_bot),
    )));
    let body = theme::WELL.lerp_to_gamma(hue, 0.5 * (a_top + a_bot));

    // Light that entered the top face gathering against the bottom edge
    // before it leaves again.
    p.add(Shape::mesh(vgradient_mesh(
        Rect::from_min_max(
            Pos2::new(pane.left(), pane.bottom() - 7.0 * z),
            pane.right_bottom(),
        ),
        Rounding::ZERO,
        Color32::TRANSPARENT,
        lighten(hue, 0.35).gamma_multiply(if on { 0.14 + 0.38 * lv } else { 0.10 }),
    )));

    // The window standing in the polish: two uprights, wide and narrow,
    // running the full depth of the face. They have to go all the way from
    // the top cut to the bottom one — a reflection that stops halfway down
    // is not a reflection, it is a mark on the glass — and they lean on
    // being hard-edged, because a soft blob pasted on the face is the
    // plastic tell. Faint enough that the name reads straight through
    // them, which is also what a reflection does.
    let face_top = pane.top() + c;
    let face_bot = pane.bottom() - c;
    for (x0, x1, top, bot) in [(13.4, 10.8, 104, 26), (9.4, 8.2, 62, 14)] {
        p.add(Shape::mesh(vgradient_mesh(
            Rect::from_min_max(
                Pos2::new(pane.right() - x0 * z, face_top),
                Pos2::new(pane.right() - x1 * z, face_bot),
            ),
            Rounding::ZERO,
            Color32::from_white_alpha(top),
            Color32::from_white_alpha(bot),
        )));
    }

    // Name and address read off the composited face rather than off the
    // hue, which is why `body` is a gamma lerp and not a linear one — get
    // that wrong and a mid-level lamp picks white letters for a face that
    // renders pale. Of the hues on the grid only a white one crosses the
    // black/white boundary, around 43%, and the heavy opposite-value
    // shadow keeps the name legible either side of it.
    let (txt, dim) = ink(body);
    let sh = if txt == Color32::BLACK {
        Color32::from_white_alpha(170)
    } else {
        Color32::from_black_alpha(195)
    };
    shadowed_text(
        p,
        Pos2::new(pane.left() + 4.0 * z, pane.top() + 4.0 * z),
        Align2::LEFT_TOP,
        t.name,
        FontId::proportional(10.0 * z),
        txt,
        sh,
        pane.width() - 22.0 * z,
        z,
    );
    shadowed_text(
        p,
        Pos2::new(pane.left() + 4.0 * z, pane.bottom() - 5.6 * z),
        Align2::LEFT_BOTTOM,
        t.addr,
        FontId::monospace(8.0 * z),
        dim,
        sh,
        pane.width() - 8.0 * z,
        z,
    );

    // The chamfer all round, lit from the upper left: the top run carries
    // the specular, the left is mid, the right falls into shadow and the
    // bottom is the caustic — the light that went in the top leaving
    // again, concentrated and still carrying the tint. A selected slab is
    // cut from blue glass instead.
    let bev = if t.selected {
        SELECT.lerp_to_gamma(body, 0.22)
    } else {
        body
    };
    bevel_run(
        p,
        Rect::from_min_max(pane.left_top(), Pos2::new(pane.left() + c, pane.bottom())),
        false,
        lighten(bev, 0.46),
        lighten(bev, 0.06),
    );
    bevel_run(
        p,
        Rect::from_min_max(Pos2::new(pane.right() - c, pane.top()), pane.right_bottom()),
        false,
        darken(bev, 0.22),
        darken(bev, 0.58),
    );
    bevel_run(
        p,
        Rect::from_min_max(pane.left_top(), Pos2::new(pane.right(), pane.top() + c)),
        true,
        lighten(bev, 0.85),
        lighten(bev, 0.22),
    );
    let caustic = if on {
        lighten(hue, 0.42)
    } else {
        lighten(bev, 0.28)
    };
    bevel_run(
        p,
        Rect::from_min_max(Pos2::new(pane.left(), pane.bottom() - c), pane.right_bottom()),
        true,
        caustic.gamma_multiply(if on { 0.50 + 0.30 * lv } else { 1.0 }),
        caustic.gamma_multiply(if on { 0.75 + 0.25 * lv } else { 1.0 }),
    );
    // Where the two cut faces meet at the lit corner the light comes to a
    // point. It runs a little way down each of them rather than sitting in
    // the corner as a square, which is the difference between a polished
    // arris and a chipped pixel.
    let g = pane.min;
    p.add(Shape::mesh(vgradient_mesh(
        Rect::from_min_max(g, Pos2::new(g.x + c, g.y + 3.2 * z)),
        Rounding::ZERO,
        Color32::from_white_alpha(185),
        Color32::TRANSPARENT,
    )));
    bevel_run(
        p,
        Rect::from_min_max(g, Pos2::new(g.x + 3.6 * z, g.y + c)),
        true,
        Color32::from_white_alpha(185),
        Color32::TRANSPARENT,
    );
    // The far face of the slab, seen through the near one: its bottom edge
    // sits a couple of pixels short, so the caustic arrives doubled. Two
    // parallel lines down there is what thickness looks like.
    p.hline(
        (pane.left() + c)..=(pane.right() - c),
        pane.bottom() - c - 2.4 * z,
        Stroke::new(
            hw,
            caustic.gamma_multiply(if on { 0.34 + 0.30 * lv } else { 0.42 }),
        ),
    );

    // The edge stack, outside in: a black rim, the lit chamfer, a dark
    // step where the cut meets the face, and the line the refraction
    // throws just inside that. Four values inside four pixels is what
    // tells a slab of glass from a coloured card, and it costs four
    // strokes.
    p.rect_stroke(
        pane.shrink(c),
        Rounding::ZERO,
        Stroke::new(hw, Color32::from_black_alpha(125)),
    );
    let refr = if t.selected {
        SELECT
    } else {
        Color32::from_white_alpha(115)
    };
    let d = c + 1.2 * z;
    p.hline(
        (pane.left() + d)..=(pane.right() - d),
        pane.top() + d,
        Stroke::new(hw, refr),
    );
    p.vline(
        pane.left() + d,
        (pane.top() + d)..=(pane.bottom() - d),
        Stroke::new(hw, refr.gamma_multiply(0.5)),
    );
    p.rect_stroke(
        pane,
        Rounding::ZERO,
        Stroke::new(hw, Color32::from_black_alpha(215)),
    );
    select_ring(p, rect, r, z, t);
}

// ---- chrome ----

/// How much a 45° corner cut shrinks when the shape carrying it is offset
/// inward by one unit. Cutting the aperture by this much less than the
/// frame keeps the two the same shape, instead of a hole floating inside a
/// border.
const CHAMFER_INSET: f32 = 2.0 - std::f32::consts::SQRT_2;

/// A tone of the one bright source overhead, mirrored: grey run cool.
const fn sky(v: u8) -> Color32 {
    Color32::from_rgb(v.saturating_sub(11), v.saturating_sub(4), v)
}

/// A tone of the dark room under it: the same grey run warm.
const fn ground(v: u8) -> Color32 {
    Color32::from_rgb(v, v.saturating_sub(2), v.saturating_sub(5))
}

/// What one face of the bezel gives back, read from its outer edge inward:
/// how far across the face the horizon falls, then the tone at the outer
/// edge, the tone just short of the horizon, the tone just past it, and the
/// tone at the inner lip. That middle pair is a step, not a fade, and the
/// step is the whole difference between chrome and light grey plastic, so
/// nothing is allowed to smooth it out.
struct Mirror {
    horizon: f32,
    tone: [Color32; 4],
}

/// The top rail faces up and out: one pixel of source, blown out right at
/// the horizon, then the dark room it turns down into. Mostly dark, because
/// the room is mostly dark — chrome in here is a black bar with a white
/// line on it, not a pale one.
const M_TOP: Mirror = Mirror {
    horizon: 0.26,
    tone: [sky(0x96), sky(0xFF), ground(0x0C), ground(0x2E)],
};

/// The bottom rail faces down and out, so it gets the same room upside
/// down: dark where it leaves the tile, dead black at the turn, then the
/// inner lip tips back up into the source and throws that light into the
/// well.
const M_BOTTOM: Mirror = Mirror {
    horizon: 0.42,
    tone: [ground(0x2A), ground(0x0C), sky(0xF2), sky(0xB0)],
};

/// The side rails turn about a vertical axis, so they sweep the room
/// sideways and hold one tone across their width, losing a little as they
/// turn in. What changes is where you are along them: these four are the
/// tile's top corner, the horizon from just above and just below, and the
/// bottom corner.
const M_SIDE_TOP: Mirror = Mirror {
    horizon: 0.5,
    tone: [sky(0x52), sky(0x48), sky(0x40), sky(0x20)],
};
const M_SIDE_HAZE: Mirror = Mirror {
    horizon: 0.5,
    tone: [sky(0xE8), sky(0xC6), sky(0xAC), sky(0x4C)],
};
const M_SIDE_GROUND: Mirror = Mirror {
    horizon: 0.5,
    tone: [ground(0x1C), ground(0x14), ground(0x10), ground(0x0A)],
};
const M_SIDE_LOW: Mirror = Mirror {
    horizon: 0.5,
    tone: [ground(0x50), ground(0x3E), ground(0x32), ground(0x16)],
};

/// Selection reaches chrome by changing the room it mirrors: what catches
/// the source turns blue, while the dark, having nothing of its own to give
/// back, barely moves.
fn sel_sky(c: Color32) -> Color32 {
    let b = c.r().max(c.g()).max(c.b()) as f32 / 255.0;
    c.lerp_to_gamma(SELECT, 0.62 * b * b)
}

/// One face of the bezel with the tile's own light in it: the inner lip
/// catches a little of the lens it surrounds, and a selected tile mirrors a
/// blue room.
fn mirrored(m: &Mirror, glow: Color32, k: f32, sel: bool) -> Mirror {
    let mut tone = m.tone;
    tone[2] = tone[2].lerp_to_gamma(glow, k * 0.45);
    tone[3] = tone[3].lerp_to_gamma(glow, k);
    if sel {
        for c in &mut tone {
            *c = sel_sky(*c);
        }
    }
    Mirror {
        horizon: m.horizon,
        tone,
    }
}

/// The eight corners of a rectangle with its corners cut at 45°, clockwise
/// from the left end of the top edge.
fn chamfered(rect: Rect, cut: f32) -> [Pos2; 8] {
    let (l, t, r, b) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    [
        Pos2::new(l + cut, t),
        Pos2::new(r - cut, t),
        Pos2::new(r, t + cut),
        Pos2::new(r, b - cut),
        Pos2::new(r - cut, b),
        Pos2::new(l + cut, b),
        Pos2::new(l, b - cut),
        Pos2::new(l, t + cut),
    ]
}

/// One run of the bezel: the band between an outer edge and the inner edge
/// behind it, in two pieces that share no vertices so the horizon between
/// them stays a step instead of a fade. `ends` is what each end of the run
/// mirrors; the band twists from one to the other along its length.
fn bezel_run(mesh: &mut egui::Mesh, outer: [Pos2; 2], inner: [Pos2; 2], ends: [&Mirror; 2]) {
    for (a, b) in [(0usize, 1usize), (2usize, 3usize)] {
        let n = mesh.vertices.len() as u32;
        for idx in [a, b] {
            for (e, m) in ends.iter().enumerate() {
                let k = match idx {
                    0 => 0.0,
                    3 => 1.0,
                    _ => m.horizon,
                };
                mesh.colored_vertex(outer[e].lerp(inner[e], k), m.tone[idx]);
            }
        }
        mesh.add_triangle(n, n + 1, n + 3);
        mesh.add_triangle(n, n + 3, n + 2);
    }
}

/// Chrome: a machined bezel with a colour lens sunk in the well behind it.
///
/// Chrome is not grey, it is a mirror, and this room is dark with one hard
/// source over it — so the bezel is mostly near-black metal carrying two
/// catches: a blown-white line along the top edge, and the floor's bounce
/// on the bottom lip, thrown back up into the well. Between them the tone
/// does not fade, it steps. The top and bottom rails face up and down and
/// sweep the whole room across their five pixels; the side rails turn about
/// a vertical axis, hold one tone across their width and break along their
/// length instead, which is why the four rails never match. The frame is
/// one constant width with its corners cut at 45°, the aperture repeats the
/// cut so it is the frame offset inward rather than a second rounded box,
/// and the lens sits far enough down the well for the inner lip to throw a
/// hard shadow over its top and left.
fn chrome(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let base = if t.level > 0.02 {
        t.color
    } else {
        Color32::from_rgb(0x24, 0x26, 0x2B)
    };
    let sel = t.selected;

    // The rail and the hairlines on it keep a floor in real pixels: below
    // about three, the catch and the turn-away eat the mirror between them
    // and the frame collapses to two lines.
    let w = (5.0 * z).max(3.0);
    let hair = (1.0 * z).max(1.0);
    let cut = 5.5 * z;
    let well = rect.shrink(w);
    let outer = chamfered(rect, cut);
    let aper = chamfered(well, (cut - w * CHAMFER_INSET).max(1.0));

    // ---- the lens, down in the well ----
    let pw = p.with_clip_rect(well);
    let (ww, wh) = (well.width().max(1.0), well.height().max(1.0));
    // Bare machined metal, in case a corner of the well shows past the cut.
    pw.rect_filled(well, Rounding::ZERO, Color32::from_rgb(0x12, 0x13, 0x16));
    // The lens body, top of the well to the bottom of it. Its top stays
    // deep in the lip's shade whatever colour the fixture is, which is half
    // of what keeps the name readable.
    let body = |k: f32| {
        darken(base, 0.66 - 0.18 * lv).lerp_to_gamma(darken(base, 0.22 - 0.18 * lv), k)
    };
    let dome = lighten(base, 0.06 + 0.40 * lv);
    pw.add(Shape::mesh(convex_mesh(&aper, |q| {
        body(((q.y - well.top()) / wh).clamp(0.0, 1.0))
    })));
    // The dome's own light, low and left of centre so it lifts the floor of
    // the well and not the band the name sits in.
    pw.add(Shape::mesh(radial_mesh(
        well.center() + egui::vec2(-0.10 * ww, 0.20 * wh),
        ww * 0.54,
        dome.gamma_multiply(0.88),
        Color32::TRANSPARENT,
    )));
    // The inner lip overhangs the lens: a hard shadow across its top with a
    // short penumbra under it, so the shadow has an edge AND an exit.
    let cast = Rect::from_min_max(well.min, Pos2::new(well.right(), well.top() + wh * 0.16));
    pw.rect_filled(cast, Rounding::ZERO, Color32::from_black_alpha(165));
    pw.add(Shape::mesh(vgradient_mesh(
        Rect::from_min_max(
            cast.left_bottom(),
            Pos2::new(well.right(), well.top() + wh * 0.36),
        ),
        Rounding::ZERO,
        Color32::from_black_alpha(165),
        Color32::TRANSPARENT,
    )));
    // The same lip down the left side, softer: the light comes from up-left.
    let side = Rect::from_min_max(well.min, Pos2::new(well.left() + ww * 0.30, well.bottom()));
    pw.add(Shape::mesh(convex_mesh(
        &[
            side.left_top(),
            side.right_top(),
            side.right_bottom(),
            side.left_bottom(),
        ],
        |q| {
            let k = ((q.x - side.left()) / side.width().max(1.0)).clamp(0.0, 1.0);
            Color32::from_black_alpha((110.0 * (1.0 - k)) as u8)
        },
    )));
    // The dome's lower rim turns away from the eye, so the last third of the
    // lens falls back into the well. It is also what gives the address a
    // ground that stays dark whatever colour the fixture is.
    pw.add(Shape::mesh(vgradient_mesh(
        Rect::from_min_max(Pos2::new(well.left(), well.top() + wh * 0.62), well.max),
        Rounding::ZERO,
        Color32::TRANSPARENT,
        Color32::from_black_alpha(185),
    )));
    // Light off the bright bottom lip, bounced back into the lower right.
    pw.add(Shape::mesh(radial_mesh(
        Pos2::new(well.right() - ww * 0.10, well.bottom()),
        ww * 0.42,
        Color32::from_white_alpha(78),
        Color32::TRANSPARENT,
    )));
    // The lens's own bottom rim picking that bounce straight back up.
    pw.hline(
        (well.left() + 3.0 * z)..=(well.right() - 3.0 * z),
        well.bottom() - 0.5 * z,
        Stroke::new(hair, Color32::from_white_alpha(46)),
    );
    // What each line actually sits on, followed layer by layer: the lens
    // body at that height, the dome bleeding through it, then the lip's
    // shade over the name and the rim's over the address. Guessing instead
    // of following the layers is how a white fixture loses a line.
    let (name_c, _) = ink(darken(body(0.30).lerp_to_gamma(dome, 0.28), 0.34));
    let (_, addr_c) = ink(darken(body(0.88).lerp_to_gamma(dome, 0.35), 0.50));
    let halo = |c: Color32| {
        if c.r() < 80 {
            Color32::from_white_alpha(110)
        } else {
            Color32::from_black_alpha(180)
        }
    };
    let tw = ww - 8.0 * z;
    shadowed_text(
        &pw,
        well.left_top() + egui::vec2(4.0 * z, 3.0 * z),
        Align2::LEFT_TOP,
        t.name,
        FontId::proportional(10.5 * z),
        name_c,
        halo(name_c),
        tw,
        z,
    );
    shadowed_text(
        &pw,
        well.left_bottom() + egui::vec2(4.0 * z, -3.0 * z),
        Align2::LEFT_BOTTOM,
        t.addr,
        FontId::monospace(8.5 * z),
        addr_c,
        halo(addr_c),
        tw,
        z,
    );

    // ---- the bezel ----
    let k = 0.22 * lv;
    let m_top = mirrored(&M_TOP, t.color, k, sel);
    let m_bot = mirrored(&M_BOTTOM, t.color, k, sel);
    let m_st = mirrored(&M_SIDE_TOP, t.color, k, sel);
    let m_haze = mirrored(&M_SIDE_HAZE, t.color, k, sel);
    let m_gnd = mirrored(&M_SIDE_GROUND, t.color, k, sel);
    let m_low = mirrored(&M_SIDE_LOW, t.color, k, sel);
    let my = rect.center().y;
    let (lo, li) = (Pos2::new(rect.left(), my), Pos2::new(well.left(), my));
    let (ro, ri) = (Pos2::new(rect.right(), my), Pos2::new(well.right(), my));

    let mut frame = egui::Mesh::default();
    let mut run = |o: [Pos2; 2], i: [Pos2; 2], ends: [&Mirror; 2]| bezel_run(&mut frame, o, i, ends);
    run([outer[0], outer[1]], [aper[0], aper[1]], [&m_top, &m_top]);
    run([outer[4], outer[5]], [aper[4], aper[5]], [&m_bot, &m_bot]);
    // Each side rail is broken at the horizon halfway down it.
    run([outer[2], ro], [aper[2], ri], [&m_st, &m_haze]);
    run([ro, outer[3]], [ri, aper[3]], [&m_gnd, &m_low]);
    run([outer[6], lo], [aper[6], li], [&m_low, &m_gnd]);
    run([lo, outer[7]], [li, aper[7]], [&m_haze, &m_st]);
    // The cut corners twist from one rail's room to the next one's, sharing
    // their end vertices exactly so the mitres show no seam.
    run([outer[1], outer[2]], [aper[1], aper[2]], [&m_top, &m_st]);
    run([outer[3], outer[4]], [aper[3], aper[4]], [&m_low, &m_bot]);
    run([outer[5], outer[6]], [aper[5], aper[6]], [&m_bot, &m_low]);
    run([outer[7], outer[0]], [aper[7], aper[0]], [&m_st, &m_top]);
    p.add(Shape::mesh(frame));

    // The extreme outer edge is rolled, so it catches a hard line the whole
    // way round: blown out across the top, the floor's bounce along the
    // bottom, and on the sides only what the walls have, which is why they
    // stay dim and drop a step at the halfway mark. These strokes are
    // anti-aliased and land on the mesh's diagonals, hiding the stair-step
    // the unfeathered chamfers would otherwise show.
    let catch = |c: Color32| Stroke::new(hair, if sel { sel_sky(c) } else { c });
    p.add(Shape::line(
        vec![outer[7], outer[0], outer[1], outer[2]],
        catch(sky(0xFF)),
    ));
    p.add(Shape::line(
        vec![outer[3], outer[4], outer[5], outer[6]],
        catch(ground(0xA8)),
    ));
    p.line_segment([outer[2], ro], catch(sky(0xBE)));
    p.line_segment([ro, outer[3]], catch(ground(0x5A)));
    p.line_segment([outer[6], lo], catch(ground(0x5A)));
    p.line_segment([lo, outer[7]], catch(sky(0xBE)));
    // Where the lip turns away from the room there is nothing left to
    // mirror, so the aperture is cut by a black line all the way round.
    p.add(Shape::closed_line(
        aper.to_vec(),
        Stroke::new(hair, Color32::from_black_alpha(215)),
    ));
    if sel {
        // Inside the aperture, on the lens's own rim, so selection reads as
        // the well being lit blue and not as a ring stuck on the frame.
        let d = 1.0 * z;
        let ring = chamfered(well.shrink(d), (cut - (w + d) * CHAMFER_INSET).max(1.0));
        p.add(Shape::closed_line(
            ring.to_vec(),
            Stroke::new((2.0 * z).max(1.8), SELECT),
        ));
    }
}

/// Light escaping a lit pad into the seam around it: four rounded
/// rectangles stepping outwards, each one well under the last, so the halo
/// has died before it reaches the plate.
fn pad_bleed(p: &egui::Painter, pad: Rect, pad_r: f32, z: f32, color: Color32) {
    for step in (1..=4).rev() {
        let grow = step as f32 * 0.8 * z;
        let k = 1.0 - (step - 1) as f32 * 0.25;
        p.rect_filled(
            pad.expand(grow),
            pad_r + grow,
            color.gamma_multiply(0.34 * k * k),
        );
    }
}

/// A pool of light shaped like the thing it is inside: a disc squashed to
/// the pad's proportions, so the glow reaches the sides and the ends
/// together. Three rings rather than two, so the falloff has a shoulder
/// the way light through rubber does instead of ramping straight off.
/// Unfeathered like the helpers in [`theme`], but it is gone before its own
/// rim, so there is no edge to see.
fn pool_mesh(center: Pos2, rx: f32, ry: f32, inner: Color32) -> egui::Mesh {
    const N: u32 = 20;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, inner);
    for (k, a) in [(0.45_f32, 0.62_f32), (1.0, 0.0)] {
        for i in 0..N {
            let th = i as f32 / N as f32 * 2.0 * PI;
            mesh.colored_vertex(
                center + egui::vec2(th.cos() * rx * k, th.sin() * ry * k),
                inner.gamma_multiply(a),
            );
        }
    }
    for i in 0..N {
        let j = (i + 1) % N;
        mesh.add_triangle(0, 1 + i, 1 + j);
        mesh.add_triangle(1 + i, 1 + N + i, 1 + j);
        mesh.add_triangle(1 + N + i, 1 + N + j, 1 + j);
    }
    mesh
}

/// A backlit silicone pad, the kind a drum machine has: soft rubber sunk
/// into a hole punched in the chassis, a dark seam all the way round it,
/// lit from behind so the light comes through the body instead of off it.
/// The rim is the brightest part, because that is where you are looking
/// through the thin part of the edge. Selecting a fixture presses its pad
/// further into the hole and lights the channel it sits in.
fn pillow(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let on = t.level > 0.02;
    let pressed = t.selected;
    // Unlit silicone is dark grey, not black: it still catches the room.
    let base = if on {
        t.color
    } else {
        Color32::from_rgb(0x2E, 0x31, 0x35)
    };

    // Chassis. The corners stay nearly square on purpose — a plate with
    // holes punched in it, not a row of rounded buttons — so a gridful of
    // these reads as one perforated sheet.
    let plate_r = 2.5 * z;
    p.add(Shape::mesh(vgradient_mesh(
        rect,
        Rounding::same(plate_r),
        Color32::from_rgb(0x3C, 0x41, 0x4A),
        Color32::from_rgb(0x20, 0x23, 0x2A),
    )));
    p.hline(
        (rect.left() + plate_r)..=(rect.right() - plate_r),
        rect.top() + 0.5,
        Stroke::new(1.0, Color32::from_white_alpha(34)),
    );
    p.rect_stroke(
        rect,
        plate_r,
        Stroke::new(1.0, Color32::from_rgb(0x10, 0x12, 0x15)),
    );

    // The hole. Nothing lights the bottom of it, and the lip shades its
    // upper edge hardest, so the seam is darkest above the pad and lifts a
    // little below it — that gradient round the gap is what makes it read
    // as a hole rather than a drawn outline.
    let well = rect.shrink(2.8 * z);
    let well_r = 8.0 * z;
    p.add(Shape::mesh(vgradient_mesh(
        well,
        Rounding::same(well_r),
        Color32::from_rgb(0x02, 0x02, 0x03),
        Color32::from_rgb(0x13, 0x15, 0x19),
    )));
    // The bright edge the cut leaves in the plate.
    p.rect_stroke(
        well.expand(0.5 * z),
        well_r + 0.5 * z,
        Stroke::new(1.0, Color32::from_white_alpha(16)),
    );

    // The pad, a little further down the hole when it is selected.
    let pad_r = 6.5 * z;
    let pad = if pressed {
        rect.shrink(6.4 * z).translate(egui::vec2(0.0, 1.2 * z))
    } else {
        rect.shrink(5.8 * z)
    };
    if on {
        pad_bleed(p, pad, pad_r, z, t.color.gamma_multiply(0.55 + 0.45 * lv));
    }
    if pressed {
        // Selection owns the seam, painted over the colour bleed so a
        // bright fixture cannot hide its own selection, and echoed on the
        // plate edge so it still carries at half zoom.
        p.rect_filled(well, well_r, SELECT.gamma_multiply(0.88));
        p.rect_stroke(
            well.shrink(0.5 * z),
            well_r,
            Stroke::new(1.0, lighten(SELECT, 0.45)),
        );
    }

    // Body, then the light diffusing through it. A pad at a low level is a
    // dark pad — the colour is in there, the light behind it is not — and
    // the face is held well under white even at full, because rubber
    // diffuses and never goes to paper. That cap is also what keeps one
    // ink colour readable through a whole fade.
    let lum = (0.299 * base.r() as f32 + 0.587 * base.g() as f32 + 0.114 * base.b() as f32) / 255.0;
    let face = darken(base, 0.46 + 0.30 * (1.0 - lv));
    let pool = lighten(base, 0.34);
    let pool_a = (0.16 + 0.50 * lv) * (1.0 - 0.35 * lum);
    p.rect_filled(pad, pad_r, face);
    let pc = p.with_clip_rect(pad);
    pc.add(Shape::mesh(pool_mesh(
        pad.center(),
        pad.width() * 0.50,
        pad.height() * 0.56,
        pool.gamma_multiply(pool_a),
    )));

    // Matte modelling: the housing lip overhangs the top of the pad, the
    // underside turns away at the bottom, and the lit middle is the pool
    // above. No gloss crown — a crown would turn the rubber straight back
    // into plastic.
    let lip_r = pad_r - 0.6 * z;
    let lip = Rect::from_min_max(
        pad.min + egui::vec2(0.6 * z, 0.6 * z),
        Pos2::new(pad.right() - 0.6 * z, pad.top() + 0.6 * z + 2.0 * lip_r),
    );
    p.add(Shape::mesh(vgradient_mesh(
        lip,
        top_rounding(lip_r),
        Color32::from_black_alpha(if pressed { 180 } else { 118 }),
        Color32::TRANSPARENT,
    )));
    let foot = Rect::from_min_max(
        Pos2::new(pad.left(), pad.bottom() - pad.height() * 0.5),
        pad.max,
    );
    p.add(Shape::mesh(vgradient_mesh(
        foot,
        bottom_rounding(pad_r),
        Color32::TRANSPARENT,
        Color32::from_black_alpha(48),
    )));

    // The rim. Drawn last of the material, so it also covers every
    // unfeathered mesh edge above. Unlit it is only a faint grey line: a
    // hundred bright rings in a grid of dark fixtures would read as a
    // hundred buttons and throw the whole look away.
    let rim = lighten(base, 0.82);
    let rim_k = if on { 0.34 + 0.30 * lv } else { 0.16 };
    p.rect_stroke(
        pad.shrink(0.5 * z),
        pad_r,
        Stroke::new((1.2 * z).max(1.0), rim.gamma_multiply(rim_k)),
    );
    // ...and pools in the lower rim, where nothing shades it: a bright U
    // round the sides and the base. This is the one cue that says
    // translucent rubber and not a stroked outline, so it is drawn with
    // anti-aliased lines rather than another mesh.
    let hot = lighten(base, 0.92).gamma_multiply(if on { 0.50 + 0.42 * lv } else { 0.14 });
    p.hline(
        (pad.left() + pad_r)..=(pad.right() - pad_r),
        pad.bottom() - 1.1 * z,
        Stroke::new((1.1 * z).max(1.0), hot),
    );
    for x in [pad.left() + 1.1 * z, pad.right() - 1.1 * z] {
        p.vline(
            x,
            (pad.top() + pad.height() * 0.58)..=(pad.bottom() - pad_r),
            Stroke::new((1.0 * z).max(1.0), hot.gamma_multiply(0.75)),
        );
    }

    // The legend, printed on the rubber. The ink is picked from the tone
    // the pad actually reaches at its brightest — face plus pool — not
    // from the raw colour, so it does not flip mid-fade.
    let (txt, dim) = ink(face.lerp_to_gamma(pool, pool_a));
    let sh = if txt == Color32::BLACK {
        Color32::from_white_alpha(70)
    } else {
        Color32::from_black_alpha(150)
    };
    let w = pad.width() - 11.0 * z;
    shadowed_text(
        p,
        pad.left_top() + egui::vec2(5.5 * z, 3.5 * z),
        Align2::LEFT_TOP,
        t.name,
        FontId::proportional(10.5 * z),
        txt,
        sh,
        w,
        z,
    );
    shadowed_text(
        p,
        pad.left_bottom() + egui::vec2(5.5 * z, -2.5 * z),
        Align2::LEFT_BOTTOM,
        t.addr,
        FontId::monospace(9.0 * z),
        dim,
        sh,
        w,
        z,
    );
    if pressed {
        p.rect_stroke(
            rect.shrink(0.8 * z),
            plate_r,
            Stroke::new((1.4 * z).max(1.0), SELECT),
        );
    }
}

/// A soft round bloom: `inner` at the centre, nothing at `radius`.
///
/// Coarser than [`theme::radial_mesh`] — ten sides rather than forty-eight
/// — because one pixel tile carries twenty of these and the rim is
/// transparent, so nobody can count the corners.
fn bloom_mesh(center: Pos2, radius: f32, inner: Color32) -> egui::Mesh {
    const N: u32 = 10;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, inner);
    for i in 0..N {
        let a = i as f32 / N as f32 * 2.0 * PI;
        mesh.colored_vertex(
            center + egui::vec2(a.cos(), a.sin()) * radius,
            Color32::TRANSPARENT,
        );
    }
    for i in 0..N {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % N);
    }
    mesh
}

/// An LED batten: a seven-by-three array of emitters behind a dark
/// diffuser, sunk into a slot milled out of an extruded housing, with the
/// silk-screen printed under it.
///
/// The array is the meter. Level fills it left to right a column at a
/// time — the column the head stops inside takes the remainder — so a wall
/// of these reads like a row of faders and a fixture at 20% is a glance
/// away from one at 80%. A lit emitter burns out towards white and leaves
/// the hue to the bloom around it, which is what separates light from
/// paint; an unlit one is still there as a dark package, which is what
/// keeps the grid legible with the rig out. The colour also scatters
/// faintly through the whole cover, so a rig sitting at 4% shows what
/// colour it is instead of reading as a dead rig.
///
/// Both labels sit on the housing, never on the colour, so they read the
/// same under a white fixture and a black one.
fn pixel(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    const COLS: usize = 7;
    const ROWS: usize = 3;
    // A dead emitter still has a package behind the diffuser: a dark dot,
    // not a hole.
    const OFF: Color32 = Color32::from_rgb(0x18, 0x1B, 0x21);
    // Below theme::WELL. The diffuser has to be the darkest thing on the
    // tile or the slot reads as printed on the housing rather than cut
    // into it.
    const GLASS: Color32 = Color32::from_rgb(0x05, 0x06, 0x08);

    let lv = vis(t.level);
    let lvl = t.level.clamp(0.0, 1.0);
    let on = t.level > 0.02;
    let rr = 3.0 * z;
    let m = 3.0 * z;
    let pad = 4.0 * z;

    // Housing: dark extruded aluminium, lit along its top edge.
    p.add(Shape::mesh(vgradient_mesh(
        rect,
        Rounding::same(rr),
        Color32::from_rgb(0x28, 0x2C, 0x34),
        Color32::from_rgb(0x10, 0x12, 0x16),
    )));
    p.hline(
        (rect.left() + rr)..=(rect.right() - rr),
        rect.top() + 1.5 * z,
        Stroke::new(1.0, Color32::from_white_alpha(24)),
    );
    p.rect_stroke(rect, rr, Stroke::new(1.0, theme::EDGE));

    // The silk-screen is laid out before anything is drawn, because what
    // is left above it is the slot. Name left, address hard right, one
    // line: two stacked rows do not fit under an array three deep, and
    // one row is what a fixture's label strip actually looks like.
    let addr_g = galley(
        p,
        t.addr,
        FontId::monospace(8.5 * z),
        theme::TEXT_DIM,
        rect.width() * 0.5,
    );
    let name_g = galley(
        p,
        t.name,
        FontId::proportional(10.0 * z),
        theme::TEXT,
        rect.width() - 2.0 * pad - addr_g.size().x - 4.0 * z,
    );
    let lab_h = name_g.size().y.max(addr_g.size().y);

    // The slot: square-ended, because it is milled out of the extrusion,
    // and everything between the housing's top margin and the labels.
    let win = Rect::from_min_max(
        rect.left_top() + egui::vec2(m, m),
        Pos2::new(
            rect.right() - m,
            (rect.bottom() - m - lab_h - 1.0 * z).max(rect.top() + m + 6.0 * z),
        ),
    );
    p.rect_filled(win, Rounding::ZERO, GLASS);

    // The array, snapped to whole screen pixels: a dot grid that lands
    // between pixels crawls as the panel is zoomed. The pitch comes from
    // whichever axis runs out first and is floored, never rounded, so the
    // array provably fits the slot at any zoom and any display scale.
    let ppp = p.ctx().pixels_per_point();
    let floor_px = |v: f32| (v * ppp).floor().max(1.0) / ppp;
    let field = win.shrink(2.0 * z);
    let pitch = floor_px((field.width() / COLS as f32).min(field.height() / ROWS as f32));
    let side = floor_px(pitch * 0.58);
    let inset = ((pitch - side) * 0.5 * ppp).floor() / ppp;
    let origin = p.round_pos_to_pixels(
        win.center() - egui::vec2(pitch * COLS as f32, pitch * ROWS as f32) * 0.5,
    );
    let dot = |c: usize, r: usize| {
        Rect::from_min_size(
            origin + egui::vec2(c as f32 * pitch + inset, r as f32 * pitch + inset),
            egui::Vec2::splat(side),
        )
    };
    // How much of column `c` the bar covers. Level is taken raw here, not
    // perceptually: the bar is a measurement and it should use all of its
    // travel. Only the light is scaled by `lv`. The head is measured off
    // the array, not the slot, so the hard edge of the haze always lands
    // on a column boundary.
    let column = |c: usize| (lvl * COLS as f32 - c as f32).clamp(0.0, 1.0);
    let head = origin.x + pitch * COLS as f32 * lvl;

    // Everything that glows is confined to the slot, so no bloom leaks
    // out over the housing.
    let pw = p.with_clip_rect(win);

    if on {
        // Scatter through the whole cover: the colour is readable at any
        // level, while the bar below stays an honest meter.
        pw.rect_filled(
            win,
            Rounding::ZERO,
            t.color.gamma_multiply(0.05 + 0.09 * lv),
        );
        // Haze on the diffuser behind the lit part of the array, falling
        // off just past the head. Its hard edge is the reading.
        let tint = t.color.gamma_multiply(0.10 + 0.14 * lv);
        pw.rect_filled(
            Rect::from_min_max(win.min, Pos2::new(head.min(win.right()), win.bottom())),
            Rounding::ZERO,
            tint,
        );
        let fade = 6.0 * z;
        let tail = (head + fade).min(win.right());
        if tail > head {
            let quad = [
                Pos2::new(head, win.top()),
                Pos2::new(tail, win.top()),
                Pos2::new(tail, win.bottom()),
                Pos2::new(head, win.bottom()),
            ];
            // Axis-aligned, and it fades to nothing, so the unfeathered
            // mesh has no silhouette to show its steps.
            pw.add(Shape::mesh(convex_mesh(&quad, |q| {
                tint.gamma_multiply(1.0 - ((q.x - head) / fade).clamp(0.0, 1.0))
            })));
        }
        // Blooms in a pass of their own, so no emitter ends up buried
        // under its neighbour's haze. They are meant to overlap: discrete
        // cores inside one continuous glow is the signature of a lit
        // array, where isolated bright squares are a checkerboard.
        for c in 0..COLS {
            let k = column(c);
            if k <= 0.0 {
                continue;
            }
            let glow = t.color.gamma_multiply((0.34 + 0.26 * lv) * k);
            for r in 0..ROWS {
                pw.add(Shape::mesh(bloom_mesh(
                    dot(c, r).center(),
                    pitch * 0.85,
                    glow,
                )));
            }
        }
    }

    // The emitters. A lit one burns towards white at its face and leaves
    // the hue to the bloom around it — a core that stays the fixture's
    // colour reads as a painted square, and a deep blue one reads as
    // nearly off.
    let hot = lighten(t.color, 0.26 + 0.44 * lv);
    for c in 0..COLS {
        let face = OFF.lerp_to_gamma(hot, column(c));
        for r in 0..ROWS {
            pw.rect_filled(dot(c, r), Rounding::ZERO, face);
        }
    }

    // What makes the slot read as cut rather than printed: the housing
    // lip shades the top of the recess, the bottom lip picks the light
    // back up, and a hairline on the housing just above catches the
    // milled edge. Three gradients and a line, no drawn outlines.
    pw.add(Shape::mesh(vgradient_mesh(
        Rect::from_min_max(win.min, Pos2::new(win.right(), win.top() + 4.0 * z)),
        Rounding::ZERO,
        Color32::from_black_alpha(130),
        Color32::TRANSPARENT,
    )));
    pw.add(Shape::mesh(vgradient_mesh(
        Rect::from_min_max(Pos2::new(win.left(), win.bottom() - 4.0 * z), win.max),
        Rounding::ZERO,
        Color32::TRANSPARENT,
        Color32::from_white_alpha(14),
    )));
    p.hline(
        win.x_range(),
        win.top() - 1.0 * z,
        Stroke::new(1.0, Color32::from_white_alpha(26)),
    );
    p.rect_stroke(
        win,
        Rounding::ZERO,
        Stroke::new(
            1.0,
            if t.selected {
                SELECT
            } else {
                Color32::from_black_alpha(215)
            },
        ),
    );

    // Silk-screen, on the housing where no dot can fight it.
    let ly = rect.bottom() - m - lab_h;
    p.galley(
        Pos2::new(rect.left() + pad, ly + (lab_h - name_g.size().y) * 0.5),
        name_g,
        theme::TEXT,
    );
    p.galley(
        Pos2::new(
            rect.right() - pad - addr_g.size().x,
            ly + (lab_h - addr_g.size().y) * 0.5,
        ),
        addr_g,
        theme::TEXT_DIM,
    );
    select_ring(p, rect, rr, z, t);
}

/// The shadow the frame's folded lip drops onto the gel it grips.
///
/// The top lip is the one the light comes over, so it gets two ramps: a
/// short hard one for the lip itself and a long soft wash under it. Two
/// straight ramps laid over each other have no visible end, which one does
/// — and the wash doubles as the dark bed the fixture's name is written on.
fn lip_shadow(p: &egui::Painter, film: Rect, reach: f32, wash: f32, top: u8, rest: u8) {
    for (depth, alpha) in [(reach * 1.6, top), (wash, (top as f32 * 0.55) as u8)] {
        p.add(Shape::mesh(vgradient_mesh(
            Rect::from_min_max(film.min, Pos2::new(film.right(), film.top() + depth)),
            Rounding::ZERO,
            Color32::from_black_alpha(alpha),
            Color32::TRANSPARENT,
        )));
    }
    p.add(Shape::mesh(vgradient_mesh(
        Rect::from_min_max(Pos2::new(film.left(), film.bottom() - reach), film.max),
        Rounding::ZERO,
        Color32::TRANSPARENT,
        Color32::from_black_alpha(rest),
    )));
    // The sides want a horizontal fade, which is a four-corner mesh rather
    // than a `vgradient_mesh`. Both quads are axis-aligned, so the
    // unfeathered edges land on whole columns.
    for (a, b, from_left) in [
        (film.left(), film.left() + reach, true),
        (film.right() - reach, film.right(), false),
    ] {
        let quad = [
            Pos2::new(a, film.top()),
            Pos2::new(b, film.top()),
            Pos2::new(b, film.bottom()),
            Pos2::new(a, film.bottom()),
        ];
        p.add(Shape::mesh(convex_mesh(&quad, |q| {
            let k = ((q.x - a) / (b - a).max(0.5)).clamp(0.0, 1.0);
            let k = if from_left { 1.0 - k } else { k };
            Color32::from_black_alpha((rest as f32 * k) as u8)
        })));
    }
}

/// A sheet of colour gel in a black steel frame, the way it rides in the
/// runners on the front of a fresnel.
///
/// The lamp is behind the film, so the colour lies thickest against the
/// frame and thins towards the middle, where a small filament core burns
/// through; the lit sheet throws that colour back onto the steel and leaks
/// a line of it round its own edge. The frame is one folded sheet — a thin
/// border, a lip laid back over the gel with its shadow on it, a pull tab
/// standing above the top edge and a punched foot to write the number on.
/// And the gel is slack in its frame: it bows, and one corner has curled
/// away from the heat.
fn gel(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let on = t.level > 0.02;
    let r = 2.0 * z;
    // One folded sheet of steel. The border is deliberately thin — a gel
    // frame is a rim, not a chassis, and the gel is what the grid is for.
    let rise = 6.0 * z;
    let rail = 4.5 * z;
    let foot = 10.5 * z;
    let lip = 1.5 * z;

    let body = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + rise), rect.max);
    let win = Rect::from_min_max(
        Pos2::new(body.left() + rail, body.top() + rail),
        Pos2::new(body.right() - rail, body.bottom() - foot),
    );
    // The gel carries on under the frame, so it is painted wider than the
    // window and the lip is laid back over its edges.
    let film = win.expand(lip);
    let tab = Rect::from_min_max(
        Pos2::new(rect.left() + 9.0 * z, rect.top()),
        Pos2::new(rect.left() + 27.0 * z, body.top() + r),
    );

    let h = seed(t);
    let u = |s: u32| ((h >> s) & 0xFF) as f32 / 255.0;
    let base = if on {
        t.color
    } else {
        Color32::from_rgb(0x1E, 0x21, 0x25)
    };
    let lamp = Pos2::new(
        film.center().x + (u(0) - 0.5) * film.width() * 0.12,
        film.top() + film.height() * 0.54,
    );

    // The tab goes down first; the face then covers where it folds under.
    // It stands clear of everything else, so it takes the most light.
    p.add(Shape::mesh(vgradient_mesh(
        tab,
        top_rounding(2.0 * z),
        Color32::from_rgb(0x3C, 0x3A, 0x37),
        Color32::from_rgb(0x1E, 0x1D, 0x1C),
    )));
    p.rect_stroke(
        tab,
        top_rounding(2.0 * z),
        Stroke::new(1.0, Color32::from_rgb(0x4C, 0x49, 0x44)),
    );
    // Matte powder coat: barely any gradient across it, only enough to say
    // where the light is.
    p.add(Shape::mesh(vgradient_mesh(
        body,
        Rounding::same(r),
        Color32::from_rgb(0x24, 0x23, 0x22),
        Color32::from_rgb(0x11, 0x11, 0x12),
    )));
    if on {
        // A lit gel washes its own frame. This is most of what separates a
        // framed sheet from a screen in a bezel, so it is not subtle.
        let steel = p.with_clip_rect(body.shrink(1.0 * z));
        steel.add(Shape::mesh(radial_mesh(
            lamp,
            film.width() * 0.88,
            t.color.gamma_multiply(0.40 * lv),
            Color32::TRANSPARENT,
        )));
    }

    // ---- the film ----
    let pf = p.with_clip_rect(film);
    // Dye at full thickness: what the sheet looks like away from the lamp.
    pf.rect_filled(
        film,
        Rounding::ZERO,
        darken(base, if on { 0.54 } else { 0.30 }),
    );
    if on {
        // Light coming through. The pool keeps the hue and only gains
        // value — a gel reads more saturated lit than unlit — and how far
        // it spreads is the level.
        pf.add(Shape::mesh(radial_mesh(
            lamp,
            film.width() * (0.66 + 0.30 * lv),
            base.gamma_multiply(0.30 + 0.48 * lv),
            Color32::TRANSPARENT,
        )));
        pf.add(Shape::mesh(radial_mesh(
            lamp,
            film.height() * (0.48 + 0.34 * lv),
            lighten(base, 0.08 + 0.22 * lv).gamma_multiply(0.22 + 0.42 * lv),
            Color32::TRANSPARENT,
        )));
        // The filament itself, burning a small hole in the colour. Kept
        // tight, so a pale gel keeps its hue everywhere but the middle.
        pf.add(Shape::mesh(radial_mesh(
            lamp,
            film.height() * 0.15,
            Color32::from_white_alpha((46.0 * lv * lv * lv) as u8),
            Color32::TRANSPARENT,
        )));
    }
    // A few hundred hours in, the dye has bleached where the lamp sat. It
    // does not come back, so it is there when the lamp is out too.
    pf.add(Shape::mesh(radial_mesh(
        lamp + egui::vec2((u(4) - 0.5) * 6.0 * z, (u(12) - 0.5) * 4.0 * z),
        film.height() * 0.30,
        Color32::from_rgba_unmultiplied(255, 246, 232, if on { 26 } else { 16 }),
        Color32::TRANSPARENT,
    )));
    // Room light on the face of the sheet — most of what an unlit gel
    // shows, and the thing that says polyester rather than paint.
    pf.add(Shape::mesh(vgradient_mesh(
        Rect::from_min_max(
            film.min,
            Pos2::new(film.right(), film.top() + film.height() * 0.62),
        ),
        Rounding::ZERO,
        Color32::from_white_alpha(if on { 9 } else { 15 }),
        Color32::TRANSPARENT,
    )));
    // The slack in the sheet: a lit bow with its shadow under it, both wide
    // and faint. Strokes, so they can run off the horizontal without
    // stepping — and wide enough that they can never read as a scratch.
    // The alphas are small because the blend is linear: a little white on
    // an unlit sheet goes a long way.
    let bow = film.top() + film.height() * (0.36 + 0.24 * u(16));
    let tilt = (u(8) - 0.5) * 6.0 * z;
    let over = 3.0 * z;
    pf.line_segment(
        [
            Pos2::new(film.left() - over, bow + tilt),
            Pos2::new(film.right() + over, bow - tilt),
        ],
        Stroke::new(6.0 * z, Color32::from_white_alpha(if on { 8 } else { 6 })),
    );
    pf.line_segment(
        [
            Pos2::new(film.left() - over, bow + 5.0 * z + tilt * 0.6),
            Pos2::new(film.right() + over, bow + 5.0 * z - tilt * 0.6),
        ],
        Stroke::new(7.0 * z, Color32::from_black_alpha(if on { 13 } else { 9 })),
    );

    // ---- back to the steel ----
    lip_shadow(p, film, 3.4 * z, 14.0 * z, 145, 105);
    // One corner has curled off the frame. A triangle of sheet seen edge
    // on, the fold catching the light and dropping its own shadow: the
    // cheapest thing on the tile that says "this is a sheet of something",
    // and the only diagonal here — a path, so it is feathered. It goes on
    // after the lip shadow, which would otherwise bury it.
    let curl = (7.0 + 3.5 * u(24)) * z;
    let pc = p.with_clip_rect(film);
    let br = Pos2::new(film.right(), film.bottom());
    let c0 = Pos2::new(br.x - curl, br.y);
    let c1 = Pos2::new(br.x, br.y - curl);
    pc.add(Shape::convex_polygon(
        vec![c0, br, c1],
        lighten(base, 0.40).gamma_multiply(if on { 0.62 } else { 0.45 }),
        Stroke::NONE,
    ));
    pc.line_segment(
        [c0 + egui::vec2(-1.2 * z, 0.0), c1 + egui::vec2(0.0, -1.2 * z)],
        Stroke::new(1.6 * z, Color32::from_black_alpha(120)),
    );
    pc.line_segment([c0, c1], Stroke::new(1.0 * z, Color32::from_white_alpha(70)));
    // The lip, folded back across the gel's edges: a flat band over the
    // outer 1.5 px of the film, a hard fold line where it turns away from
    // the face, a lit step along its top and a dark one where it lets go
    // of the sheet.
    p.rect_stroke(
        win,
        Rounding::ZERO,
        Stroke::new(2.0 * lip, Color32::from_rgb(0x26, 0x25, 0x23)),
    );
    p.rect_stroke(
        film,
        Rounding::ZERO,
        Stroke::new(1.0, Color32::from_black_alpha(175)),
    );
    p.rect_stroke(
        win.shrink(lip),
        Rounding::ZERO,
        Stroke::new(1.0, Color32::from_black_alpha(95)),
    );
    p.hline(
        film.x_range(),
        film.top() + 0.5,
        Stroke::new(1.0, Color32::from_white_alpha(26)),
    );
    // Where the folds lap at the corners. One piece of steel cannot turn a
    // corner, so the uprights run over the rails and leave a step — which
    // no bezel has, and which is most of what stops this reading as a
    // screen in a frame.
    let lap_dark = Stroke::new(1.0, Color32::from_black_alpha(140));
    let lap_lit = Stroke::new(1.0, Color32::from_white_alpha(20));
    for x in [win.left(), win.right()] {
        for (y0, y1) in [
            (film.top(), film.top() + 2.0 * lip),
            (film.bottom() - 2.0 * lip, film.bottom()),
        ] {
            p.vline(x - lip, y0..=y1, lap_dark);
            p.vline(x + lip, y0..=y1, lap_lit);
        }
    }
    if on {
        // Light getting past the edge of the sheet, between gel and steel.
        p.rect_stroke(
            film.expand(1.0 * z),
            Rounding::ZERO,
            Stroke::new(1.0 * z, t.color.gamma_multiply(0.42 * lv)),
        );
    }

    // Lit top edge of the face, dark where the tab folds under it, and the
    // shadow the whole frame sits in along the bottom.
    let top_lit = Stroke::new(1.0, Color32::from_white_alpha(22));
    p.hline((body.left() + r)..=(tab.left() - 0.5), body.top() + 0.5, top_lit);
    p.hline((tab.right() + 0.5)..=(body.right() - r), body.top() + 0.5, top_lit);
    p.hline(
        (tab.left() - 0.5)..=(tab.right() + 0.5),
        body.top() + 0.5,
        Stroke::new(1.0, Color32::from_black_alpha(160)),
    );
    p.hline(
        (body.left() + r)..=(body.right() - r),
        body.bottom() - 0.5,
        Stroke::new(1.0, Color32::from_black_alpha(160)),
    );
    p.rect_stroke(
        body,
        Rounding::same(r),
        Stroke::new(1.0, Color32::from_rgb(0x2E, 0x2D, 0x2C)),
    );

    // Punched hole in the foot, lit along its lower lip.
    let foot_y = (win.bottom() + body.bottom()) * 0.5;
    let hole = Pos2::new(body.right() - 5.5 * z, foot_y);
    p.circle_filled(
        hole + egui::vec2(0.0, 0.7 * z),
        2.4 * z,
        Color32::from_white_alpha(40),
    );
    p.circle_filled(hole, 2.4 * z, Color32::from_rgb(0x08, 0x08, 0x09));

    // The name goes on the gel, in the bed of shadow the top lip drops —
    // that shadow is there on every colour, so the label reads on a white
    // fixture as well as a near-black one. The address goes on the foot in
    // silver pen, the way the gel number is written on a real frame.
    shadowed_text(
        p,
        Pos2::new(win.left() + 2.0 * z, win.top() + 1.0 * z),
        Align2::LEFT_TOP,
        t.name,
        FontId::proportional(10.0 * z),
        Color32::WHITE,
        Color32::from_black_alpha(190),
        win.width() - 4.0 * z,
        z,
    );
    text(
        p,
        Pos2::new(body.left() + 5.0 * z, foot_y),
        Align2::LEFT_CENTER,
        t.addr,
        FontId::monospace(8.5 * z),
        Color32::from_rgb(0xC6, 0xC2, 0xBA),
        body.width() - 18.0 * z,
    );

    if t.selected {
        // The tab is the one part standing clear of the frame: flood it, so
        // a picked gel reads down the grid even where tiles are packed.
        let above = p.with_clip_rect(Rect::from_min_max(
            rect.min,
            Pos2::new(rect.right(), body.top() + 1.0),
        ));
        above.rect_filled(tab, top_rounding(2.0 * z), SELECT);
    }
    select_ring(p, body, r, z, t);
}

/// The four corners of the tape strip. Nothing about it is parallel: a
/// hand puts tape on crooked and pulls one end tighter than the other.
struct Strip {
    tl: Pos2,
    tr: Pos2,
    bl: Pos2,
    br: Pos2,
}

impl Strip {
    /// A point on the strip: `u` runs 0..1 from the left tear across to
    /// the right one, `v` 0..1 from the top edge down.
    fn at(&self, u: f32, v: f32) -> Pos2 {
        let top = self.tl + (self.tr - self.tl) * u;
        let bot = self.bl + (self.br - self.bl) * u;
        top + (bot - top) * v
    }
}

/// One 0..1 draw from `seed`, picked by `k`.
fn wobble(seed: u32, k: u32) -> f32 {
    let mut h = seed ^ k.wrapping_mul(0x9E37_79B9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    ((h >> 8) & 0xFFFF) as f32 / 65_535.0
}

/// The same draw, centred on zero: −1..1.
fn wobble2(seed: u32, k: u32) -> f32 {
    wobble(seed, k) * 2.0 - 1.0
}

/// One line written on the tape, turned to sit with the strip.
///
/// The galley goes down twice — a dim pass a hair up and left of the wet
/// one — so the stroke comes out fatter and less even than the font draws
/// it. A pen is not a typeface and should not look like one. Under about
/// a 8 px face the letters are too small to carry the trick, so the bleed
/// drops out rather than turning the word to mush; the caller sends
/// `angle` to zero at the same point.
fn marker(
    p: &egui::Painter,
    pos: Pos2,
    s: &str,
    font: FontId,
    ink: Color32,
    max_w: f32,
    angle: f32,
) {
    let bleed = font.size * 0.05;
    let g = galley(p, s, font, ink, max_w);
    if bleed > 0.4 {
        p.add(
            egui::epaint::TextShape::new(pos - egui::vec2(bleed, bleed * 0.7), g.clone(), ink)
                .with_override_text_color(ink.gamma_multiply(0.42))
                .with_angle(angle),
        );
    }
    p.add(egui::epaint::TextShape::new(pos, g, ink).with_angle(angle));
}

/// A strip of gaffer tape with the name on it in silver marker, stuck
/// across the fixture's own face so the live colour glows out above and
/// below it: hand-torn ends, a corner lifting, and a contact shadow.
///
/// Gaffer is the one material in the panel that never catches a highlight,
/// so it is carried by what still reads across a dark room — the scrim
/// grain running the length of the strip, the ragged ends, the hard little
/// shadow under it, and the fact that nothing on the tile lines up with
/// the tile. Where the strip landed is hashed from the address, so every
/// fixture is crooked in its own way and stays that way on every frame.
fn tape(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let on = t.level > 0.02;
    let seed = seed(t);
    let r = 2.5 * z;

    // The fixture's face, lit from inside. The hot spot is a disc, so its
    // unfeathered rim never steps, and it is keyed to the short side so it
    // dies inside the tile instead of being cut off square at the top.
    p.rect_filled(rect, r, Color32::from_rgb(0x0A, 0x0B, 0x0D));
    p.rect_filled(rect, r, t.color.gamma_multiply(0.18 + 0.44 * lv));
    if on {
        p.add(Shape::mesh(radial_mesh(
            rect.center(),
            rect.height() * 0.46,
            t.color.gamma_multiply(0.42 * lv),
            Color32::TRANSPARENT,
        )));
    }
    let foot = Rect::from_min_max(
        Pos2::new(rect.left(), rect.bottom() - rect.height() * 0.28),
        rect.max,
    );
    p.add(Shape::mesh(vgradient_mesh(
        foot,
        bottom_rounding(r),
        Color32::TRANSPARENT,
        Color32::from_black_alpha(72),
    )));
    p.rect_stroke(
        rect,
        r,
        Stroke::new(
            1.0,
            if on {
                theme::EDGE.lerp_to_gamma(t.color, 0.40 * lv)
            } else {
                theme::EDGE
            },
        ),
    );
    p.hline(
        (rect.left() + r)..=(rect.right() - r),
        rect.top() + 1.0,
        Stroke::new(1.0, Color32::from_white_alpha(20)),
    );

    // Where the strip landed: a degree or so of tilt, the whole thing a
    // touch high or low, one end pulled tighter than the other.
    let tilt = wobble2(seed, 1) * 0.022;
    let drop = wobble2(seed, 2) * 0.8 * z;
    let flare = wobble2(seed, 3) * 0.7 * z;
    let x0 = rect.left() + 3.5 * z;
    let x1 = rect.right() - 3.5 * z;
    let y0 = rect.top() + 8.0 * z + drop;
    let y1 = rect.bottom() - 8.0 * z + drop;
    let span = (x1 - x0).max(1.0);
    let hs = (y1 - y0).max(1.0);
    let pivot = Pos2::new((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    let (sa, ca) = tilt.sin_cos();
    let spin = |q: Pos2| {
        let d = q - pivot;
        pivot + egui::vec2(d.x * ca - d.y * sa, d.x * sa + d.y * ca)
    };
    let strip = Strip {
        tl: spin(Pos2::new(x0, y0 + flare)),
        tr: spin(Pos2::new(x1, y0 - flare)),
        bl: spin(Pos2::new(x0, y1 - 0.4 * flare)),
        br: spin(Pos2::new(x1, y1 + 0.4 * flare)),
    };

    // Both ends were torn, not cut, and no hand tears the two the same:
    // one comes off short, one long, both meander instead of zigzagging —
    // each boundary point is the smoothed average of three draws — and
    // each has one band where the scrim let go and the tear bit deeper.
    const BANDS: usize = 9;
    let lmid = (2.0 + 1.1 * wobble(seed, 20)) * z / span;
    let rmid = (2.0 + 1.1 * wobble(seed, 21)) * z / span;
    let amp = 1.6 * z / span;
    let bite = 2.6 * z / span;
    let nick = (seed as usize % BANDS, (seed >> 9) as usize % BANDS);
    let draw = |k: u32, i: i32| wobble2(seed, k + i.clamp(0, BANDS as i32) as u32);
    // Weighted so the smoothed run still covers the whole −1..1, or the
    // averaging quietly eats half the tear.
    let smooth = |k: u32, j: i32| (0.3 * draw(k, j - 1) + draw(k, j) + 0.3 * draw(k, j + 1)) / 1.6;
    let mut cut = [(0.0f32, 0.0f32); BANDS + 1];
    for (i, c) in cut.iter_mut().enumerate() {
        let j = i as i32;
        *c = (
            lmid + smooth(60, j) * amp + if i == nick.0 { bite } else { 0.0 },
            1.0 - rmid - smooth(90, j) * amp - if i == nick.1 { bite } else { 0.0 },
        );
    }

    // Gaffer is thin, so its shadow is tight, and it falls where the light
    // does not reach: a hard seam along the bottom edge, next to nothing
    // above it, and a pool under the corner that has lifted.
    let low = wobble(seed, 4) > 0.5;
    let fold = (4.4 + 2.2 * wobble(seed, 5)) * z;
    for (gx, up, down, a) in [
        (2.4 * z, 0.4 * z, 3.0 * z, 52u8),
        (1.0 * z, 0.2 * z, 1.3 * z, 96),
    ] {
        p.add(Shape::convex_polygon(
            vec![
                strip.tl + egui::vec2(-gx, -up),
                strip.tr + egui::vec2(gx, -up),
                strip.br + egui::vec2(gx, down),
                strip.bl + egui::vec2(-gx, down),
            ],
            Color32::from_black_alpha(a),
            Stroke::NONE,
        ));
    }
    p.add(Shape::mesh(radial_mesh(
        strip.at(1.0 - 1.4 * z / span, if low { 1.0 } else { 0.0 }),
        3.6 * z,
        Color32::from_black_alpha(110),
        Color32::TRANSPARENT,
    )));

    // The cloth. Nothing on a matte surface shines, so all it has is the
    // fall — bright at the lit top edge, half that at the bottom — and the
    // scrim: nine warp bands with a wobble in the grey between them, wide
    // enough apart to be seen, and the face's own colour bouncing back up
    // onto the strip at both edges. Each band is a polygon, not a mesh, so
    // the tilted silhouette stays smooth, and it overruns the next one so
    // no seam of the face shows through between them.
    let over = 1.3 * z / hs;
    for i in 0..BANDS {
        let v0 = i as f32 / BANDS as f32;
        let v1 = (i + 1) as f32 / BANDS as f32 + if i + 1 < BANDS { over } else { 0.0 };
        let vm = (i as f32 + 0.5) / BANDS as f32;
        let g = 48.0 - 24.0 * vm + wobble2(seed, 30 + i as u32) * 5.0;
        let bounce = (vm * 2.0 - 1.0).abs();
        let cloth = Color32::from_rgb(g as u8, (g - 1.0).max(0.0) as u8, (g - 5.0).max(0.0) as u8);
        p.add(Shape::convex_polygon(
            vec![
                strip.at(cut[i].0, v0),
                strip.at(cut[i].1, v0),
                strip.at(cut[i + 1].1, v1),
                strip.at(cut[i + 1].0, v1),
            ],
            cloth.lerp_to_gamma(t.color, 0.22 * lv * bounce * bounce),
            Stroke::NONE,
        ));
    }
    // The tape has thickness. Its top edge catches the room and its bottom
    // is pressed into its own shadow — that pair, more than anything else
    // here, is what says the strip is lying on the face and not printed
    // into it.
    p.line_segment(
        [
            strip.at(cut[0].0 + 0.02, 0.05),
            strip.at(cut[0].1 - 0.02, 0.05),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(26)),
    );
    p.line_segment(
        [
            strip.at(cut[BANDS].0 + 0.02, 0.95),
            strip.at(cut[BANDS].1 - 0.02, 0.95),
        ],
        Stroke::new(1.0, Color32::from_black_alpha(90)),
    );
    // A few threads of the scrim pulled loose where it was torn.
    let thread = Color32::from_rgba_unmultiplied(148, 142, 129, 170);
    for i in 0..3u32 {
        let v = 0.2 + 0.3 * i as f32 + wobble2(seed, 150 + i) * 0.06;
        let b = ((v * BANDS as f32) as usize).min(BANDS);
        let out = (1.0 + 1.4 * wobble(seed, 160 + i)) * z / span;
        let pair = if i % 2 == 0 {
            [strip.at(cut[b].0, v), strip.at(cut[b].0 - out, v + 0.02)]
        } else {
            [strip.at(cut[b].1, v), strip.at(cut[b].1 + out, v - 0.02)]
        };
        p.line_segment(pair, Stroke::new(1.0, thread));
    }

    // One corner has lifted and curled back, showing the dull adhesive
    // side. The tip is tilted up into the light and the crease folds away
    // from it, so the shading runs pale at the tip to dark at the fold —
    // the other way round and it reads as a chip out of the corner. The
    // crease bows outward, because a curl is not a cut corner. The
    // silhouette is a polygon and the shading a mesh inset inside it, so
    // the diagonal stays smooth.
    let du = fold / span;
    let dv = fold / hs;
    let ci = if low { BANDS } else { 0 };
    let (vc, sv) = if low { (1.0, -dv) } else { (0.0, dv) };
    let ue = cut[ci].1;
    let tip = strip.at(ue, vc);
    let curl = [
        tip,
        strip.at(ue - du, vc),
        strip.at(ue - 0.60 * du, vc + 0.60 * sv),
        strip.at(ue, vc + sv),
    ];
    let cast = egui::vec2(-1.0 * z, 1.8 * z);
    p.add(Shape::convex_polygon(
        curl.iter().map(|q| *q + cast).collect(),
        Color32::from_black_alpha(120),
        Stroke::NONE,
    ));
    p.add(Shape::convex_polygon(
        curl.to_vec(),
        Color32::from_rgb(0x3A, 0x36, 0x2F),
        Stroke::NONE,
    ));
    let cen = tip + ((curl[1] - tip) + (curl[2] - tip) + (curl[3] - tip)) * 0.25;
    let inner: Vec<Pos2> = curl
        .iter()
        .map(|q| *q + (cen - *q).normalized() * (0.9 * z))
        .collect();
    p.add(Shape::mesh(convex_mesh(&inner, |q| {
        let k = ((q - tip).length() / fold.max(1.0)).clamp(0.0, 1.0);
        Color32::from_rgb(0x8A, 0x82, 0x72).lerp_to_gamma(Color32::from_rgb(0x2C, 0x29, 0x24), k)
    })));
    p.add(Shape::line(
        vec![curl[1], curl[2], curl[3]],
        Stroke::new(1.0, Color32::from_black_alpha(150)),
    ));

    // Silver marker on black cloth: not white, not opaque, and fat — the
    // console's heaviest weight is the nearest thing it owns to a paint
    // pen. Neither line is square to the tape, and the tape is not square
    // to the tile. Below about half zoom the glyphs are too small to
    // survive being turned, so the writing goes back square and only the
    // tape stays crooked. Each line keeps clear of the curl, but only on
    // the line the curl is actually on.
    let ink = if t.selected {
        Color32::from_rgba_unmultiplied(238, 246, 252, 250)
    } else {
        Color32::from_rgba_unmultiplied(208, 205, 194, 236)
    };
    let nf = FontId::new(9.6 * z, theme::semibold());
    let af = FontId::new(8.0 * z, theme::medium());
    let lead = p.fonts(|f| f.row_height(&nf));
    let turn = if z < 0.7 { 0.0 } else { 1.0 };
    let pad = 7.0 * z / span;
    let curl_clear = |mine: bool| span - 15.0 * z - if mine { fold } else { 0.0 };
    marker(
        p,
        strip.at(pad, (2.6 * z + wobble2(seed, 7) * 0.5 * z) / hs),
        t.name,
        nf,
        ink,
        curl_clear(!low),
        turn * (tilt + wobble2(seed, 8) * 0.018),
    );
    marker(
        p,
        strip.at(
            pad + 1.6 * z * wobble(seed, 9) / span,
            (2.8 * z + lead + wobble2(seed, 10) * 0.5 * z) / hs,
        ),
        t.addr,
        af,
        ink.gamma_multiply(0.82),
        curl_clear(low),
        turn * (tilt + wobble2(seed, 11) * 0.026),
    );

    select_ring(p, rect, r, z, t);
}

/// A capsule of glow lying along the tube run `x0..=x1` at `cy`: `inner` on
/// the run itself, gone `rx` past each end and `ry` above and below.
///
/// The two reaches are separate because a tile is wider than it is tall and
/// the wash has to stay on the board — it spreads up and down and stops
/// well short of the sides. This is the long faint tail only; the near
/// field is stroked along the tube, where it can follow the bends. The
/// outer ring is fully transparent, so the mesh's unfeathered silhouette
/// never shows.
fn tube_glow(x0: f32, x1: f32, cy: f32, rx: f32, ry: f32, inner: Color32) -> egui::Mesh {
    const N: u32 = 24;
    let mut mesh = egui::Mesh::default();
    // Closes by repeating the first pair at a = 2π rather than wrapping.
    for i in 0..=N {
        let a = i as f32 / N as f32 * 2.0 * PI;
        let (dx, dy) = (a.cos(), a.sin());
        // Every point on the outside answers to the nearer end of the run,
        // which is what makes this a capsule and not a disc.
        let cx = if dx >= 0.0 { x1 } else { x0 };
        mesh.colored_vertex(Pos2::new(cx, cy), inner);
        mesh.colored_vertex(Pos2::new(cx + dx * rx, cy + dy * ry), Color32::TRANSPARENT);
    }
    for i in 0..N {
        mesh.add_triangle(2 * i, 2 * i + 1, 2 * i + 3);
        mesh.add_triangle(2 * i, 2 * i + 3, 2 * i + 2);
    }
    mesh
}

/// The centre line of a staple of tube: a run from `x0` to `x1` at height
/// `y` that turns down through a bend of radius `r` at each end and stops
/// `drop` below the run, where it dives into the board.
///
/// The bends are real arcs, five segments each. A mitred right angle under
/// a thick stroke reads as a folded strip, and bent glass has no corners —
/// the bend is the one cue that separates a neon tube from a lit bar, so it
/// is worth the ten extra points. Keep every stroke drawn along this
/// narrower than `2 * r`, or its inner edge folds back on itself at the
/// bend and blends over itself.
fn staple(x0: f32, x1: f32, y: f32, drop: f32, r: f32) -> Vec<Pos2> {
    const ARC: usize = 5;
    let mut pts = Vec::with_capacity(2 * ARC + 4);
    pts.push(Pos2::new(x0, y + drop));
    // Left quarter turn from straight-down to along-the-run, then the right
    // one back to straight-down. The straight run is just the two points
    // where those arcs end.
    for (cx, a0) in [(x0 + r, PI), (x1 - r, 1.5 * PI)] {
        for i in 0..=ARC {
            let a = a0 + i as f32 / ARC as f32 * (0.5 * PI);
            pts.push(Pos2::new(cx + r * a.cos(), y + r + r * a.sin()));
        }
    }
    pts.push(Pos2::new(x1, y + drop));
    pts
}

/// A neon tube bent into a staple on a dark board: cold grey glass while
/// the fixture is out, and once it strikes, a saturated run with a
/// blown-out centre line sitting in its own wash.
///
/// The board never takes the colour, so the name and the address are
/// light-on-dark whatever the fixture is doing — the one look in the set
/// where a white lamp and a black one read the same.
fn neon(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let on = t.level > 0.02;
    let rr = 3.0 * z;

    // Matt dark ply, lit faintly along its top edge.
    p.add(Shape::mesh(vgradient_mesh(
        rect,
        Rounding::same(rr),
        Color32::from_rgb(0x1A, 0x1B, 0x1F),
        Color32::from_rgb(0x0B, 0x0C, 0x0E),
    )));
    p.hline(
        (rect.left() + rr)..=(rect.right() - rr),
        rect.top() + 1.5,
        Stroke::new(1.0, Color32::from_white_alpha(14)),
    );

    // One staple of tube, low on the board, both ends diving into it. Every
    // pass over the glass is a stroke along the same centre line, so the
    // glow bends with the glass and nothing needs an unfeathered mesh.
    let run_y = rect.top() + 32.0 * z;
    let bend = 5.0 * z;
    let drop = 9.0 * z;
    let (x0, x1) = (rect.left() + 10.0 * z, rect.right() - 10.0 * z);
    let path = staple(x0, x1, run_y, drop, bend);
    let tube = |w: f32, c: Color32| {
        p.add(Shape::line(path.clone(), Stroke::new(w, c)));
    };
    // An offset curve of a staple is just a staple grown by `d`: the arc
    // centres do not move, so every radius and every edge steps out
    // together. That buys the highlight along the lit side of the glass and
    // the shade under its belly for one path each, and both of them follow
    // the bends instead of stopping at them.
    let offset = |d: f32| staple(x0 - d, x1 + d, run_y - d, drop + d, bend + d);

    // The shadow the glass throws on the board, down and to the right of
    // it. A bright tube washes its own shadow out, which is what it should
    // do; a cold one keeps it and stops floating.
    p.add(Shape::line(
        path.iter().map(|q| *q + egui::vec2(0.7 * z, 1.8 * z)).collect(),
        Stroke::new(3.4 * z, Color32::from_black_alpha(90)),
    ));

    if on {
        // The wash: a long faint tail across the board, then four bands
        // hugging the glass. Together they fall off the way a discharge
        // does — hard for a few pixels, then a glow that reaches the
        // address. The widest band is 9z across a 10z bend, which is the
        // limit before its inner edge folds at the corners.
        p.add(Shape::mesh(tube_glow(
            x0,
            x1,
            run_y + 2.0 * z,
            8.0 * z,
            11.0 * z,
            t.color.gamma_multiply(0.24 * lv),
        )));
        tube(9.0 * z, t.color.gamma_multiply(0.06 * lv));
        tube(7.0 * z, t.color.gamma_multiply(0.10 * lv));
        tube(5.2 * z, t.color.gamma_multiply(0.15 * lv));
        tube(3.6 * z, lighten(t.color, 0.20).gamma_multiply(0.24 * lv));
        // Glass, then the discharge inside it. A struck tube does blow out
        // to white in the middle, but only the thin centre line does it
        // here: a hundred of these have to be told apart by colour, so the
        // wall keeps the hue and the white stays about a pixel wide per
        // zoom step. At a low level the core never gets there and the tube
        // is a dim, honest version of its own colour — never a warmer one.
        tube(3.2 * z, lighten(t.color, 0.10 + 0.22 * lv));
        tube(1.3 * z, lighten(t.color, 0.42 + 0.50 * lv));
    } else {
        // Cold: a hollow grey cylinder, plainly visible on the near-black
        // board — a pale rim with the dark bore showing through it. Seeing
        // the glass unlit is what makes the struck tube land.
        tube(3.2 * z, Color32::from_rgb(0x72, 0x76, 0x7E));
        tube(1.3 * z, Color32::from_rgb(0x26, 0x28, 0x2D));
    }
    // The hard line the glass catches along its lit side and the shade
    // under its belly. Between the two the run is a cylinder and not a bar,
    // and both stay one pixel wide at every zoom because a reflection does.
    p.add(Shape::line(
        offset(0.9 * z),
        Stroke::new(1.0, Color32::from_white_alpha(if on { 150 } else { 175 })),
    ));
    p.add(Shape::line(
        offset(-0.9 * z),
        Stroke::new(1.0, Color32::from_black_alpha(if on { 90 } else { 130 })),
    ));

    // Electrodes: each foot ends in a metal sleeve, which is where the
    // light stops instead of running off a cut end. They also cover the
    // flat cap the glow strokes leave at the bottom of each leg.
    let ew = 2.6 * z;
    let cap = Rounding::same(1.2 * z);
    let foot = run_y + drop;
    for x in [x0, x1] {
        let shell = Rect::from_min_max(
            Pos2::new(x - ew, foot - 3.2 * z),
            Pos2::new(x + ew, foot + 3.4 * z),
        );
        p.add(Shape::mesh(vgradient_mesh(
            shell,
            cap,
            Color32::from_rgb(0x62, 0x66, 0x6D),
            Color32::from_rgb(0x15, 0x16, 0x19),
        )));
        p.rect_stroke(shell, cap, Stroke::new(1.0, Color32::from_black_alpha(190)));
    }

    // Legend screen-printed on the board above the tube, shadowed so it
    // survives the wash. The address sits at the top edge of the glow and
    // takes the tube's colour while it is running.
    let w = rect.width() - 10.0 * z;
    let sh = Color32::from_black_alpha(200);
    shadowed_text(
        p,
        rect.left_top() + egui::vec2(5.0 * z, 2.5 * z),
        Align2::LEFT_TOP,
        t.name,
        FontId::proportional(10.5 * z),
        theme::TEXT,
        sh,
        w,
        z,
    );
    shadowed_text(
        p,
        rect.left_top() + egui::vec2(5.0 * z, 15.5 * z),
        Align2::LEFT_TOP,
        t.addr,
        FontId::monospace(8.5 * z),
        if on {
            lighten(t.color, 0.55)
        } else {
            theme::TEXT_DIM
        },
        sh,
        w,
        z,
    );

    p.rect_stroke(rect, rr, Stroke::new(1.0, Color32::from_rgb(0x2E, 0x31, 0x37)));
    select_ring(p, rect, rr, z, t);
}

#[cfg(test)]
mod tests {
    use crate::net::{Frame, DMX_SLOTS};
    use crate::stage::headless::{render_frames, save};
    use crate::stage::RaidLook;

    /// The Fixtures panel in every look, zoomed up, written to
    /// `target/raid_look_<name>.png` so a restyle can be checked by eye.
    #[test]
    fn raid_looks_render_headless() {
        let mut app = crate::app::App::new();
        let mut frame = [0u8; DMX_SLOTS];
        for (i, v) in frame.iter_mut().enumerate() {
            *v = ((i * 53) % 256) as u8;
        }
        // The first few fixtures fully dark, to see the unlit state.
        for f in app.patch.fixtures.iter().take(3) {
            for a in f.from..=f.to {
                if a >= 1 && a as usize <= DMX_SLOTS {
                    frame[a as usize - 1] = 0;
                }
            }
        }
        *app.net.dmx.lock() = Frame(frame);
        app.collapsed.insert("inspector");
        app.collapsed.insert("channels");
        app.show_log = false;
        if app.patch.fixtures.len() > 5 {
            app.stage.select_fixture(5, false);
        }
        let size = [1100, 800];
        for look in RaidLook::ALL {
            app.settings.raid_look = look;
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
            save(&pixels, size, &format!("raid_look_{}", look.label().to_lowercase().replace(' ', "_")));
        }
    }
}
