use glam::Vec2;
use mega_ui::{Ui, Window};

use crate::app::EditorApp;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let Some(action) = app.unsaved else {
        return;
    };
    let name = app
        .workspace
        .path
        .as_deref()
        .map(crate::app::file_name)
        .unwrap_or("Untitled");
    let mut open = true;
    let mut save = false;
    let mut discard = false;
    let mut cancel = false;

    ui.modal(
        Window::new("unsaved_changes", "Unsaved Changes")
            .size(Vec2::new(420.0, 160.0))
            .open(&mut open),
        |ui| {
            ui.horizontal(|ui| {
                ui.icon_colored("warning", 18.0, [0.95, 0.72, 0.18, 1.0]);
                ui.label(&format!("The document '{name}' has been modified."));
            });
            ui.label("Do you want to save it?");
            ui.space(16.0);
            ui.horizontal(|ui| {
                save = ui.button("unsaved_yes", "Yes").clicked();
                discard = ui.button("unsaved_no", "No").clicked();
                cancel = ui.button("unsaved_cancel", "Cancel").clicked();
            });
        },
    );

    if !open || cancel || app.input.key_escape {
        app.cancel_unsaved_prompt();
        return;
    }
    if save {
        if app.try_save_project(ui) {
            app.finish_unsaved_prompt(action, ui);
        }
        return;
    }
    if discard {
        app.finish_unsaved_prompt(action, ui);
    }
}
