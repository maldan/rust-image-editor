use std::collections::HashSet;

use crate::editing::PixelRect;
use crate::project::{Layer, Project};
use crate::raster::{SparsePixels, Tile};
use crate::tiles::TILE_SIZE;

const MAX_ENTRIES: usize = 64;

#[derive(Clone)]
struct TileSnapshot {
    tx: i32,
    ty: i32,
    data: Option<Tile>,
}

#[derive(Clone)]
pub struct HistoryEdit {
    pub layer: usize,
    offset: (i32, i32),
    before: Vec<TileSnapshot>,
    after: Vec<TileSnapshot>,
}

/// Share physical tiles until a touched tile is changed (copy on write).
pub struct EditCapture {
    layer: usize,
    touched: HashSet<(i32, i32)>,
    before: Vec<TileSnapshot>,
    offset: (i32, i32),
}

impl EditCapture {
    pub fn new(layer: usize, _width: u32, _height: u32) -> Self {
        Self {
            layer,
            touched: HashSet::new(),
            before: Vec::new(),
            offset: (0, 0),
        }
    }
    pub fn layer(&self) -> usize {
        self.layer
    }
    pub fn touch_rect(&mut self, pixels: &SparsePixels, rect: PixelRect) {
        if rect.is_empty() {
            return;
        }
        self.offset = pixels.offset;
        let side = TILE_SIZE as i32;
        let x0 = (rect.min_x as i32 - pixels.offset.0).div_euclid(side);
        let y0 = (rect.min_y as i32 - pixels.offset.1).div_euclid(side);
        let x1 = (rect.max_x as i32 - 1 - pixels.offset.0).div_euclid(side);
        let y1 = (rect.max_y as i32 - 1 - pixels.offset.1).div_euclid(side);
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                if self.touched.insert((tx, ty)) {
                    self.before.push(TileSnapshot {
                        tx,
                        ty,
                        data: pixels.tiles.get(&(tx, ty)).cloned(),
                    });
                }
            }
        }
    }
    pub fn finish(self, pixels: &mut SparsePixels) -> Option<HistoryEdit> {
        let mut before = Vec::new();
        let mut after = Vec::new();
        for b in self.before {
            let key = (b.tx, b.ty);
            if pixels
                .tiles
                .get(&key)
                .is_some_and(|p| p.chunks_exact(4).all(|p| p[3] == 0))
            {
                pixels.tiles.remove(&key);
            }
            let data = pixels.tiles.get(&(b.tx, b.ty)).cloned();
            if data == b.data {
                continue;
            }
            after.push(TileSnapshot {
                tx: b.tx,
                ty: b.ty,
                data,
            });
            before.push(b);
        }
        if before.is_empty() {
            return None;
        }
        Some(HistoryEdit {
            layer: self.layer,
            offset: self.offset,
            before,
            after,
        })
    }
}

enum HistoryEntry {
    Raster {
        layer: usize,
        before: SparsePixels,
        after: SparsePixels,
    },
    Pixels(HistoryEdit),
    InsertLayer {
        index: usize,
        inserted: Option<Layer>,
        active_before: usize,
        size_before: (u32, u32),
        size_after: (u32, u32),
    },
    DeleteLayer {
        index: usize,
        // Ownership moves between the document and history on Undo / Redo.
        removed: Option<Layer>,
        active_before: usize,
        active_after: usize,
        width: u32,
        height: u32,
    },
}

pub enum HistoryChange {
    Pixels { layer: usize, rect: PixelRect },
    Layers,
}

#[derive(Default)]
pub struct History {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
}

impl History {
    pub fn push_raster(&mut self, layer: usize, before: SparsePixels, after: SparsePixels) {
        if before != after {
            self.push_entry(HistoryEntry::Raster {
                layer,
                before,
                after,
            });
        }
    }

    pub fn push(&mut self, edit: HistoryEdit) {
        self.push_entry(HistoryEntry::Pixels(edit));
    }

