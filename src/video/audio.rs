//! Bounded audio decoding and native output, scheduled against the video clock.
use super::{VideoAsset, error, ffmpeg, init};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

struct Chunk {
    start: f64,
    samples: Vec<f32>,
}

struct State {
    revision: u64,
    generation: u64,
    target: f64,
    paused: bool,
    clock: Option<(Instant, f64)>,
    next_frame: Option<i64>,
    queue: VecDeque<Chunk>,
    queued: usize,
    rate: u32,
    channels: usize,
    volume: f32,
    has_audio: bool,
    probed: bool,
    failed: bool,
    error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            revision: 0,
            generation: 0,
            target: 0.0,
            paused: true,
            clock: None,
            next_frame: None,
            queue: VecDeque::new(),
            queued: 0,
            rate: 0,
            channels: 0,
            volume: 1.0,
            has_audio: false,
            probed: false,
            failed: false,
            error: None,
        }
    }
}

impl State {
    fn clear(&mut self) {
        self.queue.clear();
        self.queued = 0;
        self.clock = None;
        self.next_frame = None;
    }

    fn sample(&mut self, position: i64, channel: usize) -> f32 {
        while self.queue.front().is_some_and(|chunk| {
            position
                >= (chunk.start * self.rate as f64).round() as i64
                    + (chunk.samples.len() / self.channels) as i64
        }) {
            self.queued -= self.queue.pop_front().unwrap().samples.len();
        }
        let Some(chunk) = self.queue.front() else {
            return 0.0;
        };
        let start = (chunk.start * self.rate as f64).round() as i64;
        if position < start {
            return 0.0;
        }
        let frame = (position - start) as usize;
        chunk
            .samples
            .get(frame * self.channels + channel)
            .copied()
            .unwrap_or(0.0)
            * self.volume
    }

    fn render<T: SizedSample + FromSample<f32>>(&mut self, output: &mut [T], now: Instant) {
        output.fill(T::EQUILIBRIUM);
        let Some((start, origin)) = self.clock.filter(|_| !self.paused && !self.failed) else {
            return;
        };
        if self.rate == 0 || self.channels == 0 {
            return;
        }
        // Anchor only once. Hardware consumes a continuous sequence of samples;
        // callback scheduling jitter must never skip or repeat part of a buffer.
        let mut position = self.next_frame.unwrap_or_else(|| {
            ((origin + now.saturating_duration_since(start).as_secs_f64()) * self.rate as f64)
                .round() as i64
        });
        for frame in output.chunks_exact_mut(self.channels) {
            for (channel, sample) in frame.iter_mut().enumerate() {
                *sample = T::from_sample(self.sample(position, channel));
            }
            position += 1;
        }
        self.next_frame = Some(position);
    }
}

#[derive(Default)]
pub(super) struct AudioControl {
    state: Mutex<State>,
    wake: Condvar,
    stopped: AtomicBool,
}

impl AudioControl {
    pub(super) fn wait_ready(&self, revision: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut state = self.state.lock().unwrap();
        loop {
            if self.stopped.load(Ordering::Relaxed) || state.revision != revision || state.paused {
                return false;
            }
            if state.failed
                || (state.probed && !state.has_audio)
                || state.queued >= (state.rate as usize * state.channels / 20).max(1)
            {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return true;
            }
            state = self.wake.wait_timeout(state, remaining).unwrap().0;
        }
    }
    pub(super) fn prepare(&self, revision: u64, target: f64) {
        let mut state = self.state.lock().unwrap();
        if revision < state.revision || self.stopped.load(Ordering::Relaxed) {
            return;
        }
        state.clear();
        state.revision = revision;
        state.generation = state.generation.wrapping_add(1);
        state.target = target;
        state.paused = false;
        self.wake.notify_all();
    }

    pub(super) fn start_clock(&self, revision: u64, start: Instant, origin: f64) {
        let mut state = self.state.lock().unwrap();
        if state.revision == revision && !state.paused && !self.stopped.load(Ordering::Relaxed) {
            state.clock = Some((start, origin));
        }
    }

