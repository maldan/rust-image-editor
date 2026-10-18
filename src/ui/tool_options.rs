use mega_ui::Ui;

use crate::app::EditorApp;
use crate::editing::Tool;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    ui.label("Tool Options");
    ui.separator();
    ui.space(8.0);

    match app.tool {
        Tool::Brush => {
            ui.checkbox("brush_erase", "Erase mode", &mut app.erase_mode);
            ui.space(10.0);
            if !app.erase_mode {
                ui.label("Color");
                ui.color_edit("brush_color", &mut app.brush_color);
                ui.space(14.0);
            }
            ui.label(&format!("Size: {:.0} px", app.brush_size));
            ui.slider_stepped("brush_size", &mut app.brush_size, 1.0..=256.0, 1.0);

            ui.space(12.0);
            ui.label(&format!("Hardness: {:.0}%", app.brush_hardness * 100.0));
            ui.slider("brush_hardness", &mut app.brush_hardness, 0.0..=1.0);

            ui.space(12.0);
            ui.label(&format!("Opacity: {:.0}%", app.brush_opacity * 100.0));
            ui.slider("brush_opacity", &mut app.brush_opacity, 0.01..=1.0);
            ui.space(8.0);
            ui.label("Ctrl — eyedropper");
        }
        Tool::RectangleSelect | Tool::Lasso => {
            ui.label(if app.tool == Tool::Lasso { "Draw a freehand outline to select." } else { "Drag on the image to select an area." });
            ui.space(8.0);
            ui.label("Shift — add · Ctrl — subtract");
        }
        Tool::SelectionBrush => {
            ui.checkbox("selection_brush_erase", "Erase selection", &mut app.selection_brush_erase);
            ui.label("Paint to add to selection.");
            ui.label(&format!("Size: {:.0} px", app.brush_size));
            ui.slider_stepped("selection_brush_size", &mut app.brush_size, 1.0..=256.0, 1.0);
        }
        Tool::MagicWand => {
            ui.label("Click a pixel to select a connected region.");
            ui.space(8.0);
            ui.label("Shift — add · Ctrl — subtract");
            ui.space(12.0);
            ui.label(&format!("Threshold: {:.0}", app.wand_threshold));
            ui.slider_stepped("wand_threshold", &mut app.wand_threshold, 0.0..=255.0, 1.0);
        }
        Tool::Fill => {
            ui.checkbox("fill_erase", "Erase mode", &mut app.erase_mode);
            ui.space(10.0);
            if !app.erase_mode {
                ui.label("Color");
                ui.color_edit("fill_color", &mut app.brush_color);
                ui.space(12.0);
            }
            ui.label(&format!("Opacity: {:.0}%", app.brush_opacity * 100.0));
            ui.slider("fill_opacity", &mut app.brush_opacity, 0.01..=1.0);
            ui.space(12.0);
            ui.label(&format!("Threshold: {:.0}", app.fill_threshold));
            ui.slider_stepped("fill_threshold", &mut app.fill_threshold, 0.0..=255.0, 1.0);
        }
        Tool::Transform => {
            ui.add_enabled(app.image_size.is_some() && app.preview_asset.is_none(), |ui| {
                ui.horizontal(|ui| {
                    if ui.button("transform_flip_x", "Flip X").clicked() { app.flip_transform(true); }
                    if ui.button("transform_flip_y", "Flip Y").clicked() { app.flip_transform(false); }
                });
            });
            ui.space(8.0);
            ui.checkbox("transform_keep_aspect_ratio", "Keep aspect ratio", &mut app.transform_keep_aspect_ratio);
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
        &mut app.dim_outside_selection,
    );
    ui.space(8.0);
    if let Some(rect) = app.selection.as_ref().and_then(|sel| sel.bounds()) {
        ui.label(&format!(
            "{} × {} px",
            rect.max_x.saturating_sub(rect.min_x),
            rect.max_y.saturating_sub(rect.min_y)
        ));
        ui.space(8.0);
    } else {
        ui.label(if app.selection.is_some() { "Empty selection" } else { "None — whole image is editable." });
    }
    if app.selection.is_some() && ui.button("clear_selection", "Clear selection").clicked() {
        app.clear_selection();
    }
}
