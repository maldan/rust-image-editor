use mega_ui::Ui;

use super::value_slider::{ValueSlider, ValueSliderState};
use crate::app::EditorApp;
use crate::host::FrameInput;

struct LayerCommand {
    select: Option<usize>,
    visibility: Option<(usize, bool)>,
    duplicate: Option<usize>,
    copy: Option<usize>,
    copy_image: Option<usize>,
    rename: Option<usize>,
    delete: Option<usize>,
    merge_down: Option<usize>,
    reorder: Option<(usize, usize)>,
    opacity: Option<f32>,
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let mut grid = app.layer_grid_active();
    let row_input = !ui.context_menu_open() && !ui.pointer_over_popup_at(app.input.mouse_pos);
    ui.space(8.0);
    let new_layer = ui.button("layer_new", "New Layer").clicked() && row_input;
    if app.project().animation.is_none() {
        let can_toggle = ui.enabled() && row_input && (grid || app.session.floating.is_none());
        if ui.checkbox("toggle_layer_grid", "Layer Grid", &mut grid).changed() && can_toggle {
            if grid {
                app.open_layer_grid();
            } else {
                app.layer_grid.document = None;
            }
            ui.request_repaint();
        }
    }
    if app.layer_grid_active() {
        ui.label("Max per row (0 = unlimited)");
        let mut limit = app.layer_grid.max_per_row;
        ui.drag_int("layer_grid_max_per_row", &mut limit, 1);
        let limit = limit.max(0);
        if limit != app.layer_grid.max_per_row {
            app.layer_grid.max_per_row = limit;
            ui.request_repaint();
        }
    }
    draw_layers(ui, app);
    if new_layer {
        app.add_layer();
        ui.request_repaint();
    }
}

fn draw_layers(ui: &mut Ui, app: &mut EditorApp) {
    let rows: Vec<(String, bool, f32, bool)> = app
        .project()
        .layers
        .iter()
        .map(|layer| {
            let raster = layer.is_raster();
            let name = if raster { layer.name.clone() } else { format!("{} · 3D", layer.name) };
            (name, layer.visible, layer.opacity, raster)
        })
        .collect();
    let active = app.project().active_layer;
    let opacity_id = format!("layer_opacity:{}:{active}", app.workspace.active_doc().id);
    let command = controls(
        ui,
        &rows,
        active,
        &app.input,
        &mut app.layer_drag,
        &mut app.value_sliders,
        &opacity_id,
    );
    if app.project().animation.is_some()
        && let Some(layer) = app.project().layers.get(active)
    {
        let mut z = layer.z;
        ui.label("Z · positive = farther away");
        if ui.drag_float("layer_z", &mut z, 0.05).changed() {
            app.set_layer_depth(z);
        }
    }
    if let Some(index) = command.select {
        app.session.camera_selected = false;
        app.project_mut().active_layer = index;
        if app.active_scene_layer() {
            app.set_tool(app.session.select_tool);
        }
    }
    if let Some((index, visible)) = command.visibility {
        app.set_layer_visible(index, visible);
        ui.request_repaint();
    }
    if let Some(index) = command.rename {
        app.open_layer_rename(index);
    } else if let Some(index) = command.copy {
        app.copy_layer_at(index, ui);
        app.layer_drag = None;
    } else if let Some(index) = command.copy_image {
        app.copy_layer_as_image(index, ui);
        app.layer_drag = None;
    } else if let Some(index) = command.duplicate {
        app.duplicate_layer(index);
        app.layer_drag = None;
    } else if let Some(index) = command.delete {
        app.delete_layer(index);
    } else if let Some(index) = command.merge_down {
        app.merge_layer_down_at(index);
    } else if let Some((from, to)) = command.reorder {
        app.move_layer_to(from, to);
        app.layer_drag = Some(to);
        ui.request_repaint();
    }
    if let Some(opacity) = command.opacity {
        app.set_active_layer_opacity(opacity);
    }
}

