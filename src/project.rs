use std::fs;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageFormat};
use mega_file::{MegaFile, WriteFile};
use serde::{Deserialize, Serialize};

const MANIFEST_PATH: &str = "/manifest.json";
const IMAGE_PATH: &str = "/assets/main/image.png";
const MANIFEST_KIND: &str = "mega-image-editor/manifest+json";
const IMAGE_KIND: &str = "image/png";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub document_type: String,
    pub asset_path: Option<String>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            format: "mega-image-editor".into(),
            version: 1,
            document_type: "image".into(),
            asset_path: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Project {
    pub manifest: Manifest,
    pub image_png: Option<Vec<u8>>,
    pub path: Option<PathBuf>,
}

impl Project {
    pub fn import_png(&mut self, path: &Path) -> Result<(), String> {
        let bytes = fs::read(path).map_err(|e| format!("Cannot read image: {e}"))?;
        let decoded = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
            .map_err(|e| format!("Invalid PNG: {e}"))?;
        let normalized = encode_png(&decoded)?;
        self.image_png = Some(normalized);
        self.manifest.asset_path = Some(IMAGE_PATH.into());
        Ok(())
    }

    pub fn decoded_rgba(&self) -> Result<Option<(Vec<u8>, u32, u32)>, String> {
        let Some(bytes) = self.image_png.as_ref() else {
            return Ok(None);
        };
        let rgba = image::load_from_memory_with_format(bytes, ImageFormat::Png)
            .map_err(|e| format!("Cannot decode project image: {e}"))?
            .to_rgba8();
        let (width, height) = rgba.dimensions();
        Ok(Some((rgba.into_raw(), width, height)))
    }

    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        let archive = MegaFile::create(path).map_err(|e| format!("Cannot create project: {e}"))?;
        let manifest = serde_json::to_vec_pretty(&self.manifest)
            .map_err(|e| format!("Cannot encode manifest: {e}"))?;
        let mut files = vec![WriteFile::new(MANIFEST_PATH, MANIFEST_KIND, manifest)];
        if let Some(image) = self.image_png.as_ref() {
            files.push(WriteFile::new(IMAGE_PATH, IMAGE_KIND, image.clone()));
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
        if manifest.format != "mega-image-editor" || manifest.version != 1 {
            return Err("Unsupported project format or version".into());
        }
        let image_png = match manifest.asset_path.as_deref() {
            Some(asset_path) => Some(
                archive
                    .read_all(asset_path)
                    .map_err(|e| format!("Project image is missing: {e}"))?,
            ),
            None => None,
        };
        Ok(Self {
            manifest,
            image_png,
            path: Some(path.to_path_buf()),
        })
    }

    pub fn export_png(&self, path: &Path) -> Result<(), String> {
        let bytes = self
            .image_png
            .as_ref()
            .ok_or_else(|| "Import a PNG before exporting".to_string())?;
        fs::write(path, bytes).map_err(|e| format!("Cannot export PNG: {e}"))
    }
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

    fn sample_png() -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 3, Rgba([10, 20, 30, 255])));
        encode_png(&image).unwrap()
    }

    #[test]
    fn project_round_trip_preserves_manifest_and_pixels() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("roundtrip.mgimg");
        let mut project = Project {
            image_png: Some(sample_png()),
            ..Project::default()
        };
        project.manifest.asset_path = Some(IMAGE_PATH.into());

        project.save(&path).unwrap();
        let loaded = Project::open(&path).unwrap();

        assert_eq!(loaded.manifest, project.manifest);
        let (_, width, height) = loaded.decoded_rgba().unwrap().unwrap();
        assert_eq!((width, height), (2, 3));
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
}
