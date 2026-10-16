//! Editor chrome: menus, dock tabs, status bar.

pub mod ai_dialog;
mod assets_panel;
mod canvas;
mod menu_bar;
mod project_panel;
mod tool_options;
mod tools_panel;

use glam::Vec2;
use mega_ui::{DockNode, DockState, Ui};

use crate::app::EditorApp;

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

    let modal_open = app.ai.open;
    ui.add_enabled(!modal_open, |ui| {
        ui.menu_bar(|ui| {
            menu_bar::draw(ui, app);
        });

        let dock_size = Vec2::new(viewport.x, (viewport.y - menu_h - status_h).max(1.0));
        let mut dock = std::mem::replace(&mut app.dock, DockState::new(DockNode::leaf(&["tmp"])));
        ui.dock_space("main", dock_size, &mut dock, |ui, tab| match tab {
            "Tools" => tools_panel::draw(ui, app),
            "Tool Options" => tool_options::draw(ui, app),
            "Library" => assets_panel::draw(ui, app),
            "Canvas" => canvas::draw(ui, app),
            "Layers" => project_panel::draw(ui, app),
            _ => {}
        });
        app.dock = dock;

        ui.status_bar(|ui| {
            ui.label(&app.status.clone());
            ui.spacer();
            ui.label("Middle drag: pan · Wheel: zoom · Ctrl+Z/Y: undo/redo · Ctrl+V: paste");
        });
    });
    ai_dialog::draw(ui, app);
}
