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
                if let Some(sel) = self.selection.as_mut() {
                    sel.union(&next);
                    if sel.is_empty() {
                        self.clear_selection();
                    } else {
                        self.selection_mask_dirty = true;
                    }
                } else {
                    self.set_selection(next);
                }
            }
            SelectMode::Subtract => {
                if let Some(sel) = self.selection.as_mut() {
                    sel.subtract(&next);
                    if sel.is_empty() {
                        self.clear_selection();
                    } else {
                        self.selection_mask_dirty = true;
                    }
                }
            }
        }
    }

    pub fn invert_selection(&mut self) {
        let Some(selection) = self.selection.as_mut() else { return; };
        selection.invert();
        self.selection_mask_dirty = true;
        self.status = "Selection inverted".into();
    }

    pub fn clear_selection(&mut self) {
        self.lasso_points.clear();
        self.selection_brush_last = None;
        self.selection = None;
        self.selection_anchor = None;
        self.selection_current = None;
        self.selection_deselect_press = None;
        self.selection_mask_dirty = true;
    }

    pub(in crate::app) fn set_selection(&mut self, selection: Selection) {
        self.selection = Some(selection);
        self.selection_mask_dirty = true;
    }

    pub(in crate::app) fn commit_selection_drag(&mut self) {
        let Some((width, height)) = self.image_size else {
            self.selection_anchor = None;
            self.selection_current = None;
            self.selection_deselect_press = None;
            return;
        };
        if let (Some(anchor), Some(current)) = (self.selection_anchor, self.selection_current) {
            let rect = PixelRect::from_points(anchor, current, width, height);
            let mode = self.selection_mode;
            if rect.is_empty() {
                if mode == SelectMode::Replace {
                    self.clear_selection();
                }
            } else {
                self.apply_selection(Selection::from_rect(rect, width, height), mode);
            }
        }
        self.selection_anchor = None;
        self.selection_current = None;
        self.selection_deselect_press = None;
        self.selection_mode = SelectMode::Replace;
    }

    pub fn handle_canvas_edit(&mut self, view: CanvasView) {
        if self.preview_asset.is_some() { return; }
        let (width, height) = (view.width, view.height);
        if width == 0 || height == 0 {
            return;
        }
        match self.tool {
            Tool::RectangleSelect => self.handle_rect_select(view),
            Tool::Lasso => self.handle_lasso(view),
            Tool::SelectionBrush => self.handle_selection_brush(view),
            Tool::MagicWand => self.handle_wand(view),
            Tool::Fill => self.handle_fill(view),
            Tool::Brush => self.handle_brush(view, self.erase_mode),
            Tool::Transform => self.handle_transform(view),
        }
    }

    pub(in crate::app) fn handle_lasso(&mut self, view: CanvasView) {
        if self.input.key_escape {
            self.lasso_points.clear();
            return;
        }
        let point = view.screen_to_image(self.input.mouse_pos);
        if self.input.mouse_pressed && self.canvas_rect.contains(self.input.mouse_pos) {
            self.lasso_points.clear();
            self.lasso_points.push(point);
            self.selection_mode = self.select_mode();
        }
        if (self.input.mouse_down || self.input.mouse_released)
            && let Some(last) = self.lasso_points.last()
            && last.distance_squared(point) > 0.01
        {
            self.lasso_points.push(point);
        }
        if self.input.mouse_released && !self.lasso_points.is_empty() {
            let selection = Selection::from_polygon(&self.lasso_points, view.width, view.height);
            self.lasso_points.clear();
            self.apply_selection(selection, self.selection_mode);
        }
    }

    pub(in crate::app) fn handle_selection_brush(&mut self, view: CanvasView) {
        let point = view.screen_to_image(self.input.mouse_pos);
        if self.input.mouse_pressed && brush_overlaps_image(self.input.mouse_pos, self.brush_size * view.zoom * 0.5, view) {
            self.selection_brush_last = Some(point);
        }
        if (self.input.mouse_down || self.input.mouse_pressed || self.input.mouse_released)
            && let Some(previous) = self.selection_brush_last
        {
            let selection = self.selection.get_or_insert_with(|| Selection {
                width: view.width, height: view.height,
                mask: vec![0; view.width as usize * view.height as usize],
            });
            selection.paint_segment(previous, point, self.brush_size, self.selection_brush_erase);
            self.selection_brush_last = Some(point);
            self.selection_mask_dirty = true;
        }
        if self.input.mouse_released {
            self.selection_brush_last = None;
            // Keep an empty mask: an erased selection must not turn into an unrestricted canvas.
        }
    }

    pub fn draw_lasso_path(&self, ui: &mut Ui, view: CanvasView) {
        for pair in self.lasso_points.windows(2) {
            ui.line(view.image_to_screen(pair[0]), view.image_to_screen(pair[1]), 1.0, [1.0, 0.85, 0.2, 1.0]);
        }
        if let (Some(first), Some(last)) = (self.lasso_points.first(), self.lasso_points.last()) {
            ui.line(view.image_to_screen(*last), view.image_to_screen(*first), 1.0, [1.0, 0.85, 0.2, 1.0]);
        }
    }

    pub(in crate::app) fn handle_rect_select(&mut self, view: CanvasView) {
        if self.input.mouse_pressed {
            let mode = self.select_mode();
            self.selection_mode = mode;
            if let Some(point) = screen_to_pixel(self.input.mouse_pos, view) {
                if mode == SelectMode::Replace
                    && self
                        .selection
                        .as_ref()
                        .is_some_and(|sel| !sel.contains(point.0, point.1))
                {
                    self.clear_selection();
                    self.selection_deselect_press = Some(self.input.mouse_pos);
                } else {
                    self.selection_deselect_press = None;
                    self.selection_anchor = Some(point);
                    self.selection_current = Some(point);
                }
            } else if mode == SelectMode::Replace
                && self.canvas_rect.contains(self.input.mouse_pos)
                && self.selection.is_some()
            {
                self.clear_selection();
            }
        }
        if self.input.mouse_down {
            if let Some(origin) = self.selection_deselect_press {
                let drag = self.input.mouse_pos - origin;
                if drag.length() >= 4.0
                    && let (Some(anchor), Some(current)) = (
                        screen_to_pixel_clamped(origin, view),
                        screen_to_pixel_clamped(self.input.mouse_pos, view),
                    )
                {
                    self.selection_anchor = Some(anchor);
                    self.selection_current = Some(current);
                    self.selection_deselect_press = None;
                }
            } else if self.selection_anchor.is_some()
                && let Some(point) = screen_to_pixel_clamped(self.input.mouse_pos, view)
            {
                self.selection_current = Some(point);
            }
        }
        if self.input.mouse_released {
            if self.selection_deselect_press.is_some() {
                self.selection_deselect_press = None;
            } else if self.selection_anchor.is_some() {
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
            if mode == SelectMode::Replace && self.canvas_rect.contains(self.input.mouse_pos) {
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
            self.wand_threshold.round().clamp(0.0, 255.0) as u8,
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
        if !self.input.mouse_pressed {
            return;
        }
        let Some(point) = screen_to_pixel(self.input.mouse_pos, view) else {
            return;
        };
        let color = [
            self.brush_color[0],
            self.brush_color[1],
            self.brush_color[2],
        ];
        let opacity = if self.erase_mode {
            self.brush_opacity
        } else {
            self.brush_opacity * self.brush_color[3]
        }
        .clamp(0.0, 1.0);
        let threshold = self.fill_threshold.round().clamp(0.0, 255.0) as u8;
        let erase = self.erase_mode;
        let layer = self.workspace.active_mut().active_layer;
        let selection = self.selection.take();
        let Some(pixels) = self.workspace.active_mut().active_pixels_mut() else {
            self.selection = selection;
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
            self.selection = selection;
            self.mark_dirty_rect(rect);
        } else {
            self.selection = selection;
        }
    }

    /// Brush tool with Ctrl held samples the composited image instead of painting.
    pub fn eyedropper_armed(&self) -> bool {
        self.tool == Tool::Brush
            && self.input.key_ctrl
            && self.brush_last.is_none()
            && self.preview_asset.is_none()
    }

    pub(in crate::app) fn pick_composite_color(&mut self, view: CanvasView) {
        let Some((x, y)) = screen_to_pixel(self.input.mouse_pos, view) else {
            return;
        };
        let pixel = composite_pixel(&self.project().layers, x as i32, y as i32);
        self.brush_color = pixel.map(|channel| channel as f32 / 255.0);
        self.status = format!("Picked #{:02X}{:02X}{:02X}", pixel[0], pixel[1], pixel[2]);
    }

    pub(in crate::app) fn handle_brush(&mut self, view: CanvasView, erase: bool) {
        if self.eyedropper_armed() {
            if self.input.mouse_pressed {
                self.pick_composite_color(view);
            }
            return;
        }
        let radius_screen = (self.brush_size * self.zoom * 0.5).max(0.5);
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
                if let Some(pixels) = self.workspace.active_mut().layers.get_mut(layer).map(|l| &mut l.pixels)
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
            self.brush_opacity
        } else {
            self.brush_opacity * self.brush_color[3]
        }
        .clamp(0.0, 1.0);
        let color = [
            self.brush_color[0],
            self.brush_color[1],
            self.brush_color[2],
        ];
        let size = self.brush_size;
        let active = self.workspace.active_mut().active_layer;
        let stroke_id = self.stroke_id;
        let selection = self.selection.take();
        let mut marks = std::mem::take(&mut self.stroke_marks);
        let mut soft_coverage = std::mem::take(&mut self.soft_brush_coverage);
        let predicted = brush_dirty_bounds(from, to, size, width, height);
        if let Some(capture) = self.stroke_capture.as_mut()
            && let Some(pixels) = self.workspace.active_mut().layers.get(active).map(|l| &l.pixels)
        {
            capture.touch_rect(pixels, predicted);
        }
        let Some(pixels) = self.workspace.active_mut().layers.get_mut(active).map(|l| &mut l.pixels) else {
            self.selection = selection;
            self.stroke_marks = marks;
            self.soft_brush_coverage = soft_coverage;
            return;
        };
        let mut stroke = BrushStroke {
            from,
            to,
            size,
            opacity,
            hardness: self.brush_hardness,
            erase,
            color,
            selection: selection.as_ref(),
            coverage_marks: Some(marks.as_mut_slice()),
            soft_coverage: Some(&mut soft_coverage),
            stroke_id,
        };
        let dirty = apply_brush(pixels, width, height, &mut stroke);
        self.selection = selection;
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
