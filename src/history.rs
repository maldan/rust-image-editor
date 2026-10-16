use std::collections::HashSet;

use crate::editing::PixelRect;
use crate::tiles::TILE_SIZE;

const MAX_ENTRIES: usize = 64;

#[derive(Clone)]
struct TileSnapshot {
    tx: u32,
    ty: u32,
    w: u32,
    h: u32,
    data: Vec<u8>,
}

#[derive(Clone)]
pub struct HistoryEdit {
    pub layer: usize,
    before: Vec<TileSnapshot>,
    after: Vec<TileSnapshot>,
}

/// Captures tile snapshots before pixels change, then freezes after on finish.
pub struct EditCapture {
    layer: usize,
    width: u32,
    height: u32,
    touched: HashSet<(u32, u32)>,
    before: Vec<TileSnapshot>,
}

impl EditCapture {
    pub fn new(layer: usize, width: u32, height: u32) -> Self {
        Self {
            layer,
            width,
            height,
            touched: HashSet::new(),
            before: Vec::new(),
        }
    }

    pub fn layer(&self) -> usize {
        self.layer
    }

    /// Snapshot tiles covering `rect` from current `pixels` (call before mutating).
    pub fn touch_rect(&mut self, pixels: &[u8], rect: PixelRect) {
        if rect.is_empty() || self.width == 0 || self.height == 0 {
            return;
        }
        let tx0 = rect.min_x / TILE_SIZE;
        let ty0 = rect.min_y / TILE_SIZE;
        let tx1 = (rect.max_x.saturating_sub(1)) / TILE_SIZE;
        let ty1 = (rect.max_y.saturating_sub(1)) / TILE_SIZE;
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                if !self.touched.insert((tx, ty)) {
                    continue;
                }
                if let Some(snap) = snapshot_tile(pixels, self.width, self.height, tx, ty) {
                    self.before.push(snap);
                }
            }
        }
    }

    pub fn finish(self, pixels: &[u8]) -> Option<HistoryEdit> {
        if self.before.is_empty() {
            return None;
        }
        let after = self
            .before
            .iter()
            .filter_map(|b| snapshot_tile(pixels, self.width, self.height, b.tx, b.ty))
            .collect::<Vec<_>>();
        Some(HistoryEdit {
            layer: self.layer,
            before: self.before,
            after,
        })
    }

    pub fn restore_before(&self, pixels: &mut [u8]) -> Option<PixelRect> {
        apply_tiles(pixels, self.width, self.height, &self.before)
    }
}

#[derive(Default)]
pub struct History {
    undo: Vec<HistoryEdit>,
    redo: Vec<HistoryEdit>,
}

impl History {
    pub fn push(&mut self, edit: HistoryEdit) {
        self.undo.push(edit);
        if self.undo.len() > MAX_ENTRIES {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(
        &mut self,
        layers: &mut [crate::project::Layer],
        width: u32,
        height: u32,
    ) -> Option<(usize, PixelRect)> {
        let edit = self.undo.pop()?;
        let layer = edit.layer;
        let pixels = layers.get_mut(layer).map(|l| l.pixels.as_mut_slice())?;
        let rect = apply_tiles(pixels, width, height, &edit.before)?;
        self.redo.push(edit);
        Some((layer, rect))
    }

    pub fn redo(
        &mut self,
        layers: &mut [crate::project::Layer],
        width: u32,
        height: u32,
    ) -> Option<(usize, PixelRect)> {
        let edit = self.redo.pop()?;
        let layer = edit.layer;
        let pixels = layers.get_mut(layer).map(|l| l.pixels.as_mut_slice())?;
        let rect = apply_tiles(pixels, width, height, &edit.after)?;
        self.undo.push(edit);
        Some((layer, rect))
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

fn snapshot_tile(pixels: &[u8], width: u32, height: u32, tx: u32, ty: u32) -> Option<TileSnapshot> {
    let x = tx * TILE_SIZE;
    let y = ty * TILE_SIZE;
    if x >= width || y >= height {
        return None;
    }
    let w = TILE_SIZE.min(width - x);
    let h = TILE_SIZE.min(height - y);
    let mut data = vec![0u8; (w * h * 4) as usize];
    for row in 0..h {
        let src = (((y + row) * width + x) * 4) as usize;
        let dst = (row * w * 4) as usize;
        data[dst..dst + (w * 4) as usize]
            .copy_from_slice(&pixels[src..src + (w * 4) as usize]);
    }
    Some(TileSnapshot { tx, ty, w, h, data })
}

fn apply_tiles(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    tiles: &[TileSnapshot],
) -> Option<PixelRect> {
    let mut dirty: Option<PixelRect> = None;
    for tile in tiles {
        let x = tile.tx * TILE_SIZE;
        let y = tile.ty * TILE_SIZE;
        if x >= width || y >= height {
            continue;
        }
        let w = tile.w.min(width - x);
        let h = tile.h.min(height - y);
        for row in 0..h {
            let dst = (((y + row) * width + x) * 4) as usize;
            let src = (row * tile.w * 4) as usize;
            let bytes = (w * 4) as usize;
            if src + bytes <= tile.data.len() && dst + bytes <= pixels.len() {
                pixels[dst..dst + bytes].copy_from_slice(&tile.data[src..src + bytes]);
            }
        }
        let rect = PixelRect {
            min_x: x,
            min_y: y,
            max_x: x + w,
            max_y: y + h,
        };
        dirty = Some(match dirty {
            Some(prev) => PixelRect {
                min_x: prev.min_x.min(rect.min_x),
                min_y: prev.min_y.min(rect.min_y),
                max_x: prev.max_x.max(rect.max_x),
                max_y: prev.max_y.max(rect.max_y),
            },
            None => rect,
        });
    }
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_restores_only_touched_tiles() {
        let mut pixels = vec![0u8; 128 * 128 * 4];
        pixels[0] = 9;
        let mut capture = EditCapture::new(0, 128, 128);
        capture.touch_rect(
            &pixels,
            PixelRect {
                min_x: 0,
                min_y: 0,
                max_x: 1,
                max_y: 1,
            },
        );
        pixels[0] = 42;
        let edit = capture.finish(&pixels).unwrap();
        let mut history = History::default();
        history.push(edit);
        let mut layers = vec![crate::project::Layer {
            name: "L".into(),
            visible: true,
            opacity: 1.0,
            pixels: pixels.clone(),
        }];
        let (_, rect) = history.undo(&mut layers, 128, 128).unwrap();
        assert_eq!(layers[0].pixels[0], 9);
        assert!(!rect.is_empty());
        history.redo(&mut layers, 128, 128).unwrap();
        assert_eq!(layers[0].pixels[0], 42);
    }
}
