use super::*;
use crate::editing::deform::{DeformMesh, MeshGrab};
use crate::raster::SparsePixels;

pub(super) struct DeformStroke {
    pub(super) mesh: DeformMesh,
    source: SparsePixels,
    pub(super) layer: usize,
    document: String,
    grab: Option<MeshGrab>,
}

impl EditorApp {
    pub(super) fn finish_deformation(&mut self, cancel: bool) {
        let Some(stroke) = self.deformation.take() else {
            return;
        };
        if self.workspace.active_doc().id != stroke.document {
            return;
        }
        let Some(layer) = self.workspace.active_mut().layers.get_mut(stroke.layer) else {
            return;
        };
        if cancel {
            layer.pixels = stroke.source;
            self.status = "Deformation cancelled".into();
        } else {
            layer.pixels = stroke
                .mesh
                .render(&stroke.source, self.session.selection.as_ref())
                .0;
            if let Some(edit) =
                crate::history::HistoryEdit::from_diff(stroke.layer, &stroke.source, &layer.pixels)
            {
                self.history.push(edit);
                self.dirty = true;
                self.status = "Deformation applied".into();
            }
        }
        self.layers_reload = true;
        self.workspace.mark_active_thumb_dirty();
    }

    pub(super) fn handle_deformation(&mut self, view: CanvasView) {
        if self.input.key_escape {
            self.finish_deformation(true);
            return;
        }
        let point = view.screen_to_image(self.input.mouse_pos);
        if self.input.mouse_pressed
            && self.session.canvas_rect.contains(self.input.mouse_pos)
            && brush_overlaps_image(
                self.input.mouse_pos,
                self.session.deform_radius * view.zoom,
                view,
            )
        {
            if let Some(stroke) = self.deformation.as_mut() {
                stroke.grab = Some(stroke.mesh.grab(
                    point,
                    self.session.deform_radius,
                    self.session.deform_strength,
                    self.session.deform_softness,
                    self.session.deform_mode,
                ));
            } else {
                let layer = self.project().active_layer;
                let Some(target) = self.project().layers.get(layer) else {
                    return;
                };
                self.deformation = Some(DeformStroke {
                    mesh: DeformMesh::new(
                        view.width,
                        view.height,
                        self.session.deform_density as usize,
                    ),
                    source: target.pixels.clone(),
                    layer,
                    document: self.workspace.active_doc().id.clone(),
                    grab: None,
                });
                let stroke = self.deformation.as_mut().unwrap();
                stroke.grab = Some(stroke.mesh.grab(
                    point,
                    self.session.deform_radius,
                    self.session.deform_strength,
                    self.session.deform_softness,
                    self.session.deform_mode,
                ));
            }
        }
        if let Some(mut stroke) = self.deformation.take() {
            if stroke.document != self.workspace.active_doc().id
                || stroke.layer != self.project().active_layer
            {
                self.deformation = Some(stroke);
                self.finish_deformation(false);
                return;
            }
            if (self.input.mouse_down || self.input.mouse_pressed || self.input.mouse_released)
                && let Some(grab) = stroke.grab.as_ref()
            {
                stroke.mesh.drag(grab, point);
            }
            if self.input.mouse_released || (!self.input.mouse_down && !self.input.mouse_pressed) {
                stroke.grab = None;
            }
            self.deformation = Some(stroke);
        }
    }

