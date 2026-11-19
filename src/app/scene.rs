use super::*;

impl Scene for EditorApp {
    fn request_close(&mut self) -> crate::host::ClosePoll {
        use crate::host::ClosePoll;
        if self.close_aborted {
            self.close_aborted = false;
            return ClosePoll::Abort;
        }
        if self.close_ready {
            self.close_ready = false;
            self.cancel_ai();
            self.close_asset_preview();
            self.video_poster = None;
            return ClosePoll::Exit;
        }
        if self.unsaved == Some(UnsavedAction::Close) {
            return ClosePoll::Wait;
        }
        if self.dirty {
            self.unsaved = Some(UnsavedAction::Close);
            return ClosePoll::Wait;
        }
        self.cancel_ai();
        self.close_asset_preview();
        self.video_poster = None;
        ClosePoll::Exit
    }

    fn title() -> &'static str {
        "Mega Image Editor"
    }

    fn window_title(&self) -> String {
        let mut title = Self::title().to_string();
        if let Some(name) = self.workspace.path.as_deref().map(file_name) {
            title.push_str(" | ");
            title.push_str(name);
        }
        if self.dirty {
            title.push_str(" | Unsaved");
        }
        title
    }

    fn init(ui: &mut Ui) {
        ui.set_theme(Theme::dark());
        ui.load_builtin_icons();
        crate::ui::load_tool_icons(ui);
    }

    fn ui_scale(&self) -> f32 {
        self.settings_dialog.as_ref()
            .map_or(self.settings.ui_scale, |dialog| dialog.settings.ui_scale)
    }

    fn prepare_ui_input(&mut self, input: &FrameInput) {
        self.value_sliders.begin_frame(input);
    }

    fn wants_keyboard(&self) -> bool {
        self.value_sliders.is_editing()
    }

    fn begin_frame(&mut self, input: &FrameInput) {
        self.session.canvas_cursor = None;
        self.ai.preview_input = input.clone();
        self.input = if self.value_sliders.is_editing() {
            let mut blocked = FrameInput::default();
            blocked.dt = input.dt;
            blocked
        } else if self.ai.open
            || self.ai_settings.is_some()
            || self.settings_dialog.is_some()
            || self.hsv.open
            || self.rename.is_some()
            || self.unsaved.is_some()
            || self.delete_prompt.is_some()
            || self.effect_dialog.is_some()
        {
            let mut blocked = FrameInput::default();
            blocked.key_escape = input.key_escape;
            blocked.key_save = self.ai.open && self.unsaved.is_none() && input.key_save;
            blocked.dt = input.dt;
            if self.hsv.open || self.rename.is_some() || self.delete_prompt.is_some() {
                blocked.key_enter = input.key_enter;
            }
            blocked
        } else {
            input.clone()
        };
    }

    fn build(&mut self, ui: &mut Ui, viewport: Vec2, _stats: DrawStats) -> bool {
        self.session.viewport_size = viewport;
        self.note_frame(self.input.dt);
        if self.input.key_escape { self.finish_deformation(true); }
        self.poll_all_graphs(ui);
        self.poll_ai(ui);
        self.poll_video(ui);
        if let Some(error) = self.video_editor.tick(self.input.dt) {
            self.status = error;
        }
        if !self.nodes.visible && self.video_editor.active.is_none() {
            self.handle_tool_hotkeys();
            self.handle_animation_keys();
            self.tick_animation();
            if self.input.key_undo { self.undo(); }
            if self.input.key_redo { self.redo(); }
        }
        if self.input.key_save {
            self.save_project(ui);
        }
        if self.rename.is_none() && self.input.key_escape && self.session.floating.is_some() {
            self.cancel_transform();
        }
        if self.rename.is_none() && self.input.key_enter && self.session.floating.is_some() {
            self.commit_transform();
        }
        let thumb_waiting = self.workspace.tick_thumbs();
        let paste = self.input.key_paste;
        let copy = self.input.key_copy;
        let cut = self.input.key_cut;
        self.video_editor.preview_opacity.clear();
        ui::draw(ui, self, viewport);
        if self.session.deform_cancel || self.input.key_escape {
            self.finish_deformation(true);
        } else if self.session.deform_apply || self.input.key_enter {
            self.finish_deformation(false);
        }
        self.session.deform_cancel = false;
        self.session.deform_apply = false;
        if self.video_editor.active.is_none() {
            self.tick_animation_preview();
        }
        if !self.nodes.visible && self.video_editor.active.is_none() {
            if cut {
                self.cut_selection(ui);
            } else if copy {
                self.copy_shortcut(ui);
            } else if paste {
                self.paste_clipboard(ui);
            } else if self.input.key_delete {
                self.delete_selection();
            }
        }
        // Static tools follow the pointer via mouse events. Only animated
        // overlays and unfinished uploads need another frame on their own.
        self.ai.open
            || self.ai_settings.is_some()
            || self.settings_dialog.is_some()
            || self.animation_playing()
            || self.hsv.open
            || self.rename.is_some()
            || self.unsaved.is_some()
            || self.delete_prompt.is_some()
            || self.effect_dialog.is_some()
            || self
                .video_preview
                .as_ref()
                .is_some_and(crate::video::Playback::needs_repaint)
            || self.graphs_need_repaint()
            || self.video_poster.is_some()
            || self.video_trim.job.is_some()
            || self.video_editor.needs_repaint()
            || (!self.layer_grid_active() && self.session.preview_asset.is_none() && self.session.selection.is_some())
            || thumb_waiting
            || !self.dirty_tiles.is_empty()
    }

    fn finish_ui(&self, output: &mut mega_ui::UiOutput) {
        if self.video_editor.preview_opacity.is_empty() {
            return;
        }
        for command in output.draw_list.iter_mut().filter(|command| command.kind == 1.0) {
            let Some(slot) = command.tex.checked_sub(crate::video_document::PREVIEW_TEXTURE) else { continue };
            if let Some(&opacity) = self.video_editor.preview_opacity.get(slot as usize) {
                for color in &mut command.colors {
                    color[3] *= opacity;
                }
            }
        }
    }

    fn sync_ui_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut mega_ui::wgpu::UiRenderer,
    ) {
        let layers = self.video_editor.previews.iter_mut().filter(|track| track.frame().is_some());
        for (index, track) in layers.enumerate() {
            let slot = crate::video_document::PREVIEW_TEXTURE + index as u32;
            if let Some(frame) = track.upload_frame(slot) {
                renderer.set_texture_rgba(device, queue, slot,
                    frame.as_raw(), frame.width(), frame.height(), "video document preview");
            }
        }
        if self.nodes.live_dirty {
            if let Some(preview) = &self.nodes.live_display {
                renderer.set_texture_rgba(device, queue, crate::nodes::LIVE_PREVIEW_TEXTURE, &preview.pixels, crate::nodes::CELL, crate::nodes::CELL, "live node preview");
            }
            self.nodes.live_dirty = false;
        }
        if self.nodes.atlas_dirty {
            renderer.set_texture_rgba(device, queue, crate::nodes::PREVIEW_TEXTURE, &self.nodes.atlas, crate::nodes::ATLAS, crate::nodes::ATLAS, "node previews");
            self.nodes.atlas_dirty = false;
        }
        self.workspace.prepare_thumb_atlas();
        self.ai.reference_picker.sync_textures(device, queue, renderer);
        if self.workspace.thumb_needs_upload() {
            let (width, height) = self.workspace.thumb_atlas_size;
            renderer.set_texture_rgba_nearest(
                device,
                queue,
                crate::host::THUMB_ATLAS,
                &self.workspace.thumb_atlas,
                width,
                height,
                "library thumbs",
            );
            self.workspace.mark_thumb_atlas_uploaded();
        }
        if self.ai.preview_dirty {
            let anim_frame = self
                .ai
                .anim_frames
                .as_ref()
                .and_then(|frames| frames.get(self.ai.anim_shown));
            if let Some(preview) = anim_frame.or(self.ai.result.as_ref()).or(self.ai.preview.as_ref())
            {
                let limit = device.limits().max_texture_dimension_2d;
                let resized;
                let preview = if preview.width() > limit || preview.height() > limit {
                    resized = image::imageops::thumbnail(preview, limit, limit);
                    &resized
                } else {
                    preview
                };
                renderer.set_texture_rgba(
                    device,
                    queue,
                    crate::host::AI_PREVIEW_TEXTURE,
                    preview.as_raw(),
                    preview.width(),
                    preview.height(),
                    "AI preview",
                );
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
        if let Some(player) = &mut self.video_preview {
            if let Some(frame) = &player.frame {
                if player.dirty {
                    canvas.upload_video_frame(device, queue, frame);
                    player.dirty = false;
                }
            } else {
                canvas.clear();
            }
            canvas.clear_transform();
            canvas.set_selection_mask(device, queue, None, 0, 0);
            return;
        }
        if let Some(id) = &self.session.preview_asset
            && let Some(asset) = self.workspace.assets.iter().find(|a| &a.id == id)
        {
            if self.session.preview_asset_dirty {
                canvas.upload_all_layers(
                    device,
                    queue,
                    &[&asset.pixels],
                    asset.width,
                    asset.height,
                );
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
            if let Some(pixels) = self
                .workspace
                .active_mut()
                .layers
                .get(layer)
                .map(|l| &l.pixels)
            {
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
        canvas.sync_deformation(device, queue, self.deformation.as_ref().map(|d| (&d.mesh, d.layer)));
        if self.session.floating.is_none() {
            canvas.clear_transform();
            self.floating_reload = false;
        } else if let Some(floating) = self.session.floating.as_ref() {
            let ready = if let Some(source) = floating.source.as_ref() {
                canvas.sync_sparse_transform(device, queue, source, self.floating_reload)
            } else {
                canvas.sync_transform(
                    device,
                    queue,
                    floating.layer,
                    &floating.mask,
                    &floating.pixels,
                    floating.bw,
                    floating.bh,
                    self.floating_reload,
                )
            };
            if ready {
                self.floating_reload = false;
            }
        }
    }

    fn canvas_present(&self) -> Option<(Rect, CanvasView, CanvasOverlay)> {
        if self.video_editor.active.is_some() {
            return None;
        }
        let view = self.canvas_view()?;
        if self.session.preview_asset.is_some() {
            return Some((
                self.session.canvas_rect,
                view,
                CanvasOverlay {
                    layers: vec![LayerDraw {
                        visible: true,
                        opacity: 1.0,
                    }],
                    ..Default::default()
                },
            ));
        }
        let mut overlay = CanvasOverlay {
            dim_outside: !self.camera_visible() && self.session.selection.is_some(),
            selection_uv: self.session.selection.as_ref().and_then(selection_overlay_uv),
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
        if self.camera_visible() {
            let animation = self.project().animation.as_ref().unwrap();
            overlay.camera = Some((animation.camera.sample(animation.frame), self.project().layers.iter().map(|l| l.z).collect()));
            return Some((self.session.canvas_rect, view, overlay));
        }
        if self.hsv.open
            && self.hsv.preview
            && self.hsv.document_id == self.workspace.active_doc().id
        {
            if let Some(adjustment) = self.hsv.brightness_contrast {
                overlay.brightness_contrast = Some((self.hsv.layer, adjustment));
            } else {
                overlay.hsv = Some((self.hsv.layer, self.hsv.adjustment));
            }
        }
        if self.layer_grid_active() {
            let layout = self.layer_grid.layout(self.project());
            overlay.layer_grid = Some(layout.centers(view).collect());
            return Some((self.session.canvas_rect, view, overlay));
        }
        overlay.deformation = self.deformation.as_ref().map(|d| d.layer);
        if matches!(self.session.tool, Tool::Brush | Tool::SelectionBrush)
            && !self.eyedropper_armed()
        {
            let radius = (self.session.brush_size * self.session.zoom * 0.5).max(2.5);
            if let Some((origin, _)) = self.brush_size_drag {
                overlay.brush_center = Some(origin);
                overlay.brush_radius = radius;
            } else {
                let expanded = Rect {
                    min: self.session.canvas_rect.min - Vec2::splat(radius),
                    max: self.session.canvas_rect.max + Vec2::splat(radius),
                };
                if expanded.contains(self.input.mouse_pos) {
                    overlay.brush_center = Some(self.input.mouse_pos);
                    overlay.brush_radius = radius;
                }
            }
        }
        if let (Some(anchor), Some(current)) = (
            self.session.selection_anchor,
            self.session.selection_current,
        ) {
            if anchor != current {
                overlay.selection_rect = Some(view.signed_pixel_corners(anchor, current));
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

    fn canvas_cursor(&self) -> Option<TransformCursor> {
        self.session.canvas_cursor
    }
}

fn selection_overlay_uv(selection: &crate::editing::Selection) -> Option<[Vec2; 2]> {
    let bounds = selection.bounds()?;
    let width = selection.width.max(1) as f32;
    let height = selection.height.max(1) as f32;
    let pad_x = 2.0 / width;
    let pad_y = 2.0 / height;
    Some([
        Vec2::new(
            (bounds.min_x as f32 / width - pad_x).max(0.0),
            (bounds.min_y as f32 / height - pad_y).max(0.0),
        ),
        Vec2::new(
            (bounds.max_x as f32 / width + pad_x).min(1.0),
            (bounds.max_y as f32 / height + pad_y).min(1.0),
        ),
    ])
}
