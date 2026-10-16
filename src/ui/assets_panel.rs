use glam::Vec2;
use mega_ui::Ui;

use crate::app::EditorApp;
use crate::workspace::{self, THUMB_VIEW};

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    ui.label("Library");
    ui.separator();
    ui.space(6.0);

    if ui.button("asset_import", "Import Asset…").clicked() {
        app.import_png(ui);
    }
    ui.space(4.0);
    if ui.button("doc_new", "New Document").clicked() {
        app.cancel_transform();
        let idx = app.workspace.create_document(&format!(
            "Document {}",
            app.workspace.documents.len() + 1
        ));
        app.switch_document(idx);
    }
    ui.space(8.0);

    let mut switch_doc: Option<usize> = None;
    let mut delete_doc: Option<usize> = None;
    let mut place_asset: Option<usize> = None;
    let mut delete_asset: Option<usize> = None;
    let mut sel = app.workspace.tree_selected.clone();
    let thumb_size = THUMB_VIEW * ui.scale();

    ui.tree_scope(&mut sel, |ui| {
        ui.tree_node_icon_open("images_root", "folder", "images", true, |ui| {
            let asset_count = app.workspace.assets.len();
            let mut start_drag: Option<(usize, Vec2)> = None;
            for index in 0..asset_count {
                let id = format!("asset_{}", app.workspace.assets[index].id);
                let label = format!(
                    "{} ({}×{})",
                    app.workspace.assets[index].name,
                    app.workspace.assets[index].width,
                    app.workspace.assets[index].height
                );
                let thumb_pixels = app.workspace.assets[index].thumb.clone();
                ui.horizontal(|ui| {
                    let thumb = ui.area(&format!("asset_thumb_{index}"), Vec2::splat(thumb_size));
                    ui.fill_rect(thumb.rect, [0.12, 0.12, 0.14, 1.0]);
                    draw_thumb_in(ui, &thumb_pixels, thumb.rect);
                    // Grab thumbnail → drag onto canvas.
                    if thumb.hovered && app.input.mouse_pressed {
                        start_drag = Some((index, app.input.mouse_pos));
                    }

                    ui.vertical(|ui| {
                        let _resp = ui.tree_leaf_icon(&id, "image", &label);
                        if ui.button(&format!("asset_del_{index}"), "Del").clicked() {
                            delete_asset = Some(index);
                        }
                    });
                });
            }
            if let Some((index, origin)) = start_drag {
                app.workspace.begin_asset_drag(index, origin);
            }
        });

        ui.tree_node_icon_open("documents_root", "folder", "documents", true, |ui| {
            for (index, doc) in app.workspace.documents.iter().enumerate() {
                let id = format!("doc_{}", doc.id);
                let label = if index == app.workspace.active_document {
                    format!("• {}", doc.name)
                } else {
                    doc.name.clone()
                };
                ui.horizontal(|ui| {
                    let thumb = ui.area(&format!("doc_thumb_{index}"), Vec2::splat(thumb_size));
                    ui.fill_rect(thumb.rect, [0.12, 0.12, 0.14, 1.0]);
                    draw_thumb_in(ui, &doc.thumb, thumb.rect);
                    ui.vertical(|ui| {
                        let resp = ui.tree_leaf_icon(&id, "file", &label);
                        if resp.clicked() {
                            switch_doc = Some(index);
                        }
                        if app.workspace.documents.len() > 1
                            && ui.button(&format!("doc_del_{index}"), "Delete").clicked()
                        {
                            delete_doc = Some(index);
                        }
                    });
                });
            }
        });
    });
    app.workspace.tree_selected = sel;

    // Promote press → drag after threshold; ghost follows cursor.
    if let (Some(index), Some(origin)) = (
        app.workspace.asset_drag,
        app.workspace.asset_drag_origin,
    ) {
        if app.input.mouse_down {
            if (app.input.mouse_pos - origin).length() > 6.0 * ui.scale() {
                app.workspace.asset_dragging = true;
            }
            if app.workspace.asset_dragging {
                if let Some(asset) = app.workspace.assets.get(index) {
                    let ghost = mega_ui::Rect::from_min_size(
                        app.input.mouse_pos - Vec2::splat(thumb_size * 0.5),
                        Vec2::splat(thumb_size),
                    );
                    ui.fill_rect(ghost, [0.12, 0.12, 0.14, 0.85]);
                    draw_thumb_in(ui, &asset.thumb, ghost);
                }
                ui.request_repaint();
            }
        }
    }

    if app.input.mouse_released {
        if let Some(asset) = app.workspace.asset_drag {
            let dropped = app.workspace.asset_dragging
                && app.canvas_rect.contains(app.input.mouse_pos);
            app.workspace.clear_asset_drag();
            if dropped {
                place_asset = Some(asset);
            }
        }
    }

    if let Some(index) = switch_doc {
        app.switch_document(index);
    }
    if let Some(index) = delete_doc {
        app.cancel_transform();
        app.workspace.delete_document(index);
        app.clear_history();
        let _ = app.refresh_image();
        app.dirty = true;
    }
    if let Some(index) = place_asset {
        app.place_asset(index, ui);
    }
    if let Some(index) = delete_asset {
        app.workspace.delete_asset(index);
        app.dirty = true;
    }
}

fn draw_thumb_in(ui: &mut Ui, thumb: &[u8], rect: mega_ui::Rect) {
    const DRAW: u32 = 16;
    let cell_w = rect.width() / DRAW as f32;
    let cell_h = rect.height() / DRAW as f32;
    let step = (workspace::THUMB_SIZE / DRAW).max(1);
    for y in 0..DRAW {
        for x in 0..DRAW {
            let sx = x * step;
            let sy = y * step;
            let o = ((sy * workspace::THUMB_SIZE + sx) * 4) as usize;
            if o + 4 > thumb.len() {
                continue;
            }
            let a = thumb[o + 3] as f32 / 255.0;
            if a < 0.02 {
                continue;
            }
            let color = [
                thumb[o] as f32 / 255.0,
                thumb[o + 1] as f32 / 255.0,
                thumb[o + 2] as f32 / 255.0,
                a,
            ];
            let pr = mega_ui::Rect::from_min_size(
                rect.min + Vec2::new(x as f32 * cell_w, y as f32 * cell_h),
                Vec2::new(cell_w.ceil(), cell_h.ceil()),
            );
            ui.fill_rect(pr, color);
        }
    }
}
