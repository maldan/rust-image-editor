use crate::app::EditorApp;
use crate::nodes::{self, Destination, NodeKind, Source};
use glam::Vec2;
use mega_ui::{NodePortSide, Ui};

const PREVIEW_DISPLAY_SIZE: f32 = 192.0;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    app.refresh_node_previews();
    let mut editor = std::mem::take(&mut app.nodes);
    let previous_template = nodes::encode_graph(&editor).ok();
    let current_document = app.workspace.active_doc().id.clone();
    ui.horizontal(|ui| {
        if ui.button("fit_nodes", "Fit view").clicked() {
            editor.space.fit_view = true;
        }
        let can_stop = editor
            .run
            .as_ref()
            .is_some_and(|run| !run.cancel.load(std::sync::atomic::Ordering::Relaxed));
        ui.add_enabled(can_stop, |ui| {
            if ui.button("stop_graph", "Stop").clicked() {
                if let Some(run) = &editor.run {
                    run.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                editor.message = "Stopping…".into();
                ui.request_repaint();
            }
        });
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
    let progress = editor.run.as_ref().and_then(|run| {
        run.progress.try_lock().ok().map(|progress| {
            let mut progress = progress.clone();
            if run.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                progress.message = "Stopping…".into();
                progress.fraction = None;
            }
            progress
        })
    });
    let previews = &mut editor.previews;
    let live_display = &editor.live_display;
    let size = ui.available_size().max(Vec2::splat(1.0));
    let space_id = format!("image_nodes_{}", app.workspace.graphs.entries[app.workspace.graphs.active].id);
    let graph_rect = ui.surface(size, [0.02, 0.02, 0.02, 1.0]);
    app.session.workspace_rect = graph_rect;
    ui.node_space_rect(&space_id, graph_rect, &mut editor.space, |ui| {
        for node in &mut editor.nodes {
            let title = match &node.kind {
                NodeKind::ImageDocument(_) => "Image Document",
                NodeKind::ImageAsset { .. } => "Image Asset",
                NodeKind::Resize { .. } => "Resize",
                NodeKind::Lineart { .. } => "Lineart",
                NodeKind::MinimaxRefs { .. } => "Minimax H3 Refs",
                NodeKind::Output(_) => "Output",
            };
            let id = node.id.clone();
            ui.node(&id, title, &mut node.position, |ui| match &mut node.kind {
                NodeKind::ImageDocument(source) => {
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
                    draw_source_preview(ui, previews.get(&id), node.cell);
                    ui.node_port(NodePortSide::Output, "image", nodes::IMAGE_PORT);
                    ui.node_port(NodePortSide::Output, "size", nodes::SIZE_PORT);
                }
                NodeKind::ImageAsset { id: asset_id } => {
                    let before = asset_id.clone();
                    asset_select(ui, "asset", asset_id, &app.workspace.assets);
                    if *asset_id != before {
                        previews.remove(&id);
                    }
                    draw_source_preview(ui, previews.get(&id), node.cell);
                    ui.node_port(NodePortSide::Output, "image", nodes::IMAGE_PORT);
                    ui.node_port(NodePortSide::Output, "size", nodes::SIZE_PORT);
                }
                NodeKind::Resize { scale } => {
                    ui.node_port(NodePortSide::Input, "image", nodes::IMAGE_PORT);
                    ui.node_port(NodePortSide::Input, "size", nodes::SIZE_PORT);
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
                    draw_progress(ui, progress.as_ref(), &id);
                    if live_display
                        .as_ref()
                        .is_some_and(|preview| preview.node == id)
                    {
                        let area = ui.area(
                            "generation_preview",
                            Vec2::splat(PREVIEW_DISPLAY_SIZE * ui.scale()),
                        );
                        ui.texture_rect(
                            nodes::LIVE_PREVIEW_TEXTURE,
                            area.rect,
                            [0.0, 0.0],
                            [1.0, 1.0],
                        );
                    } else if previews.get(&id).is_some_and(|preview| preview.ready) {
                        let area = ui.area(
                            "generation_preview",
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
                    }
                    ui.node_port(NodePortSide::Output, "image", nodes::IMAGE_PORT);
                }
                NodeKind::MinimaxRefs { prompt, megapixels } => {
                    // The node sizes itself from its contents; reserve a wider
                    // content area so the prompt and controls can fill it.
                    ui.area("minimax_width", Vec2::new(320.0 * ui.scale(), ui.scale()));
                    for port in ["image_1", "image_2", "image_3", "image_4"] {
                        ui.node_port(NodePortSide::Input, port, nodes::IMAGE_PORT);
                    }
                    ui.label("Prompt");
                    ui.text_area("prompt", prompt, Vec2::new(0.0, 110.0 * ui.scale()));
                    app.value_sliders.draw(
                        ui,
                        "megapixels",
                        megapixels,
                        super::value_slider::ValueSlider::new("Megapixels", 1.0, 16.0, 1.0, " MP"),
                    );
                    draw_progress(ui, progress.as_ref(), &id);
                    if live_display
                        .as_ref()
                        .is_some_and(|preview| preview.node == id)
                    {
                        let area = ui.area(
                            "generation_preview",
                            Vec2::splat(PREVIEW_DISPLAY_SIZE * ui.scale()),
                        );
                        ui.texture_rect(
                            nodes::LIVE_PREVIEW_TEXTURE,
                            area.rect,
                            [0.0, 0.0],
                            [1.0, 1.0],
                        );
                    } else if previews.get(&id).is_some_and(|p| p.ready) {
                        let area = ui.area(
                            "generation_preview",
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
    if editor.space.context_menu_request {
        editor.add_menu = nodes::AddMenu::Root;
    }
    let mut add = None;
    ui.context_menu("add_node_menu", editor.space.context_menu_request, |ui| {
        match editor.add_menu {
            nodes::AddMenu::Root => {
                if ui.menu_item_submenu("source", "Source").clicked() {
                    editor.add_menu = nodes::AddMenu::Source;
                    ui.request_repaint();
                }
                if ui.menu_item_submenu("ai", "AI").clicked() {
                    editor.add_menu = nodes::AddMenu::Ai;
                    ui.request_repaint();
                }
                if ui.menu_item_submenu("utils", "Utils").clicked() {
                    editor.add_menu = nodes::AddMenu::Utils;
                    ui.request_repaint();
                }
                if ui.menu_item("output", "Output").clicked() {
                    add = Some(NodeKind::Output(Destination::NewDocument));
                }
            }
            nodes::AddMenu::Source => {
                if ui.menu_item_keep_open("back", "Back").clicked() {
                    editor.add_menu = nodes::AddMenu::Root;
                    ui.request_repaint();
                }
                if ui.menu_item("image_document", "Image Document").clicked() {
                    add = Some(NodeKind::ImageDocument(Source {
                        document: current_document.clone(),
                        layer: None,
                    }));
                }
                if ui.menu_item("image_asset", "Image Asset").clicked() {
                    add = Some(NodeKind::ImageAsset {
                        id: app
                            .workspace
                            .assets
                            .first()
                            .map(|asset| asset.id.clone())
                            .unwrap_or_default(),
                    });
                }
            }
            nodes::AddMenu::Ai => {
                if ui.menu_item_keep_open("back", "Back").clicked() {
                    editor.add_menu = nodes::AddMenu::Root;
                    ui.request_repaint();
                }
                if ui.menu_item("lineart", "Lineart").clicked() {
                    add = Some(NodeKind::Lineart { resolution: 512 });
                }
                if ui.menu_item("minimax_refs", "Minimax H3 Refs").clicked() {
                    add = Some(NodeKind::MinimaxRefs {
                        prompt: String::new(),
                        megapixels: 4.0,
                    });
                }
            }
            nodes::AddMenu::Utils => {
                if ui.menu_item_keep_open("back", "Back").clicked() {
                    editor.add_menu = nodes::AddMenu::Root;
                    ui.request_repaint();
                }
                if ui.menu_item("resize", "Resize").clicked() {
                    add = Some(NodeKind::Resize { scale: 1.0 });
                }
            }
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
    if nodes::encode_graph(&app.nodes).ok() != previous_template {
        app.dirty = true;
    }
    if let Some(id) = run {
        app.run_node_output(&id);
    }
}

fn draw_progress(ui: &mut Ui, progress: Option<&nodes::NodeProgress>, id: &str) {
    if let Some(progress) = progress.filter(|progress| progress.node == id) {
        if !progress.message.is_empty() {
            ui.label(&progress.message);
        }
        if let Some(fraction) = progress.fraction {
            ui.progress_bar(fraction);
        }
    }
}

fn draw_source_preview(ui: &mut Ui, preview: Option<&nodes::Preview>, cell: usize) {
    if let Some(preview) = preview {
        if preview.ready {
            let area = ui.area(
                "source_preview",
                Vec2::splat(PREVIEW_DISPLAY_SIZE * ui.scale()),
            );
            let x = (cell % 8) as f32 / 8.0;
            let y = (cell / 8) as f32 / 8.0;
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
}

fn asset_select(
    ui: &mut Ui,
    id: &str,
    asset: &mut String,
    assets: &[crate::workspace::ImageAsset],
) {
    let mut names = vec!["Select asset…".to_string()];
    names.extend(assets.iter().map(|a| a.name.clone()));
    let mut selected = assets
        .iter()
        .position(|a| a.id == *asset)
        .map_or(0, |i| i + 1);
    let options: Vec<_> = names.iter().map(String::as_str).collect();
    if ui.select(id, &mut selected, &options).changed {
        *asset = selected
            .checked_sub(1)
            .and_then(|i| assets.get(i))
            .map_or_else(String::new, |a| a.id.clone());
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

    fn click_menu(ui: &mut Ui, app: &mut EditorApp, popup: mega_ui::Rect, item: usize) -> mega_ui::UiOutput {
        let position = Vec2::new(
            popup.min.x + 30.0,
            popup.min.y - 1.0 + 4.0 + ui.theme().metrics.menu_item_h * (item as f32 + 0.5),
        );
        frame(
            ui,
            app,
            mega_ui::UiInput {
                mouse_pos: position,
                mouse_pressed: true,
                mouse_down: true,
                ..Default::default()
            },
        );
        frame(
            ui,
            app,
            mega_ui::UiInput {
                mouse_pos: position,
                mouse_released: true,
                ..Default::default()
            },
        )
    }

    fn popup_rect(ui: &Ui, output: &mega_ui::UiOutput) -> mega_ui::Rect {
        output
            .draw_list
            .iter()
            .find(|c| c.colors == [ui.theme().popup.bg; 4])
            .unwrap()
            .rect
    }

    #[test]
    fn nodes_context_menu_adds_each_kind_at_pointer_without_changing_view() {
        let cases: &[(Option<usize>, usize, fn(&NodeKind) -> bool)] = &[
            (Some(0), 1, |k| matches!(k, NodeKind::ImageDocument(_))),
            (Some(0), 2, |k| matches!(k, NodeKind::ImageAsset { .. })),
            (Some(1), 1, |k| matches!(k, NodeKind::Lineart { resolution: 512 })),
            (Some(1), 2, |k| matches!(k, NodeKind::MinimaxRefs { .. })),
            (Some(2), 1, |k| matches!(k, NodeKind::Resize { scale: 1.0 })),
            (None, 3, |k| matches!(k, NodeKind::Output(_))),
        ];
        for &(group, item, matches_kind) in cases {
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
            let mut popup = popup_rect(&ui, &output);
            if let Some(group) = group {
                click_menu(&mut ui, &mut app, popup, group);
                assert!(ui.context_menu_open());
                let output = frame(&mut ui, &mut app, Default::default());
                popup = popup_rect(&ui, &output);
            }
            click_menu(&mut ui, &mut app, popup, item);
            assert!(!ui.context_menu_open());
            assert_eq!(app.nodes.nodes.len(), 1);
            let node = &app.nodes.nodes[0];
            assert!((node.position - (pointer - pan) / zoom).length() < 0.001);
            assert!(matches_kind(&node.kind));
            for _ in 0..4 {
                frame(&mut ui, &mut app, Default::default());
                assert_eq!(app.nodes.space.pan, pan);
                assert_eq!(app.nodes.space.zoom, zoom);
                assert!(!app.nodes.space.fit_view);
            }
        }
    }

    #[test]
    fn nodes_add_menu_back_returns_to_groups() {
        let mut app = EditorApp::default();
        let mut ui = Ui::new();
        let pointer = Vec2::new(400.0, 280.0);
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
        let popup = popup_rect(&ui, &output);
        click_menu(&mut ui, &mut app, popup, 1);
        assert_eq!(app.nodes.add_menu, nodes::AddMenu::Ai);
        let output = frame(&mut ui, &mut app, Default::default());
        let popup = popup_rect(&ui, &output);
        click_menu(&mut ui, &mut app, popup, 0);
        assert_eq!(app.nodes.add_menu, nodes::AddMenu::Root);
        assert!(ui.context_menu_open());
        assert!(app.nodes.nodes.is_empty());
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
