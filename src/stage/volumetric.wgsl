// Analytic participating-media beam shader, plus the pool a beam throws on a
// surface. Each instance rasterizes only its projected bounds. A beam then
// integrates a soft finite cone along the view ray; a pool intersects the
// ray with the surface plane and maps the hit back through the cone. Both
// sample the gobo mask atlas where the beam carries a gobo.

struct Camera {
    eye: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    viewport: vec4<f32>, // aspect, tan(fov/2), unused, unused
};

struct Beam {
    apex: vec4<f32>,
    direction_length: vec4<f32>,
    color_density: vec4<f32>,
    bounds: vec4<f32>, // min u/v, max u/v in the stage viewport
    params: vec4<f32>, // cone tan, aperture radius, unused, unused
    u_axis: vec4<f32>, // gobo-plane basis across the beam
    v_axis: vec4<f32>,
    gobo: vec4<f32>,   // atlas layer A (-1 = none), layer B, angle A, angle B
};

struct Pool {
    apex: vec4<f32>,   // xyz, surface plane height
    axis: vec4<f32>,   // xyz, cone tan
    u_axis: vec4<f32>,
    v_axis: vec4<f32>,
    color: vec4<f32>,  // rgb, peak alpha
    bounds: vec4<f32>,
    gobo: vec4<f32>,
    params: vec4<f32>, // aperture radius, distance to the surface, unused, unused
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> beams: array<Beam>;
@group(0) @binding(2) var<storage, read> pools: array<Pool>;
@group(0) @binding(3) var atlas: texture_2d_array<f32>;
@group(0) @binding(4) var atlas_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) index: u32,
};

fn quad_corner(vertex: u32) -> vec2<f32> {
    switch vertex {
        case 1u, 4u: { return vec2<f32>(1.0, 0.0); }
        case 2u, 3u: { return vec2<f32>(0.0, 1.0); }
        case 5u: { return vec2<f32>(1.0, 1.0); }
        default: { return vec2<f32>(0.0, 0.0); }
    }
}

