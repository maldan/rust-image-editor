use crate::raster::Raster;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Brush,
    RectangleSelect,
    Lasso,
    SelectionBrush,
    MagicWand,
    Fill,
    Transform,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Brush => "Brush",
            Tool::RectangleSelect => "Select",
            Tool::Lasso => "Lasso",
            Tool::SelectionBrush => "Select Brush",
            Tool::MagicWand => "Wand",
            Tool::Fill => "Fill",
            Tool::Transform => "Transform",
        }
    }

    pub fn all() -> [Tool; 7] {
        [
            Tool::Brush,
            Tool::RectangleSelect,
            Tool::Lasso,
            Tool::SelectionBrush,
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
    /// Scanline polygon fill, testing pixel centers and clipping to the canvas.
    pub fn from_polygon(points: &[glam::Vec2], width: u32, height: u32) -> Self {
        let mut result = Self { width, height, mask: vec![0; width as usize * height as usize] };
        if points.len() < 3 { return result; }
        let min_y = points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min).floor().max(0.0) as u32;
        let max_y = points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max).ceil().min(height as f32) as u32;
        let mut crossings = Vec::new();
        for y in min_y..max_y {
            crossings.clear();
            let scan_y = y as f32 + 0.5;
            let mut previous = points[points.len() - 1];
            for &current in points {
                if (previous.y > scan_y) != (current.y > scan_y) {
                    crossings.push(previous.x + (scan_y - previous.y) * (current.x - previous.x) / (current.y - previous.y));
                }
                previous = current;
            }
            crossings.sort_by(f32::total_cmp);
            for pair in crossings.chunks_exact(2) {
                let start = (pair[0] - 0.5).ceil().clamp(0.0, width as f32) as u32;
                let end = (pair[1] - 0.5).ceil().clamp(0.0, width as f32) as u32;
                for x in start..end { result.mask[(y * width + x) as usize] = 1; }
            }
        }
        result
    }

    /// Paint a continuous circular selection stroke directly into the mask.
    pub fn paint_segment(&mut self, from: glam::Vec2, to: glam::Vec2, size: f32, erase: bool) {
        let bounds = brush_dirty_bounds(from.into(), to.into(), size, self.width, self.height);
        let delta = to - from;
        let length_sq = delta.length_squared();
        let radius_sq = (size * 0.5).max(0.5).powi(2);
        for y in bounds.min_y..bounds.max_y {
            for x in bounds.min_x..bounds.max_x {
                let point = glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let t = if length_sq > 0.0 { ((point - from).dot(delta) / length_sq).clamp(0.0, 1.0) } else { 0.0 };
                if point.distance_squared(from + delta * t) <= radius_sq {
                    self.mask[(y * self.width + x) as usize] = u8::from(!erase);
                }
            }
        }
    }

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

    pub fn invert(&mut self) {
        for value in &mut self.mask {
            *value = u8::from(*value == 0);
        }
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

#[derive(Default)]
pub struct SoftBrushCoverage {
    stroke_id: u32,
    pixels: std::collections::HashMap<usize, (f32, [u8; 4])>,
}

impl SoftBrushCoverage {
    pub fn clear(&mut self) { self.pixels.clear(); }

    fn begin(&mut self, stroke_id: u32) {
        if self.stroke_id != stroke_id {
            self.clear();
            self.stroke_id = stroke_id;
        }
    }
}

pub struct BrushStroke<'a> {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub size: f32,
    pub opacity: f32,
    pub hardness: f32,
    pub erase: bool,
    pub color: [f32; 3],
    pub selection: Option<&'a Selection>,
    /// Generation marks: pixel is touched this stroke when `marks[i] == stroke_id`.
    pub coverage_marks: Option<&'a mut [u32]>,
    pub soft_coverage: Option<&'a mut SoftBrushCoverage>,
    pub stroke_id: u32,
}

