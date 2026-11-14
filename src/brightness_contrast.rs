use crate::raster::SparsePixels;

/// Brightness and contrast in -100..=100, applied to 8-bit sRGB; alpha is preserved.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BrightnessContrast {
    pub brightness: f32,
    pub contrast: f32,
}

impl BrightnessContrast {
    pub fn is_identity(self) -> bool {
        self == Self::default()
    }

    pub fn pixel(self, pixel: [u8; 4]) -> [u8; 4] {
        if pixel[3] == 0 || self.is_identity() {
            return pixel;
        }
        let factor = (1.0 + self.contrast.clamp(-100.0, 100.0) / 100.0).powi(2);
        let brightness = self.brightness.clamp(-100.0, 100.0) / 100.0;
        let mut result = pixel;
        for channel in &mut result[..3] {
            let contrasted = ((*channel as f32 / 255.0 - 0.5) * factor + 0.5).clamp(0.0, 1.0);
            *channel = ((contrasted + brightness).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        result
    }

    pub fn apply(self, pixels: &mut SparsePixels) -> bool {
        if self.is_identity() {
            return false;
        }
        let mut changed = false;
        for tile in pixels.tiles.values_mut() {
            let Some(first) = tile
                .chunks_exact(4)
                .position(|p| self.pixel(p.try_into().unwrap()) != p)
            else {
                continue;
            };
            for pixel in std::sync::Arc::make_mut(tile)[first * 4..].chunks_exact_mut(4) {
                pixel.copy_from_slice(&self.pixel((&*pixel).try_into().unwrap()));
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
    fn brightness_contrast_extremes_preserve_alpha_and_hidden_rgb() {
        assert_eq!(
            BrightnessContrast {
                brightness: 100.0,
                contrast: 100.0
            }
            .pixel([20, 80, 160, 77]),
            [255, 255, 255, 77]
        );
        assert_eq!(
            BrightnessContrast {
                brightness: -100.0,
                contrast: 100.0
            }
            .pixel([20, 80, 160, 77]),
            [0, 0, 0, 77]
        );
        assert_eq!(
            BrightnessContrast {
                brightness: 0.0,
                contrast: -100.0
            }
            .pixel([20, 80, 160, 77]),
            [128, 128, 128, 77]
        );
        assert_eq!(
            BrightnessContrast {
                brightness: 25.0,
                contrast: 0.0
            }
            .pixel([64, 128, 192, 255]),
            [128, 192, 255, 255]
        );
        assert_eq!(
            BrightnessContrast {
                brightness: 100.0,
                contrast: 100.0
            }
            .pixel([20, 80, 160, 0]),
            [20, 80, 160, 0]
        );
    }

    #[test]
    fn sparse_adjustment_keeps_offsets_and_identity_does_not_copy_tiles() {
        let mut pixels = SparsePixels::default();
        pixels.write(0, -10000, 20000, [64, 128, 192, 128]);
        pixels.offset = (13, -7);
        let original = pixels.clone();
        assert!(!BrightnessContrast::default().apply(&mut pixels));
        for (key, tile) in &original.tiles {
            assert!(std::sync::Arc::ptr_eq(tile, &pixels.tiles[key]));
        }
        assert!(
            BrightnessContrast {
                brightness: 25.0,
                contrast: 0.0
            }
            .apply(&mut pixels)
        );
        assert_eq!(pixels.offset, original.offset);
        assert_eq!(pixels.tiles.len(), original.tiles.len());
        assert_eq!(pixels.read(0, -9987, 19993), [128, 192, 255, 128]);
        assert_eq!(original.read(0, -9987, 19993), [64, 128, 192, 128]);
    }
}
