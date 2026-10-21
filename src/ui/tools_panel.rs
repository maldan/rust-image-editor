use mega_ui::Ui;

use crate::app::DocumentSession;
use crate::editing::Tool;

/// Picks a tool. The editor applies it so committing a transform stays outside the panel.
pub fn draw(ui: &mut Ui, session: &mut DocumentSession) -> Option<Tool> {
    ui.label("Tool");
    ui.space(8.0);
    let mut chosen = None;
    let tools = Tool::all();
    ui.grid(2, |ui| {
        for tool in tools {
            ui.grid_cell(|ui| {
                let selected = session.tool == tool;
                if ui
                    .selectable(&format!("tool_{}", tool.label()), selected, |ui| {
                        ui.label(tool.label());
                    })
                    .clicked()
                {
                    chosen = Some(tool);
                }
            });
        }
    });
    chosen
}