/// Returns axis-aligned dirty pixel bounds if anything changed.
pub fn apply_brush(
    pixels: &mut (impl Raster + ?Sized),
    width: u32,
    height: u32,
    stroke: &mut BrushStroke<'_>,
) -> Option<PixelRect> {
    if width == 0 || height == 0 {
        return None;
    }
    let radius = (stroke.size * 0.5).max(0.5);
    let hardness = stroke.hardness.clamp(0.0, 1.0);
    let mut local_coverage = SoftBrushCoverage::default();
    let soft_coverage = stroke.soft_coverage.as_deref_mut().unwrap_or(&mut local_coverage);
    soft_coverage.begin(stroke.stroke_id);
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
            hardness,
            stroke.opacity.clamp(0.0, 1.0),
            stroke.erase,
            stroke.color,
            stroke.selection,
            stroke.coverage_marks.as_deref_mut(),
            stroke.stroke_id,
            soft_coverage,
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
    pixels: &mut (impl Raster + ?Sized),
    width: u32,
    height: u32,
    center_x: f32,
    center_y: f32,
    radius: f32,
    hardness: f32,
    opacity: f32,
    erase: bool,
    color: [f32; 3],
    selection: Option<&Selection>,
    mut coverage: Option<&mut [u32]>,
    stroke_id: u32,
    soft_coverage: &mut SoftBrushCoverage,
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
            let distance_sq = (px - center_x).powi(2) + (py - center_y).powi(2);
            if distance_sq > radius_sq {
                continue;
            }
            let index = (y * width + x) as usize;
            let mut pixel = pixels.read(width, x as i32, y as i32);
            let before = pixel;
            let opacity = if hardness < 1.0 {
                let inner = radius * hardness;
                let t = ((distance_sq.sqrt() - inner) / (radius - inner)).clamp(0.0, 1.0);
                let amount = opacity * (1.0 - t * t * (3.0 - 2.0 * t));
                if amount <= 0.0 { continue; }
                let (maximum, original) = soft_coverage.pixels.entry(index).or_insert((0.0, pixel));
                if amount <= *maximum { continue; }
                *maximum = amount;
                pixel = *original;
                amount
            } else {
                if let Some(cov) = coverage.as_mut() {
                    if index >= cov.len() {
                        continue;
                    }
                    if cov[index] == stroke_id {
                        continue;
                    }
                    cov[index] = stroke_id;
                }
                opacity
            };
            if erase { erase_pixel(&mut pixel, opacity); }
            else { paint_color(&mut pixel, opacity, color); }
            if pixel == before { continue; }
            pixels.write(width, x as i32, y as i32, pixel);
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

#[cfg(test)]
mod soft_brush_tests {
    use super::*;

    fn stroke<'a>(hardness: f32, coverage: Option<&'a mut SoftBrushCoverage>) -> BrushStroke<'a> {
        BrushStroke {
            from: (4.5, 4.5), to: (4.5, 4.5), size: 8.0,
            opacity: 1.0, hardness, erase: false, color: [1.0, 0.0, 0.0],
            selection: None, coverage_marks: None, soft_coverage: coverage, stroke_id: 1,
        }
    }

    #[test]
    fn hardness_controls_smooth_falloff_and_keeps_hard_edge_at_100_percent() {
        for (hardness, expected) in [(0.0, [255, 215, 128, 40, 0]), (0.5, [255, 255, 255, 128, 0]), (1.0, [255; 5])] {
            let mut pixels = vec![0; 9 * 9 * 4];
            apply_brush(&mut pixels, 9, 9, &mut stroke(hardness, None));
            for (distance, alpha) in expected.into_iter().enumerate() {
                assert_eq!(pixels.read(9, 4 + distance as i32, 4)[3], alpha);
            }
        }
    }

    #[test]
    fn soft_brush_strengthens_fringe_from_original_pixel_without_compounding() {
        for erase in [false, true] {
            let mut pixels = if erase { [10, 20, 30, 200].repeat(81) } else { vec![0; 81 * 4] };
            let mut coverage = SoftBrushCoverage::default();
            let mut brush = stroke(0.0, Some(&mut coverage));
            brush.opacity = 0.5;
            brush.erase = erase;
            brush.from = (2.5, 4.5);
            brush.to = brush.from;
            apply_brush(&mut pixels, 9, 9, &mut brush);
            assert_eq!(pixels.read(9, 4, 4)[3], if erase { 150 } else { 64 });
            brush.from = (4.5, 4.5);
            brush.to = brush.from;
            apply_brush(&mut pixels, 9, 9, &mut brush);
            assert_eq!(pixels.read(9, 4, 4)[3], if erase { 100 } else { 128 });
            let once = pixels.clone();
            apply_brush(&mut pixels, 9, 9, &mut brush);
            assert_eq!(pixels, once);
            brush.stroke_id = 2;
            apply_brush(&mut pixels, 9, 9, &mut brush);
            assert_eq!(pixels.read(9, 4, 4)[3], if erase { 50 } else { 192 });
            if erase { assert_eq!(&pixels.read(9, 4, 4)[..3], &[10, 20, 30]); }
        }
    }

    #[test]
    fn soft_brush_respects_selection_and_opacity() {
        let selection = Selection::from_rect(PixelRect { min_x: 4, min_y: 4, max_x: 5, max_y: 5 }, 9, 9);
        let mut pixels = vec![0; 9 * 9 * 4];
        let mut brush = stroke(0.0, None);
        brush.selection = Some(&selection);
        brush.opacity = 0.5;
        apply_brush(&mut pixels, 9, 9, &mut brush);
        assert_eq!(pixels.read(9, 4, 4), [255, 0, 0, 128]);
        assert_eq!(pixels.chunks_exact(4).filter(|pixel| pixel[3] != 0).count(), 1);
    }
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

pub fn sample_pixel(pixels: &(impl Raster + ?Sized), width: u32, x: u32, y: u32) -> [u8; 4] {
    pixels.read(width, x as i32, y as i32)
}

fn sample_pixel_clamped(pixels: &[u8], width: u32, height: u32, x: i32, y: i32) -> [u8; 4] {
    if width == 0 || height == 0 || pixels.len() < 4 {
        return [0; 4];
    }
    let x = x.clamp(0, width as i32 - 1) as u32;
    let y = y.clamp(0, height as i32 - 1) as u32;
    sample_pixel(pixels, width, x, y)
}

/// Bilinear sample with premultiplied-alpha interpolation (anti-aliased transform).
pub fn sample_bilinear(pixels: &[u8], width: u32, height: u32, fx: f32, fy: f32) -> [u8; 4] {
    if width == 0 || height == 0 {
        return [0; 4];
    }
    let x = fx - 0.5;
    let y = fy - 0.5;
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let tx = (x - x0 as f32).clamp(0.0, 1.0);
    let ty = (y - y0 as f32).clamp(0.0, 1.0);
    let p00 = sample_pixel_clamped(pixels, width, height, x0, y0);
    let p10 = sample_pixel_clamped(pixels, width, height, x0 + 1, y0);
    let p01 = sample_pixel_clamped(pixels, width, height, x0, y0 + 1);
    let p11 = sample_pixel_clamped(pixels, width, height, x0 + 1, y0 + 1);
    lerp_premul(
        lerp_premul(p00, p10, tx),
        lerp_premul(p01, p11, tx),
        ty,
    )
}

