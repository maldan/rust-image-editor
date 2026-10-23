use std::collections::HashSet;

use crate::editing::PixelRect;
use crate::project::{Layer, Project};
use crate::raster::{SparsePixels, Tile};
use crate::tiles::TILE_SIZE;
use crate::workspace::Document;

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
    offset_before: (i32, i32),
    offset_after: (i32, i32),
    before: Vec<TileSnapshot>,
    after: Vec<TileSnapshot>,
}

impl HistoryEdit {
    /// Tile copies plus the layer offset. Unchanged tiles are left out, so a full-layer
    /// filter does not keep a second copy of the sparse buffer.
    pub fn from_diff(layer: usize, before: &SparsePixels, after: &SparsePixels) -> Option<Self> {
        let mut keys: Vec<(i32, i32)> = before
            .tiles
            .keys()
            .chain(after.tiles.keys())
            .copied()
            .collect();
        keys.sort_unstable();
        keys.dedup();
        let mut before_tiles = Vec::new();
        let mut after_tiles = Vec::new();
        for key in keys {
            let old = before.tiles.get(&key);
            let new = after.tiles.get(&key);
            let same = match (old, new) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b) || a == b,
                (None, None) => true,
                _ => false,
            };
            if same {
                continue;
            }
            before_tiles.push(TileSnapshot {
                tx: key.0,
                ty: key.1,
                data: old.cloned(),
            });
            after_tiles.push(TileSnapshot {
                tx: key.0,
                ty: key.1,
                data: new.cloned(),
            });
        }
        if before_tiles.is_empty() && before.offset == after.offset {
            return None;
        }
        Some(Self {
            layer,
            offset_before: before.offset,
            offset_after: after.offset,
            before: before_tiles,
            after: after_tiles,
        })
    }
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
        if self.touched.is_empty() {
            self.offset = pixels.offset;
        }
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
        if before.is_empty() && self.offset == pixels.offset {
            return None;
        }
        Some(HistoryEdit {
            layer: self.layer,
            offset_before: self.offset,
            offset_after: pixels.offset,
            before,
            after,
        })
    }
}

enum HistoryEntry {
    Drawing {
        layer: usize,
        frame: u32,
        before: Option<SparsePixels>,
        after: Option<SparsePixels>,
    },
    TrimCanvas {
        origin: (i32, i32),
        size_before: (u32, u32),
        size_after: (u32, u32),
    },
    Pixels(HistoryEdit),
    MergeDown {
        index: usize,
        lower: Layer,
        upper: Layer,
        merged: Layer,
    },
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
    /// The document itself moves between the workspace and this entry.
    DeleteDocument {
        index: usize,
        id: String,
        document: Option<Document>,
        /// Document that stayed open after the deletion.
        fallback_id: String,
        /// The deleted document was the one on screen.
        switched: bool,
    },
}

pub enum HistoryChange {
    Pixels { layer: usize, rect: PixelRect },
    Layers,
    Document { active_changed: bool },
}

#[derive(Default)]
pub struct History {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
}

impl History {
    pub fn change_drawing(&mut self, project: &mut Project, duplicate: Option<bool>) -> bool {
        let Some(animation) = &project.animation else {
            return false;
        };
        let frame = animation.frame;
        let index = project.active_layer;
        if project.layers.get(index).is_none() {
            return false;
        }
        project.flush_drawings();
        let before = project.layers[index].drawings.get(&frame).cloned();
        if let Some(duplicate) = duplicate {
            project.new_drawing(duplicate);
        } else {
            project.delete_drawing();
        }
        let after = project.layers[index].drawings.get(&frame).cloned();
        if before == after {
            return false;
        }
        self.push_entry(HistoryEntry::Drawing {
            layer: index,
            frame,
            before,
            after,
        });
        true
    }

    pub fn trim_canvas(&mut self, project: &mut Project) -> bool {
        let Some(bounds) = project.visible_content_bounds() else {
            return false;
        };
        let size_before = (project.width, project.height);
        let size_after = bounds.size();
        let origin = (bounds.min_x, bounds.min_y);
        if size_before == size_after && origin == (0, 0) {
            return false;
        }
        if !shift_canvas(project, (-origin.0, -origin.1), size_after) {
            return false;
        }
        self.push_entry(HistoryEntry::TrimCanvas {
            origin,
            size_before,
            size_after,
        });
        true
    }

