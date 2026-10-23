use crate::raster::{Raster, SparsePixels};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use image::{DynamicImage, ImageFormat, RgbaImage};

#[derive(Debug, Clone)]
pub struct Layer {
    pub z: f32,
    /// Held drawings keyed by their first frame. `pixels` is the editable current drawing.
    pub drawings: BTreeMap<u32, SparsePixels>,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub pixels: SparsePixels,
}

#[derive(Debug, Clone, Default)]
pub struct Project {
    pub animation: Option<Animation>,
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
    pub active_layer: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Animation {
    #[serde(default)]
    pub camera: crate::camera::CameraTrack,
    pub fps: u32,
    pub length: u32,
    pub frame: u32,
    /// Inclusive playback start. Older projects omit it and begin at the first frame.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub play_start: u32,
    /// Inclusive playback end. `u32::MAX` follows the last frame until the user sets it.
    #[serde(default = "playback_follows_end", skip_serializing_if = "playback_is_open")]
    pub play_end: u32,
}

fn shift_keys<T>(keys: &mut BTreeMap<u32, T>, at: u32, delta: i32) {
    let existing = std::mem::take(keys);
    *keys = existing.into_iter().map(|(key, value)| {
        let shifted = if key >= at { (i64::from(key) + i64::from(delta)) as u32 } else { key };
        (shifted, value)
    }).collect();
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

fn playback_follows_end() -> u32 {
    u32::MAX
}

fn playback_is_open(value: &u32) -> bool {
    *value == u32::MAX
}

/// Undo snapshot for a duration change on one timeline track.
#[derive(Clone)]
pub(crate) struct TimelineShift {
    pub frame_before: u32,
    pub frame_after: u32,
    pub length_before: u32,
    pub length_after: u32,
    pub play_before: (u32, u32),
    pub play_after: (u32, u32),
    pub camera_before: crate::camera::CameraTrack,
    pub camera_after: crate::camera::CameraTrack,
    pub before: Vec<BTreeMap<u32, SparsePixels>>,
    pub after: Vec<BTreeMap<u32, SparsePixels>>,
}

impl Animation {
    pub fn new(fps: u32, length: u32) -> Self {
        Self {
            camera: Default::default(),
            fps,
            length,
            frame: 0,
            play_start: 0,
            play_end: u32::MAX,
        }
    }

    /// Inclusive frame range used by Play. An unset end stays on the last frame.
    pub fn playback_range(&self) -> (u32, u32) {
        let last = self.length.saturating_sub(1);
        let end = if self.play_end >= self.length {
            last
        } else {
            self.play_end
        };
        (self.play_start.min(end), end)
    }

    pub fn clamp_playback(&mut self) {
        if self.length == 0 {
            return;
        }
        let last = self.length - 1;
        if self.play_end != u32::MAX {
            self.play_end = self.play_end.min(last);
            self.play_start = self.play_start.min(self.play_end);
        } else {
            self.play_start = self.play_start.min(last);
        }
    }
}

impl Project {
    pub fn flush_drawings(&mut self) {
        let Some(animation) = &self.animation else {
            return;
        };
        for layer in &mut self.layers {
            let start = layer
                .drawings
                .range(..=animation.frame)
                .next_back()
                .map(|(&start, _)| start)
                .unwrap_or(0);
            layer.drawings.insert(start, layer.pixels.clone());
        }
    }

    pub fn seek(&mut self, frame: u32) {
        self.flush_drawings();
        let Some(animation) = &mut self.animation else {
            return;
        };
        animation.frame = frame.min(animation.length - 1);
        for layer in &mut self.layers {
            layer.pixels = layer
                .drawings
                .range(..=animation.frame)
                .next_back()
                .map(|(_, pixels)| pixels.clone())
                .unwrap_or_default();
        }
    }

    pub fn new_drawing(&mut self, duplicate: bool) {
        self.flush_drawings();
        let Some(animation) = &self.animation else {
            return;
        };
        let Some(layer) = self.layers.get_mut(self.active_layer) else {
            return;
        };
        if !duplicate {
            layer.pixels = SparsePixels::default();
        }
        layer.drawings.insert(animation.frame, layer.pixels.clone());
    }

    pub fn delete_drawing(&mut self) {
        self.flush_drawings();
        let Some(animation) = &self.animation else {
            return;
        };
        let Some(layer) = self.layers.get_mut(self.active_layer) else {
            return;
        };
        // The initial drawing anchors the track; clearing it uses New blank drawing.
        if animation.frame == 0 {
            return;
        }
        layer.drawings.remove(&animation.frame);
        layer.pixels = layer
            .drawings
            .range(..=animation.frame)
            .next_back()
            .map(|(_, pixels)| pixels.clone())
            .unwrap_or_default();
    }

    /// Frames the active layer's current drawing holds, including its key.
    pub fn held_length(&self) -> u32 {
        let Some((start, end)) = self.held_span() else {
            return 1;
        };
        end.saturating_sub(start).max(1)
    }

    fn held_span(&self) -> Option<(u32, u32)> {
        let animation = self.animation.as_ref()?;
        let layer = self.layers.get(self.active_layer)?;
        let start = layer
            .drawings
            .range(..=animation.frame)
            .next_back()
            .map(|(&key, _)| key)
            .unwrap_or(0);
        let end = layer
            .drawings
            .range((start + 1)..)
            .next()
            .map(|(&key, _)| key)
            .unwrap_or(animation.length);
        Some((start, end))
    }

    /// Grow or shrink the active drawing; only later keys on this layer move.
    pub(crate) fn resize_held_drawing(&mut self, longer: bool) -> Option<TimelineShift> {
        let span = self.held_span()?;
        self.resize_held_span(longer, span, Some(self.active_layer))
    }

    fn camera_held_span(&self) -> Option<(u32, u32)> {
        let animation = self.animation.as_ref()?;
        let start = animation.camera.keys.range(..=animation.frame).next_back().map_or(0, |(&f, _)| f);
        let end = animation.camera.keys.range((start + 1)..).next().map_or(animation.length, |(&f, _)| f);
        Some((start, end))
    }

    pub fn camera_held_length(&self) -> u32 {
        self.camera_held_span().map_or(1, |(start, end)| end.saturating_sub(start).max(1))
    }

    pub(crate) fn resize_held_camera(&mut self, longer: bool) -> Option<TimelineShift> {
        let span = self.camera_held_span()?;
        self.resize_held_span(longer, span, None)
    }

    fn resize_held_span(&mut self, longer: bool, (_start, end): (u32, u32), layer: Option<usize>) -> Option<TimelineShift> {
        let animation = self.animation.clone()?;
        self.flush_drawings();
        let duration = end.saturating_sub(_start);
        if longer {
            if animation.length >= 1200 {
                return None;
            }
        } else if duration <= 1 {
            return None;
        }
        let at = end;
        let delta = if longer { 1 } else { -1 };
        let frame_after = if longer {
            if animation.frame >= at {
                animation.frame + 1
            } else {
                animation.frame
            }
        } else if animation.frame + 1 == end {
            animation.frame.saturating_sub(1)
        } else if animation.frame >= end {
            animation.frame - 1
        } else {
            animation.frame
        };
        let length_after = if longer {
            animation.length + 1
        } else {
            animation.length - 1
        };
        let play_before = (animation.play_start, animation.play_end);
        let frame_before = animation.frame;
        let length_before = animation.length;
        let camera_before = self.animation.as_ref()?.camera.clone();
        let before = self.drawing_maps();
        if let Some(index) = layer {
            shift_keys(&mut self.layers.get_mut(index)?.drawings, at, delta);
        } else {
            shift_keys(&mut self.animation.as_mut()?.camera.keys, at, delta);
        }
        // Shortening this track must never truncate an untouched track's last key.
        let last_key = self.layers.iter().flat_map(|l| l.drawings.keys())
            .chain(self.animation.as_ref()?.camera.keys.keys()).copied().max().unwrap_or(0);
        let length_after = length_after.max(last_key + 1);
        let animation = self.animation.as_mut()?;
        animation.length = length_after;
        animation.frame = frame_after;
        animation.clamp_playback();
        let play_after = (animation.play_start, animation.play_end);
        self.load_drawing_pixels();
        Some(TimelineShift {
            camera_before,
            camera_after: self.animation.as_ref()?.camera.clone(),
            frame_before,
            frame_after,
            length_before,
            length_after,
            play_before,
            play_after,
            before,
            after: self.drawing_maps(),
        })
    }

    pub(crate) fn restore_timeline(
        &mut self,
        drawings: &[BTreeMap<u32, SparsePixels>],
        length: u32,
        frame: u32,
        play: (u32, u32),
    ) -> Option<()> {
        if self.animation.is_none()
            || drawings.len() != self.layers.len()
            || length == 0
            || frame >= length
        {
            return None;
        }
        for (layer, drawings) in self.layers.iter_mut().zip(drawings) {
            layer.drawings = drawings.clone();
        }
        let animation = self.animation.as_mut()?;
        animation.length = length;
        animation.frame = frame;
        animation.play_start = play.0;
        animation.play_end = play.1;
        self.load_drawing_pixels();
        Some(())
    }

    fn drawing_maps(&self) -> Vec<BTreeMap<u32, SparsePixels>> {
        self.layers
            .iter()
            .map(|layer| layer.drawings.clone())
            .collect()
    }

    fn load_drawing_pixels(&mut self) {
        let Some(frame) = self.animation.as_ref().map(|animation| animation.frame) else {
            return;
        };
        for layer in &mut self.layers {
            layer.pixels = layer
                .drawings
                .range(..=frame)
                .next_back()
                .map(|(_, pixels)| pixels.clone())
                .unwrap_or_default();
        }
    }

    pub fn import_png(&mut self, path: &Path) -> Result<(), String> {
        let bytes = fs::read(path).map_err(|e| format!("Cannot read image: {e}"))?;
        let decoded = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
            .map_err(|e| format!("Invalid PNG: {e}"))?;
        self.set_image(decoded)
    }

    pub fn import_rgba(&mut self, pixels: &[u8], width: u32, height: u32) -> Result<(), String> {
        let image = RgbaImage::from_raw(width, height, pixels.to_vec())
            .ok_or_else(|| "Invalid clipboard image size".to_string())?;
        self.set_image(DynamicImage::ImageRgba8(image))
    }

    fn set_image(&mut self, image: DynamicImage) -> Result<(), String> {
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        self.width = width;
        self.height = height;
        self.layers = vec![Layer {
            z: 0.0,
            drawings: Default::default(),
            name: "Layer 1".into(),
            visible: true,
            opacity: 1.0,
            pixels: SparsePixels::from_rgba(rgba.as_raw(), width, height),
        }];
        self.active_layer = 0;
        Ok(())
    }

    pub fn add_layer(&mut self) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let index = self.layers.len() + 1;
        self.layers.push(Layer {
            z: 0.0,
            drawings: Default::default(),
            name: format!("Layer {index}"),
            visible: true,
            opacity: 1.0,
            pixels: SparsePixels::default(),
        });
        self.active_layer = self.layers.len() - 1;
    }

    /// Insert into the existing canvas. Fit uses the original input bounds;
    /// native-size insertion preserves the complete image beyond the canvas.
    pub fn add_raster_layer(
        &mut self,
        image: &image::RgbaImage,
        name: String,
        region: impl Into<crate::raster::RasterRect>,
        fit: bool,
    ) -> Result<(), String> {
        let region = region.into();
        if image.width() == 0 || image.height() == 0 {
            return Err("Result image is empty".into());
        }
        if region.is_empty() {
            return Err("Result destination is empty".into());
        }
        let resized;
        let image = if fit {
            let size = region.size();
            if image.dimensions() != size {
                resized = image::imageops::resize(
                    image,
                    size.0,
                    size.1,
                    image::imageops::FilterType::Lanczos3,
                );
                &resized
            } else {
                image
            }
        } else {
            image
        };
        let (w, h) = image.dimensions();
        self.add_layer();
        let target = &mut self.layers[self.active_layer].pixels;
        *target = SparsePixels::from_rgba(image.as_raw(), w, h);
        target.offset = (region.min_x, region.min_y);
        self.layers[self.active_layer].name = name;
        Ok(())
    }

    /// Replace active-layer pixels with the result. Source alpha overwrites the destination,
    /// including fully transparent pixels.
    pub fn composite_onto_active_layer(
        &mut self,
        image: &image::RgbaImage,
        region: impl Into<crate::raster::RasterRect>,
        fit: bool,
    ) -> Result<(), String> {
        let region = region.into();
        if self.layers.is_empty() {
            return Err("Choose a layer first".into());
        }
        if image.width() == 0 || image.height() == 0 {
            return Err("Result image is empty".into());
        }
        if region.is_empty() {
            return Err("Result destination is empty".into());
        }
        let resized;
        let image = if fit {
            let size = region.size();
            if image.dimensions() != size {
                resized = image::imageops::resize(
                    image,
                    size.0,
                    size.1,
                    image::imageops::FilterType::Lanczos3,
                );
                &resized
            } else {
                image
            }
        } else {
            image
        };
        let target = &mut self.layers[self.active_layer].pixels;
        for (y, row) in image.rows().enumerate() {
            for (x, pixel) in row.enumerate() {
                let dx = region.min_x + x as i32;
                let dy = region.min_y + y as i32;
                target.write(0, dx, dy, pixel.0);
            }
        }
        target.prune();
        Ok(())
    }

    /// Resize the document frame without changing any layer pixels.
    pub fn resize_canvas(&mut self, width: u32, height: u32) -> Result<(), String> {
        let width = width.max(1);
        let height = height.max(1);
        if self.layers.is_empty() {
            self.width = width;
            self.height = height;
            self.layers.push(Layer {
                z: 0.0,
                drawings: Default::default(),
                name: "Layer 1".into(),
                visible: true,
                opacity: 1.0,
                pixels: SparsePixels::default(),
            });
            self.active_layer = 0;
            return Ok(());
        }
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Bounds of visible content inside the current canvas, independent of zoom
    /// and selection. Normal alpha-over compositing cannot cancel source alpha.
    pub fn visible_content_bounds(&self) -> Option<crate::raster::RasterRect> {
        use crate::raster::RasterRect;
        let frame = RasterRect {
            min_x: 0,
            min_y: 0,
            max_x: self.width as i32,
            max_y: self.height as i32,
        };
        let mut bounds: Option<RasterRect> = None;
        for layer in &self.layers {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            let Some(b) = layer.pixels.bounds_in(frame) else {
                continue;
            };
            bounds = Some(match bounds {
                Some(a) => RasterRect {
                    min_x: a.min_x.min(b.min_x),
                    min_y: a.min_y.min(b.min_y),
                    max_x: a.max_x.max(b.max_x),
                    max_y: a.max_y.max(b.max_y),
                },
                None => b,
            });
        }
        bounds
    }

    pub fn delete_layer(&mut self, index: usize) -> Option<Layer> {
        if index >= self.layers.len() {
            return None;
        }
        let layer = self.layers.remove(index);
        if self.layers.is_empty() {
            self.width = 0;
            self.height = 0;
            self.active_layer = 0;
        } else if index < self.active_layer {
            self.active_layer -= 1;
        } else {
            self.active_layer = self.active_layer.min(self.layers.len() - 1);
        }
        Some(layer)
    }

    pub fn move_layer_to(&mut self, from: usize, to: usize) {
        let len = self.layers.len();
        if from >= len || to >= len || from == to {
            return;
        }
        let layer = self.layers.remove(from);
        self.layers.insert(to, layer);
        if self.active_layer == from {
            self.active_layer = to;
        } else if from < self.active_layer && to >= self.active_layer {
            self.active_layer -= 1;
        } else if from > self.active_layer && to <= self.active_layer {
            self.active_layer += 1;
        }
    }

    pub fn active_pixels(&self) -> Option<&SparsePixels> {
        self.layers
            .get(self.active_layer)
            .map(|layer| &layer.pixels)
    }

    pub fn active_pixels_mut(&mut self) -> Option<&mut SparsePixels> {
        self.layers
            .get_mut(self.active_layer)
            .map(|layer| &mut layer.pixels)
    }

    pub fn decoded_rgba(&self) -> Result<Option<(Vec<u8>, u32, u32)>, String> {
        if self.layers.is_empty() || self.width == 0 || self.height == 0 {
            return Ok(None);
        }
        Ok(Some((
            crate::editing::composite_layers(&self.layers, self.width, self.height),
            self.width,
            self.height,
        )))
    }

    pub fn export_png(&self, path: &Path) -> Result<(), String> {
        self.export_image(path, ImageFormat::Png)
    }

    pub fn export_image(&self, path: &Path, format: ImageFormat) -> Result<(), String> {
        let (mut pixels, width, height) = self
            .decoded_rgba()?
            .ok_or_else(|| "Document is empty".to_string())?;
        if let Some(animation) = &self.animation {
            if animation.camera.visible {
                pixels = crate::camera::render_frame(self, animation.camera.sample(animation.frame));
            }
        }
        let image = RgbaImage::from_raw(width, height, pixels).ok_or("Invalid document raster")?;
        DynamicImage::ImageRgba8(image)
            .save_with_format(path, format)
            .map_err(|e| format!("Cannot export image: {e}"))
    }
}

pub fn encode_png_rgba(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let image = RgbaImage::from_raw(width, height, pixels.to_vec())
        .ok_or_else(|| "Invalid layer size".to_string())?;
    encode_png(&DynamicImage::ImageRgba8(image))
}

/// VP8L lossless WebP. Partial alpha is preserved; this is not the lossy encoder.
pub fn encode_webp_rgba(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| "Invalid image size".to_string())?;
    if pixels.len() != expected || width == 0 || height == 0 {
        return Err("Invalid image size".into());
    }
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::codecs::webp::WebPEncoder::new_lossless(&mut cursor)
        .encode(pixels, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("Cannot encode WebP: {e}"))?;
    Ok(cursor.into_inner())
}

fn encode_png(image: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| format!("Cannot encode PNG: {e}"))?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn animation_holds_drawings_independently_per_layer_and_copies_on_edit() {
        use crate::raster::Raster;
        let mut project = Project::default();
        project.resize_canvas(8, 8).unwrap();
        project.animation = Some(Animation::new(12, 24));
        project.layers[0].pixels.write(0, 0, 0, [10, 0, 0, 255]);
        project.add_layer();
        project.layers[1].pixels.write(0, 1, 0, [20, 0, 0, 255]);
        project.seek(6);
        project.new_drawing(true);
        project.layers[1].pixels.write(0, 1, 0, [30, 0, 0, 255]);
        project.seek(10);
        assert_eq!(project.layers[1].pixels.read(0, 1, 0)[0], 30);
        assert_eq!(project.layers[0].pixels.read(0, 0, 0)[0], 10);
        project.seek(0);
        assert_eq!(project.layers[1].pixels.read(0, 1, 0)[0], 20);
        project.seek(9);
        project.new_drawing(false);
        assert_eq!(project.layers[1].pixels.read(0, 1, 0), [0; 4]);
        project.seek(6);
        project.delete_drawing();
        assert_eq!(project.layers[1].pixels.read(0, 1, 0)[0], 20);
        project.seek(9);
        assert_eq!(project.layers[1].pixels.read(0, 1, 0), [0; 4]);
    }

