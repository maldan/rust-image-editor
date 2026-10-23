use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RenameTarget {
    Layer { document_id: String, index: usize },
    Image(String),
    Video(String),
    Document(String),
}
impl RenameTarget {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Layer { .. } => "Rename Layer",
            Self::Image(_) => "Rename Image",
            Self::Video(_) => "Rename Video",
            Self::Document(_) => "Rename Document",
        }
    }
}

impl EditorApp {
    pub fn open_layer_rename(&mut self, index: usize) {
        if self.session.preview_asset.is_some() {
            return;
        }
        self.open_rename(RenameTarget::Layer {
            document_id: self.workspace.active_doc().id.clone(),
            index,
        });
    }
    pub(crate) fn open_rename(&mut self, target: RenameTarget) {
        if self.ai.open || self.hsv.open || self.rename.is_some() {
            return;
        }
        let name = match &target {
            RenameTarget::Layer { document_id, index } => self
                .workspace
                .documents
                .iter()
                .find(|d| &d.id == document_id)
                .and_then(|d| d.project.layers.get(*index))
                .map(|l| &l.name),
            RenameTarget::Image(id) => self
                .workspace
                .assets
                .iter()
                .find(|a| &a.id == id)
                .map(|a| &a.name),
            RenameTarget::Video(id) => self
                .workspace
                .videos
                .iter()
                .find(|v| &v.id == id)
                .map(|v| &v.name),
            RenameTarget::Document(id) => self
                .workspace
                .documents
                .iter()
                .find(|d| &d.id == id)
                .map(|d| &d.name),
        };
        let Some(name) = name.cloned() else {
            return;
        };
        self.rename = Some(crate::ui::rename_dialog::RenameDialog {
            target,
            original_name: name.clone(),
            name,
        });
        self.workspace.clear_asset_drag();
        self.layer_drag = None;
    }
    pub fn apply_rename(&mut self) {
        let Some(dialog) = self.rename.as_ref() else {
            return;
        };
        let name = dialog.name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let dialog = self.rename.take().unwrap();
        let current = match &dialog.target {
            RenameTarget::Layer { document_id, index } => self
                .workspace
                .documents
                .iter_mut()
                .find(|d| &d.id == document_id)
                .and_then(|d| d.project.layers.get_mut(*index))
                .map(|l| &mut l.name),
            RenameTarget::Image(id) => self
                .workspace
                .assets
                .iter_mut()
                .find(|a| &a.id == id)
                .map(|a| &mut a.name),
            RenameTarget::Video(id) => self
                .workspace
                .videos
                .iter_mut()
                .find(|v| &v.id == id)
                .map(|v| &mut v.name),
            RenameTarget::Document(id) => self
                .workspace
                .documents
                .iter_mut()
                .find(|d| &d.id == id)
                .map(|d| &mut d.name),
        };
        let Some(current) = current else {
            return;
        };
        if *current != dialog.original_name || *current == name {
            return;
        }
        *current = name;
        if matches!(dialog.target, RenameTarget::Layer { .. }) {
            self.history.clear_redo();
        }
        self.dirty = true;
        self.status = "Name updated".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn library_renames_preserve_ids_data_preview_and_inactive_document_and_survive_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("names.mgimg");
        let mut app = EditorApp::default();
        app.workspace
            .import_asset_rgba("Original image".into(), &[1, 2, 3, 255], 1, 1)
            .unwrap();
        app.workspace
            .add_video(include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec())
            .unwrap();
        let doc_id = app.workspace.documents[0].id.clone();
        app.workspace.create_document("Open document");
        app.workspace.save(&path).unwrap();
        app.show_asset_preview(0);
        let preview = app.session.preview_asset.clone();
        let selected = app.workspace.tree_selected.clone();
        let image_id = app.workspace.assets[0].id.clone();
        let video_id = app.workspace.videos[0].id.clone();
        let pixels = app.workspace.assets[0].pixels.clone();
        let video_bytes = app.workspace.videos[0].read().unwrap();
        for (target, name) in [
            (RenameTarget::Image(image_id.clone()), "New image"),
            (RenameTarget::Video(video_id.clone()), "New video.mp4"),
            (
                RenameTarget::Document(doc_id.clone()),
                "Inactive renamed document",
            ),
        ] {
            app.dirty = false;
            app.open_rename(target);
            app.rename.as_mut().unwrap().name = name.into();
            app.apply_rename();
            assert!(app.dirty);
            assert!(app.rename.is_none());
            assert_eq!(app.workspace.active_document, 1);
            assert_eq!(app.session.preview_asset, preview);
            assert_eq!(app.workspace.tree_selected, selected);
        }
        assert_eq!(app.workspace.assets[0].pixels, pixels);
        assert_eq!(app.workspace.videos[0].read().unwrap(), video_bytes);
        app.workspace.save(&path).unwrap();
        let loaded = Workspace::open(&path).unwrap();
        assert_eq!(
            (&loaded.assets[0].id, loaded.assets[0].name.as_str()),
            (&image_id, "New image")
        );
        assert_eq!(
            (&loaded.videos[0].id, loaded.videos[0].name.as_str()),
            (&video_id, "New video.mp4")
        );
        assert_eq!(
            (&loaded.documents[0].id, loaded.documents[0].name.as_str()),
            (&doc_id, "Inactive renamed document")
        );
        assert_eq!(loaded.videos[0].read().unwrap(), video_bytes);
    }

    #[test]
    fn rename_resolves_asset_by_id_and_does_not_edit_a_replacement() {
        let mut app = EditorApp::default();
        app.workspace
            .import_asset_rgba("First".into(), &[1, 2, 3, 255], 1, 1)
            .unwrap();
        app.workspace
            .import_asset_rgba("Second".into(), &[4, 5, 6, 255], 1, 1)
            .unwrap();
        app.open_rename(RenameTarget::Image(app.workspace.assets[0].id.clone()));
        app.rename.as_mut().unwrap().name = "Renamed first".into();
        app.workspace.assets.swap(0, 1);
        app.apply_rename();
        assert_eq!(app.workspace.assets[0].name, "Second");
        assert_eq!(app.workspace.assets[1].name, "Renamed first");
        app.open_rename(RenameTarget::Image(app.workspace.assets[0].id.clone()));
        app.rename.as_mut().unwrap().name = "Should not apply".into();
        app.workspace.assets.remove(0);
        app.dirty = false;
        app.apply_rename();
        assert_eq!(app.workspace.assets[0].name, "Renamed first");
        assert!(!app.dirty);
    }
}
