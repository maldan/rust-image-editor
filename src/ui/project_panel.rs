use mega_ui::Ui;

use crate::app::EditorApp;
use crate::host::FrameInput;

struct LayerCommand {
    select: Option<usize>,
    visibility: Option<(usize, bool)>,
    copy: Option<usize>,
    rename: Option<usize>,
    delete: Option<usize>,
    reorder: Option<(usize, usize)>,
    opacity: Option<f32>,
    new_layer: bool,
    resize: bool,
    trim: bool,
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let rows: Vec<(String, bool, f32)> = app
        .project()
        .layers
        .iter()
        .map(|layer| (layer.name.clone(), layer.visible, layer.opacity))
        .collect();
    let active = app.project().active_layer;
    let image_size = app.image_size;
    let doc_name = app.workspace.active_doc().name.clone();
    let dirty = app.dirty;
    let command = controls(
        ui,
        &rows,
        active,
        &app.input,
        &mut app.layer_drag,
        app.session.zoom,
        &mut app.session.rotation,
        image_size,
        &mut app.resize_w,
        &mut app.resize_h,
        &doc_name,
        dirty,
    );
    if let Some(index) = command.select {
        app.project_mut().active_layer = index;
    }
    if let Some((index, visible)) = command.visibility {
        app.set_layer_visible(index, visible);
    }
    if let Some(index) = command.rename {
        app.open_layer_rename(index);
    } else if let Some(index) = command.copy {
        app.copy_layer_at(index, ui);
        app.layer_drag = None;
    } else if let Some(index) = command.delete {
        app.delete_layer(index);
    } else if let Some((from, to)) = command.reorder {
        app.move_layer_to(from, to);
        app.layer_drag = Some(to);
    }
    if let Some(opacity) = command.opacity {
        app.set_active_layer_opacity(opacity);
    }
    if command.new_layer {
        app.add_layer();
    }
    if command.resize {
        app.resize_canvas(ui);
    }
    if command.trim {
        app.trim_canvas(ui);
    }
}

fn controls(
    ui: &mut Ui,
    rows: &[(String, bool, f32)],
    active: usize,
    input: &FrameInput,
    drag: &mut Option<usize>,
    zoom: f32,
    rotation: &mut f32,
    image_size: Option<(u32, u32)>,
    resize_w: &mut u32,
    resize_h: &mut u32,
    doc_name: &str,
    dirty: bool,
) -> LayerCommand {
    let mut command = LayerCommand {
        select: None,
        visibility: None,
        copy: None,
        rename: None,
        delete: None,
        reorder: None,
        opacity: None,
        new_layer: false,
        resize: false,
        trim: false,
    };
    ui.label("Layers");
    ui.separator();
    ui.space(8.0);

    if rows.is_empty() {
        ui.label("No layers");
    } else {
        for index in (0..rows.len()).rev() {
            let (name, visible0, _) = &rows[index];
            let mut visible = *visible0;
            let row_id = format!("layer_row_{index}");
            let response = ui.selectable(&row_id, active == index, |ui| {
                ui.horizontal(|ui| {
                    if ui.icon_button(
                        &format!("layer_vis_{index}"),
                        if visible {
                            "visibility"
                        } else {
                            "visibility_off"
                        },
                        visible,
                    ) {
                        visible = !visible;
                        command.visibility = Some((index, visible));
                    }
                    let name_width = (ui.available_size().x / ui.scale() - 170.0).max(1.0);
                    ui.label_fixed(name_width, name);
                    ui.spacer();
                    if ui.icon_button(&format!("layer_delete_{index}"), "delete", false) {
                        command.delete = Some(index);
                    }
                    if ui.icon_button(&format!("layer_rename_{index}"), "edit", false) {
                        command.rename = Some(index);
                    }
                    if ui.icon_button(&format!("layer_copy_{index}"), "copy", false) {
                        command.copy = Some(index);
                    }
                });
            });
            if response.clicked() {
                command.select = Some(index);
            }
            if response.hovered && input.mouse_pressed {
                *drag = Some(index);
            }
            if let Some(from) = *drag
                && response.hovered
                && input.mouse_down
                && from != index
            {
                command.reorder = Some((from, index));
            }
        }
        if input.mouse_released {
            *drag = None;
        }

        ui.space(12.0);
        if let Some((_, _, opacity)) = rows.get(active) {
            let mut opacity = *opacity;
            ui.label(&format!("Opacity: {:.0}%", opacity * 100.0));
            if ui
                .slider("layer_opacity", &mut opacity, 0.0..=1.0)
                .changed()
            {
                command.opacity = Some(opacity);
            }
        }
    }

    ui.space(12.0);
    if ui.button("layer_new", "New Layer").clicked() {
        command.new_layer = true;
    }

    ui.space(16.0);
    ui.label("Canvas");
    ui.separator();
    ui.space(8.0);
    ui.label(&format!("Zoom: {:.0}%", zoom * 100.0));
    ui.space(8.0);
    ui.label("Rotation");
    ui.drag_float("canvas_rotation", rotation, 1.0);
    if rotation.abs() > 3600.0 {
        *rotation = rotation.rem_euclid(360.0);
    }

    ui.space(14.0);
    match image_size {
        Some((width, height)) => ui.label(&format!("{width} × {height}")),
        None => ui.label("No image"),
    }

    ui.space(12.0);
    ui.label("Resize canvas");
    let mut w = *resize_w as f32;
    let mut h = *resize_h as f32;
    ui.grid(2, |ui| {
        ui.grid_cell(|ui| {
            ui.property("W", 0.18, |ui| {
                ui.drag_float("canvas_w", &mut w, 1.0);
            })
        });
        ui.grid_cell(|ui| {
            ui.property("H", 0.18, |ui| {
                ui.drag_float("canvas_h", &mut h, 1.0);
            })
        });
    });
    *resize_w = w.round().clamp(1.0, 16384.0) as u32;
    *resize_h = h.round().clamp(1.0, 16384.0) as u32;
    ui.horizontal(|ui| {
        if ui.button("canvas_resize", "Apply size").clicked() {
            command.resize = true;
        }
        if ui.button("canvas_trim", "Trim").clicked() {
            command.trim = true;
        }
    });

    ui.space(14.0);
    ui.label(doc_name);
    if dirty {
        ui.label("Modified");
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    #[test]
    fn pencil_opens_rename_for_its_row_without_selecting_or_cloning_it() {
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
        let mut icons: Vec<_> = output
            .draw_list
            .iter()
            .filter(|c| {
                c.rect.min.x > 180.0
                    && c.rect.max.y < 200.0
                    && (c.rect.width() - 16.0).abs() < 0.1
                    && (c.rect.height() - 16.0).abs() < 0.1
            })
            .collect();
        icons.sort_by(|a, b| {
            a.rect
                .min
                .y
                .total_cmp(&b.rect.min.y)
                .then_with(|| a.rect.min.x.total_cmp(&b.rect.min.x))
        });
        // Trailing widgets are allocated right-to-left: copy, pencil, delete on screen.
        let icon = icons[1];
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
        let dialog = app.rename.as_ref().expect("pencil opens the rename modal");
        assert!(matches!(
            &dialog.target,
            crate::app::RenameTarget::Layer { index: 1, .. }
        ));
        assert_eq!(dialog.name, "Layer 2");
        assert_eq!(app.project().active_layer, 0);
        assert_eq!(app.project().layers.len(), 2);
        assert!(app.layer_drag.is_none());
    }

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
                c.rect.min.x > 180.0
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
