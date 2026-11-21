use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3, Vec4};
use mega_ui::{DrawCommand, Rect, wgpu::UiRenderer};

use super::{Primitive, SceneDocument, THUMB_TEXTURE, ThumbnailLayout, fit_frame};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenderStyle {
    Viewport,
    Layer,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 4],
    color: [f32; 4],
}

#[derive(Clone, Copy)]
struct MeshVertex {
    position: Vec3,
    normal: Vec3,
}

pub(crate) struct SceneRenderer {
    surfaces: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
    overlay: wgpu::RenderPipeline,
    depth: Option<((u32, u32), wgpu::TextureView)>,
    cube: Vec<MeshVertex>,
    sphere: Vec<MeshVertex>,
    plane: Vec<MeshVertex>,
    cylinder: Vec<MeshVertex>,
    cone: Vec<MeshVertex>,
    vertices: Vec<Vertex>,
    buffer: wgpu::Buffer,
}

impl SceneRenderer {
    pub(crate) const LAYER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3D scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        let pipeline = |topology, write_depth, overlay| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("3D scene"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(write_depth),
                    depth_compare: Some(if overlay {
                        wgpu::CompareFunction::Always
                    } else {
                        wgpu::CompareFunction::LessEqual
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: overlay.then_some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            surfaces: pipeline(wgpu::PrimitiveTopology::TriangleList, true, false),
            grid: pipeline(wgpu::PrimitiveTopology::LineList, false, false),
            overlay: pipeline(wgpu::PrimitiveTopology::TriangleList, false, true),
            depth: None,
            cube: cube(),
            sphere: sphere(),
            plane: plane(),
            cylinder: round_solid(false),
            cone: round_solid(true),
            vertices: Vec::new(),
            buffer: vertex_buffer(device, 4096),
        }
    }

    pub(crate) fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        views: &[(Rect, &SceneDocument, Option<f32>)],
        style: RenderStyle,
    ) {
        if !self
            .depth
            .as_ref()
            .is_some_and(|(previous, _)| *previous == size)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("3D scene depth"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.depth = Some((size, texture.create_view(&Default::default())));
        }

        self.vertices.clear();
        let mut ranges = Vec::new();
        for &(rect, scene, gizmo_scale) in views {
            if rect.width() >= 1.0
                && rect.height() >= 1.0
                && rect.min.min_element() >= 0.0
                && rect.max.x <= size.0 as f32
                && rect.max.y <= size.1 as f32
            {
                let start = self.vertices.len() as u32;
                let surface_end = self.append_view(rect, scene, style);
                let grid_end = self.vertices.len() as u32;
                if let Some(scale) = gizmo_scale {
                    self.append_gizmo(rect, scene, scale);
                }
                ranges.push((
                    rect,
                    start,
                    surface_end,
                    grid_end,
                    self.vertices.len() as u32,
                ));
            }
        }
        if ranges.is_empty() {
            return;
        }
        let bytes = bytemuck::cast_slice(&self.vertices);
        if bytes.len() as u64 > self.buffer.size() {
            self.buffer = vertex_buffer(device, bytes.len().next_power_of_two() as u64);
        }
        queue.write_buffer(&self.buffer, 0, bytes);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3D scene viewport"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(match style {
                        RenderStyle::Viewport => wgpu::Color {
                            r: 0.027,
                            g: 0.033,
                            b: 0.044,
                            a: 1.0,
                        },
                        RenderStyle::Layer => wgpu::Color::TRANSPARENT,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth.as_ref().unwrap().1,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        for (rect, start, surface_end, grid_end, end) in ranges {
            pass.set_viewport(
                rect.min.x,
                rect.min.y,
                rect.width(),
                rect.height(),
                0.0,
                1.0,
            );
            pass.set_pipeline(&self.surfaces);
            pass.draw(start..surface_end, 0..1);
            pass.set_pipeline(&self.grid);
            pass.draw(surface_end..grid_end, 0..1);
            if grid_end < end {
                pass.set_pipeline(&self.overlay);
                pass.draw(grid_end..end, 0..1);
            }
        }
    }

