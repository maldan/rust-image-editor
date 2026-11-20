use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

#[path = "desktop.rs"]
mod desktop;
#[path = "encode.rs"]
mod encode;

use desktop::Desktop;
use encode::AudioFormat;

const FPS: i32 = 30;
const MAX_QUEUE_BYTES: usize = 256 * 1024 * 1024;

pub enum Sample {
    Video { tick: i64, pixels: Vec<u8> },
    Audio { position: usize, pcm: Vec<f32> },
}

impl Sample {
    fn bytes(&self) -> usize {
        match self {
            Self::Video { pixels, .. } => pixels.capacity(),
            Self::Audio { pcm, .. } => pcm.capacity() * size_of::<f32>(),
        }
    }
}

struct State {
    samples: VecDeque<Sample>,
    bytes: usize,
    start: Instant,
    paused: Option<Instant>,
    paused_total: Duration,
    segment_start: Instant,
    duration: Option<Duration>,
    reason: Option<String>,
    last_tick: i64,
}

impl State {
    fn elapsed(&self, now: Instant) -> Duration {
        self.duration.unwrap_or_else(|| {
            self.paused
                .unwrap_or(now)
                .saturating_duration_since(self.start)
                .saturating_sub(self.paused_total)
        })
    }

    fn stop(&mut self, now: Instant, reason: Option<String>) {
        if self.duration.is_none() {
            self.duration = Some(self.elapsed(now));
            self.reason = reason;
        }
    }
}

pub struct Queue {
    state: Mutex<State>,
    available: Condvar,
}

impl Queue {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            state: Mutex::new(State {
                samples: VecDeque::new(),
                bytes: 0,
                start: now,
                paused: None,
                paused_total: Duration::ZERO,
                segment_start: now,
                duration: None,
                reason: None,
                last_tick: -1,
            }),
            available: Condvar::new(),
        }
    }

    fn push(&self, state: &mut State, sample: Sample) {
        let bytes = sample.bytes();
        if state.bytes + bytes > MAX_QUEUE_BYTES {
            state.stop(Instant::now(), Some("Capture queue reached 256 MiB".into()));
        } else {
            state.bytes += bytes;
            state.samples.push_back(sample);
        }
        self.available.notify_one();
    }

    pub fn pop(&self) -> Option<Sample> {
        let mut state = self.state.lock().unwrap();
        while state.samples.is_empty() && state.duration.is_none() {
            state = self.available.wait(state).unwrap();
        }
        let sample = state.samples.pop_front()?;
        state.bytes -= sample.bytes();
        Some(sample)
    }

    pub fn duration(&self) -> Duration {
        self.state.lock().unwrap().elapsed(Instant::now())
    }

    fn stop(&self, reason: Option<String>) {
        self.state.lock().unwrap().stop(Instant::now(), reason);
        self.available.notify_one();
    }

    fn toggle_pause(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        if let Some(paused) = state.paused.take() {
            state.paused_total += now - paused;
            state.segment_start = now;
            false
        } else {
            state.paused = Some(now);
            true
        }
    }

    fn stopped(&self) -> bool {
        self.state.lock().unwrap().duration.is_some()
    }

    fn video(&self, desktop: &mut Desktop, region: &CaptureRegion) -> Result<(), String> {
        let (tick, segment_start) = {
            let state = self.state.lock().unwrap();
            let tick =
                (state.elapsed(Instant::now()).as_nanos() * FPS as u128 / 1_000_000_000) as i64;
            if state.paused.is_some() || state.duration.is_some() || tick <= state.last_tick {
                return Ok(());
            }
            (tick, state.segment_start)
        };
        let pixels = desktop.frame(region.area())?;
        let mut state = self.state.lock().unwrap();
        if state.duration.is_none()
            && state.paused.is_none()
            && state.segment_start == segment_start
        {
            state.last_tick = tick;
            self.push(&mut state, Sample::Video { tick, pixels });
        }
        Ok(())
    }

    fn audio<T: Copy>(&self, format: AudioFormat, pcm: &[T], captured: Instant)
    where
        f32: FromSample<T>,
    {
        let mut state = self.state.lock().unwrap();
        if state.paused.is_some() || state.duration.is_some() {
            return;
        }
        let skip = captured < state.segment_start;
        let skip = if skip {
            let before_segment = state.segment_start - captured;
            format.position(before_segment) + 1
        } else {
            0
        };
        let offset = skip * format.channels;
        if offset >= pcm.len() {
            return;
        }
        let position = format.position(state.elapsed(captured.max(state.segment_start)));
        let pcm = pcm[offset..]
            .iter()
            .copied()
            .map(f32::from_sample_)
            .collect();
        self.push(&mut state, Sample::Audio { position, pcm });
    }
}

fn audio_stream<T: SizedSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: AudioFormat,
    queue: &Arc<Queue>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    f32: FromSample<T>,
{
    let capture = Arc::clone(queue);
    let errors = Arc::clone(queue);
    device.build_input_stream(
        config,
        move |pcm: &[T], info| {
            let stamp = info.timestamp();
            let delay = stamp
                .callback
                .duration_since(&stamp.capture)
                .unwrap_or_default();
            let now = Instant::now();
            capture.audio(format, pcm, now.checked_sub(delay).unwrap_or(now));
        },
        move |error| errors.stop(Some(format!("Audio capture failed: {error}"))),
        None,
    )
}

pub struct Recording {
    pub bytes: Vec<u8>,
    pub warning: Option<String>,
}

#[derive(Clone, Default)]
pub struct CaptureRegion(Arc<Mutex<[i32; 4]>>);

