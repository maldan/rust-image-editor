use std::path::{Path, PathBuf};

use glam::Vec2;
use mega_ui::wgpu::DrawStats;
use mega_ui::{DockState, Rect, Theme, Ui};

use crate::canvas_gpu::{CanvasOverlay, CanvasRenderer, FloatingDraw, LayerDraw};
use crate::editing::{
    BrushStroke, FloatingTransform, PixelRect, SelectMode, Selection, SoftBrushCoverage, Tool, TransformHit, TransformCursor,
    apply_brush, brush_dirty_bounds, composite_pixel, fill_bucket, flood_mask,
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
    pub brush_hardness: f32,
    pub brush_color: [f32; 4],
    pub wand_threshold: f32,
    pub fill_threshold: f32,
    pub dim_outside_selection: bool,
    pub preview_asset: Option<String>,
    preview_asset_dirty: bool,
    preview_document_view: Option<(Vec2, f32, f32)>,
    pub selection_brush_erase: bool,
    lasso_points: Vec<Vec2>,
    selection_brush_last: Option<Vec2>,
    pub selection: Option<Selection>,
    pub selection_anchor: Option<(u32, u32)>,
    pub selection_current: Option<(u32, u32)>,
    selection_mode: SelectMode,
    selection_deselect_press: Option<Vec2>,
    selection_mask_dirty: bool,
    brush_last: Option<(f32, f32)>,
    brush_dirty: bool,
    stroke_marks: Vec<u32>,
    soft_brush_coverage: SoftBrushCoverage,
    stroke_id: u32,
    stroke_capture: Option<EditCapture>,
    history: History,
    floating: Option<FloatingTransform>,
    transform_drag: Option<TransformHit>,
    transform_last: Option<Vec2>,
    pub transform_keep_aspect_ratio: bool,
    pub canvas_cursor: Option<TransformCursor>,
    pub layer_drag: Option<usize>,
    canvas_cleared: bool,
    pub resize_w: u32,
    pub resize_h: u32,
    pub ai: ui::ai_dialog::AiDialog,
    pub hsv: ui::hsv_dialog::HsvDialog,
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
            brush_hardness: 1.0,
            brush_color: [0.0, 0.0, 0.0, 1.0],
            wand_threshold: 32.0,
            fill_threshold: 32.0,
            dim_outside_selection: true,
            preview_asset: None,
            preview_asset_dirty: false,
            preview_document_view: None,
            selection_brush_erase: false,
            lasso_points: Vec::new(),
            selection_brush_last: None,
            selection: None,
            selection_anchor: None,
            selection_current: None,
            selection_mode: SelectMode::Replace,
            selection_deselect_press: None,
            selection_mask_dirty: true,
            brush_last: None,
            brush_dirty: false,
            stroke_marks: Vec::new(),
            soft_brush_coverage: SoftBrushCoverage::default(),
            stroke_id: 1,
            stroke_capture: None,
            history: History::default(),
            floating: None,
            transform_drag: None,
            transform_last: None,
            transform_keep_aspect_ratio: true,
            canvas_cursor: None,
            layer_drag: None,
            canvas_cleared: true,
            resize_w: 512,
            resize_h: 512,
            ai: ui::ai_dialog::AiDialog::default(),
            hsv: Default::default(),
        }
    }
}

impl EditorApp {
    pub fn open_hsv(&mut self) {
        if self.ai.open || self.hsv.open || self.project().layers.is_empty() { return; }
        self.close_asset_preview();
        self.commit_transform();
        self.hsv = ui::hsv_dialog::HsvDialog {
            open:true,layer:self.project().active_layer,document_id:self.workspace.active_doc().id.clone(),
            ..Default::default()
        };
    }

    pub fn apply_hsv(&mut self) {
        if !self.hsv.open { return; }
        self.hsv.open = false;
        if self.hsv.document_id != self.workspace.active_doc().id { return; }
        let index = self.hsv.layer;
        let adjustment = self.hsv.adjustment;
        let Some(layer) = self.workspace.active_mut().layers.get_mut(index) else { return; };
        let before = layer.pixels.clone();
        if !adjustment.apply(&mut layer.pixels) { return; }
        self.history.push_raster(index,before,layer.pixels.clone());
        self.layers_reload = true;
        self.dirty = true;
        self.workspace.mark_active_thumb_dirty();
        self.status = "HSV adjustment applied".into();
    }

    pub fn open_ai(&mut self) {
        self.open_ai_mode(AiMode::Edit);
    }

    pub fn open_ai_qwen_draw(&mut self) {
        self.open_ai_mode(AiMode::QwenDraw);
    }

    pub fn open_ai_draw(&mut self) {
        self.open_ai_mode(AiMode::Draw);
    }

    pub fn open_ai_anime_draw(&mut self) {
        self.open_ai_mode(AiMode::AnimeDraw);
    }

    pub fn open_ai_anime_redraw(&mut self) {
        self.open_ai_mode(AiMode::AnimeRedraw);
    }

    pub fn open_ai_redraw(&mut self) {
        self.open_ai_mode(AiMode::Redraw);
    }

    pub fn open_ai_inpaint(&mut self) {
        self.open_ai_mode(AiMode::Inpaint);
    }

    pub fn open_ai_lineart(&mut self) {
        self.open_ai_mode(AiMode::Lineart);
    }

    pub fn open_ai_draw_cn(&mut self) {
        self.open_ai_mode(AiMode::DrawCn);
    }

    pub fn open_ai_upscale(&mut self) {
        self.open_ai_mode(AiMode::Upscale);
    }

