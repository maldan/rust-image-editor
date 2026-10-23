use super::*;

impl EditorApp {
    pub fn invert_layer(&mut self) {
        if self.ai.open || self.hsv.open || self.project().layers.is_empty() {
            return;
        }
        self.close_asset_preview();
        self.commit_transform();
        let index = self.project().active_layer;
        let Some(layer) = self.workspace.active_mut().layers.get_mut(index) else {
            return;
        };
        let before = layer.pixels.clone();
        if !layer.pixels.invert_colors() {
            return;
        }
        let edit = crate::history::HistoryEdit::from_diff(index, &before, &layer.pixels);
        if let Some(edit) = edit {
            self.history.push(edit);
        }
        self.layers_reload = true;
        self.dirty = true;
        self.workspace.mark_active_thumb_dirty();
        self.status = "Layer colors inverted".into();
    }

    pub fn fix_alpha(&mut self) {
        self.commit_transform();
        let layer = self.project().active_layer;
        let Some(pixels) = self
            .workspace
            .active_mut()
            .layers
            .get_mut(layer)
            .map(|layer| &mut layer.pixels)
        else {
            return;
        };
        let before = pixels.clone();
        let changed = pixels.fix_alpha();
        let edit = crate::history::HistoryEdit::from_diff(layer, &before, pixels);
        if changed == 0 {
            self.status = "Fix Alpha: no pixels below alpha 5".into();
            return;
        }
        if let Some(edit) = edit {
            self.history.push(edit);
        }
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
        if index == 0 || index >= self.project().layers.len() {
            return;
        }
        self.commit_transform();
        if !self.history.merge_down(self.workspace.active_mut()) {
            return;
        }
        self.layer_drag = None;
        self.sync_layer_structure();
        self.status = "Layer merged down".into();
    }

    pub fn delete_layer(&mut self, index: usize) {
        if index >= self.project().layers.len() {
            return;
        }
        self.cancel_transform();
        if !self
            .history
            .delete_layer(self.workspace.active_mut(), index)
        {
            return;
        }
        self.layer_drag = None;
        self.sync_layer_structure();
        self.status = if self.project().layers.is_empty() {
            "Last layer deleted"
        } else {
            "Layer deleted"
        }
        .into();
    }

    pub(in crate::app) fn sync_layer_structure(&mut self) {
        let size = if self.project().layers.is_empty() {
            None
        } else {
            Some((self.project().width, self.project().height))
        };
        if size != self.image_size {
            if let Some((width, height)) = size {
                self.resize_w = width;
                self.resize_h = height;
                self.stroke_marks = vec![0; width as usize * height as usize];
                self.dirty_tiles.resize(width, height);
            } else {
                self.stroke_marks.clear();
            }
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
        if from == to || from >= self.project().layers.len() || to >= self.project().layers.len() {
            return;
        }
        // History addresses layers by index; an unrecorded reorder invalidates those indices.
        self.history.clear();
        self.workspace.active_mut().move_layer_to(from, to);
        self.workspace.mark_active_thumb_dirty();
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
            if let Some(floating) = self.session.floating.as_mut()
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
                self.history.discard_project_edits();
                self.workspace.mark_active_thumb_dirty();
                self.dirty = true;
                self.status = format!("Canvas resized to {w} × {h}");
                ui.notify_success("Canvas resized");
                let _ = self.reload_active_view();
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
        self.session.fit_image = true;
        let (width, height) = self.image_size.unwrap();
        self.status = format!("Canvas trimmed to {width} × {height}");
        ui.notify_success("Canvas trimmed");
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;
    #[test]
    fn rename_targets_clicked_layer_and_persists_without_changing_pixels_or_selection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rename.mgimg");
        let mut app = EditorApp::default();
        app.project_mut()
            .import_rgba(&[1, 2, 3, 255], 1, 1)
            .unwrap();
        app.project_mut().add_layer();
        app.project_mut().active_layer = 0;
        app.workspace.save(&path).unwrap();
        app.dirty = false;
        let pixels = app.project().layers[1].pixels.clone();
        app.open_layer_rename(1);
        assert_eq!(app.rename.as_ref().unwrap().name, "Layer 2");
        app.rename.as_mut().unwrap().name = "  Detail layer  ".into();
        app.apply_rename();
        assert!(app.rename.is_none());
        assert_eq!(app.project().layers[1].name, "Detail layer");
        assert_eq!(app.project().layers[0].name, "Layer 1");
        assert_eq!(app.project().active_layer, 0);
        assert_eq!(app.project().layers[1].pixels, pixels);
        assert!(app.dirty);
        app.workspace.save(&path).unwrap();
        let loaded = Workspace::open(&path).unwrap();
        assert_eq!(loaded.documents[0].project.layers[1].name, "Detail layer");
        app.dirty = false;
        app.open_layer_rename(1);
        app.apply_rename();
        assert!(!app.dirty, "accepting the same name is a no-op");
    }

    #[test]
    fn rename_cancel_and_empty_name_do_not_modify_document_and_modal_blocks_shortcuts() {
        let mut app = EditorApp::default();
        app.project_mut()
            .import_rgba(&[1, 2, 3, 255], 1, 1)
            .unwrap();
        app.dirty = false;
        app.open_layer_rename(0);
        app.rename.as_mut().unwrap().name = "   ".into();
        app.apply_rename();
        assert!(app.rename.is_some());
        assert_eq!(app.project().layers[0].name, "Layer 1");
        let mut input = FrameInput::default();
        input.key_undo = true;
        input.key_delete = true;
        input.mouse_down = true;
        input.key_escape = true;
        <EditorApp as Scene>::begin_frame(&mut app, &input);
        assert!(!app.input.key_undo && !app.input.key_delete && !app.input.mouse_down);
        let mut ui = Ui::new();
        ui.begin_frame(mega_ui::UiInput {
            viewport: Vec2::new(800.0, 600.0),
            ..Default::default()
        });
        crate::ui::rename_dialog::draw(&mut ui, &mut app);
        ui.end_frame();
        assert!(app.rename.is_none());
        assert!(!app.dirty);
        assert_eq!(app.project().layers[0].name, "Layer 1");
    }
}
