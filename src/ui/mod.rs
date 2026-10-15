//! Editor chrome: menus, dock tabs, status bar.

mod canvas;
mod menu_bar;
mod project_panel;

use glam::Vec2;
use mega_ui::{DockNode, DockState, Ui};

use crate::app::EditorApp;

pub fn default_dock() -> DockState {
    DockState::new(DockNode::split_h(
        0.78,
        DockNode::leaf(&["Canvas"]),
        DockNode::leaf(&["Project"]),
    ))
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp, viewport: Vec2) {
    let scale = ui.scale();
    let menu_h = 28.0 * scale;
    let status_h = 24.0 * scale;

    ui.menu_bar(|ui| {
        menu_bar::draw(ui, app);
    });

    let dock_size = Vec2::new(viewport.x, (viewport.y - menu_h - status_h).max(1.0));
    let mut dock = std::mem::replace(&mut app.dock, DockState::new(DockNode::leaf(&["tmp"])));
    ui.dock_space("main", dock_size, &mut dock, |ui, tab| match tab {
        "Canvas" => canvas::draw(ui, app),
        "Project" => project_panel::draw(ui, app),
        _ => {}
    });
    app.dock = dock;

    ui.status_bar(|ui| {
        ui.label(&app.status.clone());
        ui.spacer();
        ui.label("Middle drag: pan · Wheel: zoom · Ctrl+V: paste");
    });
}
