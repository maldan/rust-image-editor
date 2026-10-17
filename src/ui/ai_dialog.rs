use glam::Vec2;
use mega_ui::{Rect, Ui, Window};

use crate::{
    app::EditorApp,
    comfyui::{HostConfig, Job},
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum AiMode {
    #[default]
    Edit,
    Draw,
    DrawCn,
    Redraw,
    Inpaint,
    Lineart,
}

impl AiMode {
    pub fn needs_input(self) -> bool {
        self != Self::Draw
    }
    pub fn uses_sd(self) -> bool {
        matches!(self, Self::Draw | Self::DrawCn | Self::Redraw | Self::Inpaint)
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Edit => "Edit Image",
            Self::Draw => "Draw Image",
            Self::DrawCn => "Draw with ControlNet",
            Self::Redraw => "Redraw Image",
            Self::Inpaint => "Inpaint Image",
            Self::Lineart => "To Lineart",
        }
    }
    pub fn layer_prefix(self) -> &'static str {
        match self {
            Self::Edit => "AI Edit",
            Self::Draw => "AI Draw",
            Self::DrawCn => "AI Draw CN",
            Self::Redraw => "AI Redraw",
            Self::Inpaint => "AI Inpaint",
            Self::Lineart => "AI Lineart",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum AiSource {
    #[default]
    Layer,
    Document,
    Selection,
}

pub struct PreviewView {
    zoom: f32,
    pan: Vec2,
    panning: bool,
}

impl Default for PreviewView {
    fn default() -> Self {
        Self { zoom: 1.0, pan: Vec2::ZERO, panning: false }
    }
}

impl PreviewView {
    fn update(&mut self, viewport: Rect, hovered: bool, input: &crate::host::FrameInput) {
        if input.mouse_middle_pressed && hovered {
            self.panning = true;
        }
        if !input.mouse_middle_down {
            self.panning = false;
        }
        if self.panning {
            self.pan += input.mouse_delta;
        }
        if hovered && input.scroll_delta.y.abs() > 0.01 {
            let old_zoom = self.zoom;
            self.zoom = (self.zoom * (input.scroll_delta.y * 0.002).exp()).clamp(0.1, 32.0);
            let center = (viewport.min + viewport.max) * 0.5;
            self.pan -= (input.mouse_pos - center - self.pan) * (self.zoom / old_zoom - 1.0);
        }
    }

    /// Crop both the rectangle and UVs so a zoomed image stays inside the preview pane.
    fn visible_image(&self, viewport: Rect, image_size: Vec2) -> Option<(Rect, Vec2, Vec2)> {
        let available = viewport.max - viewport.min;
        let fit = (available.x / image_size.x).min(available.y / image_size.y);
        let size = image_size * fit * self.zoom;
        let origin = (viewport.min + viewport.max - size) * 0.5 + self.pan;
        let clipped = Rect { min: origin.max(viewport.min), max: (origin + size).min(viewport.max) };
        if clipped.width() <= 0.0 || clipped.height() <= 0.0 { return None; }
        Some((clipped, (clipped.min - origin) / size, (clipped.max - origin) / size))
    }
}

pub struct AiDialog {
    pub open: bool,
    pub mode: AiMode,
    pub denoise: f32,
    pub mask_softness: f32,
    pub lineart_resolution: i32,
    pub draw_settings: crate::comfyui::DrawSettings,
    pub control_net: crate::comfyui::ControlNetSettings,
    pub catalog: crate::comfyui::Catalog,
    pub catalog_load: Option<crate::comfyui::CatalogLoad>,
    pub catalog_message: String,

    pub source: AiSource,
    /// Bounds captured on opening; editor selection cannot change while the modal is open.
    pub selection_bounds: Option<crate::editing::PixelRect>,
    pub scale: f32,
    pub fit_image: bool,
    /// Original input bounds in document coordinates, captured before input scaling.
    pub input_region: Option<crate::editing::PixelRect>,
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
    pub preview_view: PreviewView,
    pub preview_input: crate::host::FrameInput,
    pub close_when_stopped: bool,
}

impl Default for AiDialog {
    fn default() -> Self {
        Self {
            open: false,
            mode: AiMode::Edit,
            denoise: 1.0,
            mask_softness: 0.0,
            lineart_resolution: 512,
            draw_settings: Default::default(),
            control_net: Default::default(),
            catalog: Default::default(),
            catalog_load: None,
            catalog_message: String::new(),
            source: AiSource::Layer,
            selection_bounds: None,
            scale: 1.0,
            fit_image: true,
            input_region: None,
            prompt: String::new(),
            config: None,
            job: None,
            document_id: String::new(),
            message: String::new(),
            fraction: None,
            preview: None,
            result: None,
            preview_dirty: false,
            preview_view: PreviewView::default(),
            preview_input: Default::default(),
            close_when_stopped: false,
        }
    }
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    if !app.ai.open {
        return;
    }
    if let Some(load) = &app.ai.catalog_load {
        if let Some(result) = load.poll() {
            match result {
                Ok((catalog, message)) => {
                    app.ai.catalog = catalog;
                    app.ai.catalog_message = message;
                }
                Err(error) => app.ai.catalog_message = error,
            }
            app.ai.catalog_load = None;
        } else {
            ui.request_repaint();
        }
    }
    let (width, height) = (app.project().width, app.project().height);
    let running = app.ai.job.is_some();
    let cancelling = app.ai.job.as_ref().is_some_and(Job::is_cancelling);
    let mut open = true;
    let mut run = false;
    let mut cancel = false;
    let mut accept = false;
    ui.modal(
        Window::new("ai_edit_image", app.ai.mode.title())
            .size(Vec2::new(1120.0, 700.0))
            .resizable(true)
            .open(&mut open),
        |ui| {
            // Keep controls beside the image so its height is not consumed by the form.
            let controls_width = (ui.available_size().x * 0.35).min(380.0 * ui.scale());
            ui.horizontal(|ui| {
                ui.flex_width(controls_width, |ui| {
                    ui.add_enabled(!running, |ui| {
                        if app.ai.mode.uses_sd() {
                            draw_controls(ui, &mut app.ai, width, height);
                        } else {
                            source_controls(ui, &mut app.ai, width, height);
                            if app.ai.mode == AiMode::Lineart {
                                ui.label("Resolution");
                                ui.drag_int("lineart_resolution", &mut app.ai.lineart_resolution, 64);
                                if let Err(error) = crate::comfyui::validate_lineart_resolution(app.ai.lineart_resolution) {
                                    ui.label(&error);
                                }
                            } else {
                                ui.label("Prompt");
                                ui.text_area("ai_prompt", &mut app.ai.prompt, Vec2::new(0.0, 110.0));
                            }
                        }
                        ui.checkbox("ai_fit_image", "Fit Image", &mut app.ai.fit_image);
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
                            && match app.ai.mode {
                                AiMode::Draw => app.ai.draw_settings.validate().is_ok(),
                                AiMode::DrawCn => {
                                    app.ai.control_net.validate(&app.ai.draw_settings).is_ok()
                                        && input_valid(app, width, height)
                                }
                                AiMode::Lineart => {
                                    crate::comfyui::validate_lineart_resolution(app.ai.lineart_resolution).is_ok()
                                        && input_valid(app, width, height)
                                }
                                AiMode::Redraw | AiMode::Inpaint => {
                                    app.ai.draw_settings.validate_redraw(app.ai.denoise).is_ok()
                                        && input_valid(app, width, height)
                                }
                                AiMode::Edit => {
                                    !app.ai.prompt.trim().is_empty()
                                        && input_valid(app, width, height)
                                }
                            };
                        ui.horizontal(|ui| {
                            let has_result = app.ai.result.is_some();
                            if has_result {
                                accept = ui.button("ai_accept", "Accept").clicked();
                            }
                            ui.add_enabled(valid, |ui| {
                                run = ui
                                    .button("ai_run", if has_result || app.ai.preview.is_some() { "Regenerate" } else { "Run" })
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
                    if let Some(preview) = app.ai.result.as_ref().or(app.ai.preview.as_ref()) {
                        let image_size = Vec2::new(preview.width() as f32, preview.height() as f32);
                        let available = ui.available_size().max(Vec2::splat(1.0));
                        let size = Vec2::new(available.x, available.y.min(560.0 * ui.scale()));
                        let area = ui.area("ai_preview_view", size);
                        app.ai.preview_view.update(area.rect, area.hovered, &app.ai.preview_input);
                        ui.fill_rect(area.rect, [0.075, 0.08, 0.095, 1.0]);
                        if let Some((rect, uv_min, uv_max)) = app.ai.preview_view.visible_image(area.rect, image_size) {
                            ui.texture_rect(crate::host::AI_PREVIEW_TEXTURE, rect, uv_min.to_array(), uv_max.to_array());
                        }
                        ui.horizontal(|ui| {
                            if ui.button("ai_preview_reset", "Fit preview").clicked() {
                                app.ai.preview_view = PreviewView::default();
                            }
                            ui.label("Wheel: zoom · Middle drag: pan");
                        });
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

fn choose(ui: &mut Ui, id: &str, selection: &mut Option<String>, names: &[String], default: &str) {
    let mut options = vec![default];
    options.extend(names.iter().map(String::as_str));
    // Preserve a previous selection if a refresh no longer lists that file.
    if let Some(name) = selection.as_deref() {
        if !names.iter().any(|n| n == name) {
            options.push(name);
        }
    }
    let mut index = selection
        .as_deref()
        .and_then(|s| options.iter().position(|n| *n == s))
        .unwrap_or(0);
    if ui.select(id, &mut index, &options).changed() {
        let next = if index == 0 {
            None
        } else {
            Some(options[index].to_string())
        };
        *selection = next;
    }
}

fn draw_controls(ui: &mut Ui, ai: &mut AiDialog, width: u32, height: u32) {
    let form_height = (ui.available_size().y - 165.0 * ui.scale()).max(180.0 * ui.scale());
    ui.scroll_area(
        "draw_settings",
        Vec2::new(ui.available_size().x, form_height),
        mega_ui::ScrollAxes::Vertical,
        |ui| {
            if ai.mode.needs_input() {
                source_controls(ui, ai, width, height);
            }
            if matches!(ai.mode, AiMode::Redraw | AiMode::Inpaint) {
                ui.label("Denoise");
                ui.slider("redraw_denoise", &mut ai.denoise, 0.0..=1.0);
            }
            if ai.mode == AiMode::DrawCn {
                ui.label("ControlNet strength");
                ui.slider("cn_strength", &mut ai.control_net.strength, 0.0..=1.0);
                ui.label("ControlNet start (%)");
                let mut start = ai.control_net.start_percent * 100.0;
                if ui.slider("cn_start", &mut start, 0.0..=100.0).changed() {
                    ai.control_net.start_percent = start / 100.0;
                }
                ui.label("ControlNet end (%)");
                let mut end = ai.control_net.end_percent * 100.0;
                if ui.slider("cn_end", &mut end, 0.0..=100.0).changed() {
                    ai.control_net.end_percent = end / 100.0;
                }
            }
            ui.label("Model");
            choose(
                ui,
                "draw_model",
                &mut ai.draw_settings.model,
                &ai.catalog.models,
                "Default (workflow)",
            );
            ui.add_enabled(ai.config.is_some() && ai.catalog_load.is_none(), |ui| {
                if ui
                    .button("draw_refresh", "Refresh models / LoRAs")
                    .clicked()
                {
                    ai.catalog_load = Some(crate::comfyui::CatalogLoad::start(
                        ai.config.clone().unwrap(),
                        true,
                    ));
                    ai.catalog_message = "Refreshing models / LoRAs...".into();
                }
            });
            ui.label(&ai.catalog_message);
            for i in 0..ai.draw_settings.loras.len() {
                if i > 0 && ai.draw_settings.loras[i - 1].name.is_none() {
                    for lora in &mut ai.draw_settings.loras[i..] {
                        lora.name = None;
                    }
                    break;
                }
                let lora = &mut ai.draw_settings.loras[i];
                ui.label(&format!("LoRA {}", i + 1));
                choose(
                    ui,
                    &format!("draw_lora_{i}"),
                    &mut lora.name,
                    &ai.catalog.loras,
                    "None",
                );
                ui.add_enabled(lora.name.is_some(), |ui| {
                    ui.slider(&format!("draw_strength_{i}"), &mut lora.strength, 0.0..=1.0);
                });
            }
            ui.label("Positive prompt");
            ui.text_area(
                "draw_positive",
                &mut ai.draw_settings.positive,
                Vec2::new(0.0, 80.0),
            );
            ui.label("Negative prompt");
            ui.text_area(
                "draw_negative",
                &mut ai.draw_settings.negative,
                Vec2::new(0.0, 65.0),
            );
            if ai.mode == AiMode::Draw {
                ui.horizontal(|ui| {
                    ui.label("Width");
                    ui.drag_int("draw_width", &mut ai.draw_settings.width, 8);
                    ui.label("Height");
                    ui.drag_int("draw_height", &mut ai.draw_settings.height, 8);
                });
                if let Err(error) = ai.draw_settings.validate() {
                    ui.label(&error);
                }
            } else if ai.mode == AiMode::DrawCn {
                if let Err(error) = ai.control_net.validate(&ai.draw_settings) {
                    ui.label(&error);
                }
            } else if let Err(error) = ai.draw_settings.validate_redraw(ai.denoise) {
                ui.label(&error);
            }
        },
    );
}

fn input_size(ai: &AiDialog, width: u32, height: u32) -> Result<(u32, u32), String> {
    if ai.mode == AiMode::Inpaint && ai.selection_bounds.is_none() {
        return Err("Select an area to use as the inpaint mask".into());
    }
    let (width, height) = if ai.source == AiSource::Selection {
        let bounds = ai
            .selection_bounds
            .ok_or("Select an area on the canvas first")?;
        (bounds.max_x - bounds.min_x, bounds.max_y - bounds.min_y)
    } else {
        (width, height)
    };
    crate::comfyui::scaled_size(width, height, ai.scale)
}

fn input_valid(app: &EditorApp, width: u32, height: u32) -> bool {
    input_size(&app.ai, width, height).is_ok()
        && (app.ai.source != AiSource::Layer || app.project().active_pixels().is_some())
}

fn source_controls(ui: &mut Ui, ai: &mut AiDialog, width: u32, height: u32) {
    ui.label("Source");
    let sources: &[&str] = if ai.mode == AiMode::Inpaint {
        if ai.source == AiSource::Selection {
            ai.source = AiSource::Layer;
        }
        &["Current layer", "Whole document"]
    } else {
        &["Current layer", "Whole document", "Selection"]
    };
    let mut source = match ai.source {
        AiSource::Layer => 0,
        AiSource::Document => 1,
        AiSource::Selection => 2,
    };
    if ui.select("ai_source", &mut source, sources).changed() {
        ai.source = match source {
            1 => AiSource::Document,
            2 => AiSource::Selection,
            _ => AiSource::Layer,
        };
    }
    if ai.mode == AiMode::Inpaint {
        ui.label("Mask: selection (white = selected, black = outside)");
        ui.horizontal(|ui| {
            ui.label("Mask softness (px)");
            ui.drag_float("ai_mask_softness", &mut ai.mask_softness, 0.5);
        });
        ai.mask_softness = ai.mask_softness.clamp(0.0, 256.0);
    }
    ui.horizontal(|ui| {
        ui.label("Input scale");
        ui.drag_float("ai_scale", &mut ai.scale, 0.01);
    });
    match input_size(ai, width, height) {
        Ok((w, h)) => ui.label(&format!("Input: {w} \u{00d7} {h} px")),
        Err(error) => ui.label(&error),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lineart_defaults_to_512_and_uses_image_sources_without_sd() {
        let mut ai = AiDialog::default();
        ai.mode = AiMode::Lineart;
        assert_eq!(ai.lineart_resolution, 512);
        assert!(ai.mode.needs_input());
        assert!(!ai.mode.uses_sd());
        ai.source = AiSource::Selection;
        assert!(input_size(&ai, 128, 128).is_err());
        ai.selection_bounds = Some(crate::editing::PixelRect {
            min_x: 16, min_y: 24, max_x: 112, max_y: 104,
        });
        assert_eq!(input_size(&ai, 128, 128).unwrap(), (96, 80));
    }

    #[test]
    fn preview_zoom_preserves_the_pixel_under_the_cursor_and_clips_uvs() {
        let viewport = Rect::from_min_size(Vec2::new(100.0, 50.0), Vec2::new(400.0, 300.0));
        let mut view = PreviewView::default();
        let mut input = crate::host::FrameInput::default();
        input.mouse_pos = Vec2::new(350.0, 150.0);
        input.scroll_delta.y = 400.0;
        let center = (viewport.min + viewport.max) * 0.5;
        let before = (input.mouse_pos - center - view.pan) / view.zoom;
        view.update(viewport, true, &input);
        let after = (input.mouse_pos - center - view.pan) / view.zoom;
        assert!(before.distance(after) < 0.001);
        let (rect, min, max) = view.visible_image(viewport, Vec2::splat(1024.0)).unwrap();
        assert!(rect.min.cmpge(viewport.min).all() && rect.max.cmple(viewport.max).all());
        assert!(min.cmpge(Vec2::ZERO).all() && max.cmple(Vec2::ONE).all());
        assert!(min.x > 0.0 || min.y > 0.0 || max.x < 1.0 || max.y < 1.0);
    }

    #[test]
    fn preview_pan_stays_active_outside_until_release_and_ignores_other_controls() {
        let viewport = Rect::from_min_size(Vec2::ZERO, Vec2::splat(300.0));
        let mut view = PreviewView::default();
        let mut input = crate::host::FrameInput::default();
        input.scroll_delta.y = 120.0;
        input.mouse_middle_pressed = true;
        input.mouse_middle_down = true;
        input.mouse_delta = Vec2::new(10.0, 20.0);
        view.update(viewport, false, &input);
        assert_eq!((view.zoom, view.pan), (1.0, Vec2::ZERO));
        input.scroll_delta = Vec2::ZERO;
        view.update(viewport, true, &input);
        assert_eq!(view.pan, Vec2::new(10.0, 20.0));
        input.mouse_middle_pressed = false;
        view.update(viewport, false, &input);
        assert_eq!(view.pan, Vec2::new(20.0, 40.0));
        input.mouse_middle_down = false;
        view.update(viewport, false, &input);
        assert!(!view.panning);
        assert_eq!(view.pan, Vec2::new(20.0, 40.0));
        view.pan = Vec2::splat(10000.0);
        assert!(view.visible_image(viewport, Vec2::splat(1024.0)).is_none());
    }

    fn render_controls(ai: &mut AiDialog) -> usize {
        let mut ui = Ui::new();
        ui.begin_frame(mega_ui::UiInput {
            viewport: Vec2::new(600.0, 1400.0),
            ..Default::default()
        });
        ui.horizontal(|ui| ui.flex_width(400.0, |ui| draw_controls(ui, ai, 128, 128)));
        ui.end_frame().draw_list.len()
    }

    #[test]
    fn lora_rows_appear_in_order_and_hidden_selections_are_cleared() {
        for mode in [AiMode::Draw, AiMode::DrawCn, AiMode::Redraw, AiMode::Inpaint] {
            let mut ai = AiDialog::default();
            ai.mode = mode;
            let one = render_controls(&mut ai);
            ai.draw_settings.loras[0].name = Some("first".into());
            let two = render_controls(&mut ai);
            assert!(two > one);
            ai.draw_settings.loras[1].name = Some("second".into());
            let three = render_controls(&mut ai);
            assert!(three > two);
            ai.draw_settings.loras[2].name = Some("third".into());
            ai.draw_settings.loras[1].name = None;
            render_controls(&mut ai);
            assert!(ai.draw_settings.loras[2].name.is_none());
            ai.draw_settings.loras[1].name = Some("second".into());
            ai.draw_settings.loras[2].name = Some("third".into());
            ai.draw_settings.loras[0].name = None;
            render_controls(&mut ai);
            assert!(ai.draw_settings.loras.iter().all(|l| l.name.is_none()));
        }
    }

    #[test]
    fn inpaint_source_controls_exclude_selection_and_require_mask() {
        let mut ai = AiDialog::default();
        ai.mode = AiMode::Inpaint;
        ai.source = AiSource::Selection;
        render_controls(&mut ai);
        assert!(ai.source == AiSource::Layer);
        assert!(input_size(&ai, 128, 128).is_err());
        ai.selection_bounds = Some(crate::editing::PixelRect {
            min_x: 16,
            min_y: 24,
            max_x: 112,
            max_y: 104,
        });
        ai.scale = 0.5;
        assert_eq!(input_size(&ai, 128, 128).unwrap(), (64, 64));
    }
}
