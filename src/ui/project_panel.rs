use mega_ui::Ui;

use crate::app::EditorApp;
use crate::host::FrameInput;

struct LayerCommand {
    select: Option<usize>,
    visibility: Option<(usize, bool)>,
    copy: Option<usize>,
    rename: Option<usize>,
    delete: Option<usize>,
    reorder: Option<(usize, usize)>,
    opacity: Option<f32>,
    new_layer: bool,
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let rows: Vec<(String, bool, f32)> = app
        .project()
        .layers
        .iter()
        .map(|layer| (layer.name.clone(), layer.visible, layer.opacity))
        .collect();
    let active = app.project().active_layer;
    let command = controls(ui, &rows, active, &app.input, &mut app.layer_drag);
    if let Some(index) = command.select {
        app.project_mut().active_layer = index;
    }
    if let Some((index, visible)) = command.visibility {
        app.set_layer_visible(index, visible);
    }
    if let Some(index) = command.rename {
        app.open_layer_rename(index);
    } else if let Some(index) = command.copy {
        app.copy_layer_at(index, ui);
        app.layer_drag = None;
    } else if let Some(index) = command.delete {
        app.delete_layer(index);
    } else if let Some((from, to)) = command.reorder {
        app.move_layer_to(from, to);
        app.layer_drag = Some(to);
    }
    if let Some(opacity) = command.opacity {
        app.set_active_layer_opacity(opacity);
    }
    if command.new_layer {
        app.add_layer();
    }
}

