pub mod deform;
mod brush;
mod composite;
mod fill;
mod sample;
mod selection;
mod transform;

#[allow(unused_imports)]
pub use brush::{BrushStroke, SoftBrushCoverage, apply_brush};
#[allow(unused_imports)]
pub use composite::{
    composite_layers, composite_pixel, composite_region, merge_layer_tracks, merge_layers,
};
#[allow(unused_imports)]
pub use fill::{fill_bucket, flood_mask};
#[allow(unused_imports)]
pub use sample::{
    color_distance, resize_rgba, sample_bilinear, sample_mask_bilinear, sample_pixel,
};
#[allow(unused_imports)]
pub use selection::{PixelRect, SelectMode, Selection, brush_dirty_bounds};
#[allow(unused_imports)]
pub use transform::{FloatingTransform, TransformCursor, TransformHit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Brush,
    RectangleSelect,
    Lasso,
    PolygonSelect,
    SelectionBrush,
    MagicWand,
    Fill,
    Transform,
    Deform,
}

impl Tool {
    pub const SELECT_MODES: [Tool; 5] = [
        Tool::RectangleSelect,
        Tool::MagicWand,
        Tool::Lasso,
        Tool::PolygonSelect,
        Tool::SelectionBrush,
    ];
    pub const SELECT_MODE_LABELS: [&'static str; 5] =
        ["Rect", "Wand", "Lasso", "Poly", "Brush"];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Brush => "Brush",
            Tool::RectangleSelect => "Select",
            Tool::Lasso => "Lasso",
            Tool::PolygonSelect => "Polygon",
            Tool::SelectionBrush => "Select Brush",
            Tool::MagicWand => "Wand",
            Tool::Fill => "Fill",
            Tool::Transform => "Transform",
            Tool::Deform => "Deform",
        }
    }

    pub fn is_select(self) -> bool {
        matches!(
            self,
            Tool::RectangleSelect
                | Tool::Lasso
                | Tool::PolygonSelect
                | Tool::SelectionBrush
                | Tool::MagicWand
        )
    }

    pub fn select_mode(self) -> Option<usize> {
        Self::SELECT_MODES.iter().position(|&tool| tool == self)
    }

    pub fn toolbar() -> [Tool; 5] {
        [
            Tool::Brush,
            Tool::RectangleSelect,
            Tool::Fill,
            Tool::Transform,
            Tool::Deform,
        ]
    }
}
