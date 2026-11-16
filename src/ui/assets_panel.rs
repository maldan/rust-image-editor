use glam::Vec2;
use mega_ui::{Rect, TextStyle, Ui};

use crate::app::RenameTarget;
use crate::host::{FrameInput, THUMB_ATLAS};
use crate::workspace::{LibraryKind, THUMB_VIEW, Workspace};

#[derive(Clone, Copy, PartialEq, Eq)]
enum LibraryGroup {
    ImageAsset,
    VideoAsset,
    ImageDocument,
    AnimationDocument,
}

impl LibraryGroup {
    fn kind(self) -> LibraryKind {
        match self {
            Self::ImageAsset => LibraryKind::Image,
            Self::VideoAsset => LibraryKind::Video,
            Self::ImageDocument | Self::AnimationDocument => LibraryKind::Document,
        }
    }
}

pub struct LibraryDrag {
    group: LibraryGroup,
    id: String,
    origin: Vec2,
    active: bool,
}

#[derive(Default)]
pub struct LibraryAction {
    pub reorder: Option<(LibraryKind, String, usize)>,
    pub import: bool,
    pub import_clipboard: bool,
    pub new_document: bool,
    pub new_animation: bool,
    pub switch_doc: Option<usize>,
    pub delete_doc: Option<usize>,
    pub clone_doc: Option<usize>,
    pub place_asset: Option<usize>,
    pub delete_asset: Option<usize>,
    pub preview_asset: Option<usize>,
    pub open_video: Option<usize>,
    pub delete_video: Option<usize>,
    pub rename: Option<RenameTarget>,
}

