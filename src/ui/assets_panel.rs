use glam::Vec2;
use mega_ui::{Rect, TextStyle, Ui};

use crate::host::{FrameInput, THUMB_ATLAS};
use crate::workspace::{Workspace, THUMB_VIEW};

#[derive(Default)]
pub struct LibraryAction {
    pub import: bool,
    pub new_document: bool,
    pub switch_doc: Option<usize>,
    pub delete_doc: Option<usize>,
    pub clone_doc: Option<usize>,
    pub place_asset: Option<usize>,
    pub delete_asset: Option<usize>,
    pub preview_asset: Option<usize>,
    pub open_video: Option<usize>,
    pub delete_video: Option<usize>,
}

pub fn draw(ui: &mut Ui, workspace: &mut Workspace, input: &FrameInput, canvas: Rect) -> LibraryAction {
    let mut action = LibraryAction::default();
    ui.label("Library");
    ui.separator();
    ui.space(6.0);

    ui.horizontal(|ui| {
        ui.flex(0.5, |ui| {
            if ui.button("asset_import", "Import Asset…").clicked() {
                action.import = true;
            }
        });
        ui.flex(1.0, |ui| {
            if ui.button("doc_new", "New Document").clicked() {
                action.new_document = true;
            }
        });
    });
    ui.space(8.0);

    let mut switch_doc: Option<usize> = None;
    let mut delete_doc: Option<usize> = None;
    let mut clone_doc: Option<usize> = None;
    let mut place_asset: Option<usize> = None;
    let mut delete_asset: Option<usize> = None;
    let mut preview_asset: Option<usize> = None;
    let mut sel = workspace.tree_selected.clone();
    let mut pick: Option<String> = None;
    let thumb_size = THUMB_VIEW * ui.scale();
    let meta_style = TextStyle { color: [0.45, 0.45, 0.48, 1.0], size: 12.0 };
    workspace.prepare_thumb_atlas();
    let asset_count = workspace.assets.len();

    ui.tree_scope(&mut sel, |ui| {
        ui.tree_node_icon_open("images_root", "folder", "images", true, |ui| {
            let mut start_drag: Option<(usize, Vec2)> = None;
            for index in 0..asset_count {
                let id = format!("asset_{}", workspace.assets[index].id);
                let name = workspace.assets[index].name.clone();
                let meta = format!(
                    "{}×{} · {}",
                    workspace.assets[index].width,
                    workspace.assets[index].height,
                    format_bytes(workspace.assets[index].byte_size),
                );
                let selected = workspace.tree_selected.as_deref() == Some(id.as_str());
                let mut thumb_release = false;
                let clicked = item_row(ui, &id, selected, &name, &meta, meta_style, 28.0, |ui| {
                    let thumb = ui.area(&format!("asset_thumb_{index}"), Vec2::splat(thumb_size));
                    draw_thumb(ui, workspace, index, thumb.rect);
                    if thumb.hovered && input.mouse_pressed {
                        start_drag = Some((index, input.mouse_pos));
                    }
                    // The thumb slot takes the press, so the row frame does not see that click.
                    if thumb.active && input.mouse_released {
                        thumb_release = true;
                    }
                }, |ui| {
                    if ui.icon_button(&format!("asset_del_{index}"), "delete", false) {
                        delete_asset = Some(index);
                    }
                });
                if clicked || thumb_release {
                    pick = Some(id);
                    if !workspace.asset_dragging {
                        preview_asset = Some(index);
                    }
                }
            }
            if let Some((index, origin)) = start_drag {
                workspace.begin_asset_drag(index, origin);
            }
        });

        ui.tree_node_icon_open("videos_root", "folder", "videos", true, |ui| {
            for (index, video) in workspace.videos.iter().enumerate() {
                let id = format!("video_{}", video.id);
                let selected = workspace.tree_selected.as_deref() == Some(id.as_str());
                let mut thumb_release = false;
                let clicked = item_row(ui, &id, selected, &video.name, &format_bytes(video.byte_size), meta_style, 28.0, |ui| {
                    let thumb = ui.area(&format!("video_thumb_{index}"), Vec2::splat(thumb_size));
                    draw_thumb(ui, workspace, asset_count + workspace.documents.len() + index, thumb.rect);
                    if video.thumb.is_empty() { ui.text_at(thumb.rect.min + Vec2::new(8.0, 28.0), "Video"); }
                    if thumb.active && input.mouse_released { thumb_release = true; }
                }, |ui| {
                    if ui.icon_button(&format!("video_delete_{index}"), "delete", false) { action.delete_video = Some(index); }
                });
                if clicked || thumb_release { pick = Some(id); action.open_video = Some(index); }
            }
        });

        ui.tree_node_icon_open("documents_root", "folder", "documents", true, |ui| {
            let doc_count = workspace.documents.len();
            let can_delete = doc_count > 1;
            for index in 0..doc_count {
                let id = format!("doc_{}", workspace.documents[index].id);
                let name = workspace.documents[index].name.clone();
                let project = &workspace.documents[index].project;
                let meta = format!(
                    "{}×{} · {}",
                    project.width,
                    project.height,
                    format_bytes(workspace.documents[index].memory_bytes()),
                );
                let selected = workspace.tree_selected.as_deref() == Some(id.as_str());
                let reserve = if can_delete { 56.0 } else { 28.0 };
                let mut thumb_release = false;
                let clicked = item_row(ui, &id, selected, &name, &meta, meta_style, reserve, |ui| {
                    let thumb = ui.area(&format!("doc_thumb_{index}"), Vec2::splat(thumb_size));
                    draw_thumb(ui, workspace, asset_count + index, thumb.rect);
                    if thumb.active && input.mouse_released {
                        thumb_release = true;
                    }
                }, |ui| {
                    if can_delete && ui.icon_button(&format!("doc_del_{index}"), "delete", false) {
                        delete_doc = Some(index);
                    }
                    if ui.icon_button(&format!("doc_copy_{index}"), "copy", false) {
                        clone_doc = Some(index);
                    }
                });
                if clicked || thumb_release {
                    pick = Some(id);
                    switch_doc = Some(index);
                }
            }
        });
    });
    if let Some(id) = pick {
        sel = Some(id);
    }
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
                if workspace.assets.get(index).is_some() {
                    let ghost = mega_ui::Rect::from_min_size(
                        input.mouse_pos - Vec2::splat(thumb_size * 0.5),
                        Vec2::splat(thumb_size),
                    );
                    draw_thumb(ui, workspace, index, ghost);
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
    action.clone_doc = clone_doc;
    action.place_asset = place_asset;
    action.delete_asset = delete_asset;
    action
}

fn item_row(
    ui: &mut Ui,
    id: &str,
    selected: bool,
    name: &str,
    meta: &str,
    meta_style: TextStyle,
    icon_reserve: f32,
    thumb: impl FnOnce(&mut Ui),
    icons: impl FnOnce(&mut Ui),
) -> bool {
    ui.selectable(id, selected, |ui| {
        ui.horizontal(|ui| {
            thumb(ui);
            ui.flex(1.0, |ui| {
                ui.horizontal(|ui| {
                    let name_w = (ui.available_size().x / ui.scale() - icon_reserve).max(8.0);
                    ui.label_fixed(name_w, name);
                    ui.spacer();
                    icons(ui);
                });
                ui.label_styled(meta, meta_style);
            });
        });
    }).clicked()
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let n = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if n < MB {
        format!("{:.1} KB", n / KB)
    } else {
        format!("{:.1} MB", n / MB)
    }
}

fn draw_thumb(ui: &mut Ui, workspace: &Workspace, cell: usize, rect: mega_ui::Rect) {
    ui.fill_rect(rect, [0.12, 0.12, 0.14, 1.0]);
    if workspace.thumb_atlas_size.0 == 0 {
        return;
    }
    let (uv_min, uv_max) = workspace.thumb_uv(cell);
    ui.texture_rect(THUMB_ATLAS, rect, uv_min, uv_max);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::THUMB_SIZE;

    #[test]
    fn opaque_thumbs_are_one_quad_each() {
        let mut workspace = Workspace::default();
        workspace.import_asset_rgba("solid".into(), &[255, 0, 0, 255].repeat(64), 8, 8).unwrap();
        let solid = vec![255u8; (THUMB_SIZE * THUMB_SIZE * 4) as usize];
        workspace.assets[0].thumb = solid.clone();
        workspace.documents[0].thumb = solid;
        workspace.prepare_thumb_atlas();

        let mut ui = Ui::new();
        ui.load_builtin_icons();
        ui.begin_frame(mega_ui::UiInput {
            viewport: Vec2::new(800.0, 1400.0),
            ..Default::default()
        });
        ui.horizontal(|ui| {
            ui.flex_width(420.0, |ui| {
                draw(ui, &mut workspace, &FrameInput::default(), Rect::from_min_size(Vec2::ZERO, Vec2::splat(200.0)));
            });
        });
        let output = ui.end_frame();
        let thumbs = output.draw_list.iter().filter(|cmd| cmd.kind == 1.0 && cmd.tex == THUMB_ATLAS).count();
        assert_eq!(thumbs, 2);
        assert!(output.draw_list.len() < 400, "draw list {}", output.draw_list.len());
        assert_eq!(workspace.thumb_atlas[0], 255);
        assert!(workspace.thumb_needs_upload());
    }
}