    pub fn push_diff(&mut self, layer: usize, before: &SparsePixels, after: &SparsePixels) {
        if let Some(edit) = HistoryEdit::from_diff(layer, before, after) {
            self.push(edit);
        }
    }

    pub fn push(&mut self, edit: HistoryEdit) {
        self.push_entry(HistoryEntry::Pixels(edit));
    }

    pub fn push_delete_document(
        &mut self,
        index: usize,
        document: Document,
        fallback_id: String,
        switched: bool,
    ) {
        let id = document.id.clone();
        self.push_entry(HistoryEntry::DeleteDocument {
            index,
            id,
            document: Some(document),
            fallback_id,
            switched,
        });
    }

    pub fn document_undo_pending(&self) -> bool {
        matches!(self.undo.last(), Some(HistoryEntry::DeleteDocument { .. }))
    }

    pub fn document_redo_pending(&self) -> bool {
        matches!(self.redo.last(), Some(HistoryEntry::DeleteDocument { .. }))
    }

    /// Drop pixel and layer edits. A deleted document can still be restored.
    pub fn discard_project_edits(&mut self) {
        self.undo
            .retain(|entry| matches!(entry, HistoryEntry::DeleteDocument { .. }));
        self.redo.clear();
    }

    pub fn undo_document(
        &mut self,
        workspace: &mut crate::workspace::Workspace,
    ) -> Option<HistoryChange> {
        let before = workspace
            .documents
            .get(workspace.active_document)?
            .id
            .clone();
        let active_changed = match self.undo.last_mut()? {
            HistoryEntry::DeleteDocument {
                index,
                document,
                switched,
                ..
            } => {
                let doc = document.take()?;
                let index = (*index).min(workspace.documents.len());
                let active = workspace.active_document;
                let switched = *switched;
                workspace.documents.insert(index, doc);
                if switched {
                    workspace.active_document = index;
                } else if index <= active {
                    workspace.active_document = active + 1;
                }
                if workspace.active_document >= workspace.documents.len() {
                    workspace.active_document = workspace.documents.len() - 1;
                }
                workspace.invalidate_thumb_atlas();
                workspace
                    .documents
                    .get(workspace.active_document)
                    .map(|doc| doc.id.clone())
                    .as_deref()
                    != Some(before.as_str())
            }
            _ => return None,
        };
        self.redo.push(self.undo.pop()?);
        Some(HistoryChange::Document { active_changed })
    }

    pub fn redo_document(
        &mut self,
        workspace: &mut crate::workspace::Workspace,
    ) -> Option<HistoryChange> {
        let before = workspace
            .documents
            .get(workspace.active_document)?
            .id
            .clone();
        let active_changed = match self.redo.last_mut()? {
            HistoryEntry::DeleteDocument {
                id,
                document,
                fallback_id,
                switched,
                ..
            } => {
                if workspace.documents.len() <= 1 {
                    return None;
                }
                let Some(pos) = workspace.documents.iter().position(|doc| &doc.id == id) else {
                    return None;
                };
                let removed = workspace.documents.remove(pos);
                let switched = *switched;
                let fallback = fallback_id.clone();
                if switched {
                    if let Some(found) = workspace
                        .documents
                        .iter()
                        .position(|doc| doc.id == fallback)
                    {
                        workspace.active_document = found;
                    }
                } else if pos < workspace.active_document {
                    workspace.active_document -= 1;
                }
                if workspace.active_document >= workspace.documents.len() {
                    workspace.active_document = workspace.documents.len() - 1;
                }
                *document = Some(removed);
                workspace.invalidate_thumb_atlas();
                workspace
                    .documents
                    .get(workspace.active_document)
                    .map(|doc| doc.id.clone())
                    .as_deref()
                    != Some(before.as_str())
            }
            _ => return None,
        };
        self.undo.push(self.redo.pop()?);
        Some(HistoryChange::Document { active_changed })
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
        self.push_entry(HistoryEntry::InsertLayer {
            index,
            inserted: None,
            active_before,
            size_before,
            size_after: (project.width, project.height),
        });
    }

