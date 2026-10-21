use super::*;

impl EditorApp {
    pub(in crate::app) fn begin_transform(&mut self) {
        if self.session.floating.is_some() {
            return;
        }
        let Some((width, height)) = self.image_size else {
            self.status = "No image".into();
            return;
        };
        let project = self.workspace.active();
        let layer = project.active_layer;
        let Some(target) = project.layers.get(layer) else { return; };
        let floating = if let Some(selection) = self.session.selection.as_ref() {
            FloatingTransform::extract(&target.pixels, width, height, selection, layer, target.opacity)
        } else {
            FloatingTransform::extract_layer(&target.pixels, layer, target.opacity)
        };
        let Some(floating) = floating else { return; };
        self.session.floating = Some(floating);
        self.floating_reload = true;
        self.clear_selection();
        self.dirty = true;
        self.status = "Transform — inside move · outside rotate · corners scale · Enter apply · Esc cancel".into();
    }

    pub fn flip_transform(&mut self, horizontal: bool) {
        if self.session.tool != Tool::Transform || self.ai.open || self.hsv.open || self.session.preview_asset.is_some() { return; }
        self.begin_transform();
        let Some(floating) = self.session.floating.as_mut() else { return; };
        if horizontal { floating.flip_x = !floating.flip_x; }
        else { floating.flip_y = !floating.flip_y; }
        self.status = if horizontal { "Transform: Flip X" } else { "Transform: Flip Y" }.into();
    }

    pub fn cancel_transform(&mut self) {
        if self.session.floating.take().is_none() {
            self.session.transform_drag = None;
            self.session.transform_last = None;
            return;
        }
        self.stroke_capture = None;
        self.session.transform_drag = None;
        self.session.transform_last = None;
        self.floating_reload = false;
        self.status = "Transform cancelled".into();
    }

    pub(in crate::app) fn commit_transform(&mut self) {
        let Some(floating) = self.session.floating.take() else {
            self.session.transform_drag = None;
            self.session.transform_last = None;
            return;
        };
        let Some((width, height)) = self.image_size else {
            return;
        };
        let layer = floating.layer;
        let Some(pixels) = self.workspace.active_mut().layers.get_mut(layer).map(|l| &mut l.pixels) else { return; };
        let before = pixels.clone();
        let mut transformed_selection = None;
        if floating.whole_layer && floating.scale == Vec2::ONE && floating.rotation == 0.0
            && !floating.flip_x && !floating.flip_y {
            let origin = glam::Vec2::new(floating.origin_x as f32 + floating.bw as f32 * 0.5,
                floating.origin_y as f32 + floating.bh as f32 * 0.5);
            let delta = (floating.center - origin).round();
            pixels.offset.0 += delta.x as i32;
            pixels.offset.1 += delta.y as i32;
        } else if floating.whole_layer {
            *pixels = crate::raster::SparsePixels::default();
            floating.stamp_layer(pixels);
        } else {
            floating.clear_source(pixels, width, height);
            transformed_selection = Some(floating.stamp(pixels, width, height).1);
            pixels.prune();
        }
        let edit = crate::history::HistoryEdit::from_diff(layer, &before, pixels);
        if let Some(edit) = edit { self.history.push(edit); }
        if let Some(selection) = transformed_selection { self.set_selection(selection); }
        self.layers_reload = true;
        self.session.transform_drag = None;
        self.session.transform_last = None;
        self.floating_reload = false;
        self.dirty = true;
        self.workspace.mark_active_thumb_dirty();
        self.status = "Transform applied".into();

    }

    pub fn transform_cursor(&self, view: CanvasView) -> Option<TransformCursor> {
        if self.session.tool != Tool::Transform || self.session.preview_asset.is_some() || self.ai.open || self.hsv.open { return None; }
        let floating = self.session.floating.as_ref()?;
        let hit = self.session.transform_drag.or_else(|| floating.hit_test(
            view.screen_to_image(self.input.mouse_pos),8.0/view.zoom.max(1.0e-6)))?;
        Some(match hit {
            TransformHit::Body => TransformCursor::Move,
            TransformHit::Pivot => TransformCursor::Pivot,
            TransformHit::Rotate => TransformCursor::Rotate,
            TransformHit::Scale(i) => TransformCursor::resize(
                view.image_to_screen(floating.corners()[i as usize])-view.image_to_screen(floating.center)),
        })
    }

    pub(in crate::app) fn handle_transform(&mut self, view: CanvasView) {
        if self.session.floating.is_none() {
            if self.input.mouse_pressed {
                self.begin_transform();
            }
            return;
        }
        if self.input.key_enter {
            self.commit_transform();
            return;
        }
        if self.input.key_escape {
            self.cancel_transform();
            return;
        }
        let image_pos = view.screen_to_image(self.input.mouse_pos);
        let handle_r = 8.0 / view.zoom.max(1e-6);

        if self.input.mouse_pressed {
            let hit = self
                .session.floating
                .as_ref()
                .and_then(|f| f.hit_test(image_pos, handle_r));
            if let Some(hit) = hit {
                self.session.transform_drag = Some(hit);
                self.session.transform_last = Some(image_pos);
            }
        }
        if self.input.mouse_down
            && let (Some(hit), Some(prev)) = (self.session.transform_drag, self.session.transform_last)
        {
            let mut scale = Vec2::ONE;
            let mut rotation = 0.0f32;
            if let Some(floating) = self.session.floating.as_mut() {
                floating.keep_aspect_ratio = self.session.transform_keep_aspect_ratio;
                floating.drag(hit,prev,image_pos);
                scale = floating.scale;
                rotation = floating.rotation;
            }
            self.session.transform_last = Some(image_pos);
            self.status = format!(
                "Transform  scale {:.2} × {:.2}  rot {:.0}°",
                scale.x, scale.y,
                rotation.to_degrees()
            );
        }
        if self.input.mouse_released {
            self.session.transform_drag = None;
            self.session.transform_last = None;
        }
    }
}
