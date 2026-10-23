use super::ai_dialog::choose;
use crate::app::EditorApp;
use crate::comfyui::{LTX_FPS, MAX_DURATION_SCALE};
use glam::Vec2;
use mega_ui::{ScrollAxes, Ui, Window};

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let mut open = true;
    let mut run = false;
    let mut cancel = false;
    let mut accept = false;
    let running = app.ai.job.is_some() || app.ai.frame_decode.is_some();
    let has_preview = app.ai.anim_frames.as_ref().is_some_and(|frames| !frames.is_empty());
    ui.modal(
        Window::new("ltx_animation_dialog", "Ltx 2.3")
            .size(Vec2::new(1100.0, 760.0))
            .resizable(true)
            .open(&mut open),
        |ui| {
            let controls_width = (ui.available_size().x * 0.42).min(420.0 * ui.scale());
            ui.horizontal(|ui| {
                ui.flex_width(controls_width, |ui| {
                    draw_controls(ui, app, running, &mut run, &mut cancel, &mut accept);
                });
                ui.flex(1.0, |ui| draw_preview(ui, app, has_preview));
            });
        },
    );
    if accept {
        app.accept_animation();
    }
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

fn draw_controls(ui: &mut Ui, app: &mut EditorApp, running: bool, run: &mut bool, cancel: &mut bool, accept: &mut bool) {
            let height = (ui.available_size().y - 150.0 * ui.scale()).max(160.0 * ui.scale());
            ui.scroll_area(
                "animation_settings",
                Vec2::new(ui.available_size().x, height),
                ScrollAxes::Vertical,
                |ui| {
                    ui.add_enabled(!running, |ui| {
                        let length = app
                            .workspace
                            .documents
                            .iter()
                            .find(|d| d.id == app.ai.document_id)
                            .and_then(|d| d.project.animation.as_ref())
                            .map(|a| a.length)
                            .unwrap_or(0);
                        if let Some(doc) = app.workspace.documents.iter().find(|d| d.id == app.ai.document_id) {
                            ui.label(&format!(
                                "{} · {} x {} px · {} frames",
                                doc.name, doc.project.width, doc.project.height, length
                            ));
                        }
                        if length > 0 {
                            let mut start = app.ai.anim_start as i32 + 1;
                            let mut end = app.ai.anim_end as i32 + 1;
                            let previous_start = start;
                            let previous_end = end;
                            ui.horizontal(|ui| {
                                ui.label("From frame");
                                ui.drag_int("anim_start", &mut start, 1);
                                ui.label("To frame");
                                ui.drag_int("anim_end", &mut end, 1);
                            });
                            start = start.clamp(1, length as i32);
                            end = end.clamp(1, length as i32);
                            if start != previous_start && start > end {
                                end = start;
                            }
                            if end != previous_end && end < start {
                                start = end;
                            }
                            app.ai.anim_start = start as u32 - 1;
                            app.ai.anim_end = end as u32 - 1;
                            app.ai.anim_apply = (app.ai.anim_start, app.ai.anim_end);
                        }
                        ui.label("The first and last frames are the video's start and end. A single frame uses image-to-video.");
                        ui.horizontal(|ui| {
                            ui.label("Width");
                            ui.drag_int("anim_width", &mut app.ai.video_settings.width, 64);
                            ui.label("Height");
                            ui.drag_int("anim_height", &mut app.ai.video_settings.height, 64);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Duration scale");
                            ui.drag_int("anim_scale", &mut app.ai.anim_scale, 1);
                        });
                        app.ai.anim_scale = app.ai.anim_scale.clamp(1, MAX_DURATION_SCALE);
                        ui.label(&format!(
                            "Fixed {LTX_FPS} fps. Scale 1 matches the frame range; 2 or 3 generates a longer clip. Extra frames are dropped to fit the range, and a short clip repeats frames."
                        ));
                        ui.label("The canvas is center-cropped to the video size.");
                        match app.animation_settings() {
                            Ok(settings) => {
                                let span = app.ai.anim_end - app.ai.anim_start + 1;
                                ui.label(&format!(
                                    "{span} frames generate {} s at {LTX_FPS} fps, then fit back onto those frames",
                                    settings.seconds
                                ));
                            }
                            Err(error) => ui.label(&error),
                        }
                        ui.label("Prompt");
                        ui.text_area(
                            "anim_prompt",
                            &mut app.ai.video_settings.prompt,
                            Vec2::new(0.0, 130.0),
                        );
                        for i in 0..app.ai.video_settings.loras.len() {
                            if i > 0 && app.ai.video_settings.loras[i - 1].name.is_none() {
                                for lora in &mut app.ai.video_settings.loras[i..] {
                                    lora.name = None;
                                }
                                break;
                            }
                            let lora = &mut app.ai.video_settings.loras[i];
                            ui.label(&format!("LoRA {}", i + 1));
                            choose(
                                ui,
                                &format!("anim_lora_{i}"),
                                &mut lora.name,
                                &app.ai.catalog.loras,
                                "None",
                            );
                            ui.add_enabled(lora.name.is_some(), |ui| {
                                ui.slider(&format!("anim_strength_{i}"), &mut lora.strength, 0.0..=1.0);
                            });
                        }
                        ui.add_enabled(app.ai.config.is_some() && app.ai.catalog_load.is_none(), |ui| {
                            if ui.button("anim_refresh", "Refresh LoRAs").clicked() {
                                app.ai.catalog_load = Some(crate::comfyui::CatalogLoad::start(
                                    app.ai.config.clone().unwrap(),
                                    true,
                                ));
                            }
                        });
                        ui.label(&app.ai.catalog_message);
                    });
                },
            );
            let has_preview = app.ai.anim_frames.as_ref().is_some_and(|frames| !frames.is_empty());
            if running {
                *cancel = ui.button("anim_cancel", "Cancel").clicked();
                ui.request_repaint();
            } else {
                ui.horizontal(|ui| {
                    if has_preview {
                        *accept = ui.button("anim_accept", "Accept").clicked();
                    }
                    ui.add_enabled(
                        app.ai.config.is_some() && app.animation_settings().is_ok(),
                        |ui| {
                            *run = ui
                                .button(
                                    "anim_run",
                                    if has_preview { "Regenerate" } else { "Generate" },
                                )
                                .clicked();
                        },
                    );
                });
            }
            if let Some(fraction) = app.ai.fraction {
                ui.progress_bar(fraction);
            }
            ui.label(&app.ai.message);
}

