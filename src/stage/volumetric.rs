//! Metal/wgpu-backed analytic volumetric beams and the pools they throw on
//! surfaces. The CPU computes a tight screen-space bound for each finite
//! cone and each pool; the fragment shaders evaluate a smooth
//! participating-media density along each camera ray, or the projected gobo
//! image on the surface, sampling the gobo mask atlas (`atlas.rs`) where a
//! beam carries a gobo.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use eframe::egui::{Color32, PaintCallback, Rect};
use eframe::egui_wgpu::{self, wgpu};
use wgpu::util::DeviceExt;

use super::atlas::AtlasUpload;
use super::fixture::vis_curve;
use super::math::{v3, Camera, V3};

const MAX_BEAMS: usize = 128;
const MAX_POOLS: usize = 128;
const RING_POINTS: usize = 24;
const SLICES: usize = 8;

/// A gobo in a beam's path: its atlas layer and how far it is turned.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GoboOn {
    pub layer: u32,
    pub angle: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct BeamSpec {
    pub apex: V3,
    pub dir: V3,
    pub len: f32,
    pub half_angle: f32,
    pub color: Color32,
    pub brightness: f32,
    pub opacity: f32,
    /// Up to two gobos in the path (a static and a rotating wheel).
    pub gobos: [Option<GoboOn>; 2],
    /// Where the beam lands on a horizontal surface, if it does: distance
    /// along the beam, and the surface's height.
    pub surface: Option<(f32, f32)>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraGpu {
    eye: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    viewport: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BeamGpu {
    apex: [f32; 4],
    direction_length: [f32; 4],
    color_density: [f32; 4],
    bounds: [f32; 4],
    params: [f32; 4],
    u_axis: [f32; 4],
    v_axis: [f32; 4],
    gobo: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PoolGpu {
    apex: [f32; 4],
    axis: [f32; 4],
    u_axis: [f32; 4],
    v_axis: [f32; 4],
    color: [f32; 4],
    bounds: [f32; 4],
    gobo: [f32; 4],
    params: [f32; 4],
}

/// The pack of four floats a beam or pool carries for its gobos.
fn gobo_params(gobos: &[Option<GoboOn>; 2]) -> [f32; 4] {
    let layer = |g: Option<GoboOn>| g.map_or(-1.0, |g| g.layer as f32);
    let angle = |g: Option<GoboOn>| g.map_or(0.0, |g| g.angle);
    [layer(gobos[0]), layer(gobos[1]), angle(gobos[0]), angle(gobos[1])]
}

pub(crate) struct VolumetricResources {
    beam_pipeline: wgpu::RenderPipeline,
    pool_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    beam_buffer: wgpu::Buffer,
    pool_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// The gobo mask texture array currently bound, and which atlas
    /// generation it was built from (0 = the blank placeholder).
    atlas_view: wgpu::TextureView,
    atlas_generation: u64,
}

impl VolumetricResources {
    pub(crate) fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("volumetric beam camera"),
            contents: bytemuck::bytes_of(&CameraGpu::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let beam_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("volumetric beam instances"),
            size: (std::mem::size_of::<BeamGpu>() * MAX_BEAMS) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pool_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("surface pool instances"),
            size: (std::mem::size_of::<PoolGpu>() * MAX_POOLS) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gobo atlas sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let buffer_entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("volumetric beam bindings"),
            entries: &[
                buffer_entry(0, wgpu::BufferBindingType::Uniform),
                buffer_entry(1, wgpu::BufferBindingType::Storage { read_only: true }),
                buffer_entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        // A one-layer open mask stands in until the first atlas arrives, so
        // the bind group is always complete.
        let atlas_view = upload_atlas(device, None, 4, &[]);
        let bind_group = make_bind_group(
            device,
            &bind_group_layout,
            &camera_buffer,
            &beam_buffer,
            &pool_buffer,
            &atlas_view,
            &sampler,
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("volumetric beam shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("volumetric.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("volumetric beam pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = |label, vs, fs| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: vs,
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: fs,
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                multiview: None,
                cache: None,
            })
        };
        let beam_pipeline = pipeline("volumetric beam pipeline", "vs_main", "fs_main");
        let pool_pipeline = pipeline("surface pool pipeline", "vs_pool", "fs_pool");
        Self {
            beam_pipeline,
            pool_pipeline,
            bind_group_layout,
            bind_group,
            camera_buffer,
            beam_buffer,
            pool_buffer,
            sampler,
            atlas_view,
            atlas_generation: 0,
        }
    }

    /// Swap in a newer atlas, if this frame carries one.
    fn refresh_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, atlas: Option<&Arc<AtlasUpload>>) {
        let Some(atlas) = atlas else { return };
        if atlas.generation == self.atlas_generation {
            return;
        }
        let layers: Vec<&[u8]> = atlas.layers.iter().map(|m| m.data.as_slice()).collect();
        self.atlas_view = upload_atlas(device, Some(queue), atlas.size, &layers);
        self.bind_group = make_bind_group(
            device,
            &self.bind_group_layout,
            &self.camera_buffer,
            &self.beam_buffer,
            &self.pool_buffer,
            &self.atlas_view,
            &self.sampler,
        );
        self.atlas_generation = atlas.generation;
    }
}

/// An R8 texture array of `size`×`size` masks. With no layers, one open
/// (all-white) placeholder layer so the binding is valid.
fn upload_atlas(
    device: &wgpu::Device,
    queue: Option<&wgpu::Queue>,
    size: u32,
    layers: &[&[u8]],
) -> wgpu::TextureView {
    let depth = layers.len().max(1) as u32;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gobo mask atlas"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: depth },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    if let Some(queue) = queue {
        let white = vec![255u8; (size * size) as usize];
        let sources: Vec<&[u8]> = if layers.is_empty() { vec![&white] } else { layers.to_vec() };
        for (i, data) in sources.iter().enumerate() {
            if data.len() != (size * size) as usize {
                continue;
            }
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: i as u32 },
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(size), rows_per_image: Some(size) },
                wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            );
        }
    }
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

