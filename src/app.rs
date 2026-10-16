use std::path::{Path, PathBuf};

use glam::Vec2;
use mega_ui::wgpu::DrawStats;
use mega_ui::{DockState, Rect, Theme, Ui};

use crate::canvas_gpu::{CanvasOverlay, CanvasRenderer, FloatingDraw, LayerDraw};
use crate::editing::{
    BrushStroke, FloatingTransform, PixelRect, SelectMode, Selection, Tool, TransformHit,
    apply_brush, brush_dirty_bounds, fill_bucket, flood_mask,
};
use crate::history::{EditCapture, History, HistoryChange};
use crate::host::{FrameInput, Scene};
use crate::project::Project;
use crate::tiles::DirtyTiles;
use crate::ui;
use crate::ui::ai_dialog::{AiMode, AiSource};
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
    pub viewport_size: Vec2,
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
    pub ai: ui::ai_dialog::AiDialog,
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
            viewport_size: Vec2::ZERO,
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
            ai: ui::ai_dialog::AiDialog::default(),
        }
    }
}

impl EditorApp {
    pub fn open_ai(&mut self) {
        self.open_ai_mode(AiMode::Edit);
    }

    pub fn open_ai_draw(&mut self) {
        self.open_ai_mode(AiMode::Draw);
    }

    pub fn open_ai_redraw(&mut self) {
        self.open_ai_mode(AiMode::Redraw);
    }

    pub fn open_ai_inpaint(&mut self) {
        self.open_ai_mode(AiMode::Inpaint);
    }

    fn open_ai_mode(&mut self, mode: AiMode) {
        if (mode.needs_input() && self.image_size.is_none()) || self.ai.job.is_some() { return; }
        self.commit_transform();
        self.ai.selection_bounds = self.selection.as_ref().and_then(Selection::bounds);
        self.workspace.clear_asset_drag();
        self.layer_drag = None;
        self.ai.open = true;
        self.ai.mode = mode;
        if mode == AiMode::Inpaint && self.ai.source == AiSource::Selection { self.ai.source = AiSource::Layer; }
        self.ai.catalog_load = None;
        if mode.uses_sd() { self.ai.catalog = Default::default(); }
        self.ai.close_when_stopped = false;
        self.ai.preview = None;
        self.ai.result = None;
        self.ai.input_region = None;
        self.ai.preview_dirty = false;
        self.ai.fraction = None;
        match crate::comfyui::HostConfig::from_env() {
            Ok(config) => {
                if mode.uses_sd() {
                    self.ai.catalog_load = Some(crate::comfyui::CatalogLoad::start(config.clone(), false));
                    self.ai.catalog_message = "Loading models / LoRAs...".into();
                }
                self.ai.config = Some(config); self.ai.message.clear();
            }
            Err(error) => { self.ai.config = None; self.ai.message = error; }
        }
    }

