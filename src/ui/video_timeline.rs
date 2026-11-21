use crate::app::EditorApp;
use crate::video_document::{CanvasDrag, CanvasDragKind, Clip, ClipDrag, ClipEdit, ClipRange, IMAGE_CLIP_DURATION, PREVIEW_TEXTURE, TrackLayout};
use crate::video_document_file::AssetType;
use crate::workspace::ThumbDrag;
use glam::Vec2;
use mega_ui::{CursorIcon, Rect, Ui};

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let scale = ui.scale();
    let available = ui.available_size().max(Vec2::splat(1.0));
    let row_height = 48.0 * scale;
    let timeline_chrome = 98.0 * scale;
    let visible_limit = ((available.y * 0.55 - timeline_chrome) / row_height).max(1.0) as usize;
    let visible_lanes = app.video_editor.current().unwrap().lane_count.min(visible_limit);
    let timeline_height = timeline_chrome + visible_lanes as f32 * row_height;
    let preview = ui.surface(Vec2::new(available.x, (available.y - timeline_height).max(1.0)), [0.065, 0.071, 0.085, 1.0]);
    draw_preview(ui, app, preview);
    ui.separator();
    toolbar(ui, app);
    let width = ui.available_size().x.max(1.0);
    let ruler = ui.area("video_ruler", Vec2::new(width, 25.0 * scale));
    let gutter = (100.0 * scale).min(width * 0.4);
    let row_height = 48.0 * scale;
    let doc = app.video_editor.current().unwrap();
    let top_lane = doc.lane_count - 1 - doc.lane_scroll.min(doc.lane_count - visible_lanes);
    let no_gap = mega_ui::LayoutOpts { spacing: Some(0.0), ..Default::default() };
    let mut layout = None;
    let mut track_hovered = false;
    ui.column_with(no_gap, |ui| {
        for row in 0..visible_lanes {
            let lane = top_lane - row;
            ui.row_with(no_gap, |ui| {
                ui.flex_width(gutter, |ui| {
                    ui.space(13.0);
                    ui.row_with(no_gap, |ui| {
                        let visible = app.video_editor.current().unwrap().lane_visible(lane);
                        ui.label_fixed((gutter / scale - 28.0).max(1.0), &format!("V{}", lane + 1));
                        let icon = if visible { "visibility" } else { "visibility_off" };
                        if gutter >= 34.0 * scale
                            && ui.icon_button(&format!("video_lane_eye_{lane}"), icon, visible)
                        {
                            app.video_editor.toggle_lane(lane);
                            ui.request_repaint();
                        }
                    });
                });
                let area = ui.area(&format!("video_track_{lane}"), Vec2::new(width - gutter, row_height));
                track_hovered |= area.hovered;
                layout.get_or_insert_with(|| TrackLayout {
                    rect: Rect::from_min_size(area.rect.min, Vec2::new(area.rect.width(), row_height * visible_lanes as f32)),
                    top_lane,
                    visible_lanes,
                    row_height,
                });
            });
        }
    });
    let mut layout = layout.unwrap();
    let ruler_rect = Rect { min: Vec2::new(layout.rect.min.x, ruler.rect.min.y), max: ruler.rect.max };
    let ruler_hovered = ruler.hovered && ruler_rect.contains(app.input.mouse_pos);
    interact(ui, app, &mut layout, ruler_hovered, track_hovered);
    app.video_editor.track = ui.enabled().then_some(layout);
    paint_timeline(ui, app, ruler_rect, layout);
}

