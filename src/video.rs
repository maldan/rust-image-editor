//! In-process, bounded video decoding. No child processes or temporary video files.
use crate::workspace::VideoAsset;
use ffmpeg_next as ffmpeg;
use image::RgbaImage;
use std::{
    collections::VecDeque,
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
    seek_start: f64,
}

const FRAME_CACHE_BYTES: usize = 2 * 1024 * 1024 * 1024;

struct CachedFrame {
    image: Arc<RgbaImage>,
    position: f64,
    duration: f64,
    seek_start: f64,
}
struct FrameCache {
    frames: VecDeque<CachedFrame>,
    bytes: usize,
    limit: usize,
}
impl FrameCache {
    fn new(limit: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            bytes: 0,
            limit,
        }
    }
    fn get(&mut self, target: f64, duration: f64) -> Option<(Arc<RgbaImage>, f64)> {
        let index = self.frames.iter().position(|frame| {
            (target <= frame.position + 0.000_001
                && (target > frame.seek_start + 0.000_001
                    || (target - frame.position).abs() <= 0.000_001))
                || (target >= duration && frame.position + frame.duration >= duration - 0.000_001)
        })?;
        let frame = self.frames.remove(index).unwrap();
        let result = (frame.image.clone(), frame.position);
        self.frames.push_back(frame);
        Some(result)
    }
    fn insert(&mut self, image: Arc<RgbaImage>, position: f64, duration: f64, seek_start: f64) {
        let bytes = image.as_raw().len();
        if bytes > self.limit {
            return;
        }
        if let Some(index) = self
            .frames
            .iter()
            .position(|f| (f.position - position).abs() < 0.000_001)
        {
            let old = self.frames.remove(index).unwrap();
            self.bytes -= old.image.as_raw().len();
        }
        while self.bytes > self.limit - bytes {
            let old = self.frames.pop_front().unwrap();
            self.bytes -= old.image.as_raw().len();
        }
        self.bytes += bytes;
        self.frames.push_back(CachedFrame {
            image,
            position,
            duration,
            seek_start,
        });
    }
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
    previous_timestamp: Option<f64>,
    #[cfg(test)]
    converted_frames: usize,
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
            previous_timestamp: None,
            #[cfg(test)]
            converted_frames: 0,
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
        self.next_frame_at(reusable, f64::NEG_INFINITY, None)
    }

    fn next_frame_at(
        &mut self,
        reusable: Option<RgbaImage>,
        target: f64,
        revision: Option<u64>,
    ) -> Result<Option<DecodedFrame>, String> {
        let mut previous = ffmpeg::frame::Video::empty();
        let mut previous_time = None;
        let mut end_time = self.start_time;
        let (timestamp, duration, seek_start) = loop {
            if revision.is_some_and(|r| !self.control.unchanged(r)) {
                return Ok(None);
            }
            if !self.decode_next()? {
                if let Some(time) = previous_time {
                    // Keep only a native frame reference during preroll, including the endpoint.
                    std::mem::swap(&mut self.decoded, &mut previous);
                    break time;
                }
                return Ok(None);
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
            let position = timestamp.unwrap_or(end_time);
            let seek_start = self.previous_timestamp.unwrap_or(position);
            self.previous_timestamp = Some(position);
            end_time = position + duration;
            if position + 0.000_001 >= target {
                break (timestamp, duration, seek_start);
            }
            std::mem::swap(&mut self.decoded, &mut previous);
            previous_time = Some((timestamp, duration, seek_start));
        };
        self.convert_frame(reusable, timestamp, duration, seek_start)
            .map(Some)
    }

    fn decode_next(&mut self) -> Result<bool, String> {
        loop {
            if self.control.stopped.load(Ordering::Relaxed) {
                return Ok(false);
            }
            match self.decoder.receive_frame(&mut self.decoded) {
                Ok(()) => return Ok(true),
                Err(ffmpeg::Error::Eof) => return Ok(false),
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {}
                Err(e) => return Err(error("Cannot decode video frame", e)),
            }
            if self.draining {
                return Err("Video decoder requested packets after EOF".into());
            }
            loop {
                if self.control.stopped.load(Ordering::Relaxed) {
                    return Ok(false);
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
    }

    fn convert_frame(
        &mut self,
        reusable: Option<RgbaImage>,
        timestamp: Option<f64>,
        duration: f64,
        seek_start: f64,
    ) -> Result<DecodedFrame, String> {
        #[cfg(test)]
        {
            self.converted_frames += 1;
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
        Ok(DecodedFrame {
            image,
            timestamp,
            duration,
            seek_start,
        })
    }

    fn seek(&mut self, seconds: f64) -> Result<(), String> {
        let timestamp = ((seconds + self.start_time) * 1_000_000.0).round() as i64;
        self.input
            .seek(timestamp, ..timestamp)
            .map_err(|e| error("Cannot seek video", e))?;
        self.decoder.flush();
        self.draining = false;
        self.previous_timestamp = None;
        Ok(())
    }

    #[cfg(test)]
    fn rewind(&mut self) -> Result<(), String> {
        self.seek(0.0)
    }
}

#[derive(Default)]
struct PlaybackState {
    frame: Option<Arc<RgbaImage>>,
    spare: Option<RgbaImage>,
    error: Option<String>,
    info: Option<VideoInfo>,
    position: f64,
    has_frame: bool,
}
pub struct Playback {
    control: Arc<DecoderControl>,
    state: Arc<Mutex<PlaybackState>>,
    pub frame: Option<Arc<RgbaImage>>,
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
    let mut cache = FrameCache::new(FRAME_CACHE_BYTES);
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
        if let Some((image, position)) = cache.get(target, decoder.info.duration) {
            if !publish(worker, output, revision, image, position) {
                continue;
            }
            if request.paused {
                worker.wait_for_change(revision);
                continue;
            }
        }
        decoder.seek(target)?;
        let limited_fps = request.fps.filter(|fps| *fps < decoder.info.fps);
        let mut next_presentation = f64::NEG_INFINITY;
        let mut clock: Option<(Instant, f64)> = None;
        let mut end_time = 0.0;
        let mut found = false;
        loop {
            if !worker.unchanged(revision) {
                break;
            }
            let reusable = output.lock().unwrap().spare.take();
            let frame = decoder.next_frame_at(
                reusable,
                if found {
                    next_presentation + decoder.start_time
                } else {
                    target + decoder.start_time
                },
                Some(revision),
            )?;
            if !worker.unchanged(revision) {
                break;
            }
            let Some(frame) = frame else {
                if !found {
                    return Err("Video has no decodable frames".into());
                }
                if request.paused {
                    worker.wait_for_change(revision);
                } else if let Some((start, position)) = clock {
                    worker.wait_until(
                        start + Duration::from_secs_f64((end_time - position).max(0.0)),
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
            // Seeking returns the last frame at EOF even when the requested time is
            // later. Keep its end time for looping, but do not present a skipped frame.
            if found && position + 0.000_001 < next_presentation {
                continue;
            }
            let (start, origin) = *clock.get_or_insert((Instant::now(), position));
            if !worker.wait_until(
                start + Duration::from_secs_f64((position - origin).max(0.0)),
                revision,
            ) {
                break;
            }
            let image = Arc::new(frame.image);
            cache.insert(
                image.clone(),
                position,
                frame.duration,
                (frame.seek_start - decoder.start_time).max(0.0),
            );
            if !publish(worker, output, revision, image, position) {
                break;
            }
            found = true;
            if let Some(fps) = limited_fps {
                // Sample on a fixed source-time grid, so fractional ratios (25 -> 12)
                // alternate skipped frame counts without accumulating timing drift.
                next_presentation =
                    origin + (((position - origin) * fps + 0.000_001).floor() + 1.0) / fps;
            }
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
    image: Arc<RgbaImage>,
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
        state.spare = previous.and_then(|image| Arc::try_unwrap(image).ok());
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
                if let Ok(previous) = Arc::try_unwrap(previous) {
                    state.spare = Some(previous);
                }
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
    #[test]
    #[ignore = "manual benchmark; generate target/seek-benchmark.mp4 first"]
    fn seek_latency_benchmark() {
        let mut workspace = crate::workspace::Workspace::default();
        workspace
            .add_video(std::fs::read("target/seek-benchmark.mp4").unwrap())
            .unwrap();
        let mut player = Playback::start(workspace.videos[0].clone());
        player.set_paused(true);
        wait_for(&mut player, |p| !p.pending_update);
        for target in [5.8, 3.0, 5.0, 5.8, 3.0, 5.0] {
            let start = Instant::now();
            player.seek(target);
            wait_for(&mut player, |p| !p.pending_update);
            eprintln!(
                "seek {target:.1}s: {:?}, presented {:.3}s",
                start.elapsed(),
                player.position
            );
        }
    }
    fn workspace() -> crate::workspace::Workspace {
        let mut workspace = crate::workspace::Workspace::default();
        workspace
            .add_video(include_bytes!("../tests/fixtures/video-preview.mp4").to_vec())
            .unwrap();
        workspace
    }

    #[test]
    fn frame_cache_bounds_bytes_evicts_lru_and_shares_pixels() {
        let image = Arc::new(RgbaImage::new(2, 2));
        let mut cache = FrameCache::new(32);
        cache.insert(image.clone(), 0.25, 0.25, 0.0);
        cache.insert(Arc::new(RgbaImage::new(2, 2)), 0.5, 0.25, 0.25);
        assert!(
            cache.get(0.0, 1.0).is_none(),
            "do not substitute a missing earlier frame"
        );
        assert!(Arc::ptr_eq(&cache.get(0.1, 1.0).unwrap().0, &image));
        cache.insert(Arc::new(RgbaImage::new(2, 2)), 0.75, 0.25, 0.5);
        assert_eq!(cache.bytes, 32);
        assert!(
            cache.get(0.5, 1.0).is_none(),
            "least recently used frame must be evicted"
        );
        assert!(cache.get(0.25, 1.0).is_some());
        assert_eq!(cache.get(1.0, 1.0).unwrap().1, 0.75);
        cache.insert(image.clone(), 0.25, 0.25, 0.0);
        assert_eq!(cache.bytes, 32, "replacement does not grow the cache");
        cache.insert(Arc::new(RgbaImage::new(4, 4)), 0.0, 0.25, 0.0);
        assert_eq!(cache.bytes, 32, "oversized frames are not cached");
        let mut vfr = FrameCache::new(32);
        vfr.insert(image, 0.75, 0.1, 0.25);
        assert!(
            vfr.get(0.5, 1.0).is_some(),
            "use actual preceding timestamps for VFR"
        );
        assert!(vfr.get(0.25, 1.0).is_none());
    }

    #[test]
    fn seek_converts_only_presented_frame_and_cancels_obsolete_requests() {
        let workspace = workspace();
        let control = Arc::new(DecoderControl::default());
        let mut decoder =
            VideoDecoder::open(&workspace.videos[0], control.clone(), (1920, 1080)).unwrap();
        let mut hashes = Vec::new();
        while let Some(frame) = decoder.next_frame(None).unwrap() {
            hashes.push(crate::safe_save::fingerprint(frame.image.as_raw()));
        }
        for (target, index) in [(0.5, 2), (1.0, 3)] {
            decoder.seek(target).unwrap();
            decoder.converted_frames = 0;
            let frame = decoder
                .next_frame_at(None, target, Some(0))
                .unwrap()
                .unwrap();
            assert_eq!(
                decoder.converted_frames, 1,
                "preroll must skip RGBA conversion"
            );
            assert_eq!(
                crate::safe_save::fingerprint(frame.image.as_raw()),
                hashes[index]
            );
            assert_eq!(frame.timestamp, Some(index as f64 * 0.25));
        }
        decoder.seek(0.75).unwrap();
        decoder.converted_frames = 0;
        control.transport.lock().unwrap().revision = 1;
        assert!(
            decoder
                .next_frame_at(None, 0.75, Some(0))
                .unwrap()
                .is_none()
        );
        assert_eq!(decoder.converted_frames, 0);
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
        let cached_last = player.frame.as_ref().unwrap().clone();
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
        player.seek(0.75);
        wait_for(&mut player, |p| !p.pending_update);
        assert!(
            Arc::ptr_eq(player.frame.as_ref().unwrap(), &cached_last),
            "cached seek shares pixels without another decode or copy"
        );
        player.seek(0.5);
        wait_for(&mut player, |p| !p.pending_update);
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
    fn fps_limit_skips_frames_preserves_duration_and_reset_restores_original() {
        let workspace = workspace();
        let mut player = Playback::start(workspace.videos[0].clone());
        player.set_paused(true);
        wait_for(&mut player, |p| p.info.is_some() && !p.pending_update);
        for (fps, expected) in [
            (Some(2.0), vec![0.0, 0.5]),
            (Some(8.0), vec![0.0, 0.25, 0.5, 0.75]),
            (None, vec![0.0, 0.25, 0.5, 0.75]),
        ] {
            player.seek(0.0);
            player.set_fps(fps);
            wait_for(&mut player, |p| !p.pending_update);
            player.set_paused(false);
            wait_for(&mut player, |p| !p.pending_update);
            let started = Instant::now();
            let mut positions = vec![player.position];
            let deadline = started + Duration::from_secs(5);
            loop {
                assert!(player.poll().is_none());
                let previous = *positions.last().unwrap();
                if player.position < previous {
                    break;
                }
                if player.position != previous {
                    positions.push(player.position);
                }
                assert!(Instant::now() < deadline, "Video did not loop");
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(positions, expected, "present source frames at FPS {fps:?}");
            assert!(
                started.elapsed() >= Duration::from_millis(900),
                "FPS {fps:?} must preserve the 1-second duration"
            );
            player.set_paused(true);
            wait_for(&mut player, |p| !p.pending_update);
        }
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
