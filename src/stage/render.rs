//! Fixture mesh construction and painter projection. Beam haze is rendered by
//! the Metal/wgpu volumetric callback in `volumetric`.

use eframe::egui::{self, Color32, Pos2, Rect, Shape};

use super::fixture::vis_curve;
use super::math::{v3, Camera, V3};

/// Soft radial pool where a beam strikes a horizontal surface. Vertex alpha
/// interpolation gives a continuous hotspot instead of stacked circles.
pub(crate) fn surface_pool_shape(
    cam: &Camera,
    rect: Rect,
    center: V3,
    direction: V3,
    radius: f32,
    color: Color32,
    brightness: f32,
    opacity: f32,
) -> Option<Shape> {
    const SIDES: usize = 24;
    const RADII: [f32; 4] = [0.0, 0.38, 0.72, 1.0];
    const DENSITY: [f32; 4] = [1.0, 0.78, 0.28, 0.0];
    let vb = vis_curve(brightness);
    let hue_scale = (vb / brightness.max(1e-3)).min(4.0);
    let rgb = [
        (color.r() as f32 * hue_scale).min(255.0) as u8,
        (color.g() as f32 * hue_scale).min(255.0) as u8,
        (color.b() as f32 * hue_scale).min(255.0) as u8,
    ];
    let base_alpha = ((28.0 + vb * 105.0) * opacity.max(0.0)).clamp(0.0, 190.0);
    let horizontal = v3(direction.x, 0.0, direction.z).norm();
    let along = if horizontal.len() > 0.01 {
        horizontal
    } else {
        v3(1.0, 0.0, 0.0)
    };
    let across = v3(-along.z, 0.0, along.x);
    let incidence_stretch = (1.0 / direction.y.abs().max(0.25)).min(3.0);
    let mut mesh = egui::Mesh::default();
    for (ri, &rf) in RADII.iter().enumerate() {
        if ri == 0 {
            let (pos, depth) = cam.project(rect, center)?;
            if depth <= 0.2 {
                return None;
            }
            mesh.vertices.push(egui::epaint::Vertex {
                pos,
                uv: egui::epaint::WHITE_UV,
                color: Color32::from_rgba_unmultiplied(
                    rgb[0], rgb[1], rgb[2], base_alpha as u8,
                ),
            });
            continue;
        }
        let alpha = (base_alpha * DENSITY[ri]) as u8;
        for k in 0..SIDES {
            let a = k as f32 / SIDES as f32 * std::f32::consts::TAU;
            let world = center
                + across * (a.cos() * radius * rf)
                + along * (a.sin() * radius * rf * incidence_stretch)
                + v3(0.0, 0.002, 0.0);
            let (pos, depth) = cam.project(rect, world)?;
            if depth <= 0.2 {
                return None;
            }
            mesh.vertices.push(egui::epaint::Vertex {
                pos,
                uv: egui::epaint::WHITE_UV,
                color: Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], alpha),
            });
        }
    }
    // Centre fan to the first ring.
    for k in 0..SIDES {
        mesh.add_triangle(0, (1 + k) as u32, (1 + (k + 1) % SIDES) as u32);
    }
    // Annuli between the remaining radial rings.
    for band in 0..2 {
        let inner = 1 + band * SIDES;
        let outer = inner + SIDES;
        for k in 0..SIDES {
            let next = (k + 1) % SIDES;
            mesh.add_triangle((inner + k) as u32, (outer + k) as u32, (inner + next) as u32);
            mesh.add_triangle((inner + next) as u32, (outer + k) as u32, (outer + next) as u32);
        }
    }
    Some(Shape::mesh(mesh))
}

// ---------------------------------------------------------------- fixture meshes

/// Tiny world-space mesh: convex faces with a flat color. `emissive` faces
/// skip shading (light-emitting apertures).
#[derive(Default)]
pub(crate) struct Mesh {
    pub faces: Vec<(Vec<V3>, Color32, bool)>,
}

