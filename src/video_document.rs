use crate::{video::Playback, workspace::VideoAsset};
use crate::video_effects::ClipEffects;
use glam::Vec2;
use mega_ui::Rect;
use image::RgbaImage;
use std::sync::{Arc, Weak};

pub const PREVIEW_TEXTURE: u32 = 7;
const EPSILON: f64 = 0.000_001;
const PREVIEW_LOOKAHEAD: f64 = 1.0;

#[derive(Clone, Copy, Debug)]
pub struct ClipRange {
    pub lane: usize,
    pub start: f64,
    pub source_in: f64,
    pub source_out: f64,
}

impl ClipRange {
    pub fn duration(self) -> f64 {
        self.source_out - self.source_in
    }

    pub fn end(self) -> f64 {
        self.start + self.duration()
    }
}

pub struct Clip {
    pub asset: Arc<VideoAsset>,
    pub source_duration: f64,
    pub range: ClipRange,
    pub scale_percent: f32,
    pub position: Vec2,
    pub effects: ClipEffects,
}

impl Clip {
    pub fn size_in_document(&self) -> Option<Vec2> {
        let info = self.asset.info?;
        Some(Vec2::new(info.width as f32, info.height as f32) * (self.scale_percent / 100.0))
    }
}

#[derive(Clone, Copy)]
pub enum ClipEdit {
    Move,
    TrimStart,
    TrimEnd,
}

pub struct ClipDrag {
    pub index: usize,
    pub edit: ClipEdit,
    pub anchor: f64,
    pub candidate: Option<ClipRange>,
}

pub enum CanvasDragKind {
    Move,
    Scale { anchor: Vec2, diagonal: Vec2 },
}

pub struct CanvasDrag {
    pub index: usize,
    pub mouse: Vec2,
    pub position: Vec2,
    pub scale_percent: f32,
    pub kind: CanvasDragKind,
}

pub struct VideoDocument {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub fps: i32,
    pub duration: f64,
    pub playhead: f64,
    pub preview_zoom: f32,
    pub preview_pan: Vec2,
    pub pixels_per_second: f32,
    pub view_start: f64,
    pub clips: Vec<Clip>,
    pub lane_count: usize,
    pub lane_scroll: usize,
    hidden_lanes: std::collections::HashSet<usize>,
    pub selected: Option<usize>,
}

impl VideoDocument {
    fn new(name: String) -> Self {
        Self {
            name,
            width: 1920,
            height: 1080,
            fps: 24,
            duration: 30.0,
            playhead: 0.0,
            preview_zoom: 1.0,
            preview_pan: Vec2::ZERO,
            pixels_per_second: 64.0,
            view_start: 0.0,
            clips: Vec::new(),
            lane_count: 1,
            lane_scroll: 0,
            hidden_lanes: Default::default(),
            selected: None,
        }
    }

    pub fn content_end(&self) -> f64 {
        self.clips.iter().map(|clip| clip.range.end()).fold(0.0, f64::max)
    }

    pub fn lane_visible(&self, lane: usize) -> bool {
        !self.hidden_lanes.contains(&lane)
    }

    pub fn active_clips(&self) -> impl Iterator<Item = (usize, &Clip)> {
        self.clips.iter().enumerate().filter(|(_, clip)| {
            self.lane_visible(clip.range.lane)
                && self.playhead >= clip.range.start - EPSILON
                && self.playhead < clip.range.end() - EPSILON
        })
    }

    pub fn snap(&self, seconds: f64, radius: f64, exclude: Option<usize>) -> f64 {
        let mut nearest = seconds;
        let mut distance = radius;
        let edges = self.clips.iter().enumerate()
            .filter(|(index, _)| Some(*index) != exclude)
            .flat_map(|(_, clip)| [clip.range.start, clip.range.end()]);
        for edge in [0.0, self.playhead, self.duration].into_iter().chain(edges) {
            let delta = (edge - seconds).abs();
            if delta < distance {
                nearest = edge;
                distance = delta;
            }
        }
        if distance < radius {
            nearest
        } else {
            let fps = f64::from(self.fps);
            (seconds * fps).round() / fps
        }
    }

    pub fn fits(&self, range: ClipRange, exclude: Option<usize>) -> bool {
        range.lane < self.lane_count
            && range.start.is_finite()
            && range.start >= 0.0
            && range.source_out - range.source_in > EPSILON
            && self.clips.iter().enumerate().all(|(index, clip)| {
                Some(index) == exclude
                    || clip.range.lane != range.lane
                    || range.end() <= clip.range.start + EPSILON
                    || range.start >= clip.range.end() - EPSILON
            })
    }

