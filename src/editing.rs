mod brush;
mod composite;
mod fill;
mod sample;
mod selection;
mod transform;

#[allow(unused_imports)]
pub use brush::{apply_brush, BrushStroke, SoftBrushCoverage};
#[allow(unused_imports)]
pub use composite::{composite_layers, composite_pixel, composite_region, merge_layers};
#[allow(unused_imports)]
pub use fill::{fill_bucket, flood_mask};
#[allow(unused_imports)]
pub use sample::{color_distance, resize_rgba, sample_bilinear, sample_mask_bilinear, sample_pixel};
#[allow(unused_imports)]
pub use selection::{brush_dirty_bounds, PixelRect, SelectMode, Selection};
#[allow(unused_imports)]
pub use transform::{FloatingTransform, TransformCursor, TransformHit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Brush,
    RectangleSelect,
    Lasso,
    SelectionBrush,
    MagicWand,
    Fill,
    Transform,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Brush => "Brush",
            Tool::RectangleSelect => "Select",
            Tool::Lasso => "Lasso",
            Tool::SelectionBrush => "Select Brush",
            Tool::MagicWand => "Wand",
            Tool::Fill => "Fill",
            Tool::Transform => "Transform",
        }
    }

    pub fn all() -> [Tool; 7] {
        [
            Tool::Brush,
            Tool::RectangleSelect,
            Tool::Lasso,
            Tool::SelectionBrush,
            Tool::MagicWand,
            Tool::Fill,
            Tool::Transform,
        ]
    }
}
