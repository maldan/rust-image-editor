use super::*;
use crate::project::Animation;

impl EditorApp {
    pub fn camera_visible(&self) -> bool {
        self.session.preview_asset.is_none() && self.project().animation.as_ref().is_some_and(|a| a.camera.visible)
    }

    pub fn set_camera_visible(&mut self, visible: bool) {
        if self.brush_last_active() { return; }
        self.commit_transform();
        self.clear_selection();
        if let Some(animation) = &mut self.project_mut().animation {
            animation.camera.visible = visible;
            self.dirty = true;
        }
    }

    /// Parameters edit the held key; timeline operations create or remove exact-frame keys.
    pub fn edit_camera_key(&mut self, key: Option<crate::camera::CameraKey>, add: bool) {
        if self.animation_playing() || self.brush_last_active() { return; }
        let Some(animation) = &mut self.project_mut().animation else { return; };
        let frame = animation.frame;
        let before = animation.camera.clone();
        if key.is_none() && frame == 0 { return; }
        let target = if add || key.is_none() { frame } else {
            animation.camera.keys.range(..=frame).next_back().map_or(frame, |(&f, _)| f)
        };
        if let Some(key) = key {
            if !key.valid() { return; }
            animation.camera.keys.insert(target, key);
        } else {
            animation.camera.keys.remove(&target);
        }
        let after = animation.camera.clone();
        if before != after {
            self.history.record_camera(frame, before, after);
            self.dirty = true;
        }
    }