    pub fn edit_range(&self, drag: &ClipDrag, lane: usize, time: f64, radius: f64) -> Option<ClipRange> {
        let clip = self.clips.get(drag.index)?;
        let original = clip.range;
        let delta = time - drag.anchor;
        let min_length = (1.0 / f64::from(self.fps)).min(original.source_out - original.source_in);
        let mut range = original;
        match drag.edit {
            ClipEdit::Move => {
                range.lane = lane;
                if delta.abs() > EPSILON {
                    let start = self.snap(original.start + delta, radius, Some(drag.index));
                    let length = original.source_out - original.source_in;
                    let end = self.snap(original.end() + delta, radius, Some(drag.index));
                    range.start = if (end - original.end() - delta).abs() < (start - original.start - delta).abs() {
                        end - length
                    } else {
                        start
                    }.max(0.0);
                }
            }
            ClipEdit::TrimStart => {
                let start = self.snap(original.start + delta, radius, Some(drag.index));
                range.start = start.clamp(
                    (original.start - original.source_in).max(0.0),
                    (original.end() - min_length).max(original.start),
                );
                range.source_in += range.start - original.start;
            }
            ClipEdit::TrimEnd => {
                let end = self.snap(original.end() + delta, radius, Some(drag.index));
                range.source_out = (original.source_out + end - original.end())
                    .clamp((original.source_in + min_length).min(original.source_out), clip.source_duration);
            }
        }
        self.fits(range, Some(drag.index)).then_some(range)
    }

    pub fn drop_range(&self, asset: &VideoAsset, lane: usize, seconds: f64, radius: f64) -> Option<ClipRange> {
        let duration = asset.info?.duration;
        let start = self.snap(seconds, radius, None).max(0.0);
        let range = ClipRange { lane, start, source_in: 0.0, source_out: duration };
        (duration.is_finite() && self.fits(range, None)).then_some(range)
    }
}

#[derive(Clone, Copy)]
pub struct TrackLayout {
    pub rect: Rect,
    pub top_lane: usize,
    pub visible_lanes: usize,
    pub row_height: f32,
}

impl TrackLayout {
    pub fn lane_at(self, point: Vec2) -> Option<usize> {
        if !self.rect.contains(point) {
            return None;
        }
        let row = ((point.y - self.rect.min.y) / self.row_height) as usize;
        (row < self.visible_lanes).then(|| self.top_lane - row)
    }

    pub fn lane_rect(self, lane: usize) -> Option<Rect> {
        let row = self.top_lane.checked_sub(lane)?;
        (row < self.visible_lanes).then(|| Rect::from_min_size(
            self.rect.min + Vec2::new(0.0, row as f32 * self.row_height),
            Vec2::new(self.rect.width(), self.row_height),
        ))
    }
}

#[derive(Default)]
pub struct TrackPreview {
    current: Option<(usize, Playback)>,
    prepared: Option<(usize, Playback)>,
    previous: Option<(usize, Arc<RgbaImage>)>,
    texture: Option<(u32, Weak<RgbaImage>)>,
}

impl TrackPreview {
    pub fn frame(&self) -> Option<(usize, &Arc<RgbaImage>)> {
        self.current.as_ref()
            .and_then(|(index, player)| player.frame.as_ref().map(|frame| (*index, frame)))
            .or_else(|| self.previous.as_ref().map(|(index, frame)| (*index, frame)))
    }

    pub fn upload_frame(&mut self, slot: u32) -> Option<Arc<RgbaImage>> {
        let (_, frame) = self.frame()?;
        let identity = Arc::downgrade(frame);
        if self.texture.as_ref().is_some_and(|(id, previous)| *id == slot && previous.ptr_eq(&identity)) {
            return None;
        }
        let frame = frame.clone();
        self.texture = Some((slot, identity));
        Some(frame)
    }

