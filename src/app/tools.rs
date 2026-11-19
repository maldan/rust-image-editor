use super::*;

impl EditorApp {
    pub(in crate::app) fn select_mode(&self) -> SelectMode {
        if self.input.key_ctrl {
            SelectMode::Subtract
        } else if self.input.key_shift {
            SelectMode::Add
        } else {
            SelectMode::Replace
        }
    }

    pub(in crate::app) fn apply_selection(&mut self, next: Selection, mode: SelectMode) {
        match mode {
            SelectMode::Replace => self.set_selection(next),
            SelectMode::Add => {
                if let Some(sel) = self.session.selection.as_mut() {
                    sel.union(&next);
                    if sel.is_empty() {
                        self.clear_selection();
                    } else {
                        self.session.selection_mask_dirty = true;
                    }
                } else {
                    self.set_selection(next);
                }
            }
            SelectMode::Subtract => {
                if let Some(sel) = self.session.selection.as_mut() {
                    sel.subtract(&next);
                    if sel.is_empty() {
                        self.clear_selection();
                    } else {
                        self.session.selection_mask_dirty = true;
                    }
                }
            }
        }
    }

    pub(in crate::app) fn commit_selection_drag(&mut self) {
        let Some((width, height)) = self.image_size else {
            self.session.selection_anchor = None;
            self.session.selection_current = None;
            self.session.selection_deselect_press = None;
            return;
        };
        if let (Some(anchor), Some(current)) = (
            self.session.selection_anchor,
            self.session.selection_current,
        ) {
            let rect = PixelRect::from_signed_points(anchor, current, width, height);
            let mode = self.session.selection_mode;
            if rect.is_empty() {
                if mode == SelectMode::Replace {
                    self.clear_selection();
                }
            } else {
                self.apply_selection(Selection::from_rect(rect, width, height), mode);
            }
        }
        self.session.selection_anchor = None;
        self.session.selection_current = None;
        self.session.selection_deselect_press = None;
        self.session.selection_mode = SelectMode::Replace;
    }

    pub(in crate::app) fn handle_tool_hotkeys(&mut self) {
        if self.ai.open || self.hsv.open || self.rename.is_some() {
            return;
        }
        if self.input.key_tool_brush {
            self.set_tool(Tool::Brush);
            self.status = "Brush".into();
        } else if self.input.key_tool_fill {
            self.set_tool(Tool::Fill);
            self.status = "Fill".into();
        } else if self.input.key_tool_transform {
            self.set_tool(Tool::Transform);
            self.status = "Transform".into();
        }
        if self.input.key_toggle_erase {
            self.session.erase_mode = !self.session.erase_mode;
            self.status = if self.session.erase_mode {
                "Erase mode on".into()
            } else {
                "Erase mode off".into()
            };
        }
    }

    pub fn handle_canvas_edit(&mut self, view: CanvasView) {
        if self.layer_grid_active() || self.camera_visible() || self.session.preview_asset.is_some() {
            return;
        }
        let (width, height) = (view.width, view.height);
        if width == 0 || height == 0 {
            return;
        }
        if self.handle_brush_size_gesture() {
            return;
        }
        match self.session.tool {
            Tool::RectangleSelect => self.handle_rect_select(view),
            Tool::Lasso => self.handle_lasso(view),
            Tool::PolygonSelect => self.handle_polygon(view),
            Tool::SelectionBrush => self.handle_selection_brush(view),
            Tool::MagicWand => self.handle_wand(view),
            Tool::Fill => self.handle_fill(view),
            Tool::Brush => self.handle_brush(view, self.session.erase_mode),
            Tool::Transform => self.handle_transform(view),
            Tool::Deform => self.handle_deformation(view),
        }
    }

    /// Krita-style: Shift + drag left/right changes brush size without painting.
    pub(in crate::app) fn handle_brush_size_gesture(&mut self) -> bool {
        if let Some((origin, start_size)) = self.brush_size_drag {
            let dx = self.input.mouse_pos.x - origin.x;
            self.session.brush_size = (start_size + dx * 0.5).round().clamp(1.0, 256.0);
            if self.input.mouse_released || !self.input.mouse_down {
                self.brush_size_drag = None;
            } else {
                self.status = format!("Brush size: {:.0} px", self.session.brush_size);
            }
            return true;
        }
        if !matches!(self.session.tool, Tool::Brush | Tool::SelectionBrush)
            || !self.input.mouse_pressed
            || !self.input.key_shift
            || self.input.key_ctrl
            || !self.session.canvas_rect.contains(self.input.mouse_pos)
            || self.brush_last.is_some()
            || self.session.selection_brush_last.is_some()
        {
            return false;
        }
        self.brush_size_drag = Some((self.input.mouse_pos, self.session.brush_size));
        true
    }