pub fn draw(
    ui: &mut Ui,
    workspace: &mut Workspace,
    input: &FrameInput,
    canvas: Rect,
    drag: &mut Option<LibraryDrag>,
) -> LibraryAction {
    let mut action = LibraryAction::default();
    // Menu actions must not also click or drag the row underneath the popup.
    // Snapshot before drawing: choosing an item closes the menu during this frame.
    let row_input = !ui.context_menu_open() && !ui.pointer_over_popup_at(input.mouse_pos);
    if !row_input {
        *drag = None;
        workspace.clear_asset_drag();
    }
    if !ui.enabled() || input.key_escape || (!input.mouse_down && !input.mouse_released) {
        *drag = None;
    }
    if let Some(drag) = drag.as_mut() {
        if input.mouse_pos.distance(drag.origin) > 6.0 * ui.scale() {
            drag.active = true;
        }
    }
    let dragging = drag.as_ref().is_some_and(|drag| drag.active);
    ui.space(6.0);

    ui.horizontal(|ui| {
        ui.flex(0.5, |ui| {
            if ui.button("asset_import", "Import Asset…").clicked() {
                action.import = true;
            }
        });
        ui.flex(1.0, |ui| {
            if ui
                .button("asset_import_clipboard", "Import Clipboard")
                .clicked()
            {
                action.import_clipboard = true;
            }
        });
    });
    ui.horizontal(|ui| {
        ui.flex(0.5, |ui| {
            if ui.button("doc_new", "New Image").clicked() {
                action.new_document = true;
            }
        });
        ui.flex(1.0, |ui| {
            if ui.button("animation_new", "New Animation").clicked() {
                action.new_animation = true;
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
    let meta_style = TextStyle {
        color: [0.45, 0.45, 0.48, 1.0],
        size: 12.0,
    };
    workspace.prepare_thumb_atlas();
    let asset_count = workspace.assets.len();
    let video_count = workspace.videos.len();
    let document_count = workspace.documents.len();
    let animation_count = workspace
        .documents
        .iter()
        .filter(|doc| doc.project.animation.is_some())
        .count();

    ui.tree_scope(&mut sel, |ui| {
        ui.tree_node_icon_open(
            "assets_root",
            "folder",
            &format!("Assets ({})", asset_count + video_count),
            true,
            |ui| {
                ui.tree_node_icon_open(
                    "images_root",
                    "folder",
                    &format!("Images ({asset_count})"),
                    true,
                    |ui| {
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
                            let clicked =
                                item_row(ui, &id, selected, &name, &meta, meta_style, |ui| {
                                    let thumb = ui.area(
                                        &format!("asset_thumb_{index}"),
                                        Vec2::splat(thumb_size),
                                    );
                                    draw_thumb(ui, workspace, index, thumb.rect);
                                    if row_input && thumb.hovered && input.mouse_pressed {
                                        start_drag = Some((index, input.mouse_pos));
                                    }
                                    // The thumb slot takes the press, so the row frame does not see that click.
                                    if row_input && thumb.active && input.mouse_released {
                                        thumb_release = true;
                                    }
                                    thumb.rect
                                });
                            ui.context_menu(
                                &format!("asset_menu_{index}"),
                                clicked.response.hovered,
                                |ui| {
                                    if ui.menu_item("edit", "Edit").clicked() {
                                        action.rename = Some(RenameTarget::Image(
                                            workspace.assets[index].id.clone(),
                                        ));
                                    }
                                    if ui.menu_item("delete", "Delete").clicked() {
                                        delete_asset = Some(index);
                                    }
                                },
                            );
                            if row_input {
                                reorder_row(
                                    ui,
                                    &clicked,
                                    LibraryGroup::ImageAsset,
                                    &workspace.assets[index].id,
                                    index,
                                    input,
                                    drag,
                                    &mut action,
                                );
                            }
                            if row_input
                                && (clicked.response.clicked() || thumb_release)
                                && !dragging
                            {
                                pick = Some(id);
                                if !workspace.asset_dragging {
                                    preview_asset = Some(index);
                                }
                            }
                        }
                        if let Some((index, origin)) = start_drag {
                            workspace.begin_asset_drag(index, origin);
                        }
                    },
                );

                ui.tree_node_icon_open(
                    "videos_root",
                    "folder",
                    &format!("Videos ({video_count})"),
                    true,
                    |ui| {
                        for (index, video) in workspace.videos.iter().enumerate() {
                            let id = format!("video_{}", video.id);
                            let selected = workspace.tree_selected.as_deref() == Some(id.as_str());
                            let mut thumb_release = false;
                            let clicked = item_row(
                                ui,
                                &id,
                                selected,
                                &video.name,
                                &format_bytes(video.byte_size),
                                meta_style,
                                |ui| {
                                    let thumb = ui.area(
                                        &format!("video_thumb_{index}"),
                                        Vec2::splat(thumb_size),
                                    );
                                    draw_thumb(
                                        ui,
                                        workspace,
                                        asset_count + workspace.documents.len() + index,
                                        thumb.rect,
                                    );
                                    if video.thumb.is_empty() {
                                        ui.text_at(thumb.rect.min + Vec2::new(8.0, 28.0), "Video");
                                    }
                                    if row_input && thumb.active && input.mouse_released {
                                        thumb_release = true;
                                    }
                                    thumb.rect
                                },
                            );
                            ui.context_menu(
                                &format!("video_menu_{index}"),
                                clicked.response.hovered,
                                |ui| {
                                    if ui.menu_item("edit", "Edit").clicked() {
                                        action.rename = Some(RenameTarget::Video(video.id.clone()));
                                    }
                                    if ui.menu_item("delete", "Delete").clicked() {
                                        action.delete_video = Some(index);
                                    }
                                },
                            );
                            if row_input {
                                reorder_row(
                                    ui,
                                    &clicked,
                                    LibraryGroup::VideoAsset,
                                    &video.id,
                                    index,
                                    input,
                                    drag,
                                    &mut action,
                                );
                            }
                            if row_input
                                && (clicked.response.clicked() || thumb_release)
                                && !dragging
                            {
                                pick = Some(id);
                                action.open_video = Some(index);
                            }
                        }
                    },
                );
            },
        );
        ui.tree_node_icon_open(
            "documents_root",
            "folder",
            &format!("Documents ({document_count})"),
            true,
            |ui| {
                for (group_id, label, group) in [
                    (
                        "document_images_root",
                        "Images",
                        LibraryGroup::ImageDocument,
                    ),
                    (
                        "document_animations_root",
                        "Animations",
                        LibraryGroup::AnimationDocument,
                    ),
                ] {
                    let count = if group == LibraryGroup::AnimationDocument {
                        animation_count
                    } else {
                        document_count - animation_count
                    };
                    ui.tree_node_icon_open(
                        group_id,
                        "folder",
                        &format!("{label} ({count})"),
                        true,
                        |ui| {
                            let doc_count = workspace.documents.len();
                            let can_delete = doc_count > 1;
                            for index in 0..doc_count {
                                if workspace.documents[index].project.animation.is_some()
                                    != (group == LibraryGroup::AnimationDocument)
                                {
                                    continue;
                                }
                                let id = format!("doc_{}", workspace.documents[index].id);
                                let name = workspace.documents[index].name.clone();
                                let project = &workspace.documents[index].project;
                                let meta = format!(
                                    "{} · {}×{} · {}",
                                    if project.animation.is_some() {
                                        "Animation"
                                    } else {
                                        "Image"
                                    },
                                    project.width,
                                    project.height,
                                    format_bytes(workspace.documents[index].memory_bytes()),
                                );
                                let selected =
                                    workspace.tree_selected.as_deref() == Some(id.as_str());
                                let mut thumb_release = false;
                                let clicked =
                                    item_row(ui, &id, selected, &name, &meta, meta_style, |ui| {
                                        let thumb = ui.area(
                                            &format!("doc_thumb_{index}"),
                                            Vec2::splat(thumb_size),
                                        );
                                        draw_thumb(ui, workspace, asset_count + index, thumb.rect);
                                        if row_input && thumb.active && input.mouse_released {
                                            thumb_release = true;
                                        }
                                        thumb.rect
                                    });
                                ui.context_menu(
                                    &format!("doc_menu_{index}"),
                                    clicked.response.hovered,
                                    |ui| {
                                        if ui.menu_item("duplicate", "Duplicate").clicked() {
                                            clone_doc = Some(index);
                                        }
                                        if ui.menu_item("edit", "Edit").clicked() {
                                            action.rename = Some(RenameTarget::Document(
                                                workspace.documents[index].id.clone(),
                                            ));
                                        }
                                        if ui
                                            .menu_item_enabled("delete", "Delete", can_delete)
                                            .clicked()
                                        {
                                            delete_doc = Some(index);
                                        }
                                    },
                                );
                                if row_input {
                                    reorder_row(
                                        ui,
                                        &clicked,
                                        group,
                                        &workspace.documents[index].id,
                                        index,
                                        input,
                                        drag,
                                        &mut action,
                                    );
                                }
                                if row_input
                                    && (clicked.response.clicked() || thumb_release)
                                    && !dragging
                                {
                                    pick = Some(id);
                                    switch_doc = Some(index);
                                }
                            }
                        },
                    );
                }
                ui.tree_node_icon_open(
                    "document_videos_root",
                    "folder",
                    "Videos (0)",
                    true,
                    |_| {},
                );
            },
        );
    });
    if let Some(id) = pick {
        sel = Some(id);
    }
    workspace.tree_selected = sel;

    // Promote press → drag after threshold; ghost follows cursor.
    if let (Some(index), Some(origin)) = (workspace.asset_drag, workspace.asset_drag_origin) {
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
                && workspace
                    .asset_drag_origin
                    .is_some_and(|origin| input.mouse_pos.distance(origin) < 6.0 * ui.scale());
            let dropped = workspace.asset_dragging && canvas.contains(input.mouse_pos);
            workspace.clear_asset_drag();
            if dropped {
                place_asset = Some(asset);
            } else if clicked {
                preview_asset = Some(asset);
            }
        }
    }

    if input.mouse_released {
        *drag = None;
    }
    action.switch_doc = switch_doc;
    action.preview_asset = preview_asset;
    action.delete_doc = delete_doc;
    action.clone_doc = clone_doc;
    action.place_asset = place_asset;
    action.delete_asset = delete_asset;
    action
}

struct LibraryRow {
    response: mega_ui::Response,
    rect: Rect,
    body: Rect,
}

fn reorder_row(
    ui: &mut Ui,
    row: &LibraryRow,
    group: LibraryGroup,
    id: &str,
    index: usize,
    input: &FrameInput,
    drag: &mut Option<LibraryDrag>,
    action: &mut LibraryAction,
) {
    if row.response.hovered
        && row.body.contains(input.mouse_pos)
        && input.mouse_pressed
        && ui.enabled()
    {
        *drag = Some(LibraryDrag {
            group,
            id: id.into(),
            origin: input.mouse_pos,
            active: false,
        });
    }
    let Some(current) = drag
        .as_ref()
        .filter(|drag| drag.active && drag.group == group)
    else {
        return;
    };
    ui.set_mouse_cursor(mega_ui::CursorIcon::Move);
    ui.request_repaint();
    if row.response.hovered {
        let after = input.mouse_pos.y >= (row.rect.min.y + row.rect.max.y) * 0.5;
        let y = if after {
            row.rect.max.y
        } else {
            row.rect.min.y
        };
        ui.line(
            Vec2::new(row.rect.min.x, y),
            Vec2::new(row.rect.max.x, y),
            2.0 * ui.scale(),
            [0.3, 0.6, 1.0, 1.0],
        );
        if input.mouse_released {
            action.reorder = Some((group.kind(), current.id.clone(), index + usize::from(after)));
        }
    }
}

fn item_row(
    ui: &mut Ui,
    id: &str,
    selected: bool,
    name: &str,
    meta: &str,
    meta_style: TextStyle,
    thumb: impl FnOnce(&mut Ui) -> Rect,
) -> LibraryRow {
    let width = ui.available_size().x;
    let scale = ui.scale();
    let mut rect = Rect::from_min_size(Vec2::ZERO, Vec2::ZERO);
    let response = ui.selectable(id, selected, |ui| {
        ui.horizontal(|ui| {
            let thumb = thumb(ui);
            rect = Rect::from_min_size(
                thumb.min - Vec2::splat(10.0 * scale),
                Vec2::new(width, thumb.height() + 20.0 * scale),
            );
            ui.flex(1.0, |ui| {
                let name_w = (ui.available_size().x / ui.scale()).max(8.0);
                ui.label_fixed(name_w, name);
                ui.label_styled(meta, meta_style);
            });
        });
    });
    let body = rect;
    LibraryRow {
        response,
        rect,
        body,
    }
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

    fn library_frame(
        ui: &mut Ui,
        workspace: &mut Workspace,
        input: &FrameInput,
        drag: &mut Option<LibraryDrag>,
    ) -> (LibraryAction, mega_ui::UiOutput) {
        ui.begin_frame(mega_ui::UiInput {
            viewport: Vec2::new(800.0, 1800.0),
            mouse_pos: input.mouse_pos,
            mouse_pressed: input.mouse_pressed,
            mouse_down: input.mouse_down,
            mouse_released: input.mouse_released,
            mouse_right_pressed: input.mouse_right_pressed,
            mouse_right_down: input.mouse_right_down,
            mouse_right_released: input.mouse_right_released,
            ..Default::default()
        });
        let mut action = LibraryAction::default();
        ui.horizontal(|ui| {
            ui.flex_width(420.0, |ui| {
                action = draw(
                    ui,
                    workspace,
                    input,
                    Rect::from_min_size(Vec2::new(500.0, 0.0), Vec2::new(300.0, 1600.0)),
                    drag,
                );
            })
        });
        (action, ui.end_frame())
    }

    #[test]
    fn dragging_library_rows_reorders_without_opening_or_switching_items() {
        for kind in [
            LibraryKind::Image,
            LibraryKind::Video,
            LibraryKind::Document,
        ] {
            let mut workspace = Workspace::default();
            for i in 0..3 {
                workspace
                    .import_asset_rgba(format!("Image {i}"), &[1, 2, 3, 255], 1, 1)
                    .unwrap();
            }
            for _ in 0..2 {
                workspace
                    .add_video(include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec())
                    .unwrap();
            }
            workspace.create_document("Second");
            workspace.create_document("Third");
            let active = workspace.active_doc().id.clone();
            let (range, id) = match kind {
                LibraryKind::Image => (0..3, workspace.assets[2].id.clone()),
                LibraryKind::Video => (3..5, workspace.videos[1].id.clone()),
                LibraryKind::Document => (5..8, workspace.documents[2].id.clone()),
            };
            let mut ui = Ui::new();
            ui.load_builtin_icons();
            let mut drag = None;
            let mut output = mega_ui::UiOutput::default();
            for _ in 0..4 {
                output =
                    library_frame(&mut ui, &mut workspace, &FrameInput::default(), &mut drag).1;
            }
            let thumbs: Vec<_> = output
                .draw_list
                .iter()
                .filter(|c| c.kind == 1.0 && c.tex == THUMB_ATLAS)
                .map(|c| c.rect)
                .collect();
            assert_eq!(thumbs.len(), 8);
            let source = thumbs[range.end - 1];
            let target = thumbs[range.start];
            let mut input = FrameInput::default();
            input.mouse_pos = (source.min + source.max) * 0.5;
            input.mouse_pressed = true;
            input.mouse_down = true;
            library_frame(&mut ui, &mut workspace, &input, &mut drag);
            input.mouse_pressed = false;
            input.mouse_pos = target.min + Vec2::splat(8.0);
            library_frame(&mut ui, &mut workspace, &input, &mut drag);
            input.mouse_down = false;
            input.mouse_released = true;
            let action = library_frame(&mut ui, &mut workspace, &input, &mut drag).0;
            assert_eq!(action.reorder, Some((kind, id.clone(), 0)));
            assert!(
                action.switch_doc.is_none()
                    && action.open_video.is_none()
                    && action.preview_asset.is_none()
                    && action.place_asset.is_none()
            );
            assert!(drag.is_none());
            let mut app = crate::app::EditorApp::default();
            app.workspace = workspace;
            crate::ui::apply_library(&mut ui, &mut app, action);
            assert!(app.dirty);
            assert_eq!(app.workspace.active_doc().id, active);
            let first = match kind {
                LibraryKind::Image => &app.workspace.assets[0].id,
                LibraryKind::Video => &app.workspace.videos[0].id,
                LibraryKind::Document => &app.workspace.documents[0].id,
            };
            assert_eq!(first, &id);
        }
    }

    #[test]
    fn mixed_documents_group_without_reordering_data_and_reject_cross_group_drops() {
        for animation in [false, true] {
            let mut workspace = Workspace::default();
            for name in ["Animation A", "Image B", "Animation B"] {
                workspace.create_document(name);
            }
            for index in [1, 3] {
                workspace.documents[index].project.animation =
                    Some(crate::project::Animation::new(12, 24));
            }
            let ids: Vec<_> = workspace
                .documents
                .iter()
                .map(|doc| doc.id.clone())
                .collect();
            let active = workspace.active_doc().id.clone();
            let selection = workspace.tree_selected.clone();
            let mut ui = Ui::new();
            ui.load_builtin_icons();
            let mut drag = None;
            let mut output = mega_ui::UiOutput::default();
            for _ in 0..4 {
                output =
                    library_frame(&mut ui, &mut workspace, &FrameInput::default(), &mut drag).1;
            }
            let thumbs: Vec<_> = output
                .draw_list
                .iter()
                .filter(|c| c.kind == 1.0 && c.tex == THUMB_ATLAS)
                .collect();
            assert_eq!(thumbs.len(), 4);
            // Images precede animations in the tree; persisted indices remain unchanged.
            for (thumb, index) in thumbs.iter().zip([0, 2, 1, 3]) {
                let (min, max) = workspace.thumb_uv(index);
                assert_eq!((thumb.uv_min, thumb.uv_max), (min, max));
            }
            assert_eq!(
                workspace
                    .documents
                    .iter()
                    .map(|doc| doc.id.clone())
                    .collect::<Vec<_>>(),
                ids
            );
            let (source, target, foreign, source_index, target_index) = if animation {
                (thumbs[3].rect, thumbs[2].rect, thumbs[0].rect, 3, 1)
            } else {
                (thumbs[1].rect, thumbs[0].rect, thumbs[2].rect, 2, 0)
            };
            for (destination, allowed) in [(foreign, false), (target, true)] {
                let mut input = FrameInput::default();
                input.mouse_pos = (source.min + source.max) * 0.5;
                input.mouse_pressed = true;
                input.mouse_down = true;
                library_frame(&mut ui, &mut workspace, &input, &mut drag);
                input.mouse_pressed = false;
                input.mouse_pos = destination.min + Vec2::splat(8.0);
                library_frame(&mut ui, &mut workspace, &input, &mut drag);
                input.mouse_down = false;
                input.mouse_released = true;
                let action = library_frame(&mut ui, &mut workspace, &input, &mut drag).0;
                assert!(action.switch_doc.is_none());
                assert!(drag.is_none());
                if allowed {
                    assert_eq!(
                        action.reorder,
                        Some((
                            LibraryKind::Document,
                            ids[source_index].clone(),
                            target_index
                        ))
                    );
                    assert!(workspace.reorder_library(
                        LibraryKind::Document,
                        &ids[source_index],
                        target_index
                    ));
                    let group_ids: Vec<_> = workspace
                        .documents
                        .iter()
                        .filter(|doc| doc.project.animation.is_some() == animation)
                        .map(|doc| doc.id.clone())
                        .collect();
                    assert_eq!(
                        group_ids,
                        vec![ids[source_index].clone(), ids[target_index].clone()]
                    );
                } else {
                    assert!(action.reorder.is_none());
                }
                assert_eq!(workspace.active_doc().id, active);
                assert_eq!(workspace.tree_selected, selection);
            }
        }
    }

    #[test]
    fn reorder_accepts_only_rows_in_the_source_group() {
        let groups = [
            LibraryGroup::ImageAsset,
            LibraryGroup::VideoAsset,
            LibraryGroup::ImageDocument,
            LibraryGroup::AnimationDocument,
        ];
        for source in groups {
            for target in groups {
                let mut ui = Ui::new();
                ui.begin_frame(mega_ui::UiInput::default());
                let rect = Rect::from_min_size(Vec2::ZERO, Vec2::splat(100.0));
                let row = LibraryRow {
                    response: mega_ui::Response {
                        hovered: true,
                        ..Default::default()
                    },
                    rect,
                    body: rect,
                };
                let mut drag = Some(LibraryDrag {
                    group: source,
                    id: "source".into(),
                    origin: Vec2::ZERO,
                    active: true,
                });
                let mut action = LibraryAction::default();
                let mut input = FrameInput::default();
                input.mouse_pos = Vec2::splat(10.0);
                input.mouse_released = true;
                reorder_row(
                    &mut ui,
                    &row,
                    target,
                    "target",
                    2,
                    &input,
                    &mut drag,
                    &mut action,
                );
                assert_eq!(action.reorder.is_some(), source == target);
                ui.end_frame();
            }
        }
    }

    #[test]
    fn image_thumbnail_still_drags_onto_canvas() {
        let mut workspace = Workspace::default();
        workspace
            .import_asset_rgba("Image".into(), &[1, 2, 3, 255], 1, 1)
            .unwrap();
        let mut ui = Ui::new();
        ui.load_builtin_icons();
        let mut drag = None;
        let mut output = mega_ui::UiOutput::default();
        for _ in 0..4 {
            output = library_frame(&mut ui, &mut workspace, &FrameInput::default(), &mut drag).1;
        }
        let source = output
            .draw_list
            .iter()
            .find(|c| c.kind == 1.0 && c.tex == THUMB_ATLAS)
            .unwrap()
            .rect;
        let mut input = FrameInput::default();
        input.mouse_pos = (source.min + source.max) * 0.5;
        input.mouse_pressed = true;
        input.mouse_down = true;
        library_frame(&mut ui, &mut workspace, &input, &mut drag);
        input.mouse_pressed = false;
        input.mouse_pos = Vec2::new(600.0, 400.0);
        library_frame(&mut ui, &mut workspace, &input, &mut drag);
        input.mouse_down = false;
        input.mouse_released = true;
        let action = library_frame(&mut ui, &mut workspace, &input, &mut drag).0;
        assert_eq!(action.place_asset, Some(0));
        assert!(action.reorder.is_none() && action.preview_asset.is_none());
    }

    #[test]
    fn library_context_actions_target_their_row_without_switching_or_previewing() {
        for kind in 0..4 {
            for item in 0..if kind >= 2 { 3 } else { 2 } {
                let mut workspace = Workspace::default();
                workspace
                    .import_asset_rgba("Image".into(), &[1, 2, 3, 255], 1, 1)
                    .unwrap();
                workspace
                    .add_video(include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec())
                    .unwrap();
                workspace.create_document("Animation");
                workspace.documents[1].project.animation =
                    Some(crate::project::Animation::new(12, 24));
                let targets = [
                    RenameTarget::Image(workspace.assets[0].id.clone()),
                    RenameTarget::Video(workspace.videos[0].id.clone()),
                    RenameTarget::Document(workspace.documents[0].id.clone()),
                    RenameTarget::Document(workspace.documents[1].id.clone()),
                ];
                let selection = workspace.tree_selected.clone();
                let active = workspace.active_document;
                let mut ui = Ui::new();
                ui.load_builtin_icons();
                ui.begin_frame(mega_ui::UiInput::default());
                let mut icons = Vec::new();
                for icon in ["copy", "edit", "delete"] {
                    ui.icon(icon, 16.0);
                }
                let reference = ui.end_frame();
                icons.extend(
                    reference
                        .draw_list
                        .iter()
                        .filter(|c| {
                            (c.rect.width() - 16.0).abs() < 0.1
                                && (c.rect.height() - 16.0).abs() < 0.1
                        })
                        .copied(),
                );
                let mut drag = None;
                let mut output = mega_ui::UiOutput::default();
                for _ in 0..4 {
                    output =
                        library_frame(&mut ui, &mut workspace, &FrameInput::default(), &mut drag).1;
                }
                for icon in &icons {
                    assert!(!output.draw_list.iter().any(|c| c.tex == icon.tex
                        && c.kind == icon.kind
                        && c.uv_min == icon.uv_min
                        && c.uv_max == icon.uv_max
                        && (c.rect.width() - 16.0).abs() < 0.1));
                }
                let thumb = output
                    .draw_list
                    .iter()
                    .filter(|c| c.kind == 1.0 && c.tex == THUMB_ATLAS)
                    .nth(kind)
                    .unwrap()
                    .rect;
                let mut input = FrameInput::default();
                // The context menu must work on the thumbnail as well as the text.
                input.mouse_pos = (thumb.min + thumb.max) * 0.5;
                input.mouse_right_pressed = true;
                input.mouse_right_down = true;
                let (opening, _) = library_frame(&mut ui, &mut workspace, &input, &mut drag);
                assert!(
                    opening.switch_doc.is_none()
                        && opening.preview_asset.is_none()
                        && opening.open_video.is_none()
                );
                input.mouse_right_pressed = false;
                input.mouse_right_down = false;
                input.mouse_right_released = true;
                output = library_frame(&mut ui, &mut workspace, &input, &mut drag).1;
                assert!(ui.context_menu_open());
                assert!(drag.is_none());
                assert!(workspace.asset_drag.is_none());
                let popup = output
                    .draw_list
                    .iter()
                    .find(|c| c.colors == [ui.theme().popup.bg; 4])
                    .unwrap()
                    .rect;
                input.mouse_right_released = false;
                input.mouse_pos = Vec2::new(
                    popup.min.x + 30.0,
                    popup.min.y - 1.0 + 4.0 + ui.theme().metrics.menu_item_h * (item as f32 + 0.5),
                );
                input.mouse_pressed = true;
                input.mouse_down = true;
                library_frame(&mut ui, &mut workspace, &input, &mut drag);
                input.mouse_pressed = false;
                input.mouse_down = false;
                input.mouse_released = true;
                let action = library_frame(&mut ui, &mut workspace, &input, &mut drag).0;
                assert!(
                    action.switch_doc.is_none()
                        && action.preview_asset.is_none()
                        && action.open_video.is_none()
                        && action.place_asset.is_none()
                        && action.reorder.is_none(),
                    "kind={kind} item={item}: switch={:?} preview={:?} video={:?} place={:?} reorder={:?}",
                    action.switch_doc,
                    action.preview_asset,
                    action.open_video,
                    action.place_asset,
                    action.reorder
                );
                if kind >= 2 && item == 0 {
                    assert_eq!(action.clone_doc, Some(kind - 2));
                } else if item == usize::from(kind >= 2) {
                    assert_eq!(action.rename, Some(targets[kind].clone()));
                } else {
                    match kind {
                        0 => assert_eq!(action.delete_asset, Some(0)),
                        1 => assert_eq!(action.delete_video, Some(0)),
                        _ => assert_eq!(action.delete_doc, Some(kind - 2)),
                    }
                }
                assert_eq!(workspace.tree_selected, selection);
                assert_eq!(workspace.active_document, active);
            }
        }
    }

    #[test]
    fn opaque_thumbs_are_one_quad_each() {
        let mut workspace = Workspace::default();
        workspace
            .import_asset_rgba("solid".into(), &[255, 0, 0, 255].repeat(64), 8, 8)
            .unwrap();
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
                draw(
                    ui,
                    &mut workspace,
                    &FrameInput::default(),
                    Rect::from_min_size(Vec2::ZERO, Vec2::splat(200.0)),
                    &mut None,
                );
            });
        });
        let output = ui.end_frame();
        let thumbs = output
            .draw_list
            .iter()
            .filter(|cmd| cmd.kind == 1.0 && cmd.tex == THUMB_ATLAS)
            .count();
        assert_eq!(thumbs, 2);
        assert!(
            output.draw_list.len() < 400,
            "draw list {}",
            output.draw_list.len()
        );
        assert_eq!(workspace.thumb_atlas[0], 255);
        assert!(workspace.thumb_needs_upload());
    }
}
