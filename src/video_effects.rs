use glam::Vec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    FadeIn,
    FadeOut,
    Move,
}

impl EffectKind {
    pub const ALL: [Self; 3] = [Self::FadeIn, Self::FadeOut, Self::Move];

    pub fn label(self) -> &'static str {
        match self {
            Self::FadeIn => "Fade In",
            Self::FadeOut => "Fade Out",
            Self::Move => "Move",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

#[derive(Clone, Copy)]
pub struct ClipEffect {
    pub kind: EffectKind,
    pub enabled: bool,
    pub seconds: f32,
    pub easing: Easing,
    pub scale: f32,
    pub from: Vec2,
    pub to: Vec2,
}

impl ClipEffect {
    pub fn new(kind: EffectKind) -> Self {
        Self {
            kind,
            enabled: true,
            seconds: 1.0,
            easing: Easing::Linear,
            scale: 1.0,
            from: Vec2::new(-15.0, 0.0),
            to: Vec2::ZERO,
        }
    }

    pub fn settings_valid(self) -> bool {
        self.seconds.is_finite() && self.seconds >= 0.0
            && match self.kind {
                EffectKind::Move => self.from.is_finite() && self.to.is_finite(),
                _ => (0.01..=100.0).contains(&self.scale),
            }
    }

    fn apply(self, frame: &mut EffectFrame, time: f64, last_sample: f64) {
        if !self.enabled {
            return;
        }
        if self.kind == EffectKind::Move {
            let progress = if self.seconds == 0.0 { 1.0 } else { (time / f64::from(self.seconds)).clamp(0.0, 1.0) as f32 };
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
    frames: Vec<EffectFrame>,
    last_sample: f64,
    fps: f64,
}

impl ClipEffects {
    pub fn duplicate_settings(&self) -> Self {
        Self { entries: self.entries.clone(), ..Self::default() }
    }

    pub fn entries(&self) -> &[ClipEffect] {
        &self.entries
    }

    pub fn set(&mut self, effect: ClipEffect, duration: f64, fps: i32) {
        if !effect.settings_valid() {
            return;
        }
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.kind == effect.kind) {
            *entry = effect;
        } else {
            self.entries.push(effect);
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
        if !self.entries.iter().any(|entry| entry.enabled && entry.seconds > 0.0) {
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
        let frame = (elapsed.max(0.0) * self.fps + 0.000_001).floor();
        if self.frames.is_empty() {
            let time = (frame / self.fps.max(1.0)).min(self.last_sample);
            return resolve_frame(&self.entries, time, self.last_sample);
        }
        self.frames[(frame as usize).min(self.frames.len() - 1)]
    }
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
    fn move_uses_easing_and_holds_the_destination_without_changing_fade() {
        let mut effects = ClipEffects::default();
        let mut motion = ClipEffect::new(EffectKind::Move);
        motion.from = Vec2::new(-20.0, 0.0);
        motion.to = Vec2::new(20.0, 40.0);
        motion.easing = Easing::EaseIn;
        effects.set(motion, 3.0, 20);
        effects.set(ClipEffect::new(EffectKind::FadeIn), 3.0, 20);
        assert_eq!(effects.frame_at(0.0).offset, Vec2::new(-0.2, 0.0));
        let halfway = effects.frame_at(0.5);
        assert!(halfway.offset.distance(Vec2::new(-0.1, 0.1)) < 0.000_01);
        assert_eq!(halfway.opacity, 0.5);
        assert_eq!(effects.frame_at(2.0).offset, Vec2::new(0.2, 0.4));
        motion.seconds = 0.0;
        effects.set(motion, 3.0, 20);
        assert_eq!(effects.frame_at(0.0).offset, Vec2::new(0.2, 0.4));
        motion.enabled = false;
        effects.set(motion, 3.0, 20);
        assert_eq!(effects.frame_at(0.0).offset, Vec2::ZERO);
    }

    #[test]
    fn fades_bake_on_the_document_frame_grid_and_compose_as_opacity_limits() {
        let mut effects = ClipEffects::default();
        let fade_in = ClipEffect::new(EffectKind::FadeIn);
        let fade_out = ClipEffect::new(EffectKind::FadeOut);
        effects.set(fade_in, 10.0, 20);
        effects.set(fade_out, 10.0, 20);
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
        effects.set(fade_in, 3.25, 4);
        assert_eq!(effects.frame_at(0.0).scale, 1.0);
        fade_in.scale = 1.1;
        fade_out.scale = 1.2;
        fade_out.easing = Easing::EaseIn;
        effects.set(fade_in, 3.25, 4);
        effects.set(fade_out, 3.25, 4);
        assert_eq!(effects.frame_at(0.0).scale, 1.1);
        assert!((effects.frame_at(0.5).scale - 1.05).abs() < 0.000_01);
        assert_eq!(effects.frame_at(1.5).scale, 1.0);
        assert!((effects.frame_at(2.5).scale - 1.05).abs() < 0.000_01);
        assert_eq!(effects.frame_at(3.0).scale, 1.2);
        fade_in.enabled = false;
        effects.set(fade_in, 3.25, 4);
        assert_eq!(effects.frame_at(0.0).scale, 1.0);
    }
}
