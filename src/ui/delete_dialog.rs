use glam::Vec2;
use mega_ui::{Ui, Window};

use crate::app::EditorApp;
use crate::workspace::LibraryKind;

pub struct DeletePrompt {
    pub kind: LibraryKind,
    pub id: String,
    pub name: String,
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let Some(prompt) = app.delete_prompt.as_ref() else {
        return;
    };
    let kind = match prompt.kind {
        LibraryKind::Image => "image",
        LibraryKind::Video => "video",
        LibraryKind::Document => "document",
    };
    let name = prompt.name.clone();
    let mut open = true;
    let mut accept = false;
    let mut cancel = false;
    ui.modal(
        Window::new("delete_item", "Delete")
            .size(Vec2::new(420.0, 150.0))
            .open(&mut open),
        |ui| {
            ui.label(&format!("Delete {kind} '{name}'?"));
            ui.space(16.0);
            ui.horizontal(|ui| {
                accept = ui.button("delete_ok", "Delete").clicked();
                cancel = ui.button("delete_cancel", "Cancel").clicked();
            });
        },
    );
    if !open || cancel || app.input.key_escape {
        app.delete_prompt = None;
    } else if accept || app.input.key_enter {
        app.confirm_library_delete();
    }
}

impl EditorApp {
    pub fn prompt_library_delete(&mut self, kind: LibraryKind, index: usize) {
        if self.delete_prompt.is_some() {
            return;
        }
        let Some((id, name)) = (match kind {
            LibraryKind::Image => self
                .workspace
                .assets
                .get(index)
                .map(|a| (a.id.clone(), a.name.clone())),
            LibraryKind::Video => self
                .workspace
                .videos
                .get(index)
                .map(|v| (v.id.clone(), v.name.clone())),
            LibraryKind::Document => self
                .workspace
                .documents
                .get(index)
                .map(|d| (d.id.clone(), d.name.clone())),
        }) else {
            return;
        };
        self.delete_prompt = Some(DeletePrompt { kind, id, name });
    }

    pub fn confirm_library_delete(&mut self) {
        let Some(prompt) = self.delete_prompt.take() else {
            return;
        };
        match prompt.kind {
            LibraryKind::Image => {
                let Some(index) = self
                    .workspace
                    .assets
                    .iter()
                    .position(|a| a.id == prompt.id)
                else {
                    return;
                };
                if self.session.preview_asset.as_ref() == Some(&prompt.id) {
                    self.close_asset_preview();
                }
                self.workspace.delete_asset(index);
                self.dirty = true;
            }
            LibraryKind::Video => {
                let Some(index) = self
                    .workspace
                    .videos
                    .iter()
                    .position(|v| v.id == prompt.id)
                else {
                    return;
                };
                if self.session.preview_asset.as_ref() == Some(&prompt.id) {
                    self.close_asset_preview();
                }
                self.workspace.delete_video(index);
                self.dirty = true;
            }
            LibraryKind::Document => {
                let Some(index) = self
                    .workspace
                    .documents
                    .iter()
                    .position(|d| d.id == prompt.id)
                else {
                    return;
                };
                self.delete_document(index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::LibraryKind;

    #[test]
    fn library_delete_waits_for_confirm_and_keeps_item_on_cancel() {
        let mut app = EditorApp::default();
        app.workspace
            .import_asset_rgba("Shot".into(), &[1, 2, 3, 255], 1, 1)
            .unwrap();
        let bytes = include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec();
        app.workspace.add_video(bytes).unwrap();
        app.workspace.create_document("Other");

        app.prompt_library_delete(LibraryKind::Video, 0);
        assert_eq!(app.workspace.videos.len(), 1);
        assert_eq!(app.delete_prompt.as_ref().unwrap().name, "Ltx 2.3 I2V 1.mp4");
        app.delete_prompt = None;
        assert_eq!(app.workspace.videos.len(), 1);

        app.prompt_library_delete(LibraryKind::Image, 0);
        app.confirm_library_delete();
        assert!(app.workspace.assets.is_empty());
        assert!(app.delete_prompt.is_none());

        app.prompt_library_delete(LibraryKind::Document, 0);
        assert_eq!(app.workspace.documents.len(), 2);
        app.confirm_library_delete();
        assert_eq!(app.workspace.documents.len(), 1);
    }
}
