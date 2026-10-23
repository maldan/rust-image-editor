use super::*;
use crate::project::Animation;

impl EditorApp {
    pub(in crate::app) fn handle_animation_keys(&mut self) {
        if self.ai.open
            || self.hsv.open
            || self.rename.is_some()
            || self.session.preview_asset.is_some()
            || self.input.key_ctrl
            || self.brush_last_active()
            || self.input.key_left == self.input.key_right
        {
            return;
        }
        let Some(animation) = &self.project().animation else {
            return;
        };
        let frame = if self.input.key_left {
            animation.frame.saturating_sub(1)
        } else {
            (animation.frame + 1).min(animation.length - 1)
        };
        let changed = frame != animation.frame;
        self.animation_clock = None;
        if changed {
            self.seek_animation(frame);
        }
    }

    pub fn new_animation(&mut self) {
        self.new_document();
        let doc = &mut self.workspace.documents[self.workspace.active_document];
        doc.name = format!("Animation {}", self.workspace.active_document + 1);
        doc.project.animation = Some(Animation {
            fps: 12,
            length: 24,
            frame: 0,
        });
        let _ = doc.project.resize_canvas(512, 512);
        doc.project.flush_drawings();
        self.workspace.mark_active_thumb_dirty();
        let _ = self.reload_active_view();
        self.status = "Animation created — select a layer and frame to draw".into();
    }

    pub fn animation_playing(&self) -> bool {
        self.animation_clock
            .as_ref()
            .is_some_and(|(id, _)| id == &self.workspace.active_doc().id)
    }

    pub fn toggle_animation(&mut self) {
        if self.animation_playing() {
            self.animation_clock = None;
        } else if self.project().animation.is_some() && !self.brush_last_active() {
            self.commit_transform();
            self.animation_clock = Some((self.workspace.active_doc().id.clone(), 0.0));
        }
    }

    pub fn seek_animation(&mut self, frame: u32) {
        if self.brush_last_active() {
            return;
        }
        self.commit_transform();
        self.project_mut().seek(frame);
        // Pixel history addresses the current drawing; never apply it to a different one.
        self.history.discard_project_edits();
        self.clear_selection();
        self.layers_reload = true;
        self.dirty_tiles.clear();
        self.dirty_layer = None;
        self.workspace.mark_active_thumb_dirty();
    }

    pub fn animation_drawing(&mut self, duplicate: bool) {
        if self.animation_playing() || self.brush_last_active() {
            return;
        }
        self.commit_transform();
        if self
            .history
            .change_drawing(self.workspace.active_mut(), Some(duplicate))
        {
            self.sync_layer_structure();
        }
    }

    pub fn remove_animation_drawing(&mut self) {
        if self.animation_playing() || self.brush_last_active() {
            return;
        }
        self.commit_transform();
        if self
            .history
            .change_drawing(self.workspace.active_mut(), None)
        {
            self.sync_layer_structure();
        }
    }

