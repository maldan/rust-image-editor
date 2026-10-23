use crate::app::EditorApp;
use glam::Vec2;
use mega_ui::{LayoutOpts, Rect, ScrollAxes, Ui};

pub fn visible(app: &EditorApp) -> bool {
    app.project().animation.is_some() && app.session.preview_asset.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_cells_select_camera_for_shared_timeline_commands() {
        let mut app = EditorApp::default();
        app.new_animation();
        let mut ui = Ui::new();
        let mut output = mega_ui::UiOutput::default();
        for _ in 0..3 {
            ui.begin_frame(mega_ui::UiInput { viewport: Vec2::new(1000.0, 280.0), ..Default::default() });
            draw(&mut ui, &mut app);
            output = ui.end_frame();
        }
        let mut cells: Vec<_> = output.draw_list.iter().filter(|c| c.colors[0] == [0.22, 0.21, 0.18, 1.0] && (c.rect.width() - 18.0).abs() < 0.1).collect();
        cells.sort_by(|a, b| a.rect.min.x.total_cmp(&b.rect.min.x));
        let position = (cells[3].rect.min + cells[3].rect.max) * 0.5;
        app.input.mouse_pos = position;
        app.input.mouse_pressed = true;
        app.input.mouse_down = true;
        ui.begin_frame(mega_ui::UiInput { viewport: Vec2::new(1000.0, 280.0), mouse_pos: position, mouse_pressed: true, mouse_down: true, ..Default::default() });
        draw(&mut ui, &mut app);
        ui.end_frame();
        assert!(app.session.camera_selected);
        assert_eq!(app.project().animation.as_ref().unwrap().frame, 3);
        app.animation_drawing(false);
        assert!(app.project().animation.as_ref().unwrap().camera.keys.contains_key(&3));
        assert!(!app.project().layers[0].drawings.contains_key(&3));
    }

    #[test]
    fn held_drawings_have_connectors_and_empty_keys_update_after_drawing() {
        use crate::raster::Raster;
        let mut app = EditorApp::default();
        app.new_animation();
        app.seek_animation(4);
        app.animation_drawing(false);
        let mut ui = Ui::new();
        let mut render = |app: &mut EditorApp| {
            ui.begin_frame(mega_ui::UiInput {
                viewport: Vec2::new(1000.0, 280.0),
                ..Default::default()
            });
            draw(&mut ui, app);
            ui.end_frame()
        };
        render(&mut app);
        let blank = render(&mut app);
        let holes = |out: &mega_ui::UiOutput| {
            out.draw_list
                .iter()
                .filter(|c| {
                    (c.rect.width() - 4.0).abs() < 0.1 && (c.rect.height() - 4.0).abs() < 0.1
                })
                .count()
        };
        assert_eq!(holes(&blank), 2);
        assert!(
            blank
                .draw_list
                .iter()
                .any(|c| c.colors[0] == [0.65, 0.75, 0.86, 1.0])
        );
        app.project_mut().layers[0].pixels.write(0, 0, 0, [255; 4]);
        let drawn = render(&mut app);
        assert_eq!(holes(&drawn), 1);
    }

    #[test]
    fn timeline_cells_select_time_and_layer_without_changing_the_canvas_view() {
        let mut app = EditorApp::default();
        app.new_animation();
        app.add_layer();
        app.project_mut().active_layer = 0;
        let mut ui = Ui::new();
        let mut output = mega_ui::UiOutput::default();
        for _ in 0..3 {
            ui.begin_frame(mega_ui::UiInput {
                viewport: Vec2::new(1000.0, 240.0),
                ..Default::default()
            });
            draw(&mut ui, &mut app);
            output = ui.end_frame();
        }
        let mut cells: Vec<_> = output
            .draw_list
            .iter()
            .filter(|command| {
                command.colors[0] == [0.21, 0.29, 0.38, 1.0]
                    && (command.rect.width() - 18.0).abs() < 0.1
            })
            .collect();
        cells.sort_by(|a, b| {
            a.rect
                .min
                .y
                .total_cmp(&b.rect.min.y)
                .then(a.rect.min.x.total_cmp(&b.rect.min.x))
        });
        // Top row is the second layer; choose its fourth frame.
        let cell = cells[3];
        let position = (cell.rect.min + cell.rect.max) * 0.5;
        app.input.mouse_pos = position;
        app.input.mouse_pressed = true;
        app.input.mouse_down = true;
        ui.begin_frame(mega_ui::UiInput {
            viewport: Vec2::new(1000.0, 240.0),
            mouse_pos: position,
            mouse_pressed: true,
            mouse_down: true,
            ..Default::default()
        });
        draw(&mut ui, &mut app);
        ui.end_frame();
        assert!(app.project().animation.as_ref().unwrap().frame > 0);
        assert_eq!(app.project().active_layer, 1);
        assert!(!app.session.camera_selected);
    }
}