    pub fn merge_down(&mut self, project: &mut Project) -> bool {
        let index = project.active_layer;
        if index == 0 || index >= project.layers.len() {
            return false;
        }
        let lower = project.layers[index - 1].clone();
        let upper = project.layers[index].clone();
        let merged = crate::editing::merge_layers(&lower, &upper);
        project.layers[index - 1] = merged.clone();
        project.layers.remove(index);
        project.active_layer = index - 1;
        self.push_entry(HistoryEntry::MergeDown {
            index,
            lower,
            upper,
            merged,
        });
        true
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
            HistoryEntry::Drawing {
                layer,
                frame,
                before,
                ..
            } => {
                restore_drawing(project, *layer, *frame, before)?;
                HistoryChange::Layers
            }
            HistoryEntry::TrimCanvas {
                origin,
                size_before,
                ..
            } => {
                if !shift_canvas(project, *origin, *size_before) {
                    return None;
                }
                HistoryChange::Layers
            }
            HistoryEntry::MergeDown {
                index,
                lower,
                upper,
                ..
            } => {
                if *index == 0 || *index > project.layers.len() {
                    return None;
                }
                project.layers[*index - 1] = lower.clone();
                project.layers.insert(*index, upper.clone());
                project.active_layer = *index;
                HistoryChange::Layers
            }
            HistoryEntry::InsertLayer {
                index,
                inserted,
                active_before,
                size_before,
                ..
            } => {
                *inserted = Some(project.delete_layer(*index)?);
                (project.width, project.height) = *size_before;
                project.active_layer = *active_before;
                HistoryChange::Layers
            }
            HistoryEntry::Pixels(edit) => {
                let pixels = &mut project.layers.get_mut(edit.layer)?.pixels;
                let rect = apply_edit(pixels, project.width, project.height, edit, false)?;
                HistoryChange::Pixels {
                    layer: edit.layer,
                    rect,
                }
            }
            HistoryEntry::DeleteDocument { .. } => return None,
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
                HistoryChange::Layers
            }
        };
        self.redo.push(self.undo.pop().unwrap());
        Some(change)
    }

    pub fn redo(&mut self, project: &mut Project) -> Option<HistoryChange> {
        let change = match self.redo.last_mut()? {
            HistoryEntry::Drawing {
                layer,
                frame,
                after,
                ..
            } => {
                restore_drawing(project, *layer, *frame, after)?;
                HistoryChange::Layers
            }
            HistoryEntry::TrimCanvas {
                origin, size_after, ..
            } => {
                if !shift_canvas(project, (-origin.0, -origin.1), *size_after) {
                    return None;
                }
                HistoryChange::Layers
            }
            HistoryEntry::MergeDown { index, merged, .. } => {
                if *index == 0 || *index >= project.layers.len() {
                    return None;
                }
                project.layers[*index - 1] = merged.clone();
                project.layers.remove(*index);
                project.active_layer = *index - 1;
                HistoryChange::Layers
            }
            HistoryEntry::InsertLayer {
                index,
                inserted,
                size_after,
                ..
            } => {
                if *index > project.layers.len() {
                    return None;
                }
                project.layers.insert(*index, inserted.take()?);
                (project.width, project.height) = *size_after;
                project.active_layer = *index;
                HistoryChange::Layers
            }
            HistoryEntry::Pixels(edit) => {
                let pixels = &mut project.layers.get_mut(edit.layer)?.pixels;
                let rect = apply_edit(pixels, project.width, project.height, edit, true)?;
                HistoryChange::Pixels {
                    layer: edit.layer,
                    rect,
                }
            }
            HistoryEntry::DeleteDocument { .. } => return None,
            HistoryEntry::DeleteLayer {
                index,
                removed,
                active_after,
                ..
            } => {
                *removed = Some(project.delete_layer(*index)?);
                project.active_layer = *active_after;
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

fn restore_drawing(
    project: &mut Project,
    index: usize,
    frame: u32,
    drawing: &Option<SparsePixels>,
) -> Option<()> {
    if index >= project.layers.len() || frame >= project.animation.as_ref()?.length {
        return None;
    }
    project.seek(frame);
    let layer = &mut project.layers[index];
    if let Some(pixels) = drawing {
        layer.drawings.insert(frame, pixels.clone());
    } else {
        layer.drawings.remove(&frame);
    }
    layer.pixels = layer
        .drawings
        .range(..=frame)
        .next_back()
        .map(|(_, pixels)| pixels.clone())
        .unwrap_or_default();
    project.active_layer = index;
    Some(())
}

fn shift_canvas(project: &mut Project, delta: (i32, i32), size: (u32, u32)) -> bool {
    // Check first so a distant hidden layer cannot cause a partial shift.
    if project.layers.iter().any(|layer| {
        layer.pixels.offset.0.checked_add(delta.0).is_none()
            || layer.pixels.offset.1.checked_add(delta.1).is_none()
    }) {
        return false;
    }
    for layer in &mut project.layers {
        layer.pixels.offset.0 += delta.0;
        layer.pixels.offset.1 += delta.1;
    }
    (project.width, project.height) = size;
    true
}

fn apply_edit(
    pixels: &mut SparsePixels,
    width: u32,
    height: u32,
    edit: &HistoryEdit,
    redo: bool,
) -> Option<PixelRect> {
    let (expect, target, tiles) = if redo {
        (edit.offset_before, edit.offset_after, &edit.after)
    } else {
        (edit.offset_after, edit.offset_before, &edit.before)
    };
    let dx = pixels.offset.0.checked_sub(expect.0)?;
    let dy = pixels.offset.1.checked_sub(expect.1)?;
    let new_offset = (target.0.checked_add(dx)?, target.1.checked_add(dy)?);
    let old_offset = pixels.offset;
    pixels.offset = new_offset;
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
    }
    let mut dirty = PixelRect {
        min_x: width,
        min_y: height,
        max_x: 0,
        max_y: 0,
    };
    let keys: Vec<(i32, i32)> = if tiles.is_empty() {
        pixels.tiles.keys().copied().collect()
    } else {
        tiles.iter().map(|tile| (tile.tx, tile.ty)).collect()
    };
    for (tx, ty) in keys {
        cover_tile(&mut dirty, tx, ty, old_offset, width, height);
        cover_tile(&mut dirty, tx, ty, new_offset, width, height);
    }
    Some(dirty)
}

fn cover_tile(
    dirty: &mut PixelRect,
    tx: i32,
    ty: i32,
    offset: (i32, i32),
    width: u32,
    height: u32,
) {
    let x = tx * TILE_SIZE as i32 + offset.0;
    let y = ty * TILE_SIZE as i32 + offset.1;
    dirty.min_x = dirty.min_x.min(x.clamp(0, width as i32) as u32);
    dirty.min_y = dirty.min_y.min(y.clamp(0, height as i32) as u32);
    dirty.max_x = dirty
        .max_x
        .max((x + TILE_SIZE as i32).clamp(0, width as i32) as u32);
    dirty.max_y = dirty
        .max_y
        .max((y + TILE_SIZE as i32).clamp(0, height as i32) as u32);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Raster;

    #[test]
    fn trim_canvas_removes_transparent_margins_without_copying_pixels_and_undoes() {
        let mut project = Project::default();
        project.resize_canvas(1000, 10000).unwrap();
        project.layers[0]
            .pixels
            .write(0, 320, 450, [10, 20, 30, 255]);
        project.layers[0].pixels.write(0, 419, 549, [40, 50, 60, 1]);
        project.layers[0].pixels.write(0, 0, 0, [255, 255, 255, 0]);
        project.layers[0]
            .pixels
            .write(0, -100000, 500, [100, 0, 0, 255]);
        project.add_layer();
        project.layers[1].visible = false;
        project.layers[1].pixels.write(0, 0, 0, [0, 255, 0, 255]);
        project.add_layer();
        project.layers[2].opacity = 0.0;
        project.layers[2]
            .pixels
            .write(0, 999, 9999, [0, 0, 255, 255]);
        let before: Vec<_> = project.layers.iter().map(|l| l.pixels.clone()).collect();
        let mut history = History::default();
        assert!(history.trim_canvas(&mut project));
        assert_eq!((project.width, project.height), (100, 100));
        assert_eq!(project.active_layer, 2);
        assert_eq!(project.layers[0].pixels.read(0, 0, 0), [10, 20, 30, 255]);
        assert_eq!(project.layers[0].pixels.read(0, 99, 99), [40, 50, 60, 1]);
        assert_eq!(
            project.layers[1].pixels.read(0, -320, -450),
            [0, 255, 0, 255]
        );
        for (layer, original) in project.layers.iter().zip(&before) {
            for (key, tile) in &original.tiles {
                assert!(std::sync::Arc::ptr_eq(tile, &layer.pixels.tiles[key]));
            }
        }
        let after: Vec<_> = project.layers.iter().map(|l| l.pixels.clone()).collect();
        assert!(
            !history.trim_canvas(&mut project),
            "already fitted canvas is a no-op"
        );
        assert!(matches!(
            history.undo(&mut project),
            Some(HistoryChange::Layers)
        ));
        assert_eq!((project.width, project.height), (1000, 10000));
        for (layer, original) in project.layers.iter().zip(&before) {
            assert_eq!(&layer.pixels, original);
        }
        assert!(
            history.undo(&mut project).is_none(),
            "no-op must not add an undo entry"
        );
        assert!(history.redo(&mut project).is_some());
        assert_eq!((project.width, project.height), (100, 100));
        for (layer, original) in project.layers.iter().zip(&after) {
            assert_eq!(&layer.pixels, original);
        }
    }

    #[test]
    fn trim_canvas_handles_empty_content_and_keeps_pixel_history_before_and_after() {
        let mut project = Project::default();
        let mut history = History::default();
        assert!(!history.trim_canvas(&mut project));
        project.resize_canvas(100, 80).unwrap();
        assert!(!history.trim_canvas(&mut project));
        assert_eq!((project.width, project.height), (100, 80));
        project.layers[0].pixels.write(0, 40, 50, [255, 0, 0, 255]);
        let before = project.layers[0].pixels.clone();
        project.layers[0].pixels.write(0, 42, 51, [0, 0, 255, 255]);
        history.push_diff(0, &before, &project.layers[0].pixels);
        assert!(history.trim_canvas(&mut project));
        let before = project.layers[0].pixels.clone();
        project.layers[0].pixels.write(0, 0, 0, [0, 255, 0, 255]);
        history.push_diff(0, &before, &project.layers[0].pixels);
        history.undo(&mut project).unwrap();
        assert_eq!(project.layers[0].pixels.read(0, 0, 0), [255, 0, 0, 255]);
        history.undo(&mut project).unwrap();
        assert_eq!((project.width, project.height), (100, 80));
        assert_eq!(project.layers[0].pixels.read(0, 40, 50), [255, 0, 0, 255]);
        history.undo(&mut project).unwrap();
        assert_eq!(project.layers[0].pixels.read(0, 42, 51), [0; 4]);
        for _ in 0..3 {
            history.redo(&mut project).unwrap();
        }
        assert_eq!((project.width, project.height), (3, 2));
        assert_eq!(project.layers[0].pixels.read(0, 0, 0), [0, 255, 0, 255]);
        assert_eq!(project.layers[0].pixels.read(0, 2, 1), [0, 0, 255, 255]);
    }

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
            drawings: Default::default(),
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

    #[test]
    fn full_layer_diff_stores_changed_tiles_and_restores_offset() {
        let mut before = SparsePixels::default();
        before.write(64, 0, 0, [1, 2, 3, 255]);
        before.write(64, 70, 0, [4, 5, 6, 255]);
        let mut after = before.clone();
        after.write(64, 0, 0, [9, 9, 9, 255]);
        after.offset = (3, -4);
        let edit = HistoryEdit::from_diff(0, &before, &after).unwrap();
        assert_eq!(edit.before.len(), 1);
        assert!(std::sync::Arc::ptr_eq(
            &before.tiles[&(1, 0)],
            &after.tiles[&(1, 0)]
        ));
        let mut project = Project {
            animation: None,
            width: 128,
            height: 64,
            layers: vec![crate::project::Layer {
                drawings: Default::default(),
                name: "L".into(),
                visible: true,
                opacity: 1.0,
                pixels: after,
            }],
            ..Default::default()
        };
        let mut history = History::default();
        history.push(edit);
        history.undo(&mut project).unwrap();
        assert_eq!(project.layers[0].pixels.offset, (0, 0));
        assert_eq!(project.layers[0].pixels.read(64, 0, 0), [1, 2, 3, 255]);
        assert_eq!(project.layers[0].pixels.read(64, 70, 0), [4, 5, 6, 255]);
        history.redo(&mut project).unwrap();
        assert_eq!(project.layers[0].pixels.offset, (3, -4));
        assert_eq!(project.layers[0].pixels.read(64, 3, -4), [9, 9, 9, 255]);
    }
}
