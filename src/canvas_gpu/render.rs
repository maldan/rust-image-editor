use super::*;

impl CanvasRenderer {
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
            hue: 0.0,
            saturation: 0.0,
            value: 0.0,
            _hsv_pad: 0.0,
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
            if overlay.deformation == Some(i) && self.deform.layer == Some(i) {
                if let Some(bind) = &self.deform.bind {
                    let mut u = view_u;
                    u.opacity = layer.opacity.clamp(0.0, 1.0);
                    u._pad = if self.has_selection { 1.0 } else { 0.0 };
                    queue.write_buffer(&self.deform.uniform, 0, bytemuck::bytes_of(&u));
                    pass.set_bind_group(0, bind, &[]);
                    if self.has_selection {
                        pass.set_pipeline(&self.deform.base_pipeline);
                        pass.draw(0..6, 0..1);
                    }
                    pass.set_pipeline(&self.deform.pipeline);
                    pass.set_vertex_buffer(0, self.deform.vertices.slice(..));
                    pass.draw(0..self.deform.count, 0..1);
                    pass.set_pipeline(&self.layer_pipeline);
                    continue;
                }
            }
            if let (Some(draw), Some(corners)) = (overlay.floating, overlay.transform_corners)
                && draw.layer == i
                && draw.whole_layer
            {
                let side = TRANSFORM_PREVIEW_SIDE as f32;
                for tile in &self.floating_tiles {
                    let local = (tile.origin - draw.origin) / draw.size;
                    let size = Vec2::splat(side) / draw.size;
                    let corner = |uv: Vec2| {
                        corners[0]
                            + (corners[1] - corners[0]) * uv.x
                            + (corners[3] - corners[0]) * uv.y
                    };
                    let points = [
                        corner(local),
                        corner(local + Vec2::new(size.x, 0.0)),
                        corner(local + size),
                        corner(local + Vec2::new(0.0, size.y)),
                    ];
                    // Skip tiles outside the visible panel without rebuilding their textures.
                    if points.iter().all(|p| p.x < canvas_rect.min.x)
                        || points.iter().all(|p| p.x > canvas_rect.max.x)
                        || points.iter().all(|p| p.y < canvas_rect.min.y)
                        || points.iter().all(|p| p.y > canvas_rect.max.y)
                    {
                        continue;
                    }
                    let fu = FloatUniforms {
                        viewport: viewport.into(),
                        opacity: draw.opacity,
                        filter_scale: ((corners[1] - corners[0]).length()
                            / draw.size.x
                            / view.zoom)
                            .clamp(0.05, 1.0),
                        origin: [TRANSFORM_PREVIEW_PAD as f32; 2],
                        size: [side; 2],
                        image_size: [side + TRANSFORM_PREVIEW_PAD as f32 * 2.0; 2],
                        c0: points[0].into(),
                        c1: points[1].into(),
                        c2: points[2].into(),
                        c3: points[3].into(),
                    };
                    queue.write_buffer(&tile.uniform, 0, bytemuck::bytes_of(&fu));
                    pass.set_pipeline(&self.float_pipeline);
                    pass.set_bind_group(0, &tile.bind_group, &[]);
                    pass.draw(0..6, 0..1);
                }
                pass.set_pipeline(&self.layer_pipeline);
                continue;
            }
            if overlay
                .floating
                .as_ref()
                .is_some_and(|draw| draw.layer == i)
                && let Some(bg) = self.punch_bind_group.as_ref()
                && let Some(draw) = overlay.floating
            {
                let punch = PunchUniforms {
                    viewport: [viewport.x, viewport.y],
                    center: [view.center.x, view.center.y],
                    image_size: [view.width as f32, view.height as f32],
                    zoom: view.zoom,
                    rotation: view.rotation,
                    opacity: layer.opacity.clamp(0.0, 1.0),
                    _pad: 0.0,
                    origin: [draw.origin.x, draw.origin.y],
                    size: [draw.size.x, draw.size.y],
                };
                queue.write_buffer(&self.punch_uniform_buf, 0, bytemuck::bytes_of(&punch));
                pass.set_pipeline(&self.punch_pipeline);
                pass.set_bind_group(0, bg, &[]);
                pass.draw(0..6, 0..1);
                if let (Some(draw), Some(corners), Some(bg)) = (
                    overlay.floating,
                    overlay.transform_corners,
                    self.floating_bind_group.as_ref(),
                ) && overlay
                    .layers
                    .get(draw.layer)
                    .is_some_and(|layer| layer.visible && layer.opacity > f32::EPSILON)
                {
                    let fu = FloatUniforms {
                        viewport: [viewport.x, viewport.y],
                        opacity: draw.opacity.clamp(0.0, 1.0),
                        filter_scale: ((corners[1] - corners[0]).length()
                            / draw.size.x
                            / view.zoom)
                            .clamp(0.05, 1.0),
                        origin: [0.0; 2],
                        size: [draw.size.x, draw.size.y],
                        image_size: [draw.size.x, draw.size.y],
                        c0: [corners[0].x, corners[0].y],
                        c1: [corners[1].x, corners[1].y],
                        c2: [corners[2].x, corners[2].y],
                        c3: [corners[3].x, corners[3].y],
                    };
                    queue.write_buffer(&self.float_uniform_buf, 0, bytemuck::bytes_of(&fu));
                    pass.set_pipeline(&self.float_pipeline);
                    pass.set_bind_group(0, bg, &[]);
                    pass.draw(0..6, 0..1);
                }

                pass.set_pipeline(&self.layer_pipeline);
                continue;
            }
            let mut u = view_u;
            u.opacity = layer.opacity.clamp(0.0, 1.0);
            if let Some((index, adjustment)) = overlay.hsv {
                if index == i {
                    u.hue = adjustment.hue;
                    u.saturation = adjustment.saturation;
                    u.value = adjustment.value;
                }
            }
            queue.write_buffer(&gpu.uniform, 0, bytemuck::bytes_of(&u));
            pass.set_bind_group(0, &gpu.bind_group, &[]);
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
            if let Some(pivot) = overlay.transform_pivot {
                for (width, color) in [(3.0, black), (1.0, cyan)] {
                    push_circle(&mut verts, pivot, 5.0, width, color);
                    push_line(
                        &mut verts,
                        pivot - Vec2::new(9.0, 0.0),
                        pivot + Vec2::new(9.0, 0.0),
                        width,
                        color,
                    );
                    push_line(
                        &mut verts,
                        pivot - Vec2::new(0.0, 9.0),
                        pivot + Vec2::new(0.0, 9.0),
                        width,
                        color,
                    );
                }
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