    #[test]
    fn trim_canvas_bounds_combine_visible_layers_and_clip_partial_off_canvas_content() {
        use crate::raster::{Raster, RasterRect};
        let mut project = Project::default();
        project.resize_canvas(20, 15).unwrap();
        project.layers[0].pixels =
            SparsePixels::from_rgba(&[100, 120, 140, 255].repeat(8 * 8), 8, 8);
        project.layers[0].pixels.offset = (-6, 5);
        project.add_layer();
        project.layers[1].opacity = 0.5;
        project.layers[1].pixels.write(0, 19, 14, [255, 0, 0, 1]);
        project.add_layer();
        project.layers[2].visible = false;
        project.layers[2].pixels.write(0, 0, 0, [255, 0, 0, 255]);
        assert_eq!(
            project.visible_content_bounds(),
            Some(RasterRect {
                min_x: 0,
                min_y: 5,
                max_x: 20,
                max_y: 15
            })
        );
        project.layers[1].pixels.offset = (1, 0);
        assert_eq!(
            project.visible_content_bounds(),
            Some(RasterRect {
                min_x: 0,
                min_y: 5,
                max_x: 2,
                max_y: 13
            })
        );
        project.layers[0].visible = false;
        assert!(project.visible_content_bounds().is_none());
    }

