use super::*;

impl EditorApp {
    pub fn undo(&mut self) {
        self.cancel_transform();
        let change = if self.history.document_undo_pending() {
            self.history.undo_document(&mut self.workspace)
        } else {
            self.history.undo(self.workspace.active_mut())
        };
        if let Some(change) = change {
            self.apply_history_change(change);
            self.status = "Undo".into();
        }
    }

    pub fn redo(&mut self) {
        self.cancel_transform();
        let change = if self.history.document_redo_pending() {
            self.history.redo_document(&mut self.workspace)
        } else {
            self.history.redo(self.workspace.active_mut())
        };
        if let Some(change) = change {
            self.apply_history_change(change);
            self.status = "Redo".into();
        }
    }

    pub(in crate::app) fn apply_history_change(&mut self, change: HistoryChange) {
        match change {
            HistoryChange::Pixels { layer, rect } => {
                self.dirty = true;
                self.mark_layer_dirty(layer, rect);
                self.workspace.mark_active_thumb_dirty();
            }
            HistoryChange::Layers => self.sync_layer_structure(),
            HistoryChange::Document { active_changed } => {
                self.workspace.select_active_document();
                self.dirty = true;
                if active_changed {
                    let _ = self.reload_active_view();
                }
            }
        }
    }

    pub fn refresh_image(&mut self) -> Result<(), String> {
        self.history.clear();
        self.reload_active_view()
    }

    pub(in crate::app) fn reload_active_view(&mut self) -> Result<(), String> {
        self.close_asset_preview();
        self.cancel_transform();
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
                self.session.fit_image = true;
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

    pub(in crate::app) fn mark_dirty_rect(&mut self, rect: PixelRect) {
        let layer = self
            .session.floating
            .as_ref()
            .map(|f| f.layer)
            .unwrap_or(self.workspace.active().active_layer);
        self.mark_layer_dirty(layer, rect);
        self.workspace.mark_active_thumb_dirty();
    }

    pub(in crate::app) fn mark_layer_dirty(&mut self, layer: usize, rect: PixelRect) {
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

    pub(in crate::app) fn show_error(&mut self, ui: &mut Ui, error: String) {
        self.status = error.clone();
        ui.notify_error(&error);
    }
}
