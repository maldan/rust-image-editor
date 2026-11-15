use super::ai_dialog::choose;
use crate::app::EditorApp;
use glam::Vec2;
use mega_ui::{ScrollAxes, Ui, Window};

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let mut open = true;
    let mut run = false;
    let mut cancel = false;
    let running = app.ai.job.is_some();
    let i2v = app.ai.mode == super::ai_dialog::AiMode::MinimaxI2V;
    let r2v = app.ai.mode == super::ai_dialog::AiMode::MinimaxR2V;
    let window_id = if i2v {
        "minimax_i2v_dialog"
    } else if r2v {
        "minimax_r2v_dialog"
    } else {
        "ltx_i2v_dialog"
    };
    ui.modal(Window::new(window_id, app.ai.mode.title()).size(Vec2::new(680.0, 760.0)).resizable(true).open(&mut open), |ui| {
        let height = (ui.available_size().y - 120.0 * ui.scale()).max(160.0 * ui.scale());
        ui.scroll_area("video_settings", Vec2::new(ui.available_size().x, height), ScrollAxes::Vertical, |ui| {
            ui.add_enabled(!running, |ui| {
                if i2v {
                    draw_refs(ui, app, true);
                } else if r2v {
                    draw_refs(ui, app, false);
                    draw_r2v_video(ui, app);
                } else {
                    ui.label("Source document");
                    let names: Vec<_> = app.workspace.documents.iter().map(|d| d.name.as_str()).collect();
                    let mut doc_index = app.workspace.documents.iter().position(|d| d.id == app.ai.document_id).unwrap_or(0);
                    if ui.select("video_document", &mut doc_index, &names).changed() {
                        app.ai.document_id = app.workspace.documents[doc_index].id.clone();
                        app.ai.video_layer = None;
                    }
                    if let Some(doc) = app.workspace.documents.get(doc_index) {
                        let mut sources = vec!["Whole document".to_string()];
                        sources.extend(doc.project.layers.iter().map(|l| format!("Layer: {}", l.name)));
                        let options: Vec<_> = sources.iter().map(String::as_str).collect();
                        let mut source = app.ai.video_layer.map_or(0, |i| i + 1).min(options.len() - 1);
                        if ui.select("video_source", &mut source, &options).changed() {
                            app.ai.video_layer = source.checked_sub(1);
                        }
                    }
                }
                if i2v || r2v {
                    ui.label(&format!("Megapixels: {:.1}", app.ai.minimax_i2v_megapixels));
                    ui.slider_stepped(
                        "minimax_i2v_megapixels",
                        &mut app.ai.minimax_i2v_megapixels,
                        0.2..=1.0,
                        0.1,
                    );
                    ui.horizontal(|ui| {
                        ui.label("Duration (seconds)");
                        ui.drag_int("video_seconds", &mut app.ai.video_settings.seconds, 1);
                    });
                    if r2v {
                        ui.checkbox(
                            "minimax_r2v_lightning",
                            "Lightning",
                            &mut app.ai.minimax_r2v_lightning,
                        );
                    } else {
                        ui.checkbox("minimax_i2v_turbo", "Turbo", &mut app.ai.minimax_i2v_turbo);
                    }
                    ui.label("Prompt");
                    ui.text_area("video_prompt", &mut app.ai.video_settings.prompt, Vec2::new(0.0, 130.0));
                    if let Err(error) = crate::comfyui::validate_minimax_i2v(
                        app.ai.minimax_i2v_megapixels,
                        app.ai.video_settings.seconds,
                        &app.ai.video_settings.prompt,
                    ) {
                        ui.label(&error);
                    }
                    return;
                }
                ui.label("End frame document (optional)");
                let mut end_names = vec!["None".to_string()];
                end_names.extend(app.workspace.documents.iter().map(|d| d.name.clone()));
                let end_options: Vec<_> = end_names.iter().map(String::as_str).collect();
                let mut end_doc = app.ai.video_end_document.as_ref()
                    .and_then(|id| app.workspace.documents.iter().position(|d| &d.id == id))
                    .map_or(0, |i| i + 1);
                if ui.select("video_end_document", &mut end_doc, &end_options).changed() {
                    app.ai.video_end_document = end_doc.checked_sub(1)
                        .map(|i| app.workspace.documents[i].id.clone());
                    app.ai.video_end_layer = None;
                }
                if let Some(doc) = end_doc.checked_sub(1).and_then(|i| app.workspace.documents.get(i)) {
                    ui.label(&format!("End canvas: {} x {} px", doc.project.width, doc.project.height));
                    let mut sources = vec!["Whole document".to_string()];
                    sources.extend(doc.project.layers.iter().map(|l| format!("Layer: {}", l.name)));
                    let options: Vec<_> = sources.iter().map(String::as_str).collect();
                    let mut source = app.ai.video_end_layer.map_or(0, |i| i + 1).min(options.len() - 1);
                    if ui.select("video_end_source", &mut source, &options).changed() {
                        app.ai.video_end_layer = source.checked_sub(1);
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("Scale");
                    ui.drag_float("video_scale", &mut app.ai.video_scale, 0.01);
                    match app.video_settings() {
                        Ok(settings) => ui.label(&format!("{} × {} px", settings.width, settings.height)),
                        Err(_) => ui.label("—"),
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("FPS");
                    ui.drag_int("video_fps", &mut app.ai.video_settings.fps, 1);
                    ui.label("Duration (seconds)");
                    ui.drag_int("video_seconds", &mut app.ai.video_settings.seconds, 1);
                });
                ui.label("Prompt");
                ui.text_area("video_prompt", &mut app.ai.video_settings.prompt, Vec2::new(0.0, 130.0));
                for i in 0..app.ai.video_settings.loras.len() {
                    if i > 0 && app.ai.video_settings.loras[i - 1].name.is_none() {
                        for lora in &mut app.ai.video_settings.loras[i..] { lora.name = None; }
                        break;
                    }
                    let lora = &mut app.ai.video_settings.loras[i];
                    ui.label(&format!("LoRA {}", i + 1));
                    choose(ui, &format!("video_lora_{i}"), &mut lora.name, &app.ai.catalog.loras, "None");
                    ui.add_enabled(lora.name.is_some(), |ui| {
                        ui.slider(&format!("video_strength_{i}"), &mut lora.strength, 0.0..=1.0);
                    });
                }
                ui.add_enabled(app.ai.config.is_some() && app.ai.catalog_load.is_none(), |ui| {
                    if ui.button("video_refresh", "Refresh LoRAs").clicked() {
                        app.ai.catalog_load = Some(crate::comfyui::CatalogLoad::start(app.ai.config.clone().unwrap(), true));
                    }
                });
                ui.label(&app.ai.catalog_message);
                if let Err(error) = app.video_settings().and_then(|settings| settings.validate()) { ui.label(&error); }
            });
        });
        if running { cancel = ui.button("video_cancel", "Cancel").clicked(); ui.request_repaint(); }
        else {
            let source_valid = if i2v {
                (0..2).all(|i| frame_ref_valid(app, &app.ai.i2v_refs, i))
            } else if r2v {
                (0..4).all(|i| frame_ref_valid(app, &app.ai.r2v_refs, i)) && r2v_video_valid(app)
            } else {
                app.workspace.documents.iter().find(|d| d.id == app.ai.document_id)
                    .is_some_and(|d| d.project.width > 0 && d.project.height > 0
                        && app.ai.video_layer.is_none_or(|i| i < d.project.layers.len()))
            };
            let end_valid = i2v || r2v || app.ai.video_end_document.as_ref().is_none_or(|id| {
                app.workspace.documents.iter().find(|d| &d.id == id)
                    .is_some_and(|d| d.project.width > 0 && d.project.height > 0
                        && app.ai.video_end_layer.is_none_or(|i| i < d.project.layers.len()))
            });
            let settings_ok = if i2v || r2v {
                crate::comfyui::validate_minimax_i2v(
                    app.ai.minimax_i2v_megapixels,
                    app.ai.video_settings.seconds,
                    &app.ai.video_settings.prompt,
                )
                .is_ok()
            } else {
                app.video_settings().and_then(|settings| settings.validate()).is_ok()
            };
            ui.add_enabled(source_valid && end_valid && app.ai.config.is_some() && settings_ok, |ui| {
                run = ui.button("video_run", "Generate MP4").clicked();
            });
        }
        if let Some(fraction) = app.ai.fraction { ui.progress_bar(fraction); }
        ui.label(&app.ai.message);
    });
    if run {
        app.start_ai();
    }
    if cancel {
        app.cancel_ai();
    }
    if !open || app.input.key_escape {
        app.close_ai();
    }
}