    fn open_ai_mode(&mut self, mode: AiMode) {
        if (mode.needs_input() && self.image_size.is_none()) || self.ai.job.is_some() { return; }
        self.commit_transform();
        self.ai.selection_bounds = self.selection.as_ref().and_then(Selection::bounds);
        self.ai.layer_bounds = self.project().active_pixels().and_then(|pixels| pixels.bounds());
        self.workspace.clear_asset_drag();
        self.layer_drag = None;
        self.ai.open = true;
        self.ai.preview_view = Default::default();
        self.ai.mode = mode;
        if mode == AiMode::Upscale { self.ai.fit_image = false; }
        if mode == AiMode::Inpaint && self.ai.source == AiSource::Selection { self.ai.source = AiSource::Layer; }
        self.ai.catalog_load = None;
        if mode.uses_sd() { self.ai.catalog = Default::default(); }
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
        self.ai.preview_view = Default::default();
        let Some(config) = self.ai.config.clone() else { return; };
        if !self.ai.mode.needs_input() {
            if let Err(error) = self.ai.draw_settings.validate() {
                self.ai.message = error;
                return;
            }
            self.ai.document_id = self.workspace.active_doc().id.clone();
            let (width, height) = if self.project().width == 0 || self.project().height == 0 {
                (self.ai.draw_settings.width as u32, self.ai.draw_settings.height as u32)
            } else { (self.project().width, self.project().height) };
            self.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: width, max_y: height }.into());
            self.ai.message = "Preparing workflow...".into();
            self.ai.preview = None;
            self.ai.result = None;
            self.ai.preview_dirty = false;
            self.ai.fraction = None;
            self.ai.job = Some(if self.ai.mode == AiMode::QwenDraw {
                crate::comfyui::Job::start_qwen_draw(config, self.ai.draw_settings.clone())
            } else if self.ai.mode == AiMode::AnimeDraw {
                crate::comfyui::Job::start_anime_draw(config, self.ai.draw_settings.clone())
            } else {
                crate::comfyui::Job::start_draw(config, self.ai.draw_settings.clone())
            });
            return;
        }
        if matches!(self.ai.mode, AiMode::Redraw | AiMode::AnimeRedraw | AiMode::Inpaint) {
            if let Err(error) = self.ai.draw_settings.validate_redraw(self.ai.denoise) {
                self.ai.message = error;
                return;
            }
        }
        if self.ai.mode == AiMode::Upscale {
            if let Err(error) = self.ai.upscale.validate(&self.ai.draw_settings) {
                self.ai.message = error;
                return;
            }
        }
        if self.ai.mode == AiMode::DrawCn {
            if let Err(error) = self.ai.control_net.validate(&self.ai.draw_settings) {
                self.ai.message = error;
                return;
            }
        }
        if self.ai.mode == AiMode::Lineart {
            if let Err(error) = crate::comfyui::validate_lineart_resolution(self.ai.lineart_resolution) {
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
        self.ai.job = Some(if let Some(mask) = mask {
            crate::comfyui::Job::start_inpaint(config, image, mask, self.ai.scale, self.ai.draw_settings.clone(), self.ai.denoise)
        } else if self.ai.mode == AiMode::AnimeRedraw {
            crate::comfyui::Job::start_anime_redraw(config, image, self.ai.scale, self.ai.draw_settings.clone(), self.ai.denoise)
        } else if self.ai.mode == AiMode::Redraw {
            crate::comfyui::Job::start_redraw(config, image, self.ai.scale, self.ai.draw_settings.clone(), self.ai.denoise)
        } else if self.ai.mode == AiMode::Lineart {
            crate::comfyui::Job::start_lineart(config, image, self.ai.scale, self.ai.lineart_resolution)
        } else if self.ai.mode == AiMode::DrawCn {
            crate::comfyui::Job::start_draw_cn(config, image, self.ai.scale, self.ai.draw_settings.clone(), self.ai.control_net)
        } else if self.ai.mode == AiMode::Upscale {
            crate::comfyui::Job::start_upscale(config, image, self.ai.scale, self.ai.draw_settings.clone(), self.ai.upscale)
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
        let region = self.ai_source_region()?;
        let (width, height) = region.size();
        crate::comfyui::scaled_size(width, height, 1.0)?;
        crate::comfyui::scaled_size(width, height, self.ai.scale)?;
        let mask = image::RgbaImage::from_fn(width, height, |x, y| {
            let x = region.min_x as i64 + x as i64;
            let y = region.min_y as i64 + y as i64;
            let value = if x >= 0 && y >= 0 && selection.contains(x as u32, y as u32) { 255 } else { 0 };
            image::Rgba([value, value, value, 255])
        });
        let softness = self.ai.mask_softness;
        if !softness.is_finite() || !(0.0..=256.0).contains(&softness) {
            return Err("Mask softness must be between 0 and 256 px".into());
        }
        Ok(if softness > 0.0 { image::imageops::blur(&mask, softness) } else { mask })
    }

    /// Crop before worker-side scaling; retain document coordinates for Accept / Fit.
    fn ai_source_region(&self) -> Result<crate::raster::RasterRect, String> {
        let p = self.project();
        if self.ai.source == AiSource::Selection {
            let selection = self.selection.as_ref().ok_or("Select an area on the canvas first")?;
            if selection.width != p.width || selection.height != p.height {
                return Err("Selection does not match the current canvas".into());
            }
            return selection.bounds().map(Into::into).ok_or("Select an area on the canvas first".into());
        }
        if self.ai.source == AiSource::Layer {
            let pixels = p.active_pixels().ok_or("Choose a raster layer first")?;
            return self.ai.source_region(p.width, p.height, pixels.bounds());
        }
        self.ai.source_region(p.width, p.height, None)
    }

    fn ai_input(&self) -> Result<(image::RgbaImage, crate::raster::RasterRect), String> {
        let p = self.project();
        let region = self.ai_source_region()?;
        let (width, height) = region.size();
        crate::comfyui::scaled_size(width, height, 1.0)?;
        crate::comfyui::scaled_size(width, height, self.ai.scale)?;
        let pixels = match self.ai.source {
            AiSource::Layer => p.active_pixels().ok_or("Choose a raster layer first")?.region(region.min_x, region.min_y, width, height),
            AiSource::Document | AiSource::Selection => crate::editing::composite_layers(&p.layers, p.width, p.height),
        };
        let image = if self.ai.source == AiSource::Selection {
            let selection = self.selection.as_ref().unwrap();
            let mut cropped = image::RgbaImage::new(width, height);
            for y in region.min_y as u32..region.max_y as u32 {
                for x in region.min_x as u32..region.max_x as u32 {
                    if selection.contains(x, y) {
                        let index = ((y as usize * p.width as usize) + x as usize) * 4;
                        cropped.put_pixel(x - region.min_x as u32, y - region.min_y as u32,
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
        if let Some(job) = self.ai.job.take() {
            job.cancel();
            self.ai.result = None;
            self.ai.fraction = None;
            self.ai.message = "Cancelled locally; remote stop requested in background".into();
        }
    }

    pub fn close_ai(&mut self) {
        self.cancel_ai();
        self.ai.open = false;
        self.ai.catalog_load = None;
        self.ai.result = None;
        self.ai.input_region = None;
        self.ai.preview = None;
        self.ai.preview_dirty = false;
        self.ai.fraction = None;
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
        let size = if self.ai.fit_image { region.size() } else { image.dimensions() };
        let action = self.ai.mode.layer_prefix();
        let name = format!("{action} {}", self.workspace.active().layers.len() + 1);
        let project = self.workspace.active_mut();
        let insertion = if !self.ai.mode.needs_input() && (project.width == 0 || project.height == 0) {
            project.import_rgba(image.as_raw(), image.width(), image.height()).map(|()| {
                project.layers[0].name = name;
                project.sync_manifest_pub();
            })
        } else {
            project.add_raster_layer(image, name, region, self.ai.fit_image)
        };
        match insertion {
            Ok(()) => {
                let needs_fit = self.image_size.is_none();
                self.history.clear();
                self.sync_layer_structure();
                self.ai.layer_bounds = self.project().active_pixels().and_then(|pixels| pixels.bounds());
                if needs_fit { self.fit_image = true; }
                self.status = format!("{action} added: {} × {} px", size.0, size.1);
                self.ai.preview = self.ai.result.take();
                self.ai.message = format!("{} — regenerate for another result", self.status);
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
            || !self.lasso_points.is_empty() || self.selection_brush_last.is_some()
    }

    pub fn has_floating_transform(&self) -> bool {
        self.floating.is_some()
    }

    pub fn display_size(&self) -> Option<(u32, u32)> {
        if let Some(id) = &self.preview_asset {
            return self.workspace.assets.iter().find(|a| &a.id == id).map(|a| (a.width, a.height));
        }
        self.image_size
    }

    pub fn show_asset_preview(&mut self, index: usize) {
        let Some(asset) = self.workspace.assets.get(index) else { return; };
        let id = asset.id.clone();
        if self.preview_asset.as_ref() == Some(&id) { return; }
        self.commit_transform();
        self.brush_last = None;
        self.lasso_points.clear();
        self.selection_brush_last = None;
        self.selection_anchor = None;
        self.selection_current = None;
        if self.preview_asset.is_none() {
            self.preview_document_view = Some((self.pan, self.zoom, self.rotation));
        }
        self.preview_asset = Some(id);
        self.preview_asset_dirty = true;
        self.rotation = 0.0;
        self.fit_image = true;
    }

    pub fn close_asset_preview(&mut self) {
        if self.preview_asset.take().is_some() {
            self.layers_reload = true;
            self.selection_mask_dirty = true;
            if let Some((pan, zoom, rotation)) = self.preview_document_view.take() {
                self.pan = pan;
                self.zoom = zoom;
                self.rotation = rotation;
            }
            self.fit_image = false;
        }
    }

    pub fn canvas_view(&self) -> Option<CanvasView> {
        let (width, height) = self.display_size()?;
        Some(CanvasView {
            center: self.canvas_rect.min + rect_size(self.canvas_rect) * 0.5 + self.pan,
            zoom: self.zoom,
            rotation: self.rotation.to_radians(),
            width,
            height,
        })
    }

    pub fn new_project(&mut self, ui: &mut Ui) {
        self.close_asset_preview();
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

    fn copy_layer_text(&mut self, index: usize) -> Result<String, String> {
        self.commit_transform();
        let project = self.project();
        let layer = project.layers.get(index).ok_or("Choose a layer to copy")?;
        crate::layer_clipboard::encode(layer, (project.width, project.height))
    }

    pub fn copy_layer(&mut self, ui: &mut Ui) {
        self.copy_layer_at(self.project().active_layer, ui);
    }

    pub fn copy_layer_at(&mut self, index: usize, ui: &mut Ui) {
        match self.copy_layer_text(index).and_then(crate::layer_clipboard::write) {
            Ok(()) => self.status = "Layer copied with position and properties".into(),
            Err(error) => self.show_error(ui, error),
        }
    }

    fn insert_copied_layer(&mut self, copied: crate::layer_clipboard::CopiedLayer) {
        self.commit_transform();
        self.close_asset_preview();
        let was_empty = self.image_size.is_none();
        self.history.insert_layer(self.workspace.active_mut(), copied.layer, copied.canvas_size);
        self.sync_layer_structure();
        if was_empty { self.fit_image = true; }
        self.status = "Layer pasted with position and properties".into();
    }

    pub fn paste_layer(&mut self, ui: &mut Ui) {
        match crate::layer_clipboard::read() {
            Ok(Some(copied)) => self.insert_copied_layer(copied),
            Ok(None) => self.show_error(ui, "Clipboard does not contain a copied layer".into()),
            Err(error) => self.show_error(ui, error),
        }
    }

    fn paste_clipboard(&mut self, ui: &mut Ui) {
        match crate::layer_clipboard::read() {
            Ok(Some(copied)) => self.insert_copied_layer(copied),
            Ok(None) => self.paste_image(ui),
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
        self.commit_transform();
        match self.workspace.save(&path) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", file_name(&path));
                ui.notify_success("Project saved");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn optimize_project(&mut self, ui: &mut Ui) {
        match self.workspace.optimize() {
            Ok((before, after)) => {
                self.status = format!("Project optimized: {} → {} KiB", before / 1024, after / 1024);
                ui.notify_success("Saved project optimized");
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
        self.close_asset_preview();
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
        self.close_asset_preview();
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

    pub fn invert_layer(&mut self) {
        if self.ai.open || self.hsv.open || self.project().layers.is_empty() { return; }
        self.close_asset_preview();
        self.commit_transform();
        let index = self.project().active_layer;
        let Some(layer) = self.workspace.active_mut().layers.get_mut(index) else { return; };
        let before = layer.pixels.clone();
        if !layer.pixels.invert_colors() { return; }
        self.history.push_raster(index, before, layer.pixels.clone());
        self.layers_reload = true;
        self.dirty = true;
        self.workspace.mark_active_thumb_dirty();
        self.status = "Layer colors inverted".into();
    }

    pub fn fix_alpha(&mut self) {
        self.commit_transform();
        let layer = self.project().active_layer;
        let Some(pixels) = self.workspace.active_mut().layers.get_mut(layer).map(|layer| &mut layer.pixels) else { return; };
        let before = pixels.clone();
        let changed = pixels.fix_alpha();
        if changed == 0 {
            self.status = "Fix Alpha: no pixels below alpha 5".into();
            return;
        }
        self.history.push_raster(layer, before, pixels.clone());
        self.layers_reload = true;
        self.dirty = true;
        self.workspace.mark_active_thumb_dirty();
        self.status = format!("Fix Alpha: cleared {changed} pixels");
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

    pub fn merge_layer_down(&mut self) {
        let index = self.project().active_layer;
        if index == 0 || index >= self.project().layers.len() { return; }
        self.commit_transform();
        if !self.history.merge_down(self.workspace.active_mut()) { return; }
        self.layer_drag = None;
        self.sync_layer_structure();
        self.status = "Layer merged down".into();
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

    pub fn trim_canvas(&mut self, ui: &mut Ui) {
        self.close_asset_preview();
        self.commit_transform();
        if !self.history.trim_canvas(self.workspace.active_mut()) {
            self.status = "Trim: no transparent border to remove".into();
            return;
        }
        self.sync_layer_structure();
        self.fit_image = true;
        let (width,height) = self.image_size.unwrap();
        self.status = format!("Canvas trimmed to {width} × {height}");
        ui.notify_success("Canvas trimmed");
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
        self.close_asset_preview();
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
        let Some((width, height)) = self.display_size() else {
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
        self.lasso_points.clear();
        self.selection_brush_last = None;
        self.brush_last = None;
        self.tool = tool;
        if tool == Tool::Transform && self.preview_asset.is_none() {
            self.begin_transform();
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

    fn handle_lasso(&mut self, view: CanvasView) {
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

    fn handle_selection_brush(&mut self, view: CanvasView) {
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

    /// Brush tool with Ctrl held samples the composited image instead of painting.
    pub fn eyedropper_armed(&self) -> bool {
        self.tool == Tool::Brush
            && self.input.key_ctrl
            && self.brush_last.is_none()
            && self.preview_asset.is_none()
    }

    fn pick_composite_color(&mut self, view: CanvasView) {
        let Some((x, y)) = screen_to_pixel(self.input.mouse_pos, view) else {
            return;
        };
        let pixel = composite_pixel(&self.project().layers, x as i32, y as i32);
        self.brush_color = pixel.map(|channel| channel as f32 / 255.0);
        self.status = format!("Picked #{:02X}{:02X}{:02X}", pixel[0], pixel[1], pixel[2]);
    }

    fn handle_brush(&mut self, view: CanvasView, erase: bool) {
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

    fn begin_transform(&mut self) {
        if self.floating.is_some() {
            return;
        }
        let Some((width, height)) = self.image_size else {
            self.status = "No image".into();
            return;
        };
        let project = self.workspace.active();
        let layer = project.active_layer;
        let Some(target) = project.layers.get(layer) else { return; };
        let floating = if let Some(selection) = self.selection.as_ref() {
            FloatingTransform::extract(&target.pixels, width, height, selection, layer, target.opacity)
        } else {
            FloatingTransform::extract_layer(&target.pixels, layer, target.opacity)
        };
        let Some(floating) = floating else { return; };
        self.floating = Some(floating);
        self.floating_reload = true;
        self.clear_selection();
        self.dirty = true;
        self.status = "Transform — inside move · outside rotate · corners scale · Enter apply · Esc cancel".into();
    }

    pub fn flip_transform(&mut self, horizontal: bool) {
        if self.tool != Tool::Transform || self.ai.open || self.hsv.open || self.preview_asset.is_some() { return; }
        self.begin_transform();
        let Some(floating) = self.floating.as_mut() else { return; };
        if horizontal { floating.flip_x = !floating.flip_x; }
        else { floating.flip_y = !floating.flip_y; }
        self.status = if horizontal { "Transform: Flip X" } else { "Transform: Flip Y" }.into();
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
        self.history.push_raster(layer, before, pixels.clone());
        if let Some(selection) = transformed_selection { self.set_selection(selection); }
        self.layers_reload = true;
        self.transform_drag = None;
        self.transform_last = None;
        self.floating_reload = false;
        self.dirty = true;
        self.workspace.mark_active_thumb_dirty();
        self.status = "Transform applied".into();

    }

    pub fn transform_cursor(&self, view: CanvasView) -> Option<TransformCursor> {
        if self.tool != Tool::Transform || self.preview_asset.is_some() || self.ai.open || self.hsv.open { return None; }
        let floating = self.floating.as_ref()?;
        let hit = self.transform_drag.or_else(|| floating.hit_test(
            view.screen_to_image(self.input.mouse_pos),8.0/view.zoom.max(1.0e-6)))?;
        Some(match hit {
            TransformHit::Body => TransformCursor::Move,
            TransformHit::Pivot => TransformCursor::Pivot,
            TransformHit::Rotate => TransformCursor::Rotate,
            TransformHit::Scale(i) => TransformCursor::resize(
                view.image_to_screen(floating.corners()[i as usize])-view.image_to_screen(floating.center)),
        })
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
        let handle_r = 8.0 / view.zoom.max(1e-6);

        if self.input.mouse_pressed {
            let hit = self
                .floating
                .as_ref()
                .and_then(|f| f.hit_test(image_pos, handle_r));
            if let Some(hit) = hit {
                self.transform_drag = Some(hit);
                self.transform_last = Some(image_pos);
            }
        }
        if self.input.mouse_down
            && let (Some(hit), Some(prev)) = (self.transform_drag, self.transform_last)
        {
            let mut scale = Vec2::ONE;
            let mut rotation = 0.0f32;
            if let Some(floating) = self.floating.as_mut() {
                floating.keep_aspect_ratio = self.transform_keep_aspect_ratio;
                floating.drag(hit,prev,image_pos);
                scale = floating.scale;
                rotation = floating.rotation;
            }
            self.transform_last = Some(image_pos);
            self.status = format!(
                "Transform  scale {:.2} × {:.2}  rot {:.0}°",
                scale.x, scale.y,
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
        self.canvas_cursor = None;
        self.ai.preview_input = input.clone();
        self.input = if self.ai.open || self.hsv.open {
            let mut blocked = FrameInput::default();
            blocked.key_escape = input.key_escape;
            if self.hsv.open { blocked.key_enter = input.key_enter; }
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
        let copy = self.input.key_copy;
        ui::draw(ui, self, viewport);
        if copy { self.copy_layer(ui); }
        if paste {
            self.paste_clipboard(ui);
        }
        self.ai.open || self.hsv.open
            || self.selection.is_some()
            || self.selection_anchor.is_some()
            || self.floating.is_some()
            || self.workspace.asset_drag.is_some()
            || thumb_waiting
            || (matches!(self.tool, Tool::Brush | Tool::SelectionBrush) && self.canvas_rect.contains(self.input.mouse_pos))
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
        if let Some(id) = &self.preview_asset
            && let Some(asset) = self.workspace.assets.iter().find(|a| &a.id == id)
        {
            if self.preview_asset_dirty {
                canvas.upload_all_layers(device, queue, &[&asset.pixels], asset.width, asset.height);
                self.preview_asset_dirty = false;
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
        if self.selection_mask_dirty {
            let mask = self.selection.as_ref().map(|s| s.mask_r8());
            canvas.set_selection_mask(device, queue, mask.as_deref(), width, height);
            self.selection_mask_dirty = false;
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
        if self.floating.is_none() {
            canvas.clear_transform();
            self.floating_reload = false;
        } else if let Some(floating) = self.floating.as_ref() {
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
        if self.preview_asset.is_some() {
            return Some((self.canvas_rect, view, CanvasOverlay {
                layers: vec![LayerDraw { visible: true, opacity: 1.0 }],
                ..Default::default()
            }));
        }
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
        if self.hsv.open && self.hsv.preview && self.hsv.document_id == self.workspace.active_doc().id {
            overlay.hsv = Some((self.hsv.layer,self.hsv.adjustment));
        }
        if matches!(self.tool, Tool::Brush | Tool::SelectionBrush) && !self.eyedropper_armed() {
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
                whole_layer: floating.whole_layer,
                layer: floating.layer,
                origin: Vec2::new(floating.origin_x as f32, floating.origin_y as f32),
                size: Vec2::new(floating.bw as f32, floating.bh as f32),
            });
            let corners = floating.source_corners().map(|p| view.image_to_screen(p));
            overlay.transform_corners = Some(corners);
            overlay.transform_pivot = Some(view.image_to_screen(floating.pivot));
        }
        Some((self.canvas_rect, view, overlay))
    }

    fn canvas_cursor(&self) -> Option<TransformCursor> { self.canvas_cursor }
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

#[cfg(test)]
mod hsv_tests {
    use super::*;
    use crate::hsv::HsvAdjustment;
    use crate::raster::Raster;

    fn document() -> EditorApp {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[0,0,255,255].repeat(4),2,2).unwrap();
        app.project_mut().add_layer();
        app.project_mut().layers[1].pixels.write(0,-20,30,[255,0,0,128]);
        app.project_mut().layers[1].opacity = 0.7;
        app.sync_layer_structure();
        app.dirty = false;
        app.canvas_rect = Rect::from_min_size(Vec2::ZERO,Vec2::splat(400.0));
        app
    }

    #[test]
    fn hsv_preview_does_not_edit_pixels_and_apply_is_one_undoable_active_layer_edit() {
        let mut app = document();
        let before:Vec<_> = app.project().layers.iter().map(|l|l.pixels.clone()).collect();
        app.open_hsv();
        app.hsv.adjustment = HsvAdjustment {hue:120.0,..Default::default()};
        assert!(app.canvas_present().unwrap().2.hsv.is_some());
        assert_eq!(app.project().layers[1].pixels,before[1]);
        assert!(!app.dirty);
        app.hsv.preview = false;
        assert!(app.canvas_present().unwrap().2.hsv.is_none());
        app.apply_hsv();
        assert!(!app.hsv.open);
        assert_eq!(app.project().layers[0].pixels,before[0]);
        assert_eq!(app.project().layers[1].pixels.read(0,-20,30),[0,255,0,128]);
        assert_eq!(app.project().layers[1].opacity,0.7);
        assert!(app.dirty);
        app.undo();
        assert_eq!(app.project().layers[1].pixels,before[1]);
        app.redo();
        assert_eq!(app.project().layers[1].pixels.read(0,-20,30),[0,255,0,128]);
        app.open_hsv();
        assert!(app.hsv.adjustment.is_identity());
        app.apply_hsv();
        app.undo();
        assert_eq!(app.project().layers[1].pixels,before[1],"neutral Apply must not add a history entry");
    }

    #[test]
    fn hsv_cancel_blocks_editor_input_and_keeps_pixels_history_and_dirty_state() {
        let mut app = document();
        let original = app.project().layers[1].pixels.clone();
        app.open_hsv();
        app.hsv.adjustment.value = -75.0;
        let mut input = FrameInput::default();
        input.mouse_pressed = true;
        input.key_undo = true;
        input.key_paste = true;
        input.key_escape = true;
        app.begin_frame(&input);
        assert!(!app.input.mouse_pressed && !app.input.key_undo && !app.input.key_paste);
        let mut ui = Ui::new();
        ui.begin_frame(mega_ui::UiInput {viewport:Vec2::new(900.0,700.0),..Default::default()});
        ui::hsv_dialog::draw(&mut ui,&mut app);
        ui.end_frame();
        assert!(!app.hsv.open);
        assert_eq!(app.project().layers[1].pixels,original);
        assert!(!app.dirty);
        assert!(app.history.undo(app.workspace.active_mut()).is_none());
    }
}

#[cfg(test)]
mod transform_interaction_tests {
    use super::*;

    fn document() -> EditorApp {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[100,150,200,255].repeat(100*80),100,80).unwrap();
        app.sync_layer_structure();
        app.canvas_rect = Rect::from_min_size(Vec2::ZERO,Vec2::splat(500.0));
        app.set_tool(Tool::Transform);
        app
    }

    fn pointer(app: &mut EditorApp, point: Vec2, pressed: bool, down: bool) {
        let view = app.canvas_view().unwrap();
        app.input = FrameInput::default();
        app.input.mouse_pos = view.image_to_screen(point);
        app.input.mouse_pressed = pressed;
        app.input.mouse_down = down;
        app.input.mouse_released = !down;
        app.handle_canvas_edit(view);
    }

    #[test]
    fn transform_flip_buttons_preview_cancel_commit_and_undo() {
        let bytes = [10,20,30,255, 40,50,60,255, 70,80,90,255, 100,110,120,255];
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&bytes,2,2).unwrap();
        app.sync_layer_structure();
        app.set_tool(Tool::Transform);
        let original = app.project().layers[0].pixels.clone();
        let corners = app.floating.as_ref().unwrap().corners();
        app.flip_transform(true);
        app.flip_transform(false);
        assert_eq!(app.project().layers[0].pixels,original);
        assert_eq!(app.floating.as_ref().unwrap().corners(),corners);
        app.cancel_transform();
        assert_eq!(app.project().layers[0].pixels,original);
        app.flip_transform(true); // Also starts a new floating transform after Cancel.
        app.flip_transform(true);
        assert!(!app.floating.as_ref().unwrap().flip_x);
        app.flip_transform(true);
        app.flip_transform(false);
        app.commit_transform();
        let expected: Vec<u8> = bytes.chunks_exact(4).rev().flatten().copied().collect();
        assert_eq!(app.project().layers[0].pixels.to_rgba(2,2),expected);
        app.undo();
        assert_eq!(app.project().layers[0].pixels,original);
        app.redo();
        assert_eq!(app.project().layers[0].pixels.to_rgba(2,2),expected);
    }

    #[test]
    fn transform_aspect_checkbox_controls_pointer_scaling() {
        for locked in [true,false] {
            let mut app = document();
            assert!(app.transform_keep_aspect_ratio);
            app.transform_keep_aspect_ratio = locked;
            let floating = app.floating.as_ref().unwrap();
            let pivot = floating.pivot;
            let corner = floating.corners()[2];
            pointer(&mut app,corner,true,true);
            pointer(&mut app,pivot+(corner-pivot)*Vec2::new(2.0,1.0),false,true);
            let scale = app.floating.as_ref().unwrap().scale;
            if locked { assert!((scale.x-scale.y).abs()<1.0e-5); }
            else { assert!((scale-Vec2::new(2.0,1.0)).length()<1.0e-5); }
        }
    }

    #[test]
    fn outside_drag_rotates_without_committing_and_escape_restores_pixels() {
        let mut app = document();
        let original = app.project().layers[0].pixels.clone();
        pointer(&mut app,Vec2::new(-20.0,40.0),true,true);
        assert_eq!(app.transform_drag,Some(TransformHit::Rotate));
        pointer(&mut app,Vec2::new(50.0,-30.0),false,true);
        assert!((app.floating.as_ref().unwrap().rotation-std::f32::consts::FRAC_PI_2).abs()<1.0e-5);
        let view = app.canvas_view().unwrap();
        assert_eq!(app.transform_cursor(view),Some(TransformCursor::Rotate));
        pointer(&mut app,Vec2::new(50.0,-30.0),false,false);
        assert!(app.transform_drag.is_none() && app.floating.is_some());
        app.input.key_escape = true;
        app.handle_canvas_edit(view);
        assert!(app.floating.is_none());
        assert_eq!(app.project().layers[0].pixels,original);
    }

    #[test]
    fn pivot_drag_keeps_image_stationary_and_body_drag_carries_pivot() {
        let mut app = document();
        let before = app.floating.as_ref().unwrap().corners();
        pointer(&mut app,Vec2::new(50.0,40.0),true,true);
        assert_eq!(app.transform_drag,Some(TransformHit::Pivot));
        pointer(&mut app,Vec2::new(35.0,55.0),false,true);
        assert_eq!(app.floating.as_ref().unwrap().corners(),before);
        pointer(&mut app,Vec2::new(35.0,55.0),false,false);
        pointer(&mut app,Vec2::new(75.0,20.0),true,true);
        assert_eq!(app.transform_drag,Some(TransformHit::Body));
        pointer(&mut app,Vec2::new(85.0,25.0),false,true);
        assert_eq!(app.floating.as_ref().unwrap().pivot,Vec2::new(45.0,60.0));
        assert_eq!(app.floating.as_ref().unwrap().center,Vec2::new(60.0,45.0));
        assert_eq!(app.transform_cursor(app.canvas_view().unwrap()),Some(TransformCursor::Move));
    }

    #[test]
    fn resize_cursors_follow_canvas_rotation_with_a_constant_screen_hit_radius() {
        let mut app = document();
        let corner = app.floating.as_ref().unwrap().corners()[0];
        for zoom in [0.25,1.0,32.0] {
            app.zoom = zoom;
            let view = app.canvas_view().unwrap();
            app.input.mouse_pos = view.image_to_screen(corner)+Vec2::X*7.0;
            assert_eq!(app.transform_cursor(view),Some(TransformCursor::ResizeNwse));
            app.input.mouse_pos = view.image_to_screen(corner)+Vec2::X*12.0;
            assert_eq!(app.transform_cursor(view),Some(TransformCursor::Move));
        }
        app.rotation = 90.0;
        let view = app.canvas_view().unwrap();
        app.input.mouse_pos = view.image_to_screen(corner);
        assert_eq!(app.transform_cursor(view),Some(TransformCursor::ResizeNesw));
        app.transform_drag = Some(TransformHit::Scale(0));
        app.input.mouse_pos = view.image_to_screen(Vec2::splat(-100.0));
        assert_eq!(app.transform_cursor(view),Some(TransformCursor::ResizeNesw));
    }
}

#[cfg(test)]
mod sparse_layer_tests {
    use super::*;
    use crate::raster::Raster;
    use std::sync::Arc;

    #[test]
    fn trim_canvas_commits_pending_transform_updates_fields_and_preserves_undo() {
        let mut app = EditorApp::default();
        app.project_mut().resize_canvas(100,80).unwrap();
        app.project_mut().layers[0].pixels.write(0,20,30,[100,150,200,255]);
        app.project_mut().layers[0].pixels.write(0,24,32,[200,150,100,255]);
        app.sync_layer_structure();
        app.begin_transform();
        app.floating.as_mut().unwrap().center += glam::Vec2::new(10.0,5.0);
        app.trim_canvas(&mut Ui::new());
        assert!(app.floating.is_none());
        assert_eq!(app.image_size,Some((5,3)));
        assert_eq!((app.resize_w,app.resize_h),(5,3));
        assert_eq!(app.project().layers[0].pixels.read(0,0,0),[100,150,200,255]);
        assert_eq!(app.project().layers[0].pixels.read(0,4,2),[200,150,100,255]);
        assert!(app.dirty && app.fit_image);
        app.undo();
        assert_eq!(app.image_size,Some((100,80)));
        assert_eq!((app.resize_w,app.resize_h),(100,80));
        assert_eq!(app.project().layers[0].pixels.read(0,30,35),[100,150,200,255]);
        app.undo();
        assert_eq!(app.project().layers[0].pixels.read(0,20,30),[100,150,200,255]);
        app.redo();
        app.redo();
        assert_eq!(app.image_size,Some((5,3)));
    }

    #[test]
    fn soft_brush_setting_reaches_raster_and_undo_restores_original_tiles() {
        let mut app = document();
        let original = app.project().layers[0].pixels.clone();
        app.brush_hardness = 0.0;
        app.brush_size = 8.0;
        app.brush_opacity = 0.5;
        app.brush_color = [1.0, 0.0, 0.0, 1.0];
        app.stroke_capture = Some(EditCapture::new(0, 10, 10));
        app.apply_brush_segment((4.5, 4.5), (4.5, 4.5), 10, 10, false);
        assert_eq!(app.project().layers[0].pixels.read(0, 4, 4), [255, 0, 0, 128]);
        assert_eq!(app.project().layers[0].pixels.read(0, 6, 4), [255, 0, 0, 64]);
        let painted = app.project().layers[0].pixels.clone();
        app.apply_brush_segment((4.5, 4.5), (4.5, 4.5), 10, 10, false);
        assert_eq!(app.project().layers[0].pixels, painted);
        let edit = app.stroke_capture.take().unwrap().finish(&mut app.project_mut().layers[0].pixels).unwrap();
        app.history.push(edit);
        app.undo();
        assert_eq!(app.project().layers[0].pixels, original);
        app.redo();
        assert_eq!(app.project().layers[0].pixels, painted);
    }

    #[test]
    fn ctrl_click_picks_the_composite_and_leaves_the_layer_unchanged() {
        let mut app = document();
        app.set_tool(Tool::Brush);
        app.project_mut().layers[0].opacity = 0.5;
        app.project_mut().add_layer();
        app.project_mut().layers[1].pixels.write(0, 4, 4, [0, 0, 255, 255]);
        let before = app.project().layers.clone();
        app.canvas_rect = Rect::from_min_size(Vec2::ZERO, Vec2::splat(400.0));
        app.brush_color = [1.0, 1.0, 1.0, 1.0];
        let view = app.canvas_view().unwrap();
        app.input.mouse_pos = view.image_to_screen(Vec2::new(2.5, 3.5));
        app.input.mouse_pressed = true;
        app.input.mouse_down = true;
        app.input.key_ctrl = true;
        assert!(app.eyedropper_armed());
        app.handle_canvas_edit(view);
        let covered = composite_pixel(&app.project().layers, 2, 3);
        assert_ne!(covered, [0, 0, 0, 0]);
        assert_eq!(app.brush_color, covered.map(|c| c as f32 / 255.0));
        app.input.mouse_pos = view.image_to_screen(Vec2::new(4.5, 4.5));
        app.handle_canvas_edit(view);
        assert_eq!(app.brush_color, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(app.project().layers[0].pixels, before[0].pixels);
        assert_eq!(app.project().layers[1].pixels, before[1].pixels);
        assert!(app.brush_last.is_none());
        assert!(app.canvas_present().unwrap().2.brush_center.is_none());
        app.input.key_ctrl = false;
        app.input.mouse_pressed = true;
        app.handle_canvas_edit(view);
        assert_ne!(app.project().layers[1].pixels, before[1].pixels);
        app.input.mouse_pressed = false;
        app.input.key_ctrl = true;
        assert!(!app.eyedropper_armed());
    }

    fn document() -> EditorApp {
        let mut app = EditorApp::default();
        app.project_mut().resize_canvas(10,10).unwrap();
        app.project_mut().layers[0].pixels.write(0,2,3,[100,20,30,255]);
        app.refresh_image().unwrap();
        app
    }

    #[test]
    fn whole_layer_moves_out_and_back_without_copying_or_losing_tiles_and_undoes() {
        let mut app = document();
        let original = app.project().layers[0].pixels.clone();
        for delta in [Vec2::new(-30.0,40.0),Vec2::new(30.0,-40.0)] {
            app.begin_transform();
            let floating = app.floating.as_mut().unwrap();
            assert!(floating.pixels.is_empty() && floating.mask.is_empty());
            floating.center += delta;
            app.commit_transform();
            assert!(Arc::ptr_eq(&original.tiles[&(0,0)],&app.project().layers[0].pixels.tiles[&(0,0)]));
        }
        assert_eq!(app.project().layers[0].pixels,original);
        app.undo();
        assert_eq!(app.project().layers[0].pixels.read(0,-28,43),[100,20,30,255]);
        app.redo();
        assert_eq!(app.project().layers[0].pixels,original);
    }

    #[test]
    fn selected_fragment_survives_negative_transform_save_and_canvas_resize() {
        let mut app = document();
        app.set_selection(Selection::from_rect(PixelRect { min_x:2,min_y:3,max_x:3,max_y:4 },10,10));
        app.begin_transform();
        app.floating.as_mut().unwrap().center -= Vec2::new(20.0,20.0);
        app.commit_transform();
        assert_eq!(app.project().layers[0].pixels.read(0,-18,-17),[100,20,30,255]);
        assert_eq!(app.project().layers[0].pixels.read(0,2,3),[0;4]);
        app.undo();
        assert_eq!(app.project().layers[0].pixels.read(0,2,3),[100,20,30,255]);
        assert_eq!(app.project().layers[0].pixels.read(0,-18,-17),[0;4]);
        app.redo();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("offcanvas.mgimg");
        app.workspace.save(&path).unwrap();
        let loaded = Workspace::open(&path).unwrap();
        assert_eq!(loaded.active().layers[0].pixels,app.project().layers[0].pixels);
        app.project_mut().resize_canvas(1,1).unwrap();
        app.project_mut().resize_canvas(100,100).unwrap();
        assert_eq!(app.project().layers[0].pixels.read(0,-18,-17),[100,20,30,255]);
    }

    #[test]
    fn brush_grows_moved_layer_and_undo_removes_new_tile() {
        let mut app = document();
        app.project_mut().layers[0].pixels.offset = (-100,0);
        app.stroke_capture = Some(EditCapture::new(0,10,10));
        app.brush_size = 1.0;
        app.brush_opacity = 1.0;
        app.brush_color = [1.0,0.0,0.0,1.0];
        app.stroke_marks = vec![0;100];
        app.stroke_id = 1;
        app.apply_brush_segment((5.5,5.5),(5.5,5.5),10,10,false);
        let pixels = &app.project().layers[0].pixels;
        assert_eq!(pixels.tiles.len(),2);
        assert_eq!(pixels.read(0,5,5),[255,0,0,255]);
        assert_eq!(pixels.read(0,-98,3),[100,20,30,255]);
        let edit = app.stroke_capture.take().unwrap().finish(&mut app.project_mut().layers[0].pixels).unwrap();
        app.history.push(edit);
        app.undo();
        assert_eq!(app.project().layers[0].pixels.tiles.len(),1);
        app.redo();
        assert_eq!(app.project().layers[0].pixels.read(0,5,5),[255,0,0,255]);
    }

    #[test]
    fn whole_layer_scale_preserves_sparse_gaps_and_offcanvas_pixels() {
        let mut app = document();
        app.project_mut().layers[0].pixels.write(0,100_000,3,[10,200,30,255]);
        app.begin_transform();
        let floating = app.floating.as_mut().unwrap();
        assert!(floating.pixels.is_empty());
        floating.scale = Vec2::splat(2.0);
        app.commit_transform();
        let raster = &app.project().layers[0].pixels;
        assert!(raster.tiles.len() <= 8);
        assert!(raster.bounds().unwrap().0 < 0);
        assert!(raster.bounds().unwrap().2 > 100_000);
        app.undo();
        assert_eq!(app.project().layers[0].pixels.read(0,100_000,3),[10,200,30,255]);
    }
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
    use crate::raster::SparsePixels;

    #[test]
    fn invert_selection_complements_mask_and_does_nothing_without_selection() {
        let mut app = EditorApp::default();
        app.selection_mask_dirty = false;
        let status = app.status.clone();
        app.invert_selection();
        assert!(app.selection.is_none());
        assert!(!app.selection_mask_dirty);
        assert_eq!(app.status, status);
        app.project_mut().import_rgba(&[1, 2, 3, 255].repeat(6), 3, 2).unwrap();
        let pixels = app.project().layers[0].pixels.clone();
        app.selection = Some(Selection { width: 3, height: 2, mask: vec![1, 0, 1, 0, 0, 1] });
        app.invert_selection();
        assert_eq!(app.selection.as_ref().unwrap().mask, [0, 1, 0, 1, 1, 0]);
        assert!(app.selection_mask_dirty);
        app.invert_selection();
        assert_eq!(app.selection.as_ref().unwrap().mask, [1, 0, 1, 0, 0, 1]);
        app.selection = Some(Selection::all(3, 2));
        app.invert_selection();
        assert!(app.selection.as_ref().unwrap().is_empty());
        app.invert_selection();
        assert_eq!(app.selection.as_ref().unwrap().mask, [1; 6]);
        assert_eq!(app.project().layers[0].pixels, pixels);
    }

    #[test]
    fn layer_clipboard_moves_between_documents_without_clipping_and_undoes() {
        use crate::raster::Raster;
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[10, 20, 30, 255].repeat(16), 4, 4).unwrap();
        let mut pixels = SparsePixels::default();
        pixels.offset = (-10, 20);
        pixels.write(0, -3, -5, [1, 2, 3, 128]);
        pixels.write(0, 1000, 800, [4, 5, 6, 255]);
        let source = &mut app.project_mut().layers[0];
        source.pixels = pixels.clone();
        source.name = "Reference".into();
        source.visible = false;
        source.opacity = 0.25;
        let text = app.copy_layer_text(0).unwrap();
        let index = app.workspace.create_document("Target");
        app.switch_document(index);
        app.project_mut().import_rgba(&[80, 90, 100, 255], 1, 1).unwrap();
        app.refresh_image().unwrap();
        let destination = app.project().layers[0].pixels.clone();
        app.insert_copied_layer(crate::layer_clipboard::decode(&text).unwrap().unwrap());
        assert_eq!(app.image_size, Some((1, 1)));
        assert_eq!(app.project().active_layer, 1);
        let pasted = &app.project().layers[1];
        assert_eq!(pasted.name, "Reference");
        assert!(!pasted.visible);
        assert_eq!(pasted.opacity, 0.25);
        assert_eq!(pasted.pixels, pixels);
        assert_eq!(app.project().layers[0].pixels, destination);
        app.undo();
        assert_eq!(app.project().layers.len(), 1);
        assert_eq!(app.project().active_layer, 0);
        app.redo();
        assert_eq!(app.project().layers.len(), 2);
        assert_eq!(app.project().layers[1].pixels, pixels);
        app.switch_document(0);
        assert_eq!(app.project().layers.len(), 1);
        assert_eq!(app.project().layers[0].pixels, pixels);
    }

    #[test]
    fn pasting_layer_into_empty_document_restores_source_canvas_and_undoes() {
        let mut source = EditorApp::default();
        source.project_mut().import_rgba(&[1, 2, 3, 255].repeat(8), 4, 2).unwrap();
        let text = source.copy_layer_text(0).unwrap();
        let mut target = EditorApp::default();
        target.insert_copied_layer(crate::layer_clipboard::decode(&text).unwrap().unwrap());
        assert_eq!(target.image_size, Some((4, 2)));
        assert_eq!(target.project().layers.len(), 1);
        target.undo();
        assert!(target.project().layers.is_empty());
        assert!(target.image_size.is_none());
        assert_eq!((target.project().width, target.project().height), (0, 0));
        target.redo();
        assert_eq!(target.image_size, Some((4, 2)));
        assert_eq!(target.project().layers[0].pixels, source.project().layers[0].pixels);
    }

    #[test]
    fn invert_layer_preserves_alpha_other_layers_and_supports_undo_redo() {
        let mut app = EditorApp::default();
        app.invert_layer();
        assert!(!app.dirty);
        app.project_mut().import_rgba(&[10, 20, 30, 255], 1, 1).unwrap();
        app.project_mut().add_layer();
        app.invert_layer();
        assert!(!app.dirty);
        let bytes = [0, 127, 255, 255, 10, 20, 30, 128, 40, 50, 60, 0];
        let mut pixels = SparsePixels::from_rgba(&bytes, 3, 1);
        pixels.offset = (-2, 1000);
        app.project_mut().layers[1].pixels = pixels.clone();
        app.project_mut().layers[1].visible = false;
        app.selection = Some(Selection::all(1, 1));
        app.invert_layer();
        let inverted = app.project().layers[1].pixels.clone();
        assert_eq!(inverted.region(-2, 1000, 3, 1),
            [255, 128, 0, 255, 245, 235, 225, 128, 40, 50, 60, 0]);
        assert_eq!(inverted.offset, pixels.offset);
        assert_eq!(inverted.tiles.len(), pixels.tiles.len());
        assert_eq!(app.project().layers[0].pixels.to_rgba(1, 1), [10, 20, 30, 255]);
        assert!(app.dirty);
        assert!(app.layers_reload);
        app.undo();
        assert_eq!(app.project().layers[1].pixels, pixels);
        app.redo();
        assert_eq!(app.project().layers[1].pixels, inverted);
        app.invert_layer();
        assert_eq!(app.project().layers[1].pixels, pixels);
    }

    #[test]
    fn fix_alpha_cleans_only_active_layer_and_supports_undo_redo() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[10, 20, 30, 4], 1, 1).unwrap();
        app.project_mut().add_layer();
        let bytes = [10, 20, 30, 0, 40, 50, 60, 1, 70, 80, 90, 4, 100, 110, 120, 5, 130, 140, 150, 255];
        let mut pixels = SparsePixels::from_rgba(&bytes, 5, 1);
        pixels.offset = (-2, 1000);
        app.project_mut().layers[1].pixels = pixels.clone();
        app.project_mut().layers[1].visible = false;
        app.selection = Some(Selection::all(1, 1));
        app.fix_alpha();
        let fixed = app.project().layers[1].pixels.clone();
        let mut expected = bytes;
        expected[4..12].fill(0);
        assert_eq!(fixed.region(-2, 1000, 5, 1), expected);
        assert_eq!(fixed.offset, pixels.offset);
        assert_eq!(app.project().layers[0].pixels.to_rgba(1, 1), [10, 20, 30, 4]);
        assert!(app.dirty);
        assert!(app.ai.job.is_none());
        app.fix_alpha(); // No change must not create a second Undo step.
        app.undo();
        assert_eq!(app.project().layers[1].pixels, pixels);
        app.redo();
        assert_eq!(app.project().layers[1].pixels, fixed);
    }

    #[test]
    fn fix_alpha_prunes_empty_tiles_and_leaves_clean_document_unchanged() {
        let mut app = EditorApp::default();
        app.fix_alpha();
        app.project_mut().import_rgba(&[10, 20, 30, 5], 1, 1).unwrap();
        app.dirty = false;
        app.fix_alpha();
        assert!(!app.dirty);
        app.project_mut().layers[0].pixels = SparsePixels::from_rgba(&[10, 20, 30, 4], 1, 1);
        let original = app.project().layers[0].pixels.clone();
        app.fix_alpha();
        assert!(app.project().layers[0].pixels.tiles.is_empty());
        app.undo();
        assert_eq!(app.project().layers[0].pixels, original);
    }

    #[test]
    fn layer_source_crops_alpha_with_padding_in_every_image_workflow() {
        use crate::raster::Raster;
        for mode in [AiMode::Edit, AiMode::Redraw, AiMode::Inpaint, AiMode::Lineart, AiMode::DrawCn, AiMode::Upscale] {
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&vec![0; 1000 * 1000 * 4], 1000, 1000).unwrap();
            let mut pixels = SparsePixels::from_rgba(&[20, 40, 60, 128].repeat(200 * 200), 200, 200);
            pixels.offset = (300, 400);
            pixels.write(0, 5, 5, [255, 0, 0, 0]);
            app.project_mut().layers[0].pixels = pixels;
            app.project_mut().layers[0].visible = false;
            app.ai.mode = mode;
            app.ai.layer_padding = 12;
            app.ai.scale = 0.5;
            let (image, region) = app.ai_input().unwrap();
            assert_eq!(image.dimensions(), (224, 224));
            assert_eq!((region.min_x, region.min_y), (288, 388));
            assert_eq!(image.get_pixel(0, 0).0, [0; 4]);
            assert_eq!(image.get_pixel(12, 12).0, [20, 40, 60, 128]);
            assert_eq!(image.get_pixel(211, 211).0, [20, 40, 60, 128]);
            assert_eq!(image.get_pixel(212, 212).0, [0; 4]);
            app.ai.layer_padding = 0;
            assert_eq!(app.ai_input().unwrap().0.dimensions(), (200, 200));
            app.ai.layer_canvas_size = true;
            app.ai.layer_padding = -1; // Canvas mode ignores the padding setting.
            let (image, region) = app.ai_input().unwrap();
            assert_eq!(image.dimensions(), (1000, 1000));
            assert_eq!((region.min_x, region.min_y), (0, 0));
            assert_eq!(image.get_pixel(300, 400).0, [20, 40, 60, 128]);
        }
    }

    #[test]
    fn cropped_layer_mask_and_accepted_result_keep_negative_origin_and_padding() {
        for fit in [true, false] {
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&[0; 4].repeat(16), 4, 4).unwrap();
            let mut pixels = SparsePixels::from_rgba(&[20, 40, 60, 128].repeat(6), 2, 3);
            pixels.offset = (-1, -2);
            app.project_mut().layers[0].pixels = pixels;
            app.ai.mode = AiMode::Inpaint;
            app.ai.layer_padding = 2;
            app.selection = Some(Selection::all(4, 4));
            let (image, region) = app.ai_input().unwrap();
            assert_eq!(image.dimensions(), (6, 7));
            assert_eq!((region.min_x, region.min_y), (-3, -4));
            let mask = app.ai_mask().unwrap();
            assert_eq!(mask.dimensions(), image.dimensions());
            for (x, y, pixel) in mask.enumerate_pixels() {
                assert_eq!(pixel.0, if x >= 3 && y >= 4 { [255; 4] } else { [0, 0, 0, 255] });
            }
            app.ai.input_region = Some(region);
            app.ai.document_id = app.workspace.active_doc().id.clone();
            app.ai.fit_image = fit;
            app.ai.result = Some(image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255])));
            app.accept_ai();
            assert_eq!(app.project().layers.len(), 2);
            let inserted = &app.project().layers[1].pixels;
            assert_eq!(inserted.offset, (-3, -4));
            assert_eq!(inserted.bounds(), Some(if fit { (-3, -4, 3, 3) } else { (-3, -4, -1, -2) }));
        }
    }

    #[test]
    fn empty_layers_and_invalid_padding_are_rejected_before_packing() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[0; 4].repeat(16), 4, 4).unwrap();
        assert!(app.ai_input().is_err());
        app.ai.layer_canvas_size = true;
        assert_eq!(app.ai_input().unwrap().0.dimensions(), (4, 4));
        app.project_mut().layers[0].pixels = SparsePixels::from_rgba(&[1, 2, 3, 1], 1, 1);
        app.ai.layer_canvas_size = false;
        for padding in [-1, 100000, i32::MAX] {
            app.ai.layer_padding = padding;
            assert!(app.ai_input().is_err());
        }
    }

    #[test]
    fn asset_preview_is_read_only_and_switching_back_preserves_document_state() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[10, 20, 30, 128].repeat(16), 4, 4).unwrap();
        app.refresh_image().unwrap();
        app.selection = Some(Selection::all(4, 4));
        let asset = app.workspace.import_asset_rgba("preview".into(), &[255, 0, 0, 255].repeat(6), 3, 2).unwrap();
        app.pan = Vec2::new(12.0, 34.0);
        app.zoom = 2.0;
        app.rotation = 37.0;
        app.show_asset_preview(asset);
        assert_eq!(app.display_size(), Some((3, 2)));
        let view = app.canvas_view().unwrap();
        app.input.mouse_pressed = true;
        app.input.mouse_down = true;
        app.input.mouse_pos = view.image_to_screen(Vec2::splat(0.5));
        app.handle_canvas_edit(view);
        assert_eq!(app.project().layers[0].pixels.to_rgba(app.project().width, app.project().height), [10, 20, 30, 128].repeat(16));
        assert_eq!(app.workspace.assets[asset].pixels, [255, 0, 0, 255].repeat(6));
        app.switch_document(0);
        assert_eq!(app.display_size(), Some((4, 4)));
        assert_eq!((app.pan, app.zoom, app.rotation), (Vec2::new(12.0, 34.0), 2.0, 37.0));
        assert_eq!(app.selection.as_ref().unwrap().mask, vec![1; 16]);
        assert!(app.layers_reload && app.selection_mask_dirty);
    }

    #[test]
    fn inpaint_softness_blurs_mask_without_changing_selection_or_alpha() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[1, 2, 3, 255].repeat(9 * 9), 9, 9).unwrap();
        app.selection = Some(Selection::from_rect(PixelRect { min_x: 3, min_y: 3, max_x: 6, max_y: 6 }, 9, 9));
        let original = app.selection.as_ref().unwrap().mask.clone();
        let sharp = app.ai_mask().unwrap();
        assert_eq!(sharp.get_pixel(2, 4).0, [0, 0, 0, 255]);
        app.ai.mask_softness = 1.5;
        let soft = app.ai_mask().unwrap();
        assert!(soft.get_pixel(2, 4)[0] > 0);
        assert!(soft.get_pixel(4, 4)[0] < 255);
        assert!(soft.pixels().all(|p| p[3] == 255 && p[0] == p[1] && p[1] == p[2]));
        assert_eq!(app.selection.as_ref().unwrap().mask, original);
    }

    #[test]
    fn selection_tools_handle_rotated_view_modifiers_and_release_outside_canvas() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[0, 0, 0, 255].repeat(100), 10, 10).unwrap();
        app.refresh_image().unwrap();
        app.canvas_rect = Rect::from_min_size(Vec2::ZERO, Vec2::splat(200.0));
        app.rotation = 45.0;
        app.zoom = 4.0;
        app.set_tool(Tool::Lasso);
        let view = app.canvas_view().unwrap();
        for (i, point) in [Vec2::new(1.0, 1.0), Vec2::new(5.0, 1.0), Vec2::new(1.0, 5.0)].into_iter().enumerate() {
            app.input = FrameInput::default();
            app.input.mouse_pressed = i == 0;
            app.input.mouse_down = true;
            app.input.mouse_pos = view.image_to_screen(point);
            app.handle_canvas_edit(view);
        }
        app.input.mouse_down = false;
        app.input.mouse_released = true;
        app.handle_canvas_edit(view);
        assert!(app.selection.as_ref().unwrap().contains(1, 1));
        assert!(!app.selection.as_ref().unwrap().contains(4, 4));
        app.set_tool(Tool::SelectionBrush);
        app.brush_size = 1.0;
        app.input = FrameInput::default();
        app.input.mouse_pressed = true;
        app.input.mouse_down = true;
        app.input.mouse_pos = view.image_to_screen(Vec2::new(8.5, 8.5));
        app.handle_canvas_edit(view);
        assert!(app.selection.as_ref().unwrap().contains(1, 1));
        assert!(app.selection.as_ref().unwrap().contains(8, 8));
        app.selection_brush_erase = true;
        app.handle_canvas_edit(view);
        assert!(!app.selection.as_ref().unwrap().contains(8, 8));
        app.input = FrameInput::default();
        app.input.mouse_released = true;
        app.input.mouse_pos = Vec2::splat(300.0);
        app.handle_canvas_edit(view);
        assert!(!app.brush_last_active());
        assert!(app.project().layers[0].pixels.to_rgba(app.project().width, app.project().height).chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
    }

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
            app.project_mut().layers[1].pixels = SparsePixels::from_rgba(&[255, 0, 0, 255].repeat(6 * 5), 6, 5);
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
            assert_eq!(region, bounds.into());
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
    fn input_workflows_upload_selected_source_and_accept_as_new_layer() {
        use std::{io::{BufRead, BufReader, Read, Write}, net::TcpListener, time::{Duration, Instant}};
        for (mode, source) in [(AiMode::AnimeRedraw, AiSource::Layer),
            (AiMode::AnimeRedraw, AiSource::Document), (AiMode::AnimeRedraw, AiSource::Selection),
            (AiMode::Redraw, AiSource::Layer), (AiMode::Redraw, AiSource::Document),
            (AiMode::Redraw, AiSource::Selection), (AiMode::Inpaint, AiSource::Layer), (AiMode::Inpaint, AiSource::Document),
            (AiMode::Lineart, AiSource::Layer), (AiMode::Lineart, AiSource::Document), (AiMode::Lineart, AiSource::Selection),
            (AiMode::DrawCn, AiSource::Layer), (AiMode::DrawCn, AiSource::Document), (AiMode::DrawCn, AiSource::Selection),
            (AiMode::Upscale, AiSource::Layer), (AiMode::Upscale, AiSource::Document), (AiMode::Upscale, AiSource::Selection)] {
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
                            let title = if mode == AiMode::AnimeRedraw { "LoadImage" } else if matches!(mode, AiMode::DrawCn | AiMode::Upscale) { "ReferenceImage" } else { "Image" };
                            let image = graph.values().find(|n| n["_meta"]["title"] == title).unwrap();
                            assert_eq!(image["inputs"]["image"], "test/source.png");
                            if mode == AiMode::Lineart {
                                let lineart = graph.values().find(|n| n["class_type"] == "AnimeLineArtPreprocessor").unwrap();
                                assert_eq!(lineart["inputs"]["resolution"], 1024);
                                assert_eq!(graph.len(), 3);
                            } else if matches!(mode, AiMode::DrawCn | AiMode::Upscale) {
                                let cn = graph.values().find(|n| n["_meta"]["title"] == "ControlNet").unwrap();
                                assert_eq!(cn["inputs"]["strength"], 0.5);
                                assert_eq!(cn["inputs"]["start_percent"], 0.25);
                                assert_eq!(cn["inputs"]["end_percent"], 0.75);
                                if mode == AiMode::Upscale {
                                    let upscale = graph.values().find(|n| n["class_type"] == "ImageScaleBy").unwrap();
                                    assert_eq!(upscale["inputs"]["scale_by"], 2.5);
                                    let sampler = graph.values().find(|n| n["_meta"]["title"] == "Sampler").unwrap();
                                    assert_eq!(sampler["inputs"]["denoise"], 0.5);
                                    assert!(graph.values().any(|n| n["class_type"] == "DepthAnythingV2Preprocessor"));
                                } else {
                                    let latent = graph.values().find(|n| n["class_type"] == "EmptyLatentImage").unwrap();
                                    assert!(latent["inputs"]["width"].is_array());
                                    assert!(latent["inputs"]["height"].is_array());
                                }
                            } else {
                                let sampler = graph.values().find(|n| n["_meta"]["title"] == "Sampler").unwrap();
                                assert_eq!(sampler["inputs"]["denoise"], 0.5);
                                assert!(graph.values().any(|n| n["class_type"] == "VAEEncode"));
                            }
                            if mode == AiMode::Inpaint {
                                let mask = graph.values().find(|n| n["_meta"]["title"] == "Mask").unwrap();
                                assert_eq!(mask["inputs"]["image"], "test/mask.png");
                                assert_eq!(mask["inputs"]["channel"], "red");
                                assert!(graph.values().any(|n| n["class_type"] == "SetLatentNoiseMask"));
                            }
                            br#"{"prompt_id":"job"}"#.to_vec()
                        }
                        "/history/job" => {
                            let save = if mode == AiMode::Lineart { "3" } else if mode == AiMode::AnimeRedraw { "62" } else { "12" };
                            serde_json::to_vec(&serde_json::json!({"job": {"outputs": {save: {"images": [{"filename": "result.png"}]}}, "status": {"completed": true}}})).unwrap()
                        }
                        _ => crate::project::encode_png_rgba(&[1, 2, 3, 255].repeat(64 * 64), 64, 64).unwrap(),
                    };
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).unwrap();
                    stream.write_all(&response).unwrap();
                }
            });
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&[10, 20, 30, 255].repeat(128 * 128), 128, 128).unwrap();
            app.project_mut().add_layer();
            app.project_mut().layers[1].pixels = SparsePixels::from_rgba(&[200, 100, 50, 255].repeat(128 * 128), 128, 128);
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
            app.ai.lineart_resolution = 1024;
            app.ai.control_net = crate::comfyui::ControlNetSettings {
                strength: 0.5, start_percent: 0.25, end_percent: 0.75,
            };
            app.ai.upscale = crate::comfyui::UpscaleSettings {
                scale_by: 2.5, denoise: 0.5, control: app.ai.control_net,
            };
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
            assert_eq!(&app.project().layers[1].pixels.to_rgba(app.project().width, app.project().height)[..4], &[200, 100, 50, 255]);
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
            let inserted = &app.project().layers[2].pixels.to_rgba(app.project().width, app.project().height);
            if source == AiSource::Selection {
                for y in 0..128 {
                    for x in 0..128 {
                        let index = (y * 128 + x) * 4;
                        assert_eq!(&inserted[index..index + 4], if (16..112).contains(&x) && (24..104).contains(&y) { &[1, 2, 3, 255] } else { &[0, 0, 0, 0] });
                    }
                }
            } else { assert_eq!(&inserted[..4], &[1, 2, 3, 255]); }
            assert!(app.ai.open);
        }
    }

    #[test]
    fn draw_on_empty_document_waits_for_acceptance_then_creates_canvas() {
        for mode in [AiMode::Draw, AiMode::AnimeDraw, AiMode::QwenDraw] {
            let mut app = EditorApp::default();
            app.ai.open = true;
            app.ai.mode = mode;
            assert!(app.project().layers.is_empty());
            app.ai.document_id = app.workspace.active_doc().id.clone();
            app.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: 64, max_y: 128 }.into());
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
            assert_eq!(app.project().layers[0].name, format!("{} 1", mode.layer_prefix()));
            assert_eq!(&app.project().layers[0].pixels.to_rgba(app.project().width, app.project().height)[..4], &[1, 2, 3, 255]);
            assert!(app.ai.open);
            app.accept_ai();
            assert_eq!(app.project().layers.len(), 1);
        }
    }

    #[test]
    fn completed_job_waits_for_acceptance_and_adds_exactly_once() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[20, 40, 60, 255], 1, 1).unwrap();
        app.refresh_image().unwrap();
        app.ai.open = true;
        app.ai.document_id = app.workspace.active_doc().id.clone();
        app.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: 1, max_y: 1 }.into());
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
        assert!(app.ai.open);
        assert!(app.ai.result.is_none());
        assert_eq!(app.ai.preview.as_ref(), Some(&result));
        assert_eq!(app.image_size, Some((1, 1)));
        assert_eq!(app.project().active_layer, 1);
        assert_eq!(app.project().layers[1].pixels.to_rgba(app.project().width, app.project().height), [100, 120, 140, 255]);
        assert_eq!(&app.project().layers[0].pixels.to_rgba(app.project().width, app.project().height)[..4], &[20, 40, 60, 255]);
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
    fn hung_generation_dialog_closes_without_waiting_for_the_worker() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[20,40,60,255],1,1).unwrap();
        app.refresh_image().unwrap();
        let original = app.project().layers[0].pixels.clone();
        let (job,sender) = crate::comfyui::Job::pending();
        assert!(job.poll().is_none());
        app.ai.open = true;
        app.ai.job = Some(job);
        app.close_ai();
        assert!(!app.ai.open,"dialog close must not depend on a network response");
        assert!(app.ai.job.is_none());
        assert!(sender.send(crate::comfyui::JobResult::Done(image::RgbaImage::new(1,1))).is_err(),
            "late result must be detached from the editor");
        assert_eq!(app.project().layers[0].pixels,original);
        assert!(!app.dirty);
    }

    #[test]
    fn hung_generation_does_not_prevent_application_exit() {
        let mut app = EditorApp::default();
        let (job,_sender) = crate::comfyui::Job::pending();
        app.ai.job = Some(job);
        assert!(app.request_close(),"window close must not wait for a stalled worker");
        assert!(app.ai.job.is_none());
    }

    #[test]
    fn cancelling_hung_generation_frees_dialog_and_rejects_late_results() {
        let mut app = EditorApp::default();
        let (job,old_sender) = crate::comfyui::Job::pending();
        app.ai.open = true;
        app.ai.job = Some(job);
        app.cancel_ai();
        assert!(app.ai.open && app.ai.job.is_none());
        let (replacement,_new_sender) = crate::comfyui::Job::pending();
        app.ai.job = Some(replacement);
        assert!(old_sender.send(crate::comfyui::JobResult::Done(image::RgbaImage::new(1,1))).is_err());
        app.poll_ai(&mut Ui::new());
        assert!(app.ai.job.is_some() && app.ai.result.is_none());
    }

    #[test]
    fn fit_uses_unscaled_input_bounds_and_can_be_disabled() {
        for fit in [true, false] {
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&vec![20, 40, 60, 255].repeat(8), 4, 2).unwrap();
            app.refresh_image().unwrap();
            app.ai.open = true;
            app.ai.document_id = app.workspace.active_doc().id.clone();
            app.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: 4, max_y: 2 }.into());
            app.ai.scale = 0.5;
            app.ai.fit_image = fit;
            app.ai.result = Some(image::RgbaImage::from_pixel(2, 1, image::Rgba([100, 120, 140, 255])));
            app.accept_ai();
            assert_eq!(app.image_size, Some((4, 2)));
            assert_eq!(app.project().layers.len(), 2);
            let inserted = &app.project().layers[1].pixels.to_rgba(app.project().width, app.project().height);
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
        app.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: 1, max_y: 1 }.into());
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
        assert_eq!(app.project().layers[1].pixels.to_rgba(app.project().width, app.project().height), [80, 0, 0, 255]);
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
        assert_eq!(app.project().layers[0].pixels.to_rgba(app.project().width, app.project().height), [20, 40, 60, 255]);
    }
}

