use crate::app::EditorApp;
use mega_ui::Ui;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let mut resize = false;
    let mut trim = false;
    ui.label("Canvas");
    ui.separator();
    ui.space(8.0);
    ui.label(&format!("Zoom: {:.0}%", app.session.zoom * 100.0));
    ui.space(8.0);
    let rotation = &mut app.session.rotation;
    ui.label("Rotation");
    ui.drag_float("canvas_rotation", rotation, 1.0);
    if rotation.abs() > 3600.0 {
        *rotation = rotation.rem_euclid(360.0);
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
    app.resize_w = w.round().clamp(1.0, 16384.0) as u32;
    app.resize_h = h.round().clamp(1.0, 16384.0) as u32;
    ui.horizontal(|ui| {
        if ui.button("canvas_resize", "Apply size").clicked() {
            resize = true;
        }
        if ui.button("canvas_trim", "Trim").clicked() {
            trim = true;
        }
    });

    ui.space(14.0);
    ui.label(&app.workspace.active_doc().name);
    if app.dirty {
        ui.label("Modified");
    }
    if resize {
        app.resize_canvas(ui);
    }
    if trim {
        app.trim_canvas(ui);
    }
}