    fn update(&mut self, target: Option<(usize, &Clip)>, time: f64, fps: f64, playing: bool, seek: bool) {
        if seek {
            self.prepared = None;
            self.previous = None;
        }
        let Some((index, clip)) = target else {
            self.current = None;
            self.previous = None;
            return;
        };
        let source_time = clip.range.source_in + (time - clip.range.start).max(0.0);
        if let Some((current, player)) = &mut self.current && *current == index {
            player.set_fps(Some(fps));
            player.set_paused(!playing);
            if seek || playing && !player.frame_pending() && (player.position - source_time).abs() > 0.25_f64.max(2.0 / player.info.map_or(fps, |info| info.fps)) {
                player.seek(source_time);
            }
            return;
        }

        self.previous = if playing && !seek { self.frame().map(|(index, frame)| (index, frame.clone())) } else { None };
        let player = if playing && self.prepared.as_ref().is_some_and(|(next, _)| *next == index) {
            let (_, mut player) = self.prepared.take().unwrap();
            player.set_fps(Some(fps));
            player.set_paused(false);
            player
        } else {
            Playback::start_at((*clip.asset).clone(), source_time, !playing, Some(fps))
        };
        self.current = Some((index, player));
    }

    fn poll(&mut self) -> Option<String> {
        let error = self.current.as_mut().and_then(|(_, player)| player.poll());
        if self.current.as_ref().is_some_and(|(_, player)| player.frame.is_some()) {
            self.previous = None;
        }
        if let Some((_, player)) = &mut self.prepared {
            player.poll();
        }
        error
    }
}

#[derive(Default)]
pub struct VideoEditor {
    pub documents: Vec<VideoDocument>,
    pub active: Option<usize>,
    pub track: Option<TrackLayout>,
    pub drag: Option<ClipDrag>,
    pub scrubbing: bool,
    pub canvas_drag: Option<CanvasDrag>,
    pub playing: bool,
    pub previews: Vec<TrackPreview>,
    pub preview_opacity: Vec<f32>,
    next_name: usize,
}

impl VideoEditor {
    pub fn current(&self) -> Option<&VideoDocument> {
        self.active.and_then(|index| self.documents.get(index))
    }

    pub fn current_mut(&mut self) -> Option<&mut VideoDocument> {
        self.active.and_then(|index| self.documents.get_mut(index))
    }

    pub fn new_document(&mut self) -> usize {
        self.next_name += 1;
        self.documents.push(VideoDocument::new(format!("Video {}", self.next_name)));
        self.documents.len() - 1
    }

    pub fn close_view(&mut self) {
        self.active = None;
        self.track = None;
        self.drag = None;
        self.scrubbing = false;
        self.playing = false;
        self.canvas_drag = None;
        self.previews.clear();
    }

    pub fn open(&mut self, index: usize) {
        if self.active == Some(index) || index >= self.documents.len() {
            return;
        }
        self.close_view();
        self.active = Some(index);
        self.refresh_preview(true);
    }

    pub fn insert(&mut self, asset: &VideoAsset, lane: usize, seconds: f64, radius: f64) -> bool {
        let Some(doc) = self.current_mut() else { return false };
        let Some(range) = doc.drop_range(asset, lane, seconds, radius) else { return false };
        let index = doc.clips.len();
        doc.clips.push(Clip {
            asset: Arc::new(asset.clone()),
            source_duration: range.source_out,
            range,
            scale_percent: 100.0,
            position: Vec2::splat(0.5),
            effects: ClipEffects::default(),
        });
        doc.selected = Some(index);
        doc.duration = doc.duration.max(range.end());
        self.set_playing(false);
        self.seek(range.start);
        true
    }

    pub fn apply_edit(&mut self, index: usize, range: ClipRange) {
        let Some(doc) = self.current_mut() else { return };
        if let Some(clip) = doc.clips.get_mut(index) {
            if (clip.range.duration() - range.duration()).abs() > EPSILON {
                clip.effects.bake(range.duration(), doc.fps);
            }
            clip.range = range;
            doc.duration = doc.duration.max(range.end());
            self.refresh_preview(true);
        }
    }

    pub fn split(&mut self) {
        let Some(doc) = self.current_mut() else { return };
        let Some(index) = doc.selected else { return };
        let clip = &mut doc.clips[index];
        let cut = doc.playhead;
        if cut <= clip.range.start + EPSILON || cut >= clip.range.end() - EPSILON {
            return;
        }
        let source_cut = clip.range.source_in + (cut - clip.range.start);
        let mut right = Clip {
            range: ClipRange { start: cut, source_in: source_cut, ..clip.range },
            asset: clip.asset.clone(),
            source_duration: clip.source_duration,
            scale_percent: clip.scale_percent,
            position: clip.position,
            effects: clip.effects.duplicate_settings(),
        };
        clip.range.source_out = source_cut;
        clip.effects.bake(clip.range.duration(), doc.fps);
        right.effects.bake(right.range.duration(), doc.fps);
        doc.selected = Some(doc.clips.len());
        doc.clips.push(right);
        self.refresh_preview(true);
    }