    fn push_entry(&mut self, entry: HistoryEntry) {
        self.undo.push(entry);
        if self.undo.len() > MAX_ENTRIES {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn insert_layer(&mut self, project: &mut Project, layer: Layer, canvas_size: (u32, u32)) {
        let index = project.layers.len();
        let active_before = project.active_layer;
        let size_before = (project.width, project.height);
        if project.width == 0 || project.height == 0 {
            (project.width, project.height) = canvas_size;
        }
        project.layers.push(layer);
        project.active_layer = index;
        project.sync_manifest_pub();
        self.push_entry(HistoryEntry::InsertLayer {
            index, inserted: None, active_before, size_before,
            size_after: (project.width, project.height),
        });
    }

    pub fn delete_layer(&mut self, project: &mut Project, index: usize) -> bool {
        let (width, height, active_before) = (project.width, project.height, project.active_layer);
        let Some(layer) = project.delete_layer(index) else {
            return false;
        };
        self.push_entry(HistoryEntry::DeleteLayer {
            index,
            removed: Some(layer),
            active_before,
            active_after: project.active_layer,
            width,
            height,
        });
        true
    }

    pub fn undo(&mut self, project: &mut Project) -> Option<HistoryChange> {
        let change = match self.undo.last_mut()? {
            HistoryEntry::InsertLayer { index, inserted, active_before, size_before, .. } => {
                *inserted = Some(project.delete_layer(*index)?);
                (project.width, project.height) = *size_before;
                project.active_layer = *active_before;
                project.sync_manifest_pub();
                HistoryChange::Layers
            }
            HistoryEntry::Raster { layer, before, .. } => {
                project.layers.get_mut(*layer)?.pixels = before.clone();
                HistoryChange::Layers
            }
            HistoryEntry::Pixels(edit) => {
                let pixels = &mut project.layers.get_mut(edit.layer)?.pixels;
                let rect = apply_tiles(
                    pixels,
                    project.width,
                    project.height,
                    edit.offset,
                    &edit.before,
                )?;
                HistoryChange::Pixels {
                    layer: edit.layer,
                    rect,
                }
            }
            HistoryEntry::DeleteLayer {
                index,
                removed,
                active_before,
                width,
                height,
                ..
            } => {
                if *index > project.layers.len() {
                    return None;
                }
                project.layers.insert(*index, removed.take()?);
                project.width = *width;
                project.height = *height;
                project.active_layer = *active_before;
                project.sync_manifest_pub();
                HistoryChange::Layers
            }
        };
        self.redo.push(self.undo.pop().unwrap());
        Some(change)
    }

    pub fn redo(&mut self, project: &mut Project) -> Option<HistoryChange> {
        let change = match self.redo.last_mut()? {
            HistoryEntry::InsertLayer { index, inserted, size_after, .. } => {
                if *index > project.layers.len() { return None; }
                project.layers.insert(*index, inserted.take()?);
                (project.width, project.height) = *size_after;
                project.active_layer = *index;
                project.sync_manifest_pub();
                HistoryChange::Layers
            }
            HistoryEntry::Raster { layer, after, .. } => {
                project.layers.get_mut(*layer)?.pixels = after.clone();
                HistoryChange::Layers
            }
            HistoryEntry::Pixels(edit) => {
                let pixels = &mut project.layers.get_mut(edit.layer)?.pixels;
                let rect = apply_tiles(
                    pixels,
                    project.width,
                    project.height,
                    edit.offset,
                    &edit.after,
                )?;
                HistoryChange::Pixels {
                    layer: edit.layer,
                    rect,
                }
            }
            HistoryEntry::DeleteLayer {
                index,
                removed,
                active_after,
                ..
            } => {
                *removed = Some(project.delete_layer(*index)?);
                project.active_layer = *active_after;
                project.sync_manifest_pub();
                HistoryChange::Layers
            }
        };
        self.undo.push(self.redo.pop().unwrap());
        Some(change)
    }

    pub fn clear_redo(&mut self) {
        self.redo.clear();
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

fn apply_tiles(
    pixels: &mut SparsePixels,
    width: u32,
    height: u32,
    offset: (i32, i32),
    tiles: &[TileSnapshot],
) -> Option<PixelRect> {
    let mut dirty = PixelRect {
        min_x: width,
        min_y: height,
        max_x: 0,
        max_y: 0,
    };
    for tile in tiles {
        let key = (tile.tx, tile.ty);
        match &tile.data {
            Some(data) => {
                pixels.tiles.insert(key, data.clone());
            }
            None => {
                pixels.tiles.remove(&key);
            }
        }
        let x = tile.tx * TILE_SIZE as i32 + offset.0;
        let y = tile.ty * TILE_SIZE as i32 + offset.1;
        dirty.min_x = dirty.min_x.min(x.clamp(0, width as i32) as u32);
        dirty.min_y = dirty.min_y.min(y.clamp(0, height as i32) as u32);
        dirty.max_x = dirty
            .max_x
            .max((x + TILE_SIZE as i32).clamp(0, width as i32) as u32);
        dirty.max_y = dirty
            .max_y
            .max((y + TILE_SIZE as i32).clamp(0, height as i32) as u32);
    }
    Some(dirty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Raster;

    #[test]
    fn undo_restores_only_touched_tiles() {
        let mut pixels = SparsePixels::default();
        pixels.write(128, 0, 0, [9, 0, 0, 255]);
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
        pixels.write(128, 0, 0, [42, 0, 0, 255]);
        let edit = capture.finish(&mut pixels).unwrap();
        let mut history = History::default();
        history.push(edit);
        let layers = vec![crate::project::Layer {
            name: "L".into(),
            visible: true,
            opacity: 1.0,
            pixels: pixels.clone(),
        }];
        let mut project = Project {
            layers,
            width: 128,
            height: 128,
            ..Default::default()
        };
        let HistoryChange::Pixels { rect, .. } = history.undo(&mut project).unwrap() else {
            panic!("Expected pixel edit")
        };
        assert_eq!(project.layers[0].pixels.read(128, 0, 0)[0], 9);
        assert!(!rect.is_empty());
        history.redo(&mut project).unwrap();
        assert_eq!(project.layers[0].pixels.read(128, 0, 0)[0], 42);
    }
}
