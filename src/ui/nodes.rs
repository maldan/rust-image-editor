use crate::app::EditorApp;
use crate::nodes::{self, Destination, Generation, NodeKind, Source};
use glam::Vec2;
use mega_ui::{NodePortSide, Ui};

const PREVIEW_DISPLAY_SIZE: f32 = 192.0;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    app.poll_catalog(ui);
    if app
        .nodes
        .nodes
        .iter()
        .any(|node| match &node.kind {
            NodeKind::SdxlRedraw { .. } => true,
            NodeKind::Generate(generation) => generation.uses_catalog(),
            _ => false,
        })
    {
        app.ensure_sd_catalog();
    }
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
                NodeKind::Invert => "Invert",
                NodeKind::Lineart { .. } => "Lineart",
                NodeKind::MinimaxRefs { .. } => "Minimax H3 Refs",
                NodeKind::SdxlRedraw { .. } => "SDXL Redraw",
                NodeKind::Generate(generation) => generation.label(),
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
                NodeKind::Invert => {
                    ui.node_port(NodePortSide::Input, "image", nodes::IMAGE_PORT);
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
                NodeKind::MinimaxRefs {
                    prompt,
                    megapixels,
                    seed,
                } => {
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
                    ui.label("Seed (0 = random)");
                    seed_field(ui, seed);
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
                NodeKind::SdxlRedraw {
                    denoise,
                    seed,
                    model,
                    loras,
                    positive,
                    negative,
                } => {
                    ui.area("sdxl_width", Vec2::new(320.0 * ui.scale(), ui.scale()));
                    ui.node_port(NodePortSide::Input, "image", nodes::IMAGE_PORT);
                    app.value_sliders.draw(
                        ui,
                        &format!("{id}_denoise"),
                        denoise,
                        super::value_slider::ValueSlider::percent("Denoise", 0.0),
                    );
                    ui.label("Model");
                    super::ai_dialog::choose(
                        ui,
                        &format!("{id}_model"),
                        model,
                        &app.ai.catalog.models,
                        "Default (workflow)",
                    );
                    if !app.ai.catalog_message.is_empty() {
                        ui.label(&app.ai.catalog_message);
                    }
                    for i in 0..loras.len() {
                        if i > 0 && loras[i - 1].name.is_none() {
                            for lora in &mut loras[i..] {
                                lora.name = None;
                            }
                            break;
                        }
                        ui.label(&format!("LoRA {}", i + 1));
                        super::ai_dialog::choose(
                            ui,
                            &format!("{id}_lora_{i}"),
                            &mut loras[i].name,
                            &app.ai.catalog.loras,
                            "None",
                        );
                        if loras[i].name.is_some() {
                            app.value_sliders.draw(
                                ui,
                                &format!("{id}_lora_strength_{i}"),
                                &mut loras[i].strength,
                                super::value_slider::ValueSlider::percent("Strength", 0.0),
                            );
                        }
                    }
                    ui.label("Positive prompt");
                    ui.text_area("positive", positive, Vec2::new(0.0, 80.0 * ui.scale()));
                    ui.label("Negative prompt");
                    ui.text_area("negative", negative, Vec2::new(0.0, 65.0 * ui.scale()));
                    ui.label("Seed (0 = random)");
                    seed_field(ui, seed);
                    draw_progress(ui, progress.as_ref(), &id);
                    draw_generation_preview(ui, live_display.as_ref(), previews.get(&id), &id, node.cell);
                    ui.node_port(NodePortSide::Output, "image", nodes::IMAGE_PORT);
                }
                NodeKind::Generate(generation) => {
                    ui.area("generation_width", Vec2::new(320.0 * ui.scale(), ui.scale()));
                    if generation.needs_image() {
                        ui.node_port(NodePortSide::Input, "image", nodes::IMAGE_PORT);
                    }
                    if generation.is_video() {
                        ui.node_port(NodePortSide::Input, "end_image", nodes::IMAGE_PORT);
                        ui.label("Optional end frame");
                    }
                    generation_controls(ui, app, generation);
                    draw_progress(ui, progress.as_ref(), &id);
                    draw_generation_preview(ui, live_display.as_ref(), previews.get(&id), &id, node.cell);
                    if generation.is_video() {
                        ui.add_enabled(!busy, |ui| {
                            if ui.button("run", "Run to Video Assets").clicked() {
                                run = Some(id.clone());
                            }
                        });
                    } else {
                        ui.node_port(NodePortSide::Output, "image", nodes::IMAGE_PORT);
                    }
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
                        seed: 0,
                    });
                }
                if ui.menu_item("sdxl_redraw", "SDXL Redraw").clicked() {
                    add = Some(NodeKind::sdxl_redraw());
                }
                for (key, generation) in [
                    ("ltx_video", Generation::LtxVideo { settings: Default::default(), scale: 1.0 }),
                    ("qwen_draw", Generation::QwenDraw { settings: Default::default() }),
                    ("qwen_redraw", Generation::QwenRedraw { prompt: String::new() }),
                    ("minimax_redraw", Generation::MinimaxRedraw { prompt: String::new(), megapixels: 4.0 }),
                    ("sdxl_draw", Generation::SdxlDraw { settings: Default::default() }),
                    ("control_net", Generation::ControlNet { settings: Default::default(), control: Default::default() }),
                    ("upscale", Generation::Upscale { settings: Default::default(), upscale: Default::default() }),
                ] {
                    if ui.menu_item(key, generation.label()).clicked() {
                        add = Some(NodeKind::Generate(generation));
                    }
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
                if ui.menu_item("invert", "Invert").clicked() {
                    add = Some(NodeKind::Invert);
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

fn draw_generation_preview(
    ui: &mut Ui,
    live_display: Option<&nodes::LivePreview>,
    preview: Option<&nodes::Preview>,
    id: &str,
    cell: usize,
) {
    if live_display.is_some_and(|preview| preview.node == id) {
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
    } else if preview.is_some_and(|preview| preview.ready) {
        let area = ui.area(
            "generation_preview",
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
    }
}

fn generation_controls(ui: &mut Ui, app: &mut EditorApp, generation: &mut Generation) {
    match generation {
        Generation::QwenRedraw { prompt } => edit_prompt(ui, "prompt", "Prompt", prompt),
        Generation::MinimaxRedraw { prompt, megapixels } => {
            ui.label("Megapixels");
            ui.drag_float("megapixels", megapixels, 0.1);
            edit_prompt(ui, "prompt", "Prompt", prompt);
        }
        Generation::LtxVideo { settings, scale } => {
            ui.label("Output scale");
            ui.drag_float("scale", scale, 0.05);
            ui.label("FPS");
            ui.drag_int("fps", &mut settings.fps, 1);
            ui.label("Duration (seconds)");
            ui.drag_int("seconds", &mut settings.seconds, 1);
            generation_loras(ui, &mut settings.loras, &app.ai.catalog.loras);
            edit_prompt(ui, "prompt", "Prompt", &mut settings.prompt);
        }
        Generation::QwenDraw { settings } => generation_draw(ui, app, settings, false, true),
        Generation::SdxlDraw { settings } => generation_draw(ui, app, settings, true, true),
        Generation::ControlNet { settings, control } => {
            generation_control_net(ui, control);
            generation_draw(ui, app, settings, true, false);
        }
        Generation::Upscale { settings, upscale } => {
            ui.label("Upscale factor");
            ui.drag_float("scale_by", &mut upscale.scale_by, 0.05);
            ui.label("Denoise");
            ui.slider("denoise", &mut upscale.denoise, 0.0..=1.0);
            generation_control_net(ui, &mut upscale.control);
            generation_draw(ui, app, settings, true, false);
        }
    }
}

fn generation_draw(
    ui: &mut Ui,
    app: &EditorApp,
    settings: &mut crate::comfyui::DrawSettings,
    use_catalog: bool,
    set_size: bool,
) {
    if use_catalog {
        ui.label("Model");
        super::ai_dialog::choose(ui, "model", &mut settings.model, &app.ai.catalog.models, "Default (workflow)");
        generation_loras(ui, &mut settings.loras, &app.ai.catalog.loras);
    }
    edit_prompt(ui, "positive", "Positive prompt", &mut settings.positive);
    edit_prompt(ui, "negative", "Negative prompt", &mut settings.negative);
    if set_size {
        ui.label("Width");
        ui.drag_int("width", &mut settings.width, 8);
        ui.label("Height");
        ui.drag_int("height", &mut settings.height, 8);
    }
}

fn generation_loras(ui: &mut Ui, loras: &mut [crate::comfyui::LoraSelection], names: &[String]) {
    let mut show = true;
    for (i, lora) in loras.iter_mut().enumerate() {
        if !show {
            lora.name = None;
            continue;
        }
        ui.label(&format!("LoRA {}", i + 1));
        super::ai_dialog::choose(ui, &format!("lora_{i}"), &mut lora.name, names, "None");
        show = lora.name.is_some();
        if show {
            ui.slider(&format!("strength_{i}"), &mut lora.strength, 0.0..=1.0);
        }
    }
}

fn generation_control_net(ui: &mut Ui, control: &mut crate::comfyui::ControlNetSettings) {
    ui.label("ControlNet strength");
    ui.slider("strength", &mut control.strength, 0.0..=1.0);
    ui.label("ControlNet start");
    ui.slider("start", &mut control.start_percent, 0.0..=1.0);
    ui.label("ControlNet end");
    ui.slider("end", &mut control.end_percent, 0.0..=1.0);
}

fn edit_prompt(ui: &mut Ui, id: &str, label: &str, prompt: &mut String) {
    ui.label(label);
    ui.text_area(id, prompt, Vec2::new(0.0, 80.0 * ui.scale()));
}

fn seed_field(ui: &mut Ui, seed: &mut u64) {
    let mut value = i32::try_from(*seed).unwrap_or(i32::MAX);
    ui.drag_int("seed", &mut value, 1);
    *seed = value.max(0) as u64;
}

fn draw_progress(ui: &mut Ui, progress: Option<&nodes::NodeProgress>, id: &str) {
    if let Some(progress) = progress.filter(|progress| progress.node == id) {
        if !progress.message.is_empty() {
            ui.label(&progress.message);
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
            (Some(1), 3, |k| matches!(k, NodeKind::SdxlRedraw { .. })),
            (Some(2), 1, |k| matches!(k, NodeKind::Resize { scale: 1.0 })),
            (Some(2), 2, |k| matches!(k, NodeKind::Invert)),
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
