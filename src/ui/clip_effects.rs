use crate::app::EditorApp;
use crate::host::FrameInput;
use crate::video_effects::{ClipEffect, Easing, EffectKind};
use glam::Vec2;
use mega_ui::{CursorIcon, DockNode, Rect, Ui, Window};

const TAB: &str = "Effects";

pub struct Dialog {
    document: usize,
    clip: usize,
    draft: Option<ClipEffect>,
}

pub struct KeyStripDrag {
    document: usize,
    clip: usize,
    key: usize,
    origin_x: f32,
    target: f64,
}

#[derive(Default)]
struct KeyStripAction {
    seek: Option<f64>,
    move_key: Option<(usize, f64)>,
    pause: bool,
}

pub fn dock(root: &mut DockNode, active: bool) {
    fn contains(node: &DockNode) -> bool {
        match node {
            DockNode::Leaf { tabs, .. } => tabs.iter().any(|tab| tab == TAB),
            DockNode::Split { first, second, .. } => contains(first) || contains(second),
        }
    }
    fn insert(node: &mut DockNode) -> bool {
        match node {
            DockNode::Leaf { tabs, .. } if tabs.iter().any(|tab| tab == "Properties") => {
                let properties = std::mem::replace(node, DockNode::leaf(&[]));
                *node = DockNode::split_v(0.58, properties, DockNode::leaf(&[TAB]));
                true
            }
            DockNode::Split { first, second, .. } => insert(second) || insert(first),
            _ => false,
        }
    }
    fn remove(node: &mut DockNode) -> bool {
        match node {
            DockNode::Leaf { tabs, active } => {
                if let Some(index) = tabs.iter().position(|tab| tab == TAB) {
                    tabs.remove(index);
                    *active = (*active - usize::from(*active > index)).min(tabs.len().saturating_sub(1));
                    tabs.is_empty()
                } else {
                    false
                }
            }
            DockNode::Split { first, second, .. } => {
                let empty_first = remove(first);
                let empty_second = remove(second);
                if empty_first {
                    *node = std::mem::replace(second.as_mut(), DockNode::leaf(&[]));
                } else if empty_second {
                    *node = std::mem::replace(first.as_mut(), DockNode::leaf(&[]));
                }
                empty_first && empty_second
            }
        }
    }
    if active {
        if !contains(root) && !insert(root) {
            let main = std::mem::replace(root, DockNode::leaf(&[]));
            *root = DockNode::split_h(0.78, main, DockNode::leaf(&[TAB]));
        }
    } else {
        remove(root);
    }
}