impl CaptureRegion {
    pub fn update(&self, area: [i32; 4]) {
        *self.0.lock().unwrap() = area;
    }

    pub(super) fn area(&self) -> [i32; 4] {
        *self.0.lock().unwrap()
    }
}

pub struct Recorder {
    queue: Arc<Queue>,
    worker: Option<JoinHandle<Result<Recording, String>>>,
}

impl Recorder {
    pub fn start(region: CaptureRegion) -> Result<Self, String> {
        let area = region.area();
        let [_, _, width, height] = area;
        if !(16..=3840).contains(&width)
            || !(16..=2160).contains(&height)
            || width % 2 != 0
            || height % 2 != 0
        {
            return Err("Choose a capture area between 16 × 16 and 3840 × 2160".into());
        }
        let queue = Arc::new(Queue::new());
        let capture_queue = Arc::clone(&queue);
        let worker = thread::Builder::new()
            .name("desktop-capture".into())
            .spawn(move || record(area, region, capture_queue))
            .map_err(|error| format!("Cannot start recording: {error}"))?;
        Ok(Self {
            queue,
            worker: Some(worker),
        })
    }

    pub fn toggle_pause(&self) -> bool {
        self.queue.toggle_pause()
    }

    pub fn duration(&self) -> Duration {
        self.queue.duration()
    }

    pub fn finishing(&self) -> bool {
        self.queue.stopped()
            || self
                .worker
                .as_ref()
                .is_some_and(|worker| worker.is_finished())
    }

    pub fn finish(mut self) -> Result<Recording, String> {
        self.stop();
        self.worker
            .take()
            .unwrap()
            .join()
            .map_err(|_| "Capture thread panicked")?
    }

    pub fn stop(&self) {
        self.queue.stop(None);
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.queue.stop(None);
    }
}

fn record(area: [i32; 4], region: CaptureRegion, queue: Arc<Queue>) -> Result<Recording, String> {
    let temporary = tempfile::Builder::new()
        .suffix(".mp4")
        .tempfile()
        .map_err(|error| format!("Cannot create temporary MP4: {error}"))?
        .into_temp_path();
    let mut desktop = Desktop::new([area[2], area[3]])?;
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("No system audio output device")?;
    // CPAL's WASAPI backend enables loopback for input on an output endpoint.
    let config = device
        .default_output_config()
        .map_err(|error| error.to_string())?;
    let format = AudioFormat {
        channels: config.channels() as usize,
        rate: config.sample_rate().0,
    };
    if !(1..=2).contains(&format.channels) || ![44_100, 48_000].contains(&format.rate) {
        return Err("Screen recording requires mono or stereo audio at 44.1 or 48 kHz".into());
    }
    let sample_format = config.sample_format();
    let config = config.config();
    let audio = match sample_format {
        SampleFormat::F32 => audio_stream::<f32>(&device, &config, format, &queue),
        SampleFormat::I16 => audio_stream::<i16>(&device, &config, format, &queue),
        SampleFormat::I32 => audio_stream::<i32>(&device, &config, format, &queue),
        other => return Err(format!("Unsupported audio format: {other}")),
    }
    .map_err(|error| format!("Cannot open system audio: {error}"))?;

    {
        let mut state = queue.state.lock().unwrap();
        let now = Instant::now();
        if state.duration.is_none() {
            state.start = now;
            state.segment_start = now;
            state.paused = state.paused.map(|_| now);
            state.paused_total = Duration::ZERO;
        }
    }
    audio
        .play()
        .map_err(|error| format!("Cannot start system audio: {error}"))?;
    let worker_queue = Arc::clone(&queue);
    let worker_path = temporary.to_path_buf();
    let worker = thread::Builder::new()
        .name("screen-encoder".into())
        .spawn(move || encode::record(&worker_path, area, FPS, format, &worker_queue))
        .map_err(|error| format!("Cannot start MP4 encoder: {error}"))?;

    while !worker.is_finished() && !queue.stopped() {
        if let Err(error) = queue.video(&mut desktop, &region) {
            queue.stop(Some(error));
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    queue.stop(None);
    drop(audio);
    let frames = worker.join().map_err(|_| "Encoder thread panicked")??;
    let warning = queue.state.lock().unwrap().reason.clone();
    if frames == 0 {
        return Err(warning.unwrap_or_else(|| "No frames captured".into()));
    }
    let bytes =
        std::fs::read(&temporary).map_err(|error| format!("Cannot read recorded MP4: {error}"))?;
    Ok(Recording { bytes, warning })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_drains_pending_frames_after_stop() {
        let queue = Queue::new();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queued.mp4");
        let format = AudioFormat {
            channels: 2,
            rate: 48_000,
        };
        {
            let mut state = queue.state.lock().unwrap();
            for tick in 0..3 {
                queue.push(
                    &mut state,
                    Sample::Video {
                        tick,
                        pixels: vec![128; 64 * 64 * 4],
                    },
                );
            }
            queue.push(
                &mut state,
                Sample::Audio {
                    position: 0,
                    pcm: vec![0.0; 4_800 * format.channels],
                },
            );
            let end = state.start + Duration::from_millis(100);
            state.stop(end, None);
        }

        let frames = encode::record(&path, [0, 0, 64, 64], FPS, format, &queue).unwrap();
        assert_eq!(frames, 3);
        assert_eq!(queue.state.lock().unwrap().bytes, 0);
        let input = ffmpeg_next::format::input(&path).unwrap();
        assert_eq!(input.stream(0).unwrap().frames(), 3);
        assert!(
            input
                .streams()
                .best(ffmpeg_next::media::Type::Audio)
                .is_some()
        );
    }
}
