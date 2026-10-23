use crate::raster::{Raster, SparsePixels};
use bytemuck::{Pod, Zeroable};
use glam::Vec2;
use mega_ui::Rect;
use wgpu::util::DeviceExt;

use crate::tiles::DirtyTiles;
use crate::view::CanvasView;

mod layers;
mod deform;
mod pipelines;
mod render;
mod transform;

// Larger preview cores keep texture/pass counts low; the fixed halo covers
// trilinear filtering down to Transform's minimum scale (1/20).
const TRANSFORM_PREVIEW_SIDE: u32 = 256;
const TRANSFORM_PREVIEW_PAD: u32 = 32;

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
    hue: f32,
    saturation: f32,
    value: f32,
    _hsv_pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FloatUniforms {
    viewport: [f32; 2],
    opacity: f32,
    filter_scale: f32,
    origin: [f32; 2],
    size: [f32; 2],
    image_size: [f32; 2],
    c0: [f32; 2],
    c1: [f32; 2],
    c2: [f32; 2],
    c3: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PunchUniforms {
    viewport: [f32; 2],
    center: [f32; 2],
    image_size: [f32; 2],
    zoom: f32,
    rotation: f32,
    opacity: f32,
    _pad: f32,
    origin: [f32; 2],
    size: [f32; 2],
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
    pub deformation: Option<usize>,
    pub hsv: Option<(usize, crate::hsv::HsvAdjustment)>,
    pub brush_center: Option<Vec2>,
    pub brush_radius: f32,
    pub selection_rect: Option<[Vec2; 4]>,
    pub transform_corners: Option<[Vec2; 4]>,
    pub transform_pivot: Option<Vec2>,
    pub dim_outside: bool,
    pub floating: Option<FloatingDraw>,
    pub layers: Vec<LayerDraw>,
}

#[derive(Clone, Copy)]
pub struct FloatingDraw {
    pub opacity: f32,
    pub whole_layer: bool,
    pub layer: usize,
    pub origin: Vec2,
    pub size: Vec2,
}

struct FloatingTileGpu {
    _texture: wgpu::Texture,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    origin: Vec2,
}

struct LayerGpu {
    uniform: wgpu::Buffer,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DeformVertex { position: [f32; 2], source: [f32; 2] }

struct DeformGpu {
    pipeline: wgpu::RenderPipeline,
    base_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    vertices: wgpu::Buffer,
    uniform: wgpu::Buffer,
    bind: Option<wgpu::BindGroup>,
    count: u32,
    layer: Option<usize>,
}

pub struct CanvasRenderer {
    deform: DeformGpu,
    checker_pipeline: wgpu::RenderPipeline,
    layer_pipeline: wgpu::RenderPipeline,
    float_pipeline: wgpu::RenderPipeline,
    mip_init_pipeline: wgpu::RenderPipeline,
    mip_down_pipeline: wgpu::RenderPipeline,
    mip_bind_layout: wgpu::BindGroupLayout,
    punch_pipeline: wgpu::RenderPipeline,
    sel_pipeline: wgpu::RenderPipeline,
    solid_pipeline: wgpu::RenderPipeline,
    view_bind_layout: wgpu::BindGroupLayout,
    float_bind_layout: wgpu::BindGroupLayout,
    sel_bind_layout: wgpu::BindGroupLayout,
    #[allow(dead_code)]
    solid_bind_layout: wgpu::BindGroupLayout,
    view_uniform_buf: wgpu::Buffer,
    float_uniform_buf: wgpu::Buffer,
    punch_uniform_buf: wgpu::Buffer,
    sel_uniform_buf: wgpu::Buffer,
    solid_uniform_buf: wgpu::Buffer,
    solid_vertex_buf: wgpu::Buffer,
    sampler_near: wgpu::Sampler,
    sampler_linear: wgpu::Sampler,
    layers: Vec<Option<LayerGpu>>,
    /// Bumped when a layer texture is reallocated so transform bindings are rebuilt.
    layer_epoch: u64,
    transform_epoch: u64,
    transform_layer: Option<usize>,
    mask_texture: Option<wgpu::Texture>,
    floating_texture: Option<wgpu::Texture>,
    floating_tiles: Vec<FloatingTileGpu>,
    tile_mask: Option<wgpu::Texture>,
    mask_view: Option<wgpu::TextureView>,
    mask_size: (u32, u32),
    punch_bind_group: Option<wgpu::BindGroup>,
    floating_bind_group: Option<wgpu::BindGroup>,
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

fn buffer_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
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

fn push_circle(
    out: &mut Vec<SolidVertex>,
    center: Vec2,
    radius: f32,
    thickness: f32,
    color: [f32; 4],
) {
    let steps = ((radius * 2.0).ceil() as i32).clamp(24, 96);
    for i in 0..steps {
        let a0 = i as f32 / steps as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / steps as f32 * std::f32::consts::TAU;
        let p0 = center + Vec2::new(a0.cos(), a0.sin()) * radius;
        let p1 = center + Vec2::new(a1.cos(), a1.sin()) * radius;
        push_line(out, p0, p1, thickness, color);
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;

    #[test]
    #[ignore = "requires a GPU adapter; verifies live HSV preview against committed pixels"]
    fn hsv_preview_matches_cpu_filter_and_only_targets_the_requested_layer() {
        use crate::hsv::HsvAdjustment;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            force_fallback_adapter: false,
            power_preference: wgpu::PowerPreference::HighPerformance,
            apply_limit_buckets: false,
        }))
        .unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut renderer = CanvasRenderer::new(&device, format);
        let mut pixels = Vec::new();
        for y in 0..64 {
            for x in 0..64 {
                pixels.extend_from_slice(&[(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8, 255]);
            }
        }
        let source = SparsePixels::from_rgba(&pixels, 64, 64);
        renderer.upload_sparse_layers(&device, &queue, &[&source], 64, 64);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("HSV proof"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64 * 64 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for (index, adjustment) in [
            (0, HsvAdjustment::default()),
            (
                0,
                HsvAdjustment {
                    hue: 15.0,
                    saturation: -10.0,
                    value: 3.0,
                },
            ),
            (
                0,
                HsvAdjustment {
                    hue: -180.0,
                    saturation: 100.0,
                    value: -40.0,
                },
            ),
            (
                0,
                HsvAdjustment {
                    hue: 90.0,
                    saturation: 50.0,
                    value: 100.0,
                },
            ),
            (
                1,
                HsvAdjustment {
                    hue: 120.0,
                    saturation: 100.0,
                    value: 50.0,
                },
            ),
        ] {
            let view = target.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                renderer.render(
                    &queue,
                    &mut pass,
                    Vec2::splat(64.0),
                    Rect::from_min_size(Vec2::ZERO, Vec2::splat(64.0)),
                    CanvasView {
                        center: Vec2::splat(32.0),
                        zoom: 1.0,
                        rotation: 0.0,
                        width: 64,
                        height: 64,
                    },
                    CanvasOverlay {
                        hsv: Some((index, adjustment)),
                        layers: vec![LayerDraw {
                            visible: true,
                            opacity: 1.0,
                        }],
                        ..Default::default()
                    },
                    0.0,
                );
            }
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(64),
                    },
                },
                wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                tx.send(r).unwrap();
            });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            rx.recv().unwrap().unwrap();
            let data = buffer.slice(..).get_mapped_range().unwrap();
            for (pixel, got) in pixels.chunks_exact(4).zip(data.chunks_exact(4)) {
                let original = pixel.try_into().unwrap();
                let expected = if index == 0 {
                    adjustment.pixel(original)
                } else {
                    original
                };
                for c in 0..4 {
                    assert!(
                        (expected[c] as i32 - got[c] as i32).abs() <= 1,
                        "{adjustment:?} layer={index}, original={pixel:?}, expected={expected:?}, got={got:?}"
                    );
                }
            }
            drop(data);
            buffer.unmap();
        }
    }

    #[test]
    #[ignore = "manual color diagnostic: compare CPU export, GPU canvas and final UI pass"]
    fn color_pipeline_probe() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            force_fallback_adapter: false,
            power_preference: wgpu::PowerPreference::HighPerformance,
            apply_limit_buckets: false,
        }))
        .unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut canvas = CanvasRenderer::new(&device, format);
        let make_target = || {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("color probe"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let scene = make_target();
        let screen = make_target();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 2 * 64 * 64 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let area = Rect::from_min_size(Vec2::ZERO, Vec2::splat(64.0));
        let mut ui = mega_ui::Ui::new();
        ui.begin_frame(mega_ui::UiInput {
            viewport: Vec2::splat(64.0),
            ..Default::default()
        });
        ui.texture_rect(crate::host::CANVAS_TEXTURE, area, [0.0, 0.0], [1.0, 1.0]);
        let output = ui.end_frame();
        let mut renderer = mega_ui::wgpu::UiRenderer::new(&device, &queue, format, &ui);
        renderer.set_viewport(&queue, 64.0, 64.0);
        renderer.bind_texture_view(
            &device,
            crate::host::CANVAS_TEXTURE,
            scene.create_view(&Default::default()),
        );
        for (name, upper) in [
            ("opaque", None),
            ("layer opacity 50%", Some(([210, 60, 20, 255], 0.5))),
            ("pixel alpha 128", Some(([210, 60, 20, 128], 1.0))),
        ] {
            let mut project = crate::project::Project::default();
            project
                .import_rgba(&[30, 100, 200, 255].repeat(64 * 64), 64, 64)
                .unwrap();
            if let Some((pixel, opacity)) = upper {
                project.add_layer();
                project.layers[1].pixels = SparsePixels::from_rgba(&pixel.repeat(64 * 64), 64, 64);
                project.layers[1].opacity = opacity;
            }
            let expected = project.decoded_rgba().unwrap().unwrap().0;
            let probe = (32 * 64 + 32) * 4;
            let temp = tempfile::tempdir().unwrap();
            for (extension, format) in [
                ("png", image::ImageFormat::Png),
                ("webp", image::ImageFormat::WebP),
            ] {
                let path = temp.path().join(format!("probe.{extension}"));
                project.export_image(&path, format).unwrap();
                let decoded = image::open(path).unwrap().into_rgba8().into_raw();
                assert_eq!(decoded, expected, "{extension} must preserve exported RGB");
            }
            let sources: Vec<_> = project.layers.iter().map(|l| &l.pixels).collect();
            canvas.upload_sparse_layers(&device, &queue, &sources, 64, 64);
            let mut encoder = device.create_command_encoder(&Default::default());
            for (is_scene, texture) in [(true, &scene), (false, &screen)] {
                let view = texture.create_view(&Default::default());
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if is_scene {
                    canvas.render(
                        &queue,
                        &mut pass,
                        Vec2::splat(64.0),
                        area,
                        CanvasView {
                            center: Vec2::splat(32.0),
                            zoom: 1.0,
                            rotation: 0.0,
                            width: 64,
                            height: 64,
                        },
                        CanvasOverlay {
                            layers: project
                                .layers
                                .iter()
                                .map(|l| LayerDraw {
                                    visible: l.visible,
                                    opacity: l.opacity,
                                })
                                .collect(),
                            ..Default::default()
                        },
                        0.0,
                    );
                } else {
                    renderer.draw(&queue, &mut pass, &output.draw_list);
                }
            }
            for (index, texture) in [&scene, &screen].into_iter().enumerate() {
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: (index * 64 * 64 * 4) as u64,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(64),
                        },
                    },
                    wgpu::Extent3d {
                        width: 64,
                        height: 64,
                        depth_or_array_layers: 1,
                    },
                );
            }
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                tx.send(r).unwrap();
            });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            rx.recv().unwrap().unwrap();
            let data = buffer.slice(..).get_mapped_range().unwrap();
            let cpu = &expected[probe..probe + 4];
            let gpu = &data[probe..probe + 4];
            let displayed = &data[64 * 64 * 4 + probe..64 * 64 * 4 + probe + 4];
            eprintln!(
                "{name}: CPU / PNG / WebP = {cpu:?}, GPU canvas = {gpu:?}, final UI = {displayed:?}"
            );
            assert_eq!(
                gpu, displayed,
                "UI composition must not alter canvas colors"
            );
            if upper.is_none() {
                assert_eq!(
                    cpu, gpu,
                    "opaque source must survive the complete display pipeline"
                );
            }
            drop(data);
            buffer.unmap();
        }
    }

    #[test]
    #[ignore = "requires a GPU adapter; run explicitly for renderer changes"]
    fn transform_preview_downscale_filters_stripes_across_tile_boundaries() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            force_fallback_adapter: false,
            power_preference: wgpu::PowerPreference::HighPerformance,
            apply_limit_buckets: false,
        }))
        .unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut renderer = CanvasRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        let mut pixels = vec![0; 512 * 512 * 4];
        for y in 0..512 {
            for x in 0..512 {
                let v = if x % 4 == 0 { 255 } else { 0 };
                pixels[(y * 512 + x) * 4..(y * 512 + x) * 4 + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let source = SparsePixels::from_rgba(&pixels, 512, 512);
        renderer.upload_sparse_layers(&device, &queue, &[&source], 512, 512);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64 * 64 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for whole_layer in [true, false] {
            if whole_layer {
                renderer.sync_sparse_transform(&device, &queue, &source, true);
            } else {
                renderer.clear_transform();
                assert!(renderer.sync_transform(
                    &device,
                    &queue,
                    0,
                    &vec![1; 512 * 512],
                    &pixels,
                    512,
                    512,
                    true
                ));
            }
            let mut encoder = device.create_command_encoder(&Default::default());
            let view = target.create_view(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                renderer.render(
                    &queue,
                    &mut pass,
                    Vec2::splat(64.0),
                    Rect {
                        min: Vec2::ZERO,
                        max: Vec2::splat(64.0),
                    },
                    CanvasView {
                        center: Vec2::splat(256.0),
                        zoom: 1.0,
                        rotation: 0.0,
                        width: 512,
                        height: 512,
                    },
                    CanvasOverlay {
                        layers: vec![LayerDraw {
                            visible: true,
                            opacity: 1.0,
                        }],
                        floating: Some(FloatingDraw {
                            opacity: 1.0,
                            whole_layer,
                            layer: 0,
                            origin: Vec2::ZERO,
                            size: Vec2::splat(512.0),
                        }),
                        transform_corners: Some([
                            Vec2::ZERO,
                            Vec2::new(64.0, 0.0),
                            Vec2::splat(64.0),
                            Vec2::new(0.0, 64.0),
                        ]),
                        ..Default::default()
                    },
                    0.0,
                );
            }
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &output,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(64),
                    },
                },
                wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            output.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                tx.send(r).unwrap();
            });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            rx.recv().unwrap().unwrap();
            let data = output.slice(..).get_mapped_range().unwrap();
            for x in 24..40 {
                let p = &data[(32 * 64 + x) * 4..(32 * 64 + x) * 4 + 4];
                // Target is linear UNORM: quarter white is 64 (sRGB would be 137).
                assert!(
                    (p[0] as i32 - 64).abs() <= 1,
                    "whole_layer={whole_layer}, x={x}: {p:?}"
                );
                assert_eq!(p[3], 255);
            }
            drop(data);
            output.unmap();
        }
    }

    #[test]
    #[ignore = "requires a GPU adapter; measures realistic interactive Transform frames"]
    fn transform_preview_large_image_reuses_mips_while_scaling() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            force_fallback_adapter: false,
            power_preference: wgpu::PowerPreference::HighPerformance,
            apply_limit_buckets: false,
        }))
        .unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut renderer = CanvasRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        let pixels = [190, 130, 70, 255].repeat(2048 * 2048);
        let mask = vec![1; 2048 * 2048];
        let source = SparsePixels::from_rgba(&pixels, 2048, 2048);
        renderer.upload_sparse_layers(&device, &queue, &[&source], 2048, 2048);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 512,
                height: 512,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        for whole_layer in [true, false] {
            renderer.clear_transform();
            let prepared = std::time::Instant::now();
            if whole_layer {
                renderer.sync_sparse_transform(&device, &queue, &source, true);
                assert!(
                    renderer.floating_tiles.len() <= 100,
                    "preview must group physical source tiles into larger cores"
                );
            } else {
                assert!(
                    renderer.sync_transform(&device, &queue, 0, &mask, &pixels, 2048, 2048, true)
                );
            }
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            eprintln!(
                "2048x2048 preview preparation, whole_layer={whole_layer}: {:?}",
                prepared.elapsed()
            );
            let cached: Vec<_> = renderer
                .floating_tiles
                .iter()
                .map(|t| t._texture.clone())
                .collect();
            let dense_cached = renderer.floating_texture.clone();
            for scale in [1.0, 0.5, 0.25, 0.1, 0.05, 0.2, 0.6] {
                let started = std::time::Instant::now();
                if whole_layer {
                    renderer.sync_sparse_transform(&device, &queue, &source, false);
                } else {
                    renderer.sync_transform(&device, &queue, 0, &mask, &pixels, 2048, 2048, false);
                }
                assert!(
                    cached
                        .iter()
                        .zip(&renderer.floating_tiles)
                        .all(|(before, after)| *before == after._texture)
                );
                assert_eq!(cached.len(), renderer.floating_tiles.len());
                assert_eq!(dense_cached, renderer.floating_texture);
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            depth_slice: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    let size = 2048.0 * scale;
                    renderer.render(
                        &queue,
                        &mut pass,
                        Vec2::splat(512.0),
                        Rect {
                            min: Vec2::ZERO,
                            max: Vec2::splat(512.0),
                        },
                        CanvasView {
                            center: Vec2::splat(1024.0),
                            zoom: 1.0,
                            rotation: 0.0,
                            width: 2048,
                            height: 2048,
                        },
                        CanvasOverlay {
                            layers: vec![LayerDraw {
                                visible: true,
                                opacity: 1.0,
                            }],
                            floating: Some(FloatingDraw {
                                opacity: 1.0,
                                whole_layer,
                                layer: 0,
                                origin: Vec2::ZERO,
                                size: Vec2::splat(2048.0),
                            }),
                            transform_corners: Some([
                                Vec2::ZERO,
                                Vec2::new(size, 0.0),
                                Vec2::splat(size),
                                Vec2::new(0.0, size),
                            ]),
                            ..Default::default()
                        },
                        0.0,
                    );
                }
                queue.submit([encoder.finish()]);
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                eprintln!(
                    "2048x2048 preview at {scale}, whole_layer={whole_layer}: {:?}",
                    started.elapsed()
                );
            }
        }
    }

    #[test]
    #[ignore = "requires a GPU adapter; run explicitly for renderer changes"]
    fn tiled_preview_uploads_and_composes_in_layer_order() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            force_fallback_adapter: false,
            power_preference: wgpu::PowerPreference::HighPerformance,
            apply_limit_buckets: false,
        }))
        .unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut renderer = CanvasRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        let red = SparsePixels::from_rgba(&[255, 0, 0, 255].repeat(64 * 64), 64, 64);
        let blue = SparsePixels::from_rgba(&[0, 0, 255, 255].repeat(64 * 64), 64, 64);
        renderer.upload_sparse_layers(&device, &queue, &[&red, &blue], 64, 64);
        let mut dirty = DirtyTiles::default();
        dirty.resize(64, 64);
        renderer.upload_layer_dirty(&device, &queue, 0, &red, 64, 64, &mut dirty);
        assert_eq!(
            renderer.layers.len(),
            2,
            "dirty upload must retain higher layers"
        );
        renderer.sync_sparse_transform(&device, &queue, &red, true);
        assert_eq!(renderer.floating_tiles.len(), 4);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64 * 64 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let view = target.create_view(&Default::default());
        // Upper blue layer must cover a floating red layer, including its preview.
        for upper_opacity in [1.0, 0.0] {
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                renderer.render(
                    &queue,
                    &mut pass,
                    Vec2::splat(64.0),
                    Rect {
                        min: Vec2::ZERO,
                        max: Vec2::splat(64.0),
                    },
                    CanvasView {
                        center: Vec2::splat(32.0),
                        zoom: 1.0,
                        rotation: 0.0,
                        width: 64,
                        height: 64,
                    },
                    CanvasOverlay {
                        layers: vec![
                            LayerDraw {
                                visible: true,
                                opacity: 1.0,
                            },
                            LayerDraw {
                                visible: true,
                                opacity: upper_opacity,
                            },
                        ],
                        floating: Some(FloatingDraw {
                            opacity: 1.0,
                            whole_layer: true,
                            layer: 0,
                            origin: Vec2::ZERO,
                            size: Vec2::splat(64.0),
                        }),
                        transform_corners: Some([
                            Vec2::ZERO,
                            Vec2::new(64.0, 0.0),
                            Vec2::splat(64.0),
                            Vec2::new(0.0, 64.0),
                        ]),
                        transform_pivot: Some(Vec2::splat(8.0)),
                        ..Default::default()
                    },
                    0.0,
                );
            }
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &output,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(64),
                    },
                },
                wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            output.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                tx.send(r).unwrap();
            });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            rx.recv().unwrap().unwrap();
            let data = output.slice(..).get_mapped_range().unwrap();
            let offset = (32 * 64 + 32) * 4;
            assert_eq!(
                &data[offset..offset + 4],
                if upper_opacity == 1.0 {
                    &[0, 0, 255, 255]
                } else {
                    &[255, 0, 0, 255]
                }
            );
            drop(data);
            output.unmap();
        }
    }
    #[test]
    #[ignore = "requires a GPU adapter; run explicitly for renderer changes"]
    fn deformation_gpu_preview_respects_selection_and_layer_order() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            force_fallback_adapter: false,
            power_preference: wgpu::PowerPreference::HighPerformance,
            apply_limit_buckets: false,
        }))
        .unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut renderer = CanvasRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        let bytes: Vec<u8> = (0..64).flat_map(|_| (0..64).flat_map(|x| if x < 32 { [255,0,0,255] } else { [0,0,255,255] })).collect();
        let red = SparsePixels::from_rgba(&bytes,64,64);
        let mut mesh = crate::editing::deform::DeformMesh::new(64,64,16);
        mesh.dab(Vec2::splat(32.0),Vec2::new(8.0,0.0),24.0,1.0,1.0,0);
        let blue = SparsePixels::from_rgba(&[0, 0, 255, 255].repeat(64 * 64), 64, 64);
        renderer.upload_sparse_layers(&device, &queue, &[&red, &blue], 64, 64);
        let mut dirty = DirtyTiles::default();
        dirty.resize(64, 64);
        renderer.upload_layer_dirty(&device, &queue, 0, &red, 64, 64, &mut dirty);
        assert_eq!(
            renderer.layers.len(),
            2,
            "dirty upload must retain higher layers"
        );

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64 * 64 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let view = target.create_view(&Default::default());
        // Upper blue layer must cover the deformed lower layer.
        for (upper_opacity, selected) in [(1.0,true),(0.0,true),(0.0,false)] {
            let mask = vec![if selected {255} else {0};64*64];
            renderer.set_selection_mask(&device,&queue,Some(&mask),64,64);
            renderer.sync_deformation(&device,&queue,Some((&mesh,0)));
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                renderer.render(
                    &queue,
                    &mut pass,
                    Vec2::splat(64.0),
                    Rect {
                        min: Vec2::ZERO,
                        max: Vec2::splat(64.0),
                    },
                    CanvasView {
                        center: Vec2::splat(32.0),
                        zoom: 1.0,
                        rotation: 0.0,
                        width: 64,
                        height: 64,
                    },
                    CanvasOverlay {
                        deformation: Some(0),
                        layers: vec![
                            LayerDraw {
                                visible: true,
                                opacity: 1.0,
                            },
                            LayerDraw {
                                visible: true,
                                opacity: upper_opacity,
                            },
                        ],
                        ..Default::default()
                    },
                    0.0,
                );
            }
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &output,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(64),
                    },
                },
                wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            output.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                tx.send(r).unwrap();
            });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            rx.recv().unwrap().unwrap();
            let data = output.slice(..).get_mapped_range().unwrap();
            let offset = (32 * 64 + 32) * 4;
            assert_eq!(
                &data[offset..offset + 4],
                if upper_opacity == 1.0 || !selected {
                    &[0, 0, 255, 255]
                } else {
                    &[255, 0, 0, 255]
                }
            );
            drop(data);
            output.unmap();
        }
    }
}