fn controls(
    ui: &mut Ui,
    rows: &[(String, bool, f32, bool)],
    active: usize,
    input: &FrameInput,
    drag: &mut Option<usize>,
    sliders: &mut ValueSliderState,
    opacity_id: &str,
) -> LayerCommand {
    let mut command = LayerCommand {
        select: None,
        visibility: None,
        duplicate: None,
        copy: None,
        copy_image: None,
        rename: None,
        delete: None,
        merge_down: None,
        reorder: None,
        opacity: None,
    };
    let row_input = ui.enabled() && !ui.context_menu_open() && !ui.pointer_over_popup_at(input.mouse_pos);
    if !row_input {
        *drag = None;
    }
    ui.space(12.0);

    if rows.is_empty() {
        ui.label("No layers");
    } else {
        for index in (0..rows.len()).rev() {
            let (name, visible0, _, raster) = &rows[index];
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
                    ) && row_input
                    {
                        visible = !visible;
                        command.visibility = Some((index, visible));
                    }
                    let name_width = (ui.available_size().x / ui.scale()).max(1.0);
                    ui.label_fixed(name_width, name);
                });
            });
            if row_input && response.clicked() {
                command.select = Some(index);
            }
            ui.context_menu(&format!("layer_menu_{index}"), response.hovered, |ui| {
                if ui.menu_item("copy", "Copy").clicked() {
                    command.copy = Some(index);
                }
                if ui.menu_item("duplicate", "Duplicate").clicked() {
                    command.duplicate = Some(index);
                }
                if ui.menu_item("copy_image", "Copy as Image").clicked() {
                    command.copy_image = Some(index);
                }
                if ui.menu_item("edit", "Edit").clicked() {
                    command.rename = Some(index);
                }
                if ui.menu_item("delete", "Delete").clicked() {
                    command.delete = Some(index);
                }
                if ui
                    .menu_item_enabled("merge_down", "Merge Down", *raster && index > 0 && rows[index - 1].3)
                    .clicked()
                {
                    command.merge_down = Some(index);
                }
            });
            if row_input && response.hovered && input.mouse_pressed {
                *drag = Some(index);
            }
            if let Some(from) = *drag
                && row_input
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
        if let Some((_, _, opacity, _)) = rows.get(active) {
            let mut opacity = *opacity;
            if sliders
                .draw(ui, opacity_id, &mut opacity, ValueSlider::percent("Opacity", 0.0))
                .changed()
            {
                command.opacity = Some(opacity);
            }
        }
    }

    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Raster;
    use glam::Vec2;

    fn panel_frame(
        ui: &mut Ui,
        app: &mut EditorApp,
        position: Vec2,
        pressed: bool,
        released: bool,
        right: bool,
    ) -> mega_ui::UiOutput {
        app.input.mouse_pos = position;
        app.input.mouse_pressed = pressed && !right;
        app.input.mouse_down = !right && !released && (pressed || app.input.mouse_down);
        app.input.mouse_released = released && !right;
        app.input.mouse_right_pressed = pressed && right;
        app.input.mouse_right_down = pressed && right;
        app.input.mouse_right_released = released && right;
        ui.begin_frame(mega_ui::UiInput {
            viewport: Vec2::new(400.0, 900.0),
            mouse_pos: position,
            mouse_pressed: pressed && !right,
            mouse_down: app.input.mouse_down,
            mouse_released: released && !right,
            mouse_right_pressed: pressed && right,
            mouse_right_down: pressed && right,
            mouse_right_released: released && right,
            ..Default::default()
        });
        ui.horizontal(|ui| ui.flex_width(360.0, |ui| draw(ui, app)));
        ui.end_frame()
    }

    fn setup() -> (Ui, EditorApp, mega_ui::UiOutput) {
        let mut app = EditorApp::default();
        app.project_mut()
            .import_rgba(&[1, 2, 3, 255], 1, 1)
            .unwrap();
        app.project_mut().add_layer();
        app.project_mut().layers[1]
            .pixels
            .write(0, 0, 0, [9, 8, 7, 255]);
        app.project_mut().active_layer = 0;
        app.refresh_image().unwrap();
        app.workspace.select_active_document();
        let mut ui = Ui::new();
        ui.load_builtin_icons();
        let mut output = mega_ui::UiOutput::default();
        for _ in 0..3 {
            output = panel_frame(&mut ui, &mut app, Vec2::ZERO, false, false, false);
        }
        (ui, app, output)
    }

    fn eyes(output: &mega_ui::UiOutput) -> impl Iterator<Item = mega_ui::Rect> + '_ {
        output
            .draw_list
            .iter()
            .filter(|c| {
                c.kind == 0.0
                    && c.uv_min != c.uv_max
                    && (c.rect.width() - 16.0).abs() < 0.1
                    && (c.rect.height() - 16.0).abs() < 0.1
            })
            .map(|c| c.rect)
    }

    fn first_eye(output: &mega_ui::UiOutput) -> mega_ui::Rect {
        eyes(output).next().unwrap()
    }

    #[test]
    fn layer_menu_actions_target_their_row_and_lmb_only_selects() {
        // Copy as Image writes the OS clipboard and is tested through its image preparation.
        for item in [0, 1, 3, 4, 5] {
            let (mut ui, mut app, output) = setup();
            let eye = first_eye(&output);
            let name = Vec2::new(90.0, (eye.min.y + eye.max.y) * 0.5);
            panel_frame(&mut ui, &mut app, name, true, false, false);
            panel_frame(&mut ui, &mut app, name, false, true, false);
            assert_eq!(app.project().active_layer, 1);
            assert!(!ui.context_menu_open());
            app.project_mut().active_layer = 0;
            panel_frame(&mut ui, &mut app, name, true, false, true);
            let output = panel_frame(&mut ui, &mut app, name, false, true, true);
            assert!(ui.context_menu_open());
            assert!(app.layer_drag.is_none());
            let popup = output
                .draw_list
                .iter()
                .find(|c| c.colors == [ui.theme().popup.bg; 4])
                .unwrap()
                .rect;
            let position = Vec2::new(
                popup.min.x + 30.0,
                popup.min.y - 1.0 + 4.0 + ui.theme().metrics.menu_item_h * (item as f32 + 0.5),
            );
            panel_frame(&mut ui, &mut app, position, true, false, false);
            panel_frame(&mut ui, &mut app, position, false, true, false);
            match item {
                0 => {
                    assert_eq!(app.project().active_layer, 0);
                    app.paste_clipboard(&mut ui);
                    assert_eq!(app.project().layers.len(), 3);
                    assert_eq!(app.project().layers[2].pixels.read(0, 0, 0), [9, 8, 7, 255]);
                }
                1 => {
                    assert_eq!(app.project().layers.len(), 3);
                    assert_eq!(app.project().layers[2].name, "Layer 2 copy");
                    app.undo();
                    assert_eq!(app.project().layers.len(), 2);
                }
                3 => {
                    let dialog = app.rename.as_ref().expect("Edit opens rename");
                    assert!(matches!(
                        &dialog.target,
                        crate::app::RenameTarget::Layer { index: 1, .. }
                    ));
                    assert_eq!(app.project().active_layer, 0);
                    assert_eq!(app.project().layers.len(), 2);
                }
                4 => {
                    assert_eq!(app.project().layers.len(), 1);
                    assert_eq!(app.project().layers[0].name, "Layer 1");
                    assert_eq!(app.project().active_layer, 0);
                    app.undo();
                    assert_eq!(app.project().layers.len(), 2);
                    app.redo();
                    assert_eq!(app.project().layers.len(), 1);
                }
                5 => {
                    assert_eq!(app.project().layers.len(), 1);
                    assert_eq!(app.project().layers[0].pixels.read(0, 0, 0), [9, 8, 7, 255]);
                    app.undo();
                    assert_eq!(app.project().layers.len(), 2);
                    app.redo();
                    assert_eq!(app.project().layers.len(), 1);
                }
                _ => unreachable!(),
            }
            assert!(app.layer_drag.is_none());
        }
    }

    #[test]
    fn new_layer_is_above_rows_and_rows_keep_only_visibility_icons() {
        let (mut ui, mut app, output) = setup();
        let eye_rect = first_eye(&output);
        let eye = output
            .draw_list
            .iter()
            .find(|c| c.rect.min == eye_rect.min && c.rect.max == eye_rect.max)
            .unwrap();
        let icons: Vec<_> = output
            .draw_list
            .iter()
            .filter(|c| {
                c.kind == eye.kind
                    && c.tex == eye.tex
                    && c.uv_min != c.uv_max
                    && (c.rect.width() - 16.0).abs() < 0.1
                    && (c.rect.height() - 16.0).abs() < 0.1
            })
            .collect();
        assert_eq!(icons.len(), 2);
        assert!(icons.iter().all(|c| c.rect.max.x < 70.0));
        // Top padding is 8px; the New Layer button occupies the next 28px.
        assert!(36.0 < first_eye(&output).min.y);
        let position = Vec2::new(100.0, 22.0);
        panel_frame(&mut ui, &mut app, position, true, false, false);
        panel_frame(&mut ui, &mut app, position, false, true, false);
        assert_eq!(app.project().layers.len(), 3);
    }

    #[test]
    fn layer_grid_allows_layer_panel_edits() {
        let (mut ui, mut app, output) = setup();
        let checkbox = output
            .draw_list
            .iter()
            .find(|c| {
                c.kind != 0.0
                    && (c.rect.width() - 16.0).abs() < 0.1
                    && (c.rect.height() - 16.0).abs() < 0.1
            })
            .unwrap()
            .rect;
        let toggle = (checkbox.min + checkbox.max) * 0.5;
        panel_frame(&mut ui, &mut app, toggle, true, false, false);
        let output = panel_frame(&mut ui, &mut app, toggle, false, true, false);
        assert!(app.layer_grid_active());

        let mut eyes = eyes(&output);
        let top_eye = eyes.next().unwrap();
        let bottom_eye = eyes.next().unwrap();
        let eye = (top_eye.min + top_eye.max) * 0.5;
        panel_frame(&mut ui, &mut app, eye, true, false, false);
        panel_frame(&mut ui, &mut app, eye, false, true, false);
        assert!(!app.project().layers[1].visible);
        panel_frame(&mut ui, &mut app, eye, true, false, false);
        panel_frame(&mut ui, &mut app, eye, false, true, false);
        assert!(app.project().layers[1].visible);

        let top_layer = app.project().layers[1].id.clone();
        let from = Vec2::new(90.0, eye.y);
        let to = Vec2::new(90.0, (bottom_eye.min.y + bottom_eye.max.y) * 0.5);
        panel_frame(&mut ui, &mut app, from, true, false, false);
        panel_frame(&mut ui, &mut app, to, false, false, false);
        panel_frame(&mut ui, &mut app, to, false, true, false);
        assert_eq!(app.project().layers[0].id, top_layer);

        let new_layer = Vec2::new(100.0, 22.0);
        panel_frame(&mut ui, &mut app, new_layer, true, false, false);
        panel_frame(&mut ui, &mut app, new_layer, false, true, false);
        assert_eq!(app.project().layers.len(), 3);

        panel_frame(&mut ui, &mut app, toggle, true, false, false);
        panel_frame(&mut ui, &mut app, toggle, false, true, false);
        assert!(!app.layer_grid_active());
    }

    #[test]
    fn depth_controls_appear_only_for_animation_documents() {
        let (mut ui, mut app, image) = setup();
        let bottom = |out: &mega_ui::UiOutput| {
            out.draw_list
                .iter()
                .map(|c| c.rect.max.y)
                .fold(0.0f32, f32::max)
        };
        app.project_mut().animation = Some(crate::project::Animation::new(12, 24));
        let animation = panel_frame(&mut ui, &mut app, Vec2::ZERO, false, false, false);
        assert!(bottom(&animation) > bottom(&image) + 20.0);
        app.project_mut().animation = None;
        let image_again = panel_frame(&mut ui, &mut app, Vec2::ZERO, false, false, false);
        assert_eq!(bottom(&image_again), bottom(&image));
    }
}
