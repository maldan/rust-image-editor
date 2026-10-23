//! Editor chrome: menus, dock tabs, status bar.

pub mod ai_dialog;
mod animation_dialog;
mod asset_properties;
mod assets_panel;
mod canvas;
mod canvas_panel;
mod camera_panel;
pub mod hsv_dialog;
mod menu_bar;
mod project_panel;
pub mod rename_dialog;
mod tool_options;
mod tools_panel;
mod timeline;
mod video_dialog;

use glam::Vec2;
use mega_ui::{DockNode, DockState, Ui};

use crate::app::EditorApp;
use assets_panel::LibraryAction;

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

fn context_panel(node: &mut DockNode, preview: bool) {
    match node {
        DockNode::Leaf { tabs, .. } => {
            for tab in tabs {
                if tab == "Layers" || tab == "Properties" {
                    *tab = if preview { "Properties" } else { "Layers" }.into();
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

pub fn draw(ui: &mut Ui, app: &mut EditorApp, viewport: Vec2) {
    let scale = ui.scale();
    let menu_h = 28.0 * scale;
    let status_h = 24.0 * scale;

    let modal_open = app.ai.open || app.hsv.open || app.rename.is_some();
    ui.add_enabled(!modal_open, |ui| {
        ui.menu_bar(|ui| {
            menu_bar::draw(ui, app);
        });

        let dock_size = Vec2::new(viewport.x, (viewport.y - menu_h - status_h).max(1.0));
        let mut dock = std::mem::replace(&mut app.dock, DockState::new(DockNode::leaf(&["tmp"])));
        let preview = app.session.preview_asset.is_some();
        context_panel(&mut dock.root, preview);
        camera_dock(&mut dock.root, !preview && app.project().animation.is_some());
        ui.dock_space("main", dock_size, &mut dock, |ui, tab| {
            if tab == "Workspace" {
                canvas::draw(ui, app);
                if timeline::visible(app) { timeline::draw(ui, app); }
                return;
            }
            let size = ui.available_size().max(Vec2::splat(1.0));
            let axes = if tab == "Library" { mega_ui::ScrollAxes::Vertical } else { mega_ui::ScrollAxes::Both };
            ui.scroll_area(&format!("dock_scroll_{tab}"), size, axes, |ui| match tab {
                "Tools" => {
                    if let Some(tool) = tools_panel::draw(ui, &mut app.session) {
                        app.set_tool(tool);
                    }
                }
                "Tool Options" => {
                    let image_ready = app.image_size.is_some();
                    if let Some(horizontal) = tool_options::draw(ui, &mut app.session, image_ready) {
                        app.flip_transform(horizontal);
                    }
                }
                "Library" => {
                    let action = assets_panel::draw(ui, &mut app.workspace, &app.input, app.session.canvas_rect);
                    apply_library(ui, app, action);
                }
                "Camera" => camera_panel::draw(ui, app),
                "Canvas" => canvas_panel::draw(ui, app),
                "Layers" | "Properties" => {
                    if app.session.preview_asset.is_some() { asset_properties::draw(ui, app); }
                    else { project_panel::draw(ui, app); }
                }
                _ => {}
            });
        });
        app.dock = dock;
        if preview != app.session.preview_asset.is_some() { ui.request_repaint(); }

        ui.status_bar(|ui| {
            ui.label(&app.status.clone());
            ui.spacer();
            ui.label("Middle drag: pan · Wheel: zoom · Ctrl+Z/Y: undo/redo · Ctrl+X/C/V: cut/copy/paste · Delete: erase selection");
            let fps = if app.fps > 0.0 { format!("{:.0} FPS", app.fps) } else { "— FPS".into() };
            ui.label(&fps);
        });
    });
    ai_dialog::draw(ui, app);
    hsv_dialog::draw(ui, app);
    rename_dialog::draw(ui, app);
}

fn apply_library(ui: &mut Ui, app: &mut EditorApp, action: LibraryAction) {
    if let Some(target) = action.rename {
        app.open_rename(target);
        return;
    }
    if action.import {
        app.import_image(ui);
    }
    if action.new_document {
        app.new_document();
    }
    if action.new_animation {
        app.new_animation();
    }
    if let Some(index) = action.switch_doc {
        app.switch_document(index);
    }
    if let Some(index) = action.open_video {
        app.show_video_preview(index);
    }
    if let Some(index) = action.delete_video {
        if app.session.preview_asset.as_ref() == app.workspace.videos.get(index).map(|v| &v.id) {
            app.close_asset_preview();
        }
        app.workspace.delete_video(index);
        app.dirty = true;
    }
    if let Some(index) = action.preview_asset {
        app.show_asset_preview(index);
    }
    if let Some(index) = action.delete_doc {
        app.delete_document(index);
    }
    if let Some(index) = action.clone_doc {
        app.duplicate_document(index);
    }
    if let Some(index) = action.place_asset {
        app.place_asset(index, ui);
    }
    if let Some(index) = action.delete_asset {
        if app.session.preview_asset.as_ref() == app.workspace.assets.get(index).map(|a| &a.id) {
            app.close_asset_preview();
        }
        app.workspace.delete_asset(index);
        app.dirty = true;
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
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
        let preview = tabs(&dock.root);
        assert!(preview.iter().any(|tab| tab == "Canvas"));
        assert!(!preview.iter().any(|tab| tab == "Layers"));
        assert_eq!(preview.iter().filter(|tab| *tab == "Properties").count(), 1);
        if let DockNode::Split { ratio, .. } = &mut dock.root {
            *ratio = 0.27;
        }
        context_panel(&mut dock.root, false);
        assert_eq!(tabs(&dock.root), original);
        if let DockNode::Split { ratio, .. } = &dock.root {
            assert_eq!(*ratio, 0.27);
        }
    }
}
