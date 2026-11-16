use crate::project::Layer;

/// Editor-local copy, shared across documents without replacing the OS clipboard.
#[derive(Clone)]
pub struct CopiedLayer {
    pub layer: Layer,
    pub canvas_size: (u32, u32),
    pub fragment: bool,
}
