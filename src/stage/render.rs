//! Fixture mesh construction and painter projection. Beam haze and the pools
//! beams throw on surfaces are rendered by the Metal/wgpu callbacks in
//! `volumetric`.

use eframe::egui::{self, Color32, Pos2, Rect, Shape};

use super::math::{v3, Camera, V3};

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
