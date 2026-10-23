use mega_ui::Ui;

use crate::app::DocumentSession;
use crate::editing::Tool;

/// `Some(horizontal)` asks the editor to flip the active transform.
pub fn draw(ui: &mut Ui, session: &mut DocumentSession, image_ready: bool) -> Option<bool> {
    let mut flip = None;
    ui.label("Tool Options");
    ui.separator();
    ui.space(8.0);

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
            ui.checkbox("brush_erase", "Erase mode", &mut session.erase_mode);
            ui.space(10.0);
            if !session.erase_mode {
                ui.label("Color");
                ui.color_edit("brush_color", &mut session.brush_color);
                ui.space(14.0);
            }
            ui.label(&format!("Size: {:.0} px", session.brush_size));
            ui.slider_stepped("brush_size", &mut session.brush_size, 1.0..=256.0, 1.0);

            ui.space(12.0);
            ui.label(&format!("Hardness: {:.0}%", session.brush_hardness * 100.0));
            ui.slider("brush_hardness", &mut session.brush_hardness, 0.0..=1.0);

            ui.space(12.0);
            ui.label(&format!("Opacity: {:.0}%", session.brush_opacity * 100.0));
            ui.slider("brush_opacity", &mut session.brush_opacity, 0.01..=1.0);
            ui.space(8.0);
            ui.label("Ctrl — eyedropper");
        }
        Tool::RectangleSelect | Tool::Lasso => {
            ui.label(if session.tool == Tool::Lasso {
                "Draw a freehand outline to select."
            } else {
                "Drag on the image to select an area."
            });
            ui.space(8.0);
            ui.label("Shift — add · Ctrl — subtract");
        }
        Tool::SelectionBrush => {
            ui.checkbox(
                "selection_brush_erase",
                "Erase selection",
                &mut session.selection_brush_erase,
            );
            ui.label("Paint to add to selection.");
            ui.label(&format!("Size: {:.0} px", session.brush_size));
            ui.slider_stepped(
                "selection_brush_size",
                &mut session.brush_size,
                1.0..=256.0,
                1.0,
            );
        }
        Tool::MagicWand => {
            ui.label("Click a pixel to select a connected region.");
            ui.space(8.0);
            ui.label("Shift — add · Ctrl — subtract");
            ui.space(12.0);
            ui.label(&format!("Threshold: {:.0}", session.wand_threshold));
            ui.slider_stepped(
                "wand_threshold",
                &mut session.wand_threshold,
                0.0..=255.0,
                1.0,
            );
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

    ui.space(16.0);
    ui.separator();
    ui.label("Selection");
    ui.space(8.0);
    ui.checkbox(
        "dim_outside",
        "Dim outside selection",
        &mut session.dim_outside_selection,
    );
    ui.space(8.0);
    if let Some(rect) = session.selection.as_ref().and_then(|sel| sel.bounds()) {
        ui.label(&format!(
            "{} × {} px",
            rect.max_x.saturating_sub(rect.min_x),
            rect.max_y.saturating_sub(rect.min_y)
        ));
        ui.space(8.0);
    } else {
        ui.label(if session.selection.is_some() {
            "Empty selection"
        } else {
            "None — whole image is editable."
        });
    }
    if session.selection.is_some() && ui.button("clear_selection", "Clear selection").clicked() {
        session.clear_selection();
    }
    flip
}
