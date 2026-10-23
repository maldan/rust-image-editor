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
        doc.project.animation = Some(Animation::new(12, 24));
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
        self.history.bind_animation_edits(self.workspace.active());
        self.project_mut().seek(frame);
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
        let (start, end) = animation.playback_range();
        let span = u64::from(end - start + 1);
        let inside = (start..=end).contains(&animation.frame);
        let next = {
            let Some((_, elapsed)) = &mut self.animation_clock else {
                return;
            };
            *elapsed += self.input.dt.max(0.0) as f64;
            let steps = (*elapsed * animation.fps as f64).floor() as u64;
            if steps == 0 && inside {
                return;
            }
            if steps > 0 {
                *elapsed -= steps as f64 / animation.fps as f64;
            }
            let origin = if inside {
                u64::from(animation.frame)
            } else {
                u64::from(start)
            };
            start + ((origin - u64::from(start) + steps) % span) as u32
        };
        if next != animation.frame {
            self.seek_animation(next);
        }
    }

    pub fn resize_animation_frame(&mut self, longer: bool) {
        if self.animation_playing() || self.brush_last_active() {
            return;
        }
        self.commit_transform();
        if self
            .history
            .resize_drawing(self.workspace.active_mut(), longer)
        {
            self.clear_selection();
            self.sync_layer_structure();
            self.status = if longer {
                "Frame extended"
            } else {
                "Frame shortened"
            }
            .into();
        }
    }

    pub fn set_animation_loop_start(&mut self, frame: u32) {
        let Some(animation) = self.project_mut().animation.as_mut() else {
            return;
        };
        let end = animation.playback_range().1;
        animation.play_start = frame.min(end);
        self.dirty = true;
    }

    pub fn set_animation_loop_end(&mut self, frame: u32) {
        let Some(animation) = self.project_mut().animation.as_mut() else {
            return;
        };
        let last = animation.length.saturating_sub(1);
        let start = animation.play_start.min(last);
        animation.play_start = start;
        animation.play_end = frame.clamp(start, last);
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Raster;

    #[test]
    fn layer_restore_after_navigation_keeps_each_drawing() {
        let mut app = EditorApp::default();
        app.new_animation();
        app.project_mut().layers[0].pixels.write(0,2,3,[100,0,0,255]);
        let first = app.project().layers[0].pixels.clone();
        app.seek_animation(5);
        app.animation_drawing(false);
        app.project_mut().layers[0].pixels.write(0,2,3,[0,100,0,255]);
        let second = app.project().layers[0].pixels.clone();
        app.seek_animation(0);
        app.delete_layer(0);
        app.seek_animation(5);
        app.undo();
        assert_eq!(app.project().animation.as_ref().unwrap().frame,0);
        assert_eq!(app.project().layers[0].pixels,first);
        app.seek_animation(5);
        assert_eq!(app.project().layers[0].pixels,second);
        app.redo();
        assert_eq!(app.project().animation.as_ref().unwrap().frame,0);
        assert!(app.project().layers.is_empty());
    }

    #[test]
    fn pixel_history_keeps_independent_drawings_across_seek_and_redo() {
        let mut app = EditorApp::default();
        app.new_animation();
        let blank = app.project().layers[0].pixels.clone();
        app.project_mut().layers[0].pixels.write(0, 2, 3, [100,0,0,255]);
        let first = app.project().layers[0].pixels.clone();
        app.history.push_diff(0, &blank, &first);
        app.seek_animation(5);
        app.animation_drawing(false);
        app.project_mut().layers[0].pixels.write(0, 2, 3, [0,100,0,255]);
        let second = app.project().layers[0].pixels.clone();
        app.history.push_diff(0, &blank, &second);
        app.seek_animation(0);
        app.undo();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 5);
        assert_eq!(app.project().layers[0].pixels, blank);
        app.seek_animation(0);
        assert_eq!(app.project().layers[0].pixels, first);
        app.redo();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 5);
        assert_eq!(app.project().layers[0].pixels, second);
        app.seek_animation(0);
        assert_eq!(app.project().layers[0].pixels, first);
    }

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

    #[test]
    fn extending_and_shortening_a_frame_shifts_later_drawings_and_undoes() {
        use crate::raster::Raster;
        let mut app = EditorApp::default();
        app.new_animation();
        app.project_mut().layers[0]
            .pixels
            .write(0, 0, 0, [1, 0, 0, 255]);
        app.add_layer();
        app.seek_animation(4);
        app.animation_drawing(true);
        app.project_mut().layers[1]
            .pixels
            .write(0, 0, 0, [2, 0, 0, 255]);
        app.seek_animation(1);
        assert_eq!(app.project().held_length(), 4);
        app.resize_animation_frame(true);
        let project = app.project();
        assert_eq!(project.animation.as_ref().unwrap().length, 25);
        assert_eq!(project.animation.as_ref().unwrap().frame, 1);
        assert_eq!(project.held_length(), 5);
        assert!(project.layers[1].drawings.contains_key(&5));
        assert!(!project.layers[1].drawings.contains_key(&4));
        assert_eq!(
            project.layers[1].drawings.get(&5).unwrap().read(0, 0, 0)[0],
            2
        );
        assert_eq!(project.layers[0].pixels.read(0, 0, 0)[0], 1);
        assert_eq!(project.layers[1].pixels.read(0, 0, 0)[0], 0);
        app.resize_animation_frame(false);
        assert_eq!(app.project().animation.as_ref().unwrap().length, 24);
        assert!(app.project().layers[1].drawings.contains_key(&4));
        assert_eq!(
            app.project().layers[1].drawings.get(&4).unwrap().read(0, 0, 0)[0],
            2
        );
        app.undo();
        assert_eq!(app.project().animation.as_ref().unwrap().length, 25);
        assert!(app.project().layers[1].drawings.contains_key(&5));
        app.redo();
        assert_eq!(app.project().animation.as_ref().unwrap().length, 24);
        assert!(app.project().layers[1].drawings.contains_key(&4));

        app.seek_animation(4);
        app.animation_drawing(false);
        app.seek_animation(5);
        app.animation_drawing(false);
        app.seek_animation(4);
        app.project_mut().active_layer = 1;
        assert_eq!(app.project().held_length(), 1);
        app.resize_animation_frame(false);
        assert_eq!(app.project().held_length(), 1);
        assert_eq!(app.project().animation.as_ref().unwrap().length, 24);
        app.resize_animation_frame(true);
        assert_eq!(app.project().held_length(), 2);
        assert_eq!(app.project().animation.as_ref().unwrap().length, 25);
        assert!(app.project().layers[1].drawings.contains_key(&6));
        assert!(!app.project().layers[1].drawings.contains_key(&5));
        app.project_mut().animation.as_mut().unwrap().length = 1200;
        app.resize_animation_frame(true);
        assert_eq!(app.project().animation.as_ref().unwrap().length, 1200);
    }

    #[test]
    fn playback_loops_between_the_selected_frames() {
        let mut app = EditorApp::default();
        app.new_animation();
        app.set_animation_loop_start(2);
        app.set_animation_loop_end(4);
        app.seek_animation(10);
        app.toggle_animation();
        app.input.dt = 0.0;
        app.tick_animation();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 2);
        app.input.dt = 1.0 / 12.0;
        app.tick_animation();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 3);
        app.input.dt = 2.0 / 12.0;
        app.tick_animation();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 2);
        app.set_animation_loop_end(2);
        app.input.dt = 1.0 / 12.0;
        app.tick_animation();
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 2);
    }
}