    fn append_view(&mut self, rect: Rect, scene: &SceneDocument, style: RenderStyle) -> u32 {
        let view = scene.camera.view_projection(rect.max - rect.min);
        let light = Vec3::new(0.45, 0.85, 0.4).normalize();
        for (index, object) in scene.objects.items.iter().enumerate() {
            if !object.visible {
                continue;
            }
            let mesh = match object.primitive {
                Primitive::Cube => &self.cube,
                Primitive::Sphere => &self.sphere,
                Primitive::Plane => &self.plane,
                Primitive::Cylinder => &self.cylinder,
                Primitive::Cone => &self.cone,
            };
            let transform = view * object.matrix();
            let rotation = object.orientation();
            let color = if style == RenderStyle::Viewport && scene.objects.selected == Some(index) {
                Vec3::new(0.22, 0.55, 0.92)
            } else {
                Vec3::new(0.49, 0.53, 0.61)
            };
            self.vertices.extend(mesh.iter().map(|vertex| {
                let normal = (rotation * (vertex.normal / object.scale)).normalize();
                let shade = 0.38 + 0.62 * normal.dot(light).max(0.0);
                Vertex {
                    position: (transform * vertex.position.extend(1.0)).to_array(),
                    color: (color * shade).extend(1.0).to_array(),
                }
            }));
        }
        let surface_end = self.vertices.len() as u32;
        if style == RenderStyle::Layer {
            return surface_end;
        }
        for index in -32..=32 {
            let offset = index as f32;
            let major = [0.12, 0.14, 0.17];
            let minor = [0.055, 0.065, 0.085];
            let color = if index % 5 == 0 { major } else { minor };
            for (a, b, color) in [
                (
                    Vec3::new(-32.0, -0.002, offset),
                    Vec3::new(32.0, -0.002, offset),
                    if index == 0 {
                        [0.31, 0.075, 0.075]
                    } else {
                        color
                    },
                ),
                (
                    Vec3::new(offset, -0.002, -32.0),
                    Vec3::new(offset, -0.002, 32.0),
                    if index == 0 {
                        [0.065, 0.14, 0.34]
                    } else {
                        color
                    },
                ),
            ] {
                self.vertices.extend([a, b].map(|point| Vertex {
                    position: (view * point.extend(1.0)).to_array(),
                    color: [color[0], color[1], color[2], 1.0],
                }));
            }
        }

        surface_end
    }

    pub(crate) fn render_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &SceneDocument,
        size: (u32, u32),
    ) -> Result<crate::raster::SparsePixels, String> {
        let (width, height) = crate::comfyui::scaled_size(size.0, size.1, 1.0)
            .map_err(|_| "Canvas is too large to render a scene layer")?;
        let limit = device.limits().max_texture_dimension_2d;
        let row_bytes = width * 4;
        let stride = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer_size = u64::from(stride) * u64::from(height);
        if width > limit || height > limit || buffer_size > device.limits().max_buffer_size {
            return Err("Canvas is too large to render a scene layer".into());
        }
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("3D layer render"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::LAYER_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("3D layer readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        let canvas = Rect::from_min_size(Vec2::ZERO, Vec2::new(width as f32, height as f32));
        let frame = fit_frame(canvas, scene.view_aspect);
        self.render(
            device,
            queue,
            &mut encoder,
            &target.create_view(&Default::default()),
            size,
            &[(frame, scene, None)],
            RenderStyle::Layer,
        );
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            extent,
        );
        let submission = queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(5)),
            })
            .map_err(|error| format!("Could not render scene layer: {error}"))?;
        receive
            .try_recv()
            .map_err(|error| format!("Could not read scene layer: {error}"))?
            .map_err(|error| format!("Could not read scene layer: {error}"))?;
        let mapped = slice
            .get_mapped_range()
            .map_err(|error| format!("Could not read scene layer: {error}"))?;
        let mut pixels = Vec::with_capacity(row_bytes as usize * height as usize);
        for row in mapped.chunks_exact(stride as usize) {
            pixels.extend_from_slice(&row[..row_bytes as usize]);
        }
        Ok(crate::raster::SparsePixels::from_rgba(
            &pixels, width, height,
        ))
    }

    fn append_gizmo(&mut self, rect: Rect, scene: &SceneDocument, scale: f32) {
        let extent = rect.max - rect.min;
        let view = scene.camera.view_projection(extent);
        let draw = scene.gizmo_geometry(rect, scale);
        for triangle in draw.tris {
            self.vertices
                .extend([triangle.a, triangle.b, triangle.c].map(|position| Vertex {
                    position: (view * position.extend(1.0)).to_array(),
                    color: triangle.color,
                }));
        }
        for line in draw.lines {
            let a = view * line.start.extend(1.0);
            let b = view * line.end.extend(1.0);
            if a.w <= 0.0 || b.w <= 0.0 {
                continue;
            }
            let delta = (b.truncate().truncate() / b.w - a.truncate().truncate() / a.w) * extent;
            let normal = Vec2::new(-delta.y, delta.x).normalize_or_zero() * scale / extent;
            let oa = normal * (line.opts.width_from * a.w);
            let ob = normal * (line.opts.width_to * b.w);
            let oa = Vec4::new(oa.x, oa.y, 0.0, 0.0);
            let ob = Vec4::new(ob.x, ob.y, 0.0, 0.0);
            let corners = [
                (a + oa, line.opts.color_from),
                (a - oa, line.opts.color_from),
                (b - ob, line.opts.color_to),
                (b + ob, line.opts.color_to),
            ];
            self.vertices.extend([0, 1, 2, 0, 2, 3].map(|i| Vertex {
                position: corners[i].0.to_array(),
                color: corners[i].1,
            }));
        }
    }
}

