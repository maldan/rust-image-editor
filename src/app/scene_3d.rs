use super::*;
use crate::project::{Layer, LayerKind};
use crate::raster::SparsePixels;
use crate::scene_3d::SceneDocument;
use std::collections::HashMap;

impl EditorApp {
    pub(crate) fn place_scene_layer(&mut self, index: usize) {
        if self.scene_editor.active.is_some()
            || self.video_editor.active.is_some()
            || self.session.preview_graph
            || self.session.preview_asset.is_some()
            || self.layer_grid_active()
            || self.project().animation.is_some()
        {
            return;
        }
        let Some(scene) = self.scene_editor.documents.get(index) else {
            return;
        };
        let layer = Layer {
            id: crate::project::new_layer_id(),
            identity: Default::default(),
            kind: LayerKind::Scene {
                document_id: scene.id,
                rendered: None,
            },
            ai_metadata: None,
            z: 0.0,
            drawings: Default::default(),
            name: scene.name.clone(),
            visible: true,
            opacity: 1.0,
            pixels: SparsePixels::default(),
        };
        self.commit_transform();
        let dirty = self.dirty;
        let had_image = self.image_size.is_some();
        let fallback = (self.resize_w.max(1), self.resize_h.max(1));
        self.history
            .insert_layer(self.workspace.active_mut(), layer, fallback);
        self.sync_layer_structure();
        self.dirty = dirty;
        self.set_tool(self.session.select_tool);
        if !had_image {
            self.session.fit_image = true;
        }
        self.status = "3D scene layer added for this session".into();
    }

    pub(super) fn refresh_scene_layers(
        &mut self,
        render: &mut dyn FnMut(&SceneDocument, (u32, u32)) -> Result<SparsePixels, String>,
    ) {
        if self.scene_editor.active.is_some() {
            return;
        }
        let mut frames = HashMap::new();
        let mut refresh_thumbs = false;
        for (index, doc) in self.workspace.documents.iter_mut().enumerate() {
            let size = (doc.project.width, doc.project.height);
            let mut changed = false;
            for layer in &mut doc.project.layers {
                let LayerKind::Scene {
                    document_id,
                    rendered,
                } = &mut layer.kind
                else {
                    continue;
                };
                let source = self
                    .scene_editor
                    .documents
                    .iter()
                    .find(|scene| scene.id == *document_id);
                let Some(scene) = source else {
                    if rendered.take().is_some() {
                        layer.pixels = SparsePixels::default();
                        changed = true;
                    }
                    continue;
                };
                let stamp = scene.layer_render_stamp(size);
                if *rendered == Some(stamp) {
                    continue;
                }
                match frames
                    .entry((scene.id, stamp))
                    .or_insert_with(|| render(scene, size))
                {
                    Ok(pixels) => {
                        layer.pixels = pixels.clone();
                        *rendered = Some(stamp);
                        changed = true;
                    }
                    Err(error) => self.status = error.clone(),
                }
            }
            if changed {
                doc.rebuild_thumb();
                refresh_thumbs = true;
                if index == self.workspace.active_document {
                    self.layers_reload = true;
                }
            }
        }
        if refresh_thumbs {
            self.workspace.invalidate_thumb_atlas();
        }
    }

    pub(crate) fn new_scene_document(&mut self) {
        let index = self.scene_editor.create();
        self.show_scene_document(index);
    }

    pub(crate) fn show_scene_document(&mut self, index: usize) {
        if index >= self.scene_editor.documents.len() || self.scene_editor.active == Some(index) {
            return;
        }
        self.finish_deformation(false);
        self.commit_transform();
        self.animation_clock = None;
        self.close_asset_preview();
        self.close_graph_view();
        self.video_editor.close_view();
        self.close_scene_document();
        self.scene_editor.active = Some(index);
        self.workspace.tree_selected = Some(format!("scene_document_{index}"));
        self.status = "3D scene · This session only".into();
    }

    pub(crate) fn close_scene_document(&mut self) {
        if self.scene_editor.close_view() {
            self.workspace.select_active_document();
            self.status = format!("Document: {}", self.workspace.active_doc().name);
        }
    }

