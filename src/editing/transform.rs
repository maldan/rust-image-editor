use super::composite::over;
use super::selection::{PixelRect, Selection};
use crate::raster::Raster;

fn lanczos3(x: f32) -> f32 {
    let x = x.abs();
    if x < 1.0e-5 {
        return 1.0;
    }
    if x >= 3.0 {
        return 0.0;
    }
    let p = std::f32::consts::PI * x;
    p.sin() * (p / 3.0).sin() / (p * p / 3.0)
}

fn linear_to_srgb(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let value = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (value * 255.0).round() as u8
}

/// Scale-aware Lanczos3 in linear light and premultiplied alpha. Missing source
/// pixels are transparent; the selection is filtered together with the image.
/// Widening the kernel when shrinking is essential to suppress aliasing.
fn sample_transform(
    x: f32,
    y: f32,
    scale: glam::Vec2,
    read: impl Fn(i32, i32) -> ([u8; 4], f32),
) -> ([u8; 4], f32) {
    static LINEAR: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    let linear = LINEAR.get_or_init(|| {
        std::array::from_fn(|i| {
            let s = i as f32 / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        })
    });
    let scale = scale.clamp(glam::Vec2::splat(0.05), glam::Vec2::ONE);
    let radius = glam::Vec2::splat(3.0) / scale;
    let (x, y) = (x - 0.5, y - 0.5);
    // At the minimum Transform scale there are at most 120 taps per axis.
    let x0 = (x - radius.x).ceil() as i32;
    let x1 = (x + radius.x).floor() as i32;
    let mut wx = [0.0; 121];
    for sx in x0..=x1 {
        wx[(sx - x0) as usize] = lanczos3((sx as f32 - x) * scale.x);
    }
    let mut sum = [0.0; 4];
    let mut weight = 0.0;
    let mut coverage = 0.0;
    for sy in (y - radius.y).ceil() as i32..=(y + radius.y).floor() as i32 {
        let wy = lanczos3((sy as f32 - y) * scale.y);
        for sx in x0..=x1 {
            let w = wx[(sx - x0) as usize] * wy;
            weight += w;
            if w.abs() < 1.0e-10 {
                continue;
            }
            let (p, mask) = read(sx, sy);
            coverage += w * mask;
            let a = p[3] as f32 / 255.0 * mask * w;
            for c in 0..3 {
                sum[c] += linear[p[c] as usize] * a;
            }
            sum[3] += a;
        }
    }
    if weight <= f32::EPSILON {
        return ([0; 4], 0.0);
    }
    let coverage = (coverage / weight).clamp(0.0, 1.0);
    let alpha = (sum[3] / weight).clamp(0.0, 1.0);
    if alpha < 0.5 / 255.0 {
        return ([0; 4], coverage);
    }
    (
        [
            linear_to_srgb(sum[0] / sum[3]),
            linear_to_srgb(sum[1] / sum[3]),
            linear_to_srgb(sum[2] / sum[3]),
            (alpha * 255.0).round() as u8,
        ],
        coverage,
    )
}

