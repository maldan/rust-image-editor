use bytemuck::{Pod, Zeroable};
use glam::Vec2;
use mega_ui::Rect;
use wgpu::util::DeviceExt;

use crate::tiles::DirtyTiles;
use crate::view::CanvasView;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    viewport: [f32; 2],
    center: [f32; 2],
    image_size: [f32; 2],
    zoom: f32,
    rotation: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SolidUniforms {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SolidVertex {
    pos: [f32; 2],
    color: [f32; 4],
}

#[derive(Clone, Copy, Default)]
pub struct CanvasOverlay {
    pub brush_center: Option<Vec2>,
    pub brush_radius: f32,
    pub selection: Option<[Vec2; 4]>,
}

pub struct CanvasRenderer {
    pipeline: wgpu::RenderPipeline,
    solid_pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    solid_bind_layout: wgpu::BindGroupLayout,
    uniform_buf: wgpu::Buffer,
    solid_uniform_buf: wgpu::Buffer,
    solid_vertex_buf: wgpu::Buffer,
    sampler: wgpu::Sampler,
    texture: Option<wgpu::Texture>,
    bind_group: Option<wgpu::BindGroup>,
    solid_bind_group: wgpu::BindGroup,
    size: (u32, u32),
}

impl CanvasRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("canvas"),
            source: wgpu::ShaderSource::Wgsl(include_str!("canvas_gpu.wgsl").into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canvas bind"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });
        let solid_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canvas solid bind"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canvas pipeline layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let solid_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canvas solid layout"),
            bind_group_layouts: &[Some(&solid_bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("canvas pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let solid_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("canvas solid pipeline"),
            layout: Some(&solid_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_solid"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SolidVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_solid"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("canvas uniforms"),
            contents: bytemuck::bytes_of(&Uniforms {
                viewport: [1.0, 1.0],
                center: [0.0, 0.0],
                image_size: [1.0, 1.0],
                zoom: 1.0,
                rotation: 0.0,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let solid_uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("canvas solid uniforms"),
            contents: bytemuck::bytes_of(&SolidUniforms {
                viewport: [1.0, 1.0],
                _pad: [0.0; 2],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let solid_vertex_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("canvas solid verts"),
            size: (std::mem::size_of::<SolidVertex>() * 4096) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("canvas nearest"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let solid_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("canvas solid bg"),
            layout: &solid_bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: solid_uniform_buf.as_entire_binding(),
            }],
        });
        Self {
            pipeline,
            solid_pipeline,
            bind_layout,
            solid_bind_layout,
            uniform_buf,
            solid_uniform_buf,
            solid_vertex_buf,
            sampler,
            texture: None,
            bind_group: None,
            solid_bind_group,
            size: (0, 0),
        }
    }

    pub fn clear(&mut self) {
        self.texture = None;
        self.bind_group = None;
        self.size = (0, 0);
    }

    pub fn has_image(&self) -> bool {
        self.texture.is_some()
    }

    pub fn ensure_size(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.size == (width, height) && self.texture.is_some() {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("canvas image"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("canvas bind group"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.texture = Some(texture);
        self.bind_group = Some(bind_group);
        self.size = (width, height);
        let _ = &self.solid_bind_layout;
    }

    pub fn upload_dirty(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pixels: &[u8],
        width: u32,
        height: u32,
        dirty: &mut DirtyTiles,
    ) {
        if width == 0 || height == 0 || pixels.len() < width as usize * height as usize * 4 {
            return;
        }
        let need_full = self.size != (width, height) || self.texture.is_none();
        self.ensure_size(device, width, height);
        let Some(texture) = self.texture.as_ref() else {
            return;
        };
        if need_full {
            dirty.mark_all();
        }
        if dirty.is_empty() {
            return;
        }
        let bytes_per_row = width * 4;
        for (x, y, w, h) in dirty.drain() {
            if w == 0 || h == 0 {
                continue;
            }
            let offset = ((y * width + x) * 4) as usize;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels[offset..],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    pub fn render(
        &self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        viewport: Vec2,
        canvas_rect: Rect,
        view: CanvasView,
        overlay: CanvasOverlay,
    ) {
        if canvas_rect.width() < 1.0 || canvas_rect.height() < 1.0 {
            return;
        }
        let x = canvas_rect.min.x.floor().max(0.0) as u32;
        let y = canvas_rect.min.y.floor().max(0.0) as u32;
        let w = canvas_rect.width().ceil().max(1.0) as u32;
        let h = canvas_rect.height().ceil().max(1.0) as u32;
        let max_w = viewport.x.max(1.0) as u32;
        let max_h = viewport.y.max(1.0) as u32;
        let x = x.min(max_w.saturating_sub(1));
        let y = y.min(max_h.saturating_sub(1));
        let w = w.min(max_w.saturating_sub(x)).max(1);
        let h = h.min(max_h.saturating_sub(y)).max(1);
        pass.set_scissor_rect(x, y, w, h);

        if let Some(bind_group) = self.bind_group.as_ref() {
            queue.write_buffer(
                &self.uniform_buf,
                0,
                bytemuck::bytes_of(&Uniforms {
                    viewport: [viewport.x, viewport.y],
                    center: [view.center.x, view.center.y],
                    image_size: [view.width as f32, view.height as f32],
                    zoom: view.zoom,
                    rotation: view.rotation,
                }),
            );
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        let mut verts = Vec::new();
        if let Some(corners) = overlay.selection {
            let white = [0.95, 0.95, 0.95, 1.0];
            let black = [0.02, 0.02, 0.02, 1.0];
            for i in 0..4 {
                push_line(&mut verts, corners[i], corners[(i + 1) % 4], 2.0, black);
                push_line(&mut verts, corners[i], corners[(i + 1) % 4], 1.0, white);
            }
        }
        if let Some(center) = overlay.brush_center {
            let r = overlay.brush_radius.max(2.0);
            push_circle(&mut verts, center, r, 2.0, [0.02, 0.02, 0.02, 1.0]);
            push_circle(&mut verts, center, r, 1.0, [0.95, 0.95, 0.95, 1.0]);
        }
        if verts.is_empty() {
            return;
        }
        let bytes = bytemuck::cast_slice(&verts);
        if bytes.len() as u64 > self.solid_vertex_buf.size() {
            return;
        }
        queue.write_buffer(&self.solid_uniform_buf, 0, bytemuck::bytes_of(&SolidUniforms {
            viewport: [viewport.x, viewport.y],
            _pad: [0.0; 2],
        }));
        queue.write_buffer(&self.solid_vertex_buf, 0, bytes);
        pass.set_pipeline(&self.solid_pipeline);
        pass.set_bind_group(0, &self.solid_bind_group, &[]);
        pass.set_vertex_buffer(0, self.solid_vertex_buf.slice(..));
        pass.draw(0..verts.len() as u32, 0..1);
    }
}

fn push_line(out: &mut Vec<SolidVertex>, a: Vec2, b: Vec2, thickness: f32, color: [f32; 4]) {
    let delta = b - a;
    let len = delta.length();
    if len < 0.5 {
        return;
    }
    let dir = delta / len;
    let n = Vec2::new(-dir.y, dir.x) * (thickness * 0.5);
    let p0 = a - n;
    let p1 = a + n;
    let p2 = b + n;
    let p3 = b - n;
    for p in [p0, p1, p2, p0, p2, p3] {
        out.push(SolidVertex {
            pos: [p.x, p.y],
            color,
        });
    }
}

fn push_circle(out: &mut Vec<SolidVertex>, center: Vec2, radius: f32, thickness: f32, color: [f32; 4]) {
    let steps = ((radius * 2.0).ceil() as i32).clamp(24, 96);
    for i in 0..steps {
        let a0 = i as f32 / steps as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / steps as f32 * std::f32::consts::TAU;
        let p0 = center + Vec2::new(a0.cos(), a0.sin()) * radius;
        let p1 = center + Vec2::new(a1.cos(), a1.sin()) * radius;
        push_line(out, p0, p1, thickness, color);
    }
}
