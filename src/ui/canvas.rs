use glam::Vec2;
use mega_ui::Ui;

use crate::app::{EditorApp, rect_size};
use crate::editing::Tool;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let size = ui.available_size().max(Vec2::splat(1.0));
    let area = ui.area("canvas_view", size);
    app.canvas_rect = area.rect;

    // Background only — image / checker / brush / selection are drawn by canvas wgpu.
    ui.fill_rect(app.canvas_rect, [0.075, 0.08, 0.095, 1.0]);
    if app.display_size().is_some() {
        // Host renders the canvas into a viewport-sized texture before UI composition.
        let viewport = app.viewport_size;
        if viewport.x > 0.0 && viewport.y > 0.0 {
            let min = app.canvas_rect.min / viewport;
            let max = app.canvas_rect.max / viewport;
            ui.texture_rect(crate::host::CANVAS_TEXTURE, app.canvas_rect, min.to_array(), max.to_array());
        }
    }

    if app.ai.open || app.hsv.open { return; }

    if app.fit_image {
        app.fit_to_canvas();
    }
    app.update_view();

    if let Some(view) = app.canvas_view() {
        if area.hovered || area.active || app.brush_last_active() {
            app.handle_canvas_edit(view);
            app.canvas_cursor = app.transform_cursor(view);
        }
        if app.preview_asset.is_none() {
            app.draw_lasso_path(ui, view);
        }
        if app.selection.is_some() || app.selection_anchor.is_some() {
            ui.request_repaint();
        }
        if (matches!(app.tool, Tool::Brush | Tool::SelectionBrush) || app.has_floating_transform())
            && app.canvas_rect.contains(app.input.mouse_pos)
        {
            ui.request_repaint();
        }
    } else {
        let message = "Place an asset or resize canvas to begin";
        let pos = app.canvas_rect.min + rect_size(app.canvas_rect) * 0.5 - Vec2::new(130.0, 8.0);
        ui.text_at(pos, message);
    }
}
