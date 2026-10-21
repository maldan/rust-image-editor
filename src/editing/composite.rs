use crate::raster::Raster;

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
    use crate::raster::Raster;
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

pub(super) fn over(dst: &mut [u8], src: &[u8], opacity: f32) {
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
