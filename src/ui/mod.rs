//! Editor chrome: menus, dock tabs, status bar.

pub mod ai_dialog;
mod prompt_presets;
mod preview_select;
mod reference_picker;
pub mod ai_settings;
pub mod editor_settings;
mod animation_dialog;
mod asset_properties;
mod assets_panel;
mod canvas;
pub(crate) mod layer_grid;
mod nodes;
mod canvas_panel;
mod camera_panel;
pub mod hsv_dialog;
mod menu_bar;
mod new_document;
mod project_panel;
mod scene_3d;
pub mod rename_dialog;
pub mod delete_dialog;
mod unsaved_dialog;
mod tool_options;
pub(crate) mod value_slider;
mod tools_panel;
pub(crate) use tools_panel::load_icons as load_tool_icons;
mod timeline;
mod video_dialog;
mod video_timeline;
pub(crate) mod clip_effects;

use glam::Vec2;
use mega_ui::{DockNode, DockState, Ui};

use crate::app::EditorApp;
use assets_panel::LibraryAction;
pub(crate) use assets_panel::LibraryDrag;

pub fn default_dock() -> DockState {
    DockState::new(DockNode::split_h(
        0.18,
        DockNode::split_v(
            0.28,
            DockNode::leaf(&["Tools"]),
            DockNode::split_v(
                0.42,
                DockNode::leaf(&["Tool Options"]),
                DockNode::leaf(&["Library"]),
            ),
        ),
        DockNode::split_h(
            0.76,
            DockNode::leaf(&["Workspace"]),
            DockNode::leaf(&["Layers", "Canvas"]),
        ),
    ))
}

pub(crate) fn default_scene_dock() -> DockState {
    DockState::new(DockNode::split_h(
        0.18,
        DockNode::split_v(0.28, DockNode::leaf(&["Scene Tools"]), DockNode::leaf(&["Library"])),
        DockNode::split_h(0.70, DockNode::leaf(&["Workspace"]), DockNode::leaf(&["Objects", "Camera"])),
    ))
}

fn context_panel(node: &mut DockNode, preview: bool) {
    match node {
        DockNode::Leaf { tabs, .. } => {
            for tab in tabs {
                let label = match tab.as_str() {
                    "Layers" | "Properties" if preview => "Properties",
                    "Layers" | "Properties" => "Layers",
                    _ => continue,
                };
                if *tab != label {
                    *tab = label.into();
                }
            }
        }
        DockNode::Split { first, second, .. } => {
            context_panel(first, preview);
            context_panel(second, preview);
        }
    }
}

fn camera_dock(node: &mut DockNode, enabled: bool) {
    fn contains(node: &DockNode) -> bool {
        match node {
            DockNode::Leaf { tabs, .. } => tabs.iter().any(|t| t == "Camera"),
            DockNode::Split { first, second, .. } => contains(first) || contains(second),
        }
    }
    fn update(node: &mut DockNode, enabled: bool, add: &mut bool) {
        match node {
            DockNode::Leaf { tabs, active } => {
                if !enabled {
                    tabs.retain(|t| t != "Camera");
                    *active = (*active).min(tabs.len().saturating_sub(1));
                } else if *add && tabs.iter().any(|t| t == "Layers") {
                    tabs.push("Camera".into());
                    *add = false;
                }
            }
            DockNode::Split { first, second, .. } => {
                update(first, enabled, add);
                update(second, enabled, add);
            }
        }
    }
    let mut add = enabled && !contains(node);
    update(node, enabled, &mut add);
}

// Hide Canvas only for this render; keep its original dock location in app.dock.
fn hide_canvas_tabs(node: &mut DockNode) -> Vec<(Vec<bool>, usize)> {
    fn visit(node: &mut DockNode, path: &mut Vec<bool>, hidden: &mut Vec<(Vec<bool>, usize)>) {
        match node {
            DockNode::Leaf { tabs, active } => {
                if let Some(index) = tabs.iter().position(|tab| tab == "Canvas") {
                    tabs.remove(index);
                    if *active > index { *active -= 1; }
                    *active = (*active).min(tabs.len().saturating_sub(1));
                    hidden.push((path.clone(), index));
                }
            }
            DockNode::Split { first, second, .. } => {
                path.push(false); visit(first, path, hidden); path.pop();
                path.push(true); visit(second, path, hidden); path.pop();
            }
        }
    }
    let mut hidden = Vec::new();
    visit(node, &mut Vec::new(), &mut hidden);
    hidden
}