pub fn panel(ui: &mut Ui, app: &mut EditorApp) {
    let Some(document) = app.video_editor.active else { return };
    let doc = &mut app.video_editor.documents[document];
    let Some(index) = doc.selected else {
        app.effect_key_drag = None;
        ui.label("Select a clip to edit its effects.");
        return;
    };
    let clip = &mut doc.clips[index];
    let duration = clip.range.duration();
    let start = clip.range.start;
    let elapsed = doc.playhead - start;
    let fps = doc.fps;
    let gap = 4.0 * ui.scale();
    let width = (ui.available_size().x - gap * 2.0).max(3.0);
    ui.row_with(mega_ui::LayoutOpts { spacing: Some(gap), ..Default::default() }, |ui| {
        ui.flex_width(width * 0.43, |ui| {
            if ui.button("add_clip_effect", "Add Effect…").clicked() {
                app.effect_dialog = Some(Dialog { document, clip: index, draft: None });
            }
        });
        ui.flex_width(width * 0.25, |ui| {
            ui.add_enabled(!app.effect_clipboard.is_empty(), |ui| {
                if ui.button("paste_clip_effects", "Paste").clicked() {
                    clip.effects.merge(&app.effect_clipboard, duration, doc.fps);
                    app.status = "Effects pasted".into();
                    ui.request_repaint();
                }
            });
        });
        ui.flex_width(width * 0.32, |ui| {
            ui.add_enabled(!clip.effects.entries().is_empty(), |ui| {
                if ui.button("copy_all_clip_effects", "Copy All").clicked() {
                    app.effect_clipboard = clip.effects.entries().to_vec();
                    app.status = "All clip effects copied".into();
                }
            });
        });
    });
    // Only the clicked row mutates the stack, after the list has been drawn.
    let mut toggle = None;
    let mut toggle_key = None;
    let mut remove = None;
    let mut strip_action = KeyStripAction::default();
    for effect in clip.effects.entries() {
        let name = effect.kind.label();
        ui.separator();
        let row = ui.selectable(&format!("{name}_row"), false, |ui| {
            ui.horizontal(|ui| {
                let icon = if effect.enabled { "visibility" } else { "visibility_off" };
                if ui.icon_button(&format!("{name}_visible"), icon, effect.enabled) {
                    toggle = Some(ClipEffect { enabled: !effect.enabled, ..effect.clone() });
                }
                if ui.icon_button(&format!("{name}_edit"), "edit", false) {
                    app.effect_dialog = Some(Dialog { document, clip: index, draft: Some(effect.clone()) });
                }
                if ui.icon_button(&format!("{name}_delete"), "delete", false) {
                    remove = Some(effect.kind);
                }
                ui.label(name);
            });
            if effect.kind == EffectKind::TransformAnimation {
                ui.label(&format!("{} keys · {}", effect.keys.len(), effect.easing.label()));
                strip_action = key_strip(
                    ui, &app.input, effect, document, index, elapsed, duration, fps, &mut app.effect_key_drag,
                );
                let has_key = effect.key_at(elapsed, doc.fps).is_some();
                ui.add_enabled(effect.enabled && (0.0..duration).contains(&elapsed), |ui| {
                    let label = if has_key { "Remove Key" } else { "Add Key" };
                    if ui.button("toggle_transform_key", label).clicked() {
                        toggle_key = Some(effect.clone());
                    }
                });
            } else if effect.kind == EffectKind::Move {
                ui.label(&format!("Start {} s · {} s · {}", effect.start_seconds, effect.seconds, effect.easing.label()));
            } else {
                ui.label(&format!("{} s · {} · {}×", effect.seconds, effect.easing.label(), effect.scale));
            }
        });
        ui.context_menu(&format!("{name}_context"), row.hovered, |ui| {
            if ui.menu_item("copy_effect", "Copy Effect").clicked() {
                app.effect_clipboard = vec![effect.clone()];
                app.status = format!("{name} copied");
            }
        });
    }
    if let Some(effect) = toggle {
        clip.effects.set(effect, duration, doc.fps);
        ui.request_repaint();
    }
    if let Some(kind) = remove {
        clip.effects.remove(kind, duration, doc.fps);
        app.effect_key_drag = None;
        ui.request_repaint();
    }
    let toggled_key = toggle_key.is_some();
    if let Some(mut effect) = toggle_key {
        effect.toggle_key(elapsed, doc.fps);
        clip.effects.set(effect, duration, doc.fps);
        ui.request_repaint();
    }
    if let Some((key, time)) = strip_action.move_key
        && let Some(mut effect) = clip.effects.entries().iter()
            .find(|effect| effect.kind == EffectKind::TransformAnimation).cloned()
        && effect.move_key(key, time, fps)
    {
        clip.effects.set(effect, duration, fps);
        ui.request_repaint();
    }
    if toggled_key || strip_action.pause {
        app.video_editor.set_playing(false);
    }
    if let Some(time) = strip_action.seek {
        if (time - elapsed).abs() > 0.000_001 {
            app.video_editor.seek(start + time);
        }
        if let Some(layout) = app.video_editor.track
            && let Some(doc) = app.video_editor.current_mut()
        {
            let visible_seconds = f64::from(layout.rect.width() / (doc.pixels_per_second * ui.scale()));
            if doc.playhead < doc.view_start || doc.playhead >= doc.view_start + visible_seconds {
                doc.view_start = (doc.playhead - visible_seconds * 0.5)
                    .clamp(0.0, (doc.duration - visible_seconds).max(0.0));
            }
        }
        ui.request_repaint();
    }
}