fn draw_preview(ui: &mut Ui, app: &mut EditorApp, preview: Rect) {
    let input = &app.input;
    let transforming = app.video_editor.canvas_drag.is_some();
    let doc = app.video_editor.current_mut().unwrap();
    let center = (preview.min + preview.max) * 0.5;
    if !transforming && ui.enabled() && ui.rect_hovered(preview) {
        if input.mouse_middle_down {
            doc.preview_pan += input.mouse_delta;
            ui.set_mouse_cursor(CursorIcon::Move);
        }
        if input.scroll_delta.y.abs() > 0.01 {
            let old_zoom = doc.preview_zoom;
            let factor = (input.scroll_delta.y * 0.002).exp();
            doc.preview_zoom = (old_zoom * factor).clamp(0.05, 32.0);
            let anchor = input.mouse_pos - center - doc.preview_pan;
            doc.preview_pan -= anchor * (doc.preview_zoom / old_zoom - 1.0);
        }
    }

    let fitted = fit(preview.inset(12.0 * ui.scale()), doc.width as f32 / doc.height as f32);
    let half_size = (fitted.max - fitted.min) * (0.5 * doc.preview_zoom);
    let center = center + doc.preview_pan;
    let canvas = Rect { min: center - half_size, max: center + half_size };
    edit_canvas(ui, app, canvas, preview);
    let mut opacities = Vec::new();
    if let Some(visible_canvas) = canvas.intersect(preview) {
        let doc = app.video_editor.current().unwrap();
        ui.fill_rect(visible_canvas, doc.background);
        let layers = app.video_editor.previews.iter().filter_map(|track| track.frame());
        for (slot, (index, _)) in layers.enumerate() {
            let clip = &doc.clips[index];
            opacities.push(clip.effects.frame_at(doc.playhead - clip.range.start).opacity);
            let Some(frame) = preview_clip_rect(doc, clip, canvas) else { continue };
            if let Some(visible) = frame.intersect(visible_canvas) {
                let size = frame.max - frame.min;
                let uv_min = (visible.min - frame.min) / size;
                let uv_max = (visible.max - frame.min) / size;
                ui.texture_rect(PREVIEW_TEXTURE + slot as u32, visible, uv_min.to_array(), uv_max.to_array());
            }
        }
        if let Some((_, clip)) = doc.active_clips().find(|(index, _)| Some(*index) == doc.selected)
            && let Some(frame) = transform_frame_rect(doc, clip, canvas)
        {
            if let Some(effected) = preview_clip_rect(doc, clip, canvas)
                && ((effected.min - frame.min).abs().max_element() >= 0.5
                    || (effected.max - frame.max).abs().max_element() >= 0.5)
            {
                draw_clip_outline(ui, effected, preview, [1.0, 0.49, 0.16, 1.0]);
            }
            let color = [0.92, 0.95, 1.0, 1.0];
            draw_clip_outline(ui, frame, preview, color);
            if transform_target(doc, clip).is_some() {
                let handle = 7.0 * ui.scale();
                for point in transform_handles(frame) {
                    let rect = Rect::from_min_size(point - Vec2::splat(handle * 0.5), Vec2::splat(handle));
                    if let Some(rect) = rect.intersect(preview) {
                        ui.fill_round(rect, ui.scale(), color);
                    }
                }
            }
        }
    }
    app.video_editor.preview_opacity = opacities;
}

fn draw_clip_outline(ui: &mut Ui, frame: Rect, preview: Rect, color: [f32; 4]) {
    let corners = [
        frame.min,
        Vec2::new(frame.max.x, frame.min.y),
        frame.max,
        Vec2::new(frame.min.x, frame.max.y),
    ];
    for (from, to) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
        let edge = Rect {
            min: from.min(to) - Vec2::splat(0.5),
            max: from.max(to) + Vec2::splat(0.5),
        };
        if let Some(visible) = edge.intersect(preview) {
            ui.fill_rect(visible, color);
        }
    }
}

fn base_clip_rect(doc: &crate::video_document::VideoDocument, clip: &Clip, canvas: Rect) -> Option<Rect> {
    let canvas_size = canvas.max - canvas.min;
    let size = clip.size_in_document()? * (canvas.width() / doc.width as f32);
    let center = canvas.min + clip.position * canvas_size;
    Some(Rect::from_min_size(center - size * 0.5, size))
}

fn preview_clip_rect(doc: &crate::video_document::VideoDocument, clip: &Clip, canvas: Rect) -> Option<Rect> {
    let effect = clip.effects.frame_at(doc.playhead - clip.range.start);
    Some(apply_transform(base_clip_rect(doc, clip, canvas)?, canvas, effect.offset, effect.scale))
}

fn transform_frame_rect(doc: &crate::video_document::VideoDocument, clip: &Clip, canvas: Rect) -> Option<Rect> {
    let (offset, scale) = clip.effects.animated_transform(doc.playhead - clip.range.start, doc.fps);
    Some(apply_transform(base_clip_rect(doc, clip, canvas)?, canvas, offset, scale))
}

fn apply_transform(base: Rect, canvas: Rect, offset: Vec2, scale: f32) -> Rect {
    let size = (base.max - base.min) * scale;
    let center = (base.min + base.max) * 0.5 + offset * (canvas.max - canvas.min);
    Rect::from_min_size(center - size * 0.5, size)
}

fn transform_target(
    doc: &crate::video_document::VideoDocument,
    clip: &Clip,
) -> Option<(Option<usize>, crate::video_document::ClipTransform)> {
    let Some(effect) = clip.effects.transform_animation().filter(|effect| !effect.keys.is_empty()) else {
        return Some((None, clip.transform()));
    };
    let index = effect.key_at(doc.playhead - clip.range.start, doc.fps)?;
    let key = effect.keys[index];
    Some((Some(index), crate::video_document::ClipTransform {
        position: clip.position + Vec2::from(key.offset),
        scale_percent: clip.scale_percent * key.scale,
    }))
}

