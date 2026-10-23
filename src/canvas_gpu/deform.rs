use super::*;
use crate::editing::deform::DeformMesh;

impl DeformGpu {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("deformation"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../deform_gpu.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("deform bindings"),
            entries: &[
                buffer_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                texture_entry(1, false),
                texture_entry(2, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("deform layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];
        let pipeline = make_pipeline(
            device,
            &pipeline_layout,
            &shader,
            "vs_mesh",
            "fs_mesh",
            format,
            wgpu::BlendState::ALPHA_BLENDING,
            &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<DeformVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
        );
        let base_pipeline = make_pipeline(
            device,
            &pipeline_layout,
            &shader,
            "vs_base",
            "fs_base",
            format,
            wgpu::BlendState::ALPHA_BLENDING,
            &[],
        );
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("deform vertices"),
            size: 96 * 96 * 6 * 16,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("deform view"),
            size: std::mem::size_of::<ViewUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            base_pipeline,
            layout,
            vertices,
            uniform,
            bind: None,
            count: 0,
            layer: None,
        }
    }
}

impl CanvasRenderer {
    /// Layer pixels stay resident and unchanged. Pointer updates upload geometry only.
    pub fn sync_deformation(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: Option<(&DeformMesh, usize)>,
    ) {
        let Some((mesh, index)) = mesh else {
            self.deform.layer = None;
            self.deform.bind = None;
            self.deform.count = 0;
            return;
        };
        let Some(Some(layer)) = self.layers.get(index) else {
            self.deform.layer = None;
            return;
        };
        let vertices: Vec<DeformVertex> = mesh
            .triangles()
            .flat_map(|tri| {
                tri.map(|i| DeformVertex {
                    position: mesh.positions[i].to_array(),
                    source: mesh.original[i].to_array(),
                })
            })
            .collect();
        queue.write_buffer(&self.deform.vertices, 0, bytemuck::cast_slice(&vertices));
        self.deform.count = vertices.len() as u32;
        self.deform.layer = Some(index);
        let mask = self
            .sel_texture
            .as_ref()
            .unwrap_or(&self.empty_sel_texture)
            .create_view(&Default::default());
        self.deform.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("deform source"),
            layout: &self.deform.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.deform.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&layer.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&mask),
                },
            ],
        }));
    }
}
