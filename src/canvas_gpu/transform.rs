use super::*;

impl CanvasRenderer {
    pub fn sync_sparse_transform(&mut self, device: &wgpu::Device, queue: &wgpu::Queue,
        source: &SparsePixels, reload: bool) -> bool {
        // Scale changes only select a mip level; they never rebuild textures.
        if !reload && !self.floating_tiles.is_empty() { return true; }
        self.floating_tiles.clear();
        let side = TRANSFORM_PREVIEW_SIDE;
        let padding = TRANSFORM_PREVIEW_PAD;
        let texture_side = side + padding * 2;
        let make_texture = |label, format| device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label), size: wgpu::Extent3d { width: texture_side, height: texture_side, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let upload = |texture: &wgpu::Texture, bytes: &[u8], stride| queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: Some(texture_side) },
            wgpu::Extent3d { width: texture_side, height: texture_side, depth_or_array_layers: 1 });
        let mask = make_texture("tile mask", wgpu::TextureFormat::R8Unorm);
        upload(&mask, &vec![255; (texture_side*texture_side) as usize], texture_side);
        let mask_view = mask.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {label:Some("transform mip upload")});
        // Disjoint core quads, with neighbouring texels in each texture's halo.
        // Empty neighbouring cores carry filter tails across occupied tile edges.
        let mut cores = std::collections::HashSet::new();
        let tiles_per_core = (side / crate::tiles::TILE_SIZE) as i32;
        for &(tx,ty) in source.tiles.keys() {
            let (tx,ty) = (tx.div_euclid(tiles_per_core),ty.div_euclid(tiles_per_core));
            for dy in -1..=1 { for dx in -1..=1 { cores.insert((tx+dx,ty+dy)); } }
        }
        for (tx,ty) in cores {
            let x = tx*side as i32+source.offset.0;
            let y = ty*side as i32+source.offset.1;
            let pixels = source.region(x-padding as i32,y-padding as i32,texture_side,texture_side);
            if !pixels.chunks_exact(4).any(|p| p[3] != 0) { continue; }
            let input = make_texture("floating source", wgpu::TextureFormat::Rgba8UnormSrgb);
            upload(&input, &pixels, texture_side*4);
            let texture = self.make_transform_mips(device,&mut encoder,&input.create_view(&Default::default()),&mask_view,texture_side,texture_side);
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("floating tile uniforms"), size: std::mem::size_of::<FloatUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
            });
            let bind_group = self.transform_bind_group(device, &uniform, &texture.create_view(&Default::default()), &mask_view, "floating tile",true);
            self.floating_tiles.push(FloatingTileGpu { _texture: texture, uniform, bind_group,
                origin: Vec2::new(x as f32,y as f32) });
        }
        queue.submit([encoder.finish()]);
        self.tile_mask = Some(mask);
        true
    }

    /// Upload the transform mask and bind the active layer texture for the preview.
    /// Returns false when that layer texture is not on the GPU yet.
    pub fn sync_transform(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layer_index: usize,
        mask: &[u8],
        pixels: &[u8],
        width: u32,
        height: u32,
        reload_mask: bool,
    ) -> bool {
        if width == 0 || height == 0 || mask.len() < width as usize * height as usize {
            self.clear_transform();
            return false;
        }
        let layer_view = {
            let Some(Some(layer)) = self.layers.get(layer_index) else {
                return false;
            };
            layer.view.clone()
        };
        let need_mask = reload_mask || self.mask_size != (width, height) || self.mask_texture.is_none();
        if need_mask {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("canvas transform mask"),
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
            let expanded: Vec<u8> = mask.iter().map(|&v| if v != 0 { 255 } else { 0 }).collect();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &expanded,
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
            let floating_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("floating pixels"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &floating_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                pixels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {label:Some("selection mip upload")});
            self.floating_texture = Some(self.make_transform_mips(device,&mut encoder,
                &floating_texture.create_view(&Default::default()),&view,width,height));
            queue.submit([encoder.finish()]);
            self.mask_texture = Some(texture);
            self.mask_view = Some(view);
            self.mask_size = (width, height);
            self.punch_bind_group = None;
            self.floating_bind_group = None;
        }
        let stale = self.punch_bind_group.is_none()
            || self.floating_bind_group.is_none()
            || self.transform_layer != Some(layer_index)
            || self.transform_epoch != self.layer_epoch;
        if stale {
            let Some(mask_view) = self.mask_view.as_ref() else {
                return false;
            };
            self.punch_bind_group = Some(self.transform_bind_group(
                device,
                &self.punch_uniform_buf,
                &layer_view,
                mask_view,
                "canvas punch bg",
                false,
            ));
            self.floating_bind_group = Some(self.transform_bind_group(
                device,
                &self.float_uniform_buf,
                &self.floating_texture.as_ref().unwrap().create_view(&Default::default()),
                mask_view,
                "canvas float bg",
                true,
            ));
            self.transform_layer = Some(layer_index);
            self.transform_epoch = self.layer_epoch;
        }
        true
    }

    /// Build a premultiplied, linear-light mip pyramid once on the GPU. Both
    /// selections and sparse cores use the same constant-cost preview shader.
    fn make_transform_mips(&self,device:&wgpu::Device,encoder:&mut wgpu::CommandEncoder,
        input:&wgpu::TextureView,mask:&wgpu::TextureView,width:u32,height:u32) -> wgpu::Texture {
        let levels = width.max(height).ilog2()+1;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label:Some("transform mip pyramid"),
            size:wgpu::Extent3d {width,height,depth_or_array_layers:1},
            mip_level_count:levels,sample_count:1,dimension:wgpu::TextureDimension::D2,
            format:wgpu::TextureFormat::Rgba8UnormSrgb,
            usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats:&[],
        });
        for level in 0..levels {
            let destination = texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level:level,mip_level_count:Some(1),..Default::default()
            });
            let previous;
            let source = if level==0 {input} else {
                previous = texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level:level-1,mip_level_count:Some(1),..Default::default()
                });
                &previous
            };
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label:Some("transform mip source"),layout:&self.mip_bind_layout,
                entries:&[
                    wgpu::BindGroupEntry {binding:0,resource:wgpu::BindingResource::TextureView(source)},
                    wgpu::BindGroupEntry {binding:1,resource:wgpu::BindingResource::TextureView(mask)},
                    wgpu::BindGroupEntry {binding:2,resource:wgpu::BindingResource::Sampler(&self.sampler_linear)},
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label:Some("transform mip level"),
                color_attachments:&[Some(wgpu::RenderPassColorAttachment {
                    view:&destination,resolve_target:None,depth_slice:None,
                    ops:wgpu::Operations {load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store},
                })],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None,multiview_mask:None,
            });
            pass.set_pipeline(if level==0 {&self.mip_init_pipeline} else {&self.mip_down_pipeline});
            pass.set_bind_group(0,&bind,&[]);
            pass.draw(0..3,0..1);
        }
        texture
    }

    fn transform_bind_group(
        &self,
        device: &wgpu::Device,
        uniform: &wgpu::Buffer,
        layer_view: &wgpu::TextureView,
        mask_view: &wgpu::TextureView,
        label: &str,
        filtered: bool,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.float_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(layer_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(if filtered {&self.sampler_linear} else {&self.sampler_near}),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(mask_view),
                },
            ],
        })
    }
}
