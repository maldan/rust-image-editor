use glam::Vec2;
use mega_ui::Ui;

use crate::app::{EditorApp, rect_size};

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let size = ui.available_size().max(Vec2::splat(1.0));
    let area = ui.area("canvas_view", size);
    app.session.canvas_rect = area.rect;

    // Background only — image / checker / brush / selection are drawn by canvas wgpu.
    ui.fill_rect(app.session.canvas_rect, [0.075, 0.08, 0.095, 1.0]);
    if app.display_size().is_some() {
        // Host renders the canvas into a viewport-sized texture before UI composition.
        let viewport = app.session.viewport_size;
        if viewport.x > 0.0 && viewport.y > 0.0 {
            let min = app.session.canvas_rect.min / viewport;
            let max = app.session.canvas_rect.max / viewport;
            ui.texture_rect(
                crate::host::CANVAS_TEXTURE,
                app.session.canvas_rect,
                min.to_array(),
                max.to_array(),
            );
        }
    }

    if app.ai.open || app.hsv.open || app.rename.is_some() {
        return;
    }

    if app.session.fit_image {
        app.fit_to_canvas();
    }
    app.update_view();

    if let Some(view) = app.canvas_view() {
        if area.hovered || area.active || app.brush_last_active() {
            app.handle_canvas_edit(view);
            app.session.canvas_cursor = app.transform_cursor(view);
        }
        if app.eyedropper_armed() && app.session.canvas_rect.contains(app.input.mouse_pos) {
            app.session.canvas_cursor = Some(crate::editing::TransformCursor::Eyedropper);
        }
        if app.session.preview_asset.is_none() {
            app.draw_lasso_path(ui, view);
        }
        // Marching ants advance with frame time. The brush ring does not.
        if app.session.preview_asset.is_none() && app.session.selection.is_some() {
            ui.request_repaint();
        }
    } else {
        let message = if app.session.preview_asset.is_some() {
            app.status.as_str()
        } else {
            "Place an asset or resize canvas to begin"
        };
        let pos = app.session.canvas_rect.min + rect_size(app.session.canvas_rect) * 0.5
            - Vec2::new(130.0, 8.0);
        ui.text_at(pos, message);
    }
}
