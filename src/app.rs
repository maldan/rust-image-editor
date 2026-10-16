use std::path::{Path, PathBuf};

use glam::Vec2;
use mega_ui::wgpu::DrawStats;
use mega_ui::{DockState, Rect, Theme, Ui};

use crate::canvas_gpu::{CanvasOverlay, CanvasRenderer, FloatingDraw, LayerDraw};
use crate::editing::{
    BrushStroke, FloatingTransform, PixelRect, SelectMode, Selection, Tool, TransformHit,
    apply_brush, brush_dirty_bounds, fill_bucket, flood_mask,
};
use crate::history::{EditCapture, History};
use crate::host::{FrameInput, Scene};
use crate::project::Project;
use crate::tiles::DirtyTiles;
use crate::ui;
use crate::view::CanvasView;
use crate::workspace::Workspace;

pub struct EditorApp {
    pub workspace: Workspace,
    pub input: FrameInput,
    pub image_size: Option<(u32, u32)>,
    pub dirty_tiles: DirtyTiles,
    /// Layer index that `dirty_tiles` refer to.
    dirty_layer: Option<usize>,
    layers_reload: bool,
    floating_reload: bool,
    pub canvas_rect: Rect,
    pub pan: Vec2,
    pub zoom: f32,
    pub rotation: f32,
    pub fit_image: bool,
    pub dirty: bool,
    pub status: String,
    pub dock: DockState,
    pub tool: Tool,
    /// Erase mode for Brush and Fill (not a separate tool).
    pub erase_mode: bool,
    pub brush_size: f32,
    pub brush_opacity: f32,
    pub brush_color: [f32; 4],
    pub wand_threshold: f32,
    pub fill_threshold: f32,
    pub dim_outside_selection: bool,
    pub selection: Option<Selection>,
    pub selection_anchor: Option<(u32, u32)>,
    pub selection_current: Option<(u32, u32)>,
    selection_mode: SelectMode,
    selection_deselect_press: Option<Vec2>,
    selection_mask_dirty: bool,
    brush_last: Option<(f32, f32)>,
    brush_dirty: bool,
    stroke_marks: Vec<u32>,
    stroke_id: u32,
    stroke_capture: Option<EditCapture>,
    history: History,
    floating: Option<FloatingTransform>,
    transform_drag: Option<TransformHit>,
    transform_last: Option<Vec2>,
    pub layer_drag: Option<usize>,
    canvas_cleared: bool,
    pub resize_w: u32,
    pub resize_h: u32,
}

impl Default for EditorApp {
    fn default() -> Self {
        Self {
            workspace: Workspace::default(),
            input: FrameInput::default(),
            image_size: None,
            dirty_tiles: DirtyTiles::default(),
            dirty_layer: None,
            layers_reload: false,
            floating_reload: false,
            canvas_rect: Rect::from_min_size(Vec2::ZERO, Vec2::ZERO),
            pan: Vec2::ZERO,
            zoom: 1.0,
            rotation: 0.0,
            fit_image: false,
            dirty: false,
            status: "Import assets, place them on a document, or resize canvas".into(),
            dock: ui::default_dock(),
            tool: Tool::Brush,
            erase_mode: false,
            brush_size: 24.0,
            brush_opacity: 1.0,
            brush_color: [0.0, 0.0, 0.0, 1.0],
            wand_threshold: 32.0,
            fill_threshold: 32.0,
            dim_outside_selection: false,
            selection: None,
            selection_anchor: None,
            selection_current: None,
            selection_mode: SelectMode::Replace,
            selection_deselect_press: None,
            selection_mask_dirty: true,
            brush_last: None,
            brush_dirty: false,
            stroke_marks: Vec::new(),
            stroke_id: 1,
            stroke_capture: None,
            history: History::default(),
            floating: None,
            transform_drag: None,
            transform_last: None,
            layer_drag: None,
            canvas_cleared: true,
            resize_w: 512,
            resize_h: 512,
        }
    }
}

impl EditorApp {
    pub fn project(&self) -> &Project {
        self.workspace.active()
    }

    pub fn project_mut(&mut self) -> &mut Project {
        self.workspace.active_mut()
    }

    pub fn brush_last_active(&self) -> bool {
        self.brush_last.is_some() || self.transform_drag.is_some()
    }

