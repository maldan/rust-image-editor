use glam::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum Transition {
    Hold,
    #[default]
    Linear,
    Smooth,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraKey {
    pub pan: [f32; 2],
    pub zoom: f32,
    /// 0 is the scene, 1 is full black. Animate either direction for fades.
    pub fade: f32,
    /// Transition from this key to the next.
    pub transition: Transition,
}

impl Default for CameraKey {
    fn default() -> Self {
        Self {
            pan: [0.0; 2],
            zoom: 1.0,
            fade: 0.0,
            transition: Transition::Linear,
        }
    }
}

impl CameraKey {
    pub fn valid(&self) -> bool {
        self.pan
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1_000_000.0)
            && self.zoom.is_finite()
            && (0.01..=100.0).contains(&self.zoom)
            && self.fade.is_finite()
            && (0.0..=1.0).contains(&self.fade)
    }

    /// Positive Z recedes. At neutral camera settings all depths retain their layout.
    pub fn layer_transform(&self, z: f32) -> (Vec2, f32) {
        let depth = 2.0_f32.powf(-z.clamp(-10.0, 10.0));
        let scale = self.zoom.powf(depth).clamp(0.001, 1000.0);
        (-Vec2::from(self.pan) * depth * scale, scale)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraTrack {
    #[serde(default)]
    pub visible: bool,
    pub keys: BTreeMap<u32, CameraKey>,
}

impl Default for CameraTrack {
    fn default() -> Self {
        Self {
            visible: false,
            keys: BTreeMap::from([(0, CameraKey::default())]),
        }
    }
}

impl CameraTrack {
    pub fn sample(&self, frame: u32) -> CameraKey {
        let Some((&start, &a)) = self.keys.range(..=frame).next_back() else {
            return self
                .keys
                .first_key_value()
                .map(|(_, k)| *k)
                .unwrap_or_default();
        };
        let Some((&end, &b)) = self
            .keys
            .range((std::ops::Bound::Excluded(start), std::ops::Bound::Unbounded))
            .next()
        else {
            return a;
        };
        let t = (frame - start) as f32 / (end - start) as f32;
        let t = match a.transition {
            Transition::Hold => 0.0,
            Transition::Linear => t,
            Transition::Smooth => t * t * (3.0 - 2.0 * t),
        };
        let lerp = |x, y| x + (y - x) * t;
        CameraKey {
            pan: [lerp(a.pan[0], b.pan[0]), lerp(a.pan[1], b.pan[1])],
            zoom: lerp(a.zoom, b.zoom),
            fade: lerp(a.fade, b.fade),
            transition: a.transition,
        }
    }
}

/// CPU path for file export only; interactive playback transforms the GPU quads.
pub fn render_frame(project: &crate::project::Project, key: CameraKey) -> Vec<u8> {
    use crate::raster::Raster;
    let center = Vec2::new(project.width as f32, project.height as f32) * 0.5;
    let transforms: Vec<_> = project
        .layers
        .iter()
        .map(|l| key.layer_transform(l.z))
        .collect();
    let linear = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let srgb = |v: f32| {
        let v = if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    let mut pixels = vec![0; project.width as usize * project.height as usize * 4];
    for y in 0..project.height {
        for x in 0..project.width {
            let target = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let mut color = [0.0; 3];
            for (layer, &(offset, scale)) in project.layers.iter().zip(&transforms) {
                if !layer.visible {
                    continue;
                }
                let source = (target - center - offset) / scale + center;
                if source.x < 0.0
                    || source.y < 0.0
                    || source.x >= project.width as f32
                    || source.y >= project.height as f32
                {
                    continue;
                }
                let p = layer
                    .pixels
                    .read(0, source.x.floor() as i32, source.y.floor() as i32);
                let alpha = p[3] as f32 / 255.0 * layer.opacity.clamp(0.0, 1.0);
                for c in 0..3 {
                    color[c] = linear(p[c]) * alpha + color[c] * (1.0 - alpha);
                }
            }
            let i = (y as usize * project.width as usize + x as usize) * 4;
            for c in 0..3 {
                pixels[i + c] = srgb(color[c] * (1.0 - key.fade));
            }
            pixels[i + 3] = 255;
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_holds_endpoints_and_fades_both_ways() {
        let mut track = CameraTrack::default();
        track.keys.insert(
            10,
            CameraKey {
                pan: [100.0, -20.0],
                zoom: 3.0,
                fade: 1.0,
                ..Default::default()
            },
        );
        assert_eq!(track.sample(5).pan, [50.0, -10.0]);
        assert_eq!(track.sample(5).zoom, 2.0);
        assert_eq!(track.sample(5).fade, 0.5);
        assert_eq!(track.sample(100), track.keys[&10]);
        track.keys.get_mut(&0).unwrap().transition = Transition::Hold;
        assert_eq!(track.sample(9).zoom, 1.0);
        assert_eq!(track.sample(10).zoom, 3.0);
        track.keys.get_mut(&0).unwrap().transition = Transition::Smooth;
        assert!(track.sample(2).pan[0] < 20.0);
        track.keys.insert(20, CameraKey::default());
        assert_eq!(track.sample(15).fade, 0.5);
    }

    #[test]
    fn depth_only_affects_camera_motion() {
        for z in [-2.0, 0.0, 2.0] {
            assert_eq!(CameraKey::default().layer_transform(z), (Vec2::ZERO, 1.0));
        }
        let k = CameraKey {
            pan: [100.0, 0.0],
            zoom: 2.0,
            ..Default::default()
        };
        assert!(k.layer_transform(2.0).0.length() < k.layer_transform(0.0).0.length());
        assert!(k.layer_transform(2.0).1 < k.layer_transform(0.0).1);
    }
}