pub fn height(available: f32, scale: f32) -> f32 {
    (280.0 * scale).min(available * 0.5)
}

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let Some(animation) = app.project().animation.clone() else {
        return;
    };
    ui.separator();
    ui.horizontal(|ui| {
        if ui
            .button(
                "anim_play",
                if app.animation_playing() {
                    "Pause"
                } else {
                    "Play"
                },
            )
            .clicked()
        {
            app.toggle_animation();
        }
        if ui.button("anim_prev", "<").clicked() {
            app.seek_animation(animation.frame.saturating_sub(1));
        }
        if ui.button("anim_next", ">").clicked() {
            app.seek_animation((animation.frame + 1).min(animation.length - 1));
        }
        ui.label(&format!("{} / {}", animation.frame + 1, animation.length));
        ui.label("FPS");
        let mut fps = animation.fps as f32;
        if ui.drag_float("anim_fps", &mut fps, 1.0).changed() {
            app.project_mut().animation.as_mut().unwrap().fps =
                fps.round().clamp(1.0, 120.0) as u32;
            app.dirty = true;
        }
        ui.label("Frames");
        let mut length = animation.length as f32;
        if ui.drag_float("anim_length", &mut length, 1.0).changed() {
            let last = app
                .project()
                .layers
                .iter()
                .flat_map(|l| l.drawings.keys())
                .chain(app.project().animation.as_ref().unwrap().camera.keys.keys())
                .copied()
                .max()
                .unwrap_or(0);
            let length = (length.round().clamp(1.0, 1200.0) as u32).max(last + 1);
            app.seek_animation(animation.frame.min(length - 1));
            let animation = app.project_mut().animation.as_mut().unwrap();
            animation.length = length;
            animation.clamp_playback();
            app.dirty = true;
        }
        ui.label(&format!(
            "{:.2} s",
            animation.length as f32 / animation.fps as f32
        ));
    });
    ui.horizontal(|ui| {
        ui.add_enabled(!app.animation_playing(), |ui| {
            if ui.button("anim_blank", "New blank").clicked() {
                app.animation_drawing(false);
            }
            if ui.button("anim_duplicate", "New copy").clicked() {
                app.animation_drawing(true);
            }
            if ui.button("anim_remove", "Remove drawing").clicked() {
                app.remove_animation_drawing();
            }
            if ui.button("anim_layer", "+ Layer").clicked() { app.add_layer(); }
            let hold = app.animation_held_length();
            let at_limit = app
                .project()
                .animation
                .as_ref()
                .is_some_and(|animation| animation.length >= 1200);
            ui.add_enabled(hold > 1, |ui| {
                if ui.button("anim_shorter", "Shorter").clicked() {
                    app.resize_animation_frame(false);
                }
            });
            ui.add_enabled(!at_limit, |ui| {
                if ui.button("anim_longer", "Longer").clicked() {
                    app.resize_animation_frame(true);
                }
            });
            ui.label(&format!("Hold {}", app.animation_held_length()));
        });
    });
    let animation = app.project().animation.clone().unwrap();
    let (loop_from, loop_to) = animation.playback_range();
    ui.horizontal(|ui| {
        ui.label("Loop");
        let mut from = (loop_from + 1) as f32;
        if ui.drag_float("anim_loop_from", &mut from, 1.0).changed() {
            let end = app
                .project()
                .animation
                .as_ref()
                .map(|animation| animation.playback_range().1)
                .unwrap_or(loop_to);
            app.set_animation_loop_start(from.round().clamp(1.0, (end + 1) as f32) as u32 - 1);
        }
        ui.label("to");
        let mut to = (loop_to + 1) as f32;
        if ui.drag_float("anim_loop_to", &mut to, 1.0).changed() {
            let length = app
                .project()
                .animation
                .as_ref()
                .map(|animation| animation.length)
                .unwrap_or(animation.length);
            app.set_animation_loop_end(to.round().clamp(1.0, length as f32) as u32 - 1);
        }
    });
    let scale = ui.scale();
    let size = ui.available_size().max(Vec2::splat(1.0));
    let mut pick = None;
    let mut camera_visibility = None;
    let mut layer_visibility = None;
    let frame = app.project().animation.as_ref().unwrap().frame;
    let length = app.project().animation.as_ref().unwrap().length;
    ui.scroll_area("animation_tracks", size, ScrollAxes::Both, |ui| {
        ui.column_with(
            LayoutOpts {
                spacing: Some(2.0),
                ..Default::default()
            },
            |ui| {
                ui.row_with(
                    LayoutOpts {
                        spacing: Some(1.0),
                        ..Default::default()
                    },
                    |ui| {
                        ui.label_fixed(120.0, "Layers / frames");
                        for f in 0..length {
                            let area =
                                ui.area(&format!("anim_ruler_{f}"), Vec2::new(18.0, 18.0) * scale);
                            if f == frame {
                                ui.fill_rect(area.rect, [0.23, 0.38, 0.56, 1.0]);
                            }
                            ui.text_at_size(
                                area.rect.min + Vec2::splat(2.0 * scale),
                                &(f + 1).to_string(),
                                10.0,
                            );
                            if area.hovered && app.input.mouse_pressed {
                                pick = Some((f, if app.session.camera_selected { None } else { Some(app.project().active_layer) }));
                            }
                        }
                    },
                );
                ui.row_with(LayoutOpts { spacing: Some(1.0), ..Default::default() }, |ui| {
                    ui.flex_width(120.0, |ui| ui.horizontal(|ui| {
                        if ui.icon_button("timeline_camera_visibility", if animation.camera.visible { "visibility" } else { "visibility_off" }, animation.camera.visible) {
                            camera_visibility = Some(!animation.camera.visible);
                        }
                        let name = ui.area("timeline_camera_name", Vec2::new(84.0, 18.0) * scale);
                        ui.text_at(name.rect.min + Vec2::new(0.0, 2.0 * scale), "Camera");
                        if name.hovered && app.input.mouse_pressed { pick = Some((frame, None)); }
                    }));
                    for f in 0..length {
                        let area = ui.area(&format!("camera_cell_{f}"), Vec2::new(18.0, 18.0) * scale);
                        ui.fill_rect(area.rect, if app.session.camera_selected && f == frame { [0.45, 0.38, 0.2, 1.0] } else { [0.22, 0.21, 0.18, 1.0] });
                        let start = animation.camera.keys.range(..=f).next_back().map_or(0, |(&key, _)| key);
                        let center = (area.rect.min + area.rect.max) * 0.5;
                        let left = if start == f { center.x } else { area.rect.min.x - scale };
                        let right = if f + 1 == length || animation.camera.keys.contains_key(&(f + 1)) { area.rect.max.x - 3.0 * scale } else { area.rect.max.x + scale };
                        ui.line(Vec2::new(left, center.y), Vec2::new(right, center.y), 2.0 * scale, [0.85, 0.65, 0.3, 1.0]);
                        if animation.camera.keys.contains_key(&f) {
                            let min = area.rect.min + Vec2::splat(6.0 * scale);
                            ui.fill_rect(Rect::from_min_size(min, Vec2::splat(6.0 * scale)), [0.95, 0.73, 0.3, 1.0]);
                        }
                        if area.hovered && app.input.mouse_pressed { pick = Some((f, None)); }
                    }
                });
                for (index, layer) in app.project().layers.iter().enumerate().rev() {
                    ui.row_with(
                        LayoutOpts {
                            spacing: Some(1.0),
                            ..Default::default()
                        },
                        |ui| {
                            ui.flex_width(120.0, |ui| ui.horizontal(|ui| {
                                if ui.icon_button(&format!("timeline_layer_visibility_{index}"), if layer.visible { "visibility" } else { "visibility_off" }, layer.visible) {
                                    layer_visibility = Some((index, !layer.visible));
                                }
                                ui.label(&layer.name);
                            }));
                            for f in 0..length {
                                let area = ui.area(
                                    &format!("anim_cell_{index}_{f}"),
                                    Vec2::new(18.0, 18.0) * scale,
                                );
                                let start = layer
                                    .drawings
                                    .range(..=f)
                                    .next_back()
                                    .map(|(&key, _)| key);
                                let selected = !app.session.camera_selected && index == app.project().active_layer && f == frame;
                                let color = if selected {
                                    [0.34, 0.58, 0.84, 1.0]
                                } else if start.is_some_and(|start| start % 2 == 1) {
                                    [0.26, 0.34, 0.43, 1.0]
                                } else {
                                    [0.21, 0.29, 0.38, 1.0]
                                };
                                ui.fill_rect(area.rect, color);
                                if let Some(start) = start {
                                // A continuous spine joins held frames to their starting drawing.
                                let center = (area.rect.min + area.rect.max) * 0.5;
                                let next_key = layer.drawings.contains_key(&(f + 1));
                                let left = if start == f {
                                    center.x
                                } else {
                                    area.rect.min.x - scale
                                };
                                let right = if f + 1 == length || next_key {
                                    area.rect.max.x - 3.0 * scale
                                } else {
                                    area.rect.max.x + scale
                                };
                                ui.line(
                                    Vec2::new(left, center.y),
                                    Vec2::new(right, center.y),
                                    2.0 * scale,
                                    [0.65, 0.75, 0.86, 1.0],
                                );
                                if start == f {
                                    // The editable buffer is authoritative until the next seek/save.
                                    let live_start = layer
                                        .drawings
                                        .range(..=frame)
                                        .next_back()
                                        .map(|(&key, _)| key);
                                    let pixels = if live_start == Some(start) {
                                        Some(&layer.pixels)
                                    } else {
                                        layer.drawings.get(&start)
                                    };
                                    let empty = pixels.is_none_or(|pixels| {
                                        !pixels.tiles.values().any(|tile| {
                                            tile.chunks_exact(4).any(|pixel| pixel[3] != 0)
                                        })
                                    });
                                    let min = area.rect.min + Vec2::splat(6.0 * scale);
                                    ui.fill_rect(
                                        Rect {
                                            min,
                                            max: min + Vec2::splat(6.0 * scale),
                                        },
                                        [0.8, 0.85, 0.92, 1.0],
                                    );
                                    if empty {
                                        ui.fill_rect(
                                            Rect {
                                                min: min + Vec2::splat(scale),
                                                max: min + Vec2::splat(5.0 * scale),
                                            },
                                            color,
                                        );
                                    }
                                }
                                }
                                if f == frame {
                                    ui.fill_rect(
                                        Rect {
                                            min: area.rect.min,
                                            max: Vec2::new(
                                                area.rect.min.x + 2.0 * scale,
                                                area.rect.max.y,
                                            ),
                                        },
                                        [0.9, 0.65, 0.3, 1.0],
                                    );
                                }
                                if area.hovered && app.input.mouse_pressed {
                                    pick = Some((f, Some(index)));
                                }
                            }
                        },
                    );
                }
            },
        );
    });
    if let Some(visible) = camera_visibility { app.set_camera_visible(visible); }
    if let Some((index, visible)) = layer_visibility { app.set_layer_visible(index, visible); }
    if let Some((frame, layer)) = pick {
        if app.animation_playing() {
            app.toggle_animation();
        }
        app.seek_animation(frame);
        app.session.camera_selected = layer.is_none();
        if let Some(index) = layer { app.project_mut().active_layer = index; }
    }
}