fn frame_ref_valid(app: &EditorApp, refs: &[super::ai_dialog::VideoRef], index: usize) -> bool {
    let reference = &refs[index];
    if index > 0 && !reference.enabled {
        return true;
    }
    app.workspace
        .documents
        .iter()
        .find(|d| d.id == reference.document_id)
        .is_some_and(|d| {
            d.project.width > 0
                && d.project.height > 0
                && reference.layer.is_none_or(|i| i < d.project.layers.len())
        })
}

fn r2v_video_valid(app: &EditorApp) -> bool {
    !app.ai.r2v_video
        || app
            .workspace
            .videos
            .iter()
            .any(|v| v.id == app.ai.r2v_video_id)
}

fn draw_r2v_video(ui: &mut Ui, app: &mut EditorApp) {
    ui.horizontal(|ui| {
        ui.checkbox("r2v_video", "Video", &mut app.ai.r2v_video);
        if app.workspace.videos.is_empty() {
            ui.label("No video assets");
            return;
        }
        if app.ai.r2v_video
            && app
                .workspace
                .videos
                .iter()
                .all(|v| v.id != app.ai.r2v_video_id)
        {
            app.ai.r2v_video_id = app.workspace.videos[0].id.clone();
        }
        let names: Vec<String> = app.workspace.videos.iter().map(|v| v.name.clone()).collect();
        let options: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut index = app
            .workspace
            .videos
            .iter()
            .position(|v| v.id == app.ai.r2v_video_id)
            .unwrap_or(0);
        let enabled = app.ai.r2v_video;
        ui.add_enabled(enabled, |ui| {
            if ui.select("r2v_video_asset", &mut index, &options).changed() {
                app.ai.r2v_video_id = app.workspace.videos[index].id.clone();
            }
        });
    });
}

