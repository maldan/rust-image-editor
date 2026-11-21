use glam::Vec2;
use mega_ui::{Ui, Window};

use crate::app::EditorApp;

pub(super) fn draw(ui: &mut Ui, app: &mut EditorApp) {
    if !app.new_document_open {
        return;
    }
    let mut open = true;
    let mut choice = None;
    let mut cancel = false;
    let choices: [(&str, &str, fn(&mut EditorApp)); 5] = [
        ("image", "Image", EditorApp::new_document),
        ("animation", "Animation", EditorApp::new_animation),
        ("video", "Video", EditorApp::new_video_document),
        ("scene", "3D Scene", EditorApp::new_scene_document),
        ("graph", "Graph", EditorApp::new_graph),
    ];
    ui.modal(
        Window::new("new_document", "New Document")
            .size(Vec2::new(300.0, 285.0))
            .open(&mut open),
        |ui| {
            for (id, label, create) in choices {
                if ui.button(id, label).clicked() {
                    choice = Some(create);
                }
            }
            ui.label("3D scenes are kept for this session.");
            ui.separator();
            cancel = ui.button("cancel", "Cancel").clicked();
        },
    );
    if let Some(create) = choice {
        create(app);
    }
    if !open || cancel || choice.is_some() || app.input.key_escape {
        app.new_document_open = false;
        ui.request_repaint();
    }
}
