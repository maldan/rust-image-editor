use crate::{app::EditorApp, comfyui::HostConfig};
use glam::Vec2;
use mega_ui::{Ui, Window};

#[derive(Default)]
pub struct AiSettings {
    pub host: String,
    pub login: String,
    pub password: String,
    pub message: String,
    // Preserve a custom legacy WebSocket endpoint until the host changes.
    pub original: Option<HostConfig>,
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let Some(dialog) = &mut app.ai_settings else {
        return;
    };
    let mut open = true;
    let mut save = false;
    let mut cancel = false;
    ui.modal(
        Window::new("ai_settings", "AI Settings")
            .size(Vec2::new(440.0, 300.0))
            .open(&mut open),
        |ui| {
            ui.label("Server");
            ui.text_input("comfy_host", &mut dialog.host);
            ui.label("Login");
            ui.text_input("comfy_login", &mut dialog.login);
            ui.label("Password");
            ui.text_input("comfy_password", &mut dialog.password);
            if !dialog.message.is_empty() {
                ui.label(&dialog.message);
            }
            ui.space(12.0);
            ui.horizontal(|ui| {
                save = ui.button("comfy_save", "Save").clicked();
                cancel = ui.button("comfy_cancel", "Cancel").clicked();
            });
        },
    );
    if !open || cancel || app.input.key_escape {
        app.ai_settings = None;
    } else if save {
        app.save_ai_settings();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{FrameInput, Scene};

    #[test]
    fn settings_modal_blocks_editor_input_and_escape_discards_edits() {
        let mut app = EditorApp::default();
        app.ai.config = Some(
            HostConfig::from_fields("http://localhost:8188".into(), "".into(), "".into()).unwrap(),
        );
        app.ai_settings = Some(AiSettings {
            host: "http://other-host:8188".into(),
            ..Default::default()
        });
        let mut input = FrameInput::default();
        input.key_undo = true;
        input.key_delete = true;
        input.key_enter = true;
        input.key_escape = true;
        input.mouse_down = true;
        app.begin_frame(&input);
        assert!(
            !app.input.key_undo
                && !app.input.key_delete
                && !app.input.key_enter
                && !app.input.mouse_down
        );
        let mut ui = Ui::new();
        ui.begin_frame(mega_ui::UiInput {
            viewport: Vec2::new(800.0, 600.0),
            ..Default::default()
        });
        draw(&mut ui, &mut app);
        ui.end_frame();
        assert!(app.ai_settings.is_none());
        assert_eq!(
            app.ai.config.as_ref().unwrap().host,
            "http://localhost:8188"
        );
    }

    #[test]
    fn invalid_settings_stay_open_without_changing_active_config() {
        let mut app = EditorApp::default();
        app.ai_settings = Some(AiSettings {
            host: "invalid".into(),
            ..Default::default()
        });
        app.save_ai_settings();
        assert!(!app.ai_settings.as_ref().unwrap().message.is_empty());
        assert!(app.ai.config.is_none());
    }
}
