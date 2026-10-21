use super::*;

impl EditorApp {
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

    pub(in crate::app) fn sync_layer_structure(&mut self) {
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
}
