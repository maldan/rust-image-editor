use glam::{Vec2, Vec3};
use mega_render::{GizmoMode, Hud, InputFrame as HudInput, Projection, view_gizmo};
use mega_ui::{Rect, Ui};

use crate::app::EditorApp;
use crate::host::{CANVAS_TEXTURE, FrameInput};
use crate::scene_3d::{Primitive, SceneDocument};

pub(super) fn viewport(ui: &mut Ui, app: &mut EditorApp) {
    let area = ui.area("scene_viewport", ui.available_size().max(Vec2::ONE));
    let rect = area.rect;
    let window = app.session.viewport_size;
    app.session.workspace_rect = rect;
    if rect.width() < 1.0
        || rect.height() < 1.0
        || rect.min.min_element() < 0.0
        || rect.max.x > window.x
        || rect.max.y > window.y
    {
        return;
    }
    app.scene_editor.viewport = Some(rect);
    let Some(index) = app.scene_editor.active else {
        return;
    };
    let scene = &mut app.scene_editor.documents[index];
    scene.view_aspect = rect.width() / rect.height();
    if scene.camera.tick_snap(app.input.dt) {
        ui.request_repaint();
    }
    let navigation = ViewNavigation::new(rect, ui.scale());
    let pointer_available = ui.enabled()
        && !app.new_document_open
        && !ui.context_menu_open()
        && !ui.pointer_over_popup_at(app.input.mouse_pos);
    if pointer_available {
        let mut over_navigation = false;
        if let Some(navigation) = &navigation
            && scene.drag.is_none()
            && !scene.orbiting
            && !scene.panning
        {
            over_navigation = navigation.handle(ui, scene, &app.input, area.hovered);
        }
        interact(
            scene,
            &app.input,
            rect,
            area.hovered && !over_navigation,
            ui.scale(),
        );
    } else {
        scene.finish_edit(false);
        scene.orbiting = false;
        scene.panning = false;
        scene.gizmo_hover = None;
    }
    ui.texture_rect(
        CANVAS_TEXTURE,
        rect,
        (rect.min / window).to_array(),
        (rect.max / window).to_array(),
    );
    if let Some(navigation) = navigation {
        navigation.draw(
            ui,
            &mut app.scene_editor.view_hud,
            scene,
            if pointer_available {
                app.input.mouse_pos
            } else {
                Vec2::splat(-10000.0)
            },
        );
    }
}

struct ViewNavigation {
    origin: Vec2,
    size: Vec2,
    scale: f32,
    projection_button: Rect,
}

impl ViewNavigation {
    fn new(rect: Rect, scale: f32) -> Option<Self> {
        let size = (rect.max - rect.min) / scale;
        let widget = view_gizmo::widget_rect(size);
        let projection = Vec2::new(widget.min.x, widget.max.y + 3.0);
        let button_size = Vec2::new(widget.width(), 25.0);
        (projection.x >= 0.0 && projection.y + button_size.y <= size.y).then_some(Self {
            origin: rect.min,
            size,
            scale,
            projection_button: Rect::from_min_size(
                rect.min + projection * scale,
                button_size * scale,
            ),
        })
    }

    fn handle(
        &self,
        ui: &mut Ui,
        scene: &mut SceneDocument,
        input: &FrameInput,
        hovered: bool,
    ) -> bool {
        if !hovered {
            return false;
        }
        let local = (input.mouse_pos - self.origin) / self.scale;
        if self.projection_button.contains(input.mouse_pos) {
            ui.set_mouse_cursor(mega_ui::CursorIcon::Pointer);
            if input.mouse_pressed {
                scene.camera.toggle_projection();
                ui.request_repaint();
            }
            return true;
        }
        if let Some(axis) = view_gizmo::hit_test(&scene.camera.render_camera(), self.size, local) {
            ui.set_mouse_cursor(mega_ui::CursorIcon::Pointer);
            if input.mouse_pressed {
                scene.camera.snap_to_axis(axis);
                ui.request_repaint();
            }
        }
        view_gizmo::contains_cursor(self.size, local)
    }

