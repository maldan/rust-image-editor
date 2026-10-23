use crate::{app::EditorApp, camera::Transition};
use mega_ui::Ui;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    let Some(animation) = app.project().animation.clone() else {
        return;
    };
    let camera = &animation.camera;
    let mut visible = camera.visible;
    if ui.icon_button(
        "camera_preview",
        if visible {
            "visibility"
        } else {
            "visibility_off"
        },
        visible,
    ) {
        visible = !visible;
        app.set_camera_visible(visible);
    }
    ui.label(if visible {
        "Camera preview · canvas tools disabled"
    } else {
        "Camera hidden · canvas tools enabled"
    });
    ui.separator();
    ui.add_enabled(!app.animation_playing(), |ui| {
        let current = camera.keys.range(..=animation.frame).next_back();
        let mut key = current
            .map(|(_, k)| *k)
            .unwrap_or_else(|| camera.sample(animation.frame));
        ui.label(&format!(
            "Frame {} · Key {}",
            animation.frame + 1,
            current.map_or(animation.frame + 1, |(&f, _)| f + 1)
        ));
        let mut changed = false;
        ui.label("Pan X (pixels)");
        changed |= ui
            .drag_float("camera_pan_x", &mut key.pan[0], 1.0)
            .changed();
        ui.label("Pan Y (pixels)");
        changed |= ui
            .drag_float("camera_pan_y", &mut key.pan[1], 1.0)
            .changed();
        ui.label("Zoom");
        changed |= ui.drag_float("camera_zoom", &mut key.zoom, 0.01).changed();
        ui.label("Fade to black");
        changed |= ui.slider("camera_fade", &mut key.fade, 0.0..=1.0).changed();
        ui.label("Transition to next key");
        ui.horizontal(|ui| {
            for (id, title, transition) in [
                ("hold", "Hold", Transition::Hold),
                ("linear", "Linear", Transition::Linear),
                ("smooth", "Smooth", Transition::Smooth),
            ] {
                if ui
                    .button(
                        &format!("camera_transition_{id}"),
                        &format!(
                            "{}{}",
                            if key.transition == transition {
                                "• "
                            } else {
                                ""
                            },
                            title
                        ),
                    )
                    .clicked()
                {
                    key.transition = transition;
                    changed = true;
                }
            }
        });
        if changed {
            key.zoom = key.zoom.clamp(0.01, 100.0);
            for pan in &mut key.pan {
                *pan = pan.clamp(-1_000_000.0, 1_000_000.0);
            }
            app.edit_camera_key(Some(key), false);
        }
    });
}
