use super::*;

pub fn validate_range(start: f64, end: f64, duration: f64) -> Result<(), String> {
    if !start.is_finite()
        || !end.is_finite()
        || !duration.is_finite()
        || start < 0.0
        || end <= start
        || end > duration
        || duration <= 0.0
    {
        return Err("Choose 0 ≤ From < To ≤ video duration".into());
    }
    Ok(())
}

pub struct TrimJob {
    pub name: String,
    pub ai_metadata: Option<crate::project::AiMetadata>,
    control: Arc<DecoderControl>,
    receiver: mpsc::Receiver<Result<Vec<u8>, String>>,
}

impl TrimJob {
    pub fn start(asset: VideoAsset, start: f64, end: f64) -> Self {
        let name = format!(
            "{} [{start:.3}–{end:.3}s]",
            asset.name.trim_end_matches(".mp4")
        );
        let control = Arc::new(DecoderControl::default());
        let ai_metadata = asset.ai_metadata.clone();
        let worker = control.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(trim(&asset, start, end, worker));
        });
        Self {
            name,
            ai_metadata,
            control,
            receiver,
        }
    }

    pub fn poll(&self) -> Option<Result<Vec<u8>, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err("Video trimming worker stopped unexpectedly".into())),
        }
    }
}

impl Drop for TrimJob {
    fn drop(&mut self) {
        self.control.stop();
    }
}

