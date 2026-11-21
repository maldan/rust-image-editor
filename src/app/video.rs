use super::*;

#[derive(Default)]
pub(crate) struct TrimState {
    pub asset_id: Option<String>,
    pub start: f32,
    pub end: f32,
    pub job: Option<crate::video::trim::TrimJob>,
}

impl EditorApp {
    pub fn new_video_document(&mut self) {
        let index = self.video_editor.new_document();
        self.show_video_document(index);
    }

    pub fn show_video_document(&mut self, index: usize) {
        if index >= self.video_editor.documents.len() {
            return;
        }
        self.finish_deformation(false);
        self.commit_transform();
        self.animation_clock = None;
        self.close_asset_preview();
        self.close_graph_view();
        self.close_scene_document();
        self.video_editor.open(index);
        self.workspace.tree_selected = Some(format!("video_document_{index}"));
    }

    pub fn delete_video_document(&mut self, index: usize) {
        if index >= self.video_editor.documents.len() {
            return;
        }
        if self.video_editor.active == Some(index) {
            self.video_editor.close_view();
            self.workspace.select_active_document();
        }
        self.video_editor.documents.remove(index);
        if let Some(active) = &mut self.video_editor.active && *active > index {
            *active -= 1;
            self.workspace.tree_selected = Some(format!("video_document_{active}"));
        }
    }

    pub fn trim_video(&mut self, index: usize, ui: &mut Ui) {
        if self.video_trim.job.is_some() { return; }
        let Some(asset) = self.workspace.videos.get(index) else { return; };
        let Some(info) = self.video_preview.as_ref().and_then(|player| player.info) else { return; };
        let start = self.video_trim.start as f64;
        let end = (self.video_trim.end as f64).min(info.duration);
        if let Err(error) = crate::video::trim::validate_range(start, end, info.duration) {
            self.show_error(ui, error);
            return;
        }
        self.video_trim.job = Some(crate::video::trim::TrimJob::start(asset.clone(), start, end));
        self.status = "Trimming video…".into();
    }

    pub fn create_document_from_video_frame(&mut self, ui: &mut Ui) {
        let Some(player) = self.video_preview.as_ref() else { return; };
        let Some(frame) = player.frame.as_ref() else { return; };
        let source = self.workspace.videos.iter()
            .find(|video| Some(&video.id) == self.session.preview_asset.as_ref())
            .map_or("Video", |video| video.name.as_str());
        let name = format!("{source} — {:.3}s", player.position);
        let mut project = Project::default();
        if let Err(error) = project.import_rgba(frame.as_raw(), frame.width(), frame.height()) {
            self.show_error(ui, error);
            return;
        }
        self.cancel_transform();
        self.animation_clock = None;
        self.close_asset_preview();
        self.close_scene_document();
        let index = self.workspace.create_document(&name);
        self.workspace.documents[index].project = project;
        self.workspace.rebuild_active_thumb_now();
        self.history.discard_project_edits();
        let _ = self.reload_active_view();
        self.dirty = true;
        self.status = format!("Document: {name}");
    }