fn make_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    camera: &wgpu::Buffer,
    beams: &wgpu::Buffer,
    pools: &wgpu::Buffer,
    atlas: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("volumetric beam bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: camera.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: beams.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: pools.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(atlas) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

pub(crate) fn initialize(render_state: &egui_wgpu::RenderState) {
    let resources = VolumetricResources::new(&render_state.device, render_state.target_format);
    render_state
        .renderer
        .write()
        .callback_resources
        .insert(resources);
}

/// Which of the two passes a callback draws. Pools go under the fixture
/// bodies, beams over them, so they are two callbacks in the painter's list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    Pools,
    Beams,
}

struct VolumetricCallback {
    pass: Pass,
    camera: CameraGpu,
    beams: Vec<BeamGpu>,
    pools: Vec<PoolGpu>,
    atlas: Option<Arc<AtlasUpload>>,
}

impl egui_wgpu::CallbackTrait for VolumetricCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(gpu) = resources.get_mut::<VolumetricResources>() {
            gpu.refresh_atlas(device, queue, self.atlas.as_ref());
            queue.write_buffer(&gpu.camera_buffer, 0, bytemuck::bytes_of(&self.camera));
            match self.pass {
                Pass::Beams => queue.write_buffer(&gpu.beam_buffer, 0, bytemuck::cast_slice(&self.beams)),
                Pass::Pools => queue.write_buffer(&gpu.pool_buffer, 0, bytemuck::cast_slice(&self.pools)),
            }
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(gpu) = resources.get::<VolumetricResources>() else {
            return;
        };
        render_pass.set_bind_group(0, &gpu.bind_group, &[]);
        match self.pass {
            Pass::Beams => {
                render_pass.set_pipeline(&gpu.beam_pipeline);
                render_pass.draw(0..6, 0..self.beams.len() as u32);
            }
            Pass::Pools => {
                render_pass.set_pipeline(&gpu.pool_pipeline);
                render_pass.draw(0..6, 0..self.pools.len() as u32);
            }
        }
    }
}

/// The GPU callbacks for this frame's beams: the pools they throw on the
/// stage and ground (to paint under the fixture bodies), and the beams
/// themselves (over everything). Bounding each cone before it reaches the
/// GPU avoids evaluating the shader over the whole stage canvas.
pub(crate) fn paint_callbacks(
    cam: &Camera,
    rect: Rect,
    specs: &[BeamSpec],
    atlas: Option<Arc<AtlasUpload>>,
) -> (Option<PaintCallback>, Option<PaintCallback>) {
    let (pools, beams) = build_callbacks(cam, rect, specs, atlas);
    let wrap = |c: VolumetricCallback| egui_wgpu::Callback::new_paint_callback(rect, c);
    (pools.map(wrap), beams.map(wrap))
}