fn place(bounds: vec4<f32>, corner: vec2<f32>, index: u32) -> VertexOut {
    let uv = mix(bounds.xy, bounds.zw, corner);
    var out: VertexOut;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    out.index = index;
    return out;
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOut {
    return place(beams[instance].bounds, quad_corner(vertex), instance);
}

@vertex
fn vs_pool(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOut {
    return place(pools[instance].bounds, quad_corner(vertex), instance);
}

/// The camera ray through a viewport point.
fn view_ray(uv: vec2<f32>) -> vec3<f32> {
    let sx = (uv.x * 2.0 - 1.0) * camera.viewport.x * camera.viewport.y;
    let sy = (1.0 - uv.y * 2.0) * camera.viewport.y;
    return normalize(camera.forward.xyz + camera.right.xyz * sx + camera.up.xyz * sy);
}

fn hash12(p: vec2<f32>) -> f32 {
    let p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    let q = p3 + dot(p3, p3.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

/// How much light one gobo passes at `uv` (-1..1 across the beam, the unit
/// circle being the gobo's aperture), turned by `angle`. No gobo passes all.
/// Sampled at level 0 explicitly so it can sit inside data-dependent branches.
fn gobo_mask(uv: vec2<f32>, layer: f32, angle: f32) -> f32 {
    if layer < 0.0 {
        return 1.0;
    }
    let c = cos(angle);
    let s = sin(angle);
    let turned = vec2<f32>(c * uv.x - s * uv.y, s * uv.x + c * uv.y);
    let tc = turned * 0.5 + vec2<f32>(0.5, 0.5);
    return textureSampleLevel(atlas, atlas_sampler, tc, i32(layer), 0.0).r;
}

/// Both gobos in the light path (a static and a rotating wheel), multiplied.
fn gobos(uv: vec2<f32>, g: vec4<f32>) -> f32 {
    return gobo_mask(uv, g.x, g.z) * gobo_mask(uv, g.y, g.w);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let beam = beams[in.index];
    let ray = view_ray(in.uv);

    let axis = beam.direction_length.xyz;
    let beam_len = beam.direction_length.w;
    let from_apex = camera.eye.xyz - beam.apex.xyz;
    let rd = dot(ray, axis);
    let ray_origin_axis = dot(ray, from_apex);
    let axis_origin = dot(axis, from_apex);
    let denom = max(1.0 - rd * rd, 0.0005);

    // Closest points between the camera ray and beam axis, clamped to the
    // finite beam. This is an analytic approximation to the cone path integral
    // and stays smooth even with dozens of overlapping beams.
    var axial = (axis_origin - rd * ray_origin_axis) / denom;
    axial = clamp(axial, 0.0, beam_len);
    let axis_point = beam.apex.xyz + axis * axial;
    let along_ray = max(dot(axis_point - camera.eye.xyz, ray), 0.0);
    let ray_point = camera.eye.xyz + ray * along_ray;
    let radial_distance = length(ray_point - axis_point);
    let radius = beam.params.y + beam.params.x * axial;
    let q = radial_distance / max(radius, 0.001);
    if q >= 1.0 || along_ray <= 0.0 {
        discard;
    }
    let grazing = 1.0 / sqrt(max(1.0 - rd * rd, 0.035));

    // Gaussian-like core, feathered fully to zero at the physical cone edge.
    var radial_density = exp(-2.8 * q * q) * smoothstep(1.0, 0.68, q);
    if beam.gobo.x >= 0.0 || beam.gobo.y >= 0.0 {
        // A gobo cuts the cone into rays: march the chord of the cone this
        // view ray crosses and average the mask over it, so the pattern's
        // structure reads through the haze instead of painting one slice.
        let half_chord = sqrt(max(radius * radius - radial_distance * radial_distance, 0.0)) * grazing;
        var acc = 0.0;
        for (var k: i32 = 0; k < 6; k++) {
            let t = along_ray + half_chord * ((f32(k) + 0.5) / 3.0 - 1.0);
            let p = camera.eye.xyz + ray * t;
            let d = p - beam.apex.xyz;
            let ax = dot(d, axis);
            let off = d - axis * ax;
            let r = beam.params.y + beam.params.x * max(ax, 0.0);
            let qq = length(off) / max(r, 0.001);
            if ax < 0.0 || ax > beam_len || qq >= 1.0 {
                continue;
            }
            let uv = vec2<f32>(dot(off, beam.u_axis.xyz), dot(off, beam.v_axis.xyz)) / r;
            let m = gobos(uv, beam.gobo);
            acc += m * exp(-2.8 * qq * qq) * smoothstep(1.0, 0.68, qq);
        }
        // The metal blocks most of the light; lift what gets through so the
        // rays read as bright as the plain cone did.
        radial_density = acc / 6.0 * 1.7;
    }
    let distance_fade = pow(max(1.0 - axial / max(beam_len, 0.001), 0.0), 1.35);
    let aperture_fade = smoothstep(0.0, min(0.35, beam_len * 0.06), axial);
    let path_length = min(radius * 2.0 * grazing, 3.5);

    // Slight stable dither prevents low-alpha color banding without making the
    // haze visibly noisy or animated.
    let grain = 0.97 + 0.06 * hash12(in.position.xy);
    let optical_depth = beam.color_density.w * radial_density * distance_fade
        * aperture_fade * (0.22 + path_length * 0.42) * grain;
    let alpha = clamp(1.0 - exp(-optical_depth), 0.0, 0.92);
    if alpha < 0.001 {
        discard;
    }

    // Premultiplied alpha matches egui's compositor and lets crossing beams
    // build luminous haze naturally.
    return vec4<f32>(beam.color_density.rgb * alpha, alpha);
}

@fragment
fn fs_pool(in: VertexOut) -> @location(0) vec4<f32> {
    let pool = pools[in.index];
    let ray = view_ray(in.uv);
    let plane_y = pool.apex.w;
    if abs(ray.y) < 1e-4 {
        discard;
    }
    let t = (plane_y - camera.eye.y) / ray.y;
    if t <= 0.0 {
        discard;
    }
    // Where this pixel's ray meets the surface, mapped back through the cone
    // to the gobo plane: the same projective map a real lens makes.
    let hit = camera.eye.xyz + ray * t;
    let d = hit - pool.apex.xyz;
    let axial = dot(d, pool.axis.xyz);
    if axial <= 0.0 {
        discard;
    }
    let off = d - pool.axis.xyz * axial;
    let r = pool.params.x + pool.axis.w * axial;
    let q = length(off) / max(r, 0.001);
    if q >= 1.0 {
        discard;
    }
    let uv = vec2<f32>(dot(off, pool.u_axis.xyz), dot(off, pool.v_axis.xyz)) / r;
    let has_gobo = pool.gobo.x >= 0.0 || pool.gobo.y >= 0.0;
    var shape = exp(-2.2 * q * q) * smoothstep(1.0, 0.72, q);
    if has_gobo {
        // The gobo does the shaping; the lens just vignettes a little.
        shape = (0.6 + 0.4 * exp(-1.5 * q * q)) * smoothstep(1.0, 0.92, q) * gobos(uv, pool.gobo) * 1.4;
    }
    let alpha = clamp(pool.color.w * shape, 0.0, 0.9);
    if alpha < 0.002 {
        discard;
    }
    return vec4<f32>(pool.color.rgb * alpha, alpha);
}
