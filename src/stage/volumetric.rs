//! Metal/wgpu-backed analytic volumetric beams. The CPU computes a tight
//! screen-space bound for each finite cone; the fragment shader evaluates a
//! smooth participating-media density along each camera ray.

use bytemuck::{Pod, Zeroable};
use eframe::egui::{Color32, PaintCallback, Rect};
use eframe::egui_wgpu::{self, wgpu};
use wgpu::util::DeviceExt;

use super::fixture::vis_curve;
use super::math::{Camera, V3};

const MAX_BEAMS: usize = 128;
const RING_POINTS: usize = 24;
const SLICES: usize = 8;

#[derive(Clone, Copy)]
pub(crate) struct BeamSpec {
    pub apex: V3,
    pub dir: V3,
    pub len: f32,
    pub half_angle: f32,
    pub color: Color32,
    pub brightness: f32,
    pub opacity: f32,
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
}

pub(crate) struct VolumetricResources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    beam_buffer: wgpu::Buffer,
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
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("volumetric beam bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("volumetric beam bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: beam_buffer.as_entire_binding(),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("volumetric beam shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("volumetric.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("volumetric beam pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("volumetric beam pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview: None,
            cache: None,
        });
        Self {
            pipeline,
            bind_group,
            camera_buffer,
            beam_buffer,
        }
    }
}

pub(crate) fn initialize(render_state: &egui_wgpu::RenderState) {
    let resources = VolumetricResources::new(&render_state.device, render_state.target_format);
    render_state
        .renderer
        .write()
        .callback_resources
        .insert(resources);
}

struct VolumetricCallback {
    camera: CameraGpu,
    beams: Vec<BeamGpu>,
}

impl egui_wgpu::CallbackTrait for VolumetricCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(gpu) = resources.get::<VolumetricResources>() {
            queue.write_buffer(&gpu.camera_buffer, 0, bytemuck::bytes_of(&self.camera));
            queue.write_buffer(&gpu.beam_buffer, 0, bytemuck::cast_slice(&self.beams));
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
        render_pass.set_pipeline(&gpu.pipeline);
        render_pass.set_bind_group(0, &gpu.bind_group, &[]);
        render_pass.draw(0..6, 0..self.beams.len() as u32);
    }
}

/// Build one GPU callback for all visible beams. Bounding each cone before it
/// reaches the GPU avoids evaluating the shader over the whole stage canvas.
pub(crate) fn paint_callback(
    cam: &Camera,
    rect: Rect,
    specs: &[BeamSpec],
) -> Option<PaintCallback> {
    if specs.is_empty() || rect.width() < 2.0 || rect.height() < 2.0 {
        return None;
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

    let mut beams = Vec::with_capacity(specs.len().min(MAX_BEAMS));
    for spec in specs.iter().take(MAX_BEAMS) {
        let helper = if spec.dir.y.abs() > 0.9 {
            super::math::v3(1.0, 0.0, 0.0)
        } else {
            super::math::v3(0.0, 1.0, 0.0)
        };
        let u = spec.dir.cross(helper).norm();
        let v = spec.dir.cross(u).norm();
        let spread = spec.half_angle.tan();
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        // A cone that reaches past the near plane cannot be bounded by
        // projecting points: the ones behind the camera drop out and the box
        // closes over live haze, slicing the beam along a hard straight edge.
        let mut near_clipped = false;
        for slice in 0..=SLICES {
            let frac = slice as f32 / SLICES as f32;
            let center = spec.apex + spec.dir * (spec.len * frac);
            let radius = 0.05 + spread * spec.len * frac;
            for k in 0..RING_POINTS {
                let angle = k as f32 / RING_POINTS as f32 * std::f32::consts::TAU;
                let point = center + u * (angle.cos() * radius) + v * (angle.sin() * radius);
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
        }
        if !min_x.is_finite() && !near_clipped {
            continue;
        }
        // The box only tracks sampled rings, so leave room for the silhouette
        // that bulges between them rather than clipping it into a line.
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
        if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
            continue;
        }

        let vb = vis_curve(spec.brightness);
        let m = spec.brightness.max(1e-3);
        let hue_scale = (vb / m).min(4.0);
        let rgb = [
            (spec.color.r() as f32 / 255.0 * hue_scale).min(1.0),
            (spec.color.g() as f32 / 255.0 * hue_scale).min(1.0),
            (spec.color.b() as f32 / 255.0 * hue_scale).min(1.0),
        ];
        let density = (0.24 + vb * 1.15) * spec.opacity.max(0.0);
        beams.push(BeamGpu {
            apex: [spec.apex.x, spec.apex.y, spec.apex.z, 0.0],
            direction_length: [spec.dir.x, spec.dir.y, spec.dir.z, spec.len],
            color_density: [rgb[0], rgb[1], rgb[2], density],
            bounds,
            params: [spread, 0.05, 0.0, 0.0],
        });
    }
    if beams.is_empty() {
        return None;
    }
    Some(egui_wgpu::Callback::new_paint_callback(
        rect,
        VolumetricCallback { camera, beams },
    ))
}
