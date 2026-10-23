//! In-process, bounded video decoding. No child processes or temporary video files.
use crate::workspace::VideoAsset;
use ffmpeg_next as ffmpeg;
use image::RgbaImage;
use std::{
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
struct Transport {
    revision: u64,
    paused: bool,
    fps: Option<f64>,
    seek: Option<f64>,
}

#[derive(Default)]
struct DecoderControl {
    stopped: AtomicBool,
    finished: AtomicBool,
    transport: Mutex<Transport>,
    wake: Condvar,
}
impl DecoderControl {
    fn stop(&self) {
        let _transport = self.transport.lock().unwrap();
        self.stopped.store(true, Ordering::Relaxed);
        self.wake.notify_all();
    }
    fn unchanged(&self, revision: u64) -> bool {
        !self.stopped.load(Ordering::Relaxed) && self.transport.lock().unwrap().revision == revision
    }
    fn wait_until(&self, due: Instant, revision: u64) -> bool {
        let mut transport = self.transport.lock().unwrap();
        while !self.stopped.load(Ordering::Relaxed) && transport.revision == revision {
            let remaining = due.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return true;
            }
            transport = self.wake.wait_timeout(transport, remaining).unwrap().0;
        }
        false
    }
    fn wait_for_change(&self, revision: u64) {
        let mut transport = self.transport.lock().unwrap();
        while !self.stopped.load(Ordering::Relaxed) && transport.revision == revision {
            transport = self.wake.wait(transport).unwrap();
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub fps: f64,
}

fn init() -> Result<(), String> {
    static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();
    INITIALIZED
        .get_or_init(|| ffmpeg::init().map_err(|e| format!("Cannot initialize video decoder: {e}")))
        .clone()
}
fn error(context: &str, error: ffmpeg::Error) -> String {
    format!("{context}: {error}")
}
fn preview_size(
    width: u32,
    height: u32,
    max_width: u32,
    max_height: u32,
) -> Result<(u32, u32), String> {
    crate::comfyui::scaled_size(width, height, 1.0)?;
    let scale = (max_width as f64 / width as f64)
        .min(max_height as f64 / height as f64)
        .min(1.0);
    Ok((
        (width as f64 * scale).round().max(1.0) as u32,
        (height as f64 * scale).round().max(1.0) as u32,
    ))
}

struct DecodedFrame {
    image: RgbaImage,
    timestamp: Option<f64>,
    duration: f64,
}
struct VideoDecoder {
    input: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Video,
    decoded: ffmpeg::frame::Video,
    rgba: ffmpeg::frame::Video,
    scaler: Option<ffmpeg::software::scaling::Context>,
    stream: usize,
    time_base: f64,
    fallback_duration: f64,
    draining: bool,
    max_size: (u32, u32),
    control: Arc<DecoderControl>,
    info: VideoInfo,
    start_time: f64,
}
impl VideoDecoder {
    fn open(
        asset: &VideoAsset,
        control: Arc<DecoderControl>,
        max_size: (u32, u32),
    ) -> Result<Self, String> {
        init()?;
        let io = ffmpeg::format::context::StreamIo::from_read_seek_with_capacity(
            asset.reader(),
            64 * 1024,
        )
        .map_err(|e| error("Cannot open video stream", e))?;
        let token = control.clone();
        let input = ffmpeg::format::input_from_stream_with_interrupt(
            io,
            Some("preview.mp4"),
            None,
            move || token.stopped.load(Ordering::Relaxed),
        )
        .map_err(|e| error("Cannot read MP4", e))?;
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or("MP4 has no video stream")?;
        let index = stream.index();
        let time_base: f64 = stream.time_base().into();
        let rate: f64 = stream.avg_frame_rate().into();
        let fallback_duration = if rate.is_finite() && rate > 0.0 {
            1.0 / rate
        } else {
            1.0 / 25.0
        };
        let duration = stream.duration() as f64 * time_base;
        let duration = if duration.is_finite() && duration > 0.0 {
            duration
        } else {
            (input.duration() as f64 / 1_000_000.0).max(0.0)
        };
        let start_time = if stream.start_time() == i64::MIN {
            0.0
        } else {
            stream.start_time() as f64 * time_base
        };
        let mut context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .map_err(|e| error("Cannot configure video decoder", e))?;
        // Bound internal decoder threads/buffers as well as our presentation buffers.
        context.set_threading(ffmpeg::codec::threading::Config {
            kind: ffmpeg::codec::threading::Type::Frame,
            ..ffmpeg::codec::threading::Config::count(2)
        });
        let decoder = context
            .decoder()
            .video()
            .map_err(|e| error("Cannot decode video codec", e))?;
        preview_size(decoder.width(), decoder.height(), max_size.0, max_size.1)?;
        let info = VideoInfo {
            width: decoder.width(),
            height: decoder.height(),
            duration,
            fps: 1.0 / fallback_duration,
        };
        Ok(Self {
            info,
            start_time,
            input,
            decoder,
            decoded: ffmpeg::frame::Video::empty(),
            rgba: ffmpeg::frame::Video::empty(),
            scaler: None,
            stream: index,
            time_base,
            fallback_duration,
            draining: false,
            max_size,
            control,
        })
    }

    fn next_frame(&mut self, reusable: Option<RgbaImage>) -> Result<Option<DecodedFrame>, String> {
        loop {
            if self.control.stopped.load(Ordering::Relaxed) {
                return Ok(None);
            }
            match self.decoder.receive_frame(&mut self.decoded) {
                Ok(()) => break,
                Err(ffmpeg::Error::Eof) => return Ok(None),
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {}
                Err(e) => return Err(error("Cannot decode video frame", e)),
            }
            if self.draining {
                return Err("Video decoder requested packets after EOF".into());
            }
            loop {
                if self.control.stopped.load(Ordering::Relaxed) {
                    return Ok(None);
                }
                let mut packet = ffmpeg::Packet::empty();
                match packet.read(&mut self.input) {
                    Ok(()) if packet.stream() == self.stream => {
                        self.decoder
                            .send_packet(&packet)
                            .map_err(|e| error("Cannot submit video packet", e))?;
                        break;
                    }
                    Ok(()) => continue,
                    Err(ffmpeg::Error::Eof) => {
                        self.decoder
                            .send_eof()
                            .map_err(|e| error("Cannot flush video decoder", e))?;
                        self.draining = true;
                        break;
                    }
                    Err(e) => return Err(error("Cannot read video packet", e)),
                }
            }
        }
        let (width, height) = preview_size(
            self.decoded.width(),
            self.decoded.height(),
            self.max_size.0,
            self.max_size.1,
        )?;
        let definition = ffmpeg::software::scaling::context::Definition {
            format: self.decoded.format(),
            width: self.decoded.width(),
            height: self.decoded.height(),
        };
        if self
            .scaler
            .as_ref()
            .is_none_or(|s| *s.input() != definition)
        {
            self.scaler = Some(
                ffmpeg::software::scaling::Context::get(
                    definition.format,
                    definition.width,
                    definition.height,
                    ffmpeg::format::Pixel::RGBA,
                    width,
                    height,
                    ffmpeg::software::scaling::Flags::BILINEAR,
                )
                .map_err(|e| error("Cannot prepare video colors", e))?,
            );
            self.rgba = ffmpeg::frame::Video::empty();
        }
        self.scaler
            .as_mut()
            .unwrap()
            .run(&self.decoded, &mut self.rgba)
            .map_err(|e| error("Cannot convert video frame", e))?;
        let mut image = reusable
            .filter(|i| i.dimensions() == (width, height))
            .unwrap_or_else(|| RgbaImage::new(width, height));
        // FFmpeg rows can have padding; copy only visible RGBA pixels into the existing canvas buffer.
        let row_bytes = width as usize * 4;
        for (y, row) in image.as_mut().chunks_exact_mut(row_bytes).enumerate() {
            let offset = y * self.rgba.stride(0);
            row.copy_from_slice(&self.rgba.data(0)[offset..offset + row_bytes]);
        }
        let timestamp = self
            .decoded
            .timestamp()
            .map(|t| t as f64 * self.time_base)
            .filter(|t| t.is_finite());
        let duration = self.decoded.packet().duration as f64 * self.time_base;
        let duration = if duration.is_finite() && duration > 0.0 {
            duration
        } else {
            self.fallback_duration
        };
        Ok(Some(DecodedFrame {
            image,
            timestamp,
            duration,
        }))
    }

    fn seek(&mut self, seconds: f64) -> Result<(), String> {
        let timestamp = ((seconds + self.start_time) * 1_000_000.0).round() as i64;
        self.input
            .seek(timestamp, ..timestamp)
            .map_err(|e| error("Cannot seek video", e))?;
        self.decoder.flush();
        self.draining = false;
        Ok(())
    }

    #[cfg(test)]
    fn rewind(&mut self) -> Result<(), String> {
        self.seek(0.0)
    }
}

#[derive(Default)]
struct PlaybackState {
    frame: Option<RgbaImage>,
    spare: Option<RgbaImage>,
    error: Option<String>,
    info: Option<VideoInfo>,
    position: f64,
    has_frame: bool,
}
pub struct Playback {
    control: Arc<DecoderControl>,
    state: Arc<Mutex<PlaybackState>>,
    pub frame: Option<RgbaImage>,
    pub dirty: bool,
    pub failed: bool,
    pub info: Option<VideoInfo>,
    pub position: f64,
    pub paused: bool,
    pub fps_override: Option<f64>,
    pending_update: bool,
}

fn play(
    asset: &VideoAsset,
    worker: &Arc<DecoderControl>,
    output: &Mutex<PlaybackState>,
) -> Result<(), String> {
    let mut decoder = VideoDecoder::open(asset, worker.clone(), (1920, 1080))?;
    output.lock().unwrap().info = Some(decoder.info);
    let mut revision = u64::MAX;
    let mut target = 0.0;
    while !worker.stopped.load(Ordering::Relaxed) {
        let request = worker.transport.lock().unwrap().clone();
        if request.revision != revision {
            target = request
                .seek
                .unwrap_or_else(|| output.lock().unwrap().position);
            revision = request.revision;
        }
        if request.paused && request.seek.is_none() && output.lock().unwrap().has_frame {
            worker.wait_for_change(revision);
            continue;
        }
        decoder.seek(target)?;
        let speed = request.fps.unwrap_or(decoder.info.fps) / decoder.info.fps;
        let mut clock: Option<(Instant, f64)> = None;
        let mut end_time = 0.0;
        let mut found = false;
        let mut last_before_target = None;
        loop {
            if !worker.unchanged(revision) {
                break;
            }
            let reusable = output.lock().unwrap().spare.take();
            let frame = decoder.next_frame(reusable)?;
            let Some(frame) = frame else {
                // Seeking exactly to duration still displays the final frame, including on pause.
                if !found {
                    if let Some((frame, time)) = last_before_target.take() {
                        publish(worker, output, revision, frame, time);
                        found = true;
                    }
                }
                if !found {
                    return Err("Video has no decodable frames".into());
                }
                if request.paused {
                    worker.wait_for_change(revision);
                } else if let Some((start, position)) = clock {
                    worker.wait_until(
                        start + Duration::from_secs_f64(((end_time - position) / speed).max(0.0)),
                        revision,
                    );
                }
                target = 0.0;
                break;
            };
            let position = frame
                .timestamp
                .map(|t| (t - decoder.start_time).max(0.0))
                .unwrap_or(end_time);
            end_time = position + frame.duration;
            if position + 0.000_001 < target {
                last_before_target = Some((frame.image, position));
                continue;
            }
            let (start, origin) = *clock.get_or_insert((Instant::now(), position));
            if !worker.wait_until(
                start + Duration::from_secs_f64(((position - origin) / speed).max(0.0)),
                revision,
            ) {
                break;
            }
            if !publish(worker, output, revision, frame.image, position) {
                break;
            }
            found = true;
            if request.paused {
                worker.wait_for_change(revision);
                break;
            }
        }
    }
    Ok(())
}

fn publish(
    worker: &DecoderControl,
    output: &Mutex<PlaybackState>,
    revision: u64,
    image: RgbaImage,
    position: f64,
) -> bool {
    // Serialize with UI commands so a seek/pause never publishes an obsolete frame afterward.
    let request = worker.transport.lock().unwrap();
    if worker.stopped.load(Ordering::Relaxed) || request.revision != revision {
        return false;
    }
    let mut state = output.lock().unwrap();
    let previous = state.frame.replace(image);
    if state.spare.is_none() {
        state.spare = previous;
    }
    state.position = position;
    state.has_frame = true;
    true
}

impl Playback {
    pub fn start(asset: VideoAsset) -> Self {
        let control = Arc::new(DecoderControl::default());
        let state = Arc::new(Mutex::new(PlaybackState::default()));
        let worker = control.clone();
        let output = state.clone();
        std::thread::spawn(move || {
            let result = play(&asset, &worker, &output);
            if !worker.stopped.load(Ordering::Relaxed) {
                if let Err(error) = result {
                    output.lock().unwrap().error = Some(error);
                }
            }
            worker.finished.store(true, Ordering::Release);
        });
        Self {
            control,
            state,
            frame: None,
            dirty: false,
            failed: false,
            info: None,
            position: 0.0,
            paused: false,
            fps_override: None,
            pending_update: true,
        }
    }
    fn command(&mut self, seek: Option<f64>) {
        let mut transport = self.control.transport.lock().unwrap();
        transport.revision = transport.revision.wrapping_add(1);
        transport.paused = self.paused;
        transport.fps = self.fps_override;
        if seek.is_some() || !self.pending_update {
            transport.seek = seek;
        }
        let mut state = self.state.lock().unwrap();
        state.frame = None;
        if seek.is_none() && !self.pending_update {
            state.position = self.position;
        }
        self.pending_update = !self.paused || self.frame.is_none() || transport.seek.is_some();
        self.control.wake.notify_all();
    }
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
            self.paused = paused;
            self.command(None);
        }
    }
    pub fn set_fps(&mut self, fps: Option<f64>) {
        let fps = fps
            .filter(|fps| fps.is_finite())
            .map(|fps| fps.clamp(1.0, 240.0));
        if fps != self.fps_override {
            self.fps_override = fps;
            self.command(None);
        }
    }
    pub fn seek(&mut self, seconds: f64) {
        if !seconds.is_finite() {
            return;
        }
        let Some(info) = self.info else {
            return;
        };
        let seconds = seconds.clamp(0.0, info.duration);
        self.position = seconds;
        self.command(Some(seconds));
    }
    pub fn needs_repaint(&self) -> bool {
        !self.failed && (!self.paused || self.pending_update)
    }
    pub fn poll(&mut self) -> Option<String> {
        let mut state = self.state.lock().unwrap();
        self.info = state.info;
        if let Some(frame) = state.frame.take() {
            if let Some(previous) = self.frame.replace(frame) {
                state.spare = Some(previous);
            }
            self.position = state.position;
            self.pending_update = false;
            self.dirty = true;
        }
        let error = state.error.take();
        if error.is_some() {
            self.failed = true;
            self.pending_update = false;
        }
        error
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.control.stop();
    }
}