    pub fn draw_deformation(&self, ui: &mut Ui, view: CanvasView) {
        if self.session.tool != Tool::Deform {
            return;
        }
        if self.session.deform_grid {
            let idle;
            let mesh = if let Some(stroke) = &self.deformation {
                &stroke.mesh
            } else {
                idle = DeformMesh::new(
                    view.width,
                    view.height,
                    self.session.deform_density as usize,
                );
                &idle
            };
            for y in 0..=mesh.rows {
                for x in 0..=mesh.cols {
                    let i = y * (mesh.cols + 1) + x;
                    if x < mesh.cols {
                        self.deformation_line(ui, view, mesh.positions[i], mesh.positions[i + 1]);
                    }
                    if y < mesh.rows {
                        self.deformation_line(
                            ui,
                            view,
                            mesh.positions[i],
                            mesh.positions[i + mesh.cols + 1],
                        );
                    }
                }
            }
        }
        if self.session.canvas_rect.contains(self.input.mouse_pos) {
            let radius = self.session.deform_radius * view.zoom;
            let center = self.input.mouse_pos;
            for i in 0..64 {
                let a = i as f32 * std::f32::consts::TAU / 64.0;
                let b = (i + 1) as f32 * std::f32::consts::TAU / 64.0;
                self.deformation_screen_line(
                    ui,
                    center + Vec2::new(a.cos(), a.sin()) * radius,
                    center + Vec2::new(b.cos(), b.sin()) * radius,
                    [1.0, 1.0, 1.0, 0.85],
                );
            }
        }
    }

    fn deformation_line(&self, ui: &mut Ui, view: CanvasView, a: Vec2, b: Vec2) {
        self.deformation_screen_line(
            ui,
            view.image_to_screen(a),
            view.image_to_screen(b),
            [0.2, 0.8, 1.0, 0.18],
        );
    }