// Re-encode video so arbitrary boundaries do not include frames from the preceding
// keyframe. OpenH264 is included in the bundled LGPL FFmpeg.
// Copy the primary audio stream, retaining its offset relative to video.
fn trim(
    asset: &VideoAsset,
    start: f64,
    end: f64,
    control: Arc<DecoderControl>,
) -> Result<Vec<u8>, String> {
    let mut source = VideoDecoder::open(asset, control.clone(), (u32::MAX, u32::MAX))?;
    validate_range(start, end, source.info.duration)?;
    let directory = tempfile::tempdir().map_err(|e| format!("Cannot create trim output: {e}"))?;
    let path = directory.path().join("trim.mp4");
    let mut output = ffmpeg::format::output(&path).map_err(|e| error("Cannot create MP4", e))?;
    let codec =
        ffmpeg::encoder::find_by_name("libopenh264").ok_or("OpenH264 encoder unavailable")?;
    let mut encoder = ffmpeg::codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|e| error("Cannot configure trim encoder", e))?;
    encoder.set_width(source.info.width);
    encoder.set_height(source.info.height);
    encoder.set_format(ffmpeg::format::Pixel::YUV420P);
    encoder.set_aspect_ratio(source.decoder.aspect_ratio());
    encoder.set_time_base((1, 60_000));
    encoder.set_frame_rate(Some(ffmpeg::Rational(
        (source.info.fps * 1000.0).round() as i32,
        1000,
    )));
    encoder.set_max_b_frames(0);
    // Bound quantization instead of squeezing every source into a fixed bitrate.
    encoder.set_qmin(12);
    encoder.set_qmax(12);
    encoder.set_colorspace(source.decoder.color_space());
    encoder.set_color_range(source.decoder.color_range());
    encoder.set_color_primaries(source.decoder.color_primaries());
    encoder.set_color_transfer_characteristic(source.decoder.color_transfer_characteristic());
    if output
        .format()
        .flags()
        .contains(ffmpeg::format::Flags::GLOBAL_HEADER)
    {
        encoder.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
    }
    let mut encoding_options = ffmpeg::Dictionary::new();
    encoding_options.set("rc_mode", "quality");
    encoding_options.set("allow_skip_frames", "0");
    encoding_options.set("profile", "high");
    let encoder = encoder
        .open_as_with(codec, encoding_options)
        .map_err(|e| error("Cannot open trim encoder", e))?;
    {
        let mut stream = output
            .add_stream(codec)
            .map_err(|e| error("Cannot add video stream", e))?;
        stream.set_parameters(&encoder);
        stream.set_time_base((1, 60_000));
    }
    let audio = source
        .input
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .map(|s| (s.index(), s.time_base()));
    if let Some((index, time_base)) = audio {
        let mut stream = output
            .add_stream(ffmpeg::encoder::find(ffmpeg::codec::Id::None))
            .map_err(|e| error("Cannot add audio stream", e))?;
        stream.set_parameters(source.input.stream(index).unwrap().parameters());
        stream.set_time_base(time_base);
        // The MP4 muxer selects its own codec tag, matching FFmpeg's remux example.
        unsafe {
            (*stream.parameters().as_mut_ptr()).codec_tag = 0;
        }
    }
    let mut options = ffmpeg::Dictionary::new();
    options.set("use_editlist", "1");
    options.set("avoid_negative_ts", "disabled");
    output
        .write_header_with(options)
        .map_err(|e| error("Cannot write MP4 header", e))?;
    let video_tb = output.stream(0).unwrap().time_base();
    let audio_tb = audio.map(|_| output.stream(1).unwrap().time_base());
    let mut writer = FrameWriter {
        encoder,
        scaler: ffmpeg::software::scaling::Context::get(
            source.decoder.format(),
            source.info.width,
            source.info.height,
            ffmpeg::format::Pixel::YUV420P,
            source.info.width,
            source.info.height,
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .map_err(|e| error("Cannot convert trim frames", e))?,
        converted: ffmpeg::frame::Video::empty(),
        durations: VecDeque::new(),
        output_time_base: video_tb,
        start,
        end,
        origin: source.start_time,
        input_time_base: source.time_base,
        previous: -source.fallback_duration,
        fallback_duration: source.fallback_duration,
        count: 0,
        done: false,
    };
    source.seek(start)?;
    let mut audio_done = audio.is_none();
    loop {
        if control.stopped.load(Ordering::Relaxed) {
            return Err("Video trimming cancelled".into());
        }
        if writer.done && audio_done {
            break;
        }
        let mut packet = ffmpeg::Packet::empty();
        match packet.read(&mut source.input) {
            Ok(()) => {}
            Err(ffmpeg::Error::Eof) => break,
            Err(e) => return Err(error("Cannot read trim input", e)),
        }
        if packet.stream() == source.stream && !writer.done {
            source
                .decoder
                .send_packet(&packet)
                .map_err(|e| error("Cannot decode trim packet", e))?;
            writer.receive_frames(&mut source.decoder, &mut output)?;
        } else if let Some((index, tb)) = audio {
            if packet.stream() != index || audio_done {
                continue;
            }
            let seconds_per_tick: f64 = tb.into();
            let Some(pts) = packet.pts().or(packet.dts()) else {
                continue;
            };
            let position = pts as f64 * seconds_per_tick - source.start_time;
            if position >= end {
                audio_done = true;
                continue;
            }
            if position + packet.duration() as f64 * seconds_per_tick <= start {
                continue;
            }
            let offset = ((source.start_time + start) / seconds_per_tick).round() as i64;
            packet.set_pts(packet.pts().map(|t| t - offset));
            packet.set_dts(packet.dts().map(|t| t - offset));
            packet.set_duration(
                packet
                    .duration()
                    .min(((end - position) / seconds_per_tick).round() as i64),
            );
            packet.rescale_ts(tb, audio_tb.unwrap());
            packet.set_stream(1);
            packet.set_position(-1);
            packet
                .write_interleaved(&mut output)
                .map_err(|e| error("Cannot write trimmed audio", e))?;
        }
    }
    if !writer.done {
        source
            .decoder
            .send_eof()
            .map_err(|e| error("Cannot flush trim decoder", e))?;
        writer.receive_frames(&mut source.decoder, &mut output)?;
    }
    if writer.count == 0 {
        return Err("Selected range contains no video frames".into());
    }
    writer
        .encoder
        .send_eof()
        .map_err(|e| error("Cannot flush trim encoder", e))?;
    writer.receive_packets(&mut output)?;
    output
        .write_trailer()
        .map_err(|e| error("Cannot finish trimmed MP4", e))?;
    drop(output);
    std::fs::read(path).map_err(|e| format!("Cannot read trimmed MP4: {e}"))
}