    pub(in crate::app) fn handle_lasso(&mut self, view: CanvasView) {
        if self.input.key_escape {
            self.session.lasso_points.clear();
            return;
        }
        let point = view.screen_to_image(self.input.mouse_pos);
        if self.input.mouse_pressed && self.session.canvas_rect.contains(self.input.mouse_pos) {
            self.session.lasso_points.clear();
            self.session.lasso_points.push(point);
            self.session.selection_mode = self.select_mode();
        }
        if (self.input.mouse_down || self.input.mouse_released)
            && let Some(last) = self.session.lasso_points.last()
            && last.distance_squared(point) > 0.01
        {
            self.session.lasso_points.push(point);
        }
        if self.input.mouse_released && !self.session.lasso_points.is_empty() {
            let selection =
                Selection::from_polygon(&self.session.lasso_points, view.width, view.height);
            self.session.lasso_points.clear();
            self.apply_selection(selection, self.session.selection_mode);
        }
    }

    pub(in crate::app) fn handle_polygon(&mut self, view: CanvasView) {
        if self.input.key_escape {
            self.session.lasso_points.clear();
            return;
        }
        if self.input.key_enter {
            self.close_polygon(view);
            return;
        }
        if !self.input.mouse_pressed {
            return;
        }
        if self.polygon_closes_at(self.input.mouse_pos, view) {
            self.close_polygon(view);
            return;
        }
        if !self.session.canvas_rect.contains(self.input.mouse_pos) {
            return;
        }
        if self.session.lasso_points.is_empty() {
            self.session.selection_mode = self.select_mode();
        }
        self.session
            .lasso_points
            .push(view.screen_to_image(self.input.mouse_pos));
    }

    fn polygon_closes_at(&self, screen: Vec2, view: CanvasView) -> bool {
        self.session.lasso_points.len() >= 3
            && self
                .session
                .lasso_points
                .first()
                .is_some_and(|first| view.image_to_screen(*first).distance(screen) <= 12.0)
    }

    fn close_polygon(&mut self, view: CanvasView) {
        if self.session.lasso_points.len() < 3 {
            return;
        }
        let selection =
            Selection::from_polygon(&self.session.lasso_points, view.width, view.height);
        self.session.lasso_points.clear();
        self.apply_selection(selection, self.session.selection_mode);
        self.status = match self.session.selection_mode {
            SelectMode::Replace => "Polygon selection",
            SelectMode::Add => "Added to selection",
            SelectMode::Subtract => "Subtracted from selection",
        }
        .into();
    }

    pub(in crate::app) fn handle_selection_brush(&mut self, view: CanvasView) {
        let point = view.screen_to_image(self.input.mouse_pos);
        if self.input.mouse_pressed
            && brush_overlaps_image(
                self.input.mouse_pos,
                self.session.brush_size * view.zoom * 0.5,
                view,
            )
        {
            self.session.selection_brush_last = Some(point);
        }
        if (self.input.mouse_down || self.input.mouse_pressed || self.input.mouse_released)
            && let Some(previous) = self.session.selection_brush_last
        {
            let selection = self.session.selection.get_or_insert_with(|| {
                Selection::from_mask(
                    view.width,
                    view.height,
                    vec![0; view.width as usize * view.height as usize],
                )
            });
            selection.paint_segment(
                previous,
                point,
                self.session.brush_size,
                self.session.selection_brush_erase,
            );
            self.session.selection_brush_last = Some(point);
            self.session.selection_mask_dirty = true;
        }
        if self.input.mouse_released {
            self.session.selection_brush_last = None;
            // Keep an empty mask: an erased selection must not turn into an unrestricted canvas.
        }
    }

    pub fn draw_lasso_path(&self, ui: &mut Ui, view: CanvasView) {
        let color = [1.0, 0.85, 0.2, 1.0];
        for pair in self.session.lasso_points.windows(2) {
            ui.line(
                view.image_to_screen(pair[0]),
                view.image_to_screen(pair[1]),
                1.0,
                color,
            );
        }
        let Some(first) = self.session.lasso_points.first() else {
            return;
        };
        let Some(last) = self.session.lasso_points.last() else {
            return;
        };
        let first_screen = view.image_to_screen(*first);
        let last_screen = view.image_to_screen(*last);
        if self.session.tool == Tool::PolygonSelect {
            ui.line(last_screen, self.input.mouse_pos, 1.0, color);
            if self.polygon_closes_at(self.input.mouse_pos, view) {
                ui.line(self.input.mouse_pos, first_screen, 1.0, color);
            }
            for point in &self.session.lasso_points {
                let p = view.image_to_screen(*point);
                ui.fill_rect(
                    Rect::from_min_size(p - Vec2::splat(2.5), Vec2::splat(5.0)),
                    color,
                );
            }
        } else {
            ui.line(last_screen, first_screen, 1.0, color);
        }
    }