fn transform_handles(rect: Rect) -> [Vec2; 8] {
    let center = (rect.min + rect.max) * 0.5;
    [
        rect.min,
        Vec2::new(center.x, rect.min.y),
        Vec2::new(rect.max.x, rect.min.y),
        Vec2::new(rect.max.x, center.y),
        rect.max,
        Vec2::new(center.x, rect.max.y),
        Vec2::new(rect.min.x, rect.max.y),
        Vec2::new(rect.min.x, center.y),
    ]
}

fn edit_canvas(ui: &mut Ui, app: &mut EditorApp, canvas: Rect, preview: Rect) {
    let input = &app.input;
    let editor = &mut app.video_editor;
    if !ui.enabled() {
        editor.canvas_drag = None;
        return;
    }
    let canvas_size = canvas.max - canvas.min;
    if canvas_size.min_element() <= 0.0 {
        return;
    }
    let mouse = (input.mouse_pos - canvas.min) / canvas_size;
    if let Some(drag) = editor.canvas_drag.take() {
        if let Some(doc) = editor.current_mut() && let Some(clip) = doc.clips.get_mut(drag.index) {
            let mut transform = drag.transform;
            let dragging = input.mouse_down && !input.mouse_released && !input.key_escape;
            if !input.key_escape && (input.mouse_down || input.mouse_released) {
                let delta = mouse - drag.mouse;
                match drag.kind {
                    CanvasDragKind::Move => transform.position += delta,
                    CanvasDragKind::Scale { direction } => {
                        // Project in screen pixels so uniform scaling behaves the same at any canvas aspect.
                        let direction = direction * canvas_size;
                        let factor = 1.0 + (delta * canvas_size).dot(direction) / direction.length_squared();
                        transform.scale_percent = (drag.transform.scale_percent * factor).clamp(0.1, 10_000.0);
                    }
                }
            }
            if input.key_escape || input.mouse_down || input.mouse_released {
                if let Some(key) = drag.key {
                    clip.effects.edit_transform_key(
                        key, transform.position - clip.position, transform.scale_percent / clip.scale_percent,
                    );
                } else {
                    clip.set_transform(transform);
                }
            }
            if drag.key.is_some() && !dragging {
                clip.effects.bake(clip.range.duration(), doc.fps);
            }
            if dragging {
                editor.canvas_drag = Some(drag);
            }
            ui.request_repaint();
        }
        return;
    }
    if !ui.rect_hovered(preview) || app.workspace.asset_drag.is_some() {
        return;
    }
    let doc = editor.current().unwrap();
    let selected_handle = doc.active_clips()
        .find(|(index, _)| Some(*index) == doc.selected)
        .and_then(|(index, clip)| {
            transform_target(doc, clip)?;
            let rect = transform_frame_rect(doc, clip, canvas)?;
            let handles = transform_handles(rect);
            let handle = handles.iter().position(|point| point.distance(input.mouse_pos) <= 6.0 * ui.scale())?;
            let direction = handles[handle] - (rect.min + rect.max) * 0.5;
            (direction.length_squared() > 0.0).then_some((index, CanvasDragKind::Scale {
                direction: direction / canvas_size,
            }))
        });
    let hit = selected_handle.or_else(|| {
        if !canvas.contains(input.mouse_pos) {
            return None;
        }
        doc.active_clips().filter(|(_, clip)| preview_clip_rect(doc, clip, canvas).is_some_and(|rect| rect.contains(input.mouse_pos)))
            .max_by_key(|(_, clip)| clip.range.lane)
            .map(|(index, _)| (index, CanvasDragKind::Move))
    });
    if let Some((_, kind)) = &hit {
        ui.set_mouse_cursor(match kind {
            CanvasDragKind::Move => CursorIcon::Move,
            CanvasDragKind::Scale { direction } if direction.x == 0.0 => CursorIcon::ResizeNs,
            CanvasDragKind::Scale { direction } if direction.y == 0.0 => CursorIcon::ResizeEw,
            CanvasDragKind::Scale { .. } => CursorIcon::ResizeNwse,
        });
    }
    if input.mouse_pressed && !input.mouse_middle_down {
        if let Some((index, kind)) = hit {
            let drag = transform_target(doc, &doc.clips[index])
                .map(|(key, transform)| CanvasDrag { index, key, mouse, transform, kind });
            editor.set_playing(false);
            editor.current_mut().unwrap().selected = Some(index);
            editor.canvas_drag = drag;
        } else {
            editor.current_mut().unwrap().selected = None;
        }
    }
}

fn fit(rect: Rect, aspect: f32) -> Rect {
    let available = (rect.max - rect.min).max(Vec2::ZERO);
    let height = available.y.min(available.x / aspect);
    let size = Vec2::new(height * aspect, height);
    Rect::from_min_size((rect.min + rect.max - size) * 0.5, size)
}

