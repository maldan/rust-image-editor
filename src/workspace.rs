use std::fs;
use std::collections::{HashMap, HashSet};
use crate::raster::SparsePixels;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageFormat, RgbaImage};
use mega_file::{MegaFile, WriteFile};
use serde::{Deserialize, Serialize};

use crate::project::{Layer, LayerMeta, Project, encode_png_rgba};

const MANIFEST_PATH: &str = "/manifest.json";
const MANIFEST_KIND: &str = "mega-image-editor/manifest+json";
const IMAGE_KIND: &str = "image/png";
pub const THUMB_SIZE: u32 = 84;
/// On-screen library thumb size (logical px before UI scale).
pub const THUMB_VIEW: f32 = 73.0;

#[derive(Debug, Clone)]
pub struct ImageAsset {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// Packed RGBA thumb `THUMB_SIZE x THUMB_SIZE`.
    pub thumb: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AssetMeta {
    id: String,
    name: String,
    width: u32,
    height: u32,
    path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DocumentMeta {
    id: String,
    name: String,
    width: u32,
    height: u32,
    active_layer: usize,
    layers: Vec<LayerMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceManifest {
    format: String,
    version: u32,
    #[serde(default)]
    active_document: usize,
    #[serde(default)]
    assets: Vec<AssetMeta>,
    #[serde(default)]
    documents: Vec<DocumentMeta>,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub id: String,
    pub name: String,
    pub project: Project,
    /// Cached library preview; rebuilt only when `thumb_dirty`.
    pub thumb: Vec<u8>,
    pub thumb_dirty: bool,
}

impl Document {
    pub fn new(id: String, name: String, project: Project) -> Self {
        let mut doc = Self {
            id,
            name,
            project,
            thumb: empty_thumb(),
            thumb_dirty: true,
        };
        doc.rebuild_thumb();
        doc
    }

    pub fn mark_thumb_dirty(&mut self) {
        self.thumb_dirty = true;
    }

    /// Full composite preview — call from debounced tick, not every frame.
    pub fn rebuild_thumb(&mut self) {
        let p = &self.project;
        self.thumb = if p.layers.is_empty() || p.width == 0 || p.height == 0 {
            empty_thumb()
        } else {
            let pixels = crate::editing::composite_layers(&p.layers, p.width, p.height);
            make_thumb(&pixels, p.width, p.height)
        };
        self.thumb_dirty = false;
    }
}

fn empty_thumb() -> Vec<u8> {
    vec![0u8; (THUMB_SIZE * THUMB_SIZE * 4) as usize]
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub assets: Vec<ImageAsset>,
    pub documents: Vec<Document>,
    pub active_document: usize,
    pub path: Option<PathBuf>,
    pub tree_selected: Option<String>,
    /// Asset index being dragged onto the canvas (grab thumb and drop).
    pub asset_drag: Option<usize>,
    pub asset_drag_origin: Option<glam::Vec2>,
    /// True after pointer moved past drag threshold.
    pub asset_dragging: bool,
    /// Rebuild active document thumb after this instant (1s debounce).
    pub thumb_rebuild_at: Option<std::time::Instant>,
    next_id: u64,
    saved: Option<SavedResources>,
}

/// Last successful disk state. Sparse snapshots share tiles until an edit changes them.
#[derive(Debug, Clone, Default)]
struct SavedResources {
    assets: HashMap<String, (u32, u32, u64)>,
    layers: HashMap<String, SparsePixels>,
    manifest: Vec<u8>,
}

impl Default for Workspace {
    fn default() -> Self {
        let mut ws = Self {
            assets: Vec::new(),
            documents: Vec::new(),
            active_document: 0,
            path: None,
            tree_selected: None,
            asset_drag: None,
            asset_drag_origin: None,
            asset_dragging: false,
            thumb_rebuild_at: None,
            next_id: 1,
            saved: None,
        };
        ws.create_document("Document 1");
        ws
    }
}

impl Workspace {
    fn alloc_id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}_{}", self.next_id);
        self.next_id += 1;
        id
    }

    pub fn active(&self) -> &Project {
        &self.documents[self.active_document].project
    }

    pub fn active_mut(&mut self) -> &mut Project {
        &mut self.documents[self.active_document].project
    }

    pub fn active_doc(&self) -> &Document {
        &self.documents[self.active_document]
    }

    pub fn create_document(&mut self, name: &str) -> usize {
        let id = self.alloc_id("doc");
        self.documents
            .push(Document::new(id, name.into(), Project::default()));
        self.active_document = self.documents.len() - 1;
        self.active_document
    }

    pub fn mark_active_thumb_dirty(&mut self) {
        if let Some(doc) = self.documents.get_mut(self.active_document) {
            doc.mark_thumb_dirty();
        }
        // Debounce: reset 1s timer on every edit.
        self.thumb_rebuild_at =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(1));
    }

