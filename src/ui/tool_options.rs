use mega_ui::Ui;

use super::value_slider::{ValueSlider, ValueSliderState};
use crate::app::DocumentSession;
use crate::editing::Tool;

/// `Some(horizontal)` asks the editor to flip the active transform.
pub fn draw(
    ui: &mut Ui,
    session: &mut DocumentSession,
    image_ready: bool,
    sliders: &mut ValueSliderState,
) -> Option<bool> {
    let mut flip = None;

    match session.tool {
        Tool::Deform => {
            ui.select(
                "deform_mode",
                &mut session.deform_mode,
                &["Move", "Inflate", "Pinch", "Normalize", "Rotate"],
            );
            ui.label("Radius (px)");
            ui.slider("deform_radius", &mut session.deform_radius, 4.0..=512.0);
            ui.label("Strength");
            ui.slider("deform_strength", &mut session.deform_strength, 0.01..=1.0);
            ui.label("Softness");
            ui.slider("deform_softness", &mut session.deform_softness, 0.0..=1.0);
            ui.label("Mesh divisions (long edge)");
            ui.slider_stepped(
                "deform_density",
                &mut session.deform_density,
                8.0..=96.0,
                1.0,
            );
            ui.checkbox("deform_grid", "Show mesh", &mut session.deform_grid);
            ui.label("Press captures vertices until release.");
            ui.label(match session.deform_mode {
                1 | 2 => "Drag away from the press point to change size.",
                3 => "Drag to smooth the mesh using its current neighbours.",
                4 => "Drag left/right to rotate around the brush center.",
                _ => "Drag to move the captured vertices.",
            });
            ui.label("Enter — apply / Esc — cancel");
            session.deform_apply |= ui.button("deform_apply", "Apply").clicked();
            session.deform_cancel |= ui.button("deform_cancel", "Cancel").clicked();
            ui.label("Mesh density takes effect after Apply/Cancel.");
        }
        Tool::Brush => {
            ui.column_with(
                mega_ui::LayoutOpts {
                    spacing: Some(8.0),
                    ..Default::default()
                },
                |ui| {
                    ui.row_with(
                        mega_ui::LayoutOpts {
                            spacing: Some(8.0),
                            ..Default::default()
                        },
                        |ui| {
                            ui.checkbox("brush_erase", "Erase mode", &mut session.erase_mode);
                            if !session.erase_mode {
                                ui.space(8.0);
                                ui.label("Color");
                                ui.color_edit("brush_color", &mut session.brush_color);
                            }
                        },
                    );
                    sliders.draw(
                        ui,
                        "brush_size",
                        &mut session.brush_size,
                        ValueSlider::new("Size", 1.0, 256.0, 1.0, "px"),
                    );

                    sliders.draw(
                        ui,
                        "brush_hardness",
                        &mut session.brush_hardness,
                        ValueSlider::percent("Hardness", 0.0),
                    );

                    sliders.draw(
                        ui,
                        "brush_opacity",
                        &mut session.brush_opacity,
                        ValueSlider::percent("Opacity", 0.01),
                    );
                },
            );
        }
        Tool::RectangleSelect
        | Tool::Lasso
        | Tool::PolygonSelect
        | Tool::SelectionBrush
        | Tool::MagicWand => {
            draw_select_mode(ui, session);
        }
        Tool::Fill => {
            ui.checkbox("fill_erase", "Erase mode", &mut session.erase_mode);
            ui.space(10.0);
            if !session.erase_mode {
                ui.label("Color");
                ui.color_edit("fill_color", &mut session.brush_color);
                ui.space(12.0);
            }
            ui.label(&format!("Opacity: {:.0}%", session.brush_opacity * 100.0));
            ui.slider("fill_opacity", &mut session.brush_opacity, 0.01..=1.0);
            ui.space(12.0);
            ui.label(&format!("Threshold: {:.0}", session.fill_threshold));
            ui.slider_stepped(
                "fill_threshold",
                &mut session.fill_threshold,
                0.0..=255.0,
                1.0,
            );
            ui.space(8.0);
            ui.label("Ctrl — eyedropper");
        }
        Tool::Transform => {
            ui.add_enabled(image_ready && session.preview_asset.is_none(), |ui| {
                ui.horizontal(|ui| {
                    if ui.button("transform_flip_x", "Flip X").clicked() {
                        flip = Some(true);
                    }
                    if ui.button("transform_flip_y", "Flip Y").clicked() {
                        flip = Some(false);
                    }
                });
            });
            ui.space(8.0);
            ui.checkbox(
                "transform_keep_aspect_ratio",
                "Keep aspect ratio",
                &mut session.transform_keep_aspect_ratio,
            );
            ui.space(8.0);
            ui.label("Inside — move · corners — scale");
            ui.label("Outside — rotate");
            ui.label("Center cross — move pivot");
            ui.space(8.0);
            ui.label("Enter — apply · Esc — cancel");
            ui.space(8.0);
            ui.label("No selection → transforms whole layer.");
        }
    }

    if let Some(rect) = session.selection.as_ref().and_then(|sel| sel.bounds()) {
        ui.space(16.0);
        ui.separator();
        ui.space(8.0);
        ui.label(&format!(
            "Size: {}px × {}px",
            rect.max_x.saturating_sub(rect.min_x),
            rect.max_y.saturating_sub(rect.min_y)
        ));
    }
    flip
}

