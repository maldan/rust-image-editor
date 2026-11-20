use super::EditorApp;

#[cfg(windows)]
use crate::screen_capture::{Event, Job, Recording};
#[cfg(windows)]
use mega_ui::Ui;

#[derive(Default)]
pub(super) struct State {
    #[cfg(windows)]
    pub(super) window: std::sync::Weak<winit::window::Window>,
    #[cfg(windows)]
    job: Option<Job>,
}

#[cfg(all(windows, test))]
mod tests {
    use super::*;

    #[test]
    fn finished_capture_becomes_selected_video_asset() {
        let mut app = EditorApp::default();
        let mut ui = Ui::new();
        let bytes = include_bytes!("../../tests/fixtures/video-with-audio.mp4");
        app.add_screen_recording(
            &mut ui,
            Recording {
                bytes: bytes.to_vec(),
                warning: None,
            },
        );

        let video = &app.workspace.videos[0];
        assert_eq!(video.name, "Screen recording 1.mp4");
        assert_eq!(video.read().unwrap(), bytes);
        assert_eq!(
            app.workspace.tree_selected,
            Some(format!("video_{}", video.id))
        );
        assert!(app.dirty);
    }
}

impl EditorApp {
    pub(crate) fn screen_recording_active(&self) -> bool {
        #[cfg(windows)]
        return self.screen_capture.job.is_some();
        #[cfg(not(windows))]
        false
    }

    #[cfg(windows)]
    pub(crate) fn record_screen(&mut self, ui: &mut Ui) {
        if let Some(job) = &self.screen_capture.job {
            job.show();
            return;
        }
        let window = self.screen_capture.window.clone();
        match Job::start(move || {
            if let Some(window) = window.upgrade() {
                window.request_redraw();
            }
        }) {
            Ok(job) => {
                self.screen_capture.job = Some(job);
                self.status = "Move and resize the capture frame, then click Record".into();
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    #[cfg(windows)]
    pub(super) fn poll_screen_capture(&mut self, ui: &mut Ui) {
        while let Some(event) = self.screen_capture.job.as_ref().and_then(Job::poll) {
            match event {
                Event::Saving => self.status = "Saving screen recording…".into(),
                Event::Done(result) => {
                    self.screen_capture.job = None;
                    match result {
                        Ok(Some(recording)) => self.add_screen_recording(ui, recording),
                        Ok(None) => self.status = "Screen recording canceled".into(),
                        Err(error) => self.show_error(ui, error),
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    fn add_screen_recording(&mut self, ui: &mut Ui, recording: Recording) {
        match self
            .workspace
            .add_video_named(recording.bytes, "Screen recording")
        {
            Ok(index) => {
                self.dirty = true;
                self.workspace.tree_selected =
                    Some(format!("video_{}", self.workspace.videos[index].id));
                if let Some(warning) = recording.warning {
                    self.show_error(ui, format!("Partial recording added to Assets. {warning}"));
                } else {
                    self.status = "Screen recording added to Assets".into();
                    ui.notify_success("Screen recording added to Assets");
                }
                ui.request_repaint();
            }
            Err(error) => self.show_error(ui, error),
        }
    }
}
