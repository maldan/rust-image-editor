use crate::app::EditorApp;
use crate::video_effects::{ClipEffect, Easing, EffectKind};
use glam::Vec2;
use mega_ui::{DockNode, Ui, Window};

const TAB: &str = "Effects";

pub struct Dialog {
    document: usize,
    clip: usize,
    draft: Option<ClipEffect>,
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
        ui.label("Select a clip to edit its effects.");
        return;
    };
    let clip = &mut doc.clips[index];
    let duration = clip.range.duration();
    if ui.button("add_clip_effect", "Add Effect…").clicked() {
        app.effect_dialog = Some(Dialog { document, clip: index, draft: None });
    }
    // Only the clicked row mutates the stack, after the list has been drawn.
    let mut toggle = None;
    let mut remove = None;
    for effect in clip.effects.entries().iter().copied() {
        let name = effect.kind.label();
        ui.separator();
        ui.horizontal(|ui| {
            let icon = if effect.enabled { "visibility" } else { "visibility_off" };
            if ui.icon_button(&format!("{name}_visible"), icon, effect.enabled) {
                toggle = Some(ClipEffect { enabled: !effect.enabled, ..effect });
            }
            if ui.icon_button(&format!("{name}_edit"), "edit", false) {
                app.effect_dialog = Some(Dialog { document, clip: index, draft: Some(effect) });
            }
            if ui.icon_button(&format!("{name}_delete"), "delete", false) {
                remove = Some(effect.kind);
            }
            ui.label(name);
        });
        if effect.kind == EffectKind::Move {
            ui.label(&format!("{} s · {}", effect.seconds, effect.easing.label()));
        } else {
            ui.label(&format!("{} s · {} · {}×", effect.seconds, effect.easing.label(), effect.scale));
        }
    }
    if let Some(effect) = toggle {
        clip.effects.set(effect, duration, doc.fps);
        ui.request_repaint();
    }
    if let Some(kind) = remove {
        clip.effects.remove(kind, duration, doc.fps);
        ui.request_repaint();
    }
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
    let title = dialog.draft.map_or("Add Effect", |effect| effect.kind.label());
    let height = match dialog.draft.map(|effect| effect.kind) {
        Some(EffectKind::Move) => 330.0,
        Some(_) => 286.0,
        None => 218.0,
    };
    ui.modal(
        Window::new("clip_effect", title)
            .size(Vec2::new(360.0, height))
            .open(&mut open),
        |ui| {
            if let Some(effect) = &mut dialog.draft {
                ui.label("Duration (seconds)");
                ui.drag_float("clip_effect_duration", &mut effect.seconds, 0.1);
                if effect.kind == EffectKind::Move {
                    ui.label("From offset (% of canvas)");
                    ui.vec2("clip_effect_from", &mut effect.from, 0.5, Vec2::new(-15.0, 0.0));
                    ui.label("To offset (% of canvas)");
                    ui.vec2("clip_effect_to", &mut effect.to, 0.5, Vec2::ZERO);
                } else {
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
                        apply = Some(*effect);
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