    pub fn rebuild_active_thumb_now(&mut self) {
        if let Some(doc) = self.documents.get_mut(self.active_document) {
            doc.rebuild_thumb();
        }
        self.thumb_rebuild_at = None;
    }

    /// Call once per frame: rebuild doc thumb after 1s idle.
    pub fn tick_thumbs(&mut self) -> bool {
        let Some(at) = self.thumb_rebuild_at else {
            return false;
        };
        if std::time::Instant::now() < at {
            return true; // still waiting — keep redrawing so timer can fire
        }
        if let Some(doc) = self.documents.get_mut(self.active_document) {
            if doc.thumb_dirty {
                doc.rebuild_thumb();
            }
        }
        self.thumb_rebuild_at = None;
        false
    }

    pub fn begin_asset_drag(&mut self, index: usize, origin: glam::Vec2) {
        self.asset_drag = Some(index);
        self.asset_drag_origin = Some(origin);
        self.asset_dragging = false;
    }

    pub fn clear_asset_drag(&mut self) {
        self.asset_drag = None;
        self.asset_drag_origin = None;
        self.asset_dragging = false;
    }

    pub fn delete_document(&mut self, index: usize) {
        if self.documents.len() <= 1 || index >= self.documents.len() {
            return;
        }
        self.documents.remove(index);
        if self.active_document >= self.documents.len() {
            self.active_document = self.documents.len() - 1;
        } else if index < self.active_document {
            self.active_document -= 1;
        }
    }

    pub fn set_active_document(&mut self, index: usize) {
        if index < self.documents.len() {
            self.active_document = index;
        }
    }

    pub fn import_asset_image(&mut self, path: &Path) -> Result<usize, String> {
        let bytes = fs::read(path).map_err(|e| format!("Cannot read image: {e}"))?;
        let decoded = image::load_from_memory(&bytes)
            .map_err(|e| format!("Invalid image: {e}"))?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Asset")
            .to_string();
        self.add_asset_image(name, decoded)
    }