    pub fn copy_video_frame(&mut self, ui: &mut Ui) {
        let Some(frame) = self.video_preview.as_ref().and_then(|player| player.frame.as_ref()) else { return; };
        let image = arboard::ImageData {
            width: frame.width() as usize,
            height: frame.height() as usize,
            bytes: std::borrow::Cow::Borrowed(frame.as_raw()),
        };
        match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_image(image)) {
            Ok(()) => self.status = "Video frame copied to clipboard".into(),
            Err(error) => self.show_error(ui, format!("Cannot copy image to clipboard: {error}")),
        }
    }

    pub fn export_video(&mut self, index: usize, ui: &mut Ui) {
        let Some(video) = self.workspace.videos.get(index) else { return; };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("MP4 video", &["mp4"])
            .set_file_name(&video.name)
            .save_file()
            .map(|path| ensure_extension(path, "mp4"))
        else { return; };
        match video.export(&path) {
            Ok(()) => {
                self.status = format!("Exported {}", file_name(&path));
                ui.notify_success("Video exported");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub(crate) fn video_playback(&mut self) -> Option<&mut crate::video::Playback> {
        self.video_preview.as_mut()
    }

    pub fn show_video_preview(&mut self, index: usize) {
        let Some(asset) = self.workspace.videos.get(index) else {
            return;
        };
        if self.session.preview_asset.as_ref() == Some(&asset.id) {
            return;
        }
        let asset = asset.clone();
        self.begin_asset_preview(asset.id.clone());
        self.video_preview = Some(crate::video::Playback::start(asset));
        self.status = "Loading video preview…".into();
    }

    pub(in crate::app) fn poll_video(&mut self, ui: &mut Ui) {
        let records = self.video_editor.records();
        if records != self.video_snapshot {
            self.video_snapshot = records;
            self.dirty = true;
        }
        self.video_editor.refresh_assets(&self.workspace.videos);
        if let Some(result) = self.video_trim.job.as_ref().and_then(|job| job.poll()) {
            let job = self.video_trim.job.take().unwrap();
            match result.and_then(|bytes| {
                self.workspace.add_video_with_metadata(bytes, &job.name, job.ai_metadata.clone())
            }) {
                Ok(_) => {
                    self.dirty = true;
                    self.status = "Trimmed video added to assets".into();
                    ui.notify_success("Trimmed video added to assets");
                }
                Err(error) => self.show_error(ui, error),
            }
        }
        if let Some(player) = &mut self.video_preview {
            let had_frame = player.frame.is_some();
            if let Some(error) = player.poll() {
                self.status = error;
            } else if !had_frame && player.frame.is_some() {
                self.status = "Video preview · original timing · loop".into();
            }
            if !had_frame && player.frame.is_some() {
                self.session.fit_image = true;
            }
        }
        if let Some(job) = &self.video_poster {
            if let Some(result) = job.poll() {
                if let Ok(inspection) = result {
                    if let Some(asset) = self.workspace.videos.iter_mut().find(|v| v.id == job.id) {
                        asset.info = Some(inspection.info);
                    }
                    if let Some(thumb) = inspection.thumb
                        && self.workspace.set_video_thumb(&job.id, thumb)
                    {
                        self.dirty = true;
                    }
                }
                self.video_poster = None;
            }
        }
        if self.video_poster.is_none() {
            if let Some(asset) = self
                .workspace
                .videos
                .iter_mut()
                .find(|v| !v.inspection_attempted)
            {
                asset.inspection_attempted = true;
                self.video_poster = Some(crate::video::PosterJob::start(asset.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_document_is_available_during_session_and_marks_project_dirty() {
        let mut app = EditorApp::default();
        let mut ui = Ui::new();
        app.poll_video(&mut ui);
        assert!(!app.dirty);
        app.new_video_document();
        app.video_editor.current_mut().unwrap().fps = 30;
        app.switch_document(0);
        assert!(app.video_editor.current().is_none());
        app.show_video_document(0);
        assert_eq!(app.video_editor.current().unwrap().fps, 30);
        app.dirty = false;
        app.poll_video(&mut ui);
        assert!(app.dirty);
        app.dirty = false;
        app.poll_video(&mut ui);
        assert!(!app.dirty);
        app.video_editor.seek(0.0);
        app.video_editor.current_mut().unwrap().preview_zoom = 2.0;
        app.poll_video(&mut ui);
        assert!(!app.dirty, "view state must not make the project dirty");
    }

    #[test]
    fn video_documents_survive_save_and_open_with_clips_and_effects() {
        use crate::video_document::ClipSource;
        use crate::video_effects::{ClipEffect, EffectKind};
        use std::time::{Duration, Instant};

        let mut app = EditorApp::default();
        let bytes = include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec();
        app.workspace.add_video_named(bytes, "Source").unwrap();
        let video_id = app.workspace.videos[0].id.clone();
        app.workspace.videos[0].info = Some(crate::video::VideoInfo { width: 64, height: 48, duration: 1.0, fps: 4.0 });
        app.new_video_document();
        let asset = app.workspace.videos[0].clone();
        assert!(app.video_editor.insert(&asset, 0, 1.0, 0.0));
        {
            let doc = app.video_editor.current_mut().unwrap();
            doc.fps = 30;
            doc.lane_count = 2;
            doc.background = [0.2, 0.3, 0.4, 1.0];
            let clip = &mut doc.clips[0];
            clip.scale_percent = 150.0;
            clip.position = Vec2::new(0.25, 0.75);
            let duration = clip.range.duration();
            clip.effects.set(ClipEffect::new(EffectKind::FadeIn), duration, 30);
            clip.effects.set(ClipEffect::new(EffectKind::Move), duration, 30);
        }
        let expected = app.video_editor.records();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("video-document.mgimg");
        app.workspace.path = Some(path.clone());
        let mut ui = Ui::new();
        assert!(app.try_save_project(&mut ui));
        assert!(!app.dirty);

        app.open_path(&path).unwrap();
        assert!(!app.dirty);
        assert_eq!(app.workspace.documents.len(), 1);
        assert_eq!(app.video_editor.documents.len(), 1);
        assert_eq!(app.video_editor.records(), expected);
        let doc = &app.video_editor.documents[0];
        assert_eq!((doc.id.as_str(), doc.name.as_str(), doc.fps, doc.lane_count), ("vd_1", "Video 1", 30, 2));
        let clip = &doc.clips[0];
        assert!(matches!(&clip.source, ClipSource::Video { asset, .. } if asset.id == video_id));
        assert_eq!((clip.range.start, clip.range.source_in, clip.range.source_out), (1.0, 0.0, 1.0));
        assert_eq!((clip.scale_percent, clip.position), (150.0, Vec2::new(0.25, 0.75)));
        assert_eq!(clip.effects.entries().len(), 2);

        // Clip info arrives once the asset has been inspected in the background.
        let deadline = Instant::now() + Duration::from_secs(10);
        app.poll_video(&mut ui);
        while app.video_poster.is_some() {
            assert!(Instant::now() < deadline, "video inspection timed out");
            std::thread::sleep(Duration::from_millis(5));
            app.poll_video(&mut ui);
        }
        app.poll_video(&mut ui);
        let clip = &app.video_editor.documents[0].clips[0];
        assert!(matches!(&clip.source, ClipSource::Video { asset, duration }
            if asset.info.is_some() && (*duration - 1.0).abs() < 0.001));

        // A new document created after opening must not reuse the loaded id.
        assert_eq!(app.video_editor.new_document(), 1);
        assert_eq!(app.video_editor.documents[1].id, "vd_2");

        let (editor, missing) = crate::video_document::VideoEditor::from_records(&expected, &[], &[]);
        assert_eq!((editor.documents.len(), editor.documents[0].clips.len(), missing), (1, 0, 1));
    }

    #[test]
    fn saved_video_with_thumbnail_loads_info_in_background() {
        use std::time::{Duration, Instant};

        let mut app = EditorApp::default();
        let bytes = include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec();
        app.workspace.add_video_named(bytes, "Source").unwrap();
        let thumb = vec![127; (crate::workspace::THUMB_SIZE.pow(2) * 4) as usize];
        let id = app.workspace.videos[0].id.clone();
        assert!(app.workspace.set_video_thumb(&id, thumb.clone()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("video.mgimg");
        app.workspace.save(&path).unwrap();
        app.open_path(&path).unwrap();

        let mut ui = Ui::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        app.poll_video(&mut ui);
        while app.video_poster.is_some() {
            assert!(Instant::now() < deadline, "video inspection timed out");
            std::thread::sleep(Duration::from_millis(5));
            app.poll_video(&mut ui);
        }

        let asset = &app.workspace.videos[0];
        let info = asset.info.unwrap();
        assert_eq!((info.width, info.height), (64, 48));
        assert!((info.duration - 1.0).abs() < 0.001);
        assert!((info.fps - 4.0).abs() < 0.001);
        assert_eq!(asset.thumb, thumb);
        assert!(!app.dirty);
    }

    #[test]
    fn trim_adds_a_new_asset_preserves_source_and_survives_save() {
        let mut app = EditorApp::default();
        let original = include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec();
        app.workspace.add_video_named(original.clone(), "Source").unwrap();
        app.show_video_preview(0);
        app.video_playback().unwrap().info = Some(crate::video::VideoInfo {
            width: 64, height: 48, duration: 1.0, fps: 4.0,
        });
        app.video_trim.start = 0.3;
        app.video_trim.end = 0.7;
        app.dirty = false;
        let source_id = app.workspace.videos[0].id.clone();
        let mut ui = Ui::new();
        app.trim_video(0, &mut ui);
        assert!(app.video_trim.job.is_some());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.video_trim.job.is_some() {
            app.poll_video(&mut ui);
            assert!(std::time::Instant::now() < deadline, "trim timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(app.workspace.videos.len(), 2, "{}", app.status);
        assert_eq!(app.workspace.videos[0].read().unwrap(), original);
        assert_ne!(app.workspace.videos[1].id, source_id);
        assert!(app.workspace.videos[1].name.contains("0.300–0.700s"));
        assert_eq!(app.session.preview_asset.as_ref(), Some(&source_id));
        assert!(app.dirty);
        let clipped = app.workspace.videos[1].read().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trim.mgimg");
        app.workspace.save(&path).unwrap();
        let saved = Workspace::open(&path).unwrap();
        assert_eq!(saved.videos[0].read().unwrap(), original);
        assert_eq!(saved.videos[1].read().unwrap(), clipped);
    }

    #[test]
    fn video_frame_document_uses_displayed_pixels_and_preserves_source() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[1, 2, 3, 255], 1, 1).unwrap();
        app.refresh_image().unwrap();
        let original = app.project().layers[0].pixels.clone();
        let count = app.workspace.documents.len();
        let mut ui = Ui::new();
        app.create_document_from_video_frame(&mut ui);
        assert_eq!(app.workspace.documents.len(), count);
        app.workspace
            .add_video(include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec())
            .unwrap();
        app.show_video_preview(0);
        app.create_document_from_video_frame(&mut ui);
        assert_eq!(app.workspace.documents.len(), count);
        let pixels = [12, 34, 56, 255, 78, 90, 123, 255];
        app.video_playback().unwrap().frame = Some(std::sync::Arc::new(
            image::RgbaImage::from_raw(2, 1, pixels.to_vec()).unwrap(),
        ));
        app.video_playback().unwrap().position = 1.25;
        app.session.zoom = 0.25;
        app.dirty = false;
        let video_id = app.workspace.videos[0].id.clone();
        app.create_document_from_video_frame(&mut ui);
        assert_eq!(app.workspace.documents.len(), count + 1);
        assert_eq!(app.workspace.active_document, count);
        assert!(app.workspace.active_doc().name.ends_with(" — 1.250s"));
        assert_eq!(app.image_size, Some((2, 1)));
        assert_eq!(app.project().layers.len(), 1);
        assert_eq!(app.project().layers[0].pixels.to_rgba(2, 1), pixels);
        assert_eq!(app.workspace.documents[0].project.layers[0].pixels, original);
        assert_eq!(app.workspace.videos[0].id, video_id);
        assert_eq!(app.workspace.assets.len(), 0);
        assert!(app.session.preview_asset.is_none());
        assert!(app.video_preview.is_none());
        assert!(app.dirty);
    }

    #[test]
    fn video_preview_is_read_only_and_restores_document_view_when_switching() {
        let mut app = EditorApp::default();
        app.project_mut()
            .import_rgba(&[1, 2, 3, 255].repeat(16), 4, 4)
            .unwrap();
        app.refresh_image().unwrap();
        app.workspace
            .import_asset_rgba("image".into(), &[255, 0, 0, 255].repeat(4), 2, 2)
            .unwrap();
        app.workspace
            .add_video(include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec())
            .unwrap();
        let pixels = app.project().layers[0].pixels.clone();
        app.session.pan = Vec2::new(17.0, 23.0);
        app.session.zoom = 2.0;
        app.session.rotation = 30.0;
        let view = (app.session.pan, app.session.zoom, app.session.rotation);
        app.dirty = false;
        app.show_video_preview(0);
        assert!(app.video_preview.is_some());
        assert_eq!(
            app.session.preview_asset.as_ref(),
            Some(&app.workspace.videos[0].id)
        );
        assert_eq!(app.session.preview_document_view, Some(view));
        app.input.mouse_pressed = true;
        app.input.mouse_down = true;
        app.handle_canvas_edit(CanvasView {
            center: Vec2::ZERO,
            zoom: 1.0,
            rotation: 0.0,
            width: 4,
            height: 4,
        });
        assert_eq!(app.project().layers[0].pixels, pixels);
        assert!(!app.dirty);
        app.show_asset_preview(0);
        assert!(app.video_preview.is_none());
        assert_eq!(app.display_size(), Some((2, 2)));
        assert_eq!(app.session.preview_document_view, Some(view));
        app.switch_document(0);
        assert!(app.session.preview_asset.is_none());
        assert_eq!(
            (app.session.pan, app.session.zoom, app.session.rotation),
            view
        );
        assert_eq!(app.project().layers[0].pixels, pixels);
    }
}
