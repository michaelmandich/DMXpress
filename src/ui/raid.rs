//! The Fixtures panel's raid-grid tiles, drawn in each [`RaidLook`].
//!
//! Every look shows the same three things — the fixture's live colour, its
//! name and its DMX address — and answers the same selection outline. They
//! differ in the material the tile is made of: a lit colour chip, an amp
//! pilot-light jewel, a chamfered sci-fi slab, tinted glass, a chromed
//! lens, a soft pillow button. All the depth is faked with per-vertex
//! gradient meshes (see the helpers in [`theme`]), so the grid stays as
//! cheap as the flat one it replaced: no textures, no shaders.

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

/// Tile size for a look at panel zoom `z`. The jewel is taller because its
/// label sits under the lens instead of on it.
pub(crate) fn tile_size(look: RaidLook, z: f32) -> egui::Vec2 {
    let (w, h) = match look {
        RaidLook::Lit => (58.0, 38.0),
        RaidLook::Jewel => (56.0, 56.0),
        RaidLook::Future => (60.0, 40.0),
        RaidLook::Glass => (58.0, 38.0),
        RaidLook::Chrome => (60.0, 40.0),
        RaidLook::Pillow => (58.0, 38.0),
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

/// Name top-left, address bottom-left: the layout the flat grid always had.
fn corner_labels(p: &egui::Painter, rect: Rect, z: f32, t: &Tile, txt: Color32, dim: Color32) {
    let w = rect.width() - 8.0 * z;
    text(
        p,
        rect.left_top() + egui::vec2(4.0 * z, 3.0 * z),
        Align2::LEFT_TOP,
        t.name,
        FontId::proportional(11.0 * z),
        txt,
        w,
    );
    text(
        p,
        rect.left_bottom() + egui::vec2(4.0 * z, -3.0 * z),
        Align2::LEFT_BOTTOM,
        t.addr,
        FontId::monospace(9.0 * z),
        dim,
        w,
    );
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

/// The original: colour chip lit from above, hairline rim.
fn lit(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let r = 4.0 * z;
    p.rect_filled(rect, r, t.color);
    p.add(Shape::mesh(vgradient_mesh(
        rect,
        Rounding::same(r),
        Color32::from_rgba_unmultiplied(255, 255, 255, 34),
        Color32::from_rgba_unmultiplied(0, 0, 0, 40),
    )));
    let (txt, dim) = ink(t.color);
    corner_labels(p, rect, z, t, txt, dim);
    if t.selected {
        p.rect_stroke(rect, r, Stroke::new(2.0 * z, SELECT));
    } else {
        p.rect_stroke(rect, r, Stroke::new(1.0 * z, theme::EDGE));
    }
}

/// An amplifier pilot light: dark faceplate, chrome bezel, a faceted lens
/// that glows in the fixture's colour, and the label engraved beneath.
fn jewel(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let on = t.level > 0.02;
    let rr = 3.0 * z;
    // Faceplate: warm near-black steel, lit along the top.
    p.add(Shape::mesh(vgradient_mesh(
        rect,
        Rounding::same(rr),
        Color32::from_rgb(0x2C, 0x29, 0x27),
        Color32::from_rgb(0x15, 0x13, 0x12),
    )));
    p.rect_stroke(rect, rr, Stroke::new(1.0, Color32::from_rgb(0x3D, 0x39, 0x35)));
    p.hline(
        (rect.left() + rr)..=(rect.right() - rr),
        rect.top() + 1.5,
        Stroke::new(1.0, Color32::from_white_alpha(18)),
    );

    let c = Pos2::new(rect.center().x, rect.top() + 19.0 * z);
    let r = 11.0 * z;
    let bez = 3.2 * z;

    // Halo thrown onto the faceplate.
    if on {
        p.add(Shape::mesh(radial_mesh(
            c,
            r * 2.4,
            t.color.gamma_multiply(0.55 * lv),
            Color32::TRANSPARENT,
        )));
    }
    // Chrome bezel, dark hairlines either side so it reads as a ring. A
    // selected fixture's bezel takes the selection blue.
    let sel = t.selected;
    p.add(Shape::mesh(ring_mesh(c, r, r + bez, |a| {
        let v = metal(a);
        if sel {
            SELECT
                .lerp_to_gamma(Color32::WHITE, v * 0.6)
                .lerp_to_gamma(Color32::BLACK, (1.0 - v) * 0.5)
        } else {
            steel(v)
        }
    })));
    p.circle_stroke(c, r + bez, Stroke::new(1.0, Color32::from_black_alpha(150)));
    p.circle_stroke(c, r, Stroke::new(1.0, Color32::from_black_alpha(190)));

    // The lens: a cut bevel around a domed core, both in the live colour.
    let base = if on {
        t.color
    } else {
        Color32::from_rgb(0x30, 0x2E, 0x2C)
    };
    let core_r = r * 0.68;
    p.add(Shape::mesh(ring_mesh(c, core_r, r, |a| {
        let k = 0.5 + 0.5 * (a - 1.25 * PI).cos();
        lighten(base, 0.45 * k * (0.4 + 0.6 * lv))
            .lerp_to_gamma(darken(base, 0.55), (1.0 - k) * 0.7)
    })));
    p.add(Shape::mesh(radial_mesh(
        c,
        core_r,
        lighten(base, 0.15 + 0.6 * lv),
        darken(base, 0.15),
    )));
    p.circle_stroke(c, core_r, Stroke::new(1.0, Color32::from_black_alpha(70)));
    // Facets: eight cuts across the bevel.
    for i in 0..8 {
        let a = i as f32 / 8.0 * 2.0 * PI + PI / 8.0;
        let d = egui::vec2(a.cos(), a.sin());
        p.line_segment(
            [c + d * core_r, c + d * r],
            Stroke::new(1.0, Color32::from_white_alpha(if on { 60 } else { 30 })),
        );
    }
    // Specular hot-spot on the dome.
    p.add(Shape::mesh(radial_mesh(
        c + egui::vec2(-0.32 * r, -0.36 * r),
        0.3 * r,
        Color32::from_white_alpha(if on { 190 } else { 110 }),
        Color32::TRANSPARENT,
    )));

    // Engraved label under the lamp.
    let w = rect.width() - 8.0 * z;
    text(
        p,
        Pos2::new(c.x, rect.bottom() - 13.0 * z),
        Align2::CENTER_BOTTOM,
        t.name,
        FontId::proportional(10.0 * z),
        theme::TEXT,
        w,
    );
    text(
        p,
        Pos2::new(c.x, rect.bottom() - 3.0 * z),
        Align2::CENTER_BOTTOM,
        t.addr,
        FontId::monospace(8.5 * z),
        theme::TEXT_DIM,
        w,
    );
    select_ring(p, rect, rr, z, t);
}

/// Chamfered black slab, corner brackets, a colour bar glowing along the
/// bottom edge.
fn future(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let on = t.level > 0.02;
    let ch = 6.0 * z;
    let pts = [
        rect.left_top(),
        Pos2::new(rect.right() - ch, rect.top()),
        Pos2::new(rect.right(), rect.top() + ch),
        rect.right_bottom(),
        Pos2::new(rect.left() + ch, rect.bottom()),
        Pos2::new(rect.left(), rect.bottom() - ch),
    ];
    let top = Color32::from_rgb(0x16, 0x1A, 0x21);
    let bot = Color32::from_rgb(0x08, 0x0A, 0x0E);
    let h = rect.height().max(1.0);
    p.add(Shape::mesh(convex_mesh(&pts, |q| {
        top.lerp_to_gamma(bot, ((q.y - rect.top()) / h).clamp(0.0, 1.0))
    })));
    // Colour cast off the bar.
    if on {
        p.add(Shape::mesh(convex_mesh(&pts, |q| {
            let k = ((q.y - rect.top()) / h).clamp(0.0, 1.0);
            t.color.gamma_multiply(0.22 * lv * k)
        })));
    }
    // Outline in the tint.
    let edge = if on {
        t.color.gamma_multiply(0.35 + 0.45 * lv)
    } else {
        Color32::from_rgb(0x2E, 0x34, 0x3E)
    };
    p.add(Shape::closed_line(pts.to_vec(), Stroke::new(1.0, edge)));
    // Corner brackets.
    let bl = 7.0 * z;
    let br = Stroke::new(1.5 * z, lighten(edge, 0.2));
    let o = 3.0 * z;
    let tl = rect.left_top() + egui::vec2(o, o);
    p.line_segment([tl, tl + egui::vec2(bl, 0.0)], br);
    p.line_segment([tl, tl + egui::vec2(0.0, bl)], br);
    let rb = rect.right_bottom() - egui::vec2(o, o);
    p.line_segment([rb, rb - egui::vec2(bl, 0.0)], br);
    p.line_segment([rb, rb - egui::vec2(0.0, bl)], br);
    // The bar and its bloom.
    let bar = Rect::from_min_max(
        Pos2::new(rect.left() + ch + 2.0 * z, rect.bottom() - 7.0 * z),
        Pos2::new(rect.right() - 6.0 * z, rect.bottom() - 4.0 * z),
    );
    if on {
        let bloom = Rect::from_min_max(
            Pos2::new(bar.left() - 2.0 * z, bar.top() - 9.0 * z),
            Pos2::new(bar.right() + 2.0 * z, bar.top()),
        );
        p.add(Shape::mesh(vgradient_mesh(
            bloom,
            Rounding::ZERO,
            Color32::TRANSPARENT,
            t.color.gamma_multiply(0.55 * lv),
        )));
        let under = Rect::from_min_max(bar.left_bottom(), Pos2::new(bar.right(), rect.bottom()));
        p.add(Shape::mesh(vgradient_mesh(
            under,
            Rounding::ZERO,
            t.color.gamma_multiply(0.5 * lv),
            Color32::TRANSPARENT,
        )));
        p.rect_filled(bar, 1.0, lighten(t.color, 0.25 * lv));
        p.hline(bar.x_range(), bar.top() + 0.5, Stroke::new(1.0, lighten(t.color, 0.6)));
    } else {
        p.rect_filled(bar, 1.0, Color32::from_rgb(0x22, 0x27, 0x2F));
    }
    let w = rect.width() - 14.0 * z;
    text(
        p,
        rect.left_top() + egui::vec2(6.0 * z, 4.0 * z),
        Align2::LEFT_TOP,
        t.name,
        FontId::proportional(10.5 * z),
        theme::TEXT,
        w,
    );
    text(
        p,
        rect.left_top() + egui::vec2(6.0 * z, 16.0 * z),
        Align2::LEFT_TOP,
        t.addr,
        FontId::monospace(8.5 * z),
        if on {
            lighten(t.color, 0.45)
        } else {
            theme::TEXT_DIM
        },
        w,
    );
    if t.selected {
        p.add(Shape::closed_line(pts.to_vec(), Stroke::new(2.0 * z, SELECT)));
    }
}

/// A pane of tinted glass on a dark backing: translucent colour, a sheen
/// down from the top edge, a diagonal specular streak.
fn glass(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let r = 7.0 * z;
    p.rect_filled(rect, r, theme::WELL);
    let tint = if t.level > 0.02 {
        t.color.gamma_multiply(0.42 + 0.5 * lv)
    } else {
        Color32::from_rgb(0x1E, 0x22, 0x28)
    };
    p.rect_filled(rect, r, tint);
    // Sheen: the upper half catches the room light.
    let upper = Rect::from_min_max(
        rect.min,
        Pos2::new(rect.right(), rect.top() + rect.height() * 0.48),
    );
    p.add(Shape::mesh(vgradient_mesh(
        upper,
        top_rounding(r),
        Color32::from_white_alpha(72),
        Color32::from_white_alpha(6),
    )));
    // A soft vertical reflection band to the right, off the label. It is
    // axis-aligned because meshes have no anti-aliasing and a diagonal
    // would show its steps.
    let w = rect.width();
    let (x0, x1) = (rect.left() + w * 0.62, rect.left() + w * 0.94);
    let pc = p.with_clip_rect(rect);
    // Peak in the middle of the band: two meshes meeting at its centre.
    let xm = (x0 + x1) * 0.5;
    for (a, b) in [(x0, xm), (xm, x1)] {
        let quad = [
            Pos2::new(a, rect.top()),
            Pos2::new(b, rect.top()),
            Pos2::new(b, rect.bottom()),
            Pos2::new(a, rect.bottom()),
        ];
        pc.add(Shape::mesh(convex_mesh(&quad, |q| {
            let near = 1.0 - ((q.x - xm).abs() / (xm - x0).max(1.0)).clamp(0.0, 1.0);
            let k = ((q.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
            Color32::from_white_alpha((30.0 * near * (1.0 - 0.6 * k)) as u8)
        })));
    }
    // Rims: bright top, dark bottom, a fine glass edge all round.
    p.hline(
        (rect.left() + r)..=(rect.right() - r),
        rect.top() + 1.0,
        Stroke::new(1.0, Color32::from_white_alpha(120)),
    );
    p.hline(
        (rect.left() + r)..=(rect.right() - r),
        rect.bottom() - 1.0,
        Stroke::new(1.0, Color32::from_black_alpha(150)),
    );
    p.rect_stroke(rect, r, Stroke::new(1.0, Color32::from_white_alpha(46)));
    let w = rect.width() - 8.0 * z;
    let sh = Color32::from_black_alpha(160);
    shadowed_text(
        p,
        rect.left_top() + egui::vec2(4.0 * z, 3.0 * z),
        Align2::LEFT_TOP,
        t.name,
        FontId::proportional(11.0 * z),
        Color32::WHITE,
        sh,
        w,
        z,
    );
    shadowed_text(
        p,
        rect.left_bottom() + egui::vec2(4.0 * z, -3.0 * z),
        Align2::LEFT_BOTTOM,
        t.addr,
        FontId::monospace(9.0 * z),
        Color32::from_white_alpha(200),
        sh,
        w,
        z,
    );
    select_ring(p, rect, r, z, t);
}

/// A domed colour lens set into a brushed-steel frame.
fn chrome(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let r = 5.0 * z;
    // Frame: two gradient bands make the mid-tone metallic fold.
    let mid = rect.top() + rect.height() * 0.5;
    let upper = Rect::from_min_max(rect.min, Pos2::new(rect.right(), mid));
    let lower = Rect::from_min_max(Pos2::new(rect.left(), mid), rect.max);
    p.add(Shape::mesh(vgradient_mesh(
        upper,
        top_rounding(r),
        steel(0.80),
        steel(0.52),
    )));
    p.add(Shape::mesh(vgradient_mesh(
        lower,
        bottom_rounding(r),
        steel(0.40),
        steel(0.68),
    )));
    p.hline(
        (rect.left() + r)..=(rect.right() - r),
        rect.top() + 1.0,
        Stroke::new(1.0, Color32::from_white_alpha(150)),
    );
    p.rect_stroke(rect, r, Stroke::new(1.0, Color32::from_rgb(0x2A, 0x2C, 0x30)));
    // Lens well.
    let inset = 4.0 * z;
    let lens = rect.shrink(inset);
    let lr = 3.0 * z;
    let base = if t.level > 0.02 {
        t.color
    } else {
        Color32::from_rgb(0x24, 0x26, 0x2A)
    };
    p.add(Shape::mesh(vgradient_mesh(
        lens,
        Rounding::same(lr),
        lighten(base, 0.12 + 0.45 * lv),
        darken(base, 0.38),
    )));
    // The frame overhangs the lens: a shadow under its top lip.
    let lip = Rect::from_min_max(lens.min, Pos2::new(lens.right(), lens.top() + 6.0 * z));
    p.add(Shape::mesh(vgradient_mesh(
        lip,
        top_rounding(lr),
        Color32::from_black_alpha(130),
        Color32::TRANSPARENT,
    )));
    // Dome highlight: a soft band across the upper third.
    let hl = Rect::from_min_max(
        Pos2::new(lens.left() + 3.0 * z, lens.top() + 2.0 * z),
        Pos2::new(lens.right() - 3.0 * z, lens.top() + lens.height() * 0.42),
    );
    p.add(Shape::mesh(vgradient_mesh(
        hl,
        Rounding::same(lr),
        Color32::from_white_alpha(if t.level > 0.02 { 80 } else { 40 }),
        Color32::TRANSPARENT,
    )));
    p.rect_stroke(lens, lr, Stroke::new(1.0, Color32::from_black_alpha(190)));
    let (txt, dim) = ink(lighten(base, 0.2 * lv));
    let w = lens.width() - 8.0 * z;
    text(
        p,
        lens.left_top() + egui::vec2(4.0 * z, 3.0 * z),
        Align2::LEFT_TOP,
        t.name,
        FontId::proportional(10.5 * z),
        txt,
        w,
    );
    text(
        p,
        lens.left_bottom() + egui::vec2(4.0 * z, -2.0 * z),
        Align2::LEFT_BOTTOM,
        t.addr,
        FontId::monospace(8.5 * z),
        dim,
        w,
    );
    select_ring(p, rect, r, z, t);
}

/// A soft convex button: deep rounding, a lit crown, a shadow it sits in.
fn pillow(p: &egui::Painter, rect: Rect, z: f32, t: &Tile) {
    let lv = vis(t.level);
    let r = 11.0 * z;
    let body = rect
        .shrink2(egui::vec2(0.0, 1.5 * z))
        .translate(egui::vec2(0.0, -0.5 * z));
    // Shadow pooled beneath.
    p.rect_filled(
        body.translate(egui::vec2(0.0, 3.0 * z)),
        r,
        Color32::from_black_alpha(70),
    );
    p.rect_filled(
        body.translate(egui::vec2(0.0, 1.5 * z)),
        r,
        Color32::from_black_alpha(120),
    );
    let base = if t.level > 0.02 {
        t.color
    } else {
        Color32::from_rgb(0x2A, 0x2E, 0x34)
    };
    p.add(Shape::mesh(vgradient_mesh(
        body,
        Rounding::same(r),
        lighten(base, 0.28 + 0.2 * lv),
        darken(base, 0.32),
    )));
    // Crown highlight and a faint bounce along the base.
    let crown = Rect::from_min_max(
        body.min + egui::vec2(2.0 * z, 1.5 * z),
        Pos2::new(body.right() - 2.0 * z, body.top() + body.height() * 0.45),
    );
    p.add(Shape::mesh(vgradient_mesh(
        crown,
        Rounding::same(r - 2.0 * z),
        Color32::from_white_alpha(95),
        Color32::TRANSPARENT,
    )));
    let foot = Rect::from_min_max(
        Pos2::new(body.left() + 3.0 * z, body.bottom() - body.height() * 0.22),
        body.max - egui::vec2(3.0 * z, 1.0 * z),
    );
    p.add(Shape::mesh(vgradient_mesh(
        foot,
        Rounding::same(r - 3.0 * z),
        Color32::TRANSPARENT,
        Color32::from_white_alpha(22),
    )));
    p.rect_stroke(body, r, Stroke::new(1.0, darken(base, 0.55)));
    let (txt, dim) = ink(base);
    let sh = if txt == Color32::BLACK {
        Color32::from_white_alpha(70)
    } else {
        Color32::from_black_alpha(120)
    };
    let w = body.width() - 14.0 * z;
    shadowed_text(
        p,
        body.left_top() + egui::vec2(7.0 * z, 4.0 * z),
        Align2::LEFT_TOP,
        t.name,
        FontId::proportional(11.0 * z),
        txt,
        sh,
        w,
        z,
    );
    shadowed_text(
        p,
        body.left_bottom() + egui::vec2(7.0 * z, -4.0 * z),
        Align2::LEFT_BOTTOM,
        t.addr,
        FontId::monospace(9.0 * z),
        dim,
        sh,
        w,
        z,
    );
    if t.selected {
        p.rect_stroke(body, r, Stroke::new(2.0 * z, SELECT));
    }
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