fn key_strip(
    ui: &mut Ui,
    input: &FrameInput,
    effect: &ClipEffect,
    document: usize,
    clip: usize,
    elapsed: f64,
    duration: f64,
    fps: i32,
    drag: &mut Option<KeyStripDrag>,
) -> KeyStripAction {
    let scale = ui.scale();
    let area = ui.area("transform_key_strip", Vec2::new(ui.available_size().x.max(1.0), 26.0 * scale));
    let inset = (8.0 * scale).min(area.rect.width() * 0.25);
    let left = area.rect.min.x + inset;
    let right = area.rect.max.x - inset;
    let y = (area.rect.min.y + area.rect.max.y) * 0.5;
    let x_at = |time: f64| left + ((time / duration).clamp(0.0, 1.0) as f32) * (right - left);
    let time_at = || strip_frame_time(
        f64::from((input.mouse_pos.x - left) / (right - left)), duration, fps,
    );
    let hovered_key = effect.keys.iter().enumerate()
        .filter(|(_, key)| key.time < duration)
        .map(|(index, key)| (index, (input.mouse_pos.x - x_at(key.time)).abs()))
        .filter(|(_, distance)| *distance <= 7.0 * scale)
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(index, _)| index);
    let mut action = KeyStripAction::default();
    if !ui.enabled() || input.key_escape || !area.active
        || drag.as_ref().is_some_and(|drag| drag.document != document || drag.clip != clip
            || drag.key >= effect.keys.len())
    {
        *drag = None;
    }
    if ui.enabled() && area.hovered {
        ui.set_mouse_cursor(if hovered_key.is_some() { CursorIcon::Move } else { CursorIcon::Pointer });
        if input.mouse_pressed && !input.key_escape {
            if let Some(key) = hovered_key {
                let target = effect.keys[key].time;
                *drag = Some(KeyStripDrag { document, clip, key, origin_x: input.mouse_pos.x, target });
                action.seek = Some(target);
            } else {
                action.seek = Some(time_at());
            }
        }
    }
    if ui.enabled() && area.active && !input.key_escape {
        if let Some(pending) = drag.as_mut() {
            let original_time = effect.keys[pending.key].time;
            let moved = pending.target != original_time || (input.mouse_pos.x - pending.origin_x).abs() > 2.0 * scale;
            if (input.mouse_down || input.mouse_released) && moved {
                let target = time_at();
                if target != pending.target && effect.key_at(target, fps).is_none_or(|index| index == pending.key) {
                    pending.target = target;
                    action.seek = Some(target);
                    action.pause = true;
                }
            }
            if input.mouse_released || !input.mouse_down {
                let pending = drag.take().unwrap();
                if input.mouse_released && pending.target != effect.keys[pending.key].time {
                    action.move_key = Some((pending.key, pending.target));
                }
            }
        } else if input.mouse_down && !input.mouse_pressed && input.mouse_delta.x != 0.0 {
            action.seek = Some(time_at());
        }
    }

    let track = Rect { min: Vec2::new(left, y - 2.0 * scale), max: Vec2::new(right, y + 2.0 * scale) };
    ui.fill_round(track, 2.0 * scale, [0.17, 0.19, 0.23, 1.0]);
    let cursor_time = action.seek.unwrap_or(elapsed);
    let marker = x_at(cursor_time);
    if marker > left {
        ui.fill_round(Rect { min: track.min, max: Vec2::new(marker, track.max.y) }, 2.0 * scale, [0.33, 0.38, 0.48, 1.0]);
    }
    if (0.0..duration).contains(&cursor_time) {
        ui.line(Vec2::new(marker, y - 10.0 * scale), Vec2::new(marker, y + 10.0 * scale), scale, [1.0, 0.66, 0.29, 1.0]);
    }
    let active_key = (0.0..duration).contains(&cursor_time).then(|| effect.key_at(cursor_time, fps)).flatten();
    for (index, key) in effect.keys.iter().enumerate().filter(|(_, key)| key.time < duration) {
        let moving = drag.as_ref().filter(|drag| drag.key == index);
        let time = moving.map_or(key.time, |drag| drag.target);
        let color = if moving.is_some() || active_key == Some(index) {
            [1.0, 0.73, 0.38, 1.0]
        } else {
            [0.78, 0.83, 0.91, 1.0]
        };
        let x = x_at(time);
        for row in -4_i32..=4 {
            let half_width = (5 - row.abs()) as f32 * scale;
            ui.fill_rect(
                Rect::from_min_size(Vec2::new(x - half_width, y + (row as f32 - 0.5) * scale),
                    Vec2::new(2.0 * half_width, scale)),
                color,
            );
        }
    }
    action
}

fn strip_frame_time(fraction: f64, duration: f64, fps: i32) -> f64 {
    let fps = f64::from(fps);
    let frames = duration * fps;
    let last = (frames - 0.000_001).ceil().max(1.0) - 1.0;
    (fraction.clamp(0.0, 1.0) * frames).round().min(last) / fps
}