    #[test]
    fn webp_export_and_import_preserve_rgba() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("image.webp");
        let pixels = vec![10, 20, 30, 128, 40, 50, 60, 255];
        let mut project = Project::default();
        project.import_rgba(&pixels, 2, 1).unwrap();
        project.export_image(&path, ImageFormat::WebP).unwrap();
        let mut workspace = crate::workspace::Workspace::default();
        let index = workspace.import_asset_image(&path).unwrap();
        assert_eq!(workspace.assets[index].pixels, pixels);
        let decoded = crate::comfyui::decode_image(&fs::read(path).unwrap()).unwrap();
        assert_eq!(decoded.into_raw(), pixels);
    }

    #[test]
    fn native_result_preserves_pixels_without_enlarging_canvas() {
        let mut project = Project::default();
        let original = vec![10, 20, 30, 255].repeat(4);
        project.import_rgba(&original, 2, 2).unwrap();
        let result = RgbaImage::from_fn(3, 3, |x, y| Rgba([x as u8, y as u8, 60, 255]));
        let region = crate::editing::PixelRect {
            min_x: 0,
            min_y: 0,
            max_x: 2,
            max_y: 2,
        };
        project
            .add_raster_layer(&result, "AI Edit".into(), region, false)
            .unwrap();
        assert_eq!((project.width, project.height), (2, 2));
        assert_eq!(
            project.layers[0]
                .pixels
                .to_rgba(project.width, project.height),
            original
        );
        assert_eq!(
            project.layers[1]
                .pixels
                .to_rgba(project.width, project.height),
            [0, 0, 60, 255, 1, 0, 60, 255, 0, 1, 60, 255, 1, 1, 60, 255]
        );
        assert_eq!(project.layers[1].pixels.region(2, 2, 1, 1), [2, 2, 60, 255]);
        project
            .add_raster_layer(
                &RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])),
                "small".into(),
                region,
                false,
            )
            .unwrap();
        assert_eq!(
            &project.layers[2]
                .pixels
                .to_rgba(project.width, project.height)[..4],
            &[1, 2, 3, 255]
        );
        assert!(
            project.layers[2]
                .pixels
                .to_rgba(project.width, project.height)[4..]
                .iter()
                .all(|b| *b == 0)
        );
    }

    #[test]
    fn fit_result_matches_original_bounds_and_position() {
        for side in [1, 4] {
            let mut project = Project::default();
            project.import_rgba(&vec![0; 5 * 4 * 4], 5, 4).unwrap();
            let region = crate::editing::PixelRect {
                min_x: 2,
                min_y: 1,
                max_x: 4,
                max_y: 3,
            };
            let result = RgbaImage::from_pixel(side, side, Rgba([40, 50, 60, 128]));
            project
                .add_raster_layer(&result, "Fit".into(), region, true)
                .unwrap();
            assert_eq!((project.width, project.height), (5, 4));
            let actual = RgbaImage::from_raw(
                5,
                4,
                project.layers[1]
                    .pixels
                    .to_rgba(project.width, project.height)
                    .clone(),
            )
            .unwrap();
            for (x, y, pixel) in actual.enumerate_pixels() {
                assert_eq!(
                    *pixel,
                    if (2..4).contains(&x) && (1..3).contains(&y) {
                        Rgba([40, 50, 60, 128])
                    } else {
                        Rgba([0; 4])
                    }
                );
            }
            assert!(
                project.layers[0]
                    .pixels
                    .to_rgba(project.width, project.height)
                    .iter()
                    .all(|b| *b == 0)
            );
        }
    }

    fn sample_png() -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 3, Rgba([10, 20, 30, 255])));
        encode_png(&image).unwrap()
    }

    #[test]
    fn import_rgba_keeps_pixels() {
        let mut project = Project::default();
        let pixels = vec![10, 20, 30, 255, 40, 50, 60, 255];
        project.import_rgba(&pixels, 2, 1).unwrap();
        let (decoded, width, height) = project.decoded_rgba().unwrap().unwrap();
        assert_eq!((width, height), (2, 1));
        assert_eq!(&decoded[..8], &pixels[..]);
    }

    #[test]
    fn export_does_not_modify_import_source() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.png");
        let output = temp.path().join("output.png");
        let original = sample_png();
        fs::write(&source, &original).unwrap();

        let mut project = Project::default();
        project.import_png(&source).unwrap();
        project.export_png(&output).unwrap();

        assert_eq!(fs::read(source).unwrap(), original);
        assert!(image::open(output).is_ok());
    }

    #[test]
    fn add_and_reorder_layers() {
        let mut project = Project::default();
        project
            .import_rgba(&[255, 0, 0, 255, 255, 0, 0, 255], 2, 1)
            .unwrap();
        project.add_layer();
        assert_eq!(project.layers.len(), 2);
        assert_eq!(project.active_layer, 1);
        project.move_layer_to(1, 0);
        assert_eq!(project.active_layer, 0);
        project.delete_layer(project.active_layer);
        assert_eq!(project.layers.len(), 1);
    }
}
