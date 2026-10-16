use std::fs;
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
            version: 2,
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
    pub pixels: Vec<u8>,
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
            pixels: rgba.into_raw(),
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
            pixels: vec![0u8; self.width as usize * self.height as usize * 4],
        });
        self.active_layer = self.layers.len() - 1;
        self.sync_manifest();
    }

    /// Preserve server pixels at their native size. Layers share canvas dimensions,
    /// so expand the canvas if needed, and transparently pad the remaining area.
    pub fn add_raster_layer(&mut self, image: &image::RgbaImage, name: String) -> Result<(), String> {
        let (w, h) = image.dimensions();
        if w == 0 || h == 0 { return Err("Result image is empty".into()); }
        let (width, height) = (self.width.max(w), self.height.max(h));
        if self.layers.is_empty() {
            self.import_rgba(image.as_raw(), w, h)?;
        } else {
            self.resize_canvas(width, height)?;
            self.add_layer();
            let target = &mut self.layers[self.active_layer].pixels;
            for y in 0..h as usize {
                let source = y * w as usize * 4;
                let dest = y * width as usize * 4;
                target[dest..dest + w as usize * 4].copy_from_slice(&image.as_raw()[source..source + w as usize * 4]);
            }
        }
        self.layers[self.active_layer].name = name;
        self.sync_manifest();
        Ok(())
    }

    /// Resize canvas; content is kept top-left, cropped or padded with transparency.
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
                pixels: vec![0u8; width as usize * height as usize * 4],
            });
            self.active_layer = 0;
            self.sync_manifest();
            return Ok(());
        }
        let old_w = self.width;
        let old_h = self.height;
        if old_w == width && old_h == height {
            return Ok(());
        }
        for layer in &mut self.layers {
            let mut next = vec![0u8; width as usize * height as usize * 4];
            let copy_w = old_w.min(width);
            let copy_h = old_h.min(height);
            for y in 0..copy_h {
                let src = (y * old_w * 4) as usize;
                let dst = (y * width * 4) as usize;
                let bytes = (copy_w * 4) as usize;
                next[dst..dst + bytes].copy_from_slice(&layer.pixels[src..src + bytes]);
            }
            layer.pixels = next;
        }
        self.width = width;
        self.height = height;
        self.sync_manifest();
        Ok(())
    }

    pub fn delete_active_layer(&mut self) {
        if self.layers.is_empty() {
            return;
        }
        self.layers.remove(self.active_layer);
        if self.layers.is_empty() {
            self.width = 0;
            self.height = 0;
            self.active_layer = 0;
        } else {
            self.active_layer = self.active_layer.min(self.layers.len() - 1);
        }
        self.sync_manifest();
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

    pub fn active_pixels(&self) -> Option<&[u8]> {
        self.layers.get(self.active_layer).map(|layer| layer.pixels.as_slice())
    }

    pub fn active_pixels_mut(&mut self) -> Option<&mut Vec<u8>> {
        self.layers
            .get_mut(self.active_layer)
            .map(|layer| &mut layer.pixels)
    }

    pub fn sync_manifest_pub(&mut self) {
        self.sync_manifest();
    }

    fn sync_manifest(&mut self) {
        self.manifest.version = 2;
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
                path: format!("/layers/{index}.png"),
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
        let archive = MegaFile::create(path).map_err(|e| format!("Cannot create project: {e}"))?;
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
                format!("/layers/{index}.png"),
                IMAGE_KIND,
                encode_png_rgba(&layer.pixels, self.width, self.height)?,
            ));
        }
        archive
            .write_files(files)
            .map_err(|e| format!("Cannot write project: {e}"))?;
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
        if manifest.format != "mega-image-editor" || manifest.version > 2 {
            return Err("Unsupported project format or version".into());
        }
        let mut project = if manifest.version >= 2 && !manifest.layers.is_empty() {
            let mut layers = Vec::new();
            let mut width = manifest.width;
            let mut height = manifest.height;
            for meta in &manifest.layers {
                let bytes = archive
                    .read_all(&meta.path)
                    .map_err(|e| format!("Layer '{}' is missing: {e}", meta.name))?;
                let rgba = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
                    .map_err(|e| format!("Cannot decode layer '{}': {e}", meta.name))?
                    .to_rgba8();
                let (layer_w, layer_h) = rgba.dimensions();
                if width == 0 || height == 0 {
                    width = layer_w;
                    height = layer_h;
                }
                layers.push(Layer {
                    name: meta.name.clone(),
                    visible: meta.visible,
                    opacity: meta.opacity.clamp(0.0, 1.0),
                    pixels: rgba.into_raw(),
                });
            }
            Self {
                width,
                height,
                layers,
                active_layer: manifest.active_layer.min(manifest.layers.len() - 1),
                manifest,
                path: Some(path.to_path_buf()),
            }
        } else {
            let image_png = match manifest.asset_path.as_deref() {
                Some(asset_path) => Some(
                    archive
                        .read_all(asset_path)
                        .map_err(|e| format!("Project image is missing: {e}"))?,
                ),
                None => None,
            };
            let mut project = Self {
                manifest,
                path: Some(path.to_path_buf()),
                ..Self::default()
            };
            if let Some(bytes) = image_png {
                let decoded = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
                    .map_err(|e| format!("Cannot decode project image: {e}"))?;
                project.set_image(decoded)?;
            }
            project
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
    fn server_result_is_never_rescaled_or_cropped() {
        let mut project = Project::default();
        project.import_rgba(&[10, 20, 30, 255], 1, 1).unwrap();
        let result = RgbaImage::from_pixel(3, 2, Rgba([40, 50, 60, 128]));
        project.add_raster_layer(&result, "AI Edit".into()).unwrap();
        assert_eq!((project.width, project.height), (3, 2));
        assert_eq!(project.layers[1].pixels, result.into_raw());
        assert_eq!(&project.layers[0].pixels[..4], &[10, 20, 30, 255]);
        assert!(project.layers[0].pixels[4..].iter().all(|b| *b == 0));
        project.add_raster_layer(&RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])), "small".into()).unwrap();
        assert_eq!(&project.layers[2].pixels[..4], &[1, 2, 3, 255]);
        assert!(project.layers[2].pixels[4..].iter().all(|b| *b == 0));
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
        project.delete_active_layer();
        assert_eq!(project.layers.len(), 1);
    }
}
