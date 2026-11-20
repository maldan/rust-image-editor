use std::{path::Path, time::Duration};

use ffmpeg_next as ff;

use super::{Queue, Sample};

const AUDIO_FORMAT: ff::format::Sample = ff::format::Sample::F32(ff::format::sample::Type::Planar);

#[derive(Clone, Copy)]
pub struct AudioFormat {
    pub channels: usize,
    pub rate: u32,
}

impl AudioFormat {
    pub fn position(self, time: Duration) -> usize {
        (time.as_nanos() * self.rate as u128 / 1_000_000_000) as usize
    }
}

pub fn record(
    path: &Path,
    [_, _, width, height]: [i32; 4],
    fps: i32,
    format: AudioFormat,
    queue: &Queue,
) -> Result<usize, String> {
    record_inner(path, width as u32, height as u32, fps, format, queue)
        .map_err(|error| format!("MP4 encoding failed: {error}"))
}

fn record_inner(
    path: &Path,
    width: u32,
    height: u32,
    fps: i32,
    format: AudioFormat,
    queue: &Queue,
) -> Result<usize, ff::Error> {
    ff::init()?;
    let mut output = ff::format::output(path)?;
    let video_codec = ff::encoder::find_by_name("libopenh264").ok_or(ff::Error::EncoderNotFound)?;
    let mut video = ff::codec::context::Context::new_with_codec(video_codec)
        .encoder()
        .video()?;
    let video_clock = ff::Rational(1, fps);
    video.set_width(width);
    video.set_height(height);
    video.set_format(ff::format::Pixel::YUV420P);
    video.set_time_base(video_clock);
    video.set_frame_rate(Some(ff::Rational(fps, 1)));
    video.set_bit_rate(4_000_000);
    video.set_flags(ff::codec::Flags::GLOBAL_HEADER);
    let mut options = ff::Dictionary::new();
    options.set("allow_skip_frames", "0");
    let mut video = video.open_as_with(video_codec, options)?;
    {
        let mut stream = output.add_stream(video_codec)?;
        stream.set_parameters(&video);
        stream.set_time_base(video_clock);
    }
    let (audio_codec, mut audio) = Audio::new(format)?;
    {
        let mut stream = output.add_stream(audio_codec)?;
        stream.set_parameters(&audio.encoder);
        stream.set_time_base(audio.clock);
    }
    output.write_header()?;
    let video_output_clock = output.stream(0).unwrap().time_base();
    audio.output_clock = output.stream(1).unwrap().time_base();

    let mut bgra = ff::frame::Video::new(ff::format::Pixel::BGRA, width, height);
    let mut yuv = ff::frame::Video::new(ff::format::Pixel::YUV420P, width, height);
    let mut scaler = ff::software::scaling::Context::get(
        bgra.format(),
        width,
        height,
        yuv.format(),
        width,
        height,
        ff::software::scaling::Flags::BILINEAR,
    )?;
    let mut frames = 0;
    while let Some(sample) = queue.pop() {
        match sample {
            Sample::Video { tick, pixels } => {
                let stride = bgra.stride(0);
                let row_bytes = width as usize * 4;
                for (target, source) in bgra
                    .data_mut(0)
                    .chunks_mut(stride)
                    .zip(pixels.chunks_exact(row_bytes))
                {
                    target[..row_bytes].copy_from_slice(source);
                }
                scaler.run(&bgra, &mut yuv)?;
                yuv.set_pts(Some(tick));
                video.send_frame(&yuv)?;
                packets(&mut output, 0, video_clock, video_output_clock, |packet| {
                    video.receive_packet(packet)
                })?;
                frames += 1;
            }
            Sample::Audio { position, pcm } => audio.push(position, &pcm, &mut output)?,
        }
    }
    audio.finish(queue.duration(), &mut output)?;
    video.send_eof()?;
    packets(&mut output, 0, video_clock, video_output_clock, |packet| {
        video.receive_packet(packet)
    })?;
    output.write_trailer()?;
    Ok(frames)
}

fn packets(
    output: &mut ff::format::context::Output,
    stream: usize,
    from: ff::Rational,
    to: ff::Rational,
    mut receive: impl FnMut(&mut ff::Packet) -> Result<(), ff::Error>,
) -> Result<(), ff::Error> {
    let mut packet = ff::Packet::empty();
    loop {
        match receive(&mut packet) {
            Ok(()) => {
                packet.set_stream(stream);
                if stream == 0 && packet.duration() == 0 {
                    packet.set_duration(1);
                }
                packet.rescale_ts(from, to);
                packet.write_interleaved(output)?;
            }
            Err(ff::Error::Eof) => return Ok(()),
            Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => return Ok(()),
            Err(error) => return Err(error),
        }
    }
}

