use mega_ui::Ui;

use crate::app::DocumentSession;
use crate::editing::Tool;

/// Register the editor's tool artwork with Mega UI's SVG icon renderer.
pub fn load_icons(ui: &mut Ui) {
    ui.load_icons([
        (
            "tool_brush",
            include_bytes!("../../assets/icons/tools/brush.svg").as_slice(),
        ),
        (
            "tool_select",
            include_bytes!("../../assets/icons/tools/select.svg").as_slice(),
        ),
        (
            "tool_fill",
            include_bytes!("../../assets/icons/tools/fill.svg").as_slice(),
        ),
        (
            "tool_transform",
            include_bytes!("../../assets/icons/tools/transform.svg").as_slice(),
        ),
        (
            "tool_deform",
            include_bytes!("../../assets/icons/tools/deform.svg").as_slice(),
        ),
    ]);
}

fn icon(tool: Tool) -> &'static str {
    match tool {
        Tool::Brush => "tool_brush",
        Tool::Fill => "tool_fill",
        Tool::Transform => "tool_transform",
        Tool::Deform => "tool_deform",
        Tool::RectangleSelect
        | Tool::Lasso
        | Tool::PolygonSelect
        | Tool::SelectionBrush
        | Tool::MagicWand => "tool_select",
    }
}

/// Picks a tool. The editor applies it so committing a transform stays outside the panel.
pub fn draw(ui: &mut Ui, session: &mut DocumentSession) -> Option<Tool> {
    let mut chosen = None;
    ui.grid(3, |ui| {
        for tool in Tool::toolbar() {
            ui.grid_cell(|ui| {
                let selected = if tool.is_select() {
                    session.tool.is_select()
                } else {
                    session.tool == tool
                };
                if ui
                    .selectable(&format!("tool_{}", tool.label()), selected, |ui| {
                        ui.icon(icon(tool), 24.0);
                    })
                    .clicked()
                {
                    chosen = Some(if tool.is_select() && session.tool.is_select() {
                        session.tool
                    } else if tool.is_select() {
                        session.select_tool
                    } else {
                        tool
                    });
                }
            });
        }
    });
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::EditorApp;

    #[test]
    fn toolbar_keeps_one_select_tool_and_restores_last_mode() {
        assert_eq!(Tool::toolbar(), [
            Tool::Brush,
            Tool::RectangleSelect,
            Tool::Fill,
            Tool::Transform,
            Tool::Deform,
        ]);
        let mut app = EditorApp::default();
        app.set_tool(Tool::Lasso);
        app.set_tool(Tool::Fill);
        assert_eq!(app.session.select_tool, Tool::Lasso);
        app.set_tool(app.session.select_tool);
        assert_eq!(app.session.tool, Tool::Lasso);
        assert!(app.session.tool.is_select());
    }
}