fn toolbar(ui: &mut Ui, app: &mut EditorApp) {
    let editor = &mut app.video_editor;
    ui.horizontal(|ui| {
        if ui.button("sequence_play", if editor.playing { "Pause" } else { "Play" }).clicked() {
            editor.set_playing(!editor.playing);
        }
        let doc = editor.current().unwrap();
        let split = doc.selected.is_some_and(|index| {
            let range = doc.clips[index].range;
            doc.playhead > range.start && doc.playhead < range.end()
        });
        let selected = doc.selected.is_some();
        let time = format!("{} / {}", timecode(doc.playhead, doc.fps), timecode(doc.duration, doc.fps));
        ui.add_enabled(split, |ui| {
            if ui.button("sequence_split", "Split").clicked() {
                editor.split();
            }
        });
        ui.add_enabled(selected, |ui| {
            if ui.button("sequence_remove", "Remove").clicked() {
                editor.remove_selected();
            }
        });
        if ui.button("sequence_add_track", "+ Track").clicked() {
            let doc = editor.current_mut().unwrap();
            doc.lane_count += 1;
            doc.lane_scroll = 0;
            ui.request_repaint();
        }
        ui.label(&time);
    });
    if ui.enabled() && app.input.key_delete && editor.drag.is_none() && editor.canvas_drag.is_none() {
        editor.remove_selected();
    }
}

fn interact(ui: &mut Ui, app: &mut EditorApp, layout: &mut TrackLayout, ruler_hovered: bool, track_hovered: bool) {
    let editor = &mut app.video_editor;
    let track = layout.rect;
    let input = &app.input;
    let doc = editor.current_mut().unwrap();
    if ui.enabled() && track_hovered && input.key_shift && input.scroll_delta.y.is_finite() {
        if input.scroll_delta.y > 0.0 {
            doc.lane_scroll = doc.lane_scroll.saturating_sub(1);
        } else if input.scroll_delta.y < 0.0 {
            doc.lane_scroll = doc.lane_scroll.saturating_add(1);
        }
    }
    doc.lane_scroll = doc.lane_scroll.min(doc.lane_count - layout.visible_lanes);
    if layout.top_lane != doc.lane_count - 1 - doc.lane_scroll {
        ui.request_repaint();
    }
    if !ui.enabled() {
        editor.drag = None;
        editor.scrubbing = false;
        return;
    }
    let lane = track_hovered.then(|| layout.lane_at(input.mouse_pos)).flatten();
    let over_timeline = ruler_hovered || lane.is_some();
    if over_timeline && (input.key_left || input.key_right) {
        let doc = editor.current().unwrap();
        let fps = f64::from(doc.fps);
        let step = if input.key_left { -1.0 } else { 1.0 };
        let position = ((doc.playhead * fps).round() + step) / fps;
        editor.seek(position);
    }
    let playing = editor.playing;
    let doc = editor.current_mut().unwrap();
    let mut density = f64::from(doc.pixels_per_second * ui.scale());
    let offset = f64::from(input.mouse_pos.x - track.min.x);
    if over_timeline && !input.scroll_delta.is_finite() {
        return;
    }
    if over_timeline {
        if !input.key_shift && input.scroll_delta.y != 0.0 {
            let under_cursor = doc.view_start + offset / density;
            let factor = (input.scroll_delta.y * 0.004).exp();
            doc.pixels_per_second = (doc.pixels_per_second * factor).clamp(0.04, 1200.0);
            density = f64::from(doc.pixels_per_second * ui.scale());
            doc.view_start = under_cursor - offset / density;
        }
        doc.view_start -= f64::from(input.scroll_delta.x) / density;
        if input.mouse_middle_down {
            doc.view_start -= f64::from(input.mouse_delta.x) / density;
        }
    }
    let visible_duration = f64::from(track.width()) / density;
    if playing && input.scroll_delta == Vec2::ZERO && !input.mouse_middle_down
        && doc.playhead >= doc.view_start + visible_duration
    {
        doc.view_start = doc.playhead;
    }
    doc.view_start = doc.view_start.clamp(0.0, (doc.duration - visible_duration).max(0.0));
    let time = doc.view_start + offset / density;
    let radius = f64::from(7.0 * ui.scale()) / density;
    let playhead_x = (doc.playhead - doc.view_start) * density;
    let on_playhead = lane.is_some()
        && (0.0..=f64::from(track.width())).contains(&playhead_x)
        && (offset - playhead_x).abs() <= f64::from(3.0 * ui.scale());
    let hovered_clip = lane.filter(|_| !on_playhead && app.workspace.asset_drag.is_none()).and_then(|lane| {
        let row = layout.lane_rect(lane)?;
        let inset = 6.0 * ui.scale();
        if input.mouse_pos.y < row.min.y + inset || input.mouse_pos.y >= row.max.y - inset {
            return None;
        }
        doc.clips.iter().enumerate().find_map(|(index, clip)| {
            let range = clip.range;
            if range.lane != lane || time < range.start || time >= range.end() {
                return None;
            }
            let edge = radius.min((range.end() - range.start) * 0.28);
            let edit = if time - range.start < edge {
                ClipEdit::TrimStart
            } else if range.end() - time < edge {
                ClipEdit::TrimEnd
            } else {
                ClipEdit::Move
            };
            Some((index, edit))
        })
    });
    if let Some((_, edit)) = hovered_clip {
        ui.set_mouse_cursor(match edit {
            ClipEdit::Move => CursorIcon::Move,
            _ => CursorIcon::ResizeEw,
        });
    }
    if input.key_escape {
        doc.selected = None;
        editor.drag = None;
        editor.scrubbing = false;
        return;
    }
    if input.mouse_pressed && app.workspace.asset_drag.is_none() {
        if let Some((index, edit)) = hovered_clip {
            doc.selected = Some(index);
            editor.set_playing(false);
            editor.drag = Some(ClipDrag { index, edit, anchor: time, candidate: None });
        } else if ruler_hovered || track_hovered {
            if !ruler_hovered && !on_playhead {
                doc.selected = None;
            }
            editor.scrubbing = over_timeline;
        }
    }
    if let Some(drag) = &mut editor.drag {
        let doc = &editor.documents[editor.active.unwrap()];
        drag.candidate = lane.and_then(|lane| {
            let changed_lane = lane != doc.clips[drag.index].range.lane && matches!(drag.edit, ClipEdit::Move);
            let moved = (time - drag.anchor).abs() * density > f64::from(2.0 * ui.scale());
            (changed_lane || moved).then(|| doc.edit_range(drag, lane, time, radius)).flatten()
        });
        if input.mouse_released || !input.mouse_down {
            let drag = editor.drag.take().unwrap();
            if let Some(range) = drag.candidate.filter(|_| input.mouse_released) {
                editor.apply_edit(drag.index, range);
            }
        }
    } else if editor.scrubbing {
        if (input.mouse_down || input.mouse_released)
            && (input.mouse_pressed || input.mouse_delta.x != 0.0)
        {
            let doc = editor.current().unwrap();
            let fps = f64::from(doc.fps);
            let position = ((time * fps).round() / fps).clamp(0.0, doc.duration);
            if position != editor.current().unwrap().playhead {
                editor.seek(position);
            }
        }
        editor.scrubbing = input.mouse_down;
    }
}

