use crate::{app::EditorApp, editor_settings::EditorSettings};
use glam::Vec2;
use mega_ui::{Ui, Window};

pub struct SettingsDialog {
    pub settings: EditorSettings,
    pub message: String,
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let Some(dialog) = &mut app.settings_dialog else {
        return;
    };
    let mut open = true;
    let mut save = false;
    let mut cancel = false;
    ui.modal(
        Window::new("editor_settings", "Settings")
            .size(Vec2::new(360.0, 180.0))
            .open(&mut open),
        |ui| {
            ui.horizontal(|ui| {
                ui.label("UI Scale");
                ui.drag_float("editor_ui_scale", &mut dialog.settings.ui_scale, 0.01);
                dialog.settings.ui_scale = if dialog.settings.ui_scale.is_finite() {
                    dialog.settings.ui_scale.clamp(0.5, 3.0)
                } else {
                    1.0
                };
            });
            if !dialog.message.is_empty() {
                ui.label(&dialog.message);
            }
            ui.space(12.0);
            ui.horizontal(|ui| {
                save = ui.button("editor_settings_save", "Save").clicked();
                cancel = ui.button("editor_settings_cancel", "Cancel").clicked();
            });
        },
    );
    if !open || cancel || app.input.key_escape {
        app.settings_dialog = None;
        ui.request_repaint();
    } else if save {
        match dialog.settings.save() {
            Ok(()) => {
                app.settings = dialog.settings;
                app.settings_dialog = None;
                ui.request_repaint();
            }
            Err(error) => dialog.message = error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{FrameInput, Scene};

    #[test]
    fn editor_settings_preview_blocks_editing_and_escape_restores_saved_scale() {
        let mut app = EditorApp::default();
        app.settings = EditorSettings { ui_scale: 1.25 };
        app.open_editor_settings();
        app.settings_dialog.as_mut().unwrap().settings.ui_scale = 1.75;
        assert_eq!(app.ui_scale(), 1.75);
        assert_eq!(app.settings.ui_scale, 1.25);
        let mut input = FrameInput::default();
        input.key_undo = true;
        input.key_delete = true;
        input.mouse_down = true;
        input.key_escape = true;
        app.begin_frame(&input);
        assert!(!app.input.key_undo && !app.input.key_delete && !app.input.mouse_down);
        let mut ui = Ui::new();
        ui.set_scale(app.ui_scale());
        ui.begin_frame(mega_ui::UiInput {
            viewport: Vec2::new(900.0, 700.0),
            ..Default::default()
        });
        draw(&mut ui, &mut app);
        ui.end_frame();
        assert!(app.settings_dialog.is_none());
        assert_eq!(app.ui_scale(), 1.25);
        assert!(!app.dirty);
    }
}