    pub fn remove_selected(&mut self) {
        let Some(doc) = self.current_mut() else { return };
        if let Some(index) = doc.selected.take() {
            doc.clips.remove(index);
            self.previews.clear();
            self.refresh_preview(true);
        }
    }

    pub fn seek(&mut self, seconds: f64) {
        let Some(doc) = self.current_mut() else { return };
        doc.playhead = seconds.clamp(0.0, doc.duration);
        self.refresh_preview(true);
    }

    pub fn set_playing(&mut self, playing: bool) {
        if self.playing == playing {
            return;
        }
        if playing && self.current().is_some_and(|doc| doc.playhead >= doc.duration) {
            self.seek(0.0);
        }
        self.playing = playing;
        self.refresh_preview(false);
        if !playing {
            for track in &mut self.previews {
                track.prepared = None;
            }
        }
    }

    pub fn toggle_lane(&mut self, lane: usize) {
        let Some(doc) = self.current_mut() else { return };
        if !doc.hidden_lanes.remove(&lane) {
            doc.hidden_lanes.insert(lane);
        }
        self.refresh_preview(false);
    }

    pub fn refresh_preview(&mut self, seek: bool) {
        let Some(doc) = self.active.and_then(|index| self.documents.get(index)) else { return };
        self.previews.resize_with(doc.lane_count, TrackPreview::default);
        let mut targets = vec![None; doc.lane_count];
        for (index, clip) in doc.active_clips() {
            targets[clip.range.lane] = Some((index, clip));
        }
        for (lane, (track, target)) in self.previews.iter_mut().zip(targets).enumerate() {
            if !doc.lane_visible(lane) {
                // Retain the last upload identity until another frame uses the slot.
                track.current = None;
                track.prepared = None;
                track.previous = None;
                continue;
            }
            track.update(target, doc.playhead, f64::from(doc.fps), self.playing, seek);
        }
    }

    pub fn tick(&mut self, dt: f32) -> Option<String> {
        let mut error = None;
        for track in &mut self.previews {
            error = track.poll().or(error);
        }
        let waiting = self.previews.iter().any(|track| track.current.is_some())
            && self.previews.iter().filter_map(|track| track.current.as_ref())
                .all(|(_, player)| player.frame_pending());
        if self.playing && !waiting && let Some(doc) = self.current_mut() {
            doc.playhead = (doc.playhead + f64::from(dt.max(0.0))).min(doc.duration);
            self.playing = doc.playhead < doc.duration;
            self.refresh_preview(false);
        }
        if self.playing {
            self.prepare_next_previews();
        }
        error
    }

    fn prepare_next_previews(&mut self) {
        let Some(doc) = self.active.and_then(|index| self.documents.get(index)) else { return };
        let mut upcoming: Vec<Option<(usize, &Clip)>> = vec![None; self.previews.len()];
        for (index, clip) in doc.clips.iter().enumerate() {
            if doc.lane_visible(clip.range.lane)
                && clip.range.start > doc.playhead + EPSILON
                && clip.range.start - doc.playhead <= PREVIEW_LOOKAHEAD
            {
                let next = &mut upcoming[clip.range.lane];
                if next.is_none_or(|(_, earlier)| clip.range.start < earlier.range.start) {
                    *next = Some((index, clip));
                }
            }
        }
        for (track, next) in self.previews.iter_mut().zip(upcoming) {
            match next {
                Some((index, clip)) if track.prepared.as_ref().is_none_or(|(previous, _)| *previous != index) => {
                    track.prepared = Some((index, Playback::start_at(
                        (*clip.asset).clone(), clip.range.source_in, true, Some(f64::from(doc.fps)),
                    )));
                }
                None => track.prepared = None,
                _ => {}
            }
        }
    }