fn paint_timeline(ui: &mut Ui, app: &EditorApp, ruler: Rect, layout: TrackLayout) {
    let editor = &app.video_editor;
    let doc = editor.current().unwrap();
    let track = layout.rect;
    let scale = ui.scale();
    let density = f64::from(doc.pixels_per_second * scale);
    let x = |time: f64| track.min.x + ((time - doc.view_start) * density) as f32;
    ui.fill_rect(ruler, [0.078, 0.087, 0.105, 1.0]);
    ui.fill_rect(track, [0.040, 0.047, 0.059, 1.0]);
    let end_x = x(doc.duration).clamp(track.min.x, track.max.x);
    ui.fill_rect(Rect { min: track.min, max: Vec2::new(end_x, track.max.y) }, [0.064, 0.074, 0.091, 1.0]);
    let step = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 300.0, 900.0, 3600.0, 7200.0]
        .into_iter().find(|step| step * density >= f64::from(70.0 * scale)).unwrap_or(14_400.0);
    let mut time = (doc.view_start / step).ceil() * step;
    while x(time) < ruler.max.x {
        let x = x(time);
        ui.line(Vec2::new(x, ruler.max.y - 6.0 * scale), Vec2::new(x, track.max.y), scale, [0.14, 0.16, 0.20, 1.0]);
        if x + 58.0 * scale < ruler.max.x {
            ui.text_at_size(Vec2::new(x + 3.0 * scale, ruler.min.y + 3.0 * scale), &ruler_label(time, step), 11.0 * scale);
        }
        time += step;
    }
    let clip_box = |range: ClipRange| {
        let row = layout.lane_rect(range.lane)?;
        Rect {
            min: Vec2::new(x(range.start), row.min.y + 6.0 * scale),
            max: Vec2::new(x(range.end()), row.max.y - 6.0 * scale),
        }.intersect(row)
    };
    for (index, clip) in doc.clips.iter().enumerate() {
        if let Some(rect) = clip_box(clip.range) {
            let selected = doc.selected == Some(index);
            let visible = doc.lane_visible(clip.range.lane);
            let (normal, active, hidden, outline) = match clip.source.asset_type() {
                AssetType::Video => (
                    [0.16, 0.30, 0.43, 1.0], [0.19, 0.42, 0.62, 1.0],
                    [0.13, 0.16, 0.20, 1.0], [0.32, 0.52, 0.72, 1.0],
                ),
                AssetType::Image => (
                    [0.41, 0.25, 0.08, 1.0], [0.54, 0.34, 0.12, 1.0],
                    [0.23, 0.18, 0.12, 1.0], [0.76, 0.55, 0.26, 1.0],
                ),
            };
            let color = if !visible { hidden } else if selected { active } else { normal };
            let border = if selected { [0.96, 0.97, 1.0, 1.0] } else { outline };
            let stroke = if selected { 1.6 * scale } else { scale };
            let radius = (5.0 * scale).min(rect.width() * 0.5);
            ui.fill_round(rect, radius, border);
            if rect.width() > 2.0 * stroke {
                ui.fill_round(rect.inset(stroke), (radius - stroke).max(0.0), color);
            }
            if selected && rect.width() >= 10.0 * scale {
                let y = (rect.min.y + rect.max.y) * 0.5 - 7.0 * scale;
                for (time, offset) in [(clip.range.start, 0.0), (clip.range.end(), 4.0 * scale)] {
                    let handle = Rect::from_min_size(
                        Vec2::new(x(time) - offset, y),
                        Vec2::new(4.0, 14.0) * scale,
                    );
                    if let Some(visible) = handle.intersect(rect) {
                        ui.fill_round(visible, scale, border);
                    }
                }
            }
            let max_chars = ((rect.width() / scale - 14.0) / 8.0).max(0.0) as usize;
            if max_chars >= 4 {
                let name = clip.source.name();
                let mut label: String = name.chars().take(max_chars).collect();
                if name.chars().count() > max_chars {
                    label.pop();
                    label.push('…');
                }
                ui.text_at_size(rect.min + Vec2::new(7.0, 8.0) * scale, &label, 12.0 * scale);
            }
        }
    }
    let image_drop = [0.82, 0.55, 0.22, 0.65];
    if let Some(drag) = &editor.drag && let Some(rect) = drag.candidate.and_then(clip_box) {
        let color = match doc.clips[drag.index].source.asset_type() {
            AssetType::Video => [0.35, 0.68, 0.89, 0.65],
            AssetType::Image => image_drop,
        };
        ui.fill_round(rect, 4.0 * scale, color);
    }
    if app.workspace.asset_dragging
        && let Some((lane, time, radius)) = layout.drop_position(doc, app.input.mouse_pos, scale)
    {
        let candidate = match app.workspace.asset_drag {
            Some(ThumbDrag::Video(index)) => app.workspace.videos.get(index)
                .and_then(|asset| asset.info)
                .map(|info| (info.duration, [0.29, 0.72, 0.54, 0.65])),
            Some(ThumbDrag::Image(index)) => app.workspace.assets.get(index)
                .map(|_| (IMAGE_CLIP_DURATION, image_drop)),
            _ => None,
        };
        if let Some((duration, color)) = candidate
            && let Some(rect) = doc.drop_range(duration, lane, time, radius).and_then(clip_box)
        {
            ui.fill_round(rect, 4.0 * scale, color);
        }
    }
    if doc.clips.is_empty() && track.width() > 180.0 * scale {
        ui.text_at_size(track.min + Vec2::new(16.0, 22.0) * scale, "Drag an image or video here", 12.0 * scale);
    }
    let playhead = x(doc.playhead);
    if playhead >= track.min.x && playhead <= track.max.x {
        let color = [0.99, 0.71, 0.28, 1.0];
        let top = ruler.max.y - 14.0 * scale;
        ui.line(Vec2::new(playhead, top), Vec2::new(playhead, track.max.y), 1.5 * scale, color);
        let head = Rect::from_min_size(Vec2::new(playhead - 6.0 * scale, top), Vec2::new(12.0, 8.0) * scale);
        ui.fill_round(head, 2.0 * scale, color);
        for row in 0..5 {
            let half_width = (5 - row) as f32 * scale;
            ui.fill_rect(
                Rect::from_min_size(Vec2::new(playhead - half_width, head.max.y + row as f32 * scale), Vec2::new(half_width * 2.0, scale)),
                color,
            );
        }
    }
}