fn controls(
    ui: &mut Ui,
    rows: &[(String, bool, f32)],
    active: usize,
    input: &FrameInput,
    drag: &mut Option<usize>,
) -> LayerCommand {
    let mut command = LayerCommand {
        select: None,
        visibility: None,
        copy: None,
        rename: None,
        delete: None,
        reorder: None,
        opacity: None,
        new_layer: false,
    };
    ui.label("Layers");
    ui.separator();
    ui.space(8.0);

    if rows.is_empty() {
        ui.label("No layers");
    } else {
        for index in (0..rows.len()).rev() {
            let (name, visible0, _) = &rows[index];
            let mut visible = *visible0;
            let row_id = format!("layer_row_{index}");
            let response = ui.selectable(&row_id, active == index, |ui| {
                ui.horizontal(|ui| {
                    if ui.icon_button(
                        &format!("layer_vis_{index}"),
                        if visible {
                            "visibility"
                        } else {
                            "visibility_off"
                        },
                        visible,
                    ) {
                        visible = !visible;
                        command.visibility = Some((index, visible));
                    }
                    let name_width = (ui.available_size().x / ui.scale() - 170.0).max(1.0);
                    ui.label_fixed(name_width, name);
                    ui.spacer();
                    if ui.icon_button(&format!("layer_delete_{index}"), "delete", false) {
                        command.delete = Some(index);
                    }
                    if ui.icon_button(&format!("layer_rename_{index}"), "edit", false) {
                        command.rename = Some(index);
                    }
                    if ui.icon_button(&format!("layer_copy_{index}"), "copy", false) {
                        command.copy = Some(index);
                    }
                });
            });
            if response.clicked() {
                command.select = Some(index);
            }
            if response.hovered && input.mouse_pressed {
                *drag = Some(index);
            }
            if let Some(from) = *drag
                && response.hovered
                && input.mouse_down
                && from != index
            {
                command.reorder = Some((from, index));
            }
        }
        if input.mouse_released {
            *drag = None;
        }

        ui.space(12.0);
        if let Some((_, _, opacity)) = rows.get(active) {
            let mut opacity = *opacity;
            ui.label(&format!("Opacity: {:.0}%", opacity * 100.0));
            if ui
                .slider("layer_opacity", &mut opacity, 0.0..=1.0)
                .changed()
            {
                command.opacity = Some(opacity);
            }
        }
    }

    ui.space(12.0);
    if ui.button("layer_new", "New Layer").clicked() {
        command.new_layer = true;
    }

    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    #[test]
    fn pencil_opens_rename_for_its_row_without_selecting_or_cloning_it() {
        let mut app = EditorApp::default();
        app.project_mut()
            .import_rgba(&[1, 2, 3, 255], 1, 1)
            .unwrap();
        app.project_mut().add_layer();
        app.project_mut().active_layer = 0;
        app.refresh_image().unwrap();
        let mut ui = Ui::new();
        ui.load_builtin_icons();
        let mut output = mega_ui::UiOutput::default();
        for _ in 0..3 {
            ui.begin_frame(mega_ui::UiInput {
                viewport: Vec2::new(400.0, 900.0),
                ..Default::default()
            });
            ui.horizontal(|ui| ui.flex_width(360.0, |ui| draw(ui, &mut app)));
            output = ui.end_frame();
        }
        let mut icons: Vec<_> = output
            .draw_list
            .iter()
            .filter(|c| {
                c.rect.min.x > 180.0
                    && c.rect.max.y < 200.0
                    && (c.rect.width() - 16.0).abs() < 0.1
                    && (c.rect.height() - 16.0).abs() < 0.1
            })
            .collect();
        icons.sort_by(|a, b| {
            a.rect
                .min
                .y
                .total_cmp(&b.rect.min.y)
                .then_with(|| a.rect.min.x.total_cmp(&b.rect.min.x))
        });
        // Trailing widgets are allocated right-to-left: copy, pencil, delete on screen.
        let icon = icons[1];
        let position = (icon.rect.min + icon.rect.max) * 0.5;
        for pressed in [true, false] {
            app.input.mouse_pos = position;
            app.input.mouse_pressed = pressed;
            app.input.mouse_down = pressed;
            app.input.mouse_released = !pressed;
            ui.begin_frame(mega_ui::UiInput {
                viewport: Vec2::new(400.0, 900.0),
                mouse_pos: position,
                mouse_pressed: pressed,
                mouse_down: pressed,
                mouse_released: !pressed,
                ..Default::default()
            });
            ui.horizontal(|ui| ui.flex_width(360.0, |ui| draw(ui, &mut app)));
            ui.end_frame();
        }
        let dialog = app.rename.as_ref().expect("pencil opens the rename modal");
        assert!(matches!(
            &dialog.target,
            crate::app::RenameTarget::Layer { index: 1, .. }
        ));
        assert_eq!(dialog.name, "Layer 2");
        assert_eq!(app.project().active_layer, 0);
        assert_eq!(app.project().layers.len(), 2);
        assert!(app.layer_drag.is_none());
    }

    #[test]
    fn right_hand_icon_deletes_its_row_without_changing_active_layer_and_can_undo() {
        let mut app = EditorApp::default();
        app.project_mut()
            .import_rgba(&[1, 2, 3, 255], 1, 1)
            .unwrap();
        app.project_mut().add_layer();
        app.project_mut().active_layer = 0;
        app.refresh_image().unwrap();
        let mut ui = Ui::new();
        ui.load_builtin_icons();
        let mut output = mega_ui::UiOutput::default();
        for _ in 0..3 {
            ui.begin_frame(mega_ui::UiInput {
                viewport: Vec2::new(400.0, 900.0),
                ..Default::default()
            });
            ui.horizontal(|ui| ui.flex_width(360.0, |ui| draw(ui, &mut app)));
            output = ui.end_frame();
        }
        let icon = output
            .draw_list
            .iter()
            .find(|c| {
                c.rect.min.x > 180.0
                    && c.rect.max.y < 200.0
                    && (c.rect.width() - 16.0).abs() < 0.1
                    && (c.rect.height() - 16.0).abs() < 0.1
            })
            .expect("Delete icon pinned to the right of the top row");
        let position = (icon.rect.min + icon.rect.max) * 0.5;
        for pressed in [true, false] {
            app.input.mouse_pos = position;
            app.input.mouse_pressed = pressed;
            app.input.mouse_down = pressed;
            app.input.mouse_released = !pressed;
            ui.begin_frame(mega_ui::UiInput {
                viewport: Vec2::new(400.0, 900.0),
                mouse_pos: position,
                mouse_pressed: pressed,
                mouse_down: pressed,
                mouse_released: !pressed,
                ..Default::default()
            });
            ui.horizontal(|ui| ui.flex_width(360.0, |ui| draw(ui, &mut app)));
            ui.end_frame();
        }
        assert_eq!(app.project().layers.len(), 1);
        assert_eq!(app.project().layers[0].name, "Layer 1");
        assert_eq!(app.project().active_layer, 0);
        app.undo();
        assert_eq!(app.project().layers.len(), 2);
        assert_eq!(app.project().active_layer, 0);
        app.redo();
        assert_eq!(app.project().layers.len(), 1);
    }
}
