use super::selection::{PixelRect, Selection};
use crate::raster::Raster;

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

pub(super) fn stroke_color(color: [f32; 3]) -> [f32; 3] {
    [
        color[0].clamp(0.0, 1.0),
        color[1].clamp(0.0, 1.0),
        color[2].clamp(0.0, 1.0),
    ]
}

pub(super) fn paint_color(pixel: &mut [u8], opacity: f32, color: [f32; 3]) {
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

pub(super) fn erase_pixel(pixel: &mut [u8], opacity: f32) {
    pixel[3] = (pixel[3] as f32 * (1.0 - opacity))
        .round()
        .clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod integration_tests {
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
}