    pub(in crate::app) fn tick_animation(&mut self) {
        if self.ai.open
            || self.hsv.open
            || self.rename.is_some()
            || self.session.preview_asset.is_some()
            || !self.animation_playing()
        {
            self.animation_clock = None;
            return;
        }
        let Some(animation) = self.project().animation.clone() else {
            self.animation_clock = None;
            return;
        };
        let Some((_, elapsed)) = &mut self.animation_clock else {
            return;
        };
        *elapsed += self.input.dt.max(0.0) as f64;
        let steps = (*elapsed * animation.fps as f64).floor() as u64;
        if steps > 0 {
            *elapsed -= steps as f64 / animation.fps as f64;
            self.seek_animation(
                ((animation.frame as u64 + steps) % animation.length as u64) as u32,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Raster;

    #[test]
    fn creating_copy_and_blank_drawings_can_undo_and_redo_without_losing_pixels() {
        let mut app = EditorApp::default();
        app.new_animation();
        app.project_mut().layers[0]
            .pixels
            .write(0, 0, 0, [10, 20, 30, 255]);
        app.seek_animation(5);
        app.animation_drawing(true);
        assert!(app.project().layers[0].drawings.contains_key(&5));
        app.undo();
        assert!(!app.project().layers[0].drawings.contains_key(&5));
        assert_eq!(
            app.project().layers[0].pixels.read(0, 0, 0),
            [10, 20, 30, 255]
        );
        app.redo();
        assert!(app.project().layers[0].drawings.contains_key(&5));
        app.animation_drawing(false);
        assert_eq!(app.project().layers[0].pixels.read(0, 0, 0), [0; 4]);
        app.undo();
        assert_eq!(
            app.project().layers[0].pixels.read(0, 0, 0),
            [10, 20, 30, 255]
        );
        app.undo();
        assert!(!app.project().layers[0].drawings.contains_key(&5));
        app.redo();
        app.redo();
        assert!(app.project().layers[0].drawings.contains_key(&5));
        assert_eq!(app.project().layers[0].pixels.read(0, 0, 0), [0; 4]);
    }

    #[test]
    fn deleting_drawing_restores_content_and_prior_pixel_history_and_noops_keep_redo() {
        let mut app = EditorApp::default();
        app.new_animation();
        app.seek_animation(5);
        app.animation_drawing(false);
        let before = app.project().layers[0].pixels.clone();
        app.project_mut().layers[0]
            .pixels
            .write(0, 0, 0, [40, 50, 60, 255]);
        let after = app.project().layers[0].pixels.clone();
        app.history.push_diff(0, &before, &after);
        app.remove_animation_drawing();
        assert!(!app.project().layers[0].drawings.contains_key(&5));
        app.remove_animation_drawing(); // No key here: must not erase history.
        app.undo();
        assert_eq!(
            app.project().layers[0].pixels.read(0, 0, 0),
            [40, 50, 60, 255]
        );
        app.undo();
        assert_eq!(app.project().layers[0].pixels.read(0, 0, 0), [0; 4]);
        app.redo();
        app.redo();
        assert!(!app.project().layers[0].drawings.contains_key(&5));
        app.undo();
        app.animation_drawing(false); // New edit replaces the redo branch.
        app.redo();
        assert!(app.project().layers[0].drawings.contains_key(&5));
    }

    #[test]
    fn arrow_keys_seek_pause_and_respect_bounds_and_editor_context() {
        let mut app = EditorApp::default();
        app.new_animation();
        app.input.key_right = true;
        app.handle_animation_keys();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 1);
        app.toggle_animation();
        app.handle_animation_keys();
        assert!(!app.animation_playing());
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 2);
        app.input.key_ctrl = true;
        app.handle_animation_keys();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 2);
        app.input.key_ctrl = false;
        app.session.preview_asset = Some("preview".into());
        app.handle_animation_keys();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 2);
        app.session.preview_asset = None;
        app.seek_animation(23);
        app.handle_animation_keys();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 23);
        app.input.key_right = false;
        app.input.key_left = true;
        app.handle_animation_keys();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 22);
        app.seek_animation(0);
        app.handle_animation_keys();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 0);
        app.new_document();
        app.handle_animation_keys();
        assert!(app.project().animation.is_none());
    }

    #[test]
    fn playback_advances_loops_and_stops_when_document_changes() {
        let mut app = EditorApp::default();
        app.new_animation();
        app.project_mut().layers[0]
            .pixels
            .write(0, 0, 0, [10, 0, 0, 255]);
        app.seek_animation(1);
        app.animation_drawing(false);
        app.project_mut().layers[0]
            .pixels
            .write(0, 0, 0, [20, 0, 0, 255]);
        app.seek_animation(0);
        app.toggle_animation();
        app.input.dt = 1.0 / 12.0;
        app.tick_animation();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 1);
        assert_eq!(app.project().layers[0].pixels.read(0, 0, 0)[0], 20);
        app.input.dt = 23.0 / 12.0 + 0.001;
        app.tick_animation();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 0);
        assert_eq!(app.project().layers[0].pixels.read(0, 0, 0)[0], 10);
        app.new_document();
        assert!(!app.animation_playing());
        assert!(app.project().animation.is_none());
    }
}