struct Audio {
    encoder: ff::encoder::Audio,
    format: AudioFormat,
    clock: ff::Rational,
    output_clock: ff::Rational,
    layout: ff::ChannelLayout,
    frame_size: usize,
    encoded: usize,
    pending: Vec<f32>,
}

impl Audio {
    fn new(format: AudioFormat) -> Result<(ff::Codec, Self), ff::Error> {
        let codec = ff::encoder::find(ff::codec::Id::AAC).ok_or(ff::Error::EncoderNotFound)?;
        let mut encoder = ff::codec::context::Context::new_with_codec(codec)
            .encoder()
            .audio()?;
        let layout = ff::ChannelLayout::default(format.channels as i32);
        let clock = ff::Rational(1, format.rate as i32);
        encoder.set_channel_layout(layout);
        encoder.set_rate(format.rate as i32);
        encoder.set_format(AUDIO_FORMAT);
        encoder.set_time_base(clock);
        encoder.set_bit_rate(128_000);
        encoder.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        let encoder = encoder.open_as(codec)?;
        let frame_size = encoder.frame_size() as usize;
        Ok((
            codec,
            Self {
                encoder,
                format,
                clock,
                output_clock: clock,
                layout,
                frame_size,
                encoded: 0,
                pending: Vec::with_capacity(frame_size * format.channels),
            },
        ))
    }

    fn position(&self) -> usize {
        self.encoded + self.pending.len() / self.format.channels
    }

    fn push(
        &mut self,
        position: usize,
        pcm: &[f32],
        output: &mut ff::format::context::Output,
    ) -> Result<(), ff::Error> {
        // Small timestamp variations between the callback clock and the desktop clock
        // do not need silence insertion or sample trimming.
        let position = if self.position().abs_diff(position) <= self.format.rate as usize / 500 {
            self.position()
        } else {
            position
        };
        self.silence_until(position, output)?;
        let skip = self.position().saturating_sub(position) * self.format.channels;
        let mut pcm = &pcm[skip.min(pcm.len())..];
        while !pcm.is_empty() {
            let take = pcm
                .len()
                .min(self.frame_size * self.format.channels - self.pending.len());
            self.pending.extend_from_slice(&pcm[..take]);
            pcm = &pcm[take..];
            if self.pending.len() == self.frame_size * self.format.channels {
                self.encode_pending(output)?;
            }
        }
        Ok(())
    }

    fn silence_until(
        &mut self,
        end: usize,
        output: &mut ff::format::context::Output,
    ) -> Result<(), ff::Error> {
        while self.position() < end {
            let samples = (end - self.encoded).min(self.frame_size);
            self.pending.resize(samples * self.format.channels, 0.0);
            if samples == self.frame_size {
                self.encode_pending(output)?;
            }
        }
        Ok(())
    }

    fn encode_pending(
        &mut self,
        output: &mut ff::format::context::Output,
    ) -> Result<(), ff::Error> {
        let samples = self.pending.len() / self.format.channels;
        if samples == 0 {
            return Ok(());
        }
        let mut frame = ff::frame::Audio::new(AUDIO_FORMAT, samples, self.layout);
        frame.set_rate(self.format.rate);
        frame.set_pts(Some(self.encoded as i64));
        for channel in 0..self.format.channels {
            for (dst, src) in frame
                .plane_mut::<f32>(channel)
                .iter_mut()
                .zip(self.pending[channel..].iter().step_by(self.format.channels))
            {
                *dst = *src;
            }
        }
        self.encoder.send_frame(&frame)?;
        self.encoded += samples;
        self.pending.clear();
        self.drain(output)
    }

    fn finish(
        &mut self,
        duration: Duration,
        output: &mut ff::format::context::Output,
    ) -> Result<(), ff::Error> {
        self.silence_until(self.format.position(duration), output)?;
        self.encode_pending(output)?;
        self.encoder.send_eof()?;
        self.drain(output)
    }

    fn drain(&mut self, output: &mut ff::format::context::Output) -> Result<(), ff::Error> {
        packets(output, 1, self.clock, self.output_clock, |packet| {
            self.encoder.receive_packet(packet)
        })
    }
}