    pub fn start_ai(&mut self) {
        if self.ai.job.is_some() { return; }
        let Some(config) = self.ai.config.clone() else { return; };
        if self.ai.mode == AiMode::Draw {
            if let Err(error) = self.ai.draw_settings.validate() {
                self.ai.message = error;
                return;
            }
            self.ai.document_id = self.workspace.active_doc().id.clone();
            let (width, height) = if self.project().width == 0 || self.project().height == 0 {
                (self.ai.draw_settings.width as u32, self.ai.draw_settings.height as u32)
            } else { (self.project().width, self.project().height) };
            self.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: width, max_y: height });
            self.ai.message = "Preparing workflow...".into();
            self.ai.preview = None;
            self.ai.result = None;
            self.ai.preview_dirty = false;
            self.ai.fraction = None;
            self.ai.close_when_stopped = false;
            self.ai.job = Some(crate::comfyui::Job::start_draw(config, self.ai.draw_settings.clone()));
            return;
        }
        if matches!(self.ai.mode, AiMode::Redraw | AiMode::Inpaint) {
            if let Err(error) = self.ai.draw_settings.validate_redraw(self.ai.denoise) {
                self.ai.message = error;
                return;
            }
        }
        let mask = if self.ai.mode == AiMode::Inpaint {
            match self.ai_mask() {
                Ok(mask) => Some(mask),
                Err(error) => { self.ai.message = error; return; }
            }
        } else { None };
        let (image, input_region) = match self.ai_input() {
            Ok(input) => input,
            Err(error) => { self.ai.message = error; return; }
        };
        self.ai.document_id = self.workspace.active_doc().id.clone();
        self.ai.input_region = Some(input_region);
        self.ai.message = "Preparing input…".into();
        self.ai.preview = None;
        self.ai.result = None;
        self.ai.preview_dirty = false;
        self.ai.fraction = None;
        self.ai.close_when_stopped = false;
        self.ai.job = Some(if let Some(mask) = mask {
            crate::comfyui::Job::start_inpaint(config, image, mask, self.ai.scale, self.ai.draw_settings.clone(), self.ai.denoise)
        } else if self.ai.mode == AiMode::Redraw {
            crate::comfyui::Job::start_redraw(config, image, self.ai.scale, self.ai.draw_settings.clone(), self.ai.denoise)
        } else {
            crate::comfyui::Job::start(config, image, self.ai.scale, self.ai.prompt.clone())
        });
    }

    fn ai_mask(&self) -> Result<image::RgbaImage, String> {
        if self.ai.source == AiSource::Selection { return Err("Inpaint source must be a layer or the whole document".into()); }
        let p = self.project();
        let selection = self.selection.as_ref().ok_or("Select an area to use as the inpaint mask")?;
        if selection.width != p.width || selection.height != p.height {
            return Err("Selection does not match the current canvas".into());
        }
        if selection.is_empty() { return Err("Select an area to use as the inpaint mask".into()); }
        crate::comfyui::scaled_size(p.width, p.height, self.ai.scale)?;
        Ok(image::RgbaImage::from_fn(p.width, p.height, |x, y| {
            let value = if selection.contains(x, y) { 255 } else { 0 };
            image::Rgba([value, value, value, 255])
        }))
    }

    /// Crop before worker-side scaling; retain document coordinates for Accept / Fit.
    fn ai_input(&self) -> Result<(image::RgbaImage, PixelRect), String> {
        let p = self.project();
        let mut region = PixelRect { min_x: 0, min_y: 0, max_x: p.width, max_y: p.height };
        let selection = if self.ai.source == AiSource::Selection {
            let selection = self.selection.as_ref().ok_or("Select an area on the canvas first")?;
            if selection.width != p.width || selection.height != p.height {
                return Err("Selection does not match the current canvas".into());
            }
            region = selection.bounds().ok_or("Select an area on the canvas first")?;
            Some(selection)
        } else { None };
        let width = region.max_x - region.min_x;
        let height = region.max_y - region.min_y;
        crate::comfyui::scaled_size(width, height, self.ai.scale)?;
        let pixels = match self.ai.source {
            AiSource::Layer => p.active_pixels().ok_or("Choose a raster layer first")?.to_vec(),
            AiSource::Document | AiSource::Selection => crate::editing::composite_layers(&p.layers, p.width, p.height),
        };
        let image = if let Some(selection) = selection {
            let mut cropped = image::RgbaImage::new(width, height);
            for y in region.min_y..region.max_y {
                for x in region.min_x..region.max_x {
                    if selection.contains(x, y) {
                        let index = ((y as usize * p.width as usize) + x as usize) * 4;
                        cropped.put_pixel(x - region.min_x, y - region.min_y,
                            image::Rgba(pixels[index..index + 4].try_into().unwrap()));
                    }
                }
            }
            cropped
        } else {
            image::RgbaImage::from_raw(width, height, pixels).ok_or("Invalid input raster")?
        };
        Ok((image, region))
    }

    pub fn cancel_ai(&mut self) {
        if let Some(job) = &self.ai.job {
            job.cancel();
            self.ai.message = "Cancelling…".into();
        }
    }

    pub fn close_ai(&mut self) {
        if self.ai.job.is_some() {
            self.ai.close_when_stopped = true;
            self.cancel_ai();
        } else {
            self.ai.open = false;
            self.ai.result = None;
            self.ai.input_region = None;
            self.ai.preview = None;
            self.ai.preview_dirty = false;
            self.ai.fraction = None;
        }
    }

    pub fn accept_ai(&mut self) {
        if self.ai.job.is_some() || self.ai.result.is_none() { return; }
        if self.workspace.active_doc().id != self.ai.document_id {
            self.ai.message = "Destination document changed; result was not inserted".into();
            return;
        }
        let image = self.ai.result.as_ref().unwrap();
        let Some(region) = self.ai.input_region.filter(|r| !r.is_empty()) else {
            self.ai.message = "Missing source bounds; regenerate the image".into();
            return;
        };
        let size = if self.ai.fit_image { (region.max_x - region.min_x, region.max_y - region.min_y) } else { image.dimensions() };
        let action = self.ai.mode.layer_prefix();
        let name = format!("{action} {}", self.workspace.active().layers.len() + 1);
        let project = self.workspace.active_mut();
        let insertion = if self.ai.mode == AiMode::Draw && (project.width == 0 || project.height == 0) {
            project.import_rgba(image.as_raw(), image.width(), image.height()).map(|()| {
                project.layers[0].name = name;
                project.sync_manifest_pub();
            })
        } else {
            project.add_raster_layer(image, name, region, self.ai.fit_image)
        };
        match insertion {
            Ok(()) => {
                let _ = self.refresh_image();
                self.workspace.mark_active_thumb_dirty();
                self.dirty = true;
                self.status = format!("{action} added: {} × {} px", size.0, size.1);
                self.close_ai();
            }
            Err(error) => self.ai.message = error,
        }
    }

    fn poll_ai(&mut self, ui: &mut Ui) {
        let Some(job) = self.ai.job.as_ref() else { return; };
        if let Ok(mut progress) = job.progress.try_lock() {
            if !job.is_cancelling() { self.ai.message = progress.message.clone(); }
            self.ai.fraction = progress.fraction;
            if let Some(preview) = progress.preview.take() {
                self.ai.preview = Some(preview);
                self.ai.preview_dirty = true;
            }
        }
        let Some(result) = job.poll() else { return; };
        self.ai.job = None;
        match result {
            crate::comfyui::JobResult::Done(image) => {
                if self.workspace.active_doc().id != self.ai.document_id {
                    self.ai.message = "Destination document changed; result was not inserted".into();
                    return;
                }
                self.ai.message = format!("Result: {} × {} px — accept or regenerate", image.width(), image.height());
                self.ai.preview = Some(image::imageops::thumbnail(&image, 1024, 1024));
                self.ai.preview_dirty = true;
                self.ai.fraction = None;
                self.ai.result = Some(image);
            }
            crate::comfyui::JobResult::Failed(error) => {
                self.ai.message = error.clone();
                self.status = error.clone();
                ui.notify_error(&error);
            }
            crate::comfyui::JobResult::Cancelled => {
                self.ai.message = "Cancelled".into();
                self.ai.fraction = None;
                if self.ai.close_when_stopped { self.close_ai(); }
            }
        }
    }

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

    pub fn import_image(&mut self, ui: &mut Ui) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Images", &["png", "webp"])
            .pick_file()
        else {
            return;
        };
        match self.workspace.import_asset_image(&path) {
            Ok(_) => {
                self.dirty = true;
                self.status = format!("Imported asset {}", file_name(&path));
                ui.notify_success("Image imported as asset (not placed on document)");
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

    pub fn export_image(&mut self, ui: &mut Ui, webp: bool) {
        let extension = if webp { "webp" } else { "png" };
        let Some(path) = rfd::FileDialog::new()
            .add_filter(if webp { "WebP image" } else { "PNG image" }, &[extension])
            .set_file_name(format!("export.{extension}"))
            .save_file()
            .map(|p| ensure_extension(p, extension))
        else {
            return;
        };
        let format = if webp { image::ImageFormat::WebP } else { image::ImageFormat::Png };
        match self.workspace.active_mut().export_image(&path, format) {
            Ok(()) => {
                self.status = format!("Exported {}", file_name(&path));
                ui.notify_success("Image exported");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn add_layer(&mut self) {
        if self.image_size.is_none() {
            return;
        }
        self.workspace.active_mut().add_layer();
        self.history.clear_redo();
        self.dirty = true;
        self.status = "Layer added".into();
        self.layers_reload = true;
        self.workspace.mark_active_thumb_dirty();
    }

    pub fn delete_layer(&mut self, index: usize) {
        if index >= self.project().layers.len() { return; }
        self.cancel_transform();
        if !self.history.delete_layer(self.workspace.active_mut(), index) { return; }
        self.layer_drag = None;
        self.sync_layer_structure();
        self.status = if self.project().layers.is_empty() { "Last layer deleted" } else { "Layer deleted" }.into();
    }

    fn sync_layer_structure(&mut self) {
        let size = if self.project().layers.is_empty() { None } else { Some((self.project().width, self.project().height)) };
        if size != self.image_size {
            if let Some((width, height)) = size {
                self.resize_w = width;
                self.resize_h = height;
                self.stroke_marks = vec![0; width as usize * height as usize];
                self.dirty_tiles.resize(width, height);
            } else { self.stroke_marks.clear(); }
            self.clear_selection();
        }
        self.image_size = size;
        self.dirty_tiles.clear();
        self.dirty_layer = None;
        self.layers_reload = size.is_some();
        self.canvas_cleared = size.is_none();
        self.floating_reload = false;
        self.dirty = true;
        self.workspace.mark_active_thumb_dirty();
    }

    pub fn move_layer_to(&mut self, from: usize, to: usize) {
        if from == to || from >= self.project().layers.len() || to >= self.project().layers.len() { return; }
        // History addresses layers by index; an unrecorded reorder invalidates those indices.
        self.history.clear();
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
        if let Some(change) = self.history.undo(self.workspace.active_mut()) {
            self.apply_history_change(change);
            self.status = "Undo".into();
        }
    }

    pub fn redo(&mut self) {
        self.cancel_transform();
        if let Some(change) = self.history.redo(self.workspace.active_mut()) {
            self.apply_history_change(change);
            self.status = "Redo".into();
        }
    }

    fn apply_history_change(&mut self, change: HistoryChange) {
        match change {
            HistoryChange::Pixels { layer, rect } => {
                self.dirty = true;
                self.mark_layer_dirty(layer, rect);
                self.workspace.mark_active_thumb_dirty();
            }
            HistoryChange::Layers => self.sync_layer_structure(),
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
        let Some(pixels) = self.workspace.active().layers.get(layer).map(|l| l.pixels.as_slice()) else {
            return;
        };
        let Some(floating) = FloatingTransform::extract(pixels, width, height, &selection, layer, opacity)
        else {
            return;
        };
        self.floating = Some(floating);
        self.floating_reload = true;
        self.clear_selection();
        self.dirty = true;
        self.status = "Transform — drag / handles · Enter commit · Esc cancel".into();
    }

    pub fn cancel_transform(&mut self) {
        if self.floating.take().is_none() {
            self.transform_drag = None;
            self.transform_last = None;
            return;
        }
        self.stroke_capture = None;
        self.transform_drag = None;
        self.transform_last = None;
        self.floating_reload = false;
        self.status = "Transform cancelled".into();
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
        let mut capture = EditCapture::new(layer, width, height);
        let source = PixelRect {
            min_x: floating.origin_x,
            min_y: floating.origin_y,
            max_x: (floating.origin_x + floating.bw).min(width),
            max_y: (floating.origin_y + floating.bh).min(height),
        };
        // Snapshot source and destination before stamping.
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
        if let Some(pixels) = self.workspace.active().layers.get(layer).map(|l| l.pixels.as_slice()) {
            capture.touch_rect(pixels, source);
            capture.touch_rect(pixels, dest);
        }
        let Some(pixels) = self.workspace.active_mut().layers.get_mut(layer).map(|l| &mut l.pixels) else {
            return;
        };
        floating.clear_source(pixels, width, height);
        let (dirty, selection) = floating.stamp(pixels, width, height);
        if let Some(edit) = capture.finish(pixels) {
            self.history.push(edit);
        }
        self.set_selection(selection);
        self.transform_drag = None;
        self.transform_last = None;
        self.floating_reload = false;
        self.dirty = true;
        self.workspace.mark_active_thumb_dirty();
        self.status = "Transform applied".into();
        self.mark_layer_dirty(layer, source);
        if let Some(rect) = dirty {
            self.mark_layer_dirty(layer, rect);
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
    fn request_close(&mut self) -> bool {
        if self.ai.job.is_some() { self.cancel_ai(); false } else { true }
    }

    fn title() -> &'static str {
        "Mega Image Editor"
    }

    fn init(ui: &mut Ui) {
        ui.set_theme(Theme::dark());
        ui.load_builtin_icons();
    }

    fn begin_frame(&mut self, input: &FrameInput) {
        self.input = if self.ai.open {
            let mut blocked = FrameInput::default();
            blocked.key_escape = input.key_escape;
            blocked
        } else { input.clone() };
    }

    fn build(&mut self, ui: &mut Ui, viewport: Vec2, _stats: DrawStats) -> bool {
        self.viewport_size = viewport;
        self.poll_ai(ui);
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
        self.ai.open
            || self.selection.is_some()
            || self.selection_anchor.is_some()
            || self.floating.is_some()
            || self.workspace.asset_drag.is_some()
            || thumb_waiting
            || (self.tool == Tool::Brush && self.canvas_rect.contains(self.input.mouse_pos))
            || !self.dirty_tiles.is_empty()
    }

    fn sync_ui_textures(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, renderer: &mut mega_ui::wgpu::UiRenderer) {
        if self.ai.preview_dirty {
            if let Some(preview) = &self.ai.preview {
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
            canvas.clear_transform();
            self.floating_reload = false;
        } else if let Some(floating) = self.floating.as_ref() {
            let ready = canvas.sync_transform(
                device,
                queue,
                floating.layer,
                &floating.mask,
                floating.bw,
                floating.bh,
                self.floating_reload,
            );
            if ready {
                self.floating_reload = false;
            }
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
            overlay.floating = Some(FloatingDraw {
                opacity: floating.opacity,
                layer: floating.layer,
                origin: Vec2::new(floating.origin_x as f32, floating.origin_y as f32),
                size: Vec2::new(floating.bw as f32, floating.bh as f32),
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

#[cfg(test)]
mod ai_tests {
    use super::*;

    #[test]
    fn inpaint_requires_nonempty_matching_selection_and_full_image_source() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[1, 2, 3, 255].repeat(8), 4, 2).unwrap();
        app.ai.mode = AiMode::Inpaint;
        assert!(app.ai_mask().is_err());
        app.selection = Some(Selection::from_rect(PixelRect { min_x: 0, min_y: 0, max_x: 0, max_y: 0 }, 4, 2));
        assert!(app.ai_mask().is_err());
        app.selection = Some(Selection::all(5, 2));
        assert!(app.ai_mask().is_err());
        app.selection = Some(Selection::all(4, 2));
        assert!(app.ai_mask().unwrap().pixels().all(|p| p.0 == [255, 255, 255, 255]));
        app.ai.source = AiSource::Selection;
        assert!(app.ai_mask().is_err());
    }

    #[test]
    fn selection_source_crops_masks_and_preserves_alpha_for_edit_and_redraw() {
        for mode in [AiMode::Edit, AiMode::Redraw] {
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&[20, 40, 60, 128].repeat(6 * 5), 6, 5).unwrap();
            app.project_mut().add_layer();
            app.project_mut().layers[1].pixels = [255, 0, 0, 255].repeat(6 * 5);
            app.project_mut().layers[1].visible = false;
            app.ai.mode = mode;
            app.ai.source = AiSource::Selection;
            assert!(app.ai_input().is_err());
            app.selection = Some(Selection::from_rect(PixelRect { min_x: 0, min_y: 0, max_x: 0, max_y: 0 }, 6, 5));
            assert!(app.ai_input().is_err());
            let bounds = PixelRect { min_x: 1, min_y: 1, max_x: 5, max_y: 4 };
            let mut selection = Selection::from_rect(bounds, 6, 5);
            selection.subtract(&Selection::from_rect(PixelRect { min_x: 2, min_y: 2, max_x: 4, max_y: 3 }, 6, 5));
            app.selection = Some(selection);
            app.ai.scale = 2.0;
            let (image, region) = app.ai_input().unwrap();
            assert_eq!(region, bounds);
            assert_eq!(image.dimensions(), (4, 3));
            for y in 0..3 {
                for x in 0..4 {
                    assert_eq!(image.get_pixel(x, y).0, if y == 1 && (1..3).contains(&x) { [0, 0, 0, 0] } else { [20, 40, 60, 128] });
                }
            }
            assert_eq!(crate::comfyui::scaled_size(image.width(), image.height(), app.ai.scale).unwrap(), (8, 6));
            // A plain rectangle retains the complete cropped image.
            app.selection = Some(Selection::from_rect(bounds, 6, 5));
            let (image, _) = app.ai_input().unwrap();
            assert!(image.pixels().all(|p| p.0 == [20, 40, 60, 128]));
            app.selection = Some(Selection::all(7, 5));
            assert!(app.ai_input().is_err());
        }
    }

    #[test]
    fn redraw_uploads_selected_source_and_accepts_as_new_layer() {
        use std::{io::{BufRead, BufReader, Read, Write}, net::TcpListener, time::{Duration, Instant}};
        for (mode, source) in [(AiMode::Redraw, AiSource::Layer), (AiMode::Redraw, AiSource::Document),
            (AiMode::Redraw, AiSource::Selection), (AiMode::Inpaint, AiSource::Layer), (AiMode::Inpaint, AiSource::Document)] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let host = format!("http://{}", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let mut routes = vec![("/upload/image", false)];
                if mode == AiMode::Inpaint { routes.push(("/upload/image", true)); }
                routes.extend([("/prompt", false), ("/history/job", false), ("/view?", false)]);
                for (route, is_mask) in routes {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut reader = BufReader::new(&mut stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    assert!(line.contains(route), "{line}");
                    let mut length = 0;
                    loop {
                        line.clear();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" { break; }
                        if let Some((key, value)) = line.split_once(':') {
                            if key.eq_ignore_ascii_case("content-length") { length = value.trim().parse::<usize>().unwrap(); }
                        }
                    }
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    let response = match route {
                        "/upload/image" => {
                            let multipart = String::from_utf8_lossy(&body);
                            assert!(multipart.contains(if is_mask { "filename=\"mega-image-editor-mask.png\"" } else { "filename=\"mega-image-editor-input.png\"" }));
                            assert!(multipart.contains("name=\"overwrite\"\r\n\r\ntrue\r\n"));
                            let start = body.windows(8).position(|w| w == b"\x89PNG\r\n\x1a\n").unwrap();
                            let image = image::load_from_memory_with_format(&body[start..], image::ImageFormat::Png).unwrap().to_rgba8();
                            assert_eq!(image.dimensions(), if source == AiSource::Selection { (48, 40) } else { (64, 64) });
                            if is_mask {
                                assert_eq!(image.get_pixel(0, 0).0, [0, 0, 0, 255]);
                                assert_eq!(image.get_pixel(10, 14).0, [255, 255, 255, 255]);
                                assert_eq!(image.get_pixel(32, 30).0, [0, 0, 0, 255]);
                                assert!(image.pixels().all(|p| p.0 == [0, 0, 0, 255] || p.0 == [255, 255, 255, 255]));
                            } else {
                                assert_eq!(image.get_pixel(0, 0).0, if source == AiSource::Layer { [200, 100, 50, 255] } else { [10, 20, 30, 255] });
                            }
                            if source == AiSource::Selection { assert_eq!(image.get_pixel(24, 20).0, [0, 0, 0, 0]); }
                            if is_mask { br#"{"name":"mask.png","subfolder":"test"}"#.to_vec() }
                            else { br#"{"name":"source.png","subfolder":"test"}"#.to_vec() }
                        }
                        "/prompt" => {
                            let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                            let graph = request["prompt"].as_object().unwrap();
                            let image = graph.values().find(|n| n["_meta"]["title"] == "Image").unwrap();
                            assert_eq!(image["inputs"]["image"], "test/source.png");
                            let sampler = graph.values().find(|n| n["_meta"]["title"] == "Sampler").unwrap();
                            assert_eq!(sampler["inputs"]["denoise"], 0.5);
                            assert!(graph.values().any(|n| n["class_type"] == "VAEEncode"));
                            if mode == AiMode::Inpaint {
                                let mask = graph.values().find(|n| n["_meta"]["title"] == "Mask").unwrap();
                                assert_eq!(mask["inputs"]["image"], "test/mask.png");
                                assert_eq!(mask["inputs"]["channel"], "red");
                                assert!(graph.values().any(|n| n["class_type"] == "SetLatentNoiseMask"));
                            }
                            br#"{"prompt_id":"job"}"#.to_vec()
                        }
                        "/history/job" => br#"{"job":{"outputs":{"12":{"images":[{"filename":"result.png"}]}},"status":{"completed":true}}}"#.to_vec(),
                        _ => crate::project::encode_png_rgba(&[1, 2, 3, 255].repeat(64 * 64), 64, 64).unwrap(),
                    };
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).unwrap();
                    stream.write_all(&response).unwrap();
                }
            });
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&[10, 20, 30, 255].repeat(128 * 128), 128, 128).unwrap();
            app.project_mut().add_layer();
            app.project_mut().layers[1].pixels = [200, 100, 50, 255].repeat(128 * 128);
            // Active layer source ignores visibility; composite source honors it.
            app.project_mut().layers[1].visible = false;
            app.refresh_image().unwrap();
            app.ai.open = true;
            app.ai.mode = mode;
            app.ai.source = source;
            if source == AiSource::Selection || mode == AiMode::Inpaint {
                let mut selection = Selection::from_rect(PixelRect { min_x: 16, min_y: 24, max_x: 112, max_y: 104 }, 128, 128);
                selection.subtract(&Selection::from_rect(PixelRect { min_x: 48, min_y: 48, max_x: 80, max_y: 72 }, 128, 128));
                app.ai.selection_bounds = selection.bounds();
                app.selection = Some(selection);
            }
            app.ai.scale = 0.5;
            app.ai.denoise = 0.5;
            app.ai.config = Some(crate::comfyui::HostConfig { host, ws: String::new(), login: String::new(), password: String::new() });
            app.start_ai();
            let started = Instant::now();
            let mut ui = Ui::new();
            while app.ai.job.is_some() {
                assert!(started.elapsed() < Duration::from_secs(10), "Redraw timed out");
                app.poll_ai(&mut ui);
                std::thread::sleep(Duration::from_millis(10));
            }
            server.join().unwrap();
            assert!(app.ai.result.is_some(), "{}", app.ai.message);
            assert_eq!(app.project().layers.len(), 2);
            assert_eq!(&app.project().layers[1].pixels[..4], &[200, 100, 50, 255]);
            assert_eq!(app.ai.input_region.unwrap().max_x, if source == AiSource::Selection { 112 } else { 128 });
            // Accept must retain the submitted region even if selection changes afterwards.
            app.selection = Some(Selection::all(128, 128));
            // Render the Redraw modal with its source, denoise and cached selectors.
            EditorApp::init(&mut ui);
            let viewport = Vec2::new(1280.0, 800.0);
            ui.begin_frame(mega_ui::UiInput { viewport, ..Default::default() });
            app.build(&mut ui, viewport, DrawStats::default());
            ui.end_frame();
            app.accept_ai();
            assert_eq!(app.image_size, Some((128, 128)));
            assert_eq!(app.project().layers.len(), 3);
            assert_eq!(app.project().layers[2].name, format!("{} 3", mode.layer_prefix()));
            let inserted = &app.project().layers[2].pixels;
            if source == AiSource::Selection {
                for y in 0..128 {
                    for x in 0..128 {
                        let index = (y * 128 + x) * 4;
                        assert_eq!(&inserted[index..index + 4], if (16..112).contains(&x) && (24..104).contains(&y) { &[1, 2, 3, 255] } else { &[0, 0, 0, 0] });
                    }
                }
            } else { assert_eq!(&inserted[..4], &[1, 2, 3, 255]); }
            assert!(!app.ai.open);
        }
    }

    #[test]
    fn draw_on_empty_document_waits_for_acceptance_then_creates_canvas() {
        let mut app = EditorApp::default();
        app.ai.open = true;
        app.ai.mode = AiMode::Draw;
        assert!(app.project().layers.is_empty());
        app.ai.document_id = app.workspace.active_doc().id.clone();
        app.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: 64, max_y: 128 });
        let image = image::RgbaImage::from_pixel(64, 128, image::Rgba([1, 2, 3, 255]));
        app.ai.job = Some(crate::comfyui::Job::with_result(crate::comfyui::JobResult::Done(image)));
        let mut ui = Ui::new();
        app.poll_ai(&mut ui);
        assert!(app.project().layers.is_empty());
        EditorApp::init(&mut ui);
        let viewport = Vec2::new(1280.0, 800.0);
        ui.begin_frame(mega_ui::UiInput { viewport, ..Default::default() });
        app.build(&mut ui, viewport, DrawStats::default());
        let output = ui.end_frame();
        assert!(output.draw_list.iter().any(|c| c.kind == 1.0 && c.tex == crate::host::AI_PREVIEW_TEXTURE));
        app.accept_ai();
        assert_eq!(app.image_size, Some((64, 128)));
        assert_eq!(app.project().layers.len(), 1);
        assert_eq!(app.project().layers[0].name, "AI Draw 1");
        assert_eq!(&app.project().layers[0].pixels[..4], &[1, 2, 3, 255]);
        assert!(!app.ai.open);
        app.accept_ai();
        assert_eq!(app.project().layers.len(), 1);
    }

    #[test]
    fn completed_job_waits_for_acceptance_and_adds_exactly_once() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[20, 40, 60, 255], 1, 1).unwrap();
        app.refresh_image().unwrap();
        app.ai.open = true;
        app.ai.document_id = app.workspace.active_doc().id.clone();
        app.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: 1, max_y: 1 });
        let result = image::RgbaImage::from_pixel(2, 3, image::Rgba([100, 120, 140, 255]));
        app.ai.job = Some(crate::comfyui::Job::with_result(crate::comfyui::JobResult::Done(result.clone())));
        let mut ui = Ui::new();
        app.poll_ai(&mut ui);
        assert!(app.ai.open);
        assert!(app.ai.job.is_none());
        assert_eq!(app.image_size, Some((1, 1)));
        assert_eq!(app.project().layers.len(), 1);
        assert_eq!(app.ai.result.as_ref(), Some(&result));
        assert!(app.ai.preview_dirty);
        assert!(!app.dirty);

        app.accept_ai();
        assert!(!app.ai.open);
        assert!(app.ai.result.is_none());
        assert_eq!(app.image_size, Some((1, 1)));
        assert_eq!(app.project().active_layer, 1);
        assert_eq!(app.project().layers[1].pixels, [100, 120, 140, 255]);
        assert_eq!(&app.project().layers[0].pixels[..4], &[20, 40, 60, 255]);
        assert!(app.dirty);
        app.accept_ai();
        assert_eq!(app.project().layers.len(), 2);
    }

    #[test]
    fn closing_unaccepted_result_does_not_change_document() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[20, 40, 60, 255], 1, 1).unwrap();
        app.refresh_image().unwrap();
        app.ai.open = true;
        app.ai.result = Some(image::RgbaImage::new(2, 3));
        app.close_ai();
        assert!(!app.ai.open);
        assert!(app.ai.result.is_none());
        assert_eq!(app.image_size, Some((1, 1)));
        assert_eq!(app.project().layers.len(), 1);
        assert!(!app.dirty);
    }

    #[test]
    fn fit_uses_unscaled_input_bounds_and_can_be_disabled() {
        for fit in [true, false] {
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&vec![20, 40, 60, 255].repeat(8), 4, 2).unwrap();
            app.refresh_image().unwrap();
            app.ai.open = true;
            app.ai.document_id = app.workspace.active_doc().id.clone();
            app.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: 4, max_y: 2 });
            app.ai.scale = 0.5;
            app.ai.fit_image = fit;
            app.ai.result = Some(image::RgbaImage::from_pixel(2, 1, image::Rgba([100, 120, 140, 255])));
            app.accept_ai();
            assert_eq!(app.image_size, Some((4, 2)));
            assert_eq!(app.project().layers.len(), 2);
            let inserted = &app.project().layers[1].pixels;
            if fit {
                assert_eq!(*inserted, vec![100, 120, 140, 255].repeat(8));
            } else {
                assert_eq!(&inserted[..8], &[100, 120, 140, 255, 100, 120, 140, 255]);
                assert!(inserted[8..].iter().all(|b| *b == 0));
            }
        }
    }

    #[test]
    fn only_latest_result_is_accepted_and_destination_is_checked_again() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[20, 40, 60, 255], 1, 1).unwrap();
        app.refresh_image().unwrap();
        app.ai.open = true;
        app.ai.document_id = app.workspace.active_doc().id.clone();
        app.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: 1, max_y: 1 });
        let mut ui = Ui::new();
        for color in [10, 80] {
            let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([color, 0, 0, 255]));
            app.ai.job = Some(crate::comfyui::Job::with_result(crate::comfyui::JobResult::Done(image)));
            app.poll_ai(&mut ui);
        }
        assert_eq!(app.project().layers.len(), 1);
        app.workspace.create_document("Other document");
        app.accept_ai();
        assert!(app.project().layers.is_empty());
        assert!(app.ai.result.is_some());
        assert!(app.ai.open);
        app.workspace.set_active_document(0);
        app.accept_ai();
        assert_eq!(app.project().layers[1].pixels, [80, 0, 0, 255]);
    }

    #[test]
    fn modal_blocks_editor_input_and_composes_preview_above_canvas() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[20, 40, 60, 255], 1, 1).unwrap();
        app.refresh_image().unwrap();
        app.ai.open = true;
        app.ai.preview = Some(image::RgbaImage::from_pixel(10, 10, image::Rgba([1, 2, 3, 255])));
        app.ai.fraction = Some(0.4);
        let mut input = FrameInput::default();
        input.mouse_down = true;
        input.key_undo = true;
        input.key_paste = true;
        input.mouse_pos = Vec2::new(600.0, 400.0);
        app.begin_frame(&input);
        assert!(!app.input.mouse_down && !app.input.key_undo && !app.input.key_paste);

        let mut ui = Ui::new();
        EditorApp::init(&mut ui);
        let viewport = Vec2::new(1280.0, 800.0);
        ui.begin_frame(mega_ui::UiInput { viewport, ..Default::default() });
        assert!(app.build(&mut ui, viewport, DrawStats::default()));
        let output = ui.end_frame();
        let canvas = output.draw_list.iter().position(|c| c.kind == 1.0 && c.tex == crate::host::CANVAS_TEXTURE).unwrap();
        let preview = output.draw_list.iter().position(|c| c.kind == 1.0 && c.tex == crate::host::AI_PREVIEW_TEXTURE).unwrap();
        assert!(preview > canvas);
        assert!(ui.input_debug().modal_open);
        assert_eq!(app.project().layers[0].pixels, [20, 40, 60, 255]);
    }
}