    pub fn has_floating_transform(&self) -> bool {
        self.floating.is_some()
    }

    pub fn canvas_view(&self) -> Option<CanvasView> {
        let (width, height) = self.image_size?;
        Some(CanvasView {
            center: self.canvas_rect.min + rect_size(self.canvas_rect) * 0.5 + self.pan,
            zoom: self.zoom,
            rotation: self.rotation.to_radians(),
            width,
            height,
        })
    }

    pub fn new_project(&mut self, ui: &mut Ui) {
        self.cancel_transform();
        self.workspace = Workspace::default();
        self.image_size = None;
        self.dirty_tiles.clear();
        self.dirty_layer = None;
        self.layers_reload = false;
        self.floating_reload = false;
        self.canvas_cleared = true;
        self.history.clear();
        self.clear_selection();
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        self.rotation = 0.0;
        self.dirty = true;
        self.status = "New project created".into();
        ui.notify_success("New project created");
    }

    fn select_mode(&self) -> SelectMode {
        if self.input.key_ctrl {
            SelectMode::Subtract
        } else if self.input.key_shift {
            SelectMode::Add
        } else {
            SelectMode::Replace
        }
    }

    fn apply_selection(&mut self, next: Selection, mode: SelectMode) {
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

    pub fn import_png(&mut self, ui: &mut Ui) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .pick_file()
        else {
            return;
        };
        match self.workspace.import_asset_png(&path) {
            Ok(_) => {
                self.dirty = true;
                self.status = format!("Imported asset {}", file_name(&path));
                ui.notify_success("PNG imported as asset (not placed on document)");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn paste_image(&mut self, ui: &mut Ui) {
        match paste_image_from_clipboard().and_then(|(pixels, width, height)| {
            self.workspace
                .import_asset_rgba("Clipboard".into(), &pixels, width, height)?;
            Ok(())
        }) {
            Ok(()) => {
                self.dirty = true;
                self.status = "Pasted image as asset".into();
                ui.notify_success("Clipboard image imported as asset");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn save_project(&mut self, ui: &mut Ui) {
        let path = self.workspace.path.clone().or_else(|| {
            rfd::FileDialog::new()
                .add_filter("Mega Image Project", &["mgimg"])
                .set_file_name("project.mgimg")
                .save_file()
                .map(ensure_project_extension)
        });
        let Some(path) = path else { return };
        match self.workspace.save(&path) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", file_name(&path));
                ui.notify_success("Project saved");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn open_project(&mut self, ui: &mut Ui) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Mega Image Project", &["mgimg"])
            .pick_file()
        else {
            return;
        };
        match Workspace::open(&path) {
            Ok(workspace) => {
                self.cancel_transform();
                self.workspace = workspace;
                match self.refresh_image() {
                    Ok(()) => {
                        self.dirty = false;
                        self.status = format!("Opened {}", file_name(&path));
                        ui.notify_success("Project opened");
                    }
                    Err(error) => self.show_error(ui, error),
                }
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn switch_document(&mut self, index: usize) {
        if index == self.workspace.active_document {
            return;
        }
        self.cancel_transform();
        self.workspace.set_active_document(index);
        self.history.clear();
        let _ = self.refresh_image();
        self.dirty = true;
        self.status = format!("Document: {}", self.workspace.active_doc().name);
    }

    pub fn place_asset(&mut self, asset_index: usize, ui: &mut Ui) {
        self.cancel_transform();
        match self.workspace.place_asset_on_active(asset_index) {
            Ok(()) => {
                self.dirty = true;
                let _ = self.refresh_image();
                self.status = "Asset placed on document".into();
                ui.notify_success("Asset placed as layer");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn export_png(&mut self, ui: &mut Ui) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name("export.png")
            .save_file()
            .map(ensure_png_extension)
        else {
            return;
        };
        match self.workspace.active_mut().export_png(&path) {
            Ok(()) => {
                self.status = format!("Exported {}", file_name(&path));
                ui.notify_success("PNG exported");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn add_layer(&mut self) {
        if self.image_size.is_none() {
            return;
        }
        self.workspace.active_mut().add_layer();
        self.dirty = true;
        self.status = "Layer added".into();
        self.layers_reload = true;
        self.workspace.mark_active_thumb_dirty();
    }

    pub fn delete_layer(&mut self) {
        if self.image_size.is_none() {
            return;
        }
        self.cancel_transform();
        self.workspace.active_mut().delete_active_layer();
        self.history.clear();
        self.dirty = true;
        if self.workspace.active_mut().layers.is_empty() {
            self.image_size = None;
            self.dirty_tiles.clear();
            self.dirty_layer = None;
            self.layers_reload = false;
            self.canvas_cleared = true;
            self.clear_selection();
            self.status = "Last layer deleted".into();
        } else {
            self.status = "Layer deleted".into();
            self.layers_reload = true;
        }
    }

    pub fn move_layer_to(&mut self, from: usize, to: usize) {
        self.workspace.active_mut().move_layer_to(from, to);
        self.dirty = true;
        self.layers_reload = true;
    }

    pub fn set_layer_visible(&mut self, index: usize, visible: bool) {
        if let Some(layer) = self.workspace.active_mut().layers.get_mut(index) {
            layer.visible = visible;
            self.dirty = true;
            self.workspace.mark_active_thumb_dirty();
        }
    }

    pub fn set_active_layer_opacity(&mut self, opacity: f32) {
        let index = self.workspace.active_mut().active_layer;
        if let Some(layer) = self.workspace.active_mut().layers.get_mut(index) {
            layer.opacity = opacity.clamp(0.0, 1.0);
            self.dirty = true;
            if let Some(floating) = self.floating.as_mut()
                && floating.layer == index
            {
                floating.opacity = layer.opacity;
            }
            self.workspace.mark_active_thumb_dirty();
        }
    }

    pub fn resize_canvas(&mut self, ui: &mut Ui) {
        let w = self.resize_w.max(1);
        let h = self.resize_h.max(1);
        self.cancel_transform();
        match self.workspace.active_mut().resize_canvas(w, h) {
            Ok(()) => {
                self.history.clear();
                self.dirty = true;
                self.status = format!("Canvas resized to {w} × {h}");
                ui.notify_success("Canvas resized");
                let _ = self.refresh_image();
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    pub fn undo(&mut self) {
        self.cancel_transform();
        let Some((width, height)) = self.image_size else {
            return;
        };
        if let Some((_, rect)) = self.history.undo(&mut self.workspace.active_mut().layers, width, height) {
            self.dirty = true;
            self.status = "Undo".into();
            self.mark_dirty_rect(rect);
        }
    }

    pub fn redo(&mut self) {
        self.cancel_transform();
        let Some((width, height)) = self.image_size else {
            return;
        };
        if let Some((_, rect)) = self.history.redo(&mut self.workspace.active_mut().layers, width, height) {
            self.dirty = true;
            self.status = "Redo".into();
            self.mark_dirty_rect(rect);
        }
    }

    pub fn refresh_image(&mut self) -> Result<(), String> {
        self.cancel_transform();
        self.history.clear();
        match self.workspace.active_mut().decoded_rgba()? {
            Some((_, width, height)) => {
                self.image_size = Some((width, height));
                self.resize_w = width;
                self.resize_h = height;
                self.stroke_marks = vec![0; width as usize * height as usize];
                self.dirty_tiles.resize(width, height);
                self.dirty_layer = None;
                self.layers_reload = true;
                self.floating_reload = false;
                self.canvas_cleared = false;
                self.fit_image = true;
                self.clear_selection();
            }
            None => {
                self.image_size = None;
                self.dirty_tiles.clear();
                self.dirty_layer = None;
                self.layers_reload = false;
                self.canvas_cleared = true;
                self.clear_selection();
            }
        }
        Ok(())
    }

    fn mark_dirty_rect(&mut self, rect: PixelRect) {
        let layer = self
            .floating
            .as_ref()
            .map(|f| f.layer)
            .unwrap_or(self.workspace.active().active_layer);
        self.mark_layer_dirty(layer, rect);
        self.workspace.mark_active_thumb_dirty();
    }

    fn mark_layer_dirty(&mut self, layer: usize, rect: PixelRect) {
        if self.dirty_layer.is_some_and(|l| l != layer) && !self.dirty_tiles.is_empty() {
            // Different layer still pending upload — force full reload.
            self.layers_reload = true;
            self.dirty_tiles.clear();
            self.dirty_layer = None;
            return;
        }
        self.dirty_layer = Some(layer);
        self.dirty_tiles
            .mark_rect(rect.min_x, rect.min_y, rect.max_x, rect.max_y);
    }

    fn show_error(&mut self, ui: &mut Ui, error: String) {
        self.status = error.clone();
        ui.notify_error(&error);
    }

    pub fn update_view(&mut self) {
        if !self.canvas_rect.contains(self.input.mouse_pos) {
            return;
        }
        if self.input.mouse_middle_down {
            self.pan += self.input.mouse_delta;
        }
        if self.input.scroll_delta.y.abs() > 0.01 {
            let old_zoom = self.zoom;
            let factor = (self.input.scroll_delta.y * 0.002).exp();
            self.zoom = (self.zoom * factor).clamp(0.05, 32.0);
            let center = self.canvas_rect.min + rect_size(self.canvas_rect) * 0.5;
            let cursor_from_image_center = self.input.mouse_pos - center - self.pan;
            self.pan -= cursor_from_image_center * (self.zoom / old_zoom - 1.0);
        }
    }

    pub fn fit_to_canvas(&mut self) {
        let Some((width, height)) = self.image_size else {
            return;
        };
        let available = (rect_size(self.canvas_rect) - Vec2::splat(32.0)).max(Vec2::splat(1.0));
        self.zoom = (available.x / width as f32)
            .min(available.y / height as f32)
            .clamp(0.05, 1.0);
        self.pan = Vec2::ZERO;
        self.fit_image = false;
    }

    pub fn set_tool(&mut self, tool: Tool) {
        if self.tool == tool {
            return;
        }
        if self.tool == Tool::Transform && tool != Tool::Transform {
            self.commit_transform();
        }
        self.commit_selection_drag();
        self.brush_last = None;
        self.tool = tool;
        if tool == Tool::Transform {
            self.begin_transform();
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.selection_anchor = None;
        self.selection_current = None;
        self.selection_deselect_press = None;
        self.selection_mask_dirty = true;
    }

    fn set_selection(&mut self, selection: Selection) {
        self.selection = Some(selection);
        self.selection_mask_dirty = true;
    }

    fn commit_selection_drag(&mut self) {
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
        let (width, height) = (view.width, view.height);
        if width == 0 || height == 0 {
            return;
        }
        match self.tool {
            Tool::RectangleSelect => self.handle_rect_select(view),
            Tool::MagicWand => self.handle_wand(view),
            Tool::Fill => self.handle_fill(view),
            Tool::Brush => self.handle_brush(view, self.erase_mode),
            Tool::Transform => self.handle_transform(view),
        }
    }

    fn handle_rect_select(&mut self, view: CanvasView) {
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

    fn handle_wand(&mut self, view: CanvasView) {
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

    fn handle_fill(&mut self, view: CanvasView) {
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

    fn handle_brush(&mut self, view: CanvasView, erase: bool) {
        let radius_screen = (self.brush_size * self.zoom * 0.5).max(0.5);
        let overlaps = brush_overlaps_image(self.input.mouse_pos, radius_screen, view);
        if self.input.mouse_pressed {
            if !overlaps {
                return;
            }
            self.stroke_id = self.stroke_id.wrapping_add(1).max(1);
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
            if let Some(capture) = self.stroke_capture.take() {
                let layer = capture.layer();
                if let Some(pixels) = self.workspace.active_mut().layers.get(layer).map(|l| l.pixels.as_slice())
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

    fn apply_brush_segment(
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
        let predicted = brush_dirty_bounds(from, to, size, width, height);
        if let Some(capture) = self.stroke_capture.as_mut()
            && let Some(pixels) = self.workspace.active_mut().layers.get(active).map(|l| l.pixels.as_slice())
        {
            capture.touch_rect(pixels, predicted);
        }
        let Some(pixels) = self.workspace.active_mut().layers.get_mut(active).map(|l| &mut l.pixels) else {
            self.selection = selection;
            self.stroke_marks = marks;
            return;
        };
        let mut stroke = BrushStroke {
            from,
            to,
            size,
            opacity,
            erase,
            color,
            selection: selection.as_ref(),
            coverage_marks: Some(marks.as_mut_slice()),
            stroke_id,
        };
        let dirty = apply_brush(pixels, width, height, &mut stroke);
        self.selection = selection;
        self.stroke_marks = marks;
        if let Some(rect) = dirty {
            self.dirty = true;
            self.brush_dirty = true;
            self.status = if erase { "Erasing" } else { "Painting" }.into();
            self.mark_dirty_rect(rect);
        }
    }

    fn begin_transform(&mut self) {
        if self.floating.is_some() {
            return;
        }
        let Some((width, height)) = self.image_size else {
            self.status = "No image".into();
            return;
        };
        let selection = self
            .selection
            .clone()
            .unwrap_or_else(|| Selection::all(width, height));
        let layer = self.workspace.active_mut().active_layer;
        let opacity = self
            .workspace
            .active()
            .layers
            .get(layer)
            .map(|l| l.opacity)
            .unwrap_or(1.0);
        let Some(pixels) = self.workspace.active_mut().layers.get_mut(layer).map(|l| &mut l.pixels) else {
            return;
        };
        let mut capture = EditCapture::new(layer, width, height);
        if let Some(bounds) = selection.bounds() {
            capture.touch_rect(pixels, bounds);
        }
        let Some((floating, bounds)) =
            FloatingTransform::extract(pixels, width, height, &selection, layer, opacity)
        else {
            self.stroke_capture = None;
            return;
        };
        self.stroke_capture = Some(capture);
        self.floating = Some(floating);
        self.floating_reload = true;
        self.clear_selection();
        self.dirty = true;
        self.status = "Transform — drag / handles · Enter commit · Esc cancel".into();
        self.mark_layer_dirty(layer, bounds);
    }

    pub fn cancel_transform(&mut self) {
        if self.floating.take().is_none() {
            self.transform_drag = None;
            self.transform_last = None;
            return;
        }
        if let Some(capture) = self.stroke_capture.take() {
            let layer = capture.layer();
            if let Some(pixels) = self.workspace.active_mut().layers.get_mut(layer).map(|l| &mut l.pixels) {
                if let Some(rect) = capture.restore_before(pixels) {
                    self.mark_dirty_rect(rect);
                }
            }
        }
        self.transform_drag = None;
        self.transform_last = None;
        self.floating_reload = false;
        self.status = "Transform cancelled".into();
        self.layers_reload = true;
    }

    fn commit_transform(&mut self) {
        let Some(floating) = self.floating.take() else {
            self.transform_drag = None;
            self.transform_last = None;
            return;
        };
        let Some((width, height)) = self.image_size else {
            return;
        };
        let layer = floating.layer;
        let mut capture = self
            .stroke_capture
            .take()
            .unwrap_or_else(|| EditCapture::new(layer, width, height));
        // Expand capture to destination before stamping.
        let corners = floating.corners();
        let dest = PixelRect {
            min_x: corners
                .iter()
                .map(|p| p.x)
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.0) as u32,
            min_y: corners
                .iter()
                .map(|p| p.y)
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.0) as u32,
            max_x: corners
                .iter()
                .map(|p| p.x)
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil()
                .min(width as f32) as u32,
            max_y: corners
                .iter()
                .map(|p| p.y)
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil()
                .min(height as f32) as u32,
        };
        if let Some(pixels) = self.workspace.active_mut().layers.get(layer).map(|l| l.pixels.as_slice()) {
            capture.touch_rect(pixels, dest);
        }
        let Some(pixels) = self.workspace.active_mut().layers.get_mut(layer).map(|l| &mut l.pixels) else {
            return;
        };
        let (dirty, selection) = floating.stamp(pixels, width, height);
        if let Some(edit) = capture.finish(pixels) {
            self.history.push(edit);
        }
        self.set_selection(selection);
        self.transform_drag = None;
        self.transform_last = None;
        self.floating_reload = false;
        self.dirty = true;
        self.status = "Transform applied".into();
        if let Some(rect) = dirty {
            self.mark_layer_dirty(layer, rect);
        } else {
            self.layers_reload = true;
        }
    }

    fn handle_transform(&mut self, view: CanvasView) {
        if self.floating.is_none() {
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
        let handle_r = (8.0 / view.zoom.max(1e-6)).max(4.0);

        if self.input.mouse_pressed {
            let hit = self
                .floating
                .as_ref()
                .and_then(|f| f.hit_test(image_pos, handle_r));
            if let Some(hit) = hit {
                self.transform_drag = Some(hit);
                self.transform_last = Some(image_pos);
            } else if self.canvas_rect.contains(self.input.mouse_pos) {
                self.commit_transform();
            }
        }
        if self.input.mouse_down
            && let (Some(hit), Some(prev)) = (self.transform_drag, self.transform_last)
        {
            let mut scale = 1.0f32;
            let mut rotation = 0.0f32;
            if let Some(floating) = self.floating.as_mut() {
                match hit {
                    TransformHit::Body => {
                        floating.center += image_pos - prev;
                    }
                    TransformHit::Rotate => {
                        let a0 = (prev - floating.center).y.atan2((prev - floating.center).x);
                        let a1 = (image_pos - floating.center)
                            .y
                            .atan2((image_pos - floating.center).x);
                        floating.rotation += a1 - a0;
                    }
                    TransformHit::Scale(_) => {
                        let d0 = (prev - floating.center).length().max(1.0);
                        let d1 = (image_pos - floating.center).length().max(1.0);
                        floating.scale = (floating.scale * (d1 / d0)).clamp(0.05, 32.0);
                    }
                }
                scale = floating.scale;
                rotation = floating.rotation;
            }
            self.transform_last = Some(image_pos);
            self.status = format!(
                "Transform  scale {:.2}  rot {:.0}°",
                scale,
                rotation.to_degrees()
            );
        }
        if self.input.mouse_released {
            self.transform_drag = None;
            self.transform_last = None;
        }
    }
}

impl Scene for EditorApp {
    fn title() -> &'static str {
        "Mega Image Editor"
    }

    fn init(ui: &mut Ui) {
        ui.set_theme(Theme::dark());
        ui.load_builtin_icons();
    }

    fn begin_frame(&mut self, input: &FrameInput) {
        self.input = input.clone();
    }

    fn build(&mut self, ui: &mut Ui, viewport: Vec2, _stats: DrawStats) -> bool {
        if self.input.key_undo {
            self.undo();
        }
        if self.input.key_redo {
            self.redo();
        }
        if self.input.key_escape && self.floating.is_some() {
            self.cancel_transform();
        }
        if self.input.key_enter && self.floating.is_some() {
            self.commit_transform();
        }
        let thumb_waiting = self.workspace.tick_thumbs();
        let paste = self.input.key_paste;
        ui::draw(ui, self, viewport);
        if paste {
            self.paste_image(ui);
        }
        self.selection.is_some()
            || self.selection_anchor.is_some()
            || self.floating.is_some()
            || self.workspace.asset_drag.is_some()
            || thumb_waiting
            || (self.tool == Tool::Brush && self.canvas_rect.contains(self.input.mouse_pos))
            || !self.dirty_tiles.is_empty()
    }

    fn sync_canvas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        canvas: &mut CanvasRenderer,
    ) {
        if self.image_size.is_none() {
            if self.canvas_cleared || canvas.has_image() {
                canvas.clear();
                self.canvas_cleared = false;
            }
            canvas.clear_floating();
            canvas.set_selection_mask(device, queue, None, 0, 0);
            return;
        }
        let Some((width, height)) = self.image_size else {
            return;
        };
        if self.selection_mask_dirty {
            let mask = self.selection.as_ref().map(|s| s.mask_r8());
            canvas.set_selection_mask(device, queue, mask.as_deref(), width, height);
            self.selection_mask_dirty = false;
        }
        if self.layers_reload {
            let pixels: Vec<&[u8]> = self
                .workspace
                .active()
                .layers
                .iter()
                .map(|l| l.pixels.as_slice())
                .collect();
            canvas.upload_all_layers(device, queue, &pixels, width, height);
            self.layers_reload = false;
            self.dirty_tiles.clear();
            self.dirty_layer = None;
        } else if let Some(layer) = self.dirty_layer
            && !self.dirty_tiles.is_empty()
        {
            if let Some(pixels) = self.workspace.active_mut().layers.get(layer).map(|l| l.pixels.as_slice()) {
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
        if self.floating.is_none() {
            canvas.clear_floating();
            self.floating_reload = false;
        } else if self.floating_reload
            && let Some(floating) = self.floating.as_ref()
        {
            canvas.set_floating(
                device,
                queue,
                &floating.pixels,
                floating.bw,
                floating.bh,
            );
            self.floating_reload = false;
        }
    }

    fn canvas_present(&self) -> Option<(Rect, CanvasView, CanvasOverlay)> {
        let view = self.canvas_view()?;
        let mut overlay = CanvasOverlay {
            dim_outside: self.dim_outside_selection && self.selection.is_some(),
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
        if self.tool == Tool::Brush {
            let radius = (self.brush_size * self.zoom * 0.5).max(2.5);
            let expanded = Rect {
                min: self.canvas_rect.min - Vec2::splat(radius),
                max: self.canvas_rect.max + Vec2::splat(radius),
            };
            if expanded.contains(self.input.mouse_pos) {
                overlay.brush_center = Some(self.input.mouse_pos);
                overlay.brush_radius = radius;
            }
        }
        if let (Some(anchor), Some(current)) = (self.selection_anchor, self.selection_current) {
            let rect = PixelRect::from_points(anchor, current, view.width, view.height);
            if !rect.is_empty() {
                overlay.selection_rect = Some(view.pixel_corners(rect));
            }
        }
        if let Some(floating) = self.floating.as_ref() {
            let half = floating.local_size() * 0.5;
            overlay.floating = Some(FloatingDraw {
                center: floating.center,
                half,
                rotation: floating.rotation,
                opacity: floating.opacity,
            });
            let corners = floating.corners().map(|p| view.image_to_screen(p));
            overlay.transform_corners = Some(corners);
            overlay.transform_rotate = Some(view.image_to_screen(floating.rotate_handle()));
        }
        Some((self.canvas_rect, view, overlay))
    }
}

fn paste_image_from_clipboard() -> Result<(Vec<u8>, u32, u32), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("Cannot open clipboard: {e}"))?;
    let image = clipboard
        .get_image()
        .map_err(|_| "Clipboard does not contain an image".to_string())?;
    let width =
        u32::try_from(image.width).map_err(|_| "Clipboard image is too large".to_string())?;
    let height =
        u32::try_from(image.height).map_err(|_| "Clipboard image is too large".to_string())?;
    if width == 0 || height == 0 {
        return Err("Clipboard image is empty".into());
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| "Clipboard image is too large".to_string())?;
    if image.bytes.len() < expected {
        return Err("Clipboard image data is incomplete".into());
    }
    Ok((image.bytes[..expected].to_vec(), width, height))
}

pub fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project")
}

fn ensure_project_extension(path: PathBuf) -> PathBuf {
    ensure_extension(path, "mgimg")
}

fn ensure_png_extension(path: PathBuf) -> PathBuf {
    ensure_extension(path, "png")
}

fn ensure_extension(path: PathBuf, extension: &str) -> PathBuf {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
    {
        path
    } else {
        path.with_extension(extension)
    }
}

pub fn rect_size(rect: Rect) -> Vec2 {
    rect.max - rect.min
}

fn screen_to_image(point: Vec2, view: CanvasView) -> (f32, f32) {
    let p = view.screen_to_image(point);
    (p.x, p.y)
}

fn screen_to_pixel(point: Vec2, view: CanvasView) -> Option<(u32, u32)> {
    let (x, y) = screen_to_image(point, view);
    if x < 0.0 || y < 0.0 || x >= view.width as f32 || y >= view.height as f32 {
        return None;
    }
    Some((x.floor() as u32, y.floor() as u32))
}

fn screen_to_pixel_clamped(point: Vec2, view: CanvasView) -> Option<(u32, u32)> {
    let (x, y) = screen_to_image(point, view);
    Some((
        x.floor().clamp(0.0, view.width.saturating_sub(1) as f32) as u32,
        y.floor().clamp(0.0, view.height.saturating_sub(1) as f32) as u32,
    ))
}

fn brush_overlaps_image(point: Vec2, radius_screen: f32, view: CanvasView) -> bool {
    let image = view.screen_to_image(point);
    let radius_image = radius_screen / view.zoom.max(1e-6);
    image.x >= -radius_image
        && image.y >= -radius_image
        && image.x < view.width as f32 + radius_image
        && image.y < view.height as f32 + radius_image
}