/// Floating selection used by the Transform tool.
#[derive(Clone)]
pub struct FloatingTransform {
    pub layer: usize,
    pub pixels: Vec<u8>,
    pub mask: Vec<u8>,
    pub bw: u32,
    pub bh: u32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub center: glam::Vec2,
    /// Rotation/scale origin in document coordinates; moving it does not move pixels.
    pub pivot: glam::Vec2,
    pub scale: glam::Vec2,
    pub keep_aspect_ratio: bool,
    pub flip_x: bool,
    pub flip_y: bool,
    pub rotation: f32,
    /// Layer opacity baked into GPU preview; stamp writes raw pixels.
    pub opacity: f32,
    pub whole_layer: bool,
    pub source: Option<crate::raster::SparsePixels>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformHit {
    Body,
    Pivot,
    Scale(u8),
    Rotate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformCursor {
    Move,
    Pivot,
    Rotate,
    ResizeEw,
    ResizeNs,
    ResizeNwse,
    ResizeNesw,
    Eyedropper,
}

impl TransformCursor {
    pub fn resize(direction: glam::Vec2) -> Self {
        let angle = direction.y.atan2(direction.x);
        match (angle / std::f32::consts::FRAC_PI_4).round() as i32 % 4 {
            0 => Self::ResizeEw,
            1 | -3 => Self::ResizeNwse,
            2 | -2 => Self::ResizeNs,
            _ => Self::ResizeNesw,
        }
    }
}

impl FloatingTransform {
    pub fn extract(
        pixels: &(impl Raster + ?Sized),
        width: u32,
        _height: u32,
        selection: &Selection,
        layer: usize,
        opacity: f32,
    ) -> Option<Self> {
        let bounds = selection.bounds()?;
        let bw = bounds.max_x - bounds.min_x;
        let bh = bounds.max_y - bounds.min_y;
        if bw == 0 || bh == 0 {
            return None;
        }
        let mut fp = vec![0u8; (bw * bh * 4) as usize];
        let mut mask = vec![0u8; (bw * bh) as usize];
        for y in bounds.min_y..bounds.max_y {
            for x in bounds.min_x..bounds.max_x {
                if !selection.contains(x, y) {
                    continue;
                }
                let li = ((y - bounds.min_y) * bw + (x - bounds.min_x)) as usize;
                fp[li * 4..li * 4 + 4].copy_from_slice(&pixels.read(width, x as i32, y as i32));
                mask[li] = 1;
            }
        }
        if mask.iter().all(|v| *v == 0) {
            return None;
        }
        let center = glam::Vec2::new(
            bounds.min_x as f32 + bw as f32 * 0.5,
            bounds.min_y as f32 + bh as f32 * 0.5,
        );
        Some(Self {
            layer,
            pixels: fp,
            mask,
            bw,
            bh,
            origin_x: bounds.min_x as i32,
            origin_y: bounds.min_y as i32,
            center,
            pivot: center,
            scale: glam::Vec2::ONE,
            keep_aspect_ratio: true,
            flip_x: false,
            flip_y: false,
            rotation: 0.0,
            opacity: opacity.clamp(0.0, 1.0),
            whole_layer: false,
            source: None,
        })
    }

    /// Erase grabbed pixels. Call on commit, after the preview has been sampling them.
    pub fn extract_layer(
        pixels: &crate::raster::SparsePixels,
        layer: usize,
        opacity: f32,
    ) -> Option<Self> {
        let (x, y, x1, y1) = pixels.bounds()?;
        let bw = (x1 - x) as u32;
        let bh = (y1 - y) as u32;
        Some(Self {
            layer,
            pixels: Vec::new(),
            mask: Vec::new(),
            bw,
            bh,
            origin_x: x,
            origin_y: y,
            center: glam::Vec2::new(x as f32 + bw as f32 * 0.5, y as f32 + bh as f32 * 0.5),
            pivot: glam::Vec2::new(x as f32 + bw as f32 * 0.5, y as f32 + bh as f32 * 0.5),
            scale: glam::Vec2::ONE,
            keep_aspect_ratio: true,
            flip_x: false,
            flip_y: false,
            rotation: 0.0,
            opacity,
            whole_layer: true,
            source: Some(pixels.clone()),
        })
    }

    pub fn clear_source(&self, pixels: &mut (impl Raster + ?Sized), width: u32, _height: u32) {
        for y in 0..self.bh {
            for x in 0..self.bw {
                if self.mask[(y * self.bw + x) as usize] != 0 {
                    pixels.write(
                        width,
                        self.origin_x + x as i32,
                        self.origin_y + y as i32,
                        [0; 4],
                    );
                }
            }
        }
    }

    pub fn local_size(&self) -> glam::Vec2 {
        glam::Vec2::new(self.bw as f32, self.bh as f32) * self.scale.max(glam::Vec2::splat(0.05))
    }

    pub fn corners(&self) -> [glam::Vec2; 4] {
        let half = self.local_size() * 0.5;
        let locals = [
            glam::Vec2::new(-half.x, -half.y),
            glam::Vec2::new(half.x, -half.y),
            glam::Vec2::new(half.x, half.y),
            glam::Vec2::new(-half.x, half.y),
        ];
        let (s, c) = self.rotation.sin_cos();
        locals.map(|p| self.center + glam::Vec2::new(p.x * c - p.y * s, p.x * s + p.y * c))
    }

    /// Positions of the source corners; the frame itself stays fixed when flipped.
    pub fn source_corners(&self) -> [glam::Vec2; 4] {
        let mut corners = self.corners();
        if self.flip_x {
            corners = [corners[1], corners[0], corners[3], corners[2]];
        }
        if self.flip_y {
            corners = [corners[3], corners[2], corners[1], corners[0]];
        }
        corners
    }

    fn source_scale(&self) -> glam::Vec2 {
        self.scale.max(glam::Vec2::splat(0.05))
            * glam::Vec2::new(
                if self.flip_x { -1.0 } else { 1.0 },
                if self.flip_y { -1.0 } else { 1.0 },
            )
    }

    pub fn drag(&mut self, hit: TransformHit, from: glam::Vec2, to: glam::Vec2) {
        match hit {
            TransformHit::Body => {
                let delta = to - from;
                self.center += delta;
                self.pivot += delta;
            }
            TransformHit::Pivot => self.pivot += to - from,
            TransformHit::Rotate => {
                let a = from - self.pivot;
                let b = to - self.pivot;
                if a.length_squared() < 1.0e-8 || b.length_squared() < 1.0e-8 {
                    return;
                }
                let angle = a.perp_dot(b).atan2(a.dot(b));
                let (s, c) = angle.sin_cos();
                let delta = self.center - self.pivot;
                self.center = self.pivot
                    + glam::Vec2::new(delta.x * c - delta.y * s, delta.x * s + delta.y * c);
                self.rotation += angle;
            }
            TransformHit::Scale(_) => {
                let (sin, cos) = self.rotation.sin_cos();
                let local = |p: glam::Vec2| {
                    let p = p - self.pivot;
                    glam::Vec2::new(p.x * cos + p.y * sin, -p.x * sin + p.y * cos)
                };
                let from = local(from);
                let to = local(to);
                let factor = if self.keep_aspect_ratio {
                    if from.length() < 1.0e-4 {
                        return;
                    }
                    glam::Vec2::splat((to.length() / from.length()).clamp(
                        0.05 / self.scale.min_element(),
                        32.0 / self.scale.max_element(),
                    ))
                } else {
                    glam::Vec2::new(
                        if from.x.abs() < 1.0e-4 {
                            1.0
                        } else {
                            to.x / from.x
                        },
                        if from.y.abs() < 1.0e-4 {
                            1.0
                        } else {
                            to.y / from.y
                        },
                    )
                };
                let scale =
                    (self.scale * factor).clamp(glam::Vec2::splat(0.05), glam::Vec2::splat(32.0));
                let delta = local(self.center) * (scale / self.scale);
                self.center = self.pivot
                    + glam::Vec2::new(delta.x * cos - delta.y * sin, delta.x * sin + delta.y * cos);
                self.scale = scale;
            }
        }
    }

    pub fn hit_test(&self, image_pos: glam::Vec2, handle_r: f32) -> Option<TransformHit> {
        let corners = self.corners();
        let mut nearest = (self.pivot - image_pos).length();
        let mut handle = TransformHit::Pivot;
        for (i, c) in corners.iter().enumerate() {
            let distance = (*c - image_pos).length();
            if distance < nearest {
                nearest = distance;
                handle = TransformHit::Scale(i as u8);
            }
        }
        if nearest <= handle_r {
            return Some(handle);
        }
        // Inverse transform point into local unscaled bbox.
        let delta = image_pos - self.center;
        let (s, c) = (-self.rotation).sin_cos();
        let local = glam::Vec2::new(delta.x * c - delta.y * s, delta.x * s + delta.y * c)
            / self.scale.max(glam::Vec2::splat(0.05));
        let half = glam::Vec2::new(self.bw as f32, self.bh as f32) * 0.5;
        if local.x.abs() <= half.x && local.y.abs() <= half.y {
            return Some(TransformHit::Body);
        }
        Some(TransformHit::Rotate)
    }

    /// Resample occupied source tiles only, even when the layer spans a huge empty area.
    pub fn stamp_layer(&self, target: &mut crate::raster::SparsePixels) {
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let side = crate::tiles::TILE_SIZE as i32;
        let half = glam::Vec2::new(self.bw as f32, self.bh as f32) * 0.5;
        let origin = glam::Vec2::new(self.origin_x as f32, self.origin_y as f32) + half;
        let (s, c) = self.rotation.sin_cos();
        let radius =
            glam::Vec2::splat(3.0) / self.scale.clamp(glam::Vec2::splat(0.05), glam::Vec2::ONE);
        let scale = self.source_scale();
        let transform = |p: glam::Vec2| {
            let p = (p - origin) * scale;
            self.center + glam::Vec2::new(p.x * c - p.y * s, p.x * s + p.y * c)
        };
        let inverse = |p: glam::Vec2| {
            let p = p - self.center;
            glam::Vec2::new(p.x * c + p.y * s, -p.x * s + p.y * c) / scale + origin
        };
        let mut destinations = std::collections::HashSet::new();
        for &(tx, ty) in source.tiles.keys() {
            let x = tx * side + source.offset.0;
            let y = ty * side + source.offset.1;
            let corners = [(x, y), (x + side, y), (x + side, y + side), (x, y + side)]
                .map(|(x, y)| glam::Vec2::new(x as f32, y as f32));
            let corners = [
                corners[0] - radius,
                corners[1] + glam::Vec2::new(radius.x, -radius.y),
                corners[2] + radius,
                corners[3] + glam::Vec2::new(-radius.x, radius.y),
            ]
            .map(transform);
            let x0 = corners
                .iter()
                .map(|p| p.x)
                .fold(f32::INFINITY, f32::min)
                .floor() as i32;
            let y0 = corners
                .iter()
                .map(|p| p.y)
                .fold(f32::INFINITY, f32::min)
                .floor() as i32;
            let x1 = corners
                .iter()
                .map(|p| p.x)
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil() as i32;
            let y1 = corners
                .iter()
                .map(|p| p.y)
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil() as i32;
            for dy in y0.div_euclid(side)..=y1.div_euclid(side) {
                for dx in x0.div_euclid(side)..=x1.div_euclid(side) {
                    destinations.insert((dx, dy));
                }
            }
        }
        for (tx, ty) in destinations {
            // Pack this tile's source footprint once, avoiding a sparse-map lookup
            // for every Lanczos tap (especially costly during strong reductions).
            let corners = [
                (tx * side, ty * side),
                ((tx + 1) * side, ty * side),
                ((tx + 1) * side, (ty + 1) * side),
                (tx * side, (ty + 1) * side),
            ]
            .map(|(x, y)| inverse(glam::Vec2::new(x as f32, y as f32)));
            let sx = (corners.iter().map(|p| p.x).fold(f32::INFINITY, f32::min) - radius.x).floor()
                as i32;
            let sy = (corners.iter().map(|p| p.y).fold(f32::INFINITY, f32::min) - radius.y).floor()
                as i32;
            let ex = (corners
                .iter()
                .map(|p| p.x)
                .fold(f32::NEG_INFINITY, f32::max)
                + radius.x)
                .ceil() as i32;
            let ey = (corners
                .iter()
                .map(|p| p.y)
                .fold(f32::NEG_INFINITY, f32::max)
                + radius.y)
                .ceil() as i32;
            let stride = (ex - sx + 1) as u32;
            let packed = source.region(sx, sy, stride, (ey - sy + 1) as u32);
            for y in ty * side..(ty + 1) * side {
                for x in tx * side..(tx + 1) * side {
                    let p = inverse(glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
                    if p.x < self.origin_x as f32 - radius.x
                        || p.y < self.origin_y as f32 - radius.y
                        || p.x > (self.origin_x as f32 + self.bw as f32) + radius.x
                        || p.y > (self.origin_y as f32 + self.bh as f32) + radius.y
                    {
                        continue;
                    }
                    let pixel = sample_transform(p.x, p.y, self.scale, |x, y| {
                        (packed.read(stride, x - sx, y - sy), 1.0)
                    })
                    .0;
                    if pixel[3] != 0 {
                        target.write(0, x, y, pixel);
                    }
                }
            }
        }
    }

    pub fn stamp(
        &self,
        pixels: &mut (impl Raster + ?Sized),
        width: u32,
        height: u32,
    ) -> (Option<PixelRect>, Selection) {
        let mut mask = vec![0u8; width as usize * height as usize];
        let corners = self.corners();
        let min_x = corners
            .iter()
            .map(|p| p.x)
            .fold(f32::INFINITY, f32::min)
            .floor() as i32;
        let min_y = corners
            .iter()
            .map(|p| p.y)
            .fold(f32::INFINITY, f32::min)
            .floor() as i32;
        let max_x = corners
            .iter()
            .map(|p| p.x)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil() as i32;
        let max_y = corners
            .iter()
            .map(|p| p.y)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil() as i32;
        let half = glam::Vec2::new(self.bw as f32, self.bh as f32) * 0.5;
        let inv_scale = glam::Vec2::ONE / self.source_scale();
        let (s, c) = (-self.rotation).sin_cos();
        // Include the complete rotated filter footprint, including ringing lobes.
        let radius =
            glam::Vec2::splat(3.0) / self.scale.clamp(glam::Vec2::splat(0.05), glam::Vec2::ONE);
        let footprint = radius * self.scale.max(glam::Vec2::splat(0.05));
        let pad = (footprint.max_element() * (s.abs() + c.abs())).ceil() as i32;
        let min_x = min_x - pad;
        let min_y = min_y - pad;
        let max_x = max_x + pad;
        let max_y = max_y + pad;
        let mut dirty: Option<PixelRect> = None;
        for y in min_y..max_y {
            for x in min_x..max_x {
                let delta = glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - self.center;
                let local = glam::Vec2::new(delta.x * c - delta.y * s, delta.x * s + delta.y * c)
                    * inv_scale;
                let lx = local.x + half.x;
                let ly = local.y + half.y;
                if lx < -radius.x
                    || ly < -radius.y
                    || lx >= self.bw as f32 + radius.x
                    || ly >= self.bh as f32 + radius.y
                {
                    continue;
                }
                let (src_px, coverage) = sample_transform(lx, ly, self.scale, |x, y| {
                    if x < 0 || y < 0 || x >= self.bw as i32 || y >= self.bh as i32 {
                        return ([0; 4], 0.0);
                    }
                    let i = (y as u32 * self.bw + x as u32) as usize;
                    (
                        self.pixels[i * 4..i * 4 + 4].try_into().unwrap(),
                        if self.mask[i] != 0 { 1.0 } else { 0.0 },
                    )
                });
                if src_px[3] == 0 {
                    continue;
                }
                let mut pixel = pixels.read(width, x, y);
                over(&mut pixel, &src_px, 1.0);
                pixels.write(width, x, y, pixel);
                if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                    continue;
                }
                let (x, y) = (x as u32, y as u32);
                if coverage >= 0.5 {
                    mask[(y * width + x) as usize] = 1;
                }
                let rect = PixelRect {
                    min_x: x,
                    min_y: y,
                    max_x: x + 1,
                    max_y: y + 1,
                };
                dirty = Some(match dirty {
                    Some(prev) => PixelRect {
                        min_x: prev.min_x.min(rect.min_x),
                        min_y: prev.min_y.min(rect.min_y),
                        max_x: prev.max_x.max(rect.max_x),
                        max_y: prev.max_y.max(rect.max_y),
                    },
                    None => rect,
                });
            }
        }
        (
            dirty,
            Selection {
                width,
                height,
                mask,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::sample::sample_bilinear;
    use super::*;
    use crate::raster::Raster;

    #[test]
    fn transform_keeps_pixels_until_commit_clear() {
        let mut pixels = vec![0u8; 4 * 4 * 4];
        pixels[0..4].copy_from_slice(&[10, 20, 30, 255]);
        let selection = Selection::from_rect(
            PixelRect {
                min_x: 0,
                min_y: 0,
                max_x: 2,
                max_y: 2,
            },
            4,
            4,
        );
        let floating = FloatingTransform::extract(&pixels, 4, 4, &selection, 0, 1.0).unwrap();
        assert_eq!(&pixels[0..4], &[10, 20, 30, 255]);
        floating.clear_source(&mut pixels, 4, 4);
        assert_eq!(&pixels[0..4], &[0, 0, 0, 0]);
        assert_eq!(floating.pixels[0..4], [10, 20, 30, 255]);
    }

    #[test]
    fn transform_downscale_filters_stripes_for_layers_and_selections() {
        let mut pixels = vec![0; 96 * 96 * 4];
        for y in 0..96 {
            for x in 0..96 {
                let v = if x % 3 == 0 { 255 } else { 0 };
                pixels[(y * 96 + x) * 4..(y * 96 + x) * 4 + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        // Bilinear alone aliases this reduction to black instead of averaging.
        assert_eq!(sample_bilinear(&pixels, 96, 96, 49.5, 49.5), [0, 0, 0, 255]);
        let source = crate::raster::SparsePixels::from_rgba(&pixels, 96, 96);
        let mut layer = FloatingTransform::extract_layer(&source, 0, 1.0).unwrap();
        layer.scale = glam::Vec2::splat(1.0 / 3.0);
        layer.center = glam::Vec2::splat(16.0);
        let mut output = crate::raster::SparsePixels::default();
        layer.stamp_layer(&mut output);
        let selection = Selection::from_rect(
            PixelRect {
                min_x: 0,
                min_y: 0,
                max_x: 96,
                max_y: 96,
            },
            96,
            96,
        );
        let mut fragment = FloatingTransform::extract(&source, 96, 96, &selection, 0, 1.0).unwrap();
        fragment.scale = layer.scale;
        fragment.center = layer.center;
        let mut selected = crate::raster::SparsePixels::default();
        fragment.stamp(&mut selected, 96, 96);
        // One third white in linear light is about sRGB 156. The probes also
        // straddle a source tile boundary (64), which must not introduce seams.
        for x in 10..25 {
            let p = output.read(96, x, 16);
            assert!((p[0] as i32 - 156).abs() <= 2, "x={x}: {p:?}");
            assert_eq!(p[3], 255);
            assert_eq!(selected.read(96, x, 16), p);
        }
    }

    #[test]
    fn transform_flip_maps_preview_and_pixels_for_layers_and_selections() {
        use glam::Vec2;
        let bytes: Vec<u8> = (0..8).flat_map(|i| [i * 20, 100, 200, 255]).collect();
        let source = crate::raster::SparsePixels::from_rgba(&bytes, 4, 2);
        for flip_x in [false, true] {
            for flip_y in [false, true] {
                for rotated in [false, true] {
                    let selection = Selection::all(4, 2);
                    let mut layer = FloatingTransform::extract_layer(&source, 0, 1.0).unwrap();
                    let mut fragment =
                        FloatingTransform::extract(&source, 4, 2, &selection, 0, 1.0).unwrap();
                    for transform in [&mut layer, &mut fragment] {
                        transform.center = Vec2::splat(10.0);
                        transform.pivot = Vec2::new(1.0, 3.0);
                        transform.flip_x = flip_x;
                        transform.flip_y = flip_y;
                        if rotated {
                            transform.rotation = std::f32::consts::FRAC_PI_2;
                        } else {
                            transform.scale = Vec2::new(2.0, 3.0);
                        }
                        let corners = transform.corners();
                        let mut indices = [0, 1, 2, 3];
                        if flip_x {
                            indices = [indices[1], indices[0], indices[3], indices[2]];
                        }
                        if flip_y {
                            indices = [indices[3], indices[2], indices[1], indices[0]];
                        }
                        assert_eq!(transform.source_corners(), indices.map(|i| corners[i]));
                        let pivot = transform.pivot;
                        transform.flip_x = !transform.flip_x;
                        transform.flip_x = !transform.flip_x;
                        assert_eq!(transform.corners(), corners);
                        assert_eq!(transform.pivot, pivot);
                    }
                    let mut output = crate::raster::SparsePixels::default();
                    layer.stamp_layer(&mut output);
                    let mut selected = crate::raster::SparsePixels::default();
                    fragment.stamp(&mut selected, 20, 20);
                    for y in 0..2 {
                        for x in 0..4 {
                            // Source texel centers under the preview's source-corner mapping.
                            let u = (x as f32 + 0.5) / 4.0;
                            let v = (y as f32 + 0.5) / 2.0;
                            let corners = layer.source_corners();
                            let point = corners[0]
                                + (corners[1] - corners[0]) * u
                                + (corners[3] - corners[0]) * v;
                            let px = point.x.floor() as i32;
                            let py = point.y.floor() as i32;
                            if rotated {
                                assert_eq!(
                                    output.read(20, px, py),
                                    [((y * 4 + x) * 20) as u8, 100, 200, 255]
                                );
                            }
                            // Compare all reflected source positions between both commit paths.
                            assert_eq!(selected.read(20, px, py), output.read(20, px, py));
                            assert_ne!(output.read(20, px, py)[3], 0);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn transform_free_scaling_uses_rotated_axes_and_keeps_pivot_fixed() {
        use glam::Vec2;
        let source =
            crate::raster::SparsePixels::from_rgba(&[10, 20, 30, 255].repeat(100 * 80), 100, 80);
        let mut transform = FloatingTransform::extract_layer(&source, 0, 1.0).unwrap();
        assert!(transform.keep_aspect_ratio);
        transform.rotation = std::f32::consts::FRAC_PI_2;
        transform.pivot = Vec2::new(30.0, 20.0);
        let pivot = transform.pivot;
        let corners = transform.corners();
        transform.keep_aspect_ratio = false;
        // In rotated local axes this changes X by 2 and Y by 0.5.
        transform.drag(
            TransformHit::Scale(2),
            pivot + Vec2::new(-40.0, 50.0),
            pivot + Vec2::new(-20.0, 100.0),
        );
        assert!((transform.scale - Vec2::new(2.0, 0.5)).length() < 1.0e-5);
        assert_eq!(transform.pivot, pivot);
        for (before, after) in corners.into_iter().zip(transform.corners()) {
            let delta = before - pivot;
            assert!((after - (pivot + delta * Vec2::new(0.5, 2.0))).length() < 1.0e-4);
        }
        let center = transform.center;
        assert_eq!(transform.hit_test(center, 0.1), Some(TransformHit::Body));
        assert_eq!(
            transform.hit_test(center + Vec2::new(21.0, 0.0), 0.1),
            Some(TransformHit::Rotate)
        );
        // Re-enabling the lock preserves the current proportions.
        transform.keep_aspect_ratio = true;
        transform.drag(
            TransformHit::Scale(2),
            pivot + Vec2::new(10.0, 10.0),
            pivot + Vec2::new(20.0, 20.0),
        );
        assert!((transform.scale - Vec2::new(4.0, 1.0)).length() < 1.0e-5);
    }

    #[test]
    fn transform_nonuniform_stamp_matches_for_layers_and_selections() {
        use glam::Vec2;
        let source =
            crate::raster::SparsePixels::from_rgba(&[10, 20, 30, 255].repeat(16 * 16), 16, 16);
        let selection = Selection::all(16, 16);
        let mut layer = FloatingTransform::extract_layer(&source, 0, 1.0).unwrap();
        let mut fragment = FloatingTransform::extract(&source, 16, 16, &selection, 0, 1.0).unwrap();
        for transform in [&mut layer, &mut fragment] {
            transform.scale = Vec2::new(2.0, 0.5);
            transform.center = Vec2::splat(32.0);
            assert_eq!(transform.local_size(), Vec2::new(32.0, 8.0));
        }
        let mut output = crate::raster::SparsePixels::default();
        layer.stamp_layer(&mut output);
        let mut selected = crate::raster::SparsePixels::default();
        let (_, mask) = fragment.stamp(&mut selected, 64, 64);
        for x in 24..40 {
            assert_eq!(output.read(64, x, 32), [10, 20, 30, 255]);
            assert_eq!(selected.read(64, x, 32), output.read(64, x, 32));
            assert!(mask.contains(x as u32, 32));
        }
        assert_eq!(output.read(64, 32, 20), [0; 4]);
        assert_eq!(selected.read(64, 32, 20), [0; 4]);
    }

    #[test]
    fn transform_pivot_moves_independently_and_anchors_rotation_and_scale() {
        let source =
            crate::raster::SparsePixels::from_rgba(&[10, 20, 30, 255].repeat(100 * 80), 100, 80);
        let mut transform = FloatingTransform::extract_layer(&source, 0, 1.0).unwrap();
        let corners = transform.corners();
        assert_eq!(transform.pivot, transform.center);
        transform.drag(
            TransformHit::Pivot,
            transform.pivot,
            glam::Vec2::new(30.0, 50.0),
        );
        assert_eq!(
            transform.corners(),
            corners,
            "moving pivot must not move the image"
        );
        let pivot = transform.pivot;
        transform.drag(
            TransformHit::Rotate,
            pivot + glam::Vec2::X * 20.0,
            pivot + glam::Vec2::Y * 20.0,
        );
        assert_eq!(transform.pivot, pivot);
        assert!((transform.rotation - std::f32::consts::FRAC_PI_2).abs() < 1.0e-5);
        for (before, after) in corners.into_iter().zip(transform.corners()) {
            let delta = before - pivot;
            let expected = pivot + glam::Vec2::new(-delta.y, delta.x);
            assert!((after - expected).length() < 1.0e-4);
        }
        let corners = transform.corners();
        transform.drag(
            TransformHit::Scale(0),
            pivot + glam::Vec2::X * 10.0,
            pivot + glam::Vec2::X * 20.0,
        );
        assert_eq!(transform.scale, glam::Vec2::splat(2.0));
        assert_eq!(transform.pivot, pivot);
        for (before, after) in corners.into_iter().zip(transform.corners()) {
            assert!((after - (pivot + (before - pivot) * 2.0)).length() < 1.0e-4);
        }
        let center = transform.center;
        let delta = glam::Vec2::new(-15.0, 7.0);
        transform.drag(TransformHit::Body, glam::Vec2::ZERO, delta);
        assert_eq!(transform.pivot, pivot + delta);
        assert_eq!(transform.center, center + delta);
    }

    #[test]
    fn transform_outside_rotates_and_handles_follow_rotated_bounds() {
        let source =
            crate::raster::SparsePixels::from_rgba(&[10, 20, 30, 255].repeat(100 * 80), 100, 80);
        let mut transform = FloatingTransform::extract_layer(&source, 0, 1.0).unwrap();
        for rotation in [0.0, 0.7, std::f32::consts::FRAC_PI_2] {
            transform.rotation = rotation;
            assert_eq!(
                transform.hit_test(transform.pivot, 8.0),
                Some(TransformHit::Pivot)
            );
            let corners = transform.corners();
            for (i, corner) in corners.into_iter().enumerate() {
                assert_eq!(
                    transform.hit_test(corner, 8.0),
                    Some(TransformHit::Scale(i as u8))
                );
            }
            assert_eq!(
                transform.hit_test(corners[0] * 0.75 + corners[2] * 0.25, 8.0),
                Some(TransformHit::Body)
            );
            assert_eq!(
                transform.hit_test(corners[0] + (corners[0] - transform.center), 8.0),
                Some(TransformHit::Rotate)
            );
        }
        // Crossing +/-180 degrees must produce a small step, not a full turn.
        let direction = |degrees: f32| {
            glam::Vec2::new(degrees.to_radians().cos(), degrees.to_radians().sin()) * 100.0
        };
        let before = transform.rotation;
        transform.drag(
            TransformHit::Rotate,
            transform.pivot + direction(179.0),
            transform.pivot + direction(-179.0),
        );
        assert!((transform.rotation - before - 2.0f32.to_radians()).abs() < 1.0e-5);
    }

    #[test]
    #[ignore = "manual Transform commit performance measurement"]
    fn transform_commit_performance() {
        let source = crate::raster::SparsePixels::from_rgba(
            &[120, 180, 240, 255].repeat(1024 * 1024),
            1024,
            1024,
        );
        for scale in [0.5, 0.05] {
            let mut transform = FloatingTransform::extract_layer(&source, 0, 1.0).unwrap();
            transform.scale = glam::Vec2::splat(scale);
            let mut output = crate::raster::SparsePixels::default();
            let started = std::time::Instant::now();
            transform.stamp_layer(&mut output);
            eprintln!("1024x1024 commit at {scale}: {:?}", started.elapsed());
            assert!(!output.tiles.is_empty());
        }
    }

    #[test]
    fn transform_filter_preserves_identity_alpha_and_mask() {
        let pixel = [17, 128, 240, 93];
        assert_eq!(
            sample_transform(0.5, 0.5, glam::Vec2::ONE, |x, y| {
                if x == 0 && y == 0 {
                    (pixel, 1.0)
                } else {
                    ([0; 4], 0.0)
                }
            })
            .0,
            pixel
        );
        let (p, coverage) = sample_transform(0.5, 0.5, glam::Vec2::splat(0.25), |x, _| {
            // Unselected opaque blue and transparent green must not bleed into red.
            if x < 0 {
                ([0, 0, 255, 255], 0.0)
            } else if x % 2 == 0 {
                ([255, 0, 0, 255], 1.0)
            } else {
                ([0, 255, 0, 0], 1.0)
            }
        });
        assert_eq!(&p[..3], &[255, 0, 0]);
        assert!(p[3] > 0 && p[3] < 255);
        assert!(coverage > 0.0 && coverage < 1.0);
    }

    #[test]
    fn rotated_transform_downscale_averages_checker_and_retains_off_canvas_pixels() {
        let mut pixels = vec![0; 64 * 64 * 4];
        for y in 0..64 {
            for x in 0..64 {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let source = crate::raster::SparsePixels::from_rgba(&pixels, 64, 64);
        let mut transform = FloatingTransform::extract_layer(&source, 0, 1.0).unwrap();
        transform.scale = glam::Vec2::splat(0.2);
        transform.rotation = 0.6;
        transform.center = glam::Vec2::splat(-10.0);
        let mut output = crate::raster::SparsePixels::default();
        transform.stamp_layer(&mut output);
        for y in -12..-8 {
            for x in -12..-8 {
                let p = output.read(0, x, y);
                assert!((p[0] as i32 - 188).abs() <= 2, "({x},{y}): {p:?}");
                assert_eq!(p[3], 255);
            }
        }
    }
}
