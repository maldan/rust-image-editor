use super::*;
use crate::{camera::CameraKey, project::Project};

#[test]
#[ignore = "requires a GPU adapter"]
fn camera_gpu_matches_export_for_pan_zoom_depth_fade_and_hidden_preview() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = CanvasRenderer::new(&device, format);
    let mut project = Project::default();
    project
        .import_rgba(&[240, 60, 20, 255].repeat(64 * 64), 64, 64)
        .unwrap();
    project.add_layer();
    project.layers[1].z = 1.0;
    project.layers[1].pixels = SparsePixels::from_rgba(&[20, 80, 220, 128].repeat(64 * 64), 64, 64);
    renderer.upload_sparse_layers(
        &device,
        &queue,
        &project.layers.iter().map(|l| &l.pixels).collect::<Vec<_>>(),
        64,
        64,
    );
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("camera proof"),
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
        size: 64 * 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    for camera in [
        Some(CameraKey::default()),
        Some(CameraKey {
            pan: [16.0, -8.0],
            ..Default::default()
        }),
        Some(CameraKey {
            zoom: 0.5,
            ..Default::default()
        }),
        Some(CameraKey {
            zoom: 2.0,
            pan: [-8.0, 16.0],
            fade: 0.5,
            ..Default::default()
        }),
        Some(CameraKey {
            fade: 1.0,
            ..Default::default()
        }),
        None,
    ] {
        let mut encoder = device.create_command_encoder(&Default::default());
        let target_view = target.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
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
                    camera: camera.map(|k| (k, project.layers.iter().map(|l| l.z).collect())),
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
        let expected = crate::camera::render_frame(&project, camera.unwrap_or_default());
        for (i, (a, b)) in data.iter().zip(&expected).enumerate() {
            assert!(
                a.abs_diff(*b) <= 2,
                "camera {camera:?}, byte {i}: GPU {a}, export {b}"
            );
        }
        drop(data);
        buffer.unmap();
    }
}