    pub(super) fn pause(&self, revision: u64) {
        let mut state = self.state.lock().unwrap();
        state.revision = revision;
        state.generation = state.generation.wrapping_add(1);
        state.paused = true;
        state.clear();
        self.wake.notify_all();
    }

    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        let mut state = self.state.lock().unwrap();
        state.paused = true;
        state.clear();
        self.wake.notify_all();
    }

    pub(super) fn set_volume(&self, volume: f32) {
        self.state.lock().unwrap().volume = volume;
    }

    pub(super) fn poll(&self) -> (bool, Option<String>) {
        let mut state = self.state.lock().unwrap();
        (state.has_audio, state.error.take())
    }

    fn fail(&self, error: String) {
        let mut state = self.state.lock().unwrap();
        state.failed = true;
        state.error = Some(error);
        state.clear();
        self.wake.notify_all();
    }

    fn current(&self, generation: u64) -> bool {
        !self.stopped.load(Ordering::Relaxed) && self.state.lock().unwrap().generation == generation
    }
}

pub(super) fn start(asset: VideoAsset) -> Arc<AudioControl> {
    let control = Arc::new(AudioControl::default());
    let worker = control.clone();
    std::thread::spawn(move || {
        if let Err(error) = run(&asset, &worker) {
            if !worker.stopped.load(Ordering::Relaxed) {
                worker.fail(error);
            }
        }
    });
    control
}

fn output<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    control: &Arc<AudioControl>,
) -> Result<cpal::Stream, String> {
    let writer = control.clone();
    let errors = control.clone();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                data.fill(T::EQUILIBRIUM);
                // Queue operations under this lock are short. Contention must
                // not turn a whole audio buffer into silence.
                if let Ok(mut state) = writer.state.lock() {
                    let timestamp = info.timestamp();
                    let latency = timestamp
                        .playback
                        .duration_since(&timestamp.callback)
                        .unwrap_or_default();
                    let now = Instant::now();
                    state.render(data, now.checked_add(latency).unwrap_or(now));
                }
                writer.wake.notify_one();
            },
            move |error| errors.fail(format!("Audio output failed: {error}")),
            None,
        )
        .map_err(|error| format!("Cannot open audio output: {error}"))
}

fn run(asset: &VideoAsset, control: &Arc<AudioControl>) -> Result<(), String> {
    let decoder = AudioDecoder::open(asset, control.clone())?;
    {
        let mut state = control.state.lock().unwrap();
        state.probed = true;
        state.has_audio = decoder.is_some();
        control.wake.notify_all();
    }
    let Some(mut decoder) = decoder else {
        return Ok(());
    };
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("No audio output device")?;
    let supported = device
        .default_output_config()
        .map_err(|error| format!("Cannot read audio output format: {error}"))?;
    let config: cpal::StreamConfig = supported.clone().into();
    if !(1..=8).contains(&config.channels) || !(8000..=384000).contains(&config.sample_rate.0) {
        return Err("Unsupported audio output configuration".into());
    }
    decoder.rate = config.sample_rate.0;
    decoder.channels = config.channels as usize;
    {
        let mut state = control.state.lock().unwrap();
        state.rate = decoder.rate;
        state.channels = decoder.channels;
    }
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => output::<f32>(&device, &config, control),
        cpal::SampleFormat::I16 => output::<i16>(&device, &config, control),
        cpal::SampleFormat::U16 => output::<u16>(&device, &config, control),
        cpal::SampleFormat::I32 => output::<i32>(&device, &config, control),
        cpal::SampleFormat::F64 => output::<f64>(&device, &config, control),
        _ => Err("Unsupported audio output sample format".into()),
    }?;
    stream
        .play()
        .map_err(|error| format!("Cannot start audio output: {error}"))?;
    let mut generation = u64::MAX;
    let mut eof = false;
    while !control.stopped.load(Ordering::Relaxed) {
        let state = control.state.lock().unwrap();
        if state.failed {
            break;
        }
        if state.paused
            || (generation == state.generation
                && (eof || state.queued >= decoder.rate as usize * decoder.channels / 2))
        {
            drop(
                control
                    .wake
                    .wait_timeout(state, Duration::from_millis(20))
                    .unwrap(),
            );
            continue;
        }
        let requested = state.generation;
        let target = state.target;
        drop(state);
        if generation != requested {
            decoder.seek(target)?;
            generation = requested;
            eof = false;
        }
        let chunk = decoder.next_chunk(generation)?;
        if !control.current(generation) {
            continue;
        }
        let Some(mut chunk) = chunk else {
            eof = true;
            continue;
        };
        // Accurate seek: discard audio preroll instead of replaying it after the jump.
        let skip = (((target - chunk.start).max(0.0) * decoder.rate as f64).ceil() as usize)
            .min(chunk.samples.len() / decoder.channels);
        chunk.samples.drain(..skip * decoder.channels);
        chunk.start += skip as f64 / decoder.rate as f64;
        if chunk.samples.is_empty() {
            continue;
        }
        let mut state = control.state.lock().unwrap();
        if state.generation == generation
            && !state.paused
            && !control.stopped.load(Ordering::Relaxed)
        {
            if let Some(previous) = state.queue.back() {
                let end = previous.start
                    + (previous.samples.len() / decoder.channels) as f64 / decoder.rate as f64;
                if (chunk.start - end).abs() <= 2.0 / decoder.rate as f64 {
                    chunk.start = end;
                }
            }
            state.queued += chunk.samples.len();
            state.queue.push_back(chunk);
            control.wake.notify_all();
        }
    }
    drop(stream);
    Ok(())
}

