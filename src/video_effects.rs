use glam::Vec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    FadeIn,
    FadeOut,
    Move,
    TransformAnimation,
}

impl EffectKind {
    pub const ALL: [Self; 4] = [Self::FadeIn, Self::FadeOut, Self::Move, Self::TransformAnimation];

    pub fn label(self) -> &'static str {
        match self {
            Self::FadeIn => "Fade In",
            Self::FadeOut => "Fade Out",
            Self::Move => "Move",
            Self::TransformAnimation => "Transform Animation",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Easing {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Smoothstep,
}

impl Easing {
    pub const ALL: [Self; 5] = [Self::Linear, Self::EaseIn, Self::EaseOut, Self::EaseInOut, Self::Smoothstep];
    pub const LABELS: [&'static str; 5] = ["Linear", "Ease In", "Ease Out", "Ease In-Out", "Smoothstep"];

    pub fn label(self) -> &'static str {
        Self::LABELS[self.index()]
    }

    pub fn index(self) -> usize {
        self as usize
    }

    fn sample(self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t,
            Self::EaseOut => 1.0 - (1.0 - t).powi(2),
            Self::EaseInOut if t < 0.5 => 2.0 * t * t,
            Self::EaseInOut => 1.0 - 2.0 * (1.0 - t).powi(2),
            Self::Smoothstep => t * t * (3.0 - 2.0 * t),
        }
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct TransformKey {
    pub time: f64,
    pub offset: [f32; 2],
    pub scale: f32,
}

#[derive(Clone)]
pub struct ClipEffect {
    pub kind: EffectKind,
    pub enabled: bool,
    pub seconds: f32,
    pub easing: Easing,
    pub scale: f32,
    pub start_seconds: f32,
    pub from: Vec2,
    pub to: Vec2,
    pub keys: Vec<TransformKey>,
}

impl ClipEffect {
    pub fn new(kind: EffectKind) -> Self {
        Self {
            kind,
            enabled: true,
            seconds: 1.0,
            easing: Easing::Linear,
            scale: 1.0,
            start_seconds: 0.0,
            from: Vec2::new(-15.0, 0.0),
            to: Vec2::ZERO,
            keys: Vec::new(),
        }
    }

    pub fn settings_valid(&self) -> bool {
        self.seconds.is_finite() && self.seconds >= 0.0
            && match self.kind {
                EffectKind::Move => self.from.is_finite() && self.to.is_finite()
                    && self.start_seconds.is_finite() && self.start_seconds >= 0.0,
                EffectKind::TransformAnimation => self.keys.iter().all(|key| {
                    key.time.is_finite() && key.time >= 0.0 && key.scale.is_finite()
                        && key.scale > 0.0 && Vec2::from(key.offset).is_finite()
                }),
                _ => (0.01..=100.0).contains(&self.scale),
            }
    }

    pub fn key_at(&self, elapsed: f64, fps: i32) -> Option<usize> {
        let frame = frame_index(elapsed, f64::from(fps));
        self.keys.iter().position(|key| frame_index(key.time, f64::from(fps)) == frame)
    }

    fn animation_at(&self, time: f64) -> (Vec2, f32) {
        let right = self.keys.partition_point(|key| key.time <= time);
        match (right.checked_sub(1).and_then(|index| self.keys.get(index)), self.keys.get(right)) {
            (Some(from), Some(to)) => {
                let progress = self.easing.sample(((time - from.time) / (to.time - from.time)) as f32);
                (Vec2::from(from.offset).lerp(Vec2::from(to.offset), progress),
                    from.scale + (to.scale - from.scale) * progress)
            }
            (Some(key), None) | (None, Some(key)) => (key.offset.into(), key.scale),
            (None, None) => (Vec2::ZERO, 1.0),
        }
    }

    pub fn toggle_key(&mut self, elapsed: f64, fps: i32) {
        let time = frame_index(elapsed, f64::from(fps)) / f64::from(fps);
        if self.key_at(elapsed, fps).is_some() {
            self.keys.retain(|key| frame_index(key.time, f64::from(fps)) != frame_index(elapsed, f64::from(fps)));
        } else {
            let (offset, scale) = self.animation_at(time);
            let index = self.keys.partition_point(|key| key.time < time);
            self.keys.insert(index, TransformKey { time, offset: offset.into(), scale });
        }
    }

    pub fn move_key(&mut self, index: usize, elapsed: f64, fps: i32) -> bool {
        if index >= self.keys.len() || self.key_at(elapsed, fps).is_some() {
            return false;
        }
        let mut key = self.keys.remove(index);
        key.time = frame_index(elapsed, f64::from(fps)) / f64::from(fps);
        let destination = self.keys.partition_point(|other| other.time < key.time);
        self.keys.insert(destination, key);
        true
    }

    fn apply(&self, frame: &mut EffectFrame, time: f64, last_sample: f64) {
        if !self.enabled {
            return;
        }
        if self.kind == EffectKind::TransformAnimation {
            let (offset, scale) = self.animation_at(time);
            frame.offset += offset;
            frame.scale *= scale;
            return;
        }
        if self.kind == EffectKind::Move {
            let elapsed = time - f64::from(self.start_seconds);
            let progress = if self.seconds == 0.0 {
                if time as f32 >= self.start_seconds { 1.0 } else { 0.0 }
            } else {
                (elapsed / f64::from(self.seconds)).clamp(0.0, 1.0) as f32
            };
            let eased = self.easing.sample(progress);
            frame.offset += (self.from / 100.0) * (1.0 - eased) + (self.to / 100.0) * eased;
            return;
        }
        if self.seconds == 0.0 {
            return;
        }
        let seconds = f64::from(self.seconds);
        let fade_out = self.kind == EffectKind::FadeOut;
        let progress = if fade_out {
            1.0 - (last_sample - time) / seconds
        } else {
            time / seconds
        };
        let eased = self.easing.sample(progress.clamp(0.0, 1.0) as f32);
        let alpha = if fade_out { 1.0 - eased } else { eased };
        frame.opacity = frame.opacity.min(alpha);
        frame.scale *= alpha + self.scale * (1.0 - alpha);
    }
}

#[derive(Clone, Copy)]
pub struct EffectFrame {
    pub opacity: f32,
    pub scale: f32,
    pub offset: Vec2,
}

impl Default for EffectFrame {
    fn default() -> Self {
        Self { opacity: 1.0, scale: 1.0, offset: Vec2::ZERO }
    }
}

#[derive(Default)]
pub struct ClipEffects {
    entries: Vec<ClipEffect>,
    /// Effects of types this version does not know; kept verbatim so they survive a resave.
    pub(crate) unknown: Vec<crate::video_document_file::EffectData>,
    frames: Vec<EffectFrame>,
    last_sample: f64,
    fps: f64,
}

impl ClipEffects {
    pub fn duplicate_settings(&self) -> Self {
        Self { entries: self.entries.clone(), unknown: self.unknown.clone(), ..Self::default() }
    }

    pub fn entries(&self) -> &[ClipEffect] {
        &self.entries
    }

    pub fn transform_animation(&self) -> Option<&ClipEffect> {
        self.entries.iter().find(|effect| effect.kind == EffectKind::TransformAnimation && effect.enabled)
    }

    pub fn animated_transform(&self, elapsed: f64, fps: i32) -> (Vec2, f32) {
        let time = frame_index(elapsed, f64::from(fps)) / f64::from(fps);
        self.transform_animation().map_or((Vec2::ZERO, 1.0), |effect| effect.animation_at(time))
    }

    pub fn edit_transform_key(&mut self, index: usize, offset: Vec2, scale: f32) {
        if let Some(key) = self.entries.iter_mut()
            .find(|effect| effect.kind == EffectKind::TransformAnimation && effect.enabled)
            .and_then(|effect| effect.keys.get_mut(index))
        {
            key.offset = offset.into();
            key.scale = scale;
            self.frames.clear();
        }
    }

    pub fn set(&mut self, effect: ClipEffect, duration: f64, fps: i32) {
        self.merge(&[effect], duration, fps);
    }

    pub fn merge(&mut self, effects: &[ClipEffect], duration: f64, fps: i32) {
        for mut effect in effects.iter().filter(|effect| effect.settings_valid()).cloned() {
            effect.keys.sort_by(|a, b| a.time.total_cmp(&b.time));
            effect.keys.dedup_by(|a, b| a.time == b.time);
            if let Some(entry) = self.entries.iter_mut().find(|entry| entry.kind == effect.kind) {
                *entry = effect;
            } else {
                self.entries.push(effect);
            }
        }
        self.bake(duration, fps);
    }

    pub fn remove(&mut self, kind: EffectKind, duration: f64, fps: i32) {
        self.entries.retain(|entry| entry.kind != kind);
        self.bake(duration, fps);
    }

    pub fn bake(&mut self, duration: f64, fps: i32) {
        self.frames.clear();
        self.fps = f64::from(fps);
        let count = (duration * self.fps - 0.000_001).ceil().max(1.0) as usize;
        self.last_sample = (count - 1) as f64 / self.fps;
        if !self.entries.iter().any(|entry| entry.enabled && match entry.kind {
            EffectKind::TransformAnimation => !entry.keys.is_empty(),
            _ => entry.seconds > 0.0,
        }) {
            self.frames = Vec::new();
            return;
        }
        // Avoid unbounded allocations for imported clips with exceptional durations.
        if count > 3_000_000 || self.frames.try_reserve_exact(count).is_err() {
            self.frames = Vec::new();
            return;
        }
        let entries = &self.entries;
        let last_sample = self.last_sample;
        let fps = self.fps;
        self.frames.extend((0..count).map(|frame| resolve_frame(entries, frame as f64 / fps, last_sample)));
    }

    pub fn frame_at(&self, elapsed: f64) -> EffectFrame {
        if self.entries.is_empty() {
            return EffectFrame::default();
        }
        let frame = frame_index(elapsed, self.fps);
        if self.frames.is_empty() {
            let time = (frame / self.fps.max(1.0)).min(self.last_sample);
            return resolve_frame(&self.entries, time, self.last_sample);
        }
        self.frames[(frame as usize).min(self.frames.len() - 1)]
    }
}

fn frame_index(elapsed: f64, fps: f64) -> f64 {
    (elapsed.max(0.0) * fps + 0.000_001).floor()
}

fn resolve_frame(effects: &[ClipEffect], time: f64, last_sample: f64) -> EffectFrame {
    let mut frame = EffectFrame::default();
    for effect in effects {
        effect.apply(&mut frame, time, last_sample);
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_keys_toggle_on_the_frame_grid_and_round_trip_with_easing() {
        let mut stack = ClipEffects::default();
        let mut effect = ClipEffect::new(EffectKind::TransformAnimation);
        effect.easing = Easing::EaseIn;
        effect.toggle_key(0.51, 20);
        effect.toggle_key(1.51, 20);
        assert_eq!(effect.keys[0].time, 0.5);
        assert_eq!(effect.keys[1].time, 1.5);
        stack.set(effect, 3.0, 20);
        stack.edit_transform_key(1, Vec2::new(0.2, 0.0), 2.0);
        stack.bake(3.0, 20);

        let saved = stack.to_data();
        let mut restored = ClipEffects::from_data(&saved, 3.0, 20);
        assert_eq!(restored.to_data(), saved);
        assert_eq!(restored.frame_at(0.25).offset, Vec2::ZERO);
        assert_eq!(restored.frame_at(1.0).offset, Vec2::new(0.05, 0.0));
        assert_eq!(restored.frame_at(1.0).scale, 1.25);
        assert_eq!(restored.frame_at(2.0).scale, 2.0);

        let mut edit = restored.entries()[0].clone();
        edit.toggle_key(1.52, 20);
        assert_eq!(edit.keys.len(), 1);
        restored.set(edit, 3.0, 20);
        assert_eq!(restored.frame_at(2.0).scale, 1.0);
    }

    #[test]
    fn pasted_effects_replace_matching_types_and_bake_for_the_destination() {
        let mut source = ClipEffects::default();
        let mut fade = ClipEffect::new(EffectKind::FadeIn);
        fade.seconds = 2.0;
        fade.scale = 1.1;
        source.set(fade, 10.0, 20);
        let mut motion = ClipEffect::new(EffectKind::Move);
        motion.enabled = false;
        motion.start_seconds = 0.75;
        source.set(motion.clone(), 10.0, 20);
        let copied = source.entries().to_vec();
        source.remove(EffectKind::Move, 10.0, 20);

        let mut destination = ClipEffects::default();
        destination.set(ClipEffect::new(EffectKind::FadeOut), 5.0, 30);
        destination.set(ClipEffect::new(EffectKind::FadeIn), 5.0, 30);
        destination.merge(&copied, 5.0, 30);
        assert_eq!(destination.entries().len(), 3);
        assert_eq!(destination.entries()[0].kind, EffectKind::FadeOut);
        assert_eq!(destination.entries()[1].seconds, 2.0);
        assert_eq!(destination.entries()[1].scale, 1.1);
        assert!(!destination.entries()[2].enabled);
        assert_eq!(destination.entries()[2].start_seconds, 0.75);
        assert_eq!(destination.frames.len(), 150);
        assert_eq!(destination.frame_at(1.0).opacity, 0.5);
        assert_eq!(destination.frame_at(4.99).opacity, 0.0);
    }

    #[test]
    fn move_uses_easing_and_holds_the_destination_without_changing_fade() {
        let mut effects = ClipEffects::default();
        let mut motion = ClipEffect::new(EffectKind::Move);
        motion.from = Vec2::new(-20.0, 0.0);
        motion.to = Vec2::new(20.0, 40.0);
        motion.easing = Easing::EaseIn;
        effects.set(motion.clone(), 3.0, 20);
        effects.set(ClipEffect::new(EffectKind::FadeIn), 3.0, 20);
        assert_eq!(effects.frame_at(0.0).offset, Vec2::new(-0.2, 0.0));
        let halfway = effects.frame_at(0.5);
        assert!(halfway.offset.distance(Vec2::new(-0.1, 0.1)) < 0.000_01);
        assert_eq!(halfway.opacity, 0.5);
        assert_eq!(effects.frame_at(2.0).offset, Vec2::new(0.2, 0.4));
        motion.start_seconds = 0.5;
        effects.set(motion.clone(), 3.0, 20);
        assert_eq!(effects.frame_at(0.25).offset, Vec2::new(-0.2, 0.0));
        assert_eq!(effects.frame_at(0.5).offset, Vec2::new(-0.2, 0.0));
        assert!(effects.frame_at(1.0).offset.distance(Vec2::new(-0.1, 0.1)) < 0.000_01);
        assert_eq!(effects.frame_at(1.5).offset, Vec2::new(0.2, 0.4));
        motion.seconds = 0.0;
        effects.set(motion.clone(), 3.0, 20);
        assert_eq!(effects.frame_at(0.25).offset, Vec2::new(-0.2, 0.0));
        assert_eq!(effects.frame_at(0.5).offset, Vec2::new(0.2, 0.4));
        motion.enabled = false;
        effects.set(motion.clone(), 3.0, 20);
        assert_eq!(effects.frame_at(0.0).offset, Vec2::ZERO);
    }

    #[test]
    fn fades_bake_on_the_document_frame_grid_and_compose_as_opacity_limits() {
        let mut effects = ClipEffects::default();
        let fade_in = ClipEffect::new(EffectKind::FadeIn);
        let fade_out = ClipEffect::new(EffectKind::FadeOut);
        effects.set(fade_in.clone(), 10.0, 20);
        effects.set(fade_out.clone(), 10.0, 20);
        assert_eq!(effects.frames.len(), 200);
        assert_eq!(effects.frame_at(0.0).opacity, 0.0);
        assert_eq!(effects.frame_at(0.5).opacity, 0.5);
        assert_eq!(effects.frame_at(5.0).opacity, 1.0);
        assert_eq!(effects.frame_at(9.95).opacity, 0.0);

        effects.bake(1.05, 20);
        assert_eq!(effects.frame_at(0.5).opacity, 0.5);
        effects.set(ClipEffect { enabled: false, ..fade_in }, 1.05, 20);
        assert_eq!(effects.frame_at(0.0).opacity, 1.0);
        effects.remove(EffectKind::FadeOut, 1.05, 20);
        assert_eq!(effects.frame_at(1.0).opacity, 1.0);
    }

    #[test]
    fn fade_scale_follows_each_eased_edge_and_is_neutral_when_disabled() {
        let mut effects = ClipEffects::default();
        let mut fade_in = ClipEffect::new(EffectKind::FadeIn);
        let mut fade_out = ClipEffect::new(EffectKind::FadeOut);
        effects.set(fade_in.clone(), 3.25, 4);
        assert_eq!(effects.frame_at(0.0).scale, 1.0);
        fade_in.scale = 1.1;
        fade_out.scale = 1.2;
        fade_out.easing = Easing::EaseIn;
        effects.set(fade_in.clone(), 3.25, 4);
        effects.set(fade_out.clone(), 3.25, 4);
        assert_eq!(effects.frame_at(0.0).scale, 1.1);
        assert!((effects.frame_at(0.5).scale - 1.05).abs() < 0.000_01);
        assert_eq!(effects.frame_at(1.5).scale, 1.0);
        assert!((effects.frame_at(2.5).scale - 1.05).abs() < 0.000_01);
        assert_eq!(effects.frame_at(3.0).scale, 1.2);
        fade_in.enabled = false;
        effects.set(fade_in.clone(), 3.25, 4);
        assert_eq!(effects.frame_at(0.0).scale, 1.0);
    }
}
