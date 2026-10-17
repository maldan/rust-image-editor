use std::fs;
use crate::raster::SparsePixels;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageFormat, RgbaImage};
use mega_file::{MegaFile, WriteFile};
use serde::{Deserialize, Serialize};

const MANIFEST_PATH: &str = "/manifest.json";
const IMAGE_PATH: &str = "/assets/main/image.png";
const MANIFEST_KIND: &str = "mega-image-editor/manifest+json";
const IMAGE_KIND: &str = "image/png";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub document_type: String,
    #[serde(default)]
    pub asset_path: Option<String>,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub active_layer: usize,
    #[serde(default)]
    pub layers: Vec<LayerMeta>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            format: "mega-image-editor".into(),
            version: 4,
            document_type: "image".into(),
            asset_path: None,
            width: 0,
            height: 0,
            active_layer: 0,
            layers: Vec::new(),
        }
    }
}

fn default_opacity() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayerMeta {
    pub name: String,
    pub visible: bool,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub pixels: SparsePixels,
}

#[derive(Debug, Clone, Default)]
pub struct Project {
    pub manifest: Manifest,
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
    pub active_layer: usize,
    pub path: Option<PathBuf>,
}

impl Project {
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
            name: "Layer 1".into(),
            visible: true,
            opacity: 1.0,
            pixels: SparsePixels::from_rgba(rgba.as_raw(), width, height),
        }];
        self.active_layer = 0;
        self.sync_manifest();
        Ok(())
    }

    pub fn add_layer(&mut self) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let index = self.layers.len() + 1;
        self.layers.push(Layer {
            name: format!("Layer {index}"),
            visible: true,
            opacity: 1.0,
            pixels: SparsePixels::default(),
        });
        self.active_layer = self.layers.len() - 1;
        self.sync_manifest();
    }

    /// Insert into the existing canvas. Fit uses the original input bounds;
    /// native-size insertion preserves the complete image beyond the canvas.
    pub fn add_raster_layer(&mut self, image: &image::RgbaImage, name: String, region: impl Into<crate::raster::RasterRect>, fit: bool) -> Result<(), String> {
        let region = region.into();
        if image.width() == 0 || image.height() == 0 { return Err("Result image is empty".into()); }
        if region.is_empty() {
            return Err("Result destination is empty".into());
        }
        let resized;
        let image = if fit {
            let size = region.size();
            if image.dimensions() != size {
                resized = image::imageops::resize(image, size.0, size.1, image::imageops::FilterType::Lanczos3);
                &resized
            } else { image }
        } else { image };
        let (w, h) = image.dimensions();
        self.add_layer();
        let target = &mut self.layers[self.active_layer].pixels;
        *target = SparsePixels::from_rgba(image.as_raw(), w, h);
        target.offset = (region.min_x, region.min_y);
        self.layers[self.active_layer].name = name;
        self.sync_manifest();
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
                name: "Layer 1".into(),
                visible: true,
                opacity: 1.0,
                pixels: SparsePixels::default(),
            });
            self.active_layer = 0;
            self.sync_manifest();
            return Ok(());
        }
        self.width = width;
        self.height = height;
        self.sync_manifest();
        Ok(())
    }

    /// Bounds of visible content inside the current canvas, independent of zoom
    /// and selection. Normal alpha-over compositing cannot cancel source alpha.
    pub fn visible_content_bounds(&self) -> Option<crate::raster::RasterRect> {
        use crate::raster::RasterRect;
        let frame = RasterRect {min_x:0,min_y:0,max_x:self.width as i32,max_y:self.height as i32};
        let mut bounds: Option<RasterRect> = None;
        for layer in &self.layers {
            if !layer.visible || layer.opacity <= 0.0 { continue; }
            let Some(b) = layer.pixels.bounds_in(frame) else { continue; };
            bounds = Some(match bounds {
                Some(a) => RasterRect {min_x:a.min_x.min(b.min_x),min_y:a.min_y.min(b.min_y),
                    max_x:a.max_x.max(b.max_x),max_y:a.max_y.max(b.max_y)},
                None => b,
            });
        }
        bounds
    }

    pub fn delete_layer(&mut self, index: usize) -> Option<Layer> {
        if index >= self.layers.len() { return None; }
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
        self.sync_manifest();
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
        self.sync_manifest();
    }

    pub fn active_pixels(&self) -> Option<&SparsePixels> {
        self.layers.get(self.active_layer).map(|layer| &layer.pixels)
    }

    pub fn active_pixels_mut(&mut self) -> Option<&mut SparsePixels> {
        self.layers
            .get_mut(self.active_layer)
            .map(|layer| &mut layer.pixels)
    }

    pub fn sync_manifest_pub(&mut self) {
        self.sync_manifest();
    }

    fn sync_manifest(&mut self) {
        self.manifest.version = 4;
        self.manifest.width = self.width;
        self.manifest.height = self.height;
        self.manifest.active_layer = self.active_layer;
        self.manifest.layers = self
            .layers
            .iter()
            .enumerate()
            .map(|(index, layer)| LayerMeta {
                name: layer.name.clone(),
                visible: layer.visible,
                opacity: layer.opacity.clamp(0.0, 1.0),
                path: format!("/layers/{index}.tiles"),
            })
            .collect();
        self.manifest.asset_path = if self.layers.is_empty() {
            None
        } else {
            Some(IMAGE_PATH.into())
        };
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

    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        self.sync_manifest();
        let manifest = serde_json::to_vec_pretty(&self.manifest)
            .map_err(|e| format!("Cannot encode manifest: {e}"))?;
        let mut files = vec![WriteFile::new(MANIFEST_PATH, MANIFEST_KIND, manifest)];
        if let Some((pixels, width, height)) = self.decoded_rgba()? {
            files.push(WriteFile::new(
                IMAGE_PATH,
                IMAGE_KIND,
                encode_png_rgba(&pixels, width, height)?,
            ));
        }
        for (index, layer) in self.layers.iter().enumerate() {
            files.push(WriteFile::new(
                format!("/layers/{index}.tiles"),
                "application/x-rgba-tiles-64",
                layer.pixels.encode(),
            ));
        }
        if self.path.as_deref() == Some(path) && path.is_file() {
            crate::safe_save::append_archive(path, files, None)?;
        } else {
            crate::safe_save::save_archive(path, files)?;
        }
        self.path = Some(path.to_path_buf());
        Ok(())
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let archive = MegaFile::open(path).map_err(|e| format!("Cannot open project: {e}"))?;
        let manifest_bytes = archive
            .read_all(MANIFEST_PATH)
            .map_err(|e| format!("Project has no valid manifest: {e}"))?;
        let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| format!("Invalid project manifest: {e}"))?;
        if manifest.format != "mega-image-editor" || manifest.version != 4 {
            return Err("Unsupported project format or version".into());
        }
        let mut layers = Vec::new();
        for meta in &manifest.layers {
            let bytes = archive.read_all(&meta.path).map_err(|e| format!("Layer missing: {e}"))?;
            layers.push(Layer {
                name: meta.name.clone(), visible: meta.visible, opacity: meta.opacity.clamp(0.0, 1.0),
                pixels: SparsePixels::decode(&bytes)?,
            });
        }
        let mut project = Self {
            width: manifest.width, height: manifest.height,
            active_layer: manifest.active_layer.min(layers.len().saturating_sub(1)),
            layers, manifest, path: Some(path.to_path_buf()),
        };
        project.sync_manifest();
        Ok(project)
    }

    pub fn export_png(&self, path: &Path) -> Result<(), String> {
        self.export_image(path, ImageFormat::Png)
    }

    pub fn export_image(&self, path: &Path, format: ImageFormat) -> Result<(), String> {
        let (pixels, width, height) = self
            .decoded_rgba()?
            .ok_or_else(|| "Document is empty".to_string())?;
        let image = RgbaImage::from_raw(width, height, pixels).ok_or("Invalid document raster")?;
        DynamicImage::ImageRgba8(image).save_with_format(path, format)
            .map_err(|e| format!("Cannot export image: {e}"))
    }
}