struct AudioDecoder {
    input: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Audio,
    frame: ffmpeg::frame::Audio,
    resampler: Option<ffmpeg::software::resampling::Context>,
    stream: usize,
    time_base: f64,
    video_start: f64,
    draining: bool,
    rate: u32,
    channels: usize,
    next_position: f64,
    control: Arc<AudioControl>,
}

impl AudioDecoder {
    fn open(asset: &VideoAsset, control: Arc<AudioControl>) -> Result<Option<Self>, String> {
        init()?;
        let io = ffmpeg::format::context::StreamIo::from_read_seek_with_capacity(
            asset.reader(),
            64 * 1024,
        )
        .map_err(|e| error("Cannot open audio stream", e))?;
        let token = control.clone();
        let input = ffmpeg::format::input_from_stream_with_interrupt(
            io,
            Some("preview.mp4"),
            None,
            move || token.stopped.load(Ordering::Relaxed),
        )
        .map_err(|e| error("Cannot read audio stream", e))?;
        let Some(stream) = input.streams().best(ffmpeg::media::Type::Audio) else {
            return Ok(None);
        };
        let index = stream.index();
        let time_base: f64 = stream.time_base().into();
        let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .map_err(|e| error("Cannot configure audio decoder", e))?
            .decoder()
            .audio()
            .map_err(|e| error("Cannot decode audio codec", e))?;
        let video_start = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .map_or(0.0, |video| {
                if video.start_time() == i64::MIN {
                    0.0
                } else {
                    video.start_time() as f64 * f64::from(video.time_base())
                }
            });
        Ok(Some(Self {
            input,
            decoder,
            stream: index,
            time_base,
            video_start,
            frame: ffmpeg::frame::Audio::empty(),
            resampler: None,
            draining: false,
            rate: 48000,
            channels: 2,
            next_position: 0.0,
            control,
        }))
    }

    fn seek(&mut self, target: f64) -> Result<(), String> {
        let timestamp = ((target + self.video_start) * 1_000_000.0).round() as i64;
        self.input
            .seek(timestamp, ..timestamp)
            .map_err(|e| error("Cannot seek audio", e))?;
        self.decoder.flush();
        self.draining = false;
        self.resampler = None;
        self.next_position = target;
        Ok(())
    }

    fn decode_next(&mut self, generation: u64) -> Result<bool, String> {
        loop {
            if !self.control.current(generation) {
                return Ok(false);
            }
            match self.decoder.receive_frame(&mut self.frame) {
                Ok(()) => return Ok(true),
                Err(ffmpeg::Error::Eof) => return Ok(false),
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {}
                Err(e) => return Err(error("Cannot decode audio frame", e)),
            }
            if self.draining {
                return Ok(false);
            }
            loop {
                if !self.control.current(generation) {
                    return Ok(false);
                }
                let mut packet = ffmpeg::Packet::empty();
                match packet.read(&mut self.input) {
                    Ok(()) if packet.stream() == self.stream => {
                        self.decoder
                            .send_packet(&packet)
                            .map_err(|e| error("Cannot submit audio packet", e))?;
                        break;
                    }
                    Ok(()) => {}
                    Err(ffmpeg::Error::Eof) => {
                        self.decoder
                            .send_eof()
                            .map_err(|e| error("Cannot drain audio decoder", e))?;
                        self.draining = true;
                        break;
                    }
                    Err(e) => return Err(error("Cannot read audio packet", e)),
                }
            }
        }
    }