fn draw_refs(ui: &mut Ui, app: &mut EditorApp, i2v: bool) {
    let (prefix, labels): (&str, &[&str]) = if i2v {
        ("i2v", &["First", "Last"])
    } else {
        (
            "r2v",
            &["Reference 1", "Reference 2", "Reference 3", "Reference 4"],
        )
    };
    let refs = if i2v {
        &mut app.ai.i2v_refs[..]
    } else {
        &mut app.ai.r2v_refs[..]
    };
    refs[0].enabled = true;
    for i in 0..refs.len() {
        ui.horizontal(|ui| {
            if i == 0 {
                let mut on = true;
                ui.add_enabled(false, |ui| {
                    ui.checkbox(&format!("{prefix}_ref_0"), labels[0], &mut on);
                });
            } else {
                ui.checkbox(
                    &format!("{prefix}_ref_{i}"),
                    labels[i],
                    &mut refs[i].enabled,
                );
            }
            let names: Vec<_> = app.workspace.documents.iter().map(|d| d.name.as_str()).collect();
            let mut doc_index = app
                .workspace
                .documents
                .iter()
                .position(|d| d.id == refs[i].document_id)
                .unwrap_or(0);
            if ui
                .select(&format!("{prefix}_document_{i}"), &mut doc_index, &names)
                .changed()
            {
                refs[i].document_id = app.workspace.documents[doc_index].id.clone();
                refs[i].layer = None;
            }
            if let Some(doc) = app.workspace.documents.get(doc_index) {
                let mut sources = vec!["Whole document".to_string()];
                sources.extend(doc.project.layers.iter().map(|l| format!("Layer: {}", l.name)));
                let options: Vec<_> = sources.iter().map(String::as_str).collect();
                let mut source = refs[i]
                    .layer
                    .map_or(0, |layer| layer + 1)
                    .min(options.len() - 1);
                if ui
                    .select(&format!("{prefix}_layer_{i}"), &mut source, &options)
                    .changed()
                {
                    refs[i].layer = source.checked_sub(1);
                }
            }
        });
    }
}