#[cfg(test)]
mod layer_history_tests {
    use super::*;

    #[test]
    fn merge_down_restores_both_layers_and_prior_edits_through_undo_redo() {
        let mut app = app_with_layers(3);
        paint_pixel(&mut app, 2, 80);
        let before = app.project().layers.clone();
        app.project_mut().active_layer = 2;
        app.merge_layer_down();
        assert_eq!(app.project().layers.len(), 2);
        assert_eq!(app.project().active_layer, 1);
        assert_eq!(app.project().manifest.layers.len(), 2);
        assert_eq!(app.project().layers[0].pixels, before[0].pixels);
        assert!(app.layers_reload && app.dirty);
        let merged = app.project().layers[1].pixels.clone();
        app.undo();
        assert_eq!(app.project().layers.len(), 3);
        assert_eq!(app.project().active_layer, 2);
        for (restored, original) in app.project().layers.iter().zip(&before) {
            assert_eq!(restored.name, original.name);
            assert_eq!(restored.opacity, original.opacity);
            assert_eq!(restored.visible, original.visible);
            assert_eq!(restored.pixels, original.pixels);
        }
        app.undo();
        app.redo();
        app.redo();
        assert_eq!(app.project().layers.len(), 2);
        assert_eq!(app.project().layers[1].pixels, merged);
        app.undo();
        paint_pixel(&mut app, 2, 90);
        app.redo();
        assert_eq!(app.project().layers.len(), 3);
    }

