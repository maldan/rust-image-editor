//! Editor chrome: menus, dock tabs, status bar.

pub mod ai_dialog;
pub mod hsv_dialog;
mod assets_panel;
mod canvas;
mod menu_bar;
mod project_panel;
mod tool_options;
mod tools_panel;

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
            DockNode::leaf(&["Canvas"]),
            DockNode::leaf(&["Layers"]),
        ),
    ))
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp, viewport: Vec2) {
    let scale = ui.scale();
    let menu_h = 28.0 * scale;
    let status_h = 24.0 * scale;

    let modal_open = app.ai.open || app.hsv.open;
    ui.add_enabled(!modal_open, |ui| {
        ui.menu_bar(|ui| {
            menu_bar::draw(ui, app);
        });

        let dock_size = Vec2::new(viewport.x, (viewport.y - menu_h - status_h).max(1.0));
        let mut dock = std::mem::replace(&mut app.dock, DockState::new(DockNode::leaf(&["tmp"])));
        ui.dock_space("main", dock_size, &mut dock, |ui, tab| {
            if tab == "Canvas" {
                canvas::draw(ui, app);
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
                "Layers" => project_panel::draw(ui, app),
                _ => {}
            });
        });
        app.dock = dock;

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
}

fn apply_library(ui: &mut Ui, app: &mut EditorApp, action: LibraryAction) {
    if action.import {
        app.import_image(ui);
    }
    if action.new_document {
        app.new_document();
    }
    if let Some(index) = action.switch_doc {
        app.switch_document(index);
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
