use crate::app::EditorApp;
use mega_ui::Ui;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    ui.label("Properties");
    ui.separator();
    ui.space(8.0);
    let Some(id) = app.session.preview_asset.as_deref() else {
        return;
    };
    if let Some(image) = app.workspace.assets.iter().find(|a| a.id == id) {
        ui.label(&image.name);
        ui.label("Image");
        ui.space(8.0);
        ui.label(&format!(
            "Resolution: {} x {} px",
            image.width, image.height
        ));
        ui.label(&format!("File size: {}", format_bytes(image.byte_size)));
        ui.label(&format!(
            "Pixel data: {}",
            format_bytes(image.width as u64 * image.height as u64 * 4)
        ));
        ui.label(&format!("Zoom: {:.0}%", app.session.zoom * 100.0));
        return;
    }
    let Some(video) = app.workspace.videos.iter().find(|v| v.id == id) else {
        return;
    };
    ui.label(&video.name);
    ui.label("MP4 video");
    ui.label(&format!("File size: {}", format_bytes(video.byte_size)));
    ui.space(8.0);
    let Some(player) = app.video_playback() else {
        return;
    };
    let Some(info) = player.info else {
        ui.label("Loading video information...");
        return;
    };
    ui.label(&format!("Resolution: {} x {} px", info.width, info.height));
    ui.label(&format!("Duration: {}", time(info.duration)));
    ui.space(12.0);
    let mut position = player.position as f32;
    ui.label(&format!(
        "{} / {}",
        time(player.position),
        time(info.duration)
    ));
    ui.add_enabled(info.duration > 0.0 && !player.failed, |ui| {
        if ui
            .slider("video_timeline", &mut position, 0.0..=info.duration as f32)
            .changed()
        {
            player.seek(position as f64);
        }
    });
    ui.add_enabled(!player.failed, |ui| {
        if ui
            .button(
                "video_play_pause",
                if player.paused { "Play" } else { "Pause" },
            )
            .clicked()
        {
            player.set_paused(!player.paused);
        }
        ui.space(12.0);
        ui.label("Playback FPS");
        ui.horizontal(|ui| {
            let mut fps = player.fps_override.unwrap_or(info.fps) as f32;
            let field_width = (ui.available_size().x - 36.0 * ui.scale()).max(1.0);
            ui.flex_width(field_width, |ui| {
                if ui.drag_float("video_playback_fps", &mut fps, 0.1).changed() {
                    player.set_fps(Some(fps as f64));
                }
            });
            if ui.icon_button("video_fps_reset", "reset", false) {
                player.set_fps(None);
            }
        });
        ui.label(&format!("Original: {:.3} FPS", info.fps));
        ui.label("Lower FPS skips frames at original speed.");
    });
}

fn time(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}.{:03}",
        millis / 60_000,
        millis / 1000 % 60,
        millis % 1000
    )
}
fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}