fn draw_select_mode(ui: &mut Ui, session: &mut DocumentSession) {
    let mut mode = session.tool.select_mode().unwrap_or(0);
    if ui
        .toggle("select_mode", &mut mode, &Tool::SELECT_MODE_LABELS)
        .changed()
    {
        let tool = Tool::SELECT_MODES[mode];
        session.lasso_points.clear();
        session.selection_brush_last = None;
        session.selection_anchor = None;
        session.selection_current = None;
        session.select_tool = tool;
        session.tool = tool;
    }
    ui.space(12.0);
    match session.tool {
        Tool::SelectionBrush => {
            ui.checkbox(
                "selection_brush_erase",
                "Erase selection",
                &mut session.selection_brush_erase,
            );
            ui.label(&format!("Size: {:.0} px", session.brush_size));
            ui.slider_stepped(
                "selection_brush_size",
                &mut session.brush_size,
                1.0..=256.0,
                1.0,
            );
        }
        Tool::MagicWand => {
            ui.label(&format!("Threshold: {:.0}", session.wand_threshold));
            ui.slider_stepped(
                "wand_threshold",
                &mut session.wand_threshold,
                0.0..=255.0,
                1.0,
            );
        }
        Tool::PolygonSelect => {
            ui.label("Click points · click first or Enter to close");
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    #[test]
    fn brush_header_color_and_erase_stay_above_sliders_in_a_tall_panel() {
        let mut ui = Ui::new();
        let mut session = DocumentSession::default();
        let mut sliders = ValueSliderState::default();
        let mut output = mega_ui::UiOutput::default();
        for _ in 0..3 {
            ui.begin_frame(mega_ui::UiInput { viewport: Vec2::new(440.0, 600.0), ..Default::default() });
            draw(&mut ui, &mut session, true, &mut sliders);
            output = ui.end_frame();
        }
        assert!(output.draw_list.iter().any(|c| {
            c.colors == [[0.0, 0.0, 0.0, 1.0]; 4] && c.rect.width() > 30.0
                && c.rect.min.y >= 0.0 && c.rect.max.y <= 28.0
        }), "color swatch must be in the header, not centered down the panel");
        for pressed in [true, false] {
            ui.begin_frame(mega_ui::UiInput {
                viewport: Vec2::new(440.0, 600.0), mouse_pos: Vec2::new(8.0, 8.0),
                mouse_pressed: pressed, mouse_down: pressed, mouse_released: !pressed,
                ..Default::default()
            });
            draw(&mut ui, &mut session, true, &mut sliders);
            ui.end_frame();
        }
        assert!(session.erase_mode, "Erase mode must be clickable in the top row");
    }
}
