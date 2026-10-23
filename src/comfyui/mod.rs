mod video;
pub use video::VideoSettings;
mod catalog;
mod client;
mod config;
mod job;
mod workflow;
mod ws;

pub use catalog::{Catalog, CatalogLoad};
pub use config::HostConfig;
pub use job::{Job, JobResult};
pub use workflow::ControlNetSettings;
pub use workflow::DrawSettings;
pub use workflow::UpscaleSettings;
pub use workflow::validate_lineart_resolution;

/// Shared by input size feedback and worker-side resizing.
pub fn scaled_size(width: u32, height: u32, scale: f32) -> Result<(u32, u32), String> {
    if width == 0 || height == 0 || !scale.is_finite() || scale <= 0.0 {
        return Err("Choose an image and a positive, finite scale".into());
    }
    let w = (width as f64 * scale as f64).round().max(1.0);
    let h = (height as f64 * scale as f64).round().max(1.0);
    // Bound RGBA allocation to 256 MiB, including outputs decoded from the network.
    if w > 16384.0 || h > 16384.0 || w * h > 67_108_864.0 {
        return Err("Input exceeds 16384 px per side or 64 megapixels; reduce scale".into());
    }
    Ok((w as u32, h as u32))
}

pub fn decode_image(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("Invalid image: {e}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map(|i| i.to_rgba8())
        .map_err(|e| format!("Cannot decode image: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_scale_rounds_and_rejects_invalid_allocations() {
        assert_eq!(scaled_size(3, 5, 0.5).unwrap(), (2, 3));
        assert_eq!(scaled_size(1, 1, 0.01).unwrap(), (1, 1));
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 100000.0] {
            assert!(scaled_size(512, 512, scale).is_err());
        }
    }
}
