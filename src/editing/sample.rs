use crate::raster::Raster;

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
    lerp_premul(lerp_premul(p00, p10, tx), lerp_premul(p01, p11, tx), ty)
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
#[allow(dead_code)]
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

fn area_sample_rgba(src: &[u8], sw: u32, sh: u32, x0: f64, y0: f64, x1: f64, y1: f64) -> [u8; 4] {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