    pub(crate) fn delete_scene_document(&mut self, index: usize) {
        if index >= self.scene_editor.documents.len() {
            return;
        }
        if self.scene_editor.active == Some(index) {
            self.close_scene_document();
        }
        self.scene_editor.documents.remove(index);
        if let Some(active) = &mut self.scene_editor.active
            && *active > index
        {
            *active -= 1;
            self.workspace.tree_selected = Some(format!("scene_document_{active}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Raster;
    use crate::scene_3d::Primitive;

    #[test]
    fn object_commands_target_the_context_row_and_rename_is_undoable() {
        let mut app = EditorApp::default();
        app.new_scene_document();
        let scene = app.scene_editor.current_mut().unwrap();
        scene.add(Primitive::Cube);
        scene.add(Primitive::Sphere);
        scene.clone_object(0);
        assert_eq!(scene.objects.items[2].name, "Cube 1 copy");
        let target = RenameTarget::SceneObject {
            scene_id: scene.id,
            object_id: scene.objects.items[0].id,
        };
        app.open_rename(target);
        app.rename.as_mut().unwrap().name = "Backdrop".into();
        app.apply_rename();
        assert!(!app.dirty);
        let scene = app.scene_editor.current_mut().unwrap();
        assert_eq!(scene.objects.items[0].name, "Backdrop");
        assert_eq!(scene.objects.selected, Some(2));
        scene.undo();
        assert_eq!(scene.objects.items[0].name, "Cube 1");
        scene.delete_object(1);
        assert_eq!(scene.objects.items.len(), 2);
        assert_eq!(scene.objects.selected, Some(1));
        assert_eq!(scene.objects.items[1].name, "Cube 1 copy");
    }

    #[test]
    fn linked_scene_layer_refreshes_feeds_ai_and_is_omitted_from_save() {
        let mut app = EditorApp::default();
        app.project_mut()
            .import_rgba(&[0, 0, 0, 0].repeat(12), 4, 3)
            .unwrap();
        app.reload_active_view().unwrap();
        app.new_scene_document();
        app.new_scene_document();
        app.scene_editor.current_mut().unwrap().add(Primitive::Cube);
        app.close_scene_document();
        app.place_scene_layer(1);
        app.delete_scene_document(0);
        assert!(app.active_scene_layer());
        assert!(app.session.tool.is_select());

        let mut renders = 0;
        let mut render = |_: &SceneDocument, size| {
            assert_eq!(size, (4, 3));
            renders += 1;
            Ok(SparsePixels::from_rgba(
                &[renders, 2, 3, 255].repeat(12),
                4,
                3,
            ))
        };
        app.refresh_scene_layers(&mut render);
        app.refresh_scene_layers(&mut render);
        assert_eq!(
            app.project().active_pixels().unwrap().read(0, 0, 0),
            [1, 2, 3, 255]
        );
        app.invert_layer();
        assert!(app.project_mut().active_pixels_mut().is_none());

        app.show_scene_document(0);
        app.scene_editor.current_mut().unwrap().camera.zoom(40.0);
        app.close_scene_document();
        app.refresh_scene_layers(&mut render);
        assert_eq!(
            app.project().active_pixels().unwrap().read(0, 0, 0),
            [2, 2, 3, 255]
        );
        let (input, region) = app.ai_input().unwrap();
        assert_eq!(input.get_pixel(0, 0).0, [2, 2, 3, 255]);
        app.ai.document_id = app.workspace.active_doc().id.clone();
        app.ai.new_layer = false;
        app.ai.input_region = Some(region);
        let mut result = image::RgbaImage::new(4, 3);
        result.put_pixel(3, 2, image::Rgba([70, 20, 10, 255]));
        app.ai.result = Some(result);
        app.accept_ai();
        assert_eq!(app.project().layers.len(), 3);
        assert!(app.project().layers[2].is_raster());
        assert_eq!(app.project().layers[1].pixels.read(0, 0, 0), [2, 2, 3, 255]);

        app.project_mut().active_layer = 1;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scene-layer.mgimg");
        app.workspace.save(&path).unwrap();
        assert!(app.active_scene_layer());
        app.open_path(&path).unwrap();
        assert!(app.scene_editor.documents.is_empty());
        assert_eq!(app.project().layers.len(), 2);
        assert_eq!(app.project().active_layer, 0);
        assert_eq!(
            app.project().layers[1].pixels.read(0, 3, 2),
            [70, 20, 10, 255]
        );
        let (pixels, width, height) = app.project().decoded_rgba().unwrap().unwrap();
        assert_eq!(
            app.workspace.active_doc().thumb,
            crate::workspace::make_thumb(&pixels, width, height)
        );
    }

    #[test]
    fn editable_scene_is_rendered_and_survives_switching_until_project_reopens() {
        let mut app = EditorApp::default();
        let mut ui = Ui::new();
        app.new_scene_document();
        app.scene_editor.current_mut().unwrap().add(Primitive::Cube);
        assert!(!app.dirty);
        assert_eq!(
            app.workspace.tree_selected.as_deref(),
            Some("scene_document_0")
        );

        let viewport = Vec2::new(1200.0, 800.0);
        app.scene_dock = DockState::new(mega_ui::DockNode::split_h(
            0.25,
            mega_ui::DockNode::leaf(&["Library"]),
            mega_ui::DockNode::leaf(&["Workspace"]),
        ));
        ui.begin_frame(mega_ui::UiInput {
            viewport,
            ..Default::default()
        });
        app.build(&mut ui, viewport, DrawStats::default());
        let output = ui.end_frame();
        assert!(app.scene_present().is_some());
        assert!(app.canvas_present().is_none());
        assert!(
            output
                .draw_list
                .iter()
                .any(|command| command.tex == crate::scene_3d::THUMB_TEXTURE)
        );

        app.switch_document(app.workspace.active_document);
        assert!(app.scene_editor.active.is_none());
        app.show_scene_document(0);
        assert_eq!(app.scene_editor.current().unwrap().objects.items.len(), 1);

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("project.mgimg");
        app.workspace.path = Some(path.clone());
        assert!(app.try_save_project(&mut ui));
        assert!(app.scene_editor.current().is_some());
        app.open_path(&path).unwrap();
        assert!(app.scene_editor.documents.is_empty());
        assert_eq!(app.workspace.documents.len(), 1);
    }
}