    pub(in crate::app) fn handle_rect_select(&mut self, view: CanvasView) {
        if self.input.mouse_pressed && self.session.canvas_rect.contains(self.input.mouse_pos) {
            let mode = self.select_mode();
            self.session.selection_mode = mode;
            let point = screen_to_image_pixel(self.input.mouse_pos, view);
            let in_document = point.0 >= 0
                && point.1 >= 0
                && point.0 < view.width as i32
                && point.1 < view.height as i32;
            if mode == SelectMode::Replace
                && in_document
                && self.session.selection.as_ref().is_some_and(|sel| {
                    !sel.contains(point.0 as u32, point.1 as u32)
                })
            {
                self.clear_selection();
                self.session.selection_deselect_press = Some(self.input.mouse_pos);
            } else {
                self.session.selection_deselect_press = None;
                self.session.selection_anchor = Some(point);
                self.session.selection_current = Some(point);
            }
        }
        if self.input.mouse_down {
            if let Some(origin) = self.session.selection_deselect_press {
                let drag = self.input.mouse_pos - origin;
                if drag.length() >= 4.0 {
                    self.session.selection_anchor = Some(screen_to_image_pixel(origin, view));
                    self.session.selection_current =
                        Some(screen_to_image_pixel(self.input.mouse_pos, view));
                    self.session.selection_deselect_press = None;
                }
            } else if self.session.selection_anchor.is_some() {
                self.session.selection_current =
                    Some(screen_to_image_pixel(self.input.mouse_pos, view));
            }
        }
        if self.input.mouse_released {
            if self.session.selection_deselect_press.is_some() {
                self.session.selection_deselect_press = None;
            } else if self.session.selection_anchor.is_some() {
                self.commit_selection_drag();
            }
        }
    }

    pub(in crate::app) fn handle_wand(&mut self, view: CanvasView) {
        if !self.input.mouse_pressed {
            return;
        }
        let mode = self.select_mode();
        let Some(point) = screen_to_pixel(self.input.mouse_pos, view) else {
            if mode == SelectMode::Replace
                && self.session.canvas_rect.contains(self.input.mouse_pos)
            {
                self.clear_selection();
            }
            return;
        };
        let Some(pixels) = self.workspace.active_mut().active_pixels() else {
            return;
        };
        if let Some(selection) = flood_mask(
            pixels,
            view.width,
            view.height,
            point,
            self.session.wand_threshold.round().clamp(0.0, 255.0) as u8,
            None,
        ) {
            self.apply_selection(selection, mode);
            self.status = match mode {
                SelectMode::Replace => "Magic wand selection",
                SelectMode::Add => "Added to selection",
                SelectMode::Subtract => "Subtracted from selection",
            }
            .into();
        }
    }

    pub(in crate::app) fn handle_fill(&mut self, view: CanvasView) {
        if self.eyedropper_armed() {
            if self.input.mouse_pressed {
                self.pick_composite_color(view);
            }
            return;
        }
        if !self.input.mouse_pressed {
            return;
        }
        let Some(point) = screen_to_pixel(self.input.mouse_pos, view) else {
            return;
        };
        let color = [
            self.session.brush_color[0],
            self.session.brush_color[1],
            self.session.brush_color[2],
        ];
        let opacity = if self.session.erase_mode {
            self.session.brush_opacity
        } else {
            self.session.brush_opacity * self.session.brush_color[3]
        }
        .clamp(0.0, 1.0);
        let threshold = self.session.fill_threshold.round().clamp(0.0, 255.0) as u8;
        let erase = self.session.erase_mode;
        let layer = self.workspace.active_mut().active_layer;
        let selection = self.session.selection.take();
        let Some(pixels) = self.workspace.active_mut().active_pixels_mut() else {
            self.session.selection = selection;
            return;
        };
        let mut capture = EditCapture::new(layer, view.width, view.height);
        // Touch a generous area; flood bounds are applied after we know them via dry mask.
        if let Some(region) = flood_mask(
            pixels,
            view.width,
            view.height,
            point,
            threshold,
            selection.as_ref(),
        ) {
            if let Some(bounds) = region.bounds() {
                capture.touch_rect(pixels, bounds);
            }
        }
        let changed = fill_bucket(
            pixels,
            view.width,
            view.height,
            point,
            threshold,
            color,
            opacity,
            erase,
            selection.as_ref(),
        );
        if let Some(rect) = changed {
            if let Some(edit) = capture.finish(pixels) {
                self.history.push(edit);
            }
            self.dirty = true;
            self.status = if erase { "Erased fill" } else { "Filled" }.into();
            self.session.selection = selection;
            self.mark_dirty_rect(rect);
        } else {
            self.session.selection = selection;
        }
    }