fn lerp_premul(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    let aa = a[3] as f32 / 255.0;
    let ba = b[3] as f32 / 255.0;
    let ar = a[0] as f32 * aa;
    let ag = a[1] as f32 * aa;
    let ab = a[2] as f32 * aa;
    let br = b[0] as f32 * ba;
    let bg = b[1] as f32 * ba;
    let bb = b[2] as f32 * ba;
    let out_a = aa * (1.0 - t) + ba * t;
    if out_a <= f32::EPSILON {
        return [0; 4];
    }
    let r = (ar * (1.0 - t) + br * t) / out_a;
    let g = (ag * (1.0 - t) + bg * t) / out_a;
    let bch = (ab * (1.0 - t) + bb * t) / out_a;
    [
        r.round().clamp(0.0, 255.0) as u8,
        g.round().clamp(0.0, 255.0) as u8,
        bch.round().clamp(0.0, 255.0) as u8,
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

fn mask_at(mask: &[u8], width: u32, height: u32, x: i32, y: i32) -> f32 {
    if x < 0 || y < 0 || (x as u32) >= width || (y as u32) >= height {
        return 0.0;
    }
    let idx = (y as u32 * width + x as u32) as usize;
    if mask.get(idx).copied().unwrap_or(0) != 0 {
        1.0
    } else {
        0.0
    }
}

/// Bilinear coverage for a hard selection mask (soft edge when transforming).
pub fn sample_mask_bilinear(mask: &[u8], width: u32, height: u32, fx: f32, fy: f32) -> f32 {
    if width == 0 || height == 0 {
        return 0.0;
    }
    let x = fx - 0.5;
    let y = fy - 0.5;
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let tx = (x - x0 as f32).clamp(0.0, 1.0);
    let ty = (y - y0 as f32).clamp(0.0, 1.0);
    let m00 = mask_at(mask, width, height, x0, y0);
    let m10 = mask_at(mask, width, height, x0 + 1, y0);
    let m01 = mask_at(mask, width, height, x0, y0 + 1);
    let m11 = mask_at(mask, width, height, x0 + 1, y0 + 1);
    let top = m00 * (1.0 - tx) + m10 * tx;
    let bot = m01 * (1.0 - tx) + m11 * tx;
    top * (1.0 - ty) + bot * ty
}

/// Area-average (box) resize — anti-aliased downscale / upsample for thumbs and general resample.
pub fn resize_rgba(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw as usize) * (dh as usize) * 4];
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < (sw * sh * 4) as usize {
        return out;
    }
    for dy in 0..dh {
        let y0 = dy as f64 * sh as f64 / dh as f64;
        let y1 = (dy + 1) as f64 * sh as f64 / dh as f64;
        for dx in 0..dw {
            let x0 = dx as f64 * sw as f64 / dw as f64;
            let x1 = (dx + 1) as f64 * sw as f64 / dw as f64;
            let px = area_sample_rgba(src, sw, sh, x0, y0, x1, y1);
            let o = ((dy * dw + dx) * 4) as usize;
            out[o..o + 4].copy_from_slice(&px);
        }
    }
    out
}

fn area_sample_rgba(
    src: &[u8],
    sw: u32,
    sh: u32,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
) -> [u8; 4] {
    let x0 = x0.clamp(0.0, sw as f64);
    let x1 = x1.clamp(0.0, sw as f64).max(x0);
    let y0 = y0.clamp(0.0, sh as f64);
    let y1 = y1.clamp(0.0, sh as f64).max(y0);
    if x1 - x0 <= f64::EPSILON || y1 - y0 <= f64::EPSILON {
        return [0; 4];
    }
    let ix0 = x0.floor() as u32;
    let iy0 = y0.floor() as u32;
    let ix1 = ((x1 - f64::EPSILON).floor() as u32).min(sw.saturating_sub(1));
    let iy1 = ((y1 - f64::EPSILON).floor() as u32).min(sh.saturating_sub(1));
    let mut acc = [0.0f64; 4]; // premul rgb + a, plus weight in unused — use separate weight
    let mut weight = 0.0f64;
    for y in iy0..=iy1 {
        let py0 = y as f64;
        let py1 = py0 + 1.0;
        let wy = (y1.min(py1) - y0.max(py0)).max(0.0);
        if wy <= 0.0 {
            continue;
        }
        for x in ix0..=ix1 {
            let px0 = x as f64;
            let px1 = px0 + 1.0;
            let wx = (x1.min(px1) - x0.max(px0)).max(0.0);
            if wx <= 0.0 {
                continue;
            }
            let w = wx * wy;
            let o = ((y * sw + x) * 4) as usize;
            let a = src[o + 3] as f64 / 255.0;
            acc[0] += src[o] as f64 * a * w;
            acc[1] += src[o + 1] as f64 * a * w;
            acc[2] += src[o + 2] as f64 * a * w;
            acc[3] += a * w;
            weight += w;
        }
    }
    if weight <= f64::EPSILON || acc[3] <= f64::EPSILON {
        return [0; 4];
    }
    let a = acc[3] / weight;
    if a <= f64::EPSILON {
        return [0; 4];
    }
    // acc[0..3]/weight is already premul in 0..255 units; un-premultiply without *255.
    [
        ((acc[0] / weight) / a).round().clamp(0.0, 255.0) as u8,
        ((acc[1] / weight) / a).round().clamp(0.0, 255.0) as u8,
        ((acc[2] / weight) / a).round().clamp(0.0, 255.0) as u8,
        (a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// 4-connected flood fill visiting pixels within `threshold` of the seed color.
pub fn flood_mask(
    pixels: &(impl Raster + ?Sized),
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
    pixels: &mut (impl Raster + ?Sized),
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
            let mut pixel = pixels.read(width, x as i32, y as i32);
            if erase { erase_pixel(&mut pixel, opacity); }
            else { paint_color(&mut pixel, opacity, color); }
            pixels.write(width, x as i32, y as i32, pixel);
            changed = true;
        }
    }
    changed.then_some(bounds)
}

/// One composited pixel: visible layers, opacity, and offsets. Straight RGBA.
pub fn composite_pixel(layers: &[crate::project::Layer], x: i32, y: i32) -> [u8; 4] {
    let mut dst = [0u8; 4];
    for layer in layers {
        if !layer.visible {
            continue;
        }
        let opacity = layer.opacity.clamp(0.0, 1.0);
        if opacity <= f32::EPSILON {
            continue;
        }
        over(&mut dst, &layer.pixels.read(0, x, y), opacity);
    }
    dst
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
        if !layer.visible {
            continue;
        }
        let opacity = layer.opacity.clamp(0.0, 1.0);
        if opacity <= f32::EPSILON {
            continue;
        }
        let side = crate::tiles::TILE_SIZE as i32;
        for (&(tx,ty), tile) in &layer.pixels.tiles {
            let ox = tx*side + layer.pixels.offset.0;
            let oy = ty*side + layer.pixels.offset.1;
            let x0 = ox.max(x as i32); let x1 = (ox+side).min(x1 as i32);
            let y0 = oy.max(y as i32); let y1 = (oy+side).min(y1 as i32);
            for py in y0..y1 {
                for px in x0..x1 {
                    let dst = (py as usize * width as usize + px as usize)*4;
                    let src = ((py-oy)*side+px-ox) as usize*4;
                    over(&mut out[dst..dst+4], &tile[src..src+4], opacity);
                }
            }
        }
    }
}