    pub fn import_asset_rgba(
        &mut self,
        name: String,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> Result<usize, String> {
        let image = RgbaImage::from_raw(width, height, pixels.to_vec())
            .ok_or_else(|| "Invalid image size".to_string())?;
        self.add_asset_image(name, DynamicImage::ImageRgba8(image))
    }

    fn add_asset_image(&mut self, name: String, image: DynamicImage) -> Result<usize, String> {
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        if width == 0 || height == 0 {
            return Err("Image is empty".into());
        }
        let pixels = rgba.into_raw();
        let thumb = make_thumb(&pixels, width, height);
        let id = self.alloc_id("img");
        self.assets.push(ImageAsset {
            id,
            name,
            width,
            height,
            pixels,
            thumb,
        });
        Ok(self.assets.len() - 1)
    }

    pub fn delete_asset(&mut self, index: usize) {
        if index < self.assets.len() {
            self.assets.remove(index);
        }
    }

    /// Place asset pixels as a new layer on the active document (or create canvas).
    pub fn place_asset_on_active(&mut self, asset_index: usize) -> Result<(), String> {
        let asset = self
            .assets
            .get(asset_index)
            .ok_or_else(|| "Asset not found".to_string())?
            .clone();
        let doc = self.active_mut();
        if doc.layers.is_empty() || doc.width == 0 || doc.height == 0 {
            doc.import_rgba(&asset.pixels, asset.width, asset.height)?;
            if let Some(layer) = doc.layers.last_mut() {
                layer.name = asset.name;
            }
        } else {
            doc.add_layer();
            let target = &mut doc.layers[doc.active_layer];
            target.name = asset.name;
            target.pixels = SparsePixels::from_rgba(&asset.pixels, asset.width, asset.height);
        }
        self.rebuild_active_thumb_now();
        Ok(())
    }

    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        let incremental = self.path.as_deref() == Some(path) && path.is_file();
        let previous = if incremental { self.saved.as_ref() } else { None };
        let mut next = SavedResources::default();
        let mut files = Vec::new();
        let mut asset_metas = Vec::new();
        for asset in &self.assets {
            let asset_path = format!("/assets/{}.png", asset.id);
            let signature = (asset.width, asset.height, crate::safe_save::fingerprint(&asset.pixels));
            if previous.and_then(|saved| saved.assets.get(&asset_path)) != Some(&signature) {
                files.push(WriteFile::new(
                    asset_path.clone(), IMAGE_KIND,
                    encode_png_rgba(&asset.pixels, asset.width, asset.height)?,
                ));
            }
            next.assets.insert(asset_path.clone(), signature);
            asset_metas.push(AssetMeta {
                id: asset.id.clone(),
                name: asset.name.clone(),
                width: asset.width,
                height: asset.height,
                path: asset_path,
            });
        }
        let mut doc_metas = Vec::new();
        for doc in &self.documents {
            let mut layers = Vec::new();
            for (i, layer) in doc.project.layers.iter().enumerate() {
                let layer_path = format!("/documents/{}/layers/{i}.tiles", doc.id);
                if previous.and_then(|saved| saved.layers.get(&layer_path)) != Some(&layer.pixels) {
                    files.push(WriteFile::new(
                        layer_path.clone(),
                        "application/x-rgba-tiles-64",
                        layer.pixels.encode(),
                    ));
                }
                next.layers.insert(layer_path.clone(), layer.pixels.clone());
                layers.push(LayerMeta {
                    name: layer.name.clone(),
                    visible: layer.visible,
                    opacity: layer.opacity,
                    path: layer_path,
                });
            }
            doc_metas.push(DocumentMeta {
                id: doc.id.clone(),
                name: doc.name.clone(),
                width: doc.project.width,
                height: doc.project.height,
                active_layer: doc.project.active_layer,
                layers,
            });
        }
        let manifest = WorkspaceManifest {
            format: "mega-image-editor".into(),
            version: 5,
            active_document: self.active_document.min(self.documents.len().saturating_sub(1)),
            assets: asset_metas,
            documents: doc_metas,
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| format!("Cannot encode manifest: {e}"))?;
        if previous.map(|saved| saved.manifest.as_slice()) != Some(manifest_bytes.as_slice()) {
            files.push(WriteFile::new(MANIFEST_PATH, MANIFEST_KIND, manifest_bytes.clone()));
        }
        if incremental {
            crate::safe_save::append_archive(path, files, previous.map(|saved| saved.manifest.as_slice()))?;
        } else {
            crate::safe_save::save_archive(path, files)?;
        }
        next.manifest = manifest_bytes;
        self.saved = Some(next);
        self.path = Some(path.to_path_buf());
        Ok(())
    }