    pub fn needs_repaint(&self) -> bool {
        self.playing || self.previews.iter().filter_map(|track| track.current.as_ref())
            .any(|(_, player)| player.needs_repaint())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_cut_reuses_the_frame_prepared_during_playback() {
        let mut asset = VideoAsset::from_bytes(include_bytes!("../tests/fixtures/video-preview.mp4").to_vec());
        asset.info = Some(crate::video::VideoInfo { width: 64, height: 48, fps: 4.0, duration: 1.0 });
        let mut editor = VideoEditor::default();
        let index = editor.new_document();
        editor.open(index);
        assert!(editor.insert(&asset, 0, 0.0, 0.0));
        assert!(editor.insert(&asset, 0, 1.0, 0.0));
        editor.seek(0.5);
        editor.set_playing(true);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let ready_frame = loop {
            assert!(editor.tick(0.0).is_none());
            if let (Some((0, current)), Some((1, next))) = (&editor.previews[0].current, &editor.previews[0].prepared)
                && !current.frame_pending()
                && let Some(frame) = &next.frame
            {
                break frame.clone();
            }
            assert!(std::time::Instant::now() < deadline, "Video frames did not load");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(editor.tick(0.5).is_none());
        let (index, player) = editor.previews[0].current.as_ref().unwrap();
        assert_eq!(*index, 1);
        assert!(Arc::ptr_eq(player.frame.as_ref().unwrap(), &ready_frame));
        assert!(editor.playing);
    }

    #[test]
    fn tracks_preview_overlapping_clips_and_respect_visibility() {
        let mut asset = VideoAsset::from_bytes(include_bytes!("../tests/fixtures/video-preview.mp4").to_vec());
        asset.info = Some(crate::video::VideoInfo { width: 64, height: 48, fps: 4.0, duration: 1.0 });
        let mut editor = VideoEditor::default();
        let index = editor.new_document();
        editor.open(index);
        editor.current_mut().unwrap().lane_count = 2;
        assert!(editor.insert(&asset, 0, 1.0, 0.05));
        assert!(editor.insert(&asset, 1, 1.0, 0.05));
        assert!(editor.insert(&asset, 0, 2.0, 0.05));
        editor.seek(1.5);
        assert_eq!(editor.previews[0].current.as_ref().map(|(index, _)| *index), Some(0));
        assert_eq!(editor.previews[1].current.as_ref().map(|(index, _)| *index), Some(1));
        editor.toggle_lane(1);
        assert!(editor.previews[1].current.is_none());
        editor.toggle_lane(1);
        assert_eq!(editor.previews[1].current.as_ref().map(|(index, _)| *index), Some(1));

        let drag = ClipDrag { index: 1, edit: ClipEdit::Move, anchor: 1.0, candidate: None };
        let doc = editor.current().unwrap();
        assert!(doc.edit_range(&drag, 0, 1.0, 0.05).is_none());
        let range = doc.edit_range(&drag, 0, 2.0, 0.05);
        assert!(range.is_none());
        let range = doc.edit_range(&drag, 0, 3.0, 0.05).unwrap();
        editor.apply_edit(1, range);
        assert_eq!(editor.previews[0].current.as_ref().map(|(index, _)| *index), Some(0));
        assert!(editor.previews[1].current.is_none());
        assert_eq!(editor.current().unwrap().clips[1].range.lane, 0);
    }

    #[test]
    fn clip_drop_edit_and_split_keep_source_ranges_and_reject_overlap() {
        let mut asset = VideoAsset::from_bytes(include_bytes!("../tests/fixtures/video-preview.mp4").to_vec());
        asset.info = Some(crate::video::VideoInfo { width: 64, height: 48, fps: 4.0, duration: 1.0 });
        let mut editor = VideoEditor::default();
        let index = editor.new_document();
        editor.open(index);
        assert!(editor.insert(&asset, 0, 1.0, 0.05));
        assert!(!editor.insert(&asset, 0, 1.5, 0.05));
        assert!(editor.insert(&asset, 0, 2.02, 0.05));
        let doc = editor.current().unwrap();
        assert_eq!(doc.clips[0].size_in_document(), Some(Vec2::new(64.0, 48.0)));
        assert_eq!(doc.clips[1].range.start, 2.0);
        let drag = ClipDrag { index: 0, edit: ClipEdit::TrimStart, anchor: 1.0, candidate: None };
        let range = doc.edit_range(&drag, 0, 1.25, 0.05).unwrap();
        editor.apply_edit(0, range);
        let doc = editor.current_mut().unwrap();
        doc.selected = Some(0);
        doc.clips[0].scale_percent = 150.0;
        editor.seek(1.5);
        editor.split();
        let doc = editor.current().unwrap();
        assert_eq!(doc.clips[0].range.source_in, 0.25);
        assert_eq!(doc.clips[0].range.source_out, 0.5);
        assert_eq!(doc.clips[2].range.source_in, 0.5);
        assert_eq!(doc.clips[2].range.end(), doc.clips[1].range.start);
        assert_eq!(doc.clips[2].size_in_document(), Some(Vec2::new(96.0, 72.0)));
    }
}
