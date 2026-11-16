use crate::app::EditorApp;
use crate::nodes::{self, Destination, NodeKind, Source};
use glam::Vec2;
use mega_ui::{NodePortSide, Ui};

const PREVIEW_DISPLAY_SIZE: f32 = 192.0;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    app.refresh_node_previews();
    let mut editor = std::mem::take(&mut app.nodes);
    let current_document = app.workspace.active_doc().id.clone();
    ui.horizontal(|ui| {
        if ui.button("fit_nodes", "Fit view").clicked() {
            editor.space.fit_view = true;
        }
        if ui.button("delete_nodes", "Delete selected").clicked() {
            editor
                .space
                .request_delete_nodes
                .extend(editor.space.selected_nodes.clone());
        }
    });
    ui.label(&editor.message);
    let previous_links = editor.space.links.clone();
    let mut run = None;
    let busy = editor.run.is_some();
    let previews = &mut editor.previews;
    let size = ui.available_size().max(Vec2::splat(1.0));
    ui.node_space("image_nodes", size, &mut editor.space, |ui| {
        for node in &mut editor.nodes {
            let title = match &node.kind {
                NodeKind::Image(_) => "Image",
                NodeKind::Resize { .. } => "Resize",
                NodeKind::Lineart { .. } => "Lineart",
                NodeKind::Output(_) => "Output",
            };
            let id = node.id.clone();
            ui.node(&id, title, &mut node.position, |ui| match &mut node.kind {
                NodeKind::Image(source) => {
                    let before_doc = source.document.clone();
                    document_select(
                        ui,
                        "document",
                        &mut source.document,
                        &app.workspace.documents,
                    );
                    if source.document != before_doc {
                        source.layer = None;
                        previews.remove(&id);
                    }
                    if let Some(doc) = app
                        .workspace
                        .documents
                        .iter()
                        .find(|d| d.id == source.document)
                    {
                        let mut names = vec!["Whole document".to_string()];
                        names.extend(doc.project.layers.iter().map(|l| l.name.clone()));
                        let missing = source.layer.as_ref().is_some_and(|identity| {
                            !doc.project
                                .layers
                                .iter()
                                .any(|l| std::sync::Arc::ptr_eq(identity, &l.identity))
                        });
                        if missing {
                            names.push("Missing layer — select again".into());
                        }
                        let mut selected = source.layer.as_ref().map_or(0, |identity| {
                            doc.project
                                .layers
                                .iter()
                                .position(|l| std::sync::Arc::ptr_eq(identity, &l.identity))
                                .map_or(names.len() - 1, |index| index + 1)
                        });
                        let options: Vec<_> = names.iter().map(String::as_str).collect();
                        if ui.select("source_layer", &mut selected, &options).changed {
                            source.layer = if selected == 0 {
                                None
                            } else {
                                doc.project
                                    .layers
                                    .get(selected - 1)
                                    .map(|l| l.identity.clone())
                            };
                            previews.remove(&id);
                        }
                    }
                    if let Some(preview) = previews.get(&id) {
                        if preview.ready {
                            let area = ui.area(
                                "source_preview",
                                Vec2::splat(PREVIEW_DISPLAY_SIZE * ui.scale()),
                            );
                            let x = (node.cell % 8) as f32 / 8.0;
                            let y = (node.cell / 8) as f32 / 8.0;
                            ui.texture_rect(
                                nodes::PREVIEW_TEXTURE,
                                area.rect,
                                [x, y],
                                [x + 0.125, y + 0.125],
                            );
                            ui.label(&format!("{} × {} px", preview.size.0, preview.size.1));
                        } else if !preview.error.is_empty() {
                            ui.label(&preview.error);
                        } else {
                            ui.label("Updating preview…");
                        }
                    } else {
                        ui.label("Loading preview…");
                        ui.request_repaint();
                    }
                    ui.node_port(NodePortSide::Output, "image", nodes::IMAGE_PORT);
                }
                NodeKind::Resize { scale } => {
                    ui.node_port(NodePortSide::Input, "image", nodes::IMAGE_PORT);
                    ui.label("Scale");
                    ui.drag_float("scale", scale, 0.01);
                    if !scale.is_finite() {
                        *scale = 1.0;
                    }
                    *scale = scale.max(0.001);
                    ui.node_port(NodePortSide::Output, "image", nodes::IMAGE_PORT);
                }
                NodeKind::Lineart { resolution } => {
                    ui.node_port(NodePortSide::Input, "image", nodes::IMAGE_PORT);
                    ui.label("Resolution (64–2048, step 64)");
                    ui.drag_int("resolution", resolution, 64);
                    if let Err(error) = crate::comfyui::validate_lineart_resolution(*resolution) {
                        ui.label(&error);
                    }
                    ui.node_port(NodePortSide::Output, "image", nodes::IMAGE_PORT);
                }
                NodeKind::Output(destination) => {
                    ui.node_port(NodePortSide::Input, "image", nodes::IMAGE_PORT);
                    let mut mode = match destination {
                        Destination::NewDocument => 0,
                        Destination::Layer(_) => 1,
                        Destination::Memory => 2,
                    };
                    if ui
                        .select(
                            "destination_mode",
                            &mut mode,
                            &["New document", "Layer in document", "Memory"],
                        )
                        .changed
                    {
                        *destination = if mode == 0 {
                            Destination::NewDocument
                        } else if mode == 1 {
                            Destination::Layer(current_document.clone())
                        } else {
                            Destination::Memory
                        };
                    }
                    if let Destination::Layer(document) = destination {
                        document_select(
                            ui,
                            "destination_document",
                            document,
                            &app.workspace.documents,
                        );
                    }
                    if let Some(preview) = previews.get(&id).filter(|p| p.ready) {
                        let area = ui.area(
                            "output_preview",
                            Vec2::splat(PREVIEW_DISPLAY_SIZE * ui.scale()),
                        );
                        let x = (node.cell % 8) as f32 / 8.0;
                        let y = (node.cell / 8) as f32 / 8.0;
                        ui.texture_rect(
                            nodes::PREVIEW_TEXTURE,
                            area.rect,
                            [x, y],
                            [x + 0.125, y + 0.125],
                        );
                        ui.label(&format!("{} × {} px", preview.size.0, preview.size.1));
                    }
                    ui.add_enabled(!busy, |ui| {
                        if ui.button("run", "Run").clicked() {
                            run = Some(id.clone());
                        }
                    });
                }
            });
        }
    });
    let mut add = None;
    ui.context_menu("add_node_menu", editor.space.context_menu_request, |ui| {
        if ui.menu_item("image", "Image").clicked() {
            add = Some(NodeKind::Image(Source {
                document: current_document.clone(),
                layer: None,
            }));
        }
        if ui.menu_item("resize", "Resize").clicked() {
            add = Some(NodeKind::Resize { scale: 1.0 });
        }
        if ui.menu_item("lineart", "Lineart").clicked() {
            add = Some(NodeKind::Lineart { resolution: 512 });
        }
        if ui.menu_item("output", "Output").clicked() {
            add = Some(NodeKind::Output(Destination::NewDocument));
        }
    });
    if let Some(kind) = add {
        editor.add(kind);
        ui.request_repaint();
    }
    editor.remove_requested();
    if let Err(error) = nodes::validate(&editor.nodes, &editor.space.links) {
        editor.space.links = previous_links;
        // Deleting nodes must never restore their old links.
        editor.space.links.retain(|l| {
            editor.nodes.iter().any(|n| n.id == l.from_node)
                && editor.nodes.iter().any(|n| n.id == l.to_node)
        });
        editor.message = error;
        ui.request_repaint();
    }
    app.nodes = editor;
    if let Some(id) = run {
        app.run_node_output(&id);
    }
}