    fn draw(&self, ui: &mut Ui, hud: &mut Hud, scene: &SceneDocument, pointer: Vec2) {
        let input = HudInput::default();
        hud.begin(&input, self.size);
        hud.text(Vec2::ZERO, "XYZ", [1.0; 4], 1.0);
        let glyphs = std::array::from_fn::<_, 3, _>(|index| hud.primitives()[index].uv_min);
        hud.begin(&input, self.size);
        view_gizmo::draw(
            hud,
            &scene.camera.render_camera(),
            self.size,
            (pointer - self.origin) / self.scale,
        );
        for quad in hud.primitives() {
            let min = self.origin + quad.rect.min * self.scale;
            if quad.uv_min == quad.uv_max {
                let max = self.origin + quad.rect.max * self.scale;
                ui.fill_rect(Rect { min, max }, quad.color);
            } else if let Some(index) = glyphs.iter().position(|uv| *uv == quad.uv_min) {
                ui.text_at(min + Vec2::splat(2.0 * self.scale), ["X", "Y", "Z"][index]);
            }
        }
        let button = self.projection_button;
        let color = if button.contains(pointer) {
            [0.18, 0.22, 0.28, 1.0]
        } else {
            [0.11, 0.13, 0.17, 1.0]
        };
        ui.fill_round(button, 4.0 * self.scale, color);
        let label = match scene.camera.projection {
            Projection::Perspective => "Perspective",
            Projection::Orthographic => "Orthographic",
        };
        ui.text_at(button.min + Vec2::new(12.0, 4.0) * self.scale, label);
    }
}

fn interact(scene: &mut SceneDocument, input: &FrameInput, rect: Rect, hovered: bool, scale: f32) {
    if input.key_escape && scene.drag.is_some() {
        scene.finish_edit(true);
        scene.gizmo_hover = None;
        return;
    }
    if let Some(drag) = &mut scene.drag {
        if (input.mouse_down || input.mouse_released)
            && let Some(index) = scene.objects.selected
        {
            drag.update(
                &mut scene.objects.items[index],
                scene.camera.ray(rect, input.mouse_pos),
                input.mouse_pos,
            );
        }
        if !input.mouse_down {
            scene.finish_edit(false);
        }
        return;
    }

    if hovered && input.mouse_right_pressed {
        scene.orbiting = true;
    }
    if hovered && input.mouse_middle_pressed {
        scene.panning = true;
    }
    scene.orbiting &= input.mouse_right_down;
    scene.panning &= input.mouse_middle_down;
    if scene.orbiting && !input.mouse_right_pressed {
        scene.camera.orbit(input.mouse_delta / scale);
    } else if scene.panning && !input.mouse_middle_pressed {
        scene.camera.pan(input.mouse_delta, rect.height());
    }
    if hovered && input.scroll_delta.y != 0.0 {
        scene.camera.zoom(input.scroll_delta.y);
    }
    if hovered
        && input.key_tool_fill
        && let Some(object) = scene.objects.selected()
    {
        scene.camera.focus(object, rect.width() / rect.height());
    }
    if !hovered || scene.orbiting || scene.panning {
        scene.gizmo_hover = None;
        return;
    }
    let gizmo = scene.grab_gizmo(rect, input.mouse_pos, scale);
    scene.gizmo_hover = gizmo.as_ref().map(|drag| drag.axis);
    if !input.mouse_pressed {
        return;
    }
    scene.finish_edit(false);
    if let Some(gizmo) = gizmo {
        scene.begin_edit(scene.objects.clone());
        scene.drag = Some(gizmo);
    } else {
        scene.objects.selected = scene.pick(scene.camera.ray(rect, input.mouse_pos));
    }
}

pub(super) fn tools(ui: &mut Ui, app: &mut EditorApp) {
    let rect = app.session.workspace_rect;
    let Some(scene) = app.scene_editor.current_mut() else {
        return;
    };
    let modes = [GizmoMode::Translate, GizmoMode::Rotate, GizmoMode::Scale];
    let mut mode = modes
        .iter()
        .position(|mode| *mode == scene.gizmo_mode)
        .unwrap();
    if ui
        .toggle("gizmo_mode", &mut mode, &["Move", "Rotate", "Scale"])
        .changed()
    {
        scene.finish_edit(false);
        scene.gizmo_mode = modes[mode];
        scene.gizmo_hover = None;
        ui.request_repaint();
    }
    let mut add = None;
    ui.grid(2, |ui| {
        for primitive in [
            Primitive::Cube,
            Primitive::Sphere,
            Primitive::Plane,
            Primitive::Cylinder,
            Primitive::Cone,
        ] {
            ui.grid_cell(|ui| {
                if ui
                    .button(primitive.name(), &format!("+ {}", primitive.name()))
                    .clicked()
                {
                    add = Some(primitive);
                }
            });
        }
    });
    if let Some(primitive) = add {
        scene.add(primitive);
        if let Some(object) = scene.objects.selected()
            && (rect.width() < 1.0
                || rect.height() < 1.0
                || scene
                    .camera
                    .project(rect, object.position)
                    .is_none_or(|point| !rect.contains(point)))
        {
            scene
                .camera
                .focus(object, rect.width().max(1.0) / rect.height().max(1.0));
        }
        ui.request_repaint();
    }
}

