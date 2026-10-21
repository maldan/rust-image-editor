use super::*;

impl Scene for EditorApp {
    fn request_close(&mut self) -> bool {
        self.cancel_ai();
        true
    }

    fn title() -> &'static str {
        "Mega Image Editor"
    }

    fn init(ui: &mut Ui) {
        ui.set_theme(Theme::dark());
        ui.load_builtin_icons();
    }

    fn begin_frame(&mut self, input: &FrameInput) {
        self.session.canvas_cursor = None;
        self.ai.preview_input = input.clone();
        self.input = if self.ai.open || self.hsv.open {
            let mut blocked = FrameInput::default();
            blocked.key_escape = input.key_escape;
            if self.hsv.open { blocked.key_enter = input.key_enter; }
            blocked
        } else { input.clone() };
    }

    fn build(&mut self, ui: &mut Ui, viewport: Vec2, _stats: DrawStats) -> bool {
        self.session.viewport_size = viewport;
        self.poll_ai(ui);
        if self.input.key_undo {
            self.undo();
        }
        if self.input.key_redo {
            self.redo();
        }
        if self.input.key_escape && self.session.floating.is_some() {
            self.cancel_transform();
        }
        if self.input.key_enter && self.session.floating.is_some() {
            self.commit_transform();
        }
        let thumb_waiting = self.workspace.tick_thumbs();
        let paste = self.input.key_paste;
        let copy = self.input.key_copy;
        ui::draw(ui, self, viewport);
        if copy { self.copy_layer(ui); }
        if paste {
            self.paste_clipboard(ui);
        }
        self.ai.open || self.hsv.open
            || self.session.selection.is_some()
            || self.session.selection_anchor.is_some()
            || self.session.floating.is_some()
            || self.workspace.asset_drag.is_some()
            || thumb_waiting
            || (matches!(self.session.tool, Tool::Brush | Tool::SelectionBrush) && self.session.canvas_rect.contains(self.input.mouse_pos))
            || !self.dirty_tiles.is_empty()
    }

    fn sync_ui_textures(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, renderer: &mut mega_ui::wgpu::UiRenderer) {
        if self.ai.preview_dirty {
            if let Some(preview) = self.ai.result.as_ref().or(self.ai.preview.as_ref()) {
                let limit = device.limits().max_texture_dimension_2d;
                let resized;
                let preview = if preview.width() > limit || preview.height() > limit {
                    resized = image::imageops::thumbnail(preview, limit, limit);
                    &resized
                } else { preview };
                renderer.set_texture_rgba(device, queue, crate::host::AI_PREVIEW_TEXTURE,
                    preview.as_raw(), preview.width(), preview.height(), "AI preview");
            }
            self.ai.preview_dirty = false;
        }
    }

    fn sync_canvas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        canvas: &mut CanvasRenderer,
    ) {
        if let Some(id) = &self.session.preview_asset
            && let Some(asset) = self.workspace.assets.iter().find(|a| &a.id == id)
        {
            if self.session.preview_asset_dirty {
                canvas.upload_all_layers(device, queue, &[&asset.pixels], asset.width, asset.height);
                self.session.preview_asset_dirty = false;
            }
            canvas.clear_transform();
            canvas.set_selection_mask(device, queue, None, 0, 0);
            return;
        }
        if self.image_size.is_none() {
            if self.canvas_cleared || canvas.has_image() {
                canvas.clear();
                self.canvas_cleared = false;
            }
            canvas.clear_transform();
            canvas.set_selection_mask(device, queue, None, 0, 0);
            return;
        }
        let Some((width, height)) = self.image_size else {
            return;
        };
        if self.session.selection_mask_dirty {
            let mask = self.session.selection.as_ref().map(|s| s.mask_r8());
            canvas.set_selection_mask(device, queue, mask.as_deref(), width, height);
            self.session.selection_mask_dirty = false;
        }
        if self.layers_reload {
            let pixels: Vec<&crate::raster::SparsePixels> = self
                .workspace
                .active()
                .layers
                .iter()
                .map(|l| &l.pixels)
                .collect();
            canvas.upload_sparse_layers(device, queue, &pixels, width, height);
            self.layers_reload = false;
            self.dirty_tiles.clear();
            self.dirty_layer = None;
        } else if let Some(layer) = self.dirty_layer
            && !self.dirty_tiles.is_empty()
        {
            if let Some(pixels) = self.workspace.active_mut().layers.get(layer).map(|l| &l.pixels) {
                canvas.upload_layer_dirty(
                    device,
                    queue,
                    layer,
                    pixels,
                    width,
                    height,
                    &mut self.dirty_tiles,
                );
            } else {
                self.dirty_tiles.clear();
            }
            self.dirty_layer = None;
        }
        if self.session.floating.is_none() {
            canvas.clear_transform();
            self.floating_reload = false;
        } else if let Some(floating) = self.session.floating.as_ref() {
            let ready = if let Some(source) = floating.source.as_ref() {
                canvas.sync_sparse_transform(device, queue, source, self.floating_reload)
            } else {
                canvas.sync_transform(device, queue, floating.layer, &floating.mask, &floating.pixels,
                    floating.bw, floating.bh, self.floating_reload)
            };
            if ready {
                self.floating_reload = false;
            }
        }
    }

    fn canvas_present(&self) -> Option<(Rect, CanvasView, CanvasOverlay)> {
        let view = self.canvas_view()?;
        if self.session.preview_asset.is_some() {
            return Some((self.session.canvas_rect, view, CanvasOverlay {
                layers: vec![LayerDraw { visible: true, opacity: 1.0 }],
                ..Default::default()
            }));
        }
        let mut overlay = CanvasOverlay {
            dim_outside: self.session.dim_outside_selection && self.session.selection.is_some(),
            layers: self
                .workspace
                .active()
                .layers
                .iter()
                .map(|l| LayerDraw {
                    visible: l.visible,
                    opacity: l.opacity,
                })
                .collect(),
            ..Default::default()
        };
        if self.hsv.open && self.hsv.preview && self.hsv.document_id == self.workspace.active_doc().id {
            overlay.hsv = Some((self.hsv.layer,self.hsv.adjustment));
        }
        if matches!(self.session.tool, Tool::Brush | Tool::SelectionBrush) && !self.eyedropper_armed() {
            let radius = (self.session.brush_size * self.session.zoom * 0.5).max(2.5);
            let expanded = Rect {
                min: self.session.canvas_rect.min - Vec2::splat(radius),
                max: self.session.canvas_rect.max + Vec2::splat(radius),
            };
            if expanded.contains(self.input.mouse_pos) {
                overlay.brush_center = Some(self.input.mouse_pos);
                overlay.brush_radius = radius;
            }
        }
        if let (Some(anchor), Some(current)) = (self.session.selection_anchor, self.session.selection_current) {
            let rect = PixelRect::from_points(anchor, current, view.width, view.height);
            if !rect.is_empty() {
                overlay.selection_rect = Some(view.pixel_corners(rect));
            }
        }
        if let Some(floating) = self.session.floating.as_ref() {
            overlay.floating = Some(FloatingDraw {
                opacity: floating.opacity,
                whole_layer: floating.whole_layer,
                layer: floating.layer,
                origin: Vec2::new(floating.origin_x as f32, floating.origin_y as f32),
                size: Vec2::new(floating.bw as f32, floating.bh as f32),
            });
            let corners = floating.source_corners().map(|p| view.image_to_screen(p));
            overlay.transform_corners = Some(corners);
            overlay.transform_pivot = Some(view.image_to_screen(floating.pivot));
        }
        Some((self.session.canvas_rect, view, overlay))
    }

    fn canvas_cursor(&self) -> Option<TransformCursor> { self.session.canvas_cursor }
}
