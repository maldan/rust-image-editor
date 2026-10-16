#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Brush,
    Eraser,
    RectangleSelect,
    MagicWand,
    Fill,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Brush => "Brush",
            Tool::Eraser => "Eraser",
            Tool::RectangleSelect => "Select",
            Tool::MagicWand => "Wand",
            Tool::Fill => "Fill",
        }
    }

    pub fn all() -> [Tool; 5] {
        [
            Tool::Brush,
            Tool::Eraser,
            Tool::RectangleSelect,
            Tool::MagicWand,
            Tool::Fill,
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
    clip: Option<&Selection>,
) -> bool {
    let Some(region) = flood_mask(pixels, width, height, seed, threshold, clip) else {
        return false;
    };
    let color = stroke_color(color);
    let opacity = opacity.clamp(0.0, 1.0);
    let mut changed = false;
    for y in 0..height {
        for x in 0..width {
            if !region.contains(x, y) {
                continue;
            }
            let offset = ((y * width + x) * 4) as usize;
            paint_color(&mut pixels[offset..offset + 4], opacity, color);
            changed = true;
        }
    }
    changed
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
        for py in y..y1 {
            for px in x..x1 {
                let offset = ((py * width + px) * 4) as usize;
                over(
                    &mut out[offset..offset + 4],
                    &layer.pixels[offset..offset + 4],
                );
            }
        }
    }
}

fn over(dst: &mut [u8], src: &[u8]) {
    let src_a = src[3] as f32 / 255.0;
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
        fill_bucket(&mut pixels, 2, 1, (0, 0), 0, [1.0, 0.0, 0.0], 1.0, None);
        assert_eq!(&pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&pixels[4..8], &[0, 0, 0, 255]);
    }
}
