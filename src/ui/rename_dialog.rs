use crate::app::{EditorApp, RenameTarget};
use glam::Vec2;
use mega_ui::{Ui, Window};

pub struct RenameDialog {
    pub target: RenameTarget,
    pub original_name: String,
    pub name: String,
    pub original_ai_metadata: Option<crate::project::AiMetadata>,
    pub prompt: String,
    pub model: String,
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let Some(dialog) = &mut app.rename else {
        return;
    };
    let mut open = true;
    let mut accept = false;
    let mut cancel = false;
    let is_layer = matches!(dialog.target, RenameTarget::Layer { .. });
    ui.modal(
        Window::new("rename_item", dialog.target.title())
            .size(Vec2::new(420.0, if is_layer { 380.0 } else { 180.0 }))
            .open(&mut open),
        |ui| {
            ui.label("Name");
            ui.text_input("rename_name", &mut dialog.name);
            if is_layer {
                ui.space(12.0);
                ui.label("AI prompt");
                ui.text_area("layer_ai_prompt", &mut dialog.prompt, Vec2::new(0.0, 120.0));
                ui.label("AI model");
                ui.text_input("layer_ai_model", &mut dialog.model);
            }
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
    } else if accept || (!is_layer && app.input.key_enter) {
        app.apply_rename();
    }
}
