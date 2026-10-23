use crate::raster::SparsePixels;

/// Hue in degrees, saturation/value adjustments in percent. RGB is sRGB here,
/// matching an ordinary 8-bit paint layer; alpha is never adjusted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HsvAdjustment {
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
}

impl HsvAdjustment {
    pub fn is_identity(self) -> bool {
        self == Self::default()
    }

    pub fn pixel(self, pixel: [u8; 4]) -> [u8; 4] {
        if pixel[3] == 0 || self.is_identity() {
            return pixel;
        }
        let [r, g, b] = [pixel[0], pixel[1], pixel[2]].map(|c| c as f32 / 255.0);
        let high = r.max(g).max(b);
        let mut chroma = high - r.min(g).min(b);
        let mut hue = 0.0;
        if chroma > 1.0e-6 {
            hue = if r == high {
                (g - b) / chroma
            } else if g == high {
                2.0 + (b - r) / chroma
            } else {
                4.0 + (r - g) / chroma
            };
            hue = (hue + self.hue.clamp(-180.0, 180.0) / 60.0).rem_euclid(6.0);
        }
        // Modern Krita-style adjustment: preserve neutral grays, change chroma,
        // and move value toward white/black without flattening tonal variation.
        // Reference: KDE/krita plugins/color/colorspaceextensions/kis_hsv_adjustment.cpp
        let saturation = self.saturation.clamp(-100.0, 100.0) / 100.0;
        let gain = if saturation > 0.0 {
            1.0 + saturation + 2.0 * saturation * saturation
        } else {
            1.0 + saturation
        };
        chroma = (chroma * gain).min(1.0);
        let value = self.value.clamp(-100.0, 100.0) / 100.0;
        let v = high * (1.0 - value.abs()) + value.max(0.0);
        chroma = (chroma * (1.0 - value.abs())).min(v);
        let x = chroma * (1.0 - ((hue % 2.0) - 1.0).abs());
        let rgb = match hue as u32 {
            0 => [chroma, x, 0.0],
            1 => [x, chroma, 0.0],
            2 => [0.0, chroma, x],
            3 => [0.0, x, chroma],
            4 => [x, 0.0, chroma],
            _ => [chroma, 0.0, x],
        }
        .map(|c| ((c + v - chroma).clamp(0.0, 1.0) * 255.0).round() as u8);
        [rgb[0], rgb[1], rgb[2], pixel[3]]
    }

    pub fn apply(self, pixels: &mut SparsePixels) -> bool {
        if self.is_identity() {
            return false;
        }
        let mut changed = false;
        for tile in pixels.tiles.values_mut() {
            let first = tile
                .chunks_exact(4)
                .position(|p| self.pixel(p.try_into().unwrap()) != p);
            let Some(first) = first else {
                continue;
            };
            let data = std::sync::Arc::make_mut(tile);
            for pixel in data[first * 4..].chunks_exact_mut(4) {
                let adjusted = self.pixel((&*pixel).try_into().unwrap());
                pixel.copy_from_slice(&adjusted);
            }
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Raster;

    #[test]
    fn hsv_adjusts_hue_saturation_value_and_preserves_alpha() {
        assert_eq!(
            HsvAdjustment {
                hue: 120.0,
                ..Default::default()
            }
            .pixel([255, 0, 0, 123]),
            [0, 255, 0, 123]
        );
        assert_eq!(
            HsvAdjustment {
                saturation: -100.0,
                ..Default::default()
            }
            .pixel([20, 80, 160, 77]),
            [160, 160, 160, 77]
        );
        assert_eq!(
            HsvAdjustment {
                value: 100.0,
                ..Default::default()
            }
            .pixel([20, 80, 160, 77]),
            [255, 255, 255, 77]
        );
        assert_eq!(
            HsvAdjustment {
                value: -100.0,
                ..Default::default()
            }
            .pixel([20, 80, 160, 77]),
            [0, 0, 0, 77]
        );
        assert_eq!(
            HsvAdjustment {
                saturation: 100.0,
                hue: 90.0,
                value: 0.0
            }
            .pixel([80, 80, 80, 255]),
            [80, 80, 80, 255]
        );
        assert_eq!(
            HsvAdjustment {
                value: 50.0,
                ..Default::default()
            }
            .pixel([0, 0, 0, 255]),
            [128, 128, 128, 255]
        );
    }

    #[test]
    fn hsv_preserves_sparse_offsets_hidden_rgb_and_identity_tile_sharing() {
        let mut pixels = SparsePixels::default();
        pixels.write(0, -100000, 200000, [255, 0, 0, 128]);
        pixels.write(0, 0, 0, [23, 45, 67, 0]);
        pixels.offset = (13, -7);
        let original = pixels.clone();
        assert!(!HsvAdjustment::default().apply(&mut pixels));
        for (key, tile) in &original.tiles {
            assert!(std::sync::Arc::ptr_eq(tile, &pixels.tiles[key]));
        }
        assert!(
            HsvAdjustment {
                hue: 120.0,
                ..Default::default()
            }
            .apply(&mut pixels)
        );
        assert_eq!(pixels.offset, original.offset);
        assert_eq!(pixels.tiles.len(), original.tiles.len());
        assert_eq!(pixels.read(0, -99987, 199993), [0, 255, 0, 128]);
        assert_eq!(pixels.read(0, 13, -7), [23, 45, 67, 0]);
        assert_eq!(original.read(0, -99987, 199993), [255, 0, 0, 128]);
    }
}