struct FrameWriter {
    encoder: ffmpeg::encoder::Video,
    scaler: ffmpeg::software::scaling::Context,
    converted: ffmpeg::frame::Video,
    durations: VecDeque<(i64, i64)>,
    output_time_base: ffmpeg::Rational,
    start: f64,
    end: f64,
    origin: f64,
    input_time_base: f64,
    previous: f64,
    fallback_duration: f64,
    count: usize,
    done: bool,
}

impl FrameWriter {
    fn receive_frames(
        &mut self,
        decoder: &mut ffmpeg::decoder::Video,
        output: &mut ffmpeg::format::context::Output,
    ) -> Result<(), String> {
        let mut frame = ffmpeg::frame::Video::empty();
        loop {
            match decoder.receive_frame(&mut frame) {
                Ok(()) => {}
                Err(ffmpeg::Error::Eof) => break,
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => break,
                Err(e) => return Err(error("Cannot decode trim frame", e)),
            }
            let tb = self.input_time_base;
            let position = frame
                .timestamp()
                .map_or(self.previous + self.fallback_duration, |t| {
                    t as f64 * tb - self.origin
                });
            self.previous = position;
            if position >= self.end - 0.000_001 {
                self.done = true;
                break;
            }
            let duration = if frame.packet().duration > 0 {
                frame.packet().duration as f64 * tb
            } else {
                self.fallback_duration
            };
            if position + duration <= self.start + 0.000_001 {
                continue;
            }
            let pts = ((position.max(self.start) - self.start) * 60_000.0).round() as i64;
            let ticks = ((position + duration).min(self.end) - position.max(self.start)) * 60_000.0;
            self.scaler
                .run(&frame, &mut self.converted)
                .map_err(|e| error("Cannot convert trim frame", e))?;
            self.converted.set_pts(Some(pts));
            self.converted.set_kind(ffmpeg::picture::Type::None);
            self.durations
                .push_back((pts, ticks.round().max(1.0) as i64));
            self.encoder
                .send_frame(&self.converted)
                .map_err(|e| error("Cannot encode trim frame", e))?;
            self.receive_packets(output)?;
            self.count += 1;
        }
        Ok(())
    }