pub fn properties(ui: &mut Ui, app: &mut EditorApp) {
    let Some(doc) = app.video_editor.current_mut() else { return };
    if let Some(clip) = doc.selected.and_then(|index| doc.clips.get_mut(index)) {
        ui.label("Clip position (% of canvas)");
        let mut position = clip.position * 100.0;
        if ui.vec2("sequence_clip_position", &mut position, 0.5, Vec2::splat(50.0)).changed()
            && position.is_finite()
        {
            clip.position = position / 100.0;
            ui.request_repaint();
        }
        ui.label("Clip scale (%)");
        let mut scale = clip.scale_percent;
        if ui.drag_float("sequence_clip_scale", &mut scale, 1.0).changed() && scale.is_finite() {
            clip.scale_percent = scale.clamp(0.1, 10_000.0);
            ui.request_repaint();
        }
        if let Some(size) = clip.size_in_document() {
            ui.label(&format!("{:.0} × {:.0} px", size.x, size.y));
        }
        let gap = 4.0 * ui.scale();
        let width = ((ui.available_size().x - gap) * 0.5).max(1.0);
        ui.row_with(mega_ui::LayoutOpts { spacing: Some(gap), ..Default::default() }, |ui| {
            ui.flex_width(width, |ui| {
                if ui.button("copy_video_transform", "Copy Transform").clicked() {
                    app.video_transform_clipboard = Some(clip.transform());
                    app.status = "Clip transform copied".into();
                }
            });
            ui.flex_width(width, |ui| {
                ui.add_enabled(app.video_transform_clipboard.is_some(), |ui| {
                    if ui.button("paste_video_transform", "Paste Transform").clicked()
                        && let Some(transform) = app.video_transform_clipboard
                    {
                        clip.set_transform(transform);
                        app.status = "Clip transform pasted".into();
                        ui.request_repaint();
                    }
                });
            });
        });
        ui.separator();
    }
    ui.label("Canvas background");
    if ui.color_edit("video_canvas_background", &mut doc.background).changed() {
        ui.request_repaint();
    }
    let mut width = doc.width;
    let mut height = doc.height;
    let mut fps = doc.fps;
    let mut duration = doc.duration as f32;
    let gap = 6.0 * ui.scale();
    let column = ((ui.available_size().x - gap) * 0.5).max(1.0);
    let row = mega_ui::LayoutOpts { spacing: Some(gap), ..Default::default() };
    ui.row_with(row, |ui| {
        ui.flex_width(column, |ui| {
            ui.label("Width (px)");
            if ui.drag_int("sequence_width", &mut width, 1).changed() {
                doc.width = width.clamp(1, 16384);
            }
        });
        ui.flex_width(column, |ui| {
            ui.label("Height (px)");
            if ui.drag_int("sequence_height", &mut height, 1).changed() {
                doc.height = height.clamp(1, 16384);
            }
        });
    });
    let mut fps_changed = false;
    let mut duration_changed = false;
    ui.row_with(row, |ui| {
        ui.flex_width(column, |ui| {
            ui.label("FPS");
            fps_changed = ui.drag_int("sequence_fps", &mut fps, 1).changed();
        });
        ui.flex_width(column, |ui| {
            ui.label("Duration (s)");
            duration_changed = ui.drag_float("sequence_duration", &mut duration, 0.1).changed();
        });
    });
    if fps_changed {
        doc.fps = fps.clamp(1, 120);
        for clip in &mut doc.clips {
            clip.effects.bake(clip.range.duration(), doc.fps);
        }
    }
    if duration_changed && duration.is_finite() {
        doc.duration = f64::from(duration.clamp(1.0 / doc.fps as f32, 86400.0)).max(doc.content_end());
        let position = doc.playhead.min(doc.duration);
        app.video_editor.seek(position);
    } else if fps_changed {
        app.video_editor.refresh_preview(false);
    }
}

