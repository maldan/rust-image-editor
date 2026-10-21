use super::brush::{erase_pixel, paint_color, stroke_color};
use super::sample::{color_distance, sample_pixel};
use super::selection::{PixelRect, Selection};
use crate::raster::Raster;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