    fn deformation_screen_line(&self, ui: &mut Ui, a: Vec2, b: Vec2, color: [f32; 4]) {
        // Clip overlay segments to the canvas dock (also when the view is rotated).
        let rect = self.session.canvas_rect;
        let d = b - a;
        let mut lo = 0.0f32;
        let mut hi = 1.0f32;
        for (p, q) in [
            (-d.x, a.x - rect.min.x),
            (d.x, rect.max.x - a.x),
            (-d.y, a.y - rect.min.y),
            (d.y, rect.max.y - a.y),
        ] {
            if p.abs() < f32::EPSILON {
                if q < 0.0 {
                    return;
                }
            } else if p < 0.0 {
                lo = lo.max(q / p);
            } else {
                hi = hi.min(q / p);
            }
        }
        if lo <= hi {
            ui.line(a + d * lo, a + d * hi, 1.0, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> EditorApp {
        let mut app = EditorApp::default();
        let bytes: Vec<u8> = (0..64)
            .flat_map(|y| (0..64).flat_map(move |x| [x * 3, y * 3, 100, 255]))
            .collect();
        app.project_mut().import_rgba(&bytes, 64, 64).unwrap();
        app.sync_layer_structure();
        app.session.canvas_rect = Rect::from_min_size(Vec2::ZERO, Vec2::splat(500.0));
        app.set_tool(Tool::Deform);
        app.session.deform_radius = 24.0;
        app
    }
    fn pointer(app: &mut EditorApp, p: Vec2, press: bool, down: bool) {
        let view = app.canvas_view().unwrap();
        app.input = FrameInput::default();
        app.input.mouse_pos = view.image_to_screen(p);
        app.input.mouse_pressed = press;
        app.input.mouse_down = down;
        app.input.mouse_released = !down;
        app.handle_canvas_edit(view);
    }
    #[test]
    fn deformation_stroke_previews_commits_undoes_and_redoes() {
        let mut app = app();
        let original = app.project().layers[0].pixels.clone();
        pointer(&mut app, Vec2::splat(32.0), true, true);
        pointer(&mut app, Vec2::new(38.0, 32.0), false, true);
        let preview = app.project().layers[0].pixels.clone();
        assert_eq!(preview, original);
        assert!(
            app.deformation.as_ref().unwrap().mesh.positions
                != app.deformation.as_ref().unwrap().mesh.original
        );
        pointer(&mut app, Vec2::new(38.0, 32.0), false, false);
        assert!(app.deformation.is_some());
        let positions = app.deformation.as_ref().unwrap().mesh.positions.clone();
        pointer(&mut app, Vec2::new(30.0, 30.0), true, true);
        pointer(&mut app, Vec2::new(30.0, 35.0), false, true);
        pointer(&mut app, Vec2::new(30.0, 35.0), false, false);
        assert_eq!(app.project().layers[0].pixels, original);
        assert_ne!(app.deformation.as_ref().unwrap().mesh.positions, positions);
        app.finish_deformation(false);
        let preview = app.project().layers[0].pixels.clone();
        assert_ne!(preview, original);
        assert!(app.deformation.is_none());
        app.undo();
        assert_eq!(app.project().layers[0].pixels, original);
        app.redo();
        assert_eq!(app.project().layers[0].pixels, preview);
    }
    #[test]
    fn deformation_escape_and_tool_switch_finish_correctly() {
        let mut app = app();
        let original = app.project().layers[0].pixels.clone();
        pointer(&mut app, Vec2::splat(32.0), true, true);
        pointer(&mut app, Vec2::new(38.0, 32.0), false, true);
        app.input.key_escape = true;
        app.handle_canvas_edit(app.canvas_view().unwrap());
        assert!(app.deformation.is_none());
        assert_eq!(app.project().layers[0].pixels, original);
        pointer(&mut app, Vec2::splat(32.0), true, true);
        pointer(&mut app, Vec2::new(38.0, 32.0), false, true);
        app.set_tool(Tool::Brush);
        assert!(app.deformation.is_none());
        assert_ne!(app.project().layers[0].pixels, original);
        app.undo();
        assert_eq!(app.project().layers[0].pixels, original);
    }
    #[test]
    fn animation_deformation_undo_redo_survives_frame_navigation() {
        for return_to_original in [false, true] {
            let mut app = app();
            app.project_mut().animation = Some(crate::project::Animation::new(12, 24));
            app.project_mut().flush_drawings();
            app.seek_animation(5);
            app.animation_drawing(false);
            let other = app.project().layers[0].pixels.clone();
            app.seek_animation(0);
            let original = app.project().layers[0].pixels.clone();
            pointer(&mut app, Vec2::splat(32.0), true, true);
            pointer(&mut app, Vec2::new(38.0, 32.0), false, false);
            app.finish_deformation(false);
            let applied = app.project().layers[0].pixels.clone();
            assert_ne!(applied, original);
            app.seek_animation(5);
            assert_eq!(app.project().layers[0].pixels, other);
            if return_to_original {
                app.seek_animation(0);
            }
            app.undo();
            assert_eq!(app.project().animation.as_ref().unwrap().frame, 0);
            assert_eq!(app.project().layers[0].pixels, original);
            app.seek_animation(5);
            assert_eq!(app.project().layers[0].pixels, other);
            app.redo();
            assert_eq!(app.project().animation.as_ref().unwrap().frame, 0);
            assert_eq!(app.project().layers[0].pixels, applied);
            app.seek_animation(5);
            app.seek_animation(0);
            assert_eq!(app.project().layers[0].pixels, applied);
        }
    }
    #[test]
    fn pointer_grab_locks_settings_and_recaptures_only_after_release() {
        let mut app = app();
        pointer(&mut app,Vec2::splat(32.0),true,true);
        pointer(&mut app,Vec2::new(34.0,32.0),false,true);
        app.session.deform_mode = 4;
        app.session.deform_radius = 4.0;
        app.session.deform_strength = 1.0;
        app.session.deform_softness = 0.0;
        pointer(&mut app,Vec2::new(36.0,32.0),false,true);
        let i = 16*33+16;
        assert!(app.deformation.as_ref().unwrap().mesh.positions[i].distance(Vec2::new(34.0,32.0))<0.001);
        pointer(&mut app,Vec2::new(36.0,32.0),false,false);
        assert!(app.deformation.as_ref().unwrap().grab.is_none());
        pointer(&mut app,Vec2::new(36.0,32.0),true,true);
        pointer(&mut app,Vec2::new(38.0,32.0),false,false);
        assert!(app.deformation.as_ref().unwrap().mesh.positions[i].y<32.0);
    }

}
