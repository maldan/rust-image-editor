use glam::Vec2;
use mega_ui::{Ui, Window};

use crate::{
    app::EditorApp,
    comfyui::{HostConfig, Job},
};

pub struct AiDialog {
    pub open: bool,
    pub whole_document: bool,
    pub scale: f32,
    pub prompt: String,
    pub config: Option<HostConfig>,
    pub job: Option<Job>,
    pub document_id: String,
    pub message: String,
    pub fraction: Option<f32>,
    pub preview: Option<image::RgbaImage>,
    /// Full-resolution server output, held until explicitly accepted.
    pub result: Option<image::RgbaImage>,
    pub preview_dirty: bool,
    pub close_when_stopped: bool,
}

impl Default for AiDialog {
    fn default() -> Self {
        Self {
            open: false,
            whole_document: false,
            scale: 1.0,
            prompt: String::new(),
            config: None,
            job: None,
            document_id: String::new(),
            message: String::new(),
            fraction: None,
            preview: None,
            result: None,
            preview_dirty: false,
            close_when_stopped: false,
        }
    }
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    if !app.ai.open {
        return;
    }
    let running = app.ai.job.is_some();
    let cancelling = app.ai.job.as_ref().is_some_and(Job::is_cancelling);
    let mut open = true;
    let mut run = false;
    let mut cancel = false;
    let mut accept = false;
    ui.modal(
        Window::new("ai_edit_image", "Edit Image")
            .size(Vec2::new(1120.0, 700.0))
            .resizable(true)
            .open(&mut open),
        |ui| {
            // Keep controls beside the image so its height is not consumed by the form.
            let controls_width = (ui.available_size().x * 0.35).min(380.0 * ui.scale());
            ui.horizontal(|ui| {
                ui.flex_width(controls_width, |ui| {
                    ui.add_enabled(!running, |ui| {
                        ui.label("Source");
                        ui.horizontal(|ui| {
                            if ui
                                .selectable("ai_layer", !app.ai.whole_document, |ui| {
                                    ui.label("Current layer");
                                })
                                .clicked()
                            {
                                app.ai.whole_document = false;
                            }
                            if ui
                                .selectable("ai_document", app.ai.whole_document, |ui| {
                                    ui.label("Whole document");
                                })
                                .clicked()
                            {
                                app.ai.whole_document = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Input scale");
                            ui.drag_float("ai_scale", &mut app.ai.scale, 0.01);
                        });
                        match crate::comfyui::scaled_size(
                            app.project().width,
                            app.project().height,
                            app.ai.scale,
                        ) {
                            Ok((w, h)) => ui.label(&format!("Input: {w} × {h} px")),
                            Err(error) => ui.label(&error),
                        };
                        ui.label("Prompt");
                        ui.text_area("ai_prompt", &mut app.ai.prompt, Vec2::new(0.0, 110.0));
                    });
                    ui.space(8.0);
                    if running {
                        ui.add_enabled(!cancelling, |ui| {
                            cancel = ui
                                .button(
                                    "ai_cancel",
                                    if cancelling {
                                        "Cancelling…"
                                    } else {
                                        "Cancel"
                                    },
                                )
                                .clicked();
                        });
                    } else {
                        let valid = app.ai.config.is_some()
                            && !app.ai.prompt.trim().is_empty()
                            && crate::comfyui::scaled_size(
                                app.project().width,
                                app.project().height,
                                app.ai.scale,
                            )
                            .is_ok()
                            && (app.ai.whole_document || app.project().active_pixels().is_some());
                        ui.horizontal(|ui| {
                            let has_result = app.ai.result.is_some();
                            if has_result {
                                accept = ui.button("ai_accept", "Accept").clicked();
                            }
                            ui.add_enabled(valid, |ui| {
                                run = ui
                                    .button("ai_run", if has_result { "Regenerate" } else { "Run" })
                                    .clicked();
                            });
                        });
                    }
                    ui.space(8.0);
                    if let Some(fraction) = app.ai.fraction {
                        ui.progress_bar(fraction);
                    }
                });
                ui.flex(1.0, |ui| {
                    if let Some(preview) = &app.ai.preview {
                        let available = ui.available_size().max(Vec2::splat(1.0));
                        let factor = (available.x / preview.width() as f32)
                            .min(available.y.min(560.0 * ui.scale()) / preview.height() as f32);
                        ui.texture(
                            crate::host::AI_PREVIEW_TEXTURE,
                            Vec2::new(preview.width() as f32, preview.height() as f32) * factor,
                        );
                    }
                });
            });
            ui.label(&app.ai.message);
        },
    );
    if accept {
        app.accept_ai();
    } else if run {
        app.start_ai();
    }
    if cancel {
        app.cancel_ai();
    }
    if !open || app.input.key_escape {
        app.close_ai();
    }
}