pub(crate) struct SceneThumbnails {
    renderer: SceneRenderer,
    target: Option<((u32, u32), wgpu::TextureView)>,
    format: wgpu::TextureFormat,
}

impl SceneThumbnails {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            renderer: SceneRenderer::new(device, format),
            target: None,
            format,
        }
    }

    pub(crate) fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        ui_renderer: &mut UiRenderer,
        documents: &[SceneDocument],
        commands: &[DrawCommand],
    ) {
        let layout = ThumbnailLayout::new(documents.len());
        let views: Vec<_> = commands
            .iter()
            .filter(|command| command.tex == THUMB_TEXTURE)
            .filter_map(|command| {
                let uv =
                    (Vec2::from_array(command.uv_min) + Vec2::from_array(command.uv_max)) * 0.5;
                let index = layout.index_at(uv);
                let scene = documents.get(index)?;
                Some((layout.view_rect(index, scene.view_aspect), scene, None))
            })
            .collect();
        if views.is_empty() {
            return;
        }
        if !self
            .target
            .as_ref()
            .is_some_and(|(size, _)| *size == layout.size)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("3D scene thumbnails"),
                size: wgpu::Extent3d {
                    width: layout.size.0,
                    height: layout.size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            ui_renderer.bind_texture_view(device, THUMB_TEXTURE, view.clone());
            self.target = Some((layout.size, view));
        }
        let (_, view) = self.target.as_ref().unwrap();
        self.renderer.render(
            device,
            queue,
            encoder,
            view,
            layout.size,
            &views,
            RenderStyle::Viewport,
        );
    }
}