    pub fn set_layer_depth(&mut self, z: f32) {
        if !z.is_finite() { return; }
        let index = self.project().active_layer;
        let Some(layer) = self.project_mut().layers.get_mut(index) else { return; };
        let before = layer.z;
        layer.z = z.clamp(-10.0, 10.0);
        let after = layer.z;
        if before != after {
            self.history.record_depth(index, before, after);
            self.dirty = true;
        }
    }

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
        if self.session.camera_selected {
            let key = if duplicate {
                self.project().animation.as_ref().and_then(|a| a.camera.keys.range(..=a.frame).next_back().map(|(_, k)| *k)).unwrap_or_default()
            } else { crate::camera::CameraKey::default() };
            self.edit_camera_key(Some(key), true);
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
        if self.session.camera_selected {
            self.edit_camera_key(None, false);
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

    pub fn animation_held_length(&self) -> u32 {
        if self.session.camera_selected { self.project().camera_held_length() } else { self.project().held_length() }
    }

    pub fn resize_animation_frame(&mut self, longer: bool) {
        if self.animation_playing() || self.brush_last_active() {
            return;
        }
        self.commit_transform();
        let changed = if self.session.camera_selected {
            self.history.resize_camera(self.workspace.active_mut(), longer)
        } else {
            self.history.resize_drawing(self.workspace.active_mut(), longer)
        };
        if changed {
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
    fn drawing_duration_shifts_only_selected_layer_and_keeps_other_tracks_in_range() {
        let mut app = EditorApp::default();
        app.new_animation();
        app.seek_animation(4);
        app.animation_drawing(true);
        app.add_layer();
        app.seek_animation(4);
        app.animation_drawing(true);
        app.seek_animation(23);
        app.animation_drawing(true);
        app.edit_camera_key(Some(crate::camera::CameraKey::default()), true);
        app.seek_animation(4);
        app.edit_camera_key(Some(crate::camera::CameraKey::default()), true);
        app.seek_animation(1);
        app.project_mut().active_layer = 0;
        let untouched_layer = app.project().layers[1].drawings.clone();
        let untouched_camera = app.project().animation.as_ref().unwrap().camera.clone();
        app.resize_animation_frame(true);
        assert!(app.project().layers[0].drawings.contains_key(&5));
        assert_eq!(app.project().layers[1].drawings, untouched_layer);
        assert_eq!(app.project().animation.as_ref().unwrap().camera, untouched_camera);
        app.undo();
        assert!(app.project().layers[0].drawings.contains_key(&4));
        assert_eq!(app.project().layers[1].drawings, untouched_layer);
        app.redo();
        assert!(app.project().layers[0].drawings.contains_key(&5));
        assert_eq!(app.project().animation.as_ref().unwrap().camera, untouched_camera);
        app.resize_animation_frame(false);
        app.resize_animation_frame(false);
        assert!(app.project().layers[0].drawings.contains_key(&3));
        assert_eq!(app.project().layers[1].drawings, untouched_layer);
        assert_eq!(app.project().animation.as_ref().unwrap().camera, untouched_camera);
        let length = app.project().animation.as_ref().unwrap().length;
        assert!(length > 23, "untouched last keys must remain inside the timeline");
    }

    #[test]
    fn visible_camera_allows_layer_properties_structure_and_frame_commands() {
        let mut app = EditorApp::default();
        app.new_animation();
        app.project_mut().layers[0].pixels.write(0, 1, 1, [128, 64, 32, 255]);
        let pixels = app.project().layers[0].pixels.clone();
        app.set_camera_visible(true);
        app.set_layer_depth(2.0);
        assert_eq!(app.project().layers[0].z, 2.0);
        app.undo();
        assert_eq!(app.project().layers[0].z, 0.0);
        app.redo();
        assert_eq!(app.project().layers[0].z, 2.0);
        app.set_active_layer_opacity(0.5);
        assert_eq!(app.project().layers[0].opacity, 0.5);
        app.seek_animation(4);
        app.animation_drawing(true);
        assert!(app.project().layers[0].drawings.contains_key(&4));
        assert_eq!(app.project().layers[0].pixels, pixels);
        app.seek_animation(5);
        app.animation_drawing(false);
        assert!(app.project().layers[0].drawings.contains_key(&5));
        app.remove_animation_drawing();
        assert!(!app.project().layers[0].drawings.contains_key(&5));
        assert_eq!(app.project().layers[0].pixels, pixels);
        app.resize_animation_frame(true);
        assert_eq!(app.project().animation.as_ref().unwrap().length, 25);
        app.add_layer();
        assert_eq!(app.project().layers.len(), 2);
        app.duplicate_layer(0);
        assert_eq!(app.project().layers.len(), 3);
        app.delete_layer(1);
        assert_eq!(app.project().layers.len(), 2);
        app.undo();
        assert_eq!(app.project().layers.len(), 3);
        app.redo();
        assert_eq!(app.project().layers.len(), 2);
        app.open_layer_rename(0);
        assert!(app.rename.is_some());
        app.rename = None;
        let name = app.project().layers[0].name.clone();
        app.move_layer_to(0, 1);
        assert_eq!(app.project().layers[1].name, name);
        assert!(app.camera_visible());
    }

    #[test]
    fn camera_uses_the_same_timeline_key_commands_as_drawing_layers() {
        use crate::camera::CameraKey;
        let mut app = EditorApp::default();
        app.new_animation();
        app.session.camera_selected = true;
        app.seek_animation(4);
        app.animation_drawing(false);
        app.edit_camera_key(Some(CameraKey { zoom: 2.0, ..Default::default() }), false);
        app.seek_animation(12);
        app.animation_drawing(true);
        assert_eq!(app.project().animation.as_ref().unwrap().camera.keys[&12].zoom, 2.0);
        assert!(!app.project().layers[0].drawings.contains_key(&12));
        app.seek_animation(8);
        app.animation_drawing(false);
        assert_eq!(app.project().animation.as_ref().unwrap().camera.keys[&8], CameraKey::default());
        app.seek_animation(9);
        let keys = app.project().animation.as_ref().unwrap().camera.keys.clone();
        app.remove_animation_drawing();
        assert_eq!(app.project().animation.as_ref().unwrap().camera.keys, keys);
        app.seek_animation(8);
        app.set_camera_visible(true);
        app.remove_animation_drawing();
        assert!(!app.project().animation.as_ref().unwrap().camera.keys.contains_key(&8));
        app.undo();
        assert!(app.project().animation.as_ref().unwrap().camera.keys.contains_key(&8));
        app.redo();
        assert!(!app.project().animation.as_ref().unwrap().camera.keys.contains_key(&8));
        app.seek_animation(0);
        app.remove_animation_drawing();
        assert!(app.project().animation.as_ref().unwrap().camera.keys.contains_key(&0));
    }

    #[test]
    fn camera_hold_buttons_shift_only_camera_keys_with_undo() {
        let mut app = EditorApp::default();
        app.new_animation();
        app.seek_animation(4);
        app.animation_drawing(false);
        app.seek_animation(14);
        app.animation_drawing(true);
        let drawings_before = app.project().layers[0].drawings.clone();
        app.session.camera_selected = true;
        app.seek_animation(8);
        app.animation_drawing(true);
        app.seek_animation(0);
        assert_eq!(app.animation_held_length(), 8);
        app.set_camera_visible(true);
        app.resize_animation_frame(true);
        let animation = app.project().animation.as_ref().unwrap();
        assert_eq!(animation.length, 25);
        assert!(animation.camera.keys.contains_key(&9));
        assert!(app.project().layers[0].drawings.contains_key(&4));
        assert_eq!(app.project().layers[0].drawings, drawings_before);
        app.undo();
        assert!(app.camera_visible());
        assert!(app.project().animation.as_ref().unwrap().camera.keys.contains_key(&8));
        app.redo();
        assert!(app.project().animation.as_ref().unwrap().camera.keys.contains_key(&9));
        assert_eq!(app.project().layers[0].drawings, drawings_before);
        app.resize_animation_frame(false);
        assert_eq!(app.animation_held_length(), 8);
        app.set_camera_visible(false);
        app.session.camera_selected = false;
        assert_eq!(app.animation_held_length(), 4);
    }

    #[test]
    fn camera_keys_edit_hold_add_sample_and_undo_without_touching_pixels() {
        use crate::camera::CameraKey;
        let mut app = EditorApp::default();
        app.new_animation();
        assert!(!app.camera_visible());
        let pixels = app.project().layers[0].pixels.clone();
        app.seek_animation(5);
        app.edit_camera_key(Some(CameraKey { zoom: 2.0, ..Default::default() }), false);
        let track = &app.project().animation.as_ref().unwrap().camera;
        assert_eq!(track.keys.len(), 1);
        assert_eq!(track.keys[&0].zoom, 2.0);
        app.edit_camera_key(Some(CameraKey { zoom: 4.0, fade: 1.0, ..Default::default() }), true);
        assert_eq!(app.project().animation.as_ref().unwrap().camera.sample(3).zoom, 3.2);
        app.set_camera_visible(true);
        app.undo();
        assert!(app.camera_visible());
        assert!(!app.project().animation.as_ref().unwrap().camera.keys.contains_key(&5));
        app.redo();
        assert!(app.project().animation.as_ref().unwrap().camera.keys.contains_key(&5));
        assert_eq!(app.project().layers[0].pixels, pixels);
        app.set_camera_visible(false);
        app.edit_camera_key(None, false);
        assert!(!app.project().animation.as_ref().unwrap().camera.keys.contains_key(&5));
    }

    #[test]
    fn camera_preview_blocks_image_changes_but_can_be_disabled() {
        let mut app = EditorApp::default();
        app.new_animation();
        app.project_mut().layers[0].pixels.write(0, 1, 1, [128, 64, 32, 255]);
        let pixels = app.project().layers[0].pixels.clone();
        app.set_camera_visible(true);
        app.invert_layer();
        app.fix_alpha();
        app.open_hsv();
        app.input.mouse_pressed = true;
        app.input.mouse_down = true;
        app.handle_canvas_edit(CanvasView { center: Vec2::splat(256.0), width: 512, height: 512, zoom: 1.0, rotation: 0.0 });
        assert_eq!(app.project().layers.len(), 1);
        assert_eq!(app.project().layers[0].pixels, pixels);
        assert_eq!(app.project().layers[0].z, 0.0);
        assert!(!app.hsv.open);
        app.set_camera_visible(false);
        app.set_layer_depth(2.0);
        assert_eq!(app.project().layers[0].z, 2.0);
        app.undo();
        assert_eq!(app.project().layers[0].z, 0.0);
    }

    #[test]
    fn drawing_duration_keeps_camera_keys_fixed_through_undo_and_redo() {
        let mut app = EditorApp::default();
        app.new_animation();
        app.seek_animation(5);
        app.animation_drawing(false);
        app.edit_camera_key(Some(crate::camera::CameraKey::default()), true);
        app.seek_animation(0);
        let camera_before = app.project().animation.as_ref().unwrap().camera.clone();
        app.resize_animation_frame(true);
        assert!(app.project().animation.as_ref().unwrap().camera.keys.contains_key(&5));
        assert_eq!(app.project().animation.as_ref().unwrap().camera, camera_before);
        assert!(app.project().layers[0].drawings.contains_key(&6));
        app.undo();
        assert!(app.project().animation.as_ref().unwrap().camera.keys.contains_key(&5));
        app.redo();
        assert!(app.project().animation.as_ref().unwrap().camera.keys.contains_key(&5));
    }

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