fn draw_preview(ui: &mut Ui, app: &mut EditorApp, has_preview: bool) {
    let image_size = {
        let Some(frame) = app
            .ai
            .anim_frames
            .as_ref()
            .and_then(|frames| frames.get(app.ai.anim_shown))
        else {
            ui.label("Preview plays here. Accept adds the frames to the animation.");
            return;
        };
        Vec2::new(frame.width() as f32, frame.height() as f32)
    };
    let available = ui.available_size().max(Vec2::splat(1.0));
    let area = ui.area("anim_preview_view", available);
    app.ai.preview_view.update(area.rect, area.hovered, &app.ai.preview_input);
    ui.fill_rect(area.rect, [0.075, 0.08, 0.095, 1.0]);
    if let Some((rect, uv_min, uv_max)) = app.ai.preview_view.visible_image(area.rect, image_size) {
        ui.texture_rect(
            crate::host::AI_PREVIEW_TEXTURE,
            rect,
            uv_min.to_array(),
            uv_max.to_array(),
        );
    }
    ui.horizontal(|ui| {
        if ui.button("anim_preview_reset", "Fit preview").clicked() {
            app.ai.preview_view = Default::default();
        }
        let count = app.ai.anim_frames.as_ref().map(|frames| frames.len()).unwrap_or(0);
        ui.label(&format!(
            "{} / {} · {LTX_FPS} fps · Wheel: zoom · Middle drag: pan",
            app.ai.anim_shown + 1,
            count
        ));
    });
    if has_preview {
        ui.request_repaint();
    }
}