pub(super) fn camera(ui: &mut Ui, app: &mut EditorApp) {
    let Some(scene) = app.scene_editor.current_mut() else {
        return;
    };
    let mut mode = usize::from(scene.camera.projection == Projection::Orthographic);
    if ui
        .toggle(
            "scene_projection",
            &mut mode,
            &["Perspective", "Orthographic"],
        )
        .changed()
    {
        scene.camera.toggle_projection();
        ui.request_repaint();
    }
    ui.label("Vertical FOV (°)");
    let mut fov = scene.camera.fov_y.to_degrees();
    ui.add_enabled(scene.camera.projection == Projection::Perspective, |ui| {
        ui.drag_float("scene_fov", &mut fov, 1.0);
    });
    if scene.camera.projection == Projection::Perspective && fov.is_finite() {
        let radians = fov.clamp(10.0, 120.0).to_radians();
        if (radians - scene.camera.fov_y).abs() > f32::EPSILON {
            scene.camera.fov_y = radians;
            ui.request_repaint();
        }
    }
}

enum ObjectAction {
    Select,
    ToggleVisibility,
    Clone,
    Delete,
    Rename,
}

pub(super) fn objects(ui: &mut Ui, app: &mut EditorApp) {
    let mouse_down = app.input.mouse_down;
    let row_input = !ui.context_menu_open() && !ui.pointer_over_popup_at(app.input.mouse_pos);
    let Some(scene) = app.scene_editor.current_mut() else {
        return;
    };
    if scene.objects.items.is_empty() {
        ui.label("Add an object from Scene Tools.");
        return;
    }
    let mut action = None;
    for (index, object) in scene.objects.items.iter().enumerate() {
        let id = format!("scene_object_{}", object.id);
        let row = ui.selectable(&id, scene.objects.selected == Some(index), |ui| {
            ui.horizontal(|ui| {
                let icon = if object.visible {
                    "visibility"
                } else {
                    "visibility_off"
                };
                if ui.icon_button("visibility", icon, object.visible) && row_input {
                    action = Some((index, ObjectAction::ToggleVisibility));
                }
                ui.label(&object.name);
            });
        });
        if row_input && row.clicked() && action.is_none() {
            action = Some((index, ObjectAction::Select));
        }
        ui.context_menu(&format!("{id}_menu"), row.hovered, |ui| {
            for (label, command) in [
                ("Clone", ObjectAction::Clone),
                ("Rename…", ObjectAction::Rename),
                ("Delete", ObjectAction::Delete),
            ] {
                if ui.menu_item(label, label).clicked() {
                    action = Some((index, command));
                }
            }
        });
    }
    if let Some((index, action)) = action {
        match action {
            ObjectAction::Select => {
                scene.finish_edit(false);
                scene.objects.selected = Some(index);
            }
            ObjectAction::ToggleVisibility => scene.toggle_object_visibility(index),
            ObjectAction::Clone => scene.clone_object(index),
            ObjectAction::Delete => scene.delete_object(index),
            ObjectAction::Rename => {
                let target = crate::app::RenameTarget::SceneObject {
                    scene_id: scene.id,
                    object_id: scene.objects.items[index].id,
                };
                scene.finish_edit(false);
                app.open_rename(target);
                ui.request_repaint();
                return;
            }
        }
        ui.request_repaint();
    }
    if scene.objects.selected.is_some() {
        ui.separator();
        ui.label("Transform");
        transform(ui, scene, mouse_down);
    }
}

fn transform(ui: &mut Ui, scene: &mut SceneDocument, mouse_down: bool) {
    let Some(index) = scene.objects.selected else {
        return;
    };
    let mut object = scene.objects.items[index].clone();
    ui.id_scope(&format!("scene_{}_object_{}", scene.id, object.id), |ui| {
        vector_field(
            ui,
            "Position",
            &mut object.position,
            0.1,
            Vec3::ZERO,
            -10000.0..=10000.0,
        );
        vector_field(
            ui,
            "Rotation (°)",
            &mut object.rotation,
            1.0,
            Vec3::ZERO,
            -36000.0..=36000.0,
        );
        vector_field(
            ui,
            "Scale",
            &mut object.scale,
            0.05,
            Vec3::ONE,
            0.01..=1000.0,
        );
    });
    if object != scene.objects.items[index] {
        scene.begin_edit(scene.objects.clone());
        scene.objects.items[index] = object;
        ui.request_repaint();
    }
    if !mouse_down && scene.drag.is_none() {
        scene.finish_edit(false);
    }
}

