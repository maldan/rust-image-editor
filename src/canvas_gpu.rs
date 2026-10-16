use bytemuck::{Pod, Zeroable};
use glam::Vec2;
use mega_ui::Rect;
use wgpu::util::DeviceExt;

use crate::tiles::DirtyTiles;
use crate::view::CanvasView;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ViewUniforms {
    viewport: [f32; 2],
    center: [f32; 2],
    image_size: [f32; 2],
    zoom: f32,
    rotation: f32,
    opacity: f32,
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FloatUniforms {
    viewport: [f32; 2],
    view_center: [f32; 2],
    image_size: [f32; 2],
    zoom: f32,
    view_rotation: f32,
    float_center: [f32; 2],
    float_half: [f32; 2],
    float_rotation: f32,
    opacity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SelUniforms {
    viewport: [f32; 2],
    center: [f32; 2],
    image_size: [f32; 2],
    zoom: f32,
    rotation: f32,
    has_selection: f32,
    dim_outside: f32,
    time: f32,
    _pad: f32,
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
pub struct LayerDraw {
    pub visible: bool,
    pub opacity: f32,
}

#[derive(Clone, Default)]
pub struct CanvasOverlay {
    pub brush_center: Option<Vec2>,
    pub brush_radius: f32,
    pub selection_rect: Option<[Vec2; 4]>,
    pub transform_corners: Option<[Vec2; 4]>,
    pub transform_rotate: Option<Vec2>,
    pub dim_outside: bool,
    pub floating: Option<FloatingDraw>,
    pub layers: Vec<LayerDraw>,
}

#[derive(Clone, Copy)]
pub struct FloatingDraw {
    pub center: Vec2,
    pub half: Vec2,
    pub rotation: f32,
    pub opacity: f32,
}

struct LayerGpu {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
}

pub struct CanvasRenderer {
    checker_pipeline: wgpu::RenderPipeline,
    layer_pipeline: wgpu::RenderPipeline,
    float_pipeline: wgpu::RenderPipeline,
    sel_pipeline: wgpu::RenderPipeline,
    solid_pipeline: wgpu::RenderPipeline,
    view_bind_layout: wgpu::BindGroupLayout,
    float_bind_layout: wgpu::BindGroupLayout,
    sel_bind_layout: wgpu::BindGroupLayout,
    #[allow(dead_code)]
    solid_bind_layout: wgpu::BindGroupLayout,
    view_uniform_buf: wgpu::Buffer,
    float_uniform_buf: wgpu::Buffer,
    sel_uniform_buf: wgpu::Buffer,
    solid_uniform_buf: wgpu::Buffer,
    solid_vertex_buf: wgpu::Buffer,
    sampler_near: wgpu::Sampler,
    sampler_linear: wgpu::Sampler,
    layers: Vec<Option<LayerGpu>>,
    floating_texture: Option<wgpu::Texture>,
    floating_bind_group: Option<wgpu::BindGroup>,
    floating_size: (u32, u32),
    sel_texture: Option<wgpu::Texture>,
    sel_bind_group: Option<wgpu::BindGroup>,
    /// Kept alive for `checker_bind_group`.
    #[allow(dead_code)]
    empty_img_texture: wgpu::Texture,
    /// Kept alive for `empty_sel_bind_group`.
    #[allow(dead_code)]
    empty_sel_texture: wgpu::Texture,
    empty_sel_bind_group: wgpu::BindGroup,
    checker_bind_group: wgpu::BindGroup,
    solid_bind_group: wgpu::BindGroup,
    sel_size: (u32, u32),
    has_selection: bool,
    size: (u32, u32),
}

impl CanvasRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("canvas"),
            source: wgpu::ShaderSource::Wgsl(include_str!("canvas_gpu.wgsl").into()),
        });

        let view_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canvas view bind"),
            entries: &[
                buffer_entry(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                texture_entry(1, false),
                sampler_entry(2, false),
            ],
        });
        let float_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canvas float bind"),
            entries: &[
                buffer_entry(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                texture_entry(1, false),
                sampler_entry(2, false),
            ],
        });
        let sel_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canvas sel bind"),
            entries: &[
                buffer_entry(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                texture_entry(1, true),
                sampler_entry(2, true),
            ],
        });
        let solid_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canvas solid bind"),
            entries: &[buffer_entry(0, wgpu::ShaderStages::VERTEX)],
        });

        let view_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canvas view layout"),
            bind_group_layouts: &[Some(&view_bind_layout)],
            immediate_size: 0,
        });
        let float_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canvas float layout"),
            bind_group_layouts: &[Some(&float_bind_layout)],
            immediate_size: 0,
        });
        let sel_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canvas sel layout"),
            bind_group_layouts: &[Some(&sel_bind_layout)],
            immediate_size: 0,
        });
        let solid_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canvas solid layout"),
            bind_group_layouts: &[Some(&solid_bind_layout)],
            immediate_size: 0,
        });

        let checker_pipeline = make_pipeline(
            device,
            &view_layout,
            &shader,
            "vs_image",
            "fs_checker",
            format,
            wgpu::BlendState::REPLACE,
            &[],
        );
        let layer_pipeline = make_pipeline(
            device,
            &view_layout,
            &shader,
            "vs_image",
            "fs_layer",
            format,
            wgpu::BlendState::ALPHA_BLENDING,
            &[],
        );
        let float_pipeline = make_pipeline(
            device,
            &float_layout,
            &shader,
            "vs_float",
            "fs_float",
            format,
            wgpu::BlendState::ALPHA_BLENDING,
            &[],
        );
        let sel_pipeline = make_pipeline(
            device,
            &sel_layout,
            &shader,
            "vs_sel",
            "fs_sel",
            format,
            wgpu::BlendState::ALPHA_BLENDING,
            &[],
        );
        let solid_pipeline = make_pipeline(
            device,
            &solid_layout,
            &shader,
            "vs_solid",
            "fs_solid",
            format,
            wgpu::BlendState::ALPHA_BLENDING,
            &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<SolidVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
            })],
        );

        let view_uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("canvas view uniforms"),
            size: std::mem::size_of::<ViewUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let float_uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("canvas float uniforms"),
            size: std::mem::size_of::<FloatUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sel_uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("canvas sel uniforms"),
            size: std::mem::size_of::<SelUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
            size: (std::mem::size_of::<SolidVertex>() * 16384) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let sampler_near = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("canvas nearest"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let sampler_linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("canvas sel linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let empty_img = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("canvas empty img"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let empty_img_view = empty_img.create_view(&Default::default());
        let checker_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("canvas checker bg"),
            layout: &view_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: view_uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&empty_img_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler_near),
                },
            ],
        });

        let empty_sel_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("canvas empty sel"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let empty_sel_view = empty_sel_texture.create_view(&Default::default());
        let empty_sel_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("canvas empty sel bg"),
            layout: &sel_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: sel_uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&empty_sel_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler_linear),
                },
            ],
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
            checker_pipeline,
            layer_pipeline,
            float_pipeline,
            sel_pipeline,
            solid_pipeline,
            view_bind_layout,
            float_bind_layout,
            sel_bind_layout,
            solid_bind_layout,
            view_uniform_buf,
            float_uniform_buf,
            sel_uniform_buf,
            solid_uniform_buf,
            solid_vertex_buf,
            sampler_near,
            sampler_linear,
            layers: Vec::new(),
            floating_texture: None,
            floating_bind_group: None,
            floating_size: (0, 0),
            sel_texture: None,
            sel_bind_group: None,
            empty_img_texture: empty_img,
            empty_sel_texture,
            empty_sel_bind_group,
            checker_bind_group,
            solid_bind_group,
            sel_size: (0, 0),
            has_selection: false,
            size: (0, 0),
        }
    }

    pub fn clear(&mut self) {
        self.layers.clear();
        self.floating_texture = None;
        self.floating_bind_group = None;
        self.floating_size = (0, 0);
        self.size = (0, 0);
        self.has_selection = false;
    }

    pub fn has_image(&self) -> bool {
        self.layers.iter().any(|l| l.is_some())
    }

    pub fn ensure_layers(
        &mut self,
        device: &wgpu::Device,
        count: usize,
        width: u32,
        height: u32,
    ) {
        let width = width.max(1);
        let height = height.max(1);
        let size_changed = self.size != (width, height);
        if size_changed {
            self.layers.clear();
            self.size = (width, height);
        }
        while self.layers.len() < count {
            self.layers.push(None);
        }
        if self.layers.len() > count {
            self.layers.truncate(count);
        }
        for slot in self.layers.iter_mut() {
            let need = match slot {
                Some(layer) => layer.size != (width, height),
                None => true,
            };
            if !need {
                continue;
            }
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("canvas layer"),
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
            let view = texture.create_view(&Default::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("canvas layer bg"),
                layout: &self.view_bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.view_uniform_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler_near),
                    },
                ],
            });
            *slot = Some(LayerGpu {
                texture,
                bind_group,
                size: (width, height),
            });
        }
    }

    pub fn upload_layer_dirty(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layer_index: usize,
        pixels: &[u8],
        width: u32,
        height: u32,
        dirty: &mut DirtyTiles,
    ) {
        if width == 0 || height == 0 || pixels.len() < width as usize * height as usize * 4 {
            return;
        }
        self.ensure_layers(device, layer_index + 1, width, height);
        let Some(Some(layer)) = self.layers.get(layer_index) else {
            return;
        };
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
                    texture: &layer.texture,
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

    pub fn upload_all_layers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layer_pixels: &[&[u8]],
        width: u32,
        height: u32,
    ) {
        self.ensure_layers(device, layer_pixels.len(), width, height);
        for (i, pixels) in layer_pixels.iter().enumerate() {
            let mut dirty = DirtyTiles::default();
            dirty.resize(width, height);
            dirty.mark_all();
            self.upload_layer_dirty(device, queue, i, pixels, width, height, &mut dirty);
        }
    }

    pub fn set_floating(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) {
        if width == 0 || height == 0 || pixels.len() < width as usize * height as usize * 4 {
            self.clear_floating();
            return;
        }
        let need = self.floating_size != (width, height) || self.floating_texture.is_none();
        if need {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("canvas floating"),
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
            let view = texture.create_view(&Default::default());
            self.floating_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("canvas floating bg"),
                layout: &self.float_bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.float_uniform_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler_near),
                    },
                ],
            }));
            self.floating_texture = Some(texture);
            self.floating_size = (width, height);
        }
        let Some(texture) = self.floating_texture.as_ref() else {
            return;
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn clear_floating(&mut self) {
        self.floating_texture = None;
        self.floating_bind_group = None;
        self.floating_size = (0, 0);
    }

    pub fn set_selection_mask(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mask: Option<&[u8]>,
        width: u32,
        height: u32,
    ) {
        let Some(mask) = mask.filter(|m| !m.is_empty() && width > 0 && height > 0) else {
            self.has_selection = false;
            return;
        };
        let need_alloc = self.sel_size != (width, height) || self.sel_texture.is_none();
        if need_alloc {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("canvas selection"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.sel_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("canvas sel bg"),
                layout: &self.sel_bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.sel_uniform_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                    },
                ],
            }));
            self.sel_texture = Some(texture);
            self.sel_size = (width, height);
        }
        self.has_selection = true;
        let expected = width as usize * height as usize;
        if mask.len() < expected {
            return;
        }
        let Some(sel_texture) = self.sel_texture.as_ref() else {
            return;
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: sel_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &mask[..expected],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn render(
        &self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        viewport: Vec2,
        canvas_rect: Rect,
        view: CanvasView,
        overlay: CanvasOverlay,
        time: f32,
    ) {
        if canvas_rect.width() < 1.0 || canvas_rect.height() < 1.0 || view.width == 0 {
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

        let view_u = ViewUniforms {
            viewport: [viewport.x, viewport.y],
            center: [view.center.x, view.center.y],
            image_size: [view.width as f32, view.height as f32],
            zoom: view.zoom,
            rotation: view.rotation,
            opacity: 1.0,
            _pad: 0.0,
        };
        queue.write_buffer(&self.view_uniform_buf, 0, bytemuck::bytes_of(&view_u));

        pass.set_pipeline(&self.checker_pipeline);
        pass.set_bind_group(0, &self.checker_bind_group, &[]);
        pass.draw(0..6, 0..1);

        pass.set_pipeline(&self.layer_pipeline);
        for (i, layer) in overlay.layers.iter().enumerate() {
            if !layer.visible || layer.opacity <= f32::EPSILON {
                continue;
            }
            let Some(Some(gpu)) = self.layers.get(i) else {
                continue;
            };
            let mut u = view_u;
            u.opacity = layer.opacity.clamp(0.0, 1.0);
            queue.write_buffer(&self.view_uniform_buf, 0, bytemuck::bytes_of(&u));
            pass.set_bind_group(0, &gpu.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        if let (Some(draw), Some(bg)) = (overlay.floating, self.floating_bind_group.as_ref()) {
            let fu = FloatUniforms {
                viewport: [viewport.x, viewport.y],
                view_center: [view.center.x, view.center.y],
                image_size: [view.width as f32, view.height as f32],
                zoom: view.zoom,
                view_rotation: view.rotation,
                float_center: [draw.center.x, draw.center.y],
                float_half: [draw.half.x, draw.half.y],
                float_rotation: draw.rotation,
                opacity: draw.opacity.clamp(0.0, 1.0),
            };
            queue.write_buffer(&self.float_uniform_buf, 0, bytemuck::bytes_of(&fu));
            pass.set_pipeline(&self.float_pipeline);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..6, 0..1);
        }

        let sel_u = SelUniforms {
            viewport: [viewport.x, viewport.y],
            center: [view.center.x, view.center.y],
            image_size: [view.width as f32, view.height as f32],
            zoom: view.zoom,
            rotation: view.rotation,
            has_selection: if self.has_selection { 1.0 } else { 0.0 },
            dim_outside: if overlay.dim_outside { 1.0 } else { 0.0 },
            time,
            _pad: 0.0,
        };
        queue.write_buffer(&self.sel_uniform_buf, 0, bytemuck::bytes_of(&sel_u));
        if self.has_selection || overlay.dim_outside {
            pass.set_pipeline(&self.sel_pipeline);
            let bg = self
                .sel_bind_group
                .as_ref()
                .unwrap_or(&self.empty_sel_bind_group);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..6, 0..1);
        }

        let mut verts = Vec::new();
        if let Some(corners) = overlay.selection_rect {
            for i in 0..4 {
                push_ants_line(&mut verts, corners[i], corners[(i + 1) % 4], time);
            }
        }
        if let Some(corners) = overlay.transform_corners {
            let cyan = [0.2, 0.85, 0.95, 1.0];
            let black = [0.02, 0.02, 0.02, 1.0];
            for i in 0..4 {
                push_line(&mut verts, corners[i], corners[(i + 1) % 4], 2.0, black);
                push_line(&mut verts, corners[i], corners[(i + 1) % 4], 1.0, cyan);
                push_handle(&mut verts, corners[i], 5.0, cyan);
            }
            if let Some(rot) = overlay.transform_rotate {
                let top = (corners[0] + corners[1]) * 0.5;
                push_line(&mut verts, top, rot, 1.5, cyan);
                push_handle(&mut verts, rot, 5.0, [1.0, 0.85, 0.2, 1.0]);
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
        queue.write_buffer(
            &self.solid_uniform_buf,
            0,
            bytemuck::bytes_of(&SolidUniforms {
                viewport: [viewport.x, viewport.y],
                _pad: [0.0; 2],
            }),
        );
        queue.write_buffer(&self.solid_vertex_buf, 0, bytes);
        pass.set_pipeline(&self.solid_pipeline);
        pass.set_bind_group(0, &self.solid_bind_group, &[]);
        pass.set_vertex_buffer(0, self.solid_vertex_buf.slice(..));
        pass.draw(0..verts.len() as u32, 0..1);
    }
}

fn buffer_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32, filtering: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(if filtering {
            wgpu::SamplerBindingType::Filtering
        } else {
            wgpu::SamplerBindingType::NonFiltering
        }),
        count: None,
    }
}

fn make_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    vs: &str,
    fs: &str,
    format: wgpu::TextureFormat,
    blend: wgpu::BlendState,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fs),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vs),
            compilation_options: Default::default(),
            buffers,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(blend),
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
    })
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

fn push_ants_line(out: &mut Vec<SolidVertex>, a: Vec2, b: Vec2, time: f32) {
    let delta = b - a;
    let len = delta.length();
    if len < 0.5 {
        return;
    }
    let dir = delta / len;
    let dash = 4.0;
    let period = dash * 2.0;
    let offset = (time * 48.0).rem_euclid(period);
    let mut t = -offset;
    let mut black = true;
    while t < len {
        let t0 = t.max(0.0);
        let t1 = (t + dash).min(len);
        if t1 > t0 + 0.25 {
            let color = if black {
                [0.0, 0.0, 0.0, 1.0]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
            push_line(out, a + dir * t0, a + dir * t1, 1.0, color);
        }
        black = !black;
        t += dash;
    }
}

fn push_handle(out: &mut Vec<SolidVertex>, center: Vec2, radius: f32, color: [f32; 4]) {
    let r = radius;
    let p0 = center + Vec2::new(-r, -r);
    let p1 = center + Vec2::new(r, -r);
    let p2 = center + Vec2::new(r, r);
    let p3 = center + Vec2::new(-r, r);
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