    #[test]
    fn merge_down_does_nothing_for_empty_document_or_bottom_layer() {
        let mut app = EditorApp::default();
        app.merge_layer_down();
        assert!(app.project().layers.is_empty());
        let mut app = app_with_layers(2);
        app.project_mut().active_layer = 0;
        let before = app.project().layers[0].pixels.clone();
        app.merge_layer_down();
        assert_eq!(app.project().layers.len(), 2);
        assert_eq!(app.project().layers[0].pixels, before);
    }
    use crate::raster::{Raster, SparsePixels};

    fn app_with_layers(count: usize) -> EditorApp {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[1, 2, 3, 255].repeat(8), 4, 2).unwrap();
        for _ in 1..count { app.project_mut().add_layer(); }
        for (i, layer) in app.project_mut().layers.iter_mut().enumerate() {
            layer.name = format!("Layer {i}");
            layer.visible = i % 2 == 0;
            layer.opacity = 0.25 * (i + 1) as f32;
            layer.pixels = SparsePixels::from_rgba(&vec![(i + 1) as u8; 4 * 2 * 4], 4, 2);
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
        let mut pixel = p.layers[layer].pixels.read(p.width, 0, 0);
        pixel[0] = value;
        p.layers[layer].pixels.write(p.width, 0, 0, pixel);
        let edit = capture.finish(&mut p.layers[layer].pixels).unwrap();
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
        assert_eq!(app.project().layers[0].pixels.read(app.project().width, 0, 0)[0], 99);
        app.undo();
        assert_eq!(app.project().layers[0].pixels.read(app.project().width, 0, 0)[0], 1);
        app.redo();
        assert_eq!(app.project().layers[0].pixels.read(app.project().width, 0, 0)[0], 99);
        app.redo();
        assert!(app.image_size.is_none());
        app.undo();
        assert_eq!(app.project().layers[0].pixels.read(app.project().width, 0, 0)[0], 99);
    }

    #[test]
    fn edits_before_and_after_deletion_target_correct_layers_and_new_edit_clears_redo() {
        let mut app = app_with_layers(3);
        paint_pixel(&mut app, 2, 80);
        app.delete_layer(0);
        paint_pixel(&mut app, 1, 90);
        app.project_mut().active_layer = 0;
        app.undo();
        assert_eq!(app.project().layers[1].pixels.read(app.project().width, 0, 0)[0], 80);
        assert_eq!(app.dirty_layer, Some(1));
        app.undo();
        assert_eq!(app.project().layers.len(), 3);
        app.undo();
        assert_eq!(app.project().layers[2].pixels.read(app.project().width, 0, 0)[0], 3);
        app.redo();
        app.redo();
        app.redo();
        assert_eq!(app.project().layers[1].pixels.read(app.project().width, 0, 0)[0], 90);
        app.undo();
        paint_pixel(&mut app, 1, 100);
        app.redo();
        assert_eq!(app.project().layers[1].pixels.read(app.project().width, 0, 0)[0], 100);
        app.delete_layer(1);
        app.undo();
        app.add_layer();
        app.redo();
        assert_eq!(app.project().layers.len(), 3);
    }
}