fn restore_canvas_tabs(node: &mut DockNode, hidden: Vec<(Vec<bool>, usize)>) {
    for (path, index) in hidden {
        let mut target = &mut *node;
        for second in path {
            match target {
                DockNode::Split { first, second: other, .. } => { target = if second { other } else { first }; }
                DockNode::Leaf { .. } => break,
            }
        }
        // If docking split this leaf while previewing, keep Canvas with its first child.
        while let DockNode::Split { first, .. } = target { target = first; }
        if let DockNode::Leaf { tabs, active } = target {
            let index = index.min(tabs.len());
            if !tabs.is_empty() && *active >= index { *active += 1; }
            tabs.insert(index, "Canvas".into());
        }
    }
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp, viewport: Vec2) {
    let scale = ui.scale();
    let menu_h = 28.0 * scale;
    let status_h = 24.0 * scale;

    let modal_open =
        app.ai.open || app.ai_settings.is_some() || app.settings_dialog.is_some() || app.hsv.open || app.rename.is_some() || app.unsaved.is_some() || app.delete_prompt.is_some() || app.effect_dialog.is_some() || app.new_document_open;
    ui.add_enabled(!modal_open, |ui| {
        ui.menu_bar(|ui| {
            menu_bar::draw(ui, app);
        });

        let dock_size = Vec2::new(viewport.x, (viewport.y - menu_h - status_h).max(1.0));
        let graph_mode = app.session.preview_graph;
        let preview = app.session.preview_asset.is_some();
        let video_document = app.video_editor.active.is_some();
        let scene_document = app.scene_editor.active;
        let scene_mode = scene_document.is_some();
        let active_dock = if scene_mode { &mut app.scene_dock } else { &mut app.dock };
        let mut dock = std::mem::replace(active_dock, DockState::new(DockNode::leaf(&["tmp"])));
        let preview_mode = preview || graph_mode || video_document;
        if !scene_mode {
            context_panel(&mut dock.root, preview_mode);
            clip_effects::dock(&mut dock.root, video_document);
            camera_dock(&mut dock.root, !preview_mode && app.project().animation.is_some());
        }
        let hidden_canvas = if !scene_mode && preview_mode { hide_canvas_tabs(&mut dock.root) } else { Vec::new() };
        app.nodes.visible = false;
        app.session.canvas_rect = mega_ui::Rect { min: Vec2::ZERO, max: Vec2::ZERO };
        ui.dock_space(if scene_mode { "scene" } else { "main" }, dock_size, &mut dock, |ui, tab| {
            if tab == "Workspace" {
                if graph_mode {
                    app.nodes.visible = true;
                    nodes::draw(ui, app);
                } else if app.scene_editor.active.is_some() {
                    scene_3d::viewport(ui, app);
                } else if app.video_editor.active.is_some() {
                    video_timeline::draw(ui, app);
                } else {
                    canvas::draw(ui, app);
                    if timeline::visible(app) { timeline::draw(ui, app); }
                }
                return;
            }
            let size = ui.available_size().max(Vec2::splat(1.0));
            let axes = if tab == "Library" || tab == "Effects" { mega_ui::ScrollAxes::Vertical } else { mega_ui::ScrollAxes::Both };
            ui.scroll_area(&format!("dock_scroll_{tab}"), size, axes, |ui| match tab {
                "Tools" => {
                    let selection_only = app.active_scene_layer();
                    if let Some(tool) = tools_panel::draw(ui, &mut app.session, selection_only) {
                        app.set_tool(tool);
                    }
                }
                "Tool Options" => {
                    let image_ready = app.image_size.is_some() && !app.layer_grid_active();
                    if let Some(horizontal) = tool_options::draw(ui, &mut app.session, image_ready, &mut app.value_sliders) {
                        app.flip_transform(horizontal);
                    }
                }
                "Scene Tools" => scene_3d::tools(ui, app),
                "Objects" => scene_3d::objects(ui, app),
                "Library" => {
                    #[cfg(windows)]
                    {
                        let label = if app.screen_recording_active() {
                            "Show Recorder"
                        } else {
                            "Record Screen…"
                        };
                        if ui.button("record_screen", label).clicked() {
                            app.record_screen(ui);
                        }
                    }
                    let action = assets_panel::draw(
                        ui,
                        &mut app.workspace,
                        Some(&app.nodes),
                        &app.video_editor,
                        &app.scene_editor,
                        &app.input,
                        app.session.workspace_rect,
                        graph_mode,
                        &mut app.library_drag,
                    );
                    apply_library(ui, app, action);
                }
                "Camera" if scene_mode => scene_3d::camera(ui, app),
                "Camera" => camera_panel::draw(ui, app),
                "Canvas" => canvas_panel::draw(ui, app),
                "Effects" => clip_effects::panel(ui, app),
                "Layers" | "Properties" => {
                    if app.video_editor.active.is_some() { video_timeline::properties(ui, app); }
                    else if app.session.preview_asset.is_some() { asset_properties::draw(ui, app); }
                    else if !graph_mode { project_panel::draw(ui, app); }
                }
                _ => {}
            });
        });
        restore_canvas_tabs(&mut dock.root, hidden_canvas);
        if scene_mode { app.scene_dock = dock; } else { app.dock = dock; }
        if preview != app.session.preview_asset.is_some()
            || graph_mode != app.session.preview_graph
            || video_document != app.video_editor.active.is_some()
            || scene_document != app.scene_editor.active
        {
            ui.request_repaint();
        }

        ui.status_bar(|ui| {
            ui.label(&app.status.clone());
            ui.spacer();
            if app.scene_editor.active.is_some() {
                ui.label("Session only · Right drag: orbit · Middle drag: pan · Wheel: zoom · F: frame");
            } else if app.video_editor.active.is_some() {
                ui.label("Wheel: zoom · Shift+wheel: tracks · Middle drag: pan");
            } else if app.layer_grid_active() {
                ui.label("Middle drag: pan · Wheel: zoom");
            } else {
                ui.label("Middle drag: pan · Wheel: zoom · Ctrl+S: save · Ctrl+Z/Y: undo/redo · Ctrl+X/C/V: cut/copy/paste · Delete: erase selection");
            }
            let fps = if app.fps > 0.0 { format!("{:.0} FPS", app.fps) } else { "— FPS".into() };
            ui.label(&fps);
        });
    });
    new_document::draw(ui, app);
    ai_dialog::draw(ui, app);
    ai_settings::draw(ui, app);
    editor_settings::draw(ui, app);
    hsv_dialog::draw(ui, app);
    rename_dialog::draw(ui, app);
    unsaved_dialog::draw(ui, app);
    delete_dialog::draw(ui, app);
    clip_effects::modal(ui, app);
}