fn document_select(
    ui: &mut Ui,
    id: &str,
    document: &mut String,
    documents: &[crate::workspace::Document],
) {
    let mut names = vec!["Select document…".to_string()];
    names.extend(documents.iter().map(|d| d.name.clone()));
    let mut selected = documents
        .iter()
        .position(|d| d.id == *document)
        .map_or(0, |i| i + 1);
    let options: Vec<_> = names.iter().map(String::as_str).collect();
    if ui.select(id, &mut selected, &options).changed {
        *document = selected
            .checked_sub(1)
            .and_then(|i| documents.get(i))
            .map_or_else(String::new, |d| d.id.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ui: &mut Ui, app: &mut EditorApp, mut input: mega_ui::UiInput) -> mega_ui::UiOutput {
        input.viewport = Vec2::new(800.0, 700.0);
        ui.begin_frame(input);
        draw(ui, app);
        ui.end_frame()
    }

    #[test]
    fn nodes_context_menu_adds_each_kind_at_pointer_without_changing_view() {
        for item in 0..4 {
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&[255; 16], 2, 2).unwrap();
            let mut ui = Ui::new();
            let pan = Vec2::new(-120.0, 60.0);
            let zoom = 0.65;
            app.nodes.space.pan = pan;
            app.nodes.space.zoom = zoom;
            let pointer = Vec2::new(440.0, 300.0);
            frame(
                &mut ui,
                &mut app,
                mega_ui::UiInput {
                    mouse_pos: pointer,
                    mouse_right_pressed: true,
                    mouse_right_down: true,
                    ..Default::default()
                },
            );
            let output = frame(
                &mut ui,
                &mut app,
                mega_ui::UiInput {
                    mouse_pos: pointer,
                    mouse_right_released: true,
                    ..Default::default()
                },
            );
            assert!(ui.context_menu_open());
            let popup = output
                .draw_list
                .iter()
                .find(|c| c.colors == [ui.theme().popup.bg; 4])
                .unwrap()
                .rect;
            let position = Vec2::new(
                popup.min.x + 30.0,
                popup.min.y - 1.0 + 4.0 + ui.theme().metrics.menu_item_h * (item as f32 + 0.5),
            );
            frame(
                &mut ui,
                &mut app,
                mega_ui::UiInput {
                    mouse_pos: position,
                    mouse_pressed: true,
                    mouse_down: true,
                    ..Default::default()
                },
            );
            frame(
                &mut ui,
                &mut app,
                mega_ui::UiInput {
                    mouse_pos: position,
                    mouse_released: true,
                    ..Default::default()
                },
            );
            assert!(!ui.context_menu_open());
            assert_eq!(app.nodes.nodes.len(), 1);
            let node = &app.nodes.nodes[0];
            assert!((node.position - (pointer - pan) / zoom).length() < 0.001);
            assert!(matches!(
                (&node.kind, item),
                (NodeKind::Image(_), 0)
                    | (NodeKind::Resize { scale: 1.0 }, 1)
                    | (NodeKind::Lineart { resolution: 512 }, 2)
                    | (NodeKind::Output(_), 3)
            ));
            for _ in 0..4 {
                frame(&mut ui, &mut app, Default::default());
                assert_eq!(app.nodes.space.pan, pan);
                assert_eq!(app.nodes.space.zoom, zoom);
                assert!(!app.nodes.space.fit_view);
            }
        }
    }

    #[test]
    fn nodes_delete_clears_view_state() {
        let mut app = EditorApp::default();
        for _ in 0..3 {
            app.nodes.add(NodeKind::Resize { scale: 1.0 });
        }
        app.nodes.space.request_delete_nodes.push("node_2".into());
        app.nodes.remove_requested();
        assert_eq!(app.nodes.nodes.len(), 2);
        assert!(!app.nodes.nodes.iter().any(|n| n.id == "node_2"));
    }
}