    /// Brush and fill sample the composited image while Ctrl is held.
    pub fn eyedropper_armed(&self) -> bool {
        matches!(self.session.tool, Tool::Brush | Tool::Fill)
            && self.input.key_ctrl
            && self.brush_last.is_none()
            && self.session.preview_asset.is_none()
    }

    pub(in crate::app) fn pick_composite_color(&mut self, view: CanvasView) {
        let Some((x, y)) = screen_to_pixel(self.input.mouse_pos, view) else {
            return;
        };
        let pixel = composite_pixel(&self.project().layers, x as i32, y as i32);
        self.session.brush_color = pixel.map(|channel| channel as f32 / 255.0);
        self.status = format!("Picked #{:02X}{:02X}{:02X}", pixel[0], pixel[1], pixel[2]);
    }

    pub(in crate::app) fn handle_brush(&mut self, view: CanvasView, erase: bool) {
        if self.eyedropper_armed() {
            if self.input.mouse_pressed {
                self.pick_composite_color(view);
            }
            return;
        }
        let radius_screen = (self.session.brush_size * self.session.zoom * 0.5).max(0.5);
        let overlaps = brush_overlaps_image(self.input.mouse_pos, radius_screen, view);
        if self.input.mouse_pressed {
            if !overlaps {
                return;
            }
            self.stroke_id = self.stroke_id.wrapping_add(1).max(1);
            self.soft_brush_coverage.clear();
            let need = view.width as usize * view.height as usize;
            if self.stroke_marks.len() != need {
                self.stroke_marks.resize(need, 0);
            }
            self.stroke_capture = Some(EditCapture::new(
                self.workspace.active_mut().active_layer,
                view.width,
                view.height,
            ));
            let point = screen_to_image(self.input.mouse_pos, view);
            self.brush_last = Some(point);
            self.apply_brush_segment(point, point, view.width, view.height, erase);
        } else if self.input.mouse_down
            && let Some(previous) = self.brush_last
        {
            let current = screen_to_image(self.input.mouse_pos, view);
            self.apply_brush_segment(previous, current, view.width, view.height, erase);
            self.brush_last = Some(current);
        }
        if self.input.mouse_released {
            self.brush_last = None;
            self.soft_brush_coverage.clear();
            if let Some(capture) = self.stroke_capture.take() {
                let layer = capture.layer();
                if let Some(pixels) = self
                    .workspace
                    .active_mut()
                    .layers
                    .get_mut(layer)
                    .map(|l| &mut l.pixels)
                    && let Some(edit) = capture.finish(pixels)
                {
                    self.history.push(edit);
                }
            }
            if self.brush_dirty {
                self.status = "Image modified".into();
                self.brush_dirty = false;
            }
        }
    }

    pub(in crate::app) fn apply_brush_segment(
        &mut self,
        from: (f32, f32),
        to: (f32, f32),
        width: u32,
        height: u32,
        erase: bool,
    ) {
        let opacity = if erase {
            self.session.brush_opacity
        } else {
            self.session.brush_opacity * self.session.brush_color[3]
        }
        .clamp(0.0, 1.0);
        let color = [
            self.session.brush_color[0],
            self.session.brush_color[1],
            self.session.brush_color[2],
        ];
        let size = self.session.brush_size;
        let active = self.workspace.active_mut().active_layer;
        let stroke_id = self.stroke_id;
        let selection = self.session.selection.take();
        let mut marks = std::mem::take(&mut self.stroke_marks);
        let mut soft_coverage = std::mem::take(&mut self.soft_brush_coverage);
        let predicted = brush_dirty_bounds(from, to, size, width, height);
        if let Some(capture) = self.stroke_capture.as_mut()
            && let Some(pixels) = self
                .workspace
                .active_mut()
                .layers
                .get(active)
                .map(|l| &l.pixels)
        {
            capture.touch_rect(pixels, predicted);
        }
        let Some(pixels) = self
            .workspace
            .active_mut()
            .layers
            .get_mut(active)
            .map(|l| &mut l.pixels)
        else {
            self.session.selection = selection;
            self.stroke_marks = marks;
            self.soft_brush_coverage = soft_coverage;
            return;
        };
        let mut stroke = BrushStroke {
            from,
            to,
            size,
            opacity,
            hardness: self.session.brush_hardness,
            erase,
            color,
            selection: selection.as_ref(),
            coverage_marks: Some(marks.as_mut_slice()),
            soft_coverage: Some(&mut soft_coverage),
            stroke_id,
        };
        let dirty = apply_brush(pixels, width, height, &mut stroke);
        self.session.selection = selection;
        self.stroke_marks = marks;
        self.soft_brush_coverage = soft_coverage;
        if let Some(rect) = dirty {
            self.dirty = true;
            self.brush_dirty = true;
            self.status = if erase { "Erasing" } else { "Painting" }.into();
            self.mark_dirty_rect(rect);
        }
    }
}