fn add_to_graph(ui: &mut Ui, app: &mut EditorApp, kind: crate::nodes::NodeKind, ok: &str) {
    // Place the new node near the visible centre without changing pan or zoom.
    let center = app.session.viewport_size * 0.5;
    let zoom = app.nodes.space.zoom.max(1e-4);
    let offset = Vec2::splat((app.nodes.nodes.len() % 5) as f32 * 35.0);
    app.nodes.space.context_world = Some((center - app.nodes.space.pan) / zoom + offset);
    let before = app.nodes.nodes.len();
    app.nodes.add(kind);
    app.status = if app.nodes.nodes.len() > before {
        ok.into()
    } else {
        app.nodes.message.clone()
    };
    ui.request_repaint();
}

fn apply_library(ui: &mut Ui, app: &mut EditorApp, action: LibraryAction) {
    if let Some(asset) = action.drop_timeline_asset {
        if let (Some(track), Some(doc)) = (app.video_editor.track, app.video_editor.current())
            && let Some((lane, seconds, radius)) = track.drop_position(doc, app.input.mouse_pos, ui.scale())
        {
            use crate::workspace::ThumbDrag;
            let inserted = match asset {
                ThumbDrag::Video(index) => match app.workspace.videos.get(index) {
                    Some(asset) if asset.info.is_some() => Ok(app.video_editor.insert(asset, lane, seconds, radius)),
                    _ => Err("Video information is unavailable".into()),
                },
                ThumbDrag::Image(index) => app.workspace.ensure_asset_loaded(index).map(|()| {
                    app.video_editor.insert_image(&app.workspace.assets[index], lane, seconds, radius)
                }),
                ThumbDrag::Document(_) | ThumbDrag::Scene(_) => return,
            };
            match inserted {
                Ok(true) => {}
                Ok(false) => ui.notify_error("Drop the clip on free space"),
                Err(error) => ui.notify_error(error),
            };
        }
        ui.request_repaint();
        return;
    }
    if action.open_new_document {
        app.new_document_open = true;
        ui.request_repaint();
    }
    if let Some(index) = action.open_scene_document {
        app.show_scene_document(index);
        ui.request_repaint();
    }
    if let Some(index) = action.delete_scene_document {
        app.delete_scene_document(index);
        ui.request_repaint();
    }
    if let Some(index) = action.open_video_document {
        app.show_video_document(index);
        ui.request_repaint();
    }
    if let Some(index) = action.delete_video_document {
        app.delete_video_document(index);
    }
    if let Some(id) = action.add_document_to_graph {
        if app.workspace.documents.iter().any(|doc| doc.id == id) {
            add_to_graph(
                ui,
                app,
                crate::nodes::NodeKind::ImageDocument(crate::nodes::Source {
                    document: id,
                    layer: None,
                }),
                "Document added to Graph",
            );
            app.show_graph(app.workspace.graphs.active);
        }
        return;
    }
    if let Some(id) = action.add_asset_to_graph {
        if app.workspace.assets.iter().any(|asset| asset.id == id) {
            add_to_graph(
                ui,
                app,
                crate::nodes::NodeKind::ImageAsset { id },
                "Asset added to Graph",
            );
            app.show_graph(app.workspace.graphs.active);
        }
        return;
    }
    if let Some((kind, id, slot)) = action.reorder {
        if app.workspace.reorder_library(kind, &id, slot) {
            app.dirty = true;
            ui.request_repaint();
        }
        return;
    }
    if let Some(target) = action.rename {
        app.open_rename(target);
        return;
    }
    if action.import {
        app.import_image(ui);
    }
    if action.import_clipboard {
        app.import_clipboard_image(ui);
    }
    if let Some(index) = action.switch_graph {
        app.show_graph(index);
    }
    if let Some(index) = action.clone_graph {
        app.clone_graph(index);
    }
    if let Some(index) = action.delete_graph {
        app.delete_graph(index);
    }
    if let Some(index) = action.switch_doc {
        app.switch_document(index);
    }
    if let Some(index) = action.open_video {
        app.show_video_preview(index);
    }
    if let Some(index) = action.delete_video {
        app.prompt_library_delete(crate::workspace::LibraryKind::Video, index);
    }
    if let Some(index) = action.preview_asset {
        app.show_asset_preview(index);
    }
    if let Some(index) = action.delete_doc {
        app.prompt_library_delete(crate::workspace::LibraryKind::Document, index);
    }
    if let Some(index) = action.clone_doc {
        app.duplicate_document(index);
    }
    if let Some(index) = action.place_asset && !app.layer_grid_active() {
        app.place_asset(index, ui);
    }
    if let Some(index) = action.place_scene {
        app.place_scene_layer(index);
        ui.request_repaint();
    }
    if let Some(index) = action.delete_asset {
        app.prompt_library_delete(crate::workspace::LibraryKind::Image, index);
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
    #[test]
    fn workspace_dock_has_no_graph_tabs() {
        let dock = default_dock();
        fn tabs(node: &DockNode) -> Vec<String> {
            match node {
                DockNode::Leaf { tabs, .. } => tabs.clone(),
                DockNode::Split { first, second, .. } => {
                    let mut all = tabs(first);
                    all.extend(tabs(second));
                    all
                }
            }
        }
        let tabs = tabs(&dock.root);
        assert!(tabs.iter().any(|tab| tab == "Workspace"));
        assert!(tabs.iter().any(|tab| tab == "Library"));
        assert!(!tabs.iter().any(|tab| tab == "Graph" || tab == "Graphs"));
    }
    #[test]
    fn add_document_to_nodes_uses_requested_document_and_preserves_view() {
        let mut app = EditorApp::default();
        let source = app.workspace.active_doc().id.clone();
        app.new_document();
        let active = app.workspace.active_doc().id.clone();
        app.session.viewport_size = Vec2::new(1000.0, 800.0);
        app.nodes.space.pan = Vec2::new(100.0, 50.0);
        app.nodes.space.zoom = 0.75;
        let mut ui = Ui::new();
        apply_library(&mut ui, &mut app, LibraryAction {
            add_document_to_graph: Some(source.clone()), ..Default::default()
        });
        assert_eq!(app.workspace.active_doc().id, active);
        assert_eq!(app.nodes.space.pan, Vec2::new(100.0, 50.0));
        assert_eq!(app.nodes.space.zoom, 0.75);
        assert!(matches!(&app.nodes.nodes[0].kind,
            crate::nodes::NodeKind::ImageDocument(s) if s.document == source && s.layer.is_none()));
        assert!(!app.nodes.space.fit_view);
    }
    #[test]
    fn add_asset_to_graph_uses_requested_asset_and_preserves_view() {
        let mut app = EditorApp::default();
        app.workspace
            .import_asset_rgba("Ref".into(), &[1, 2, 3, 255].repeat(2), 2, 1)
            .unwrap();
        let id = app.workspace.assets[0].id.clone();
        app.session.viewport_size = Vec2::new(1000.0, 800.0);
        app.nodes.space.pan = Vec2::new(40.0, -20.0);
        app.nodes.space.zoom = 1.25;
        let mut ui = Ui::new();
        apply_library(&mut ui, &mut app, LibraryAction {
            add_asset_to_graph: Some(id.clone()), ..Default::default()
        });
        assert_eq!(app.nodes.space.pan, Vec2::new(40.0, -20.0));
        assert_eq!(app.nodes.space.zoom, 1.25);
        assert!(matches!(&app.nodes.nodes[0].kind,
            crate::nodes::NodeKind::ImageAsset { id: asset } if *asset == id));
        assert!(!app.nodes.space.fit_view);
    }
    #[test]
    fn context_panel_switches_titles_without_rebuilding_the_dock() {
        let mut dock = default_dock();
        fn tabs(node: &DockNode) -> Vec<String> {
            match node {
                DockNode::Leaf { tabs, .. } => tabs.clone(),
                DockNode::Split { first, second, .. } => {
                    let mut all = tabs(first);
                    all.extend(tabs(second));
                    all
                }
            }
        }
        let original = tabs(&dock.root);
        assert_eq!(original.iter().filter(|tab| *tab == "Workspace").count(), 1);
        assert_eq!(original.iter().filter(|tab| *tab == "Canvas").count(), 1);
        context_panel(&mut dock.root, true);
        let hidden = hide_canvas_tabs(&mut dock.root);
        let preview = tabs(&dock.root);
        assert!(!preview.iter().any(|tab| tab == "Canvas"));
        assert!(!preview.iter().any(|tab| tab == "Layers"));
        assert_eq!(preview.iter().filter(|tab| *tab == "Properties").count(), 1);
        if let DockNode::Split { ratio, .. } = &mut dock.root {
            *ratio = 0.27;
        }
        restore_canvas_tabs(&mut dock.root, hidden);
        context_panel(&mut dock.root, false);
        assert_eq!(tabs(&dock.root), original);
        if let DockNode::Split { ratio, .. } = &dock.root {
            assert_eq!(*ratio, 0.27);
        }
    }
}
