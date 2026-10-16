use mega_ui::Ui;

use crate::app::{EditorApp, file_name};

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    ui.label("Layers");
    ui.separator();
    ui.space(8.0);

    if app.project.layers.is_empty() {
        ui.label("No layers");
    } else {
        let count = app.project.layers.len();
        let mut reorder: Option<(usize, usize)> = None;
        let mut select: Option<usize> = None;
        let mut visibility: Option<(usize, bool)> = None;

        for index in (0..count).rev() {
            let name = app.project.layers[index].name.clone();
            let mut visible = app.project.layers[index].visible;
            let selected = app.project.active_layer == index;
            let row_id = format!("layer_row_{index}");

            let response = ui.selectable(&row_id, selected, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&format!("layer_vis_{index}"), "show", &mut visible)
                        .changed()
                    {
                        visibility = Some((index, visible));
                    }
                    ui.label(&name);
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
            app.project.active_layer = index;
        }
        if let Some((index, visible)) = visibility {
            app.set_layer_visible(index, visible);
        }
        if let Some((from, to)) = reorder {
            app.move_layer_to(from, to);
            app.layer_drag = Some(to);
        }
    }

    ui.space(12.0);
    if ui.button("layer_new", "New Layer").clicked() {
        app.add_layer();
    }
    if ui.button("layer_delete", "Delete Layer").clicked() {
        app.delete_layer();
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

    ui.space(14.0);
    let name = app
        .project
        .path
        .as_deref()
        .map(file_name)
        .unwrap_or("Unsaved project");
    ui.label(name);
    if app.dirty {
        ui.label("Modified");
    }
}