/// Axis-aligned-in-local-frame box from a center and three half-extent vectors.
/// `front` colors the +az face as an emitting surface.
pub(crate) fn add_box(
    mesh: &mut Mesh,
    c: V3,
    ax: V3,
    ay: V3,
    az: V3,
    col: Color32,
    front: Option<Color32>,
) {
    let corner = |sx: f32, sy: f32, sz: f32| c + ax * sx + ay * sy + az * sz;
    // (face corners CCW, is +az face)
    let faces: [([V3; 4], bool); 6] = [
        ([corner(-1.0, -1.0, 1.0), corner(1.0, -1.0, 1.0), corner(1.0, 1.0, 1.0), corner(-1.0, 1.0, 1.0)], true),
        ([corner(-1.0, -1.0, -1.0), corner(-1.0, 1.0, -1.0), corner(1.0, 1.0, -1.0), corner(1.0, -1.0, -1.0)], false),
        ([corner(-1.0, 1.0, -1.0), corner(-1.0, 1.0, 1.0), corner(1.0, 1.0, 1.0), corner(1.0, 1.0, -1.0)], false),
        ([corner(-1.0, -1.0, -1.0), corner(1.0, -1.0, -1.0), corner(1.0, -1.0, 1.0), corner(-1.0, -1.0, 1.0)], false),
        ([corner(1.0, -1.0, -1.0), corner(1.0, 1.0, -1.0), corner(1.0, 1.0, 1.0), corner(1.0, -1.0, 1.0)], false),
        ([corner(-1.0, -1.0, -1.0), corner(-1.0, -1.0, 1.0), corner(-1.0, 1.0, 1.0), corner(-1.0, 1.0, -1.0)], false),
    ];
    for (pts, is_front) in faces {
        match (is_front, front) {
            (true, Some(e)) => mesh.faces.push((pts.to_vec(), e, true)),
            _ => mesh.faces.push((pts.to_vec(), col, false)),
        }
    }
}

/// Cylinder centered at `c` along unit `axis`, half-length `hl`. The +axis cap
/// is the emitting aperture when `front` is set.
pub(crate) fn add_cylinder(
    mesh: &mut Mesh,
    c: V3,
    axis: V3,
    radius: f32,
    hl: f32,
    col: Color32,
    front: Option<Color32>,
) {
    let helper = if axis.y.abs() > 0.9 {
        v3(1.0, 0.0, 0.0)
    } else {
        v3(0.0, 1.0, 0.0)
    };
    let u = axis.cross(helper).norm();
    let v = axis.cross(u).norm();
    const N: usize = 10;
    let ring = |off: f32, k: usize| {
        let a = k as f32 / N as f32 * std::f32::consts::TAU;
        c + axis * off + u * (a.cos() * radius) + v * (a.sin() * radius)
    };
    for k in 0..N {
        mesh.faces.push((
            vec![ring(-hl, k), ring(-hl, (k + 1) % N), ring(hl, (k + 1) % N), ring(hl, k)],
            col,
            false,
        ));
    }
    let cap = |off: f32| (0..N).map(|k| ring(off, k)).collect::<Vec<_>>();
    mesh.faces.push((cap(-hl), col, false));
    match front {
        Some(e) => mesh.faces.push((cap(hl), e, true)),
        None => mesh.faces.push((cap(hl), col, false)),
    }
}

/// Project a mesh, shade by face normal, and return it as a single triangle
/// mesh with the far faces first.
///
/// The faces share one `egui::Mesh` instead of being one filled polygon each:
/// egui anti-aliases every filled shape by feathering its own outline, so two
/// abutting faces each fade out along the edge they share and leave a hairline
/// seam between them. At macOS's 2x scaling that is invisible; at the
/// fractional DPI Windows normally runs the feather lands mid-pixel and the
/// seams read as thin lines all over the bodies. Raw triangles are not
/// feathered at all, so shared edges stay watertight.
pub(crate) fn mesh_shapes(cam: &Camera, rect: Rect, mesh: &Mesh) -> Vec<Shape> {
    let light = v3(0.35, 0.8, 0.5).norm();
    let mut faces: Vec<(f32, Vec<Pos2>, Color32)> = Vec::new();
    for (pts, col, emissive) in &mesh.faces {
        let mut proj = Vec::with_capacity(pts.len());
        let mut depth = 0.0;
        let mut ok = true;
        for p in pts {
            match cam.project(rect, *p) {
                Some((sp, z)) => {
                    proj.push(sp);
                    depth += z;
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok || proj.len() < 3 {
            continue;
        }
        depth /= proj.len() as f32;
        let fill = if *emissive {
            *col
        } else {
            let n = (pts[1] - pts[0]).cross(pts[2] - pts[0]).norm();
            let k = 0.45 + 0.55 * n.dot(light).abs();
            Color32::from_rgba_unmultiplied(
                (col.r() as f32 * k) as u8,
                (col.g() as f32 * k) as u8,
                (col.b() as f32 * k) as u8,
                col.a(),
            )
        };
        faces.push((depth, proj, fill));
    }
    faces.sort_by(|a, b| b.0.total_cmp(&a.0));

    let mut out = egui::Mesh::default();
    for (_, proj, fill) in faces {
        // Convex faces, so a fan from the first corner is a valid triangulation.
        let base = out.vertices.len() as u32;
        for pos in &proj {
            out.vertices.push(egui::epaint::Vertex {
                pos: *pos,
                uv: egui::epaint::WHITE_UV,
                color: fill,
            });
        }
        for k in 1..proj.len() as u32 - 1 {
            out.add_triangle(base, base + k, base + k + 1);
        }
    }
    if out.is_empty() {
        Vec::new()
    } else {
        vec![Shape::mesh(out)]
    }
}
