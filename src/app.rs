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

mod ai;
mod video;
mod documents;
mod history;
mod layers;
mod scene;
mod session;
mod tools;
mod transform;

pub use session::DocumentSession;

#[cfg(test)]
mod ai_tests;
#[cfg(test)]
mod layer_history_tests;
#[cfg(test)]
mod loading_tests;
#[cfg(test)]
mod sparse_layer_tests;
#[cfg(test)]
mod transform_interaction_tests;

pub struct EditorApp {
    pub workspace: Workspace,
    pub input: FrameInput,
    pub image_size: Option<(u32, u32)>,
    pub dirty_tiles: DirtyTiles,
    /// Layer index that `dirty_tiles` refer to.
    dirty_layer: Option<usize>,
    layers_reload: bool,
    floating_reload: bool,
    pub session: DocumentSession,
    pub dirty: bool,
    pub status: String,
    /// Smoothed redraw rate. Idle gaps between frames are not included.
    pub fps: f32,
    pub dock: DockState,
    brush_last: Option<(f32, f32)>,
    brush_dirty: bool,
    stroke_marks: Vec<u32>,
    soft_brush_coverage: SoftBrushCoverage,
    stroke_id: u32,
    stroke_capture: Option<EditCapture>,
    history: History,
    pub layer_drag: Option<usize>,
    canvas_cleared: bool,
    pub resize_w: u32,
    pub resize_h: u32,
    pub ai: ui::ai_dialog::AiDialog,
    pub hsv: ui::hsv_dialog::HsvDialog,
    video_preview: Option<crate::video::Playback>,
    video_poster: Option<crate::video::PosterJob>,
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
            session: DocumentSession::default(),
            dirty: false,
            status: "Import assets, place them on a document, or resize canvas".into(),
            fps: 0.0,
            dock: ui::default_dock(),
            brush_last: None,
            brush_dirty: false,
            stroke_marks: Vec::new(),
            soft_brush_coverage: SoftBrushCoverage::default(),
            stroke_id: 1,
            stroke_capture: None,
            history: History::default(),
            layer_drag: None,
            canvas_cleared: true,
            resize_w: 512,
            resize_h: 512,
            ai: ui::ai_dialog::AiDialog::default(),
            hsv: Default::default(),
            video_preview: None,
            video_poster: None,
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
        let edit = crate::history::HistoryEdit::from_diff(index, &before, &layer.pixels);
        if let Some(edit) = edit { self.history.push(edit); }
        self.layers_reload = true;
        self.dirty = true;
        self.workspace.mark_active_thumb_dirty();
        self.status = "HSV adjustment applied".into();
    }
    pub fn project(&self) -> &Project {
        self.workspace.active()
    }

    pub fn project_mut(&mut self) -> &mut Project {
        self.workspace.active_mut()
    }

    pub fn brush_last_active(&self) -> bool {
        self.brush_last.is_some() || self.session.transform_drag.is_some()
            || !self.session.lasso_points.is_empty() || self.session.selection_brush_last.is_some()
    }

    pub fn display_size(&self) -> Option<(u32, u32)> {
        if let Some(player) = &self.video_preview {
            return player.frame.as_ref().map(image::RgbaImage::dimensions);
        }
        if let Some(id) = &self.session.preview_asset {
            return self.workspace.assets.iter().find(|a| &a.id == id).map(|a| (a.width, a.height));
        }
        self.image_size
    }

    pub fn show_asset_preview(&mut self, index: usize) {
        if let Err(error) = self.workspace.ensure_asset_loaded(index) {
            self.status = error;
            return;
        }
        let Some(asset) = self.workspace.assets.get(index) else { return; };
        let id = asset.id.clone();
        self.begin_asset_preview(id);
    }

    fn begin_asset_preview(&mut self, id: String) {
        if self.session.preview_asset.as_ref() == Some(&id) { return; }
        self.video_preview = None;
        self.commit_transform();
        self.brush_last = None;
        self.session.lasso_points.clear();
        self.session.selection_brush_last = None;
        self.session.selection_anchor = None;
        self.session.selection_current = None;
        if self.session.preview_asset.is_none() {
            self.session.preview_document_view = Some((self.session.pan, self.session.zoom, self.session.rotation));
        }
        self.session.preview_asset = Some(id);
        self.session.preview_asset_dirty = true;
        self.session.rotation = 0.0;
        self.session.fit_image = true;
    }

    pub fn close_asset_preview(&mut self) {
        self.video_preview = None;
        if self.session.preview_asset.take().is_some() {
            self.layers_reload = true;
            self.session.selection_mask_dirty = true;
            if let Some((pan, zoom, rotation)) = self.session.preview_document_view.take() {
                self.session.pan = pan;
                self.session.zoom = zoom;
                self.session.rotation = rotation;
            }
            self.session.fit_image = false;
        }
    }

    pub fn canvas_view(&self) -> Option<CanvasView> {
        let (width, height) = self.display_size()?;
        Some(CanvasView {
            center: self.session.canvas_rect.min + rect_size(self.session.canvas_rect) * 0.5 + self.session.pan,
            zoom: self.session.zoom,
            rotation: self.session.rotation.to_radians(),
            width,
            height,
        })
    }

    pub fn update_view(&mut self) {
        if !self.session.canvas_rect.contains(self.input.mouse_pos) {
            return;
        }
        if self.input.mouse_middle_down {
            self.session.pan += self.input.mouse_delta;
        }
        if self.input.scroll_delta.y.abs() > 0.01 {
            let old_zoom = self.session.zoom;
            let factor = (self.input.scroll_delta.y * 0.002).exp();
            self.session.zoom = (self.session.zoom * factor).clamp(0.05, 32.0);
            let center = self.session.canvas_rect.min + rect_size(self.session.canvas_rect) * 0.5;
            let cursor_from_image_center = self.input.mouse_pos - center - self.session.pan;
            self.session.pan -= cursor_from_image_center * (self.session.zoom / old_zoom - 1.0);
        }
    }

    pub(in crate::app) fn note_frame(&mut self, dt: f32) {
        // A gap means the window was idle, not that the next frame was slow.
        if !dt.is_finite() || !(1.0 / 1000.0..0.1).contains(&dt) {
            self.fps = 0.0;
            return;
        }
        let sample = 1.0 / dt;
        self.fps = if self.fps == 0.0 { sample } else { self.fps * 0.85 + sample * 0.15 };
    }

    pub fn fit_to_canvas(&mut self) {
        let Some((width, height)) = self.display_size() else {
            return;
        };
        let available = (rect_size(self.session.canvas_rect) - Vec2::splat(32.0)).max(Vec2::splat(1.0));
        self.session.zoom = (available.x / width as f32)
            .min(available.y / height as f32)
            .clamp(0.05, 1.0);
        self.session.pan = Vec2::ZERO;
        self.session.fit_image = false;
    }

    pub fn clear_selection(&mut self) {
        self.session.clear_selection();
    }

    pub(in crate::app) fn set_selection(&mut self, selection: crate::editing::Selection) {
        self.session.set_selection(selection);
    }

    pub fn invert_selection(&mut self) {
        if self.session.invert_selection() {
            self.status = "Selection inverted".into();
        }
    }

    pub fn set_tool(&mut self, tool: Tool) {
        if self.session.tool == tool {
            return;
        }
        if self.session.tool == Tool::Transform && tool != Tool::Transform {
            self.commit_transform();
        }
        self.commit_selection_drag();
        self.session.lasso_points.clear();
        self.session.selection_brush_last = None;
        self.brush_last = None;
        self.session.tool = tool;
        if tool == Tool::Transform && self.session.preview_asset.is_none() {
            self.begin_transform();
        }
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
        app.session.canvas_rect = Rect::from_min_size(Vec2::ZERO,Vec2::splat(400.0));
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

