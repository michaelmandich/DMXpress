// Analytic participating-media beam shader. Each instance rasterizes only its
// projected cone bounds, then integrates a soft finite cone along the view ray.

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
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> beams: array<Beam>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) beam_index: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOut {
    var corner = vec2<f32>(0.0, 0.0);
    switch vertex {
        case 1u, 4u: { corner = vec2<f32>(1.0, 0.0); }
        case 2u, 3u: { corner = vec2<f32>(0.0, 1.0); }
        case 5u: { corner = vec2<f32>(1.0, 1.0); }
        default: {}
    }
    let b = beams[instance];
    let uv = mix(b.bounds.xy, b.bounds.zw, corner);
    var out: VertexOut;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    out.beam_index = instance;
    return out;
}

fn hash12(p: vec2<f32>) -> f32 {
    let p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    let q = p3 + dot(p3, p3.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let beam = beams[in.beam_index];
    let sx = (in.uv.x * 2.0 - 1.0) * camera.viewport.x * camera.viewport.y;
    let sy = (1.0 - in.uv.y * 2.0) * camera.viewport.y;
    let ray = normalize(camera.forward.xyz + camera.right.xyz * sx + camera.up.xyz * sy);

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

    // Gaussian-like core, feathered fully to zero at the physical cone edge.
    let edge = smoothstep(1.0, 0.68, q);
    let core = exp(-2.8 * q * q);
    let radial_density = core * edge;
    let distance_fade = pow(max(1.0 - axial / max(beam_len, 0.001), 0.0), 1.35);
    let aperture_fade = smoothstep(0.0, min(0.35, beam_len * 0.06), axial);
    let grazing = 1.0 / sqrt(max(1.0 - rd * rd, 0.035));
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