    /// Compact only the saved revision. Unsaved in-memory edits are not committed.
    pub fn optimize(&self) -> Result<(u64, u64), String> {
        let path = self.path.as_deref().ok_or("Save or open a project before optimizing")?;
        let before = fs::metadata(path).map_err(|e| format!("Cannot inspect project: {e}"))?.len();
        let files = {
            let archive = MegaFile::open(path).map_err(|e| format!("Cannot open project: {e}"))?;
            let manifest_bytes = archive.read_all(MANIFEST_PATH).map_err(|e| format!("Cannot read saved manifest: {e}"))?;
            let manifest: WorkspaceManifest = serde_json::from_slice(&manifest_bytes).map_err(|e| format!("Invalid saved manifest: {e}"))?;
            if manifest.format != "mega-image-editor" || manifest.version != 5 {
                return Err("Unsupported project format or version".into());
            }
            let mut paths = HashSet::from([MANIFEST_PATH.to_string()]);
            for asset in manifest.assets { paths.insert(asset.path); }
            for document in manifest.documents {
                for layer in document.layers { paths.insert(layer.path); }
            }
            let mut paths: Vec<_> = paths.into_iter().collect();
            paths.sort();
            let mut files = Vec::new();
            for path in paths {
                let metadata = archive.meta(&path).map_err(|e| format!("Missing saved resource {path}: {e}"))?;
                let bytes = archive.read_all(&path).map_err(|e| format!("Cannot read saved resource {path}: {e}"))?;
                let mut file = WriteFile::new(path, metadata.kind, bytes);
                file.created_at = Some(metadata.created_at);
                file.updated_at = Some(metadata.updated_at);
                file.flags = metadata.flags;
                files.push(file);
            }
            files
        };
        crate::safe_save::save_archive(path, files)?;
        let after = fs::metadata(path).map_err(|e| format!("Cannot inspect optimized project: {e}"))?.len();
        Ok((before, after))
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let archive = MegaFile::open(path).map_err(|e| format!("Cannot open project: {e}"))?;
        let manifest_bytes = archive
            .read_all(MANIFEST_PATH)
            .map_err(|e| format!("Project has no valid manifest: {e}"))?;
        let value: serde_json::Value = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| format!("Invalid project manifest: {e}"))?;
        let manifest: WorkspaceManifest = serde_json::from_value(value)
            .map_err(|e| format!("Invalid project manifest: {e}"))?;
        if manifest.format != "mega-image-editor" || manifest.version != 5 {
            return Err("Unsupported project format or version".into());
        }
        let mut next_id = 1u64;
        let mut assets = Vec::new();
        for meta in &manifest.assets {
            let bytes = archive
                .read_all(&meta.path)
                .map_err(|e| format!("Asset '{}' missing: {e}", meta.name))?;
            let rgba = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
                .map_err(|e| format!("Cannot decode asset '{}': {e}", meta.name))?
                .to_rgba8();
            let (width, height) = rgba.dimensions();
            let pixels = rgba.into_raw();
            let thumb = make_thumb(&pixels, width, height);
            bump_id(&meta.id, &mut next_id);
            assets.push(ImageAsset {
                id: meta.id.clone(),
                name: meta.name.clone(),
                width,
                height,
                pixels,
                thumb,
            });
        }
        let mut documents = Vec::new();
        for meta in &manifest.documents {
            bump_id(&meta.id, &mut next_id);
            let mut layers = Vec::new();
            let width = meta.width;
            let height = meta.height;
            for layer_meta in &meta.layers {
                let bytes = archive
                    .read_all(&layer_meta.path)
                    .map_err(|e| format!("Layer '{}' missing: {e}", layer_meta.name))?;
                layers.push(Layer {
                    name: layer_meta.name.clone(),
                    visible: layer_meta.visible,
                    opacity: layer_meta.opacity.clamp(0.0, 1.0),
                    pixels: SparsePixels::decode(&bytes)?,
                });
            }
            let mut project = Project {
                width,
                height,
                layers,
                active_layer: meta.active_layer,
                path: None,
                manifest: Default::default(),
            };
            if !project.layers.is_empty() {
                project.active_layer = project.active_layer.min(project.layers.len() - 1);
            }
            project.sync_manifest_pub();
            documents.push(Document::new(meta.id.clone(), meta.name.clone(), project));
        }
        if documents.is_empty() {
            documents.push(Document::new(
                "doc_1".into(),
                "Document 1".into(),
                Project::default(),
            ));
        }
        let mut workspace = Self {
            assets,
            active_document: manifest.active_document.min(documents.len() - 1),
            documents,
            path: Some(path.to_path_buf()),
            tree_selected: None,
            asset_drag: None,
            asset_drag_origin: None,
            asset_dragging: false,
            thumb_rebuild_at: None,
            next_id: next_id.max(1),
            saved: None,
        };
        let mut saved = SavedResources { manifest: manifest_bytes, ..Default::default() };
        for asset in &workspace.assets {
            saved.assets.insert(format!("/assets/{}.png", asset.id),
                (asset.width, asset.height, crate::safe_save::fingerprint(&asset.pixels)));
        }
        for document in &workspace.documents {
            for (index, layer) in document.project.layers.iter().enumerate() {
                saved.layers.insert(format!("/documents/{}/layers/{index}.tiles", document.id), layer.pixels.clone());
            }
        }
        workspace.saved = Some(saved);
        Ok(workspace)
    }
}

