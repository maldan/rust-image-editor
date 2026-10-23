use super::ai_dialog::choose;
use crate::app::EditorApp;
use glam::Vec2;
use mega_ui::{ScrollAxes, Ui, Window};

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let mut open = true;
    let mut run = false;
    let mut cancel = false;
    let running = app.ai.job.is_some();
    ui.modal(Window::new("ltx_i2v_dialog", "Ltx 2.3 I2V").size(Vec2::new(680.0, 760.0)).resizable(true).open(&mut open), |ui| {
        let height = (ui.available_size().y - 120.0 * ui.scale()).max(160.0 * ui.scale());
        ui.scroll_area("video_settings", Vec2::new(ui.available_size().x, height), ScrollAxes::Vertical, |ui| {
            ui.add_enabled(!running, |ui| {
                ui.label("Source document");
                let names: Vec<_> = app.workspace.documents.iter().map(|d| d.name.as_str()).collect();
                let mut doc_index = app.workspace.documents.iter().position(|d| d.id == app.ai.document_id).unwrap_or(0);
                if ui.select("video_document", &mut doc_index, &names).changed() {
                    app.ai.document_id = app.workspace.documents[doc_index].id.clone();
                    app.ai.video_layer = None;
                }
                if let Some(doc) = app.workspace.documents.get(doc_index) {
                    ui.label(&format!("Source canvas: {} x {} px", doc.project.width, doc.project.height));
                    let mut sources = vec!["Whole document".to_string()];
                    sources.extend(doc.project.layers.iter().map(|l| format!("Layer: {}", l.name)));
                    let options: Vec<_> = sources.iter().map(String::as_str).collect();
                    let mut source = app.ai.video_layer.map_or(0, |i| i + 1).min(options.len() - 1);
                    if ui.select("video_source", &mut source, &options).changed() {
                        app.ai.video_layer = source.checked_sub(1);
                    }
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
                ui.label("Layers use the full document canvas. Input is center-cropped to video size.");
                ui.horizontal(|ui| {
                    ui.label("Width");
                    ui.drag_int("video_width", &mut app.ai.video_settings.width, 64);
                    ui.label("Height");
                    ui.drag_int("video_height", &mut app.ai.video_settings.height, 64);
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
                if let Err(error) = app.ai.video_settings.validate() { ui.label(&error); }
            });
        });
        if running { cancel = ui.button("video_cancel", "Cancel").clicked(); ui.request_repaint(); }
        else {
            let source_valid = app.workspace.documents.iter().find(|d| d.id == app.ai.document_id)
                .is_some_and(|d| d.project.width > 0 && d.project.height > 0
                    && app.ai.video_layer.is_none_or(|i| i < d.project.layers.len()));
            let end_valid = app.ai.video_end_document.as_ref().is_none_or(|id| {
                app.workspace.documents.iter().find(|d| &d.id == id)
                    .is_some_and(|d| d.project.width > 0 && d.project.height > 0
                        && app.ai.video_end_layer.is_none_or(|i| i < d.project.layers.len()))
            });
            ui.add_enabled(source_valid && end_valid && app.ai.config.is_some() && app.ai.video_settings.validate().is_ok(), |ui| {
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
