use mega_ui::Ui;

use crate::app::EditorApp;
use crate::editing::Tool;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    ui.label("Tool");
    ui.space(8.0);
    let tools = Tool::all();
    ui.grid(2, |ui| {
        for tool in tools {
            ui.grid_cell(|ui| {
                let selected = app.tool == tool;
                if ui
                    .selectable(&format!("tool_{}", tool.label()), selected, |ui| {
                        ui.label(tool.label());
                    })
                    .clicked()
                {
                    app.set_tool(tool);
                }
            });
        }
    });
}