fn vector_field(
    ui: &mut Ui,
    label: &str,
    vector: &mut Vec3,
    step: f32,
    default: Vec3,
    limits: std::ops::RangeInclusive<f32>,
) {
    ui.label(label);
    let mut edited = *vector;
    ui.vec3(label, &mut edited, step, default);
    for (index, value) in edited.to_array().into_iter().enumerate() {
        if value.is_finite() {
            vector[index] = value.clamp(*limits.start(), *limits.end());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_3d::{SceneEditor, axis_parameter};
    use mega_render::{GizmoAxis, ViewAxis};

    #[test]
    fn viewport_selects_nearest_object_and_undoes_a_whole_axis_drag() {
        let mut editor = SceneEditor::default();
        editor.active = Some(editor.create());
        let scene = editor.current_mut().unwrap();
        scene.add(Primitive::Cube);
        scene.add(Primitive::Sphere);
        let origin = scene.objects.items[0].position;
        let toward_eye = (scene.camera.eye() - origin).normalize();
        scene.objects.items[1].position = origin - toward_eye * 2.0;
        scene.objects.selected = None;
        let rect = Rect::from_min_size(Vec2::new(100.0, 60.0), Vec2::new(800.0, 600.0));
        let center = scene.camera.project(rect, origin).unwrap();
        let mut input = FrameInput::default();
        input.mouse_pos = center;
        input.mouse_pressed = true;
        input.mouse_down = true;
        interact(scene, &input, rect, true, 1.0);
        assert_eq!(scene.objects.selected, Some(0));

        let size = scene.gizmo_size(rect, 1.0, origin);
        input.mouse_pos = scene.camera.project(rect, origin + Vec3::X * size).unwrap();
        interact(scene, &input, rect, true, 1.0);
        assert_eq!(scene.drag.as_ref().unwrap().axis, GizmoAxis::X);
        let start =
            axis_parameter(scene.camera.ray(rect, input.mouse_pos), origin, Vec3::X).unwrap();
        input.mouse_pressed = false;
        for (distance, held) in [(0.6, true), (1.2, false)] {
            input.mouse_pos = scene
                .camera
                .project(rect, origin + Vec3::X * (start + distance))
                .unwrap();
            input.mouse_down = held;
            input.mouse_released = !held;
            interact(scene, &input, rect, true, 1.0);
        }
        let moved = scene.objects.items[0].position;
        assert!(moved.distance(origin + Vec3::X * 1.2) < 0.001);
        scene.undo();
        assert_eq!(scene.objects.items[0].position, origin);
        scene.redo();
        assert_eq!(scene.objects.items[0].position, moved);
    }

    #[test]
    fn shared_gizmo_rotates_and_scales_in_orthographic_view() {
        let mut editor = SceneEditor::default();
        editor.active = Some(editor.create());
        let scene = editor.current_mut().unwrap();
        scene.add(Primitive::Cube);
        let origin = scene.objects.items[0].position;
        let rect = Rect::from_min_size(Vec2::ZERO, Vec2::new(800.0, 600.0));

        scene.camera.snap_to_axis(ViewAxis::PosZ);
        assert!(scene.camera.tick_snap(0.25));
        assert_eq!(scene.camera.projection, Projection::Orthographic);
        assert!(
            (scene.camera.eye() - origin)
                .normalize()
                .abs_diff_eq(Vec3::Z, 1e-5)
        );

        scene.gizmo_mode = GizmoMode::Rotate;
        let size = scene.gizmo_size(rect, 1.0, origin);
        let diagonal = (Vec3::X + Vec3::Y).normalize();
        let start = scene
            .camera
            .project(rect, origin + diagonal * size)
            .unwrap();
        let end = scene
            .camera
            .project(rect, origin + Vec3::Z.cross(diagonal) * size)
            .unwrap();
        let mut rotate = scene.grab_gizmo(rect, start, 1.0).unwrap();
        assert_eq!(rotate.axis, GizmoAxis::Z);
        rotate.update(
            &mut scene.objects.items[0],
            scene.camera.ray(rect, end),
            end,
        );
        assert!((scene.objects.items[0].orientation() * Vec3::X).abs_diff_eq(Vec3::Y, 1e-4));

        scene.gizmo_mode = GizmoMode::Scale;
        let start = scene.camera.project(rect, origin).unwrap();
        let end = start + Vec2::new(85.0, 0.0);
        let mut scale = scene.grab_gizmo(rect, start, 1.0).unwrap();
        assert_eq!(scale.axis, GizmoAxis::Uniform);
        scale.update(
            &mut scene.objects.items[0],
            scene.camera.ray(rect, end),
            end,
        );
        assert!(
            scene.objects.items[0]
                .scale
                .abs_diff_eq(Vec3::splat(std::f32::consts::E), 1e-4)
        );

        scene.camera.orbit(Vec2::X);
        assert_eq!(scene.camera.projection, Projection::Perspective);
        scene.camera.toggle_projection();
        scene.camera.orbit(Vec2::X);
        assert_eq!(scene.camera.projection, Projection::Orthographic);
    }
}