pub struct PosterJob {
    pub id: String,
    control: Arc<DecoderControl>,
    receiver: mpsc::Receiver<Result<Vec<u8>, String>>,
}
impl PosterJob {
    pub fn start(asset: VideoAsset) -> Self {
        let id = asset.id.clone();
        let control = Arc::new(DecoderControl::default());
        let worker = control.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let mut decoder = VideoDecoder::open(&asset, worker.clone(), (84, 84))?;
                let frame = decoder
                    .next_frame(None)?
                    .ok_or("Video has no decodable frames")?;
                Ok(crate::workspace::make_thumb(
                    frame.image.as_raw(),
                    frame.image.width(),
                    frame.image.height(),
                ))
            })();
            let _ = sender.send(result);
            worker.finished.store(true, Ordering::Release);
        });
        Self {
            id,
            control,
            receiver,
        }
    }
    pub fn poll(&self) -> Option<Result<Vec<u8>, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err("Video thumbnail worker stopped unexpectedly".into())),
        }
    }
}
impl Drop for PosterJob {
    fn drop(&mut self) {
        self.control.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn workspace() -> crate::workspace::Workspace {
        let mut workspace = crate::workspace::Workspace::default();
        workspace
            .add_video(include_bytes!("../tests/fixtures/video-preview.mp4").to_vec())
            .unwrap();
        workspace
    }

    #[test]
    fn native_decoder_reads_memory_preserves_timestamps_drains_and_rewinds() {
        let workspace = workspace();
        let mut decoder = VideoDecoder::open(
            &workspace.videos[0],
            Arc::new(DecoderControl::default()),
            (1920, 1080),
        )
        .unwrap();
        let mut frames = Vec::new();
        while let Some(frame) = decoder.next_frame(None).unwrap() {
            assert_eq!(frame.image.dimensions(), (64, 48));
            assert!((frame.timestamp.unwrap() - frames.len() as f64 * 0.25).abs() < 0.001);
            assert!((frame.duration - 0.25).abs() < 0.001);
            frames.push(crate::safe_save::fingerprint(frame.image.as_raw()));
        }
        assert_eq!(frames.len(), 4, "drain delayed H.264 frames at EOF");
        assert_ne!(frames[0], frames[1]);
        decoder.rewind().unwrap();
        let first = decoder.next_frame(None).unwrap().unwrap();
        assert_eq!(
            crate::safe_save::fingerprint(first.image.as_raw()),
            frames[0]
        );
    }

    #[test]
    fn native_decoder_reads_saved_archive_without_extracting_mp4() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("video.mgimg");
        let mut workspace = workspace();
        workspace.save(&path).unwrap();
        let mut loaded = crate::workspace::Workspace::open(&path).unwrap();
        let asset = loaded.videos[0].clone();
        // Playback's source snapshot also survives deletion and compaction of the workspace.
        loaded.videos.clear();
        loaded.save(&path).unwrap();
        loaded.optimize().unwrap();
        let mut decoder =
            VideoDecoder::open(&asset, Arc::new(DecoderControl::default()), (84, 84)).unwrap();
        assert_eq!(
            decoder
                .next_frame(None)
                .unwrap()
                .unwrap()
                .image
                .dimensions(),
            (64, 48)
        );
    }