fn vertex_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("3D scene vertices"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn cube() -> Vec<MeshVertex> {
    let mut mesh = Vec::with_capacity(36);
    for normal in [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ] {
        let tangent = if normal.y == 0.0 { Vec3::Y } else { Vec3::X };
        let bitangent = normal.cross(tangent);
        let corners = [
            normal - tangent - bitangent,
            normal + tangent - bitangent,
            normal + tangent + bitangent,
            normal - tangent + bitangent,
        ];
        mesh.extend([0, 1, 2, 0, 2, 3].map(|index| MeshVertex {
            position: corners[index] * 0.5,
            normal,
        }));
    }
    mesh
}

fn sphere() -> Vec<MeshVertex> {
    const ROWS: usize = 16;
    const COLUMNS: usize = 24;
    let point = |row, column| {
        let latitude = std::f32::consts::PI * row as f32 / ROWS as f32;
        let azimuth = std::f32::consts::TAU * column as f32 / COLUMNS as f32;
        let normal = Vec3::new(
            latitude.sin() * azimuth.cos(),
            latitude.cos(),
            latitude.sin() * azimuth.sin(),
        );
        MeshVertex {
            position: normal * 0.5,
            normal,
        }
    };
    let mut mesh = Vec::with_capacity(ROWS * COLUMNS * 6);
    for row in 0..ROWS {
        for column in 0..COLUMNS {
            let a = point(row, column);
            let b = point(row + 1, column);
            let c = point(row + 1, column + 1);
            let d = point(row, column + 1);
            if row + 1 < ROWS {
                mesh.extend([a, b, c]);
            }
            if row > 0 {
                mesh.extend([a, c, d]);
            }
        }
    }
    mesh
}

fn plane() -> Vec<MeshVertex> {
    let mesh = mega_render::plane(1.0, 1.0);
    mesh.indices
        .iter()
        .map(|&index| MeshVertex {
            position: Vec3::from_array(mesh.positions[index as usize]),
            normal: Vec3::Y,
        })
        .collect()
}

fn round_solid(cone: bool) -> Vec<MeshVertex> {
    const SEGMENTS: usize = 48;
    let ring = |index| {
        let angle = std::f32::consts::TAU * index as f32 / SEGMENTS as f32;
        Vec3::new(angle.cos(), 0.0, angle.sin())
    };
    let vertex = |position, normal| MeshVertex { position, normal };
    let mut mesh = Vec::with_capacity(SEGMENTS * if cone { 6 } else { 12 });
    let bottom = Vec3::NEG_Y * 0.5;
    let top = Vec3::Y * 0.5;
    for index in 0..SEGMENTS {
        let a = ring(index);
        let b = ring(index + 1);
        let lower_a = bottom + a * 0.5;
        let lower_b = bottom + b * 0.5;
        mesh.extend([bottom, lower_a, lower_b].map(|position| vertex(position, Vec3::NEG_Y)));
        if cone {
            let normal = |radial: Vec3| (radial + top).normalize();
            mesh.extend([
                vertex(lower_a, normal(a)),
                vertex(top, normal((a + b).normalize())),
                vertex(lower_b, normal(b)),
            ]);
        } else {
            let upper_a = top + a * 0.5;
            let upper_b = top + b * 0.5;
            mesh.extend([top, upper_b, upper_a].map(|position| vertex(position, Vec3::Y)));
            mesh.extend([
                vertex(lower_a, a),
                vertex(upper_a, a),
                vertex(upper_b, b),
                vertex(lower_a, a),
                vertex(upper_b, b),
                vertex(lower_b, b),
            ]);
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn scene_layer_render_is_transparent_and_uses_image_canvas_size() {
        use crate::raster::Raster;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let mut renderer = SceneRenderer::new(&device, SceneRenderer::LAYER_FORMAT);
        let mut scene = SceneDocument::new(0, "Layer".into());
        let size = (101, 63);
        let empty = renderer
            .render_layer(&device, &queue, &scene, size)
            .unwrap();
        assert!(empty.tiles.is_empty());
        scene.add(Primitive::Cube);
        let frame = renderer
            .render_layer(&device, &queue, &scene, size)
            .unwrap();
        assert_eq!(frame.read(0, 0, 0), [0; 4]);
        let center = frame.read(0, 50, 31);
        assert_eq!(center[3], 255);
        assert!(center[0] > 0);
        for primitive in [Primitive::Cylinder, Primitive::Cone, Primitive::Plane] {
            let mut scene = SceneDocument::new(0, "Primitive".into());
            scene.add(primitive);
            let frame = renderer
                .render_layer(&device, &queue, &scene, size)
                .unwrap();
            assert!(!frame.tiles.is_empty(), "{} must render", primitive.name());
            scene.toggle_object_visibility(0);
            let frame = renderer
                .render_layer(&device, &queue, &scene, size)
                .unwrap();
            assert!(
                frame.tiles.is_empty(),
                "Hidden {} must not render",
                primitive.name()
            );
        }
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn offscreen_scene_keeps_the_nearer_object_visible() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut renderer = SceneRenderer::new(&device, format);
        let extent = wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        };
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64 * 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut scene = SceneDocument::new(0, "Depth test".into());
        scene.add(Primitive::Cube);
        scene.add(Primitive::Sphere);
        let center = scene.objects.items[0].position;
        scene.objects.items[1].position = center - (scene.camera.eye() - center).normalize() * 2.0;
        scene.objects.selected = Some(0);
        let rect = Rect::from_min_size(Vec2::splat(8.0), Vec2::splat(48.0));
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &target.create_view(&Default::default()),
            (64, 64),
            &[(rect, &scene, None)],
            RenderStyle::Viewport,
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(64),
                },
            },
            extent,
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let pixels = readback.slice(..).get_mapped_range().unwrap();
        let pixel = &pixels[32 * 256 + 32 * 4..][..4];
        assert!(u16::from(pixel[2]) > u16::from(pixel[0]) * 3, "{pixel:?}");
        assert_eq!(pixel[3], 255);
    }
}
