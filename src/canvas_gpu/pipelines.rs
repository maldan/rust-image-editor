use super::*;

impl CanvasRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("canvas"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../canvas_gpu.wgsl").into()),
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
                texture_entry(1, true),
                sampler_entry(2, true),
                texture_entry(3, true),
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
        let mip_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transform mip bind"),
            entries: &[
                texture_entry(0, true),
                texture_entry(1, true),
                sampler_entry(2, true),
            ],
        });
        let mip_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("transform mip layout"),
            bind_group_layouts: &[Some(&mip_bind_layout)],
            immediate_size: 0,
        });
        let mip_init_pipeline = make_pipeline(
            device,
            &mip_layout,
            &shader,
            "vs_mip",
            "fs_mip_init",
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::BlendState::REPLACE,
            &[],
        );
        let mip_down_pipeline = make_pipeline(
            device,
            &mip_layout,
            &shader,
            "vs_mip",
            "fs_mip_down",
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::BlendState::REPLACE,
            &[],
        );
        let punch_pipeline = make_pipeline(
            device,
            &float_layout,
            &shader,
            "vs_punch",
            "fs_punch",
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
        let punch_uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("canvas punch uniforms"),
            size: std::mem::size_of::<PunchUniforms>() as u64,
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
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
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
            deform: DeformGpu::new(device, format),
            checker_pipeline,
            layer_pipeline,
            float_pipeline,
            mip_init_pipeline,
            mip_down_pipeline,
            mip_bind_layout,
            punch_pipeline,
            sel_pipeline,
            solid_pipeline,
            view_bind_layout,
            float_bind_layout,
            sel_bind_layout,
            solid_bind_layout,
            view_uniform_buf,
            float_uniform_buf,
            punch_uniform_buf,
            sel_uniform_buf,
            solid_uniform_buf,
            solid_vertex_buf,
            sampler_near,
            sampler_linear,
            layers: Vec::new(),
            layer_epoch: 0,
            transform_epoch: 0,
            transform_layer: None,
            mask_texture: None,
            floating_texture: None,
            floating_tiles: Vec::new(),
            tile_mask: None,
            mask_view: None,
            mask_size: (0, 0),
            punch_bind_group: None,
            floating_bind_group: None,
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
}