    fn next_chunk(&mut self, generation: u64) -> Result<Option<Chunk>, String> {
        let layout = ffmpeg::ChannelLayout::default(self.channels as i32);
        let format = ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed);
        loop {
            if !self.decode_next(generation)? {
                if !self.control.current(generation) {
                    return Ok(None);
                }
                let Some(resampler) = &mut self.resampler else {
                    return Ok(None);
                };
                let mut output = ffmpeg::frame::Audio::new(format, 4096, layout);
                resampler
                    .flush(&mut output)
                    .map_err(|e| error("Cannot drain audio resampler", e))?;
                if output.samples() == 0 {
                    return Ok(None);
                }
                let chunk = Chunk {
                    start: self.next_position,
                    samples: output.plane::<f32>(0).to_vec(),
                };
                self.next_position += output.samples() as f64 / self.rate as f64;
                return Ok(Some(chunk));
            }
            let mut source_layout = self.frame.channel_layout();
            if source_layout.is_empty() {
                source_layout = ffmpeg::ChannelLayout::default(self.frame.channels() as i32);
                self.frame.set_channel_layout(source_layout);
            }
            let definition = ffmpeg::software::resampling::context::Definition {
                format: self.frame.format(),
                channel_layout: source_layout,
                rate: self.frame.rate(),
            };
            if definition.rate == 0 || definition.channel_layout.channels() == 0 {
                return Err("Invalid audio sample rate or channel layout".into());
            }
            if self
                .resampler
                .as_ref()
                .is_none_or(|resampler| resampler.input() != &definition)
            {
                self.resampler = Some(
                    ffmpeg::software::resampling::Context::get(
                        definition.format,
                        definition.channel_layout,
                        definition.rate,
                        format,
                        layout,
                        self.rate,
                    )
                    .map_err(|e| error("Cannot prepare audio resampler", e))?,
                );
            }
            let resampler = self.resampler.as_mut().unwrap();
            let delay = resampler
                .delay()
                .map_or(0, |delay| delay.input.max(0) as usize);
            let capacity = (((self.frame.samples() + delay) as f64 * self.rate as f64
                / definition.rate as f64)
                .ceil() as usize
                + 32)
                .max(1);
            let mut output = ffmpeg::frame::Audio::new(format, capacity, layout);
            let start = self
                .frame
                .timestamp()
                .map(|time| time as f64 * self.time_base - self.video_start)
                .unwrap_or(self.next_position)
                - delay as f64 / definition.rate as f64;
            resampler
                .run(&self.frame, &mut output)
                .map_err(|e| error("Cannot resample audio", e))?;
            if output.samples() == 0 {
                continue;
            }
            let chunk = Chunk {
                start,
                samples: output.plane::<f32>(0).to_vec(),
            };
            self.next_position = start + output.samples() as f64 / self.rate as f64;
            return Ok(Some(chunk));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_keeps_samples_contiguous_despite_timing_jitter_and_volume_changes() {
        let start = Instant::now();
        let mut state = State {
            rate: 100,
            channels: 2,
            paused: false,
            volume: 0.5,
            clock: Some((start, 0.0)),
            ..Default::default()
        };
        let samples: Vec<f32> = (0..100)
            .flat_map(|i| [i as f32 / 100.0, -(i as f32) / 100.0])
            .collect();
        state.queued = samples.len();
        state.queue.push_back(Chunk {
            start: 0.0,
            samples,
        });
        let mut output = [0.0f32; 4];
        state.render(&mut output, start + Duration::from_millis(250));
        assert!((output[0] - 0.125).abs() < 0.0001 && (output[1] + 0.125).abs() < 0.0001);
        state.volume = 0.0;
        state.render(&mut output, start + Duration::from_millis(500));
        assert_eq!(output, [0.0; 4]);
        state.volume = 1.0;
        state.render(&mut output, start + Duration::from_millis(750));
        assert!((output[0] - 0.29).abs() < 0.0001);
        let mut tail = [0.0f32; 200];
        state.render(&mut tail, start + Duration::from_secs(2));
        assert_eq!(&tail[196..], &[0.0; 4]);
        assert_eq!(state.queued, 0);
    }

    #[test]
    fn pause_seek_loop_and_stop_reject_obsolete_audio() {
        let control = AudioControl::default();
        control.set_volume(0.25);
        control.prepare(0, 0.0);
        control.start_clock(0, Instant::now(), 0.0);
        assert!(control.state.lock().unwrap().clock.is_some());
        control.pause(1);
        control.prepare(0, 0.0);
        control.start_clock(0, Instant::now(), 0.0);
        assert!(control.state.lock().unwrap().clock.is_none());
        control.prepare(1, 0.75);
        let generation = control.state.lock().unwrap().generation;
        control.start_clock(1, Instant::now(), 0.75);
        assert_eq!(control.state.lock().unwrap().target, 0.75);
        control.prepare(1, 0.0);
        assert!(!control.current(generation));
        assert_eq!(control.state.lock().unwrap().target, 0.0);
        assert_eq!(control.state.lock().unwrap().volume, 0.25);
        control.stop();
        control.prepare(2, 0.0);
        control.start_clock(2, Instant::now(), 0.0);
        assert!(control.state.lock().unwrap().clock.is_none());
    }

    #[test]
    fn audio_decodes_aac_resamples_and_seeks_without_loading_whole_video() {
        let control = Arc::new(AudioControl::default());
        let silent = VideoAsset::from_bytes(
            include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec(),
        );
        assert!(
            AudioDecoder::open(&silent, control.clone())
                .unwrap()
                .is_none()
        );
        let asset = VideoAsset::from_bytes(
            include_bytes!("../../tests/fixtures/video-with-audio.mp4").to_vec(),
        );
        control.prepare(0, 0.0);
        let generation = control.state.lock().unwrap().generation;
        let mut decoder = AudioDecoder::open(&asset, control.clone())
            .unwrap()
            .unwrap();
        decoder.seek(0.0).unwrap();
        let mut count = 0;
        let mut power = 0.0f64;
        let mut last = f64::NEG_INFINITY;
        let mut ended = false;
        for _ in 0..256 {
            let Some(chunk) = decoder.next_chunk(generation).unwrap() else {
                ended = true;
                break;
            };
            assert!(chunk.start >= last - 0.002);
            assert!(chunk.samples.len() < 10000);
            assert!(chunk.samples.iter().all(|sample| sample.is_finite()));
            count += chunk.samples.len();
            power += chunk
                .samples
                .iter()
                .map(|sample| (*sample as f64).powi(2))
                .sum::<f64>();
            last = chunk.start;
        }
        assert!(ended);
        let seconds = count as f64 / 2.0 / 48000.0;
        assert!((1.2..1.35).contains(&seconds), "decoded duration {seconds}");
        assert!((power / count as f64).sqrt() > 0.01);
        decoder.seek(0.75).unwrap();
        let mut reached_target = false;
        for _ in 0..128 {
            let Some(chunk) = decoder.next_chunk(generation).unwrap() else {
                break;
            };
            if chunk.start + chunk.samples.len() as f64 / 2.0 / 48000.0 > 0.75 {
                reached_target = true;
                break;
            }
        }
        assert!(reached_target);
    }

    #[test]
    #[ignore = "requires a native audio output device; output is muted"]
    fn native_audio_pause_seek_loop_volume_and_close() {
        let asset = VideoAsset::from_bytes(
            include_bytes!("../../tests/fixtures/video-with-audio.mp4").to_vec(),
        );
        let mut player = super::super::Playback::start(asset);
        player.set_volume(0.0);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                player.poll().is_none(),
                "audio error: {:?}",
                player.audio_error
            );
            if player.has_audio && player.position > 0.2 {
                break;
            }
            assert!(Instant::now() < deadline, "audio did not start");
            std::thread::sleep(Duration::from_millis(5));
        }
        let audio = player.control.audio.as_ref().unwrap().clone();
        player.set_paused(true);
        assert!(audio.state.lock().unwrap().clock.is_none());
        player.seek(0.75);
        player.set_paused(false);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut advanced = false;
        loop {
            assert!(player.poll().is_none());
            advanced |= player.position > 0.9;
            if advanced && player.position < 0.3 {
                break;
            }
            assert!(Instant::now() < deadline, "audio/video did not loop");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(audio.state.lock().unwrap().volume, 0.0);
        drop(player);
        let deadline = Instant::now() + Duration::from_secs(3);
        while Arc::strong_count(&audio) > 1 {
            assert!(Instant::now() < deadline, "audio output did not close");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
