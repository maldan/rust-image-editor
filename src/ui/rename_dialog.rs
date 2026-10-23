use crate::app::{EditorApp, RenameTarget};
use glam::Vec2;
use mega_ui::{Ui, Window};

pub struct RenameDialog {
    pub target: RenameTarget,
    pub original_name: String,
    pub name: String,
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let Some(dialog) = &mut app.rename else {
        return;
    };
    let mut open = true;
    let mut accept = false;
    let mut cancel = false;
    ui.modal(
        Window::new("rename_item", dialog.target.title())
            .size(Vec2::new(420.0, 180.0))
            .open(&mut open),
        |ui| {
            ui.label("Name");
            ui.text_input("rename_name", &mut dialog.name);
            ui.space(12.0);
            ui.horizontal(|ui| {
                ui.add_enabled(!dialog.name.trim().is_empty(), |ui| {
                    accept = ui.button("rename_ok", "OK").clicked();
                });
                cancel = ui.button("rename_cancel", "Cancel").clicked();
            });
        },
    );
    if !open || cancel || app.input.key_escape {
        app.rename = None;
    } else if accept || app.input.key_enter {
        app.apply_rename();
    }
}
