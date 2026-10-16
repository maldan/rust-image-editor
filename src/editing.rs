#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Brush,
    RectangleSelect,
    MagicWand,
    Fill,
    Transform,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Brush => "Brush",
            Tool::RectangleSelect => "Select",
            Tool::MagicWand => "Wand",
            Tool::Fill => "Fill",
            Tool::Transform => "Transform",
        }
    }

    pub fn all() -> [Tool; 5] {
        [
            Tool::Brush,
            Tool::RectangleSelect,
            Tool::MagicWand,
            Tool::Fill,
            Tool::Transform,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub min_x: u32,
    pub min_y: u32,
    pub max_x: u32,
    pub max_y: u32,
}

impl PixelRect {
    pub fn from_points(a: (u32, u32), b: (u32, u32), width: u32, height: u32) -> Self {
        let min_x = a.0.min(b.0).min(width);
        let min_y = a.1.min(b.1).min(height);
        let max_x = a.0.max(b.0).saturating_add(1).min(width);
        let max_y = a.1.max(b.1).saturating_add(1).min(height);
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    pub fn is_empty(self) -> bool {
        self.max_x <= self.min_x || self.max_y <= self.min_y
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectMode {
    Replace,
    Add,
    Subtract,
}

#[derive(Debug, Clone)]
pub struct Selection {
    pub width: u32,
    pub height: u32,
    pub mask: Vec<u8>,
}

impl Selection {
    pub fn from_rect(rect: PixelRect, width: u32, height: u32) -> Self {
        let mut mask = vec![0u8; width as usize * height as usize];
        for y in rect.min_y..rect.max_y {
            for x in rect.min_x..rect.max_x {
                mask[(y * width + x) as usize] = 1;
            }
        }
        Self {
            width,
            height,
            mask,
        }
    }

    pub fn all(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            mask: vec![1u8; width as usize * height as usize],
        }
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.mask[(y * self.width + x) as usize] != 0
    }

    pub fn bounds(&self) -> Option<PixelRect> {
        let mut min_x = self.width;
        let mut min_y = self.height;
        let mut max_x = 0u32;
        let mut max_y = 0u32;
        let mut any = false;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.mask[(y * self.width + x) as usize] != 0 {
                    any = true;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x + 1);
                    max_y = max_y.max(y + 1);
                }
            }
        }
        any.then_some(PixelRect {
            min_x,
            min_y,
            max_x,
            max_y,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.mask.iter().all(|&v| v == 0)
    }

    pub fn union(&mut self, other: &Selection) {
        if self.width != other.width || self.height != other.height {
            return;
        }
        for (dst, &src) in self.mask.iter_mut().zip(other.mask.iter()) {
            if src != 0 {
                *dst = 1;
            }
        }
    }

    pub fn subtract(&mut self, other: &Selection) {
        if self.width != other.width || self.height != other.height {
            return;
        }
        for (dst, &src) in self.mask.iter_mut().zip(other.mask.iter()) {
            if src != 0 {
                *dst = 0;
            }
        }
    }

    /// Packed R8 mask (0/255) for GPU overlay upload.
    pub fn mask_r8(&self) -> Vec<u8> {
        self.mask
            .iter()
            .map(|&v| if v != 0 { 255 } else { 0 })
            .collect()
    }
}

/// Axis-aligned bounds that a brush segment may touch.
pub fn brush_dirty_bounds(
    from: (f32, f32),
    to: (f32, f32),
    size: f32,
    width: u32,
    height: u32,
) -> PixelRect {
    let radius = (size * 0.5).max(0.5) + 1.0;
    let min_x = (from.0.min(to.0) - radius).floor().max(0.0) as u32;
    let min_y = (from.1.min(to.1) - radius).floor().max(0.0) as u32;
    let max_x = (from.0.max(to.0) + radius).ceil().min(width as f32) as u32;
    let max_y = (from.1.max(to.1) + radius).ceil().min(height as f32) as u32;
    PixelRect {
        min_x,
        min_y,
        max_x,
        max_y,
    }
}

pub struct BrushStroke<'a> {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub size: f32,
    pub opacity: f32,
    pub erase: bool,
    pub color: [f32; 3],
    pub selection: Option<&'a Selection>,
    /// Generation marks: pixel is touched this stroke when `marks[i] == stroke_id`.
    pub coverage_marks: Option<&'a mut [u32]>,
    pub stroke_id: u32,
}

/// Returns axis-aligned dirty pixel bounds if anything changed.
pub fn apply_brush(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    stroke: &mut BrushStroke<'_>,
) -> Option<PixelRect> {
    if width == 0 || height == 0 || pixels.len() < width as usize * height as usize * 4 {
        return None;
    }
    let radius = (stroke.size * 0.5).max(0.5);
    let dx = stroke.to.0 - stroke.from.0;
    let dy = stroke.to.1 - stroke.from.1;
    let distance = dx.hypot(dy);
    let step = (radius * 0.35).max(0.5);
    let steps = (distance / step).ceil() as u32;
    let mut dirty: Option<PixelRect> = None;
    for index in 0..=steps {
        let t = if steps == 0 {
            0.0
        } else {
            index as f32 / steps as f32
        };
        if let Some(rect) = stamp(
            pixels,
            width,
            height,
            stroke.from.0 + dx * t,
            stroke.from.1 + dy * t,
            radius,
            stroke.opacity.clamp(0.0, 1.0),
            stroke.erase,
            stroke.color,
            stroke.selection,
            stroke.coverage_marks.as_deref_mut(),
            stroke.stroke_id,
        ) {
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
    dirty
}

#[allow(clippy::too_many_arguments)]
fn stamp(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    center_x: f32,
    center_y: f32,
    radius: f32,
    opacity: f32,
    erase: bool,
    color: [f32; 3],
    selection: Option<&Selection>,
    mut coverage: Option<&mut [u32]>,
    stroke_id: u32,
) -> Option<PixelRect> {
    let min_x = (center_x - radius).floor().max(0.0) as u32;
    let min_y = (center_y - radius).floor().max(0.0) as u32;
    let max_x = (center_x + radius).ceil().min(width as f32) as u32;
    let max_y = (center_y + radius).ceil().min(height as f32) as u32;
    let radius_sq = radius * radius;
    let mut changed = false;
    let color = stroke_color(color);
    let mut dirty_min_x = max_x;
    let mut dirty_min_y = max_y;
    let mut dirty_max_x = min_x;
    let mut dirty_max_y = min_y;

    for y in min_y..max_y {
        for x in min_x..max_x {
            if selection.is_some_and(|sel| !sel.contains(x, y)) {
                continue;
            }
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if (px - center_x).powi(2) + (py - center_y).powi(2) > radius_sq {
                continue;
            }
            let index = (y * width + x) as usize;
            if let Some(cov) = coverage.as_mut() {
                if index >= cov.len() {
                    continue;
                }
                if cov[index] == stroke_id {
                    continue;
                }
                cov[index] = stroke_id;
            }
            let offset = index * 4;
            if erase {
                erase_pixel(&mut pixels[offset..offset + 4], opacity);
            } else {
                paint_color(&mut pixels[offset..offset + 4], opacity, color);
            }
            changed = true;
            dirty_min_x = dirty_min_x.min(x);
            dirty_min_y = dirty_min_y.min(y);
            dirty_max_x = dirty_max_x.max(x + 1);
            dirty_max_y = dirty_max_y.max(y + 1);
        }
    }
    changed.then_some(PixelRect {
        min_x: dirty_min_x,
        min_y: dirty_min_y,
        max_x: dirty_max_x,
        max_y: dirty_max_y,
    })
}

fn stroke_color(color: [f32; 3]) -> [f32; 3] {
    [
        color[0].clamp(0.0, 1.0),
        color[1].clamp(0.0, 1.0),
        color[2].clamp(0.0, 1.0),
    ]
}

fn paint_color(pixel: &mut [u8], opacity: f32, color: [f32; 3]) {
    let dst_alpha = pixel[3] as f32 / 255.0;
    let out_alpha = opacity + dst_alpha * (1.0 - opacity);
    if out_alpha <= f32::EPSILON {
        pixel.fill(0);
        return;
    }
    let dst_factor = dst_alpha * (1.0 - opacity) / out_alpha;
    for (channel, source) in pixel[..3].iter_mut().zip(color) {
        let source_premultiplied = source * opacity / out_alpha;
        let value = source_premultiplied + (*channel as f32 / 255.0) * dst_factor;
        *channel = (value * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    pixel[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn erase_pixel(pixel: &mut [u8], opacity: f32) {
    pixel[3] = (pixel[3] as f32 * (1.0 - opacity))
        .round()
        .clamp(0.0, 255.0) as u8;
}

pub fn color_distance(a: [u8; 4], b: [u8; 4]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(l, r)| l.abs_diff(r))
        .max()
        .unwrap_or(0)
}

pub fn sample_pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * width + x) * 4) as usize;
    [
        pixels[offset],
        pixels[offset + 1],
        pixels[offset + 2],
        pixels[offset + 3],
    ]
}

/// 4-connected flood fill visiting pixels within `threshold` of the seed color.
pub fn flood_mask(
    pixels: &[u8],
    width: u32,
    height: u32,
    seed: (u32, u32),
    threshold: u8,
    clip: Option<&Selection>,
) -> Option<Selection> {
    if width == 0 || height == 0 || seed.0 >= width || seed.1 >= height {
        return None;
    }
    if clip.is_some_and(|sel| !sel.contains(seed.0, seed.1)) {
        return None;
    }
    let target = sample_pixel(pixels, width, seed.0, seed.1);
    let mut mask = vec![0u8; width as usize * height as usize];
    let mut stack = vec![seed];
    while let Some((x, y)) = stack.pop() {
        let index = (y * width + x) as usize;
        if mask[index] != 0 {
            continue;
        }
        if clip.is_some_and(|sel| !sel.contains(x, y)) {
            continue;
        }
        let color = sample_pixel(pixels, width, x, y);
        if color_distance(color, target) > threshold {
            continue;
        }
        mask[index] = 1;
        if x > 0 {
            stack.push((x - 1, y));
        }
        if x + 1 < width {
            stack.push((x + 1, y));
        }
        if y > 0 {
            stack.push((x, y - 1));
        }
        if y + 1 < height {
            stack.push((x, y + 1));
        }
    }
    Some(Selection {
        width,
        height,
        mask,
    })
}

pub fn fill_bucket(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    seed: (u32, u32),
    threshold: u8,
    color: [f32; 3],
    opacity: f32,
    erase: bool,
    clip: Option<&Selection>,
) -> Option<PixelRect> {
    let region = flood_mask(pixels, width, height, seed, threshold, clip)?;
    let bounds = region.bounds()?;
    let color = stroke_color(color);
    let opacity = opacity.clamp(0.0, 1.0);
    let mut changed = false;
    for y in bounds.min_y..bounds.max_y {
        for x in bounds.min_x..bounds.max_x {
            if !region.contains(x, y) {
                continue;
            }
            let offset = ((y * width + x) * 4) as usize;
            if erase {
                erase_pixel(&mut pixels[offset..offset + 4], opacity);
            } else {
                paint_color(&mut pixels[offset..offset + 4], opacity, color);
            }
            changed = true;
        }
    }
    changed.then_some(bounds)
}

pub fn composite_layers(layers: &[crate::project::Layer], width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0u8; width as usize * height as usize * 4];
    composite_region(layers, width, height, 0, 0, width, height, &mut out);
    out
}

/// Alpha-composite visible layers into `out` for a rectangular region.
pub fn composite_region(
    layers: &[crate::project::Layer],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    tw: u32,
    th: u32,
    out: &mut [u8],
) {
    let x1 = (x + tw).min(width);
    let y1 = (y + th).min(height);
    for py in y..y1 {
        for px in x..x1 {
            let offset = ((py * width + px) * 4) as usize;
            out[offset..offset + 4].fill(0);
        }
    }
    for layer in layers {
        if !layer.visible || layer.pixels.len() < width as usize * height as usize * 4 {
            continue;
        }
        let opacity = layer.opacity.clamp(0.0, 1.0);
        if opacity <= f32::EPSILON {
            continue;
        }
        for py in y..y1 {
            for px in x..x1 {
                let offset = ((py * width + px) * 4) as usize;
                over(
                    &mut out[offset..offset + 4],
                    &layer.pixels[offset..offset + 4],
                    opacity,
                );
            }
        }
    }
}

fn over(dst: &mut [u8], src: &[u8], opacity: f32) {
    let src_a = (src[3] as f32 / 255.0) * opacity;
    if src_a <= f32::EPSILON {
        return;
    }
    let dst_a = dst[3] as f32 / 255.0;
    let out_a = src_a + dst_a * (1.0 - src_a);
    if out_a <= f32::EPSILON {
        dst.fill(0);
        return;
    }
    for c in 0..3 {
        let s = src[c] as f32 / 255.0;
        let d = dst[c] as f32 / 255.0;
        let value = (s * src_a + d * dst_a * (1.0 - src_a)) / out_a;
        dst[c] = (value * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Floating selection used by the Transform tool.
#[derive(Clone)]
pub struct FloatingTransform {
    pub layer: usize,
    pub pixels: Vec<u8>,
    pub mask: Vec<u8>,
    pub bw: u32,
    pub bh: u32,
    pub center: glam::Vec2,
    pub scale: f32,
    pub rotation: f32,
    /// Layer opacity baked into GPU preview; stamp writes raw pixels.
    pub opacity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformHit {
    Body,
    Scale(u8),
    Rotate,
}

impl FloatingTransform {
    pub fn extract(
        pixels: &mut [u8],
        width: u32,
        _height: u32,
        selection: &Selection,
        layer: usize,
        opacity: f32,
    ) -> Option<(Self, PixelRect)> {
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
                let src = ((y * width + x) * 4) as usize;
                fp[li * 4..li * 4 + 4].copy_from_slice(&pixels[src..src + 4]);
                mask[li] = 1;
                pixels[src..src + 4].fill(0);
            }
        }
        let center = glam::Vec2::new(
            bounds.min_x as f32 + bw as f32 * 0.5,
            bounds.min_y as f32 + bh as f32 * 0.5,
        );
        Some((
            Self {
                layer,
                pixels: fp,
                mask,
                bw,
                bh,
                center,
                scale: 1.0,
                rotation: 0.0,
                opacity: opacity.clamp(0.0, 1.0),
            },
            bounds,
        ))
    }

    pub fn local_size(&self) -> glam::Vec2 {
        glam::Vec2::new(self.bw as f32, self.bh as f32) * self.scale.max(0.05)
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
        locals.map(|p| {
            self.center + glam::Vec2::new(p.x * c - p.y * s, p.x * s + p.y * c)
        })
    }

    pub fn rotate_handle(&self) -> glam::Vec2 {
        let half = self.local_size() * 0.5;
        let local = glam::Vec2::new(0.0, -half.y - 28.0 / 1.0);
        let (s, c) = self.rotation.sin_cos();
        self.center + glam::Vec2::new(local.x * c - local.y * s, local.x * s + local.y * c)
    }

    pub fn hit_test(&self, image_pos: glam::Vec2, handle_r: f32) -> Option<TransformHit> {
        let corners = self.corners();
        for (i, c) in corners.iter().enumerate() {
            if (*c - image_pos).length() <= handle_r {
                return Some(TransformHit::Scale(i as u8));
            }
        }
        if (self.rotate_handle() - image_pos).length() <= handle_r {
            return Some(TransformHit::Rotate);
        }
        // Inverse transform point into local unscaled bbox.
        let delta = image_pos - self.center;
        let (s, c) = (-self.rotation).sin_cos();
        let local = glam::Vec2::new(delta.x * c - delta.y * s, delta.x * s + delta.y * c)
            / self.scale.max(0.05);
        let half = glam::Vec2::new(self.bw as f32, self.bh as f32) * 0.5;
        if local.x.abs() <= half.x && local.y.abs() <= half.y {
            let lx = (local.x + half.x).floor() as i32;
            let ly = (local.y + half.y).floor() as i32;
            if lx >= 0 && ly >= 0 && (lx as u32) < self.bw && (ly as u32) < self.bh {
                let idx = (ly as u32 * self.bw + lx as u32) as usize;
                if self.mask.get(idx).copied().unwrap_or(0) != 0 {
                    return Some(TransformHit::Body);
                }
            }
            return Some(TransformHit::Body);
        }
        None
    }

    pub fn stamp(
        &self,
        pixels: &mut [u8],
        width: u32,
        height: u32,
    ) -> (Option<PixelRect>, Selection) {
        let mut mask = vec![0u8; width as usize * height as usize];
        let corners = self.corners();
        let min_x = corners
            .iter()
            .map(|p| p.x)
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as u32;
        let min_y = corners
            .iter()
            .map(|p| p.y)
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as u32;
        let max_x = corners
            .iter()
            .map(|p| p.x)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min(width as f32) as u32;
        let max_y = corners
            .iter()
            .map(|p| p.y)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min(height as f32) as u32;
        let half = glam::Vec2::new(self.bw as f32, self.bh as f32) * 0.5;
        let inv_scale = 1.0 / self.scale.max(0.05);
        let (s, c) = (-self.rotation).sin_cos();
        let mut dirty: Option<PixelRect> = None;
        for y in min_y..max_y {
            for x in min_x..max_x {
                let delta = glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - self.center;
                let local = glam::Vec2::new(delta.x * c - delta.y * s, delta.x * s + delta.y * c)
                    * inv_scale;
                let lx = local.x + half.x;
                let ly = local.y + half.y;
                if lx < 0.0 || ly < 0.0 || lx >= self.bw as f32 || ly >= self.bh as f32 {
                    continue;
                }
                let ix = lx.floor() as u32;
                let iy = ly.floor() as u32;
                let li = (iy * self.bw + ix) as usize;
                if self.mask.get(li).copied().unwrap_or(0) == 0 {
                    continue;
                }
                let src = li * 4;
                let dst = ((y * width + x) * 4) as usize;
                let src_px = &self.pixels[src..src + 4];
                if src_px[3] == 0 {
                    continue;
                }
                over(&mut pixels[dst..dst + 4], src_px, 1.0);
                mask[(y * width + x) as usize] = 1;
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
    use super::*;

    #[test]
    fn selection_clips_brush_changes() {
        let mut pixels = vec![255; 4 * 4 * 4];
        let selection = Selection::from_rect(
            PixelRect {
                min_x: 1,
                min_y: 1,
                max_x: 2,
                max_y: 2,
            },
            4,
            4,
        );
        let mut stroke = BrushStroke {
            from: (1.5, 1.5),
            to: (1.5, 1.5),
            size: 4.0,
            opacity: 1.0,
            erase: false,
            color: [0.0, 0.0, 0.0],
            selection: Some(&selection),
            coverage_marks: None,
            stroke_id: 1,
        };
        assert!(apply_brush(&mut pixels, 4, 4, &mut stroke).is_some());
        for y in 0..4 {
            for x in 0..4 {
                let offset = ((y * 4 + x) * 4) as usize;
                if (x, y) == (1, 1) {
                    assert_eq!(&pixels[offset..offset + 4], &[0, 0, 0, 255]);
                } else {
                    assert_eq!(&pixels[offset..offset + 4], &[255, 255, 255, 255]);
                }
            }
        }
    }

    #[test]
    fn eraser_reduces_alpha_without_changing_rgb() {
        let mut pixels = vec![10, 20, 30, 200];
        let mut stroke = BrushStroke {
            from: (0.5, 0.5),
            to: (0.5, 0.5),
            size: 1.0,
            opacity: 0.5,
            erase: true,
            color: [0.0, 0.0, 0.0],
            selection: None,
            coverage_marks: None,
            stroke_id: 1,
        };
        apply_brush(&mut pixels, 1, 1, &mut stroke);
        assert_eq!(&pixels[..3], &[10, 20, 30]);
        assert_eq!(pixels[3], 100);
    }

    #[test]
    fn eraser_opacity_does_not_compound_in_one_stroke() {
        let mut pixels = vec![10, 20, 30, 200];
        let mut marks = vec![0u32; 1];
        let mut stroke = BrushStroke {
            from: (0.5, 0.5),
            to: (0.5, 0.5),
            size: 1.0,
            opacity: 0.5,
            erase: true,
            color: [0.0, 0.0, 0.0],
            selection: None,
            coverage_marks: Some(&mut marks),
            stroke_id: 7,
        };
        apply_brush(&mut pixels, 1, 1, &mut stroke);
        let mut stroke = BrushStroke {
            from: (0.5, 0.5),
            to: (0.5, 0.5),
            size: 1.0,
            opacity: 0.5,
            erase: true,
            color: [0.0, 0.0, 0.0],
            selection: None,
            coverage_marks: Some(&mut marks),
            stroke_id: 7,
        };
        apply_brush(&mut pixels, 1, 1, &mut stroke);
        assert_eq!(pixels[3], 100);
    }

    #[test]
    fn paint_uses_selected_color() {
        let mut pixels = vec![255, 255, 255, 255];
        let mut stroke = BrushStroke {
            from: (0.5, 0.5),
            to: (0.5, 0.5),
            size: 1.0,
            opacity: 1.0,
            erase: false,
            color: [1.0, 0.0, 0.5],
            selection: None,
            coverage_marks: None,
            stroke_id: 1,
        };
        apply_brush(&mut pixels, 1, 1, &mut stroke);
        assert_eq!(pixels, [255, 0, 128, 255]);
    }

    #[test]
    fn wand_selects_connected_similar_pixels() {
        let mut pixels = vec![0u8; 3 * 1 * 4];
        pixels[0..4].copy_from_slice(&[10, 10, 10, 255]);
        pixels[4..8].copy_from_slice(&[12, 10, 10, 255]);
        pixels[8..12].copy_from_slice(&[200, 0, 0, 255]);
        let selection = flood_mask(&pixels, 3, 1, (0, 0), 5, None).unwrap();
        assert!(selection.contains(0, 0));
        assert!(selection.contains(1, 0));
        assert!(!selection.contains(2, 0));
    }

    #[test]
    fn fill_paints_connected_region() {
        let mut pixels = vec![255u8; 2 * 1 * 4];
        pixels[4..8].copy_from_slice(&[0, 0, 0, 255]);
        assert!(
            fill_bucket(
                &mut pixels,
                2,
                1,
                (0, 0),
                0,
                [1.0, 0.0, 0.0],
                1.0,
                false,
                None,
            )
            .is_some()
        );
        assert_eq!(&pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&pixels[4..8], &[0, 0, 0, 255]);
    }

    #[test]
    fn fill_erase_reduces_alpha() {
        let mut pixels = vec![10, 20, 30, 200, 0, 0, 0, 255];
        fill_bucket(
            &mut pixels,
            2,
            1,
            (0, 0),
            0,
            [0.0, 0.0, 0.0],
            0.5,
            true,
            None,
        );
        assert_eq!(pixels[3], 100);
        assert_eq!(pixels[7], 255);
    }

    #[test]
    fn selection_union_and_subtract() {
        let mut a = Selection::from_rect(
            PixelRect {
                min_x: 0,
                min_y: 0,
                max_x: 2,
                max_y: 2,
            },
            4,
            4,
        );
        let b = Selection::from_rect(
            PixelRect {
                min_x: 1,
                min_y: 1,
                max_x: 3,
                max_y: 3,
            },
            4,
            4,
        );
        a.union(&b);
        assert!(a.contains(0, 0));
        assert!(a.contains(2, 2));
        a.subtract(&b);
        assert!(a.contains(0, 0));
        assert!(!a.contains(2, 2));
        assert!(!a.contains(1, 1));
    }
}