pub fn encode_png_rgba(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let image = RgbaImage::from_raw(width, height, pixels.to_vec())
        .ok_or_else(|| "Invalid layer size".to_string())?;
    encode_png(&DynamicImage::ImageRgba8(image))
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
    fn trim_canvas_bounds_combine_visible_layers_and_clip_partial_off_canvas_content() {
        use crate::raster::{Raster,RasterRect};
        let mut project = Project::default();
        project.resize_canvas(20,15).unwrap();
        project.layers[0].pixels = SparsePixels::from_rgba(&[100,120,140,255].repeat(8*8),8,8);
        project.layers[0].pixels.offset = (-6,5);
        project.add_layer();
        project.layers[1].opacity = 0.5;
        project.layers[1].pixels.write(0,19,14,[255,0,0,1]);
        project.add_layer();
        project.layers[2].visible = false;
        project.layers[2].pixels.write(0,0,0,[255,0,0,255]);
        assert_eq!(project.visible_content_bounds(),Some(RasterRect {min_x:0,min_y:5,max_x:20,max_y:15}));
        project.layers[1].pixels.offset = (1,0);
        assert_eq!(project.visible_content_bounds(),Some(RasterRect {min_x:0,min_y:5,max_x:2,max_y:13}));
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
        let region = crate::editing::PixelRect { min_x: 0, min_y: 0, max_x: 2, max_y: 2 };
        project.add_raster_layer(&result, "AI Edit".into(), region, false).unwrap();
        assert_eq!((project.width, project.height), (2, 2));
        assert_eq!(project.layers[0].pixels.to_rgba(project.width, project.height), original);
        assert_eq!(project.layers[1].pixels.to_rgba(project.width, project.height), [0, 0, 60, 255, 1, 0, 60, 255, 0, 1, 60, 255, 1, 1, 60, 255]);
        assert_eq!(project.layers[1].pixels.region(2, 2, 1, 1), [2, 2, 60, 255]);
        project.add_raster_layer(&RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])), "small".into(), region, false).unwrap();
        assert_eq!(&project.layers[2].pixels.to_rgba(project.width, project.height)[..4], &[1, 2, 3, 255]);
        assert!(project.layers[2].pixels.to_rgba(project.width, project.height)[4..].iter().all(|b| *b == 0));
    }

    #[test]
    fn fit_result_matches_original_bounds_and_position() {
        for side in [1, 4] {
            let mut project = Project::default();
            project.import_rgba(&vec![0; 5 * 4 * 4], 5, 4).unwrap();
            let region = crate::editing::PixelRect { min_x: 2, min_y: 1, max_x: 4, max_y: 3 };
            let result = RgbaImage::from_pixel(side, side, Rgba([40, 50, 60, 128]));
            project.add_raster_layer(&result, "Fit".into(), region, true).unwrap();
            assert_eq!((project.width, project.height), (5, 4));
            let actual = RgbaImage::from_raw(5, 4, project.layers[1].pixels.to_rgba(project.width, project.height).clone()).unwrap();
            for (x, y, pixel) in actual.enumerate_pixels() {
                assert_eq!(*pixel, if (2..4).contains(&x) && (1..3).contains(&y) { Rgba([40, 50, 60, 128]) } else { Rgba([0; 4]) });
            }
            assert!(project.layers[0].pixels.to_rgba(project.width, project.height).iter().all(|b| *b == 0));
        }
    }

    fn sample_png() -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 3, Rgba([10, 20, 30, 255])));
        encode_png(&image).unwrap()
    }

    #[test]
    fn project_round_trip_preserves_manifest_and_pixels() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("roundtrip.mgimg");
        let mut project = Project::default();
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 3, Rgba([10, 20, 30, 255])));
        project.set_image(image).unwrap();

        project.save(&path).unwrap();
        let loaded = Project::open(&path).unwrap();

        assert_eq!(loaded.width, 2);
        assert_eq!(loaded.height, 3);
        assert_eq!(loaded.layers.len(), 1);
        let (_, width, height) = loaded.decoded_rgba().unwrap().unwrap();
        assert_eq!((width, height), (2, 3));
    }

    #[test]
    fn import_rgba_stores_png_asset() {
        let mut project = Project::default();
        let pixels = vec![10, 20, 30, 255, 40, 50, 60, 255];
        project.import_rgba(&pixels, 2, 1).unwrap();
        assert_eq!(project.manifest.asset_path.as_deref(), Some(IMAGE_PATH));
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
