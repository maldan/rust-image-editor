use glam::Vec2;
use mega_ui::Rect;

use crate::editing::{
    FloatingTransform, SelectMode, Selection, Tool, TransformCursor, TransformHit,
};

/// View, tool, selection, and transform state for the document on screen.
/// Dock panels edit this session instead of the whole editor.
pub struct DocumentSession {
    pub(crate) canvas_rect: Rect,
    pub(crate) viewport_size: Vec2,
    pub(crate) pan: Vec2,
    pub(crate) zoom: f32,
    pub(crate) rotation: f32,
    pub(crate) fit_image: bool,
    pub(crate) tool: Tool,
    pub(crate) erase_mode: bool,
    pub(crate) brush_size: f32,
    pub(crate) brush_opacity: f32,
    pub(crate) brush_hardness: f32,
    pub(crate) brush_color: [f32; 4],
    pub(crate) wand_threshold: f32,
    pub(crate) fill_threshold: f32,
    pub(crate) dim_outside_selection: bool,
    pub(crate) preview_asset: Option<String>,
    pub(crate) preview_asset_dirty: bool,
    pub(crate) preview_document_view: Option<(Vec2, f32, f32)>,
    pub(crate) selection_brush_erase: bool,
    pub(crate) lasso_points: Vec<Vec2>,
    pub(crate) selection_brush_last: Option<Vec2>,
    pub(crate) selection: Option<Selection>,
    pub(crate) selection_anchor: Option<(u32, u32)>,
    pub(crate) selection_current: Option<(u32, u32)>,
    pub(crate) selection_mode: SelectMode,
    pub(crate) selection_deselect_press: Option<Vec2>,
    pub(crate) selection_mask_dirty: bool,
    pub(crate) floating: Option<FloatingTransform>,
    pub(crate) transform_drag: Option<TransformHit>,
    pub(crate) transform_last: Option<Vec2>,
    pub(crate) transform_keep_aspect_ratio: bool,
    pub(crate) canvas_cursor: Option<TransformCursor>,
}

impl Default for DocumentSession {
    fn default() -> Self {
        Self {
            canvas_rect: Rect::from_min_size(Vec2::ZERO, Vec2::ZERO),
            viewport_size: Vec2::ZERO,
            pan: Vec2::ZERO,
            zoom: 1.0,
            rotation: 0.0,
            fit_image: false,
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
            floating: None,
            transform_drag: None,
            transform_last: None,
            transform_keep_aspect_ratio: true,
            canvas_cursor: None,
        }
    }
}

impl DocumentSession {
    pub(crate) fn clear_selection(&mut self) {
        self.lasso_points.clear();
        self.selection_brush_last = None;
        self.selection = None;
        self.selection_anchor = None;
        self.selection_current = None;
        self.selection_deselect_press = None;
        self.selection_mask_dirty = true;
    }

    pub(crate) fn set_selection(&mut self, selection: Selection) {
        self.selection = Some(selection);
        self.selection_mask_dirty = true;
    }

    /// Returns whether a selection was inverted.
    pub(crate) fn invert_selection(&mut self) -> bool {
        let Some(selection) = self.selection.as_mut() else {
            return false;
        };
        selection.invert();
        self.selection_mask_dirty = true;
        true
    }
}