    fn receive_packets(
        &mut self,
        output: &mut ffmpeg::format::context::Output,
    ) -> Result<(), String> {
        loop {
            let mut packet = ffmpeg::Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {}
                Err(ffmpeg::Error::Eof) => break,
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => break,
                Err(e) => return Err(error("Cannot read encoded trim frame", e)),
            }
            if let Some((pts, duration)) = self.durations.pop_front() {
                if packet.pts() != Some(pts) {
                    return Err("Unexpected encoded frame timestamp".into());
                }
                packet.set_duration(duration);
            }
            packet.rescale_ts((1, 60_000), self.output_time_base);
            packet.set_stream(0);
            packet.set_position(-1);
            packet
                .write_interleaved(output)
                .map_err(|e| error("Cannot write trimmed video", e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control() -> Arc<DecoderControl> {
        Arc::new(DecoderControl::default())
    }

    #[test]
    fn trim_rejects_invalid_ranges() {
        for (start, end) in [
            (-0.1, 0.5),
            (0.5, 0.5),
            (0.8, 0.2),
            (0.0, 1.1),
            (f64::NAN, 0.5),
            (0.0, f64::INFINITY),
        ] {
            assert!(validate_range(start, end, 1.0).is_err());
        }
        assert!(validate_range(0.0, 1.0, 1.0).is_ok());
    }

    fn detailed_source() -> Vec<u8> {
        init().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("detailed.mp4");
        let mut output = ffmpeg::format::output(&path).unwrap();
        let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::MPEG4).unwrap();
        let mut encoder = ffmpeg::codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .unwrap();
        encoder.set_width(256);
        encoder.set_height(256);
        encoder.set_format(ffmpeg::format::Pixel::YUV420P);
        encoder.set_time_base((1, 60));
        encoder.set_frame_rate(Some((60, 1)));
        encoder.set_bit_rate(100_000_000);
        encoder.set_qmin(1);
        encoder.set_qmax(1);
        encoder.set_max_b_frames(0);
        encoder.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        let mut encoder = encoder.open_as(codec).unwrap();
        {
            let mut stream = output.add_stream(codec).unwrap();
            stream.set_parameters(&encoder);
            stream.set_time_base((1, 60));
        }
        output.write_header().unwrap();
        let output_tb = output.stream(0).unwrap().time_base();
        let drain = |encoder: &mut ffmpeg::encoder::Video,
                     output: &mut ffmpeg::format::context::Output| {
            let mut packet = ffmpeg::Packet::empty();
            while encoder.receive_packet(&mut packet).is_ok() {
                packet.set_duration(1);
                packet.rescale_ts((1, 60), output_tb);
                packet.set_stream(0);
                packet.write_interleaved(output).unwrap();
            }
        };
        for i in 0..32 {
            let mut frame = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::YUV420P, 256, 256);
            let stride = frame.stride(0);
            for y in 0..256 {
                for x in 0..256 {
                    // Moving fine texture exercises bitrate starvation that small
                    // low-FPS fixture videos cannot expose.
                    let hash = ((x + i * 3) as u32).wrapping_mul(1_664_525)
                        ^ (y as u32).wrapping_mul(1_013_904_223);
                    frame.data_mut(0)[y * stride + x] = 32 + ((hash >> 13) % 192) as u8;
                }
            }
            frame.data_mut(1).fill(128);
            frame.data_mut(2).fill(128);
            frame.set_pts(Some(i as i64));
            encoder.send_frame(&frame).unwrap();
            drain(&mut encoder, &mut output);
        }
        encoder.send_eof().unwrap();
        drain(&mut encoder, &mut output);
        output.write_trailer().unwrap();
        drop(output);
        std::fs::read(path).unwrap()
    }

    #[test]
    fn trim_preserves_fine_detail_at_high_frame_rate() {
        let asset = VideoAsset::from_bytes(detailed_source());
        let bytes = trim(&asset, 0.1, 0.4, control()).unwrap();
        let result = VideoAsset::from_bytes(bytes);
        let mut source = VideoDecoder::open(&asset, control(), (1920, 1080)).unwrap();
        let mut output = VideoDecoder::open(&result, control(), (1920, 1080)).unwrap();
        let mut count = 0;
        let mut worst_psnr = f64::INFINITY;
        while let Some(expected) = source.next_frame(None).unwrap() {
            let position = expected.timestamp.unwrap();
            if position < 0.1 - 0.000_001 || position >= 0.4 - 0.000_001 {
                continue;
            }
            let actual = output.next_frame(None).unwrap().expect("retained frame");
            let mse = actual
                .image
                .as_raw()
                .chunks_exact(4)
                .zip(expected.image.as_raw().chunks_exact(4))
                .map(|(a, b)| (a[0] as f64 - b[0] as f64).powi(2))
                .sum::<f64>()
                / (256.0 * 256.0);
            let psnr = 10.0 * (255.0 * 255.0 / mse).log10();
            worst_psnr = worst_psnr.min(psnr);
            count += 1;
        }
        assert_eq!(count, 18);
        assert!(output.next_frame(None).unwrap().is_none());
        eprintln!("trim detail: worst frame PSNR {worst_psnr:.2} dB");
        assert!(
            worst_psnr >= 38.0,
            "detail lost: worst frame PSNR {worst_psnr:.2} dB"
        );
    }

    #[test]
    fn trim_reads_deferred_assets_and_cancellation_stops_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.mgimg");
        let mut workspace = crate::workspace::Workspace::default();
        workspace
            .add_video(include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec())
            .unwrap();
        workspace.save(&path).unwrap();
        let workspace = crate::workspace::Workspace::open(&path).unwrap();
        let bytes = trim(&workspace.videos[0], 0.25, 0.75, control()).unwrap();
        let output = VideoAsset::from_bytes(bytes);
        let decoder = VideoDecoder::open(&output, control(), (1920, 1080)).unwrap();
        assert!((decoder.info.duration - 0.5).abs() < 0.002);
        let cancelled = control();
        cancelled.stop();
        assert!(trim(&workspace.videos[0], 0.25, 0.75, cancelled).is_err());
    }

