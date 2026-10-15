use mega_ui::Ui;

use crate::app::{file_name, EditorApp};

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    ui.label("Assets");
    ui.separator();
    match app.image_size {
        Some((width, height)) => {
            ui.label("  main/image.png");
            ui.label(&format!("  {width} × {height}"));
        }
        None => ui.label("  No imported assets"),
    }

    ui.space(14.0);
    ui.label("Layers");
    ui.separator();
    if app.image_size.is_some() {
        ui.label("  Image");
    } else {
        ui.label("  Empty");
    }

    ui.space(14.0);
    if ui.button("fit", "Fit image").clicked() {
        app.fit_image = true;
    }
    ui.label(&format!("Zoom: {:.0}%", app.zoom * 100.0));

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