/// Merge physical tiles without clipping to the document or allocating the gap between them.
pub fn merge_layers(lower: &crate::project::Layer, upper: &crate::project::Layer) -> crate::project::Layer {
    let mut pixels = crate::raster::SparsePixels { offset: lower.pixels.offset, ..Default::default() };
    let side = crate::tiles::TILE_SIZE as i32;
    for layer in [lower, upper] {
        let opacity = layer.opacity.clamp(0.0, 1.0);
        if !layer.visible || opacity <= f32::EPSILON { continue; }
        for (&(tx, ty), tile) in &layer.pixels.tiles {
            for (i, source) in tile.chunks_exact(4).enumerate() {
                if source[3] == 0 { continue; }
                let x = tx * side + (i % side as usize) as i32 + layer.pixels.offset.0;
                let y = ty * side + (i / side as usize) as i32 + layer.pixels.offset.1;
                let mut destination = pixels.read(0, x, y);
                over(&mut destination, source, opacity);
                pixels.write(0, x, y, destination);
            }
        }
    }
    pixels.prune();
    crate::project::Layer {
        name: lower.name.clone(), visible: lower.visible || upper.visible, opacity: 1.0, pixels,
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use crate::{project::Layer, raster::SparsePixels};

    #[test]
    fn composite_pixel_matches_the_full_image_composite() {
        let mut lower = Layer { name: "Lower".into(), visible: true, opacity: 0.5,
            pixels: SparsePixels::from_rgba(&[200, 40, 80, 255, 0, 0, 0, 0, 10, 20, 30, 128], 3, 1) };
        lower.pixels.offset = (1, 2);
        let mut upper = Layer { name: "Upper".into(), visible: false, opacity: 1.0,
            pixels: SparsePixels::from_rgba(&[255, 255, 255, 255].repeat(3), 3, 1) };
        upper.pixels.offset = (1, 2);
        let layers = [lower, upper];
        let full = composite_layers(&layers, 5, 4);
        for y in 0..4u32 {
            for x in 0..5u32 {
                let i = ((y * 5 + x) * 4) as usize;
                assert_eq!(composite_pixel(&layers, x as i32, y as i32), full[i..i + 4]);
            }
        }
    }

    #[test]
    fn merge_down_matches_composite_with_alpha_opacity_offsets_and_visibility() {
        for lower_visible in [false, true] {
            for upper_visible in [false, true] {
                let mut lower = Layer { name: "Lower".into(), visible: lower_visible, opacity: 0.6,
                    pixels: SparsePixels::from_rgba(&[200, 40, 80, 128].repeat(6), 3, 2) };
                lower.pixels.offset = (1, 1);
                let mut upper = Layer { name: "Upper".into(), visible: upper_visible, opacity: 0.4,
                    pixels: SparsePixels::from_rgba(&[20, 80, 240, 192].repeat(6), 2, 3) };
                upper.pixels.offset = (2, 0);
                let original = composite_layers(&[lower.clone(), upper.clone()], 5, 4);
                let merged = merge_layers(&lower, &upper);
                assert_eq!(merged.name, "Lower");
                assert_eq!(merged.opacity, 1.0);
                assert_eq!(composite_layers(&[merged], 5, 4), original);
            }
        }
    }

    #[test]
    fn merge_down_preserves_distant_offcanvas_pixels_without_filling_the_gap() {
        let mut lower = Layer { name: "Lower".into(), visible: true, opacity: 1.0, pixels: SparsePixels::default() };
        lower.pixels.offset = (-7, 9);
        lower.pixels.write(0, -100, -200, [1, 2, 3, 255]);
        let mut upper = lower.clone();
        upper.pixels = SparsePixels::default();
        upper.pixels.offset = (15, -17);
        upper.pixels.write(0, 1_000_000, 2_000_000, [4, 5, 6, 255]);
        let merged = merge_layers(&lower, &upper);
        assert_eq!(merged.pixels.read(0, -100, -200), [1, 2, 3, 255]);
        assert_eq!(merged.pixels.read(0, 1_000_000, 2_000_000), [4, 5, 6, 255]);
        assert_eq!(merged.pixels.tiles.len(), 2);
        assert_eq!(merged.pixels.read(0, 0, 0), [0; 4]);
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

fn lanczos3(x: f32) -> f32 {
    let x = x.abs();
    if x < 1.0e-5 { return 1.0; }
    if x >= 3.0 { return 0.0; }
    let p = std::f32::consts::PI * x;
    p.sin() * (p / 3.0).sin() / (p * p / 3.0)
}

fn linear_to_srgb(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let value = if value <= 0.0031308 { value * 12.92 }
        else { 1.055 * value.powf(1.0 / 2.4) - 0.055 };
    (value * 255.0).round() as u8
}

/// Scale-aware Lanczos3 in linear light and premultiplied alpha. Missing source
/// pixels are transparent; the selection is filtered together with the image.
/// Widening the kernel when shrinking is essential to suppress aliasing.
fn sample_transform(x: f32, y: f32, scale: f32,
    read: impl Fn(i32, i32) -> ([u8; 4], f32)) -> ([u8; 4], f32) {
    static LINEAR: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    let linear = LINEAR.get_or_init(|| std::array::from_fn(|i| {
        let s = i as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    }));
    let scale = scale.clamp(0.05, 1.0);
    let radius = 3.0 / scale;
    let (x, y) = (x - 0.5, y - 0.5);
    // At the minimum Transform scale there are at most 120 taps per axis.
    let x0 = (x - radius).ceil() as i32;
    let x1 = (x + radius).floor() as i32;
    let mut wx = [0.0; 121];
    for sx in x0..=x1 { wx[(sx - x0) as usize] = lanczos3((sx as f32 - x) * scale); }
    let mut sum = [0.0; 4];
    let mut weight = 0.0;
    let mut coverage = 0.0;
    for sy in (y - radius).ceil() as i32..=(y + radius).floor() as i32 {
        let wy = lanczos3((sy as f32 - y) * scale);
        for sx in x0..=x1 {
            let w = wx[(sx - x0) as usize] * wy;
            weight += w;
            if w.abs() < 1.0e-10 { continue; }
            let (p, mask) = read(sx, sy);
            coverage += w * mask;
            let a = p[3] as f32 / 255.0 * mask * w;
            for c in 0..3 { sum[c] += linear[p[c] as usize] * a; }
            sum[3] += a;
        }
    }
    if weight <= f32::EPSILON { return ([0; 4], 0.0); }
    let coverage = (coverage / weight).clamp(0.0, 1.0);
    let alpha = (sum[3] / weight).clamp(0.0, 1.0);
    if alpha < 0.5 / 255.0 { return ([0; 4], coverage); }
    ([linear_to_srgb(sum[0] / sum[3]), linear_to_srgb(sum[1] / sum[3]),
        linear_to_srgb(sum[2] / sum[3]), (alpha * 255.0).round() as u8], coverage)
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
    pub scale: f32,
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
            scale: 1.0,
            rotation: 0.0,
            opacity: opacity.clamp(0.0, 1.0),
            whole_layer: false,
            source: None,
        })
    }

    /// Erase grabbed pixels. Call on commit, after the preview has been sampling them.
    pub fn extract_layer(pixels: &crate::raster::SparsePixels, layer: usize, opacity: f32) -> Option<Self> {
        let (x,y,x1,y1) = pixels.bounds()?;
        let bw = (x1-x) as u32; let bh = (y1-y) as u32;
        Some(Self {
            layer, pixels: Vec::new(), mask: Vec::new(),
            bw, bh, origin_x: x, origin_y: y,
            center: glam::Vec2::new(x as f32 + bw as f32 * 0.5, y as f32 + bh as f32 * 0.5),
            pivot: glam::Vec2::new(x as f32 + bw as f32 * 0.5, y as f32 + bh as f32 * 0.5),
            scale: 1.0, rotation: 0.0, opacity, whole_layer: true, source: Some(pixels.clone()),
        })
    }

    pub fn clear_source(&self, pixels: &mut (impl Raster + ?Sized), width: u32, _height: u32) {
        for y in 0..self.bh {
            for x in 0..self.bw {
                if self.mask[(y * self.bw + x) as usize] != 0 {
                    pixels.write(width, self.origin_x + x as i32, self.origin_y + y as i32, [0;4]);
                }
            }
        }
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

    pub fn drag(&mut self, hit: TransformHit, from: glam::Vec2, to: glam::Vec2) {
        match hit {
            TransformHit::Body => {
                let delta = to-from;
                self.center += delta;
                self.pivot += delta;
            }
            TransformHit::Pivot => self.pivot += to-from,
            TransformHit::Rotate => {
                let a = from-self.pivot;
                let b = to-self.pivot;
                if a.length_squared() < 1.0e-8 || b.length_squared() < 1.0e-8 { return; }
                let angle = a.perp_dot(b).atan2(a.dot(b));
                let (s,c) = angle.sin_cos();
                let delta = self.center-self.pivot;
                self.center = self.pivot+glam::Vec2::new(delta.x*c-delta.y*s,delta.x*s+delta.y*c);
                self.rotation += angle;
            }
            TransformHit::Scale(_) => {
                let distance = (from-self.pivot).length();
                if distance < 1.0e-4 { return; }
                let scale = (self.scale*(to-self.pivot).length()/distance).clamp(0.05,32.0);
                self.center = self.pivot+(self.center-self.pivot)*(scale/self.scale);
                self.scale = scale;
            }
        }
    }

    pub fn hit_test(&self, image_pos: glam::Vec2, handle_r: f32) -> Option<TransformHit> {
        let corners = self.corners();
        let mut nearest = (self.pivot-image_pos).length();
        let mut handle = TransformHit::Pivot;
        for (i, c) in corners.iter().enumerate() {
            let distance = (*c-image_pos).length();
            if distance < nearest {
                nearest = distance;
                handle = TransformHit::Scale(i as u8);
            }
        }
        if nearest <= handle_r { return Some(handle); }
        // Inverse transform point into local unscaled bbox.
        let delta = image_pos - self.center;
        let (s, c) = (-self.rotation).sin_cos();
        let local = glam::Vec2::new(delta.x * c - delta.y * s, delta.x * s + delta.y * c)
            / self.scale.max(0.05);
        let half = glam::Vec2::new(self.bw as f32, self.bh as f32) * 0.5;
        if local.x.abs() <= half.x && local.y.abs() <= half.y {
            return Some(TransformHit::Body);
        }
        Some(TransformHit::Rotate)
    }

    /// Resample occupied source tiles only, even when the layer spans a huge empty area.
    pub fn stamp_layer(&self, target: &mut crate::raster::SparsePixels) {
        let Some(source) = self.source.as_ref() else { return; };
        let side = crate::tiles::TILE_SIZE as i32;
        let half = glam::Vec2::new(self.bw as f32, self.bh as f32) * 0.5;
        let origin = glam::Vec2::new(self.origin_x as f32, self.origin_y as f32) + half;
        let (s,c) = self.rotation.sin_cos();
        let radius = 3.0 / self.scale.clamp(0.05, 1.0);
        let transform = |p: glam::Vec2| {
            let p = (p - origin) * self.scale;
            self.center + glam::Vec2::new(p.x*c-p.y*s, p.x*s+p.y*c)
        };
        let inverse = |p: glam::Vec2| {
            let p = p - self.center;
            glam::Vec2::new(p.x*c+p.y*s,-p.x*s+p.y*c)/self.scale.max(0.05)+origin
        };
        let mut destinations = std::collections::HashSet::new();
        for &(tx,ty) in source.tiles.keys() {
            let x = tx * side + source.offset.0;
            let y = ty * side + source.offset.1;
            let corners = [(x,y),(x+side,y),(x+side,y+side),(x,y+side)]
                .map(|(x,y)| glam::Vec2::new(x as f32,y as f32));
            let corners = [corners[0] - glam::Vec2::splat(radius),
                corners[1] + glam::Vec2::new(radius,-radius),
                corners[2] + glam::Vec2::splat(radius),
                corners[3] + glam::Vec2::new(-radius,radius)].map(transform);
            let x0 = corners.iter().map(|p|p.x).fold(f32::INFINITY,f32::min).floor() as i32;
            let y0 = corners.iter().map(|p|p.y).fold(f32::INFINITY,f32::min).floor() as i32;
            let x1 = corners.iter().map(|p|p.x).fold(f32::NEG_INFINITY,f32::max).ceil() as i32;
            let y1 = corners.iter().map(|p|p.y).fold(f32::NEG_INFINITY,f32::max).ceil() as i32;
            for dy in y0.div_euclid(side)..=y1.div_euclid(side) {
                for dx in x0.div_euclid(side)..=x1.div_euclid(side) { destinations.insert((dx,dy)); }
            }
        }
        for (tx,ty) in destinations {
            // Pack this tile's source footprint once, avoiding a sparse-map lookup
            // for every Lanczos tap (especially costly during strong reductions).
            let corners = [(tx*side,ty*side),((tx+1)*side,ty*side),
                ((tx+1)*side,(ty+1)*side),(tx*side,(ty+1)*side)]
                .map(|(x,y)| inverse(glam::Vec2::new(x as f32,y as f32)));
            let sx = (corners.iter().map(|p|p.x).fold(f32::INFINITY,f32::min)-radius).floor() as i32;
            let sy = (corners.iter().map(|p|p.y).fold(f32::INFINITY,f32::min)-radius).floor() as i32;
            let ex = (corners.iter().map(|p|p.x).fold(f32::NEG_INFINITY,f32::max)+radius).ceil() as i32;
            let ey = (corners.iter().map(|p|p.y).fold(f32::NEG_INFINITY,f32::max)+radius).ceil() as i32;
            let stride = (ex-sx+1) as u32;
            let packed = source.region(sx,sy,stride,(ey-sy+1) as u32);
            for y in ty*side..(ty+1)*side {
                for x in tx*side..(tx+1)*side {
                    let p = inverse(glam::Vec2::new(x as f32+0.5,y as f32+0.5));
                    if p.x < self.origin_x as f32 - radius || p.y < self.origin_y as f32 - radius
                        || p.x > (self.origin_x as f32 + self.bw as f32) + radius
                        || p.y > (self.origin_y as f32 + self.bh as f32) + radius { continue; }
                    let pixel = sample_transform(p.x,p.y,self.scale,|x,y| (packed.read(stride,x-sx,y-sy),1.0)).0;
                    if pixel[3] != 0 { target.write(0,x,y,pixel); }
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
            .floor()
             as i32;
        let min_y = corners
            .iter()
            .map(|p| p.y)
            .fold(f32::INFINITY, f32::min)
            .floor()
             as i32;
        let max_x = corners
            .iter()
            .map(|p| p.x)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
             as i32;
        let max_y = corners
            .iter()
            .map(|p| p.y)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
             as i32;
        let half = glam::Vec2::new(self.bw as f32, self.bh as f32) * 0.5;
        let inv_scale = 1.0 / self.scale.max(0.05);
        let (s, c) = (-self.rotation).sin_cos();
        // Include the complete rotated filter footprint, including ringing lobes.
        let radius = 3.0 / self.scale.clamp(0.05, 1.0);
        let pad = (radius * self.scale.max(0.05) * (s.abs() + c.abs())).ceil() as i32;
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
                if lx < -radius || ly < -radius || lx >= self.bw as f32 + radius || ly >= self.bh as f32 + radius
                {
                    continue;
                }
                let (src_px, coverage) = sample_transform(lx,ly,self.scale,|x,y| {
                    if x < 0 || y < 0 || x >= self.bw as i32 || y >= self.bh as i32 {
                        return ([0;4],0.0);
                    }
                    let i = (y as u32 * self.bw + x as u32) as usize;
                    (self.pixels[i*4..i*4+4].try_into().unwrap(),
                        if self.mask[i] != 0 { 1.0 } else { 0.0 })
                });
                if src_px[3] == 0 {
                    continue;
                }
                let mut pixel = pixels.read(width, x, y);
                over(&mut pixel, &src_px, 1.0);
                pixels.write(width, x, y, pixel);
                if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 { continue; }
                let (x,y) = (x as u32, y as u32);
                if coverage >= 0.5 { mask[(y * width + x) as usize] = 1; }
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
    fn lasso_fills_concave_polygon_and_clips_outside_canvas() {
        let points = [(-2.0, -2.0), (4.0, -2.0), (4.0, 1.0), (1.0, 1.0), (1.0, 4.0), (-2.0, 4.0)].map(|(x, y)| glam::Vec2::new(x, y));
        let selection = Selection::from_polygon(&points, 5, 5);
        assert!(selection.contains(0, 3));
        assert!(selection.contains(3, 0));
        assert!(!selection.contains(2, 2));
        assert!(!selection.contains(4, 0));
        let reversed: Vec<_> = points.into_iter().rev().collect();
        assert_eq!(selection.mask, Selection::from_polygon(&reversed, 5, 5).mask);
        assert!(Selection::from_polygon(&points[..2], 5, 5).is_empty());
    }

    #[test]
    fn selection_brush_has_continuous_coverage_adds_and_erases_locally() {
        let mut selection = Selection::from_polygon(&[], 12, 8);
        selection.paint_segment(glam::Vec2::new(-2.0, 2.5), glam::Vec2::new(10.5, 2.5), 3.0, false);
        for x in 0..11 { assert!(selection.contains(x, 2)); }
        selection.paint_segment(glam::Vec2::new(6.5, 2.5), glam::Vec2::new(6.5, 6.5), 1.0, false);
        selection.paint_segment(glam::Vec2::new(3.5, 2.5), glam::Vec2::new(3.5, 2.5), 1.0, true);
        assert!(!selection.contains(3, 2));
        assert!(selection.contains(2, 2));
        assert!(selection.contains(6, 6));
        assert!(!selection.contains(11, 7));
    }

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
            hardness: 1.0,
            opacity: 1.0,
            erase: false,
            color: [0.0, 0.0, 0.0],
            selection: Some(&selection),
            coverage_marks: None,
            soft_coverage: None,
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
            hardness: 1.0,
            opacity: 0.5,
            erase: true,
            color: [0.0, 0.0, 0.0],
            selection: None,
            coverage_marks: None,
            soft_coverage: None,
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
            hardness: 1.0,
            opacity: 0.5,
            erase: true,
            color: [0.0, 0.0, 0.0],
            selection: None,
            coverage_marks: Some(&mut marks),
            soft_coverage: None,
            stroke_id: 7,
        };
        apply_brush(&mut pixels, 1, 1, &mut stroke);
        let mut stroke = BrushStroke {
            from: (0.5, 0.5),
            to: (0.5, 0.5),
            size: 1.0,
            hardness: 1.0,
            opacity: 0.5,
            erase: true,
            color: [0.0, 0.0, 0.0],
            selection: None,
            coverage_marks: Some(&mut marks),
            soft_coverage: None,
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
            hardness: 1.0,
            opacity: 1.0,
            erase: false,
            color: [1.0, 0.0, 0.5],
            selection: None,
            coverage_marks: None,
            soft_coverage: None,
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

    #[test]
    fn resize_area_average_blends_checker() {
        // 2x2: white / black → 1x1 should be mid grey, not pure nearest.
        let src = [
            255, 255, 255, 255, 0, 0, 0, 255, //
            0, 0, 0, 255, 255, 255, 255, 255,
        ];
        let out = resize_rgba(&src, 2, 2, 1, 1);
        assert!(out[0] > 64 && out[0] < 192, "got {}", out[0]);
        assert_eq!(out[3], 255);
    }

    #[test]
    fn bilinear_sample_midpoint_blends() {
        let src = [255, 0, 0, 255, 0, 0, 255, 255];
        let mid = sample_bilinear(&src, 2, 1, 1.0, 0.5);
        assert!(mid[0] > 64 && mid[0] < 192);
        assert!(mid[2] > 64 && mid[2] < 192);
    }

    #[test]
    fn transform_downscale_filters_stripes_for_layers_and_selections() {
        let mut pixels = vec![0; 96*96*4];
        for y in 0..96 { for x in 0..96 {
            let v = if x % 3 == 0 { 255 } else { 0 };
            pixels[(y*96+x)*4..(y*96+x)*4+4].copy_from_slice(&[v,v,v,255]);
        } }
        // Bilinear alone aliases this reduction to black instead of averaging.
        assert_eq!(sample_bilinear(&pixels,96,96,49.5,49.5),[0,0,0,255]);
        let source = crate::raster::SparsePixels::from_rgba(&pixels,96,96);
        let mut layer = FloatingTransform::extract_layer(&source,0,1.0).unwrap();
        layer.scale = 1.0/3.0;
        layer.center = glam::Vec2::splat(16.0);
        let mut output = crate::raster::SparsePixels::default();
        layer.stamp_layer(&mut output);
        let selection = Selection::from_rect(PixelRect { min_x:0,min_y:0,max_x:96,max_y:96 },96,96);
        let mut fragment = FloatingTransform::extract(&source,96,96,&selection,0,1.0).unwrap();
        fragment.scale = layer.scale;
        fragment.center = layer.center;
        let mut selected = crate::raster::SparsePixels::default();
        fragment.stamp(&mut selected,96,96);
        // One third white in linear light is about sRGB 156. The probes also
        // straddle a source tile boundary (64), which must not introduce seams.
        for x in 10..25 {
            let p = output.read(96,x,16);
            assert!((p[0] as i32-156).abs() <= 2,"x={x}: {p:?}");
            assert_eq!(p[3],255);
            assert_eq!(selected.read(96,x,16),p);
        }
    }

    #[test]
    fn transform_pivot_moves_independently_and_anchors_rotation_and_scale() {
        let source = crate::raster::SparsePixels::from_rgba(&[10,20,30,255].repeat(100*80),100,80);
        let mut transform = FloatingTransform::extract_layer(&source,0,1.0).unwrap();
        let corners = transform.corners();
        assert_eq!(transform.pivot,transform.center);
        transform.drag(TransformHit::Pivot,transform.pivot,glam::Vec2::new(30.0,50.0));
        assert_eq!(transform.corners(),corners,"moving pivot must not move the image");
        let pivot = transform.pivot;
        transform.drag(TransformHit::Rotate,pivot+glam::Vec2::X*20.0,pivot+glam::Vec2::Y*20.0);
        assert_eq!(transform.pivot,pivot);
        assert!((transform.rotation-std::f32::consts::FRAC_PI_2).abs()<1.0e-5);
        for (before,after) in corners.into_iter().zip(transform.corners()) {
            let delta = before-pivot;
            let expected = pivot+glam::Vec2::new(-delta.y,delta.x);
            assert!((after-expected).length()<1.0e-4);
        }
        let corners = transform.corners();
        transform.drag(TransformHit::Scale(0),pivot+glam::Vec2::X*10.0,pivot+glam::Vec2::X*20.0);
        assert_eq!(transform.scale,2.0);
        assert_eq!(transform.pivot,pivot);
        for (before,after) in corners.into_iter().zip(transform.corners()) {
            assert!((after-(pivot+(before-pivot)*2.0)).length()<1.0e-4);
        }
        let center = transform.center;
        let delta = glam::Vec2::new(-15.0,7.0);
        transform.drag(TransformHit::Body,glam::Vec2::ZERO,delta);
        assert_eq!(transform.pivot,pivot+delta);
        assert_eq!(transform.center,center+delta);
    }

    #[test]
    fn transform_outside_rotates_and_handles_follow_rotated_bounds() {
        let source = crate::raster::SparsePixels::from_rgba(&[10,20,30,255].repeat(100*80),100,80);
        let mut transform = FloatingTransform::extract_layer(&source,0,1.0).unwrap();
        for rotation in [0.0,0.7,std::f32::consts::FRAC_PI_2] {
            transform.rotation = rotation;
            assert_eq!(transform.hit_test(transform.pivot,8.0),Some(TransformHit::Pivot));
            let corners = transform.corners();
            for (i,corner) in corners.into_iter().enumerate() {
                assert_eq!(transform.hit_test(corner,8.0),Some(TransformHit::Scale(i as u8)));
            }
            assert_eq!(transform.hit_test(corners[0]*0.75+corners[2]*0.25,8.0),Some(TransformHit::Body));
            assert_eq!(transform.hit_test(corners[0]+(corners[0]-transform.center),8.0),Some(TransformHit::Rotate));
        }
        // Crossing +/-180 degrees must produce a small step, not a full turn.
        let direction = |degrees:f32| glam::Vec2::new(degrees.to_radians().cos(),degrees.to_radians().sin())*100.0;
        let before = transform.rotation;
        transform.drag(TransformHit::Rotate,transform.pivot+direction(179.0),transform.pivot+direction(-179.0));
        assert!((transform.rotation-before-2.0f32.to_radians()).abs()<1.0e-5);
    }

    #[test]
    #[ignore = "manual Transform commit performance measurement"]
    fn transform_commit_performance() {
        let source = crate::raster::SparsePixels::from_rgba(&[120,180,240,255].repeat(1024*1024),1024,1024);
        for scale in [0.5,0.05] {
            let mut transform = FloatingTransform::extract_layer(&source,0,1.0).unwrap();
            transform.scale = scale;
            let mut output = crate::raster::SparsePixels::default();
            let started = std::time::Instant::now();
            transform.stamp_layer(&mut output);
            eprintln!("1024x1024 commit at {scale}: {:?}",started.elapsed());
            assert!(!output.tiles.is_empty());
        }
    }

    #[test]
    fn transform_filter_preserves_identity_alpha_and_mask() {
        let pixel = [17,128,240,93];
        assert_eq!(sample_transform(0.5,0.5,1.0,|x,y| {
            if x==0 && y==0 { (pixel,1.0) } else { ([0;4],0.0) }
        }).0,pixel);
        let (p,coverage) = sample_transform(0.5,0.5,0.25,|x,_| {
            // Unselected opaque blue and transparent green must not bleed into red.
            if x < 0 { ([0,0,255,255],0.0) }
            else if x % 2 == 0 { ([255,0,0,255],1.0) }
            else { ([0,255,0,0],1.0) }
        });
        assert_eq!(&p[..3],&[255,0,0]);
        assert!(p[3] > 0 && p[3] < 255);
        assert!(coverage > 0.0 && coverage < 1.0);
    }

    #[test]
    fn rotated_transform_downscale_averages_checker_and_retains_off_canvas_pixels() {
        let mut pixels = vec![0; 64*64*4];
        for y in 0..64 { for x in 0..64 {
            let v = if (x+y)%2==0 {255} else {0};
            pixels[(y*64+x)*4..(y*64+x)*4+4].copy_from_slice(&[v,v,v,255]);
        } }
        let source = crate::raster::SparsePixels::from_rgba(&pixels,64,64);
        let mut transform = FloatingTransform::extract_layer(&source,0,1.0).unwrap();
        transform.scale = 0.2;
        transform.rotation = 0.6;
        transform.center = glam::Vec2::splat(-10.0);
        let mut output = crate::raster::SparsePixels::default();
        transform.stamp_layer(&mut output);
        for y in -12..-8 { for x in -12..-8 {
            let p = output.read(0,x,y);
            assert!((p[0] as i32-188).abs() <= 2,"({x},{y}): {p:?}");
            assert_eq!(p[3],255);
        } }
    }
}
