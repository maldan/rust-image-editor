use glam::Vec2;
use mega_ui::{Rect, Ui};

use crate::host::FrameInput;
use crate::workspace::{self, Workspace, THUMB_VIEW};

#[derive(Default)]
pub struct LibraryAction {
    pub import: bool,
    pub new_document: bool,
    pub switch_doc: Option<usize>,
    pub delete_doc: Option<usize>,
    pub place_asset: Option<usize>,
    pub delete_asset: Option<usize>,
    pub preview_asset: Option<usize>,
}

pub fn draw(ui: &mut Ui, workspace: &mut Workspace, input: &FrameInput, canvas: Rect) -> LibraryAction {
    let mut action = LibraryAction::default();
    ui.label("Library");
    ui.separator();
    ui.space(6.0);

    if ui.button("asset_import", "Import Asset…").clicked() {
        action.import = true;
    }
    ui.space(4.0);
    if ui.button("doc_new", "New Document").clicked() {
        action.new_document = true;
    }
    ui.space(8.0);

    let mut switch_doc: Option<usize> = None;
    let mut delete_doc: Option<usize> = None;
    let mut place_asset: Option<usize> = None;
    let mut delete_asset: Option<usize> = None;
    let mut preview_asset: Option<usize> = None;
    let mut sel = workspace.tree_selected.clone();
    let thumb_size = THUMB_VIEW * ui.scale();

    ui.tree_scope(&mut sel, |ui| {
        ui.tree_node_icon_open("images_root", "folder", "images", true, |ui| {
            let asset_count = workspace.assets.len();
            let mut start_drag: Option<(usize, Vec2)> = None;
            for index in 0..asset_count {
                let id = format!("asset_{}", workspace.assets[index].id);
                let label = format!(
                    "{} ({}×{})",
                    workspace.assets[index].name,
                    workspace.assets[index].width,
                    workspace.assets[index].height
                );
                let thumb_pixels = workspace.assets[index].thumb.clone();
                ui.horizontal(|ui| {
                    let thumb = ui.area(&format!("asset_thumb_{index}"), Vec2::splat(thumb_size));
                    ui.fill_rect(thumb.rect, [0.12, 0.12, 0.14, 1.0]);
                    draw_thumb_in(ui, &thumb_pixels, thumb.rect);
                    // Grab thumbnail → drag onto canvas.
                    if thumb.hovered && input.mouse_pressed {
                        start_drag = Some((index, input.mouse_pos));
                    }

                    ui.flex(1.0, |ui| {
                        if ui.tree_leaf_icon(&id, "image", &label).clicked() {
                            preview_asset = Some(index);
                        }
                        if ui.icon_button(&format!("asset_del_{index}"), "delete", false) {
                            delete_asset = Some(index);
                        }
                    });
                });
            }
            if let Some((index, origin)) = start_drag {
                workspace.begin_asset_drag(index, origin);
            }
        });

        ui.tree_node_icon_open("documents_root", "folder", "documents", true, |ui| {
            for (index, doc) in workspace.documents.iter().enumerate() {
                let id = format!("doc_{}", doc.id);
                let label = if index == workspace.active_document {
                    format!("• {}", doc.name)
                } else {
                    doc.name.clone()
                };
                ui.horizontal(|ui| {
                    let thumb = ui.area(&format!("doc_thumb_{index}"), Vec2::splat(thumb_size));
                    ui.fill_rect(thumb.rect, [0.12, 0.12, 0.14, 1.0]);
                    draw_thumb_in(ui, &doc.thumb, thumb.rect);
                    if thumb.hovered && input.mouse_released { switch_doc = Some(index); }
                    ui.flex(1.0, |ui| {
                        let resp = ui.tree_leaf_icon(&id, "file", &label);
                        if resp.clicked() {
                            switch_doc = Some(index);
                        }
                        if workspace.documents.len() > 1
                            && ui.icon_button(&format!("doc_del_{index}"), "delete", false)
                        {
                            delete_doc = Some(index);
                        }
                    });
                });
            }
        });
    });
    workspace.tree_selected = sel;

    // Promote press → drag after threshold; ghost follows cursor.
    if let (Some(index), Some(origin)) = (
        workspace.asset_drag,
        workspace.asset_drag_origin,
    ) {
        if input.mouse_down {
            if (input.mouse_pos - origin).length() > 6.0 * ui.scale() {
                workspace.asset_dragging = true;
            }
            if workspace.asset_dragging {
                if let Some(asset) = workspace.assets.get(index) {
                    let ghost = mega_ui::Rect::from_min_size(
                        input.mouse_pos - Vec2::splat(thumb_size * 0.5),
                        Vec2::splat(thumb_size),
                    );
                    ui.fill_rect(ghost, [0.12, 0.12, 0.14, 0.85]);
                    draw_thumb_in(ui, &asset.thumb, ghost);
                }
                ui.request_repaint();
            }
        }
    }

    if input.mouse_released {
        if let Some(asset) = workspace.asset_drag {
            let clicked = !workspace.asset_dragging
                && workspace.asset_drag_origin.is_some_and(|origin| input.mouse_pos.distance(origin) < 6.0 * ui.scale());
            let dropped = workspace.asset_dragging
                && canvas.contains(input.mouse_pos);
            workspace.clear_asset_drag();
            if dropped {
                place_asset = Some(asset);
            } else if clicked {
                preview_asset = Some(asset);
            }
        }
    }

    action.switch_doc = switch_doc;
    action.preview_asset = preview_asset;
    action.delete_doc = delete_doc;
    action.place_asset = place_asset;
    action.delete_asset = delete_asset;
    action
}

fn draw_thumb_in(ui: &mut Ui, thumb: &[u8], rect: mega_ui::Rect) {
    let n = workspace::THUMB_SIZE;
    let cell_w = rect.width() / n as f32;
    let cell_h = rect.height() / n as f32;
    for y in 0..n {
        for x in 0..n {
            let o = ((y * n + x) * 4) as usize;
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