    #[test]
    fn native_poster_playback_loop_timing_and_stop() {
        let workspace = workspace();
        let asset = workspace.videos[0].clone();
        let poster = PosterJob::start(asset.clone());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(result) = poster.poll() {
                let pixels = result.unwrap();
                assert_eq!(
                    pixels.len(),
                    (crate::workspace::THUMB_SIZE * crate::workspace::THUMB_SIZE * 4) as usize
                );
                assert!(pixels.chunks_exact(4).any(|p| p[3] == 255));
                break;
            }
            assert!(Instant::now() < deadline, "Poster decode timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut player = Playback::start(asset);
        let control = player.control.clone();
        let mut first = None;
        let mut changed = false;
        let mut started = Instant::now();
        loop {
            if let Some(error) = player.poll() {
                panic!("{error}");
            }
            if let Some(frame) = &player.frame {
                let hash = crate::safe_save::fingerprint(frame.as_raw());
                if let Some(first) = first {
                    if hash != first {
                        changed = true;
                    }
                    if changed && hash == first {
                        assert!(
                            started.elapsed() >= Duration::from_millis(800),
                            "preserve the 1-second source duration"
                        );
                        break;
                    }
                } else {
                    first = Some(hash);
                    started = Instant::now();
                }
            }
            assert!(Instant::now() < deadline, "Video did not loop");
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(player);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !control.finished.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "Decoder did not stop");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(control.stopped.load(Ordering::Relaxed));
    }

    fn wait_for(player: &mut Playback, predicate: impl Fn(&Playback) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(error) = player.poll() {
                panic!("{error}");
            }
            if predicate(player) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Playback command timed out at {}",
                player.position
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn pause_seek_latest_request_and_resume_preserve_source_frames() {
        let workspace = workspace();
        let mut decoder = VideoDecoder::open(
            &workspace.videos[0],
            Arc::new(DecoderControl::default()),
            (1920, 1080),
        )
        .unwrap();
        let mut hashes = Vec::new();
        while let Some(frame) = decoder.next_frame(None).unwrap() {
            hashes.push(crate::safe_save::fingerprint(frame.image.as_raw()));
        }
        let mut player = Playback::start(workspace.videos[0].clone());
        player.set_paused(true);
        wait_for(&mut player, |p| p.info.is_some() && !p.pending_update);
        assert_eq!(player.info.unwrap().fps, 4.0);
        assert!(!player.needs_repaint());
        player.dirty = false;
        std::thread::sleep(Duration::from_millis(300));
        assert!(player.poll().is_none());
        assert!(!player.dirty, "paused playback must not publish new frames");
        player.seek(0.25);
        player.seek(0.75);
        player.set_fps(Some(8.0));
        wait_for(&mut player, |p| !p.pending_update);
        assert!(player.paused);
        assert!((player.position - 0.75).abs() < 0.001);
        assert_eq!(
            crate::safe_save::fingerprint(player.frame.as_ref().unwrap().as_raw()),
            hashes[3]
        );
        player.seek(1.0);
        wait_for(&mut player, |p| !p.pending_update);
        assert!(
            (player.position - 0.75).abs() < 0.001,
            "the timeline endpoint displays the last frame"
        );
        player.seek(0.5);
        wait_for(&mut player, |p| !p.pending_update);
        assert_eq!(
            crate::safe_save::fingerprint(player.frame.as_ref().unwrap().as_raw()),
            hashes[2]
        );
        player.set_fps(None);
        assert!(player.fps_override.is_none());
        wait_for(&mut player, |p| !p.pending_update);
        player.set_paused(false);
        wait_for(&mut player, |p| p.position > 0.5);
        assert!(!player.paused);
        player.set_paused(true);
        let control = player.control.clone();
        drop(player);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !control.finished.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "paused worker did not stop");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn fps_override_changes_loop_speed_and_reset_restores_original() {
        let workspace = workspace();
        let mut player = Playback::start(workspace.videos[0].clone());
        player.set_paused(true);
        wait_for(&mut player, |p| p.info.is_some() && !p.pending_update);
        player.set_fps(Some(8.0));
        player.set_paused(false);
        wait_for(&mut player, |p| !p.pending_update);
        let started = Instant::now();
        wait_for(&mut player, |p| p.position >= 0.75);
        wait_for(&mut player, |p| p.position < 0.25);
        assert!(started.elapsed() >= Duration::from_millis(400));
        assert!(
            started.elapsed() < Duration::from_millis(950),
            "4 FPS source at 8 FPS should take about half a second"
        );
        player.set_paused(true);
        wait_for(&mut player, |p| !p.pending_update);
        player.seek(0.0);
        player.set_fps(None);
        wait_for(&mut player, |p| !p.pending_update);
        player.set_paused(false);
        wait_for(&mut player, |p| !p.pending_update);
        let started = Instant::now();
        wait_for(&mut player, |p| p.position >= 0.75);
        wait_for(&mut player, |p| p.position < 0.25);
        assert!(started.elapsed() >= Duration::from_millis(800));
        assert_eq!(player.info.unwrap().fps, 4.0);
        assert!(player.fps_override.is_none());
    }

    #[test]
    fn invalid_mp4_fails_and_preview_size_stays_bounded() {
        let mut workspace = crate::workspace::Workspace::default();
        workspace
            .add_video(b"\0\0\0\x18ftypisombroken".to_vec())
            .unwrap();
        assert!(
            VideoDecoder::open(
                &workspace.videos[0],
                Arc::new(DecoderControl::default()),
                (1920, 1080)
            )
            .is_err()
        );
        assert_eq!(preview_size(3840, 2160, 1920, 1080).unwrap(), (1920, 1080));
        assert_eq!(preview_size(1080, 1920, 1920, 1080).unwrap(), (608, 1080));
        assert!(preview_size(0, 0, 1920, 1080).is_err());
    }
}
