use glam::Vec2;
use mega_ui::{Rect, Ui};

use crate::app::{rect_size, EditorApp, IMAGE_TEXTURE};

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let size = ui.available_size().max(Vec2::splat(1.0));
    let area = ui.area("canvas_view", size);
    app.canvas_rect = area.rect;

    ui.fill_rect(app.canvas_rect, [0.075, 0.08, 0.095, 1.0]);

    if app.fit_image {
        app.fit_to_canvas();
    }
    app.update_view();

    if let Some((width, height)) = app.image_size {
        let image_size = Vec2::new(width as f32, height as f32) * app.zoom;
        let center = app.canvas_rect.min + rect_size(app.canvas_rect) * 0.5 + app.pan;
        let image_rect = Rect::from_min_size(center - image_size * 0.5, image_size);
        let shadow = Rect {
            min: image_rect.min - Vec2::splat(1.0),
            max: image_rect.max + Vec2::splat(1.0),
        };
        if let Some(visible_shadow) = shadow.intersect(app.canvas_rect) {
            ui.fill_rect(visible_shadow, [0.0, 0.0, 0.0, 0.8]);
        }
        if let Some(visible) = image_rect.intersect(app.canvas_rect) {
            let uv_min = [
                ((visible.min.x - image_rect.min.x) / image_size.x).clamp(0.0, 1.0),
                ((visible.min.y - image_rect.min.y) / image_size.y).clamp(0.0, 1.0),
            ];
            let uv_max = [
                ((visible.max.x - image_rect.min.x) / image_size.x).clamp(0.0, 1.0),
                ((visible.max.y - image_rect.min.y) / image_size.y).clamp(0.0, 1.0),
            ];
            ui.texture_rect(IMAGE_TEXTURE, visible, uv_min, uv_max);
        }
    } else {
        let message = "Import or paste an image to begin";
        let pos = app.canvas_rect.min + rect_size(app.canvas_rect) * 0.5 - Vec2::new(110.0, 8.0);
        ui.text_at(pos, message);
    }
}
