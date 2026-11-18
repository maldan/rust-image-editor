//! On-disk form of a video document (`DocumentMeta.video`, see docs/project-format.md).
//! Plain data only: clips reference video assets by id, and view state is not stored.

use crate::video_effects::{ClipEffect, ClipEffects, EffectKind, Easing};
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// Schema version of the `video` object, independent of `DocumentMeta.version`.
pub const VIDEO_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl From<Vec2> for Point {
    fn from(value: Vec2) -> Self {
        Self { x: value.x, y: value.y }
    }
}

impl From<Point> for Vec2 {
    fn from(value: Point) -> Self {
        Vec2::new(value.x, value.y)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoData {
    pub version: u32,
    pub fps: i32,
    pub duration: f64,
    pub lane_count: usize,
    #[serde(default)]
    pub clips: Vec<ClipData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipData {
    /// `VideoMeta.id` of the source video.
    pub asset: String,
    pub lane: usize,
    pub start: f64,
    pub source_in: f64,
    pub source_out: f64,
    pub scale_percent: f32,
    pub position: Point,
    #[serde(default)]
    pub effects: Vec<EffectData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectData {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    pub data: serde_json::Value,
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FadeData {
    seconds: f32,
    #[serde(default)]
    easing: Easing,
    scale: f32,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveData {
    start_seconds: f32,
    seconds: f32,
    #[serde(default)]
    easing: Easing,
    from: Point,
    to: Point,
}

/// A video document as stored in a manifest entry of `type = "video"`.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoRecord {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub video: VideoData,
}

impl VideoData {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != VIDEO_VERSION {
            return Err(format!("Unsupported video document version {}", self.version));
        }
        if !(1..=240).contains(&self.fps)
            || !self.duration.is_finite()
            || self.duration < 0.0
            || !(1..=1024).contains(&self.lane_count)
        {
            return Err("Invalid video document settings".into());
        }
        for clip in &self.clips {
            if clip.lane >= self.lane_count
                || !clip.start.is_finite()
                || clip.start < 0.0
                || !clip.source_in.is_finite()
                || clip.source_in < 0.0
                || !clip.source_out.is_finite()
                || clip.source_out <= clip.source_in
                || !clip.scale_percent.is_finite()
                || clip.scale_percent <= 0.0
                || !clip.position.x.is_finite()
                || !clip.position.y.is_finite()
            {
                return Err("Invalid video clip".into());
            }
        }
        Ok(())
    }
}

enum Parsed {
    Effect(ClipEffect),
    Invalid,
    Unknown,
}

fn parse_effect(item: &EffectData) -> Parsed {
    let fade = |kind| {
        match serde_json::from_value::<FadeData>(item.data.clone()) {
            Ok(data) => Parsed::Effect(ClipEffect {
                enabled: item.enabled,
                seconds: data.seconds,
                easing: data.easing,
                scale: data.scale,
                ..ClipEffect::new(kind)
            }),
            Err(_) => Parsed::Invalid,
        }
    };
    match item.kind.as_str() {
        "fadeIn" => fade(EffectKind::FadeIn),
        "fadeOut" => fade(EffectKind::FadeOut),
        "move" => match serde_json::from_value::<MoveData>(item.data.clone()) {
            Ok(data) => Parsed::Effect(ClipEffect {
                enabled: item.enabled,
                seconds: data.seconds,
                easing: data.easing,
                start_seconds: data.start_seconds,
                from: data.from.into(),
                to: data.to.into(),
                ..ClipEffect::new(EffectKind::Move)
            }),
            Err(_) => Parsed::Invalid,
        },
        _ => Parsed::Unknown,
    }
}

fn effect_data(effect: &ClipEffect) -> EffectData {
    let (kind, data) = match effect.kind {
        EffectKind::FadeIn | EffectKind::FadeOut => (
            if effect.kind == EffectKind::FadeIn { "fadeIn" } else { "fadeOut" },
            serde_json::to_value(FadeData { seconds: effect.seconds, easing: effect.easing, scale: effect.scale }),
        ),
        EffectKind::Move => (
            "move",
            serde_json::to_value(MoveData {
                start_seconds: effect.start_seconds,
                seconds: effect.seconds,
                easing: effect.easing,
                from: effect.from.into(),
                to: effect.to.into(),
            }),
        ),
    };
    EffectData { kind: kind.into(), enabled: effect.enabled, data: data.unwrap_or_default() }
}

impl ClipEffects {
    pub fn to_data(&self) -> Vec<EffectData> {
        self.entries().iter().map(effect_data).chain(self.unknown.iter().cloned()).collect()
    }

    /// Invalid effects are dropped; unknown types are kept untouched so they survive a resave.
    pub fn from_data(list: &[EffectData], duration: f64, fps: i32) -> Self {
        let mut effects = Self::default();
        let mut known = Vec::new();
        for item in list {
            match parse_effect(item) {
                Parsed::Effect(effect) => known.push(effect),
                Parsed::Invalid => {}
                Parsed::Unknown => effects.unknown.push(item.clone()),
            }
        }
        effects.merge(&known, duration, fps);
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn effects_round_trip_and_unknown_and_invalid_are_handled() {
        let list: Vec<EffectData> = serde_json::from_value(json!([
            { "type": "fadeIn", "data": { "seconds": 1.0, "easing": "smoothstep", "scale": 0.5 } },
            { "type": "move", "enabled": false, "data": {
                "startSeconds": 0.5, "seconds": 2.0, "easing": "easeInOut",
                "from": { "x": -15, "y": 0 }, "to": { "x": 0, "y": 3 } } },
            { "type": "fadeOut", "data": { "seconds": "bad" } },
            { "type": "fadeOut", "data": { "seconds": 1.0, "scale": 1000.0 } },
            { "type": "sparkle", "data": { "amount": 3 } }
        ])).unwrap();
        let effects = ClipEffects::from_data(&list, 4.0, 24);
        assert_eq!(effects.entries().len(), 2);
        let fade = effects.entries()[0];
        assert_eq!((fade.kind, fade.easing, fade.scale), (EffectKind::FadeIn, Easing::Smoothstep, 0.5));
        let motion = effects.entries()[1];
        assert!(!motion.enabled);
        assert_eq!((motion.start_seconds, motion.to), (0.5, Vec2::new(0.0, 3.0)));

        let saved = effects.to_data();
        assert_eq!(saved.len(), 3);
        assert_eq!(saved[0].kind, "fadeIn");
        assert_eq!(saved[0].data["easing"], "smoothstep");
        assert_eq!(saved[1].kind, "move");
        assert_eq!(saved[1].data["startSeconds"], 0.5);
        assert_eq!(saved[2], list[4]);
        assert_eq!(ClipEffects::from_data(&saved, 4.0, 24).to_data(), saved);
    }

    #[test]
    fn validation_rejects_unknown_versions_and_broken_clips() {
        let clip = ClipData {
            asset: "v_1".into(),
            lane: 0,
            start: 0.0,
            source_in: 0.0,
            source_out: 1.0,
            scale_percent: 100.0,
            position: Point { x: 0.5, y: 0.5 },
            effects: Vec::new(),
        };
        let data = VideoData { version: VIDEO_VERSION, fps: 24, duration: 5.0, lane_count: 1, clips: vec![clip] };
        assert!(data.validate().is_ok());
        assert!(VideoData { version: 2, ..data.clone() }.validate().is_err());
        assert!(VideoData { fps: 0, ..data.clone() }.validate().is_err());
        let mut broken = data.clone();
        broken.clips[0].lane = 1;
        assert!(broken.validate().is_err());
        let mut broken = data;
        broken.clips[0].source_out = 0.0;
        assert!(broken.validate().is_err());
    }
}
