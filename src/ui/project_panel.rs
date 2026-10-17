use mega_ui::Ui;

use crate::app::EditorApp;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    ui.label("Layers");
    ui.separator();
    ui.space(8.0);

    let project = app.project();
    if project.layers.is_empty() {
        ui.label("No layers");
    } else {
        let count = project.layers.len();
        let mut reorder: Option<(usize, usize)> = None;
        let mut select: Option<usize> = None;
        let mut delete: Option<usize> = None;
        let mut copy: Option<usize> = None;
        let mut visibility: Option<(usize, bool)> = None;

        for index in (0..count).rev() {
            let name = app.project().layers[index].name.clone();
            let mut visible = app.project().layers[index].visible;
            let selected = app.project().active_layer == index;
            let row_id = format!("layer_row_{index}");

            let response = ui.selectable(&row_id, selected, |ui| {
                ui.horizontal(|ui| {
                    if ui.icon_button(&format!("layer_vis_{index}"), if visible { "visibility" } else { "visibility_off" }, visible) {
                        visible = !visible;
                        visibility = Some((index, visible));
                    }
                    let name_width = (ui.available_size().x / ui.scale() - 142.0).max(1.0);
                    ui.label_fixed(name_width, &name);
                    ui.spacer();
                    if ui.icon_button(&format!("layer_delete_{index}"), "delete", false) {
                        delete = Some(index);
                    }
                    if ui.icon_button(&format!("layer_copy_{index}"), "copy", false) {
                        copy = Some(index);
                    }
                });
            });

            if response.clicked() {
                select = Some(index);
            }

            if response.hovered && app.input.mouse_pressed {
                app.layer_drag = Some(index);
            }
            if let Some(from) = app.layer_drag
                && response.hovered
                && app.input.mouse_down
                && from != index
            {
                reorder = Some((from, index));
            }
        }

        if app.input.mouse_released {
            app.layer_drag = None;
        }

        if let Some(index) = select {
            app.project_mut().active_layer = index;
        }
        if let Some((index, visible)) = visibility {
            app.set_layer_visible(index, visible);
        }
        if let Some(index) = copy {
            app.copy_layer_at(index, ui);
            app.layer_drag = None;
        } else if let Some(index) = delete {
            app.delete_layer(index);
        } else if let Some((from, to)) = reorder {
            app.move_layer_to(from, to);
            app.layer_drag = Some(to);
        }

        ui.space(12.0);
        let active = app.project().active_layer;
        if let Some(layer) = app.project().layers.get(active) {
            let mut opacity = layer.opacity;
            ui.label(&format!("Opacity: {:.0}%", opacity * 100.0));
            if ui
                .slider("layer_opacity", &mut opacity, 0.0..=1.0)
                .changed()
            {
                app.set_active_layer_opacity(opacity);
            }
        }
    }

    ui.space(12.0);
    if ui.button("layer_new", "New Layer").clicked() {
        app.add_layer();
    }

    ui.space(16.0);
    ui.label("Canvas");
    ui.separator();
    ui.space(8.0);
    ui.label(&format!("Zoom: {:.0}%", app.zoom * 100.0));
    ui.space(8.0);
    ui.label("Rotation");
    ui.drag_float("canvas_rotation", &mut app.rotation, 1.0);
    if app.rotation.abs() > 3600.0 {
        app.rotation = app.rotation.rem_euclid(360.0);
    }

    ui.space(14.0);
    match app.image_size {
        Some((width, height)) => ui.label(&format!("{width} × {height}")),
        None => ui.label("No image"),
    }

    ui.space(12.0);
    ui.label("Resize canvas");
    let mut w = app.resize_w as f32;
    let mut h = app.resize_h as f32;
    ui.grid(2, |ui| {
        ui.grid_cell(|ui| ui.property("W", 0.18, |ui| { ui.drag_float("canvas_w", &mut w, 1.0); }));
        ui.grid_cell(|ui| ui.property("H", 0.18, |ui| { ui.drag_float("canvas_h", &mut h, 1.0); }));
    });
    app.resize_w = w.round().clamp(1.0, 16384.0) as u32;
    app.resize_h = h.round().clamp(1.0, 16384.0) as u32;
    ui.horizontal(|ui| {
        if ui.button("canvas_resize", "Apply size").clicked() {
            app.resize_canvas(ui);
        }
        if ui.button("canvas_trim", "Trim").clicked() {
            app.trim_canvas(ui);
        }
    });

    ui.space(14.0);
    ui.label(&app.workspace.active_doc().name);
    if app.dirty {
        ui.label("Modified");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    #[test]
    fn right_hand_icon_deletes_its_row_without_changing_active_layer_and_can_undo() {
        let mut app = EditorApp::default();
        app.project_mut()
            .import_rgba(&[1, 2, 3, 255], 1, 1)
            .unwrap();
        app.project_mut().add_layer();
        app.project_mut().active_layer = 0;
        app.refresh_image().unwrap();
        let mut ui = Ui::new();
        ui.load_builtin_icons();
        let mut output = mega_ui::UiOutput::default();
        for _ in 0..3 {
            ui.begin_frame(mega_ui::UiInput {
                viewport: Vec2::new(400.0, 900.0),
                ..Default::default()
            });
            ui.horizontal(|ui| ui.flex_width(360.0, |ui| draw(ui, &mut app)));
            output = ui.end_frame();
        }
        let icon = output
            .draw_list
            .iter()
            .find(|c| {
                c.rect.min.x > 320.0
                    && c.rect.max.y < 200.0
                    && (c.rect.width() - 16.0).abs() < 0.1
                    && (c.rect.height() - 16.0).abs() < 0.1
            })
            .expect("Delete icon pinned to the right of the top row");
        let position = (icon.rect.min + icon.rect.max) * 0.5;
        for pressed in [true, false] {
            app.input.mouse_pos = position;
            app.input.mouse_pressed = pressed;
            app.input.mouse_down = pressed;
            app.input.mouse_released = !pressed;
            ui.begin_frame(mega_ui::UiInput {
                viewport: Vec2::new(400.0, 900.0),
                mouse_pos: position,
                mouse_pressed: pressed,
                mouse_down: pressed,
                mouse_released: !pressed,
                ..Default::default()
            });
            ui.horizontal(|ui| ui.flex_width(360.0, |ui| draw(ui, &mut app)));
            ui.end_frame();
        }
        assert_eq!(app.project().layers.len(), 1);
        assert_eq!(app.project().layers[0].name, "Layer 1");
        assert_eq!(app.project().active_layer, 0);
        app.undo();
        assert_eq!(app.project().layers.len(), 2);
        assert_eq!(app.project().active_layer, 0);
        app.redo();
        assert_eq!(app.project().layers.len(), 1);
    }
}
