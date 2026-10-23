use super::*;

impl CanvasRenderer {
    pub fn clear(&mut self) {
        self.layers.clear();
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.clear_transform();
        self.size = (0, 0);
        self.has_selection = false;
    }

    pub fn clear_transform(&mut self) {
        self.transform_epoch = 0;
        self.transform_layer = None;
        self.mask_texture = None;
        self.mask_view = None;
        self.mask_size = (0, 0);
        self.punch_bind_group = None;
        self.floating_bind_group = None;
        self.floating_texture = None;
        self.floating_tiles.clear();
        self.tile_mask = None;
    }

    pub fn has_image(&self) -> bool {
        self.layers.iter().any(|l| l.is_some())
    }

    pub fn ensure_layers(&mut self, device: &wgpu::Device, count: usize, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        let size_changed = self.size != (width, height);
        if size_changed {
            self.layers.clear();
            self.size = (width, height);
            self.layer_epoch = self.layer_epoch.wrapping_add(1);
            self.clear_transform();
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
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("layer uniforms"),
                size: std::mem::size_of::<ViewUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("canvas layer bg"),
                layout: &self.view_bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
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
                uniform,
                texture,
                view,
                bind_group,
                size: (width, height),
            });
            self.layer_epoch = self.layer_epoch.wrapping_add(1);
            self.punch_bind_group = None;
            self.floating_bind_group = None;
        }
    }

    pub fn upload_layer_dirty(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layer_index: usize,
        pixels: &(impl Raster + ?Sized),
        width: u32,
        height: u32,
        dirty: &mut DirtyTiles,
    ) {
        if width == 0 || height == 0 {
            return;
        }
        self.ensure_layers(
            device,
            self.layers.len().max(layer_index + 1),
            width,
            height,
        );
        let Some(Some(layer)) = self.layers.get(layer_index) else {
            return;
        };
        if dirty.is_empty() {
            return;
        }

        for (x, y, w, h) in dirty.drain() {
            if w == 0 || h == 0 {
                continue;
            }
            let packed = pixels.packed_region(width, x as i32, y as i32, w, h);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &layer.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &packed,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
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

    /// A decoded video frame changes every pixel. Keep the texture and upload in one write.
    pub fn upload_video_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &image::RgbaImage,
    ) {
        let (width, height) = frame.dimensions();
        self.ensure_layers(device, 1, width, height);
        let layer = self.layers[0].as_ref().unwrap();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &layer.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            frame.as_raw(),
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
            self.upload_layer_dirty(device, queue, i, *pixels, width, height, &mut dirty);
        }
    }

    pub fn upload_sparse_layers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layers: &[&SparsePixels],
        width: u32,
        height: u32,
    ) {
        self.ensure_layers(device, layers.len(), width, height);
        for (i, pixels) in layers.iter().enumerate() {
            let mut dirty = DirtyTiles::default();
            dirty.resize(width, height);
            self.upload_layer_dirty(device, queue, i, *pixels, width, height, &mut dirty);
        }
    }
}