    #[test]
    fn trim_between_keyframes_preserves_requested_frames_and_duration() {
        let bytes = include_bytes!("../../tests/fixtures/video-preview.mp4").to_vec();
        let asset = VideoAsset::from_bytes(bytes.clone());
        for (start, end, count) in [(0.3, 0.7, 2), (0.01, 0.05, 1), (0.75, 1.0, 1)] {
            let clipped = trim(&asset, start, end, control()).unwrap();
            assert_eq!(asset.read().unwrap(), bytes);
            assert!(!mp4_has_audio(&clipped));
            let result = VideoAsset::from_bytes(clipped);
            let mut decoded = VideoDecoder::open(&result, control(), (1920, 1080)).unwrap();
            assert!(
                (decoded.info.duration - (end - start)).abs() < 0.002,
                "duration {}",
                decoded.info.duration
            );
            assert_eq!((decoded.info.width, decoded.info.height), (64, 48));
            let mut source = VideoDecoder::open(&asset, control(), (1920, 1080)).unwrap();
            let mut expected = Vec::new();
            while let Some(frame) = source.next_frame(None).unwrap() {
                let position = frame.timestamp.unwrap();
                if position < end && position + frame.duration > start {
                    expected.push(frame.image);
                }
            }
            let mut actual = Vec::new();
            while let Some(frame) = decoded.next_frame(None).unwrap() {
                assert!(frame.timestamp.unwrap() >= -0.001);
                assert!(frame.timestamp.unwrap() < end - start);
                actual.push(frame.image);
            }
            assert_eq!(actual.len(), count);
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.iter().zip(expected) {
                let mean_error = actual
                    .as_raw()
                    .iter()
                    .zip(expected.as_raw())
                    .map(|(a, b)| (*a as f64 - *b as f64).abs())
                    .sum::<f64>()
                    / actual.as_raw().len() as f64;
                assert!(
                    mean_error < 8.0,
                    "wrong source frame: mean pixel error {mean_error}"
                );
            }
        }
    }

    #[test]
    fn trim_preserves_audio_and_normalizes_timestamps() {
        let asset = VideoAsset::from_bytes(
            include_bytes!("../../tests/fixtures/video-with-audio.mp4").to_vec(),
        );
        let bytes = trim(&asset, 0.31, 0.91, control()).unwrap();
        assert!(mp4_has_audio(&bytes));
        let output = VideoAsset::from_bytes(bytes);
        let mut decoder = VideoDecoder::open(&output, control(), (1920, 1080)).unwrap();
        assert!(
            (decoder.info.duration - 0.6).abs() < 0.002,
            "duration {}",
            decoder.info.duration
        );
        assert!(
            decoder
                .next_frame(None)
                .unwrap()
                .unwrap()
                .timestamp
                .unwrap()
                .abs()
                < 0.002
        );
        let input = open_mp4(&output.read().unwrap()).unwrap();
        let audio = input.streams().best(ffmpeg::media::Type::Audio).unwrap();
        let tb: f64 = audio.time_base().into();
        assert!(audio.start_time() as f64 * tb <= 0.002);
        assert!((audio.duration() as f64 * tb - 0.6).abs() < 0.04);
        let mut context = ffmpeg::codec::context::Context::from_parameters(audio.parameters())
            .unwrap()
            .decoder()
            .audio()
            .unwrap();
        let stream = audio.index();
        let mut input = input;
        let mut count = 0;
        let mut frame = ffmpeg::frame::Audio::empty();
        for (s, packet) in input.packets() {
            if s.index() == stream {
                context.send_packet(&packet).unwrap();
                while context.receive_frame(&mut frame).is_ok() {
                    count += frame.samples();
                }
            }
        }
        assert!(count > 0, "trimmed audio must decode");
    }
}
