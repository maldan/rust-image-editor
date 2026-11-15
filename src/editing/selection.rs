#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub min_x: u32,
    pub min_y: u32,
    pub max_x: u32,
    pub max_y: u32,
}

impl PixelRect {
    pub fn from_points(a: (u32, u32), b: (u32, u32), width: u32, height: u32) -> Self {
        Self::from_signed_points(
            (a.0 as i32, a.1 as i32),
            (b.0 as i32, b.1 as i32),
            width,
            height,
        )
    }

    /// Build a document-clamped rect from signed image pixels (drag may start outside).
    pub fn from_signed_points(
        a: (i32, i32),
        b: (i32, i32),
        width: u32,
        height: u32,
    ) -> Self {
        let raw_min_x = a.0.min(b.0);
        let raw_min_y = a.1.min(b.1);
        let raw_max_x = a.0.max(b.0).saturating_add(1);
        let raw_max_y = a.1.max(b.1).saturating_add(1);
        let min_x = raw_min_x.max(0) as u32;
        let min_y = raw_min_y.max(0) as u32;
        let max_x = raw_max_x.max(0) as u32;
        let max_y = raw_max_y.max(0) as u32;
        Self {
            min_x: min_x.min(width),
            min_y: min_y.min(height),
            max_x: max_x.min(width),
            max_y: max_y.min(height),
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
    cached_bounds: Option<PixelRect>,
}

impl Selection {
    pub fn from_mask(width: u32, height: u32, mask: Vec<u8>) -> Self {
        let mut result = Self {
            width,
            height,
            mask,
            cached_bounds: None,
        };
        result.cached_bounds = result.scan_bounds();
        result
    }

    /// Scanline polygon fill, testing pixel centers and clipping to the canvas.
    pub fn from_polygon(points: &[glam::Vec2], width: u32, height: u32) -> Self {
        let mut result = Self {
            width,
            height,
            mask: vec![0; width as usize * height as usize],
            cached_bounds: None,
        };
        if points.len() < 3 {
            return result;
        }
        let min_y = points
            .iter()
            .map(|p| p.y)
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as u32;
        let max_y = points
            .iter()
            .map(|p| p.y)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min(height as f32) as u32;
        let mut crossings = Vec::new();
        for y in min_y..max_y {
            crossings.clear();
            let scan_y = y as f32 + 0.5;
            let mut previous = points[points.len() - 1];
            for &current in points {
                if (previous.y > scan_y) != (current.y > scan_y) {
                    crossings.push(
                        previous.x
                            + (scan_y - previous.y) * (current.x - previous.x)
                                / (current.y - previous.y),
                    );
                }
                previous = current;
            }
            crossings.sort_by(f32::total_cmp);
            for pair in crossings.chunks_exact(2) {
                let start = (pair[0] - 0.5).ceil().clamp(0.0, width as f32) as u32;
                let end = (pair[1] - 0.5).ceil().clamp(0.0, width as f32) as u32;
                for x in start..end {
                    result.mask[(y * width + x) as usize] = 1;
                    result.include_pixel(x, y);
                }
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
                let t = if length_sq > 0.0 {
                    ((point - from).dot(delta) / length_sq).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                if point.distance_squared(from + delta * t) <= radius_sq {
                    self.mask[(y * self.width + x) as usize] = u8::from(!erase);
                    if !erase {
                        self.include_pixel(x, y);
                    }
                }
            }
        }
        if erase {
            self.cached_bounds = self.scan_bounds();
        }
    }

    pub fn from_rect(rect: PixelRect, width: u32, height: u32) -> Self {
        let mut mask = vec![0u8; width as usize * height as usize];
        if !rect.is_empty() {
            for y in rect.min_y..rect.max_y {
                for x in rect.min_x..rect.max_x {
                    mask[(y * width + x) as usize] = 1;
                }
            }
        }
        Self {
            width,
            height,
            mask,
            cached_bounds: (!rect.is_empty()).then_some(rect),
        }
    }

    pub fn all(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            mask: vec![1u8; width as usize * height as usize],
            cached_bounds: (width > 0 && height > 0).then_some(PixelRect {
                min_x: 0,
                min_y: 0,
                max_x: width,
                max_y: height,
            }),
        }
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.mask[(y * self.width + x) as usize] != 0
    }

    pub fn bounds(&self) -> Option<PixelRect> {
        self.cached_bounds
    }

    pub fn is_empty(&self) -> bool {
        self.cached_bounds.is_none()
    }

    pub fn invert(&mut self) {
        for value in &mut self.mask {
            *value = u8::from(*value == 0);
        }
        self.cached_bounds = self.scan_bounds();
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
        match (self.cached_bounds, other.cached_bounds) {
            (None, bounds) => self.cached_bounds = bounds,
            (Some(a), Some(b)) => {
                self.cached_bounds = Some(PixelRect {
                    min_x: a.min_x.min(b.min_x),
                    min_y: a.min_y.min(b.min_y),
                    max_x: a.max_x.max(b.max_x),
                    max_y: a.max_y.max(b.max_y),
                });
            }
            (Some(_), None) => {}
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
        self.cached_bounds = self.scan_bounds();
    }

    fn include_pixel(&mut self, x: u32, y: u32) {
        self.cached_bounds = Some(match self.cached_bounds {
            None => PixelRect {
                min_x: x,
                min_y: y,
                max_x: x + 1,
                max_y: y + 1,
            },
            Some(bounds) => PixelRect {
                min_x: bounds.min_x.min(x),
                min_y: bounds.min_y.min(y),
                max_x: bounds.max_x.max(x + 1),
                max_y: bounds.max_y.max(y + 1),
            },
        });
    }

    fn scan_bounds(&self) -> Option<PixelRect> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_points_intersect_document_and_empty_when_outside() {
        let inside = PixelRect::from_signed_points((-4, -2), (3, 5), 10, 8);
        assert_eq!(
            inside,
            PixelRect {
                min_x: 0,
                min_y: 0,
                max_x: 4,
                max_y: 6,
            }
        );
        let clipped = PixelRect::from_signed_points((7, 2), (20, 20), 10, 8);
        assert_eq!(
            clipped,
            PixelRect {
                min_x: 7,
                min_y: 2,
                max_x: 10,
                max_y: 8,
            }
        );
        assert!(PixelRect::from_signed_points((-8, -8), (-1, -1), 10, 8).is_empty());
        assert!(PixelRect::from_signed_points((12, 0), (15, 3), 10, 8).is_empty());
        assert_eq!(
            PixelRect::from_points((1, 2), (4, 5), 10, 8),
            PixelRect::from_signed_points((1, 2), (4, 5), 10, 8)
        );
    }

    #[test]
    fn lasso_fills_concave_polygon_and_clips_outside_canvas() {
        let points = [
            (-2.0, -2.0),
            (4.0, -2.0),
            (4.0, 1.0),
            (1.0, 1.0),
            (1.0, 4.0),
            (-2.0, 4.0),
        ]
        .map(|(x, y)| glam::Vec2::new(x, y));
        let selection = Selection::from_polygon(&points, 5, 5);
        assert!(selection.contains(0, 3));
        assert!(selection.contains(3, 0));
        assert!(!selection.contains(2, 2));
        assert!(!selection.contains(4, 0));
        let reversed: Vec<_> = points.into_iter().rev().collect();
        assert_eq!(
            selection.mask,
            Selection::from_polygon(&reversed, 5, 5).mask
        );
        assert!(Selection::from_polygon(&points[..2], 5, 5).is_empty());
    }

    #[test]
    fn selection_brush_has_continuous_coverage_adds_and_erases_locally() {
        let mut selection = Selection::from_polygon(&[], 12, 8);
        selection.paint_segment(
            glam::Vec2::new(-2.0, 2.5),
            glam::Vec2::new(10.5, 2.5),
            3.0,
            false,
        );
        for x in 0..11 {
            assert!(selection.contains(x, 2));
        }
        selection.paint_segment(
            glam::Vec2::new(6.5, 2.5),
            glam::Vec2::new(6.5, 6.5),
            1.0,
            false,
        );
        selection.paint_segment(
            glam::Vec2::new(3.5, 2.5),
            glam::Vec2::new(3.5, 2.5),
            1.0,
            true,
        );
        assert!(!selection.contains(3, 2));
        assert!(selection.contains(2, 2));
        assert!(selection.contains(6, 6));
        assert!(!selection.contains(11, 7));
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
        assert_eq!(
            a.bounds(),
            Some(PixelRect {
                min_x: 0,
                min_y: 0,
                max_x: 2,
                max_y: 2,
            })
        );
    }
}