fn bump_id(id: &str, next: &mut u64) {
    if let Some(num) = id.rsplit('_').next().and_then(|s| s.parse::<u64>().ok()) {
        *next = (*next).max(num + 1);
    }
}

pub fn make_thumb(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut thumb = empty_thumb();
    if width == 0 || height == 0 {
        return thumb;
    }
    // Fit inside square, preserve aspect, area-average resample (anti-aliased).
    let scale = (THUMB_SIZE as f32 / width as f32).min(THUMB_SIZE as f32 / height as f32);
    let dw = ((width as f32 * scale).round() as u32).clamp(1, THUMB_SIZE);
    let dh = ((height as f32 * scale).round() as u32).clamp(1, THUMB_SIZE);
    let resized = crate::editing::resize_rgba(pixels, width, height, dw, dh);
    let ox = (THUMB_SIZE - dw) / 2;
    let oy = (THUMB_SIZE - dh) / 2;
    for y in 0..dh {
        for x in 0..dw {
            let src = ((y * dw + x) * 4) as usize;
            let dst = (((oy + y) * THUMB_SIZE + (ox + x)) * 4) as usize;
            if src + 4 <= resized.len() && dst + 4 <= thumb.len() {
                thumb[dst..dst + 4].copy_from_slice(&resized[src..src + 4]);
            }
        }
    }
    thumb
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Raster;

    fn saved_workspace(path: &Path) -> Workspace {
        let mut workspace = Workspace::default();
        workspace.import_asset_rgba("reference".into(), &[10, 20, 30, 255].repeat(16), 4, 4).unwrap();
        workspace.place_asset_on_active(0).unwrap();
        workspace.save(path).unwrap();
        workspace
    }

    #[test]
    fn incremental_save_skips_unchanged_resources_including_after_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("incremental.mgimg");
        let mut workspace = saved_workspace(&path);
        let original = fs::read(&path).unwrap();
        workspace.save(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        let mut reopened = Workspace::open(&path).unwrap();
        reopened.save(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);

        reopened.active_mut().layers[0].name = "renamed".into();
        reopened.save(&path).unwrap();
        let renamed = fs::read(&path).unwrap();
        assert!(renamed.starts_with(&original));
        assert!(renamed.len() - original.len() < crate::raster::TILE_BYTES);
        assert_eq!(Workspace::open(&path).unwrap().active().layers[0].name, "renamed");

        reopened.active_mut().layers[0].pixels.write(0, 2, 1, [100, 120, 140, 128]);
        reopened.save(&path).unwrap();
        let edited = fs::read(&path).unwrap();
        assert!(edited.starts_with(&renamed));
        assert_eq!(Workspace::open(&path).unwrap().active().layers[0].pixels.read(0, 2, 1), [100, 120, 140, 128]);
    }

    #[test]
    fn incremental_save_keeps_resource_paths_correct_after_reorder_and_removal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layers.mgimg");
        let mut workspace = saved_workspace(&path);
        workspace.active_mut().add_layer();
        workspace.active_mut().layers[1].pixels.write(0, -20, 30, [80, 90, 100, 255]);
        workspace.save(&path).unwrap();
        workspace.active_mut().move_layer_to(1, 0);
        workspace.save(&path).unwrap();
        let loaded = Workspace::open(&path).unwrap();
        assert_eq!(loaded.active().layers[0].pixels.read(0, -20, 30), [80, 90, 100, 255]);
        assert_eq!(loaded.active().layers[1].pixels.read(0, 0, 0), [10, 20, 30, 255]);
        workspace.active_mut().delete_layer(0);
        workspace.delete_asset(0);
        workspace.save(&path).unwrap();
        let loaded = Workspace::open(&path).unwrap();
        assert_eq!(loaded.assets.len(), 0);
        assert_eq!(loaded.active().layers.len(), 1);
        assert_eq!(loaded.active().layers[0].pixels.read(0, 0, 0), [10, 20, 30, 255]);
    }

    #[test]
    fn interrupted_incremental_commit_reopens_previous_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.mgimg");
        let mut workspace = saved_workspace(&path);
        let original = fs::read(&path).unwrap();
        workspace.active_mut().layers[0].pixels.write(0, 0, 0, [200, 201, 202, 255]);
        workspace.active_mut().layers[0].name = "changed".into();
        workspace.save(&path).unwrap();
        let committed = fs::read(&path).unwrap();
        assert!(committed.starts_with(&original));
        for cut in [original.len(), original.len() + 1, committed.len() - 1] {
            let interrupted = dir.path().join(format!("interrupted-{cut}.mgimg"));
            fs::write(&interrupted, &committed[..cut]).unwrap();
            let loaded = Workspace::open(&interrupted).unwrap();
            assert_eq!(loaded.active().layers[0].pixels.read(0, 0, 0), [10, 20, 30, 255]);
            assert_ne!(loaded.active().layers[0].name, "changed");
        }
    }

    #[test]
    fn optimize_reclaims_removed_resources_and_does_not_save_memory_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("optimize.mgimg");
        let mut workspace = saved_workspace(&path);
        workspace.active_mut().add_layer();
        workspace.active_mut().layers[1].pixels.write(0, 500, -500, [50, 60, 70, 255]);
        workspace.save(&path).unwrap();
        workspace.active_mut().delete_layer(1);
        workspace.delete_asset(0);
        workspace.save(&path).unwrap();
        let saved_manifest = workspace.saved.as_ref().unwrap().manifest.clone();
        workspace.active_mut().layers[0].pixels.write(0, 0, 0, [200, 201, 202, 255]);
        let (before, after) = workspace.optimize().unwrap();
        assert!(after < before);
        assert_eq!(workspace.active().layers[0].pixels.read(0, 0, 0), [200, 201, 202, 255]);
        assert_eq!(workspace.saved.as_ref().unwrap().manifest, saved_manifest);
        let loaded = Workspace::open(&path).unwrap();
        assert_eq!(loaded.active().layers[0].pixels.read(0, 0, 0), [10, 20, 30, 255]);
        let archive = MegaFile::open(&path).unwrap();
        assert_eq!(archive.list_all().unwrap().len(), 2); // Manifest + one live layer.
        drop(archive);
        workspace.save(&path).unwrap();
        assert_eq!(Workspace::open(&path).unwrap().active().layers[0].pixels.read(0, 0, 0), [200, 201, 202, 255]);
        let optimized = fs::read(&path).unwrap();
        workspace.save(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), optimized);
        assert!(Workspace::default().optimize().is_err());
    }

    #[cfg(windows)]
    #[test]
    fn incremental_save_failure_keeps_snapshot_dirty_for_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.mgimg");
        let mut workspace = saved_workspace(&path);
        let original = fs::read(&path).unwrap();
        workspace.active_mut().layers[0].pixels.write(0, 0, 0, [200, 201, 202, 255]);
        let lock = fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        assert!(workspace.save(&path).is_err());
        assert!(workspace.optimize().is_err());
        drop(lock);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_ne!(workspace.saved.as_ref().unwrap().layers.values().next().unwrap(), &workspace.active().layers[0].pixels);
        workspace.save(&path).unwrap();
        assert_eq!(Workspace::open(&path).unwrap().active().layers[0].pixels.read(0, 0, 0), [200, 201, 202, 255]);
    }

    #[test]
    fn import_asset_does_not_touch_document() {
        let mut ws = Workspace::default();
        assert!(ws.active().layers.is_empty());
        ws.import_asset_rgba("a".into(), &[9, 8, 7, 255], 1, 1)
            .unwrap();
        assert_eq!(ws.assets.len(), 1);
        assert!(ws.active().layers.is_empty());
    }

    #[test]
    fn workspace_round_trip_keeps_assets_and_docs() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ws.mgimg");
        let mut ws = Workspace::default();
        ws.import_asset_rgba("tex".into(), &[10, 20, 30, 255, 40, 50, 60, 255], 2, 1)
            .unwrap();
        ws.place_asset_on_active(0).unwrap();
        ws.create_document("Doc B");
        ws.save(&path).unwrap();

        let loaded = Workspace::open(&path).unwrap();
        assert_eq!(loaded.assets.len(), 1);
        assert_eq!(loaded.documents.len(), 2);
        assert_eq!(loaded.assets[0].name, "tex");
        assert_eq!(loaded.documents[0].project.layers.len(), 1);
    }
}