/// The two passes as raw callbacks, before egui wraps them (so a test can
/// drive `prepare` and `paint` itself).
fn build_callbacks(
    cam: &Camera,
    rect: Rect,
    specs: &[BeamSpec],
    atlas: Option<Arc<AtlasUpload>>,
) -> (Option<VolumetricCallback>, Option<VolumetricCallback>) {
    if specs.is_empty() || rect.width() < 2.0 || rect.height() < 2.0 {
        return (None, None);
    }
    let (right, up, forward) = cam.basis();
    let eye = cam.eye();
    let camera = CameraGpu {
        eye: [eye.x, eye.y, eye.z, 0.0],
        right: [right.x, right.y, right.z, 0.0],
        up: [up.x, up.y, up.z, 0.0],
        forward: [forward.x, forward.y, forward.z, 0.0],
        viewport: [
            rect.width() / rect.height(),
            (cam.fov_y * 0.5).tan(),
            0.0,
            0.0,
        ],
    };
    let to_bounds = |points: &mut dyn Iterator<Item = V3>| -> Option<[f32; 4]> {
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        // A shape that reaches past the near plane cannot be bounded by
        // projecting points: the ones behind the camera drop out and the box
        // closes over live pixels, slicing the shape along a hard straight
        // edge. Those cover the whole viewport instead.
        let mut near_clipped = false;
        for point in points {
            match cam.project(rect, point) {
                Some((p, depth)) if depth > 0.2 => {
                    min_x = min_x.min(p.x);
                    min_y = min_y.min(p.y);
                    max_x = max_x.max(p.x);
                    max_y = max_y.max(p.y);
                }
                _ => near_clipped = true,
            }
        }
        if !min_x.is_finite() && !near_clipped {
            return None;
        }
        // The box only tracks sampled points, so leave room for the
        // silhouette that bulges between them rather than clipping it.
        let pad = 8.0;
        let bounds = if near_clipped {
            [0.0, 0.0, 1.0, 1.0]
        } else {
            [
                ((min_x - pad - rect.left()) / rect.width()).clamp(0.0, 1.0),
                ((min_y - pad - rect.top()) / rect.height()).clamp(0.0, 1.0),
                ((max_x + pad - rect.left()) / rect.width()).clamp(0.0, 1.0),
                ((max_y + pad - rect.top()) / rect.height()).clamp(0.0, 1.0),
            ]
        };
        (bounds[2] > bounds[0] && bounds[3] > bounds[1]).then_some(bounds)
    };

    let mut beams = Vec::with_capacity(specs.len().min(MAX_BEAMS));
    let mut pools = Vec::with_capacity(specs.len().min(MAX_POOLS));
    for spec in specs.iter().take(MAX_BEAMS) {
        let helper = if spec.dir.y.abs() > 0.9 { v3(1.0, 0.0, 0.0) } else { v3(0.0, 1.0, 0.0) };
        let u = spec.dir.cross(helper).norm();
        let v = spec.dir.cross(u).norm();
        let spread = spec.half_angle.tan();
        let aperture = 0.05;

        let vb = vis_curve(spec.brightness);
        let m = spec.brightness.max(1e-3);
        let hue_scale = (vb / m).min(4.0);
        let rgb = [
            (spec.color.r() as f32 / 255.0 * hue_scale).min(1.0),
            (spec.color.g() as f32 / 255.0 * hue_scale).min(1.0),
            (spec.color.b() as f32 / 255.0 * hue_scale).min(1.0),
        ];
        let gobo = gobo_params(&spec.gobos);

        let ring = |slice_len: f32, frac: f32| {
            let center = spec.apex + spec.dir * (slice_len * frac);
            let radius = aperture + spread * slice_len * frac;
            (0..RING_POINTS).map(move |k| {
                let angle = k as f32 / RING_POINTS as f32 * std::f32::consts::TAU;
                center + u * (angle.cos() * radius) + v * (angle.sin() * radius)
            })
        };
        let mut cone_points = (0..=SLICES).flat_map(|s| ring(spec.len, s as f32 / SLICES as f32));
        if let Some(bounds) = to_bounds(&mut cone_points) {
            let density = (0.24 + vb * 1.15) * spec.opacity.max(0.0);
            beams.push(BeamGpu {
                apex: [spec.apex.x, spec.apex.y, spec.apex.z, 0.0],
                direction_length: [spec.dir.x, spec.dir.y, spec.dir.z, spec.len],
                color_density: [rgb[0], rgb[1], rgb[2], density],
                bounds,
                params: [spread, aperture, 0.0, 0.0],
                u_axis: [u.x, u.y, u.z, 0.0],
                v_axis: [v.x, v.y, v.z, 0.0],
                gobo,
            });
        }

        let Some((hit_t, plane_y)) = spec.surface else { continue };
        if pools.len() >= MAX_POOLS {
            continue;
        }
        // The footprint: where the cone's edge rays meet the surface. An
        // edge ray that never comes down (a near-horizontal beam) leaves the
        // footprint unbounded, so that pool covers the viewport.
        let mut unbounded = false;
        let mut footprint: Vec<V3> = Vec::with_capacity(RING_POINTS + 1);
        footprint.push(spec.apex + spec.dir * hit_t);
        for k in 0..RING_POINTS {
            let angle = k as f32 / RING_POINTS as f32 * std::f32::consts::TAU;
            let edge = (spec.dir + u * (angle.cos() * spread) + v * (angle.sin() * spread)).norm();
            if edge.y >= -0.02 {
                unbounded = true;
                break;
            }
            let t = (plane_y - spec.apex.y) / edge.y;
            if t <= 0.0 {
                unbounded = true;
                break;
            }
            footprint.push(spec.apex + edge * t);
        }
        let bounds = if unbounded {
            Some([0.0, 0.0, 1.0, 1.0])
        } else {
            to_bounds(&mut footprint.into_iter())
        };
        let Some(bounds) = bounds else { continue };
        let peak = ((28.0 + vb * 105.0) * spec.opacity.max(0.0)).clamp(0.0, 190.0) / 255.0;
        pools.push(PoolGpu {
            apex: [spec.apex.x, spec.apex.y, spec.apex.z, plane_y],
            axis: [spec.dir.x, spec.dir.y, spec.dir.z, spread],
            u_axis: [u.x, u.y, u.z, 0.0],
            v_axis: [v.x, v.y, v.z, 0.0],
            color: [rgb[0], rgb[1], rgb[2], peak],
            bounds,
            gobo,
            params: [aperture, hit_t, 0.0, 0.0],
        });
    }
    let callback = |pass: Pass, beams: Vec<BeamGpu>, pools: Vec<PoolGpu>| {
        let any = match pass {
            Pass::Beams => !beams.is_empty(),
            Pass::Pools => !pools.is_empty(),
        };
        any.then(|| VolumetricCallback { pass, camera, beams, pools, atlas: atlas.clone() })
    };
    let pool_callback = callback(Pass::Pools, Vec::new(), pools);
    let beam_callback = callback(Pass::Beams, beams, Vec::new());
    (pool_callback, beam_callback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gobo::{Mask, MASK_SIZE};
    use egui_wgpu::CallbackTrait;
    use wgpu::naga;

    /// Poll a wgpu future to completion on this thread; the native backends
    /// resolve them without an executor.
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

    /// Render one gobo beam and one plain beam through the real pipelines
    /// into an offscreen texture, on whatever GPU the test machine has, and
    /// check both lands: a warm striped pool and a cool plain one. Writes
    /// `target/gobo_render.png` for eyeballing. Skips with a note when there
    /// is no adapter (CI without a GPU).
    #[test]
    fn renders_gobo_beam_and_pool_offscreen() {
        let instance = wgpu::Instance::default();
        let Some(adapter) = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            eprintln!("no GPU adapter — skipping");
            return;
        };
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .expect("device");
        // A validation error or device loss fails the test loudly instead of
        // surfacing as a later, unrelated allocation failure.
        device.on_uncaptured_error(Box::new(|e| panic!("wgpu error: {e}")));

        const W: u32 = 512;
        const H: u32 = 384;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut resources = egui_wgpu::CallbackResources::default();
        resources.insert(VolumetricResources::new(&device, format));

        // A synthetic gobo: a disc of vertical bars, so the projection has
        // structure to find.
        let n = MASK_SIZE as usize;
        let mut data = vec![0u8; n * n];
        for y in 0..n {
            for x in 0..n {
                let dx = x as f32 - n as f32 * 0.5 + 0.5;
                let dy = y as f32 - n as f32 * 0.5 + 0.5;
                let inside = (dx * dx + dy * dy).sqrt() < n as f32 * 0.48;
                if inside && (x / 20) % 2 == 0 {
                    data[y * n + x] = 255;
                }
            }
        }
        let atlas = Arc::new(AtlasUpload {
            generation: 1,
            size: MASK_SIZE,
            layers: vec![Arc::new(Mask { size: MASK_SIZE, data })],
        });

        let cam = Camera::default();
        let rect = Rect::from_min_size(eframe::egui::pos2(0.0, 0.0), eframe::egui::vec2(W as f32, H as f32));
        let beam = |apex: V3, at: V3, color: Color32, gobo: Option<GoboOn>| {
            let dir = (at - apex).norm();
            let hit_t = (at - apex).len();
            BeamSpec {
                apex,
                dir,
                len: hit_t,
                half_angle: 12f32.to_radians(),
                color,
                brightness: 1.0,
                opacity: 1.0,
                gobos: [gobo, None],
                surface: Some((hit_t, 0.0)),
            }
        };
        let specs = [
            beam(
                v3(-2.5, 4.5, 0.0),
                v3(-1.0, 0.0, 1.0),
                Color32::from_rgb(255, 215, 150),
                Some(GoboOn { layer: 0, angle: 0.4 }),
            ),
            beam(v3(3.0, 4.5, 0.0), v3(2.0, 0.0, 1.0), Color32::from_rgb(70, 130, 255), None),
        ];
        let (pools, beams) = build_callbacks(&cam, rect, &specs, Some(atlas));
        let pools = pools.expect("both beams land on the floor");
        let beams = beams.expect("both beams are visible");
        assert_eq!(beams.beams.len(), 2);
        assert_eq!(pools.pools.len(), 2);
        assert!(beams.beams[0].gobo[0] >= 0.0 && beams.beams[1].gobo[0] < 0.0);

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen stage"),
            size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [W, H], pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        pools.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        beams.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("offscreen stage pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.05, g: 0.05, b: 0.06, a: 1.0 }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            let info = || eframe::egui::PaintCallbackInfo {
                viewport: rect,
                clip_rect: rect,
                pixels_per_point: 1.0,
                screen_size_px: [W, H],
            };
            pools.paint(info(), &mut pass, &resources);
            beams.paint(info(), &mut pass, &resources);
        }
        let bytes_per_row = W * 4;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offscreen readback"),
            size: (bytes_per_row * H) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
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
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback"));
        device.poll(wgpu::Maintain::Wait);
        let pixels = slice.get_mapped_range().to_vec();
        readback.unmap();

        let _ = std::fs::create_dir_all("target");
        image::RgbaImage::from_raw(W, H, pixels.clone())
            .expect("image")
            .save("target/gobo_render.png")
            .expect("save");

        let lit = |p: &[u8]| p[0] as u32 + p[1] as u32 + p[2] as u32 > 150;
        let lit_count = pixels.chunks(4).filter(|p| lit(p)).count();
        assert!(lit_count > 500, "only {lit_count} lit pixels");
        let warm = pixels.chunks(4).filter(|p| lit(p) && p[0] > p[2] + 40).count();
        let cool = pixels.chunks(4).filter(|p| lit(p) && p[2] > p[0] + 40).count();
        assert!(warm > 200, "the gobo beam barely shows: {warm} warm pixels");
        assert!(cool > 200, "the plain beam barely shows: {cool} cool pixels");
    }

    /// The shader is compiled at startup; a mistake in it would otherwise
    /// only show up as the app failing to open. Parse and validate it here
    /// the way wgpu will, and check both pipelines' entry points exist.
    #[test]
    fn shader_validates() {
        let src = include_str!("volumetric.wgsl");
        let module = naga::front::wgsl::parse_str(src)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("shader validates");
        let names: Vec<&str> = module.entry_points.iter().map(|e| e.name.as_str()).collect();
        for want in ["vs_main", "fs_main", "vs_pool", "fs_pool"] {
            assert!(names.contains(&want), "missing entry point {want} in {names:?}");
        }
    }

    /// The GPU structs are laid out as vec4 columns, so their sizes must be
    /// what the WGSL side declares.
    #[test]
    fn gpu_structs_are_vec4_multiples() {
        assert_eq!(std::mem::size_of::<BeamGpu>(), 8 * 16);
        assert_eq!(std::mem::size_of::<PoolGpu>(), 8 * 16);
        assert_eq!(std::mem::size_of::<CameraGpu>(), 5 * 16);
    }

    /// A beam with no gobo encodes as layer -1 on both slots, which the
    /// shader reads as "pass everything".
    #[test]
    fn gobo_params_encode_absence() {
        assert_eq!(gobo_params(&[None, None]), [-1.0, -1.0, 0.0, 0.0]);
        let on = Some(GoboOn { layer: 7, angle: 1.5 });
        assert_eq!(gobo_params(&[on, None]), [7.0, -1.0, 1.5, 0.0]);
    }
}
