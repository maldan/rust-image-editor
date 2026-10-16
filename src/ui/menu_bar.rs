use mega_ui::Ui;

use crate::app::EditorApp;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    ui.menu("file", "File", |ui| {
        if ui.menu_item("new", "New Project").clicked() {
            app.new_project(ui);
        }
        if ui.menu_item("open", "Open Project…").clicked() {
            app.open_project(ui);
        }
        if ui.menu_item("save", "Save Project").clicked() {
            app.save_project(ui);
        }
        ui.separator();
        if ui.menu_item("import", "Import PNG…").clicked() {
            app.import_png(ui);
        }
        if ui.menu_item("export", "Export PNG…").clicked() {
            app.export_png(ui);
        }
    });

    ui.menu("edit", "Edit", |ui| {
        if ui.menu_item("undo", "Undo").clicked() {
            app.undo();
        }
        if ui.menu_item("redo", "Redo").clicked() {
            app.redo();
        }
        ui.separator();
        if ui.menu_item("paste_image", "Paste Image").clicked() {
            app.paste_image(ui);
        }
    });
}
