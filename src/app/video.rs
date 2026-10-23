use super::*;

impl EditorApp {
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

    pub(in crate::app) fn poll_video(&mut self) {
        if let Some(player) = &mut self.video_preview {
            let had_frame = player.frame.is_some();
            if let Some(error) = player.poll() {
                self.status = error;
            } else if !had_frame && player.frame.is_some() {
                self.status = "Video preview · original timing · loop · no audio".into();
            }
            if !had_frame && player.frame.is_some() {
                self.session.fit_image = true;
            }
        }
        if let Some(job) = &self.video_poster {
            if let Some(result) = job.poll() {
                if let Ok(thumb) = result {
                    if self.workspace.set_video_thumb(&job.id, thumb) {
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
                .find(|v| !v.poster_attempted)
            {
                asset.poster_attempted = true;
                self.video_poster = Some(crate::video::PosterJob::start(asset.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