pub fn modal(ui: &mut Ui, app: &mut EditorApp) {
    let Some(mut dialog) = app.effect_dialog.take() else { return };
    let Some(doc) = app.video_editor.documents.get_mut(dialog.document) else { return };
    let Some(clip) = doc.clips.get_mut(dialog.clip) else { return };
    if app.video_editor.active != Some(dialog.document) || app.input.key_escape {
        return;
    }
    let mut open = true;
    let mut apply = None;
    let title = dialog.draft.as_ref().map_or("Add Effect", |effect| effect.kind.label());
    let height = match dialog.draft.as_ref().map(|effect| effect.kind) {
        Some(EffectKind::Move) => 390.0,
        Some(EffectKind::TransformAnimation) => 170.0,
        Some(_) => 286.0,
        None => 218.0,
    };
    ui.modal(
        Window::new("clip_effect", title)
            .size(Vec2::new(360.0, height))
            .open(&mut open),
        |ui| {
            if let Some(effect) = &mut dialog.draft {
                if effect.kind == EffectKind::Move {
                    ui.label("Start (seconds from clip start)");
                    ui.drag_float("clip_effect_start", &mut effect.start_seconds, 0.1);
                }
                if effect.kind != EffectKind::TransformAnimation {
                    ui.label("Duration (seconds)");
                    ui.drag_float("clip_effect_duration", &mut effect.seconds, 0.1);
                }
                if effect.kind == EffectKind::Move {
                    ui.label("From offset (% of canvas)");
                    ui.vec2("clip_effect_from", &mut effect.from, 0.5, Vec2::new(-15.0, 0.0));
                    ui.label("To offset (% of canvas)");
                    ui.vec2("clip_effect_to", &mut effect.to, 0.5, Vec2::ZERO);
                } else if effect.kind != EffectKind::TransformAnimation {
                    ui.label("Scale at fade edge (×)");
                    ui.drag_float("clip_effect_scale", &mut effect.scale, 0.01);
                }
                ui.label("Easing");
                let mut easing = effect.easing.index();
                if ui.select("clip_effect_easing", &mut easing, &Easing::LABELS).changed {
                    effect.easing = Easing::ALL[easing];
                }
                ui.add_enabled(effect.settings_valid(), |ui| {
                    if ui.button("clip_effect_apply", "Apply").clicked() {
                        apply = Some(effect.clone());
                    }
                });
            } else {
                for kind in EffectKind::ALL {
                    let available = !clip.effects.entries().iter().any(|effect| effect.kind == kind);
                    ui.add_enabled(available, |ui| {
                        if ui.button(kind.label(), kind.label()).clicked() {
                            apply = Some(ClipEffect::new(kind));
                        }
                    });
                }
            }
        },
    );
    if let Some(effect) = apply {
        clip.effects.set(effect, clip.range.duration(), doc.fps);
    } else if open {
        app.effect_dialog = Some(dialog);
    }
    ui.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Scene;
    use mega_ui::{DrawCommand, Rect, UiOutput};

    #[test]
    fn key_strip_maps_to_clip_frames_and_retimes_without_replacing_another_key() {
        let mut effect = ClipEffect::new(EffectKind::TransformAnimation);
        assert_eq!(strip_frame_time(0.5, 4.0, 24), 2.0);
        assert_eq!(strip_frame_time(1.0, 4.0, 24), 95.0 / 24.0);
        effect.toggle_key(0.0, 24);
        effect.toggle_key(1.0, 24);
        effect.keys[0].offset = [0.25, 0.0];
        effect.keys[0].scale = 1.5;
        assert!(!effect.move_key(0, 1.0, 24));
        assert!(effect.move_key(0, strip_frame_time(0.5, 4.0, 24), 24));
        assert_eq!(effect.keys[0].time, 1.0);
        assert_eq!(effect.keys[1].time, 2.0);
        assert_eq!(effect.keys[1].offset, [0.25, 0.0]);
        assert_eq!(effect.keys[1].scale, 1.5);
    }

    #[test]
    fn video_opacity_reaches_the_gpu_draw_colors_without_touching_library_textures() {
        let mut app = EditorApp::default();
        app.video_editor.preview_opacity = vec![1.0, 0.25];
        let slot = crate::video_document::PREVIEW_TEXTURE;
        let rect = Rect::from_min_size(Vec2::ZERO, Vec2::splat(32.0));
        let mut output = UiOutput::default();
        for texture in [slot, slot + 1, crate::host::THUMB_ATLAS] {
            output.draw_list.push(DrawCommand::solid(
                rect, [0.0; 2], [1.0; 2], [1.0; 4], 1.0, texture,
            ));
        }
        app.finish_ui(&mut output);
        assert_eq!(output.draw_list[0].colors, [[1.0; 4]; 4]);
        assert_eq!(output.draw_list[1].colors, [[1.0, 1.0, 1.0, 0.25]; 4]);
        assert_eq!(output.draw_list[2].colors, [[1.0; 4]; 4]);
    }
}