#[cfg(test)]
mod layer_history_tests {
    use super::*;

    fn app_with_layers(count: usize) -> EditorApp {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[1, 2, 3, 255].repeat(8), 4, 2).unwrap();
        for _ in 1..count { app.project_mut().add_layer(); }
        for (i, layer) in app.project_mut().layers.iter_mut().enumerate() {
            layer.name = format!("Layer {i}");
            layer.visible = i % 2 == 0;
            layer.opacity = 0.25 * (i + 1) as f32;
            layer.pixels.fill((i + 1) as u8);
        }
        app.refresh_image().unwrap();
        app
    }

    #[test]
    fn deletion_restores_each_layers_order_metadata_pixels_and_active_layer() {
        for index in 0..3 {
            let mut app = app_with_layers(3);
            let before = app.project().layers.clone();
            for _ in 0..3 {
                app.delete_layer(index);
                assert_eq!(app.project().layers.len(), 2);
                assert_eq!(app.project().active_layer, 1);
                assert!(app.layers_reload);
                assert!(app.workspace.active_doc().thumb_dirty);
                app.undo();
                assert_eq!(app.project().active_layer, 2);
                assert_eq!(app.project().manifest.layers.len(), 3);
                for (restored, original) in app.project().layers.iter().zip(&before) {
                    assert_eq!(restored.name, original.name);
                    assert_eq!(restored.visible, original.visible);
                    assert_eq!(restored.opacity, original.opacity);
                    assert_eq!(restored.pixels, original.pixels);
                }
                app.redo();
                assert_eq!(app.project().layers.len(), 2);
                app.undo();
            }
        }
    }

    fn paint_pixel(app: &mut EditorApp, layer: usize, value: u8) {
        let p = app.project_mut();
        let mut capture = EditCapture::new(layer, p.width, p.height);
        capture.touch_rect(&p.layers[layer].pixels, PixelRect { min_x: 0, min_y: 0, max_x: 1, max_y: 1 });
        p.layers[layer].pixels[0] = value;
        let edit = capture.finish(&p.layers[layer].pixels).unwrap();
        app.history.push(edit);
    }

    #[test]
    fn last_layer_deletion_restores_canvas_and_keeps_pixel_history() {
        let mut app = app_with_layers(1);
        paint_pixel(&mut app, 0, 99);
        app.delete_layer(0);
        assert!(app.image_size.is_none());
        assert_eq!((app.project().width, app.project().height), (0, 0));
        app.undo();
        assert_eq!(app.image_size, Some((4, 2)));
        assert_eq!(app.stroke_marks.len(), 8);
        assert!(app.layers_reload);
        assert_eq!(app.project().layers[0].pixels[0], 99);
        app.undo();
        assert_eq!(app.project().layers[0].pixels[0], 1);
        app.redo();
        assert_eq!(app.project().layers[0].pixels[0], 99);
        app.redo();
        assert!(app.image_size.is_none());
        app.undo();
        assert_eq!(app.project().layers[0].pixels[0], 99);
    }

    #[test]
    fn edits_before_and_after_deletion_target_correct_layers_and_new_edit_clears_redo() {
        let mut app = app_with_layers(3);
        paint_pixel(&mut app, 2, 80);
        app.delete_layer(0);
        paint_pixel(&mut app, 1, 90);
        app.project_mut().active_layer = 0;
        app.undo();
        assert_eq!(app.project().layers[1].pixels[0], 80);
        assert_eq!(app.dirty_layer, Some(1));
        app.undo();
        assert_eq!(app.project().layers.len(), 3);
        app.undo();
        assert_eq!(app.project().layers[2].pixels[0], 3);
        app.redo();
        app.redo();
        app.redo();
        assert_eq!(app.project().layers[1].pixels[0], 90);
        app.undo();
        paint_pixel(&mut app, 1, 100);
        app.redo();
        assert_eq!(app.project().layers[1].pixels[0], 100);
        app.delete_layer(1);
        app.undo();
        app.add_layer();
        app.redo();
        assert_eq!(app.project().layers.len(), 3);
    }
}
