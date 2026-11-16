use crate::app::EditorApp;
use mega_ui::Ui;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    ui.label("Properties");
    ui.separator();
    ui.space(8.0);
    let Some(id) = app.session.preview_asset.as_deref() else {
        return;
    };
    if let Some(index) = app.workspace.assets.iter().position(|a| a.id == id) {
        let image = &app.workspace.assets[index];
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
        ui.space(8.0);
        ui.separator();
        if ui.button("asset_create_document", "Create Image Document").clicked() {
            app.create_document_from_asset(index, ui);
        }
        return;
    }
    let Some(index) = app.workspace.videos.iter().position(|v| v.id == id) else {
        return;
    };
    let video = &app.workspace.videos[index];
    ui.label(&video.name);
    ui.label("MP4 video");
    ui.label(&format!("File size: {}", format_bytes(video.byte_size)));
    ui.space(8.0);
    if ui.button("video_export", "Export MP4…").clicked() {
        app.export_video(index, ui);
    }
    ui.space(8.0);
    let frame_ready = app.video_playback().is_some_and(|player| player.frame.is_some());
    ui.add_enabled(frame_ready, |ui| {
        if ui.button("video_create_document", "Create Document from Frame").clicked() {
            app.create_document_from_video_frame(ui);
        }
    });
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
        ui.label(&format!("Volume: {:.0}%", player.volume * 100.0));
        ui.add_enabled(player.has_audio && player.audio_error.is_none(), |ui| {
            let mut volume = player.volume;
            if ui.slider("video_volume", &mut volume, 0.0..=1.0).changed() {
                player.set_volume(volume);
            }
        });
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
    ui.space(12.0);
    ui.separator();
    ui.space(8.0);
    ui.label("Trim video");
    let id = app.workspace.videos[index].id.clone();
    if app.video_trim.asset_id.as_ref() != Some(&id) {
        app.video_trim.asset_id = Some(id);
        app.video_trim.start = 0.0;
        app.video_trim.end = info.duration as f32;
    }
    let running = app.video_trim.job.is_some();
    ui.add_enabled(!running, |ui| {
        ui.label("From (seconds)");
        ui.drag_float("video_trim_start", &mut app.video_trim.start, 0.01);
        ui.label("To (seconds)");
        ui.drag_float("video_trim_end", &mut app.video_trim.end, 0.01);
        let valid = app.video_trim.start.is_finite() && app.video_trim.end.is_finite()
            && app.video_trim.start >= 0.0
            && app.video_trim.end > app.video_trim.start
            && app.video_trim.end <= info.duration as f32;
        if !valid { ui.label("Choose 0 ≤ From < To ≤ video duration"); }
        ui.add_enabled(valid, |ui| {
            if ui.button("video_trim", "Trim").clicked() {
                app.trim_video(index, ui);
            }
        });
    });
    if running { ui.label("Trimming video…"); }
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
