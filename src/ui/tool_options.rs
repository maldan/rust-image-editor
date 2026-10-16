use mega_ui::Ui;

use crate::app::EditorApp;
use crate::editing::Tool;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    ui.label("Tool Options");
    ui.separator();
    ui.space(8.0);

    match app.tool {
        Tool::Brush | Tool::Eraser => {
            ui.label("Color");
            ui.color_edit("brush_color", &mut app.brush_color);

            ui.space(14.0);
            ui.label(&format!("Size: {:.0} px", app.brush_size));
            ui.slider_stepped("brush_size", &mut app.brush_size, 1.0..=256.0, 1.0);

            ui.space(12.0);
            ui.label(&format!("Opacity: {:.0}%", app.brush_opacity * 100.0));
            ui.slider("brush_opacity", &mut app.brush_opacity, 0.01..=1.0);
        }
        Tool::RectangleSelect => {
            ui.label("Drag on the image to select an area.");
        }
        Tool::MagicWand => {
            ui.label("Click a pixel to select a connected region.");
            ui.space(12.0);
            ui.label(&format!("Threshold: {:.0}", app.wand_threshold));
            ui.slider_stepped("wand_threshold", &mut app.wand_threshold, 0.0..=255.0, 1.0);
        }
        Tool::Fill => {
            ui.label("Color");
            ui.color_edit("fill_color", &mut app.brush_color);
            ui.space(12.0);
            ui.label(&format!("Opacity: {:.0}%", app.brush_opacity * 100.0));
            ui.slider("fill_opacity", &mut app.brush_opacity, 0.01..=1.0);
            ui.space(12.0);
            ui.label(&format!("Threshold: {:.0}", app.fill_threshold));
            ui.slider_stepped("fill_threshold", &mut app.fill_threshold, 0.0..=255.0, 1.0);
        }
    }

    ui.space(16.0);
    ui.separator();
    ui.label("Selection");
    ui.space(8.0);
    if let Some(rect) = app.selection.as_ref().and_then(|sel| sel.bounds()) {
        ui.label(&format!(
            "{} × {} px",
            rect.max_x.saturating_sub(rect.min_x),
            rect.max_y.saturating_sub(rect.min_y)
        ));
        ui.space(8.0);
        if ui.button("clear_selection", "Clear selection").clicked() {
            app.clear_selection();
        }
    } else {
        ui.label("None — whole image is editable.");
    }
}