fn timecode(time: f64, fps: i32) -> String {
    let frames = (time * f64::from(fps) + 0.000_001).floor() as u64;
    let seconds = frames / fps as u64;
    format!("{:02}:{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60, frames % fps as u64)
}

fn ruler_label(time: f64, step: f64) -> String {
    let seconds = time.floor() as u64;
    if step < 1.0 {
        format!("{:02}:{:05.2}", seconds / 60, time % 60.0)
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_drag_selects_the_preview_but_scales_using_the_base_frame() {
        let mut app = EditorApp::default();
        app.new_video_document();
        let mut asset = crate::workspace::VideoAsset::from_bytes(
            include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec(),
        );
        asset.info = Some(crate::video::VideoInfo { width: 64, height: 48, fps: 4.0, duration: 1.0 });
        let editor = &mut app.video_editor;
        let doc = editor.current_mut().unwrap();
        doc.width = 200;
        doc.height = 200;
        doc.lane_count = 2;
        assert!(editor.insert(&asset, 0, 0.0, 0.0));
        assert!(editor.insert(&asset, 1, 0.0, 0.0));
        let doc = editor.current_mut().unwrap();
        doc.selected = None;
        let mut motion = crate::video_effects::ClipEffect::new(crate::video_effects::EffectKind::Move);
        motion.seconds = 0.0;
        motion.to = Vec2::new(10.0, 0.0);
        doc.clips[1].effects.set(motion, 1.0, doc.fps);
        let canvas = Rect::from_min_size(Vec2::ZERO, Vec2::splat(200.0));
        let mut ui = Ui::new();
        let mut step = |app: &mut EditorApp, point: Vec2, pressed: bool, down: bool, released: bool| {
            app.input.mouse_pos = point;
            app.input.mouse_pressed = pressed;
            app.input.mouse_down = down;
            app.input.mouse_released = released;
            ui.begin_frame(mega_ui::UiInput {
                viewport: Vec2::splat(200.0),
                mouse_pos: point,
                ..Default::default()
            });
            edit_canvas(&mut ui, app, canvas, canvas);
            ui.end_frame();
        };
        step(&mut app, Vec2::new(145.0, 100.0), true, true, false);
        step(&mut app, Vec2::new(155.0, 100.0), false, false, true);
        step(&mut app, Vec2::new(142.0, 124.0), true, true, false);
        step(&mut app, Vec2::new(174.0, 148.0), false, false, true);

        let doc = app.video_editor.current().unwrap();
        assert_eq!(doc.selected, Some(1));
        assert_eq!(doc.clips[0].position, Vec2::splat(0.5));
        let base = base_clip_rect(doc, &doc.clips[1], canvas).unwrap();
        let rect = preview_clip_rect(doc, &doc.clips[1], canvas).unwrap();
        assert!(doc.clips[1].position.distance(Vec2::new(0.55, 0.5)) < 0.000_1);
        assert!(base.min.distance(Vec2::new(46.0, 52.0)) < 0.01);
        assert!(base.max.distance(Vec2::new(174.0, 148.0)) < 0.01);
        assert!(rect.min.distance(Vec2::new(66.0, 52.0)) < 0.01);
        assert!(rect.max.distance(Vec2::new(194.0, 148.0)) < 0.01);

        let doc = app.video_editor.current_mut().unwrap();
        let mut animation = crate::video_effects::ClipEffect::new(crate::video_effects::EffectKind::TransformAnimation);
        animation.toggle_key(0.0, doc.fps);
        doc.clips[1].effects.set(animation, 1.0, doc.fps);
        step(&mut app, Vec2::new(130.0, 100.0), true, true, false);
        step(&mut app, Vec2::new(140.0, 100.0), false, false, true);

        let doc = app.video_editor.current().unwrap();
        let clip = &doc.clips[1];
        let key = clip.effects.transform_animation().unwrap().keys[0];
        assert!(Vec2::from(key.offset).distance(Vec2::new(0.05, 0.0)) < 0.000_1);
        assert!(clip.position.distance(Vec2::new(0.55, 0.5)) < 0.000_1);
        let frame = transform_frame_rect(doc, clip, canvas).unwrap();
        let preview = preview_clip_rect(doc, clip, canvas).unwrap();
        assert!((preview.min - frame.min).distance(Vec2::new(20.0, 0.0)) < 0.01);
    }

    #[test]
    fn ruler_and_playhead_seek_keep_selection_and_playback() {
        let mut app = EditorApp::default();
        app.new_video_document();
        let mut asset = crate::workspace::VideoAsset::from_bytes(
            include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec(),
        );
        asset.info = Some(crate::video::VideoInfo { width: 64, height: 48, fps: 4.0, duration: 1.0 });
        assert!(app.video_editor.insert(&asset, 0, 5.0, 0.0));
        app.video_editor.seek(0.0);
        app.video_editor.set_playing(true);
        let mut ui = Ui::new();
        let mut layout = TrackLayout {
            rect: Rect::from_min_size(Vec2::new(0.0, 25.0), Vec2::new(600.0, 48.0)),
            top_lane: 0,
            visible_lanes: 1,
            row_height: 48.0,
        };
        app.input.mouse_pos = Vec2::new(64.0 * ui.scale(), 12.0);
        app.input.mouse_pressed = true;
        app.input.mouse_down = true;
        interact(&mut ui, &mut app, &mut layout, true, false);
        assert!(app.video_editor.playing);
        assert_eq!(app.video_editor.current().unwrap().playhead, 1.0);
        assert_eq!(app.video_editor.current().unwrap().selected, Some(0));

        app.input.mouse_pressed = false;
        app.video_editor.tick(0.25);
        interact(&mut ui, &mut app, &mut layout, true, false);
        assert_eq!(app.video_editor.current().unwrap().playhead, 1.25);

        app.input.mouse_pos = Vec2::new(80.0 * ui.scale(), 48.0);
        app.input.mouse_pressed = true;
        interact(&mut ui, &mut app, &mut layout, false, true);
        assert_eq!(app.video_editor.current().unwrap().selected, Some(0));

        app.input.mouse_pos.x = 192.0 * ui.scale();
        interact(&mut ui, &mut app, &mut layout, false, true);
        assert_eq!(app.video_editor.current().unwrap().selected, None);
    }
}
