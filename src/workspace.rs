use crate::raster::SparsePixels;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::{DynamicImage, RgbaImage};
use mega_file::{MegaFile, WriteFile};
use serde::{Deserialize, Serialize};

use crate::project::{Layer, Project, encode_webp_rgba};

const MANIFEST_PATH: &str = "/manifest.json";
const MANIFEST_KIND: &str = "mega-image-editor/manifest+json";
const IMAGE_KIND: &str = "image/webp";
const FORMAT_VERSION: u32 = 8;
pub const THUMB_SIZE: u32 = 84;
const THUMB_ATLAS_COLS: u32 = 16;
/// On-screen library thumb size (logical px before UI scale).
pub const THUMB_VIEW: f32 = 73.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VideoMeta {
    id: String,
    name: String,
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VideoAsset {
    pub id: String,
    pub name: String,
    pub byte_size: u64,
    pub thumb: Vec<u8>,
    pub poster_attempted: bool,
    bytes: Option<Arc<Vec<u8>>>,
    deferred: Option<DeferredAsset>,
}

impl VideoAsset {
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            byte_size: bytes.len() as u64,
            thumb: Vec::new(),
            poster_attempted: true,
            bytes: Some(Arc::new(bytes)),
            deferred: None,
        }
    }

    /// Seekable access without copying a resident MP4 or loading a saved MP4 in full.
    pub(crate) fn reader(&self) -> VideoReader {
        VideoReader {
            asset: self.clone(),
            position: 0,
        }
    }

    pub fn read(&self) -> Result<Vec<u8>, String> {
        if let Some(bytes) = &self.bytes {
            return Ok(bytes.as_ref().clone());
        }
        let source = self.deferred.as_ref().ok_or("Video has no data")?;
        source
            .source
            .0
            .read_all(&source.path)
            .map_err(|e| format!("Cannot read video: {e}"))
    }
}

pub(crate) struct VideoReader {
    asset: VideoAsset,
    position: u64,
}

impl std::io::Read for VideoReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.asset.byte_size.saturating_sub(self.position);
        let count = buffer.len().min(remaining.min(64 * 1024) as usize);
        if count == 0 {
            return Ok(0);
        }
        if let Some(bytes) = &self.asset.bytes {
            buffer[..count]
                .copy_from_slice(&bytes[self.position as usize..self.position as usize + count]);
        } else {
            let source = self
                .asset
                .deferred
                .as_ref()
                .ok_or_else(|| std::io::Error::other("Video has no data"))?;
            let bytes = source
                .source
                .0
                .read_at(&source.path, self.position, count)
                .map_err(std::io::Error::other)?;
            buffer[..count].copy_from_slice(&bytes);
        }
        self.position += count as u64;
        Ok(count)
    }
}

impl std::io::Seek for VideoReader {
    fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> {
        use std::io::SeekFrom;
        let position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self.asset.byte_size.checked_add_signed(offset),
        }
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid video seek")
        })?;
        self.position = position;
        Ok(position)
    }
}

#[derive(Debug, Clone)]
pub struct ImageAsset {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// Source file length. Clipboard imports keep the decoded buffer length.
    pub byte_size: u64,
    /// Packed RGBA thumb `THUMB_SIZE x THUMB_SIZE`.
    pub thumb: Vec<u8>,
    deferred: Option<DeferredAsset>,
}

/// An immutable archive table and file handle keep deferred entries readable
/// across append saves, compaction, document deletion/undo, and Save As.
#[derive(Clone)]
struct ArchiveSource(Arc<MegaFile<mega_file::FileBackend>>);

impl std::fmt::Debug for ArchiveSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArchiveSource")
    }
}

#[derive(Debug, Clone)]
struct DeferredAsset {
    source: ArchiveSource,
    path: String,
}

#[derive(Debug, Clone)]
struct DeferredDocument {
    source: ArchiveSource,
    layers: Vec<LayerMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AssetMeta {
    id: String,
    name: String,
    width: u32,
    height: u32,
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
}

fn default_opacity() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct LayerMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeline: Option<String>,
    name: String,
    visible: bool,
    #[serde(default = "default_opacity")]
    opacity: f32,
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compression: Option<String>,
}

/// Compression wraps the existing tile stream; codec names keep their meaning
/// permanently so future writers can change without losing older readers.
fn encode_layer(pixels: &SparsePixels) -> Result<Vec<u8>, String> {
    zstd::stream::encode_all(pixels.encode().as_slice(), 3)
        .map_err(|e| format!("Cannot compress layer with zstd: {e}"))
}

fn decode_layer(bytes: &[u8], compression: Option<&str>) -> Result<SparsePixels, String> {
    match compression.unwrap_or("none") {
        "none" => SparsePixels::decode(bytes),
        "zstd" => {
            let raw = zstd::stream::decode_all(bytes)
                .map_err(|e| format!("Cannot decompress layer with zstd: {e}"))?;
            SparsePixels::decode(&raw)
        }
        codec => Err(format!("Unsupported layer compression: {codec}")),
    }
}

fn encode_drawings(
    drawings: &std::collections::BTreeMap<u32, SparsePixels>,
) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    for (&frame, pixels) in drawings {
        let bytes = pixels.encode();
        raw.extend_from_slice(&frame.to_le_bytes());
        raw.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        raw.extend_from_slice(&bytes);
    }
    zstd::stream::encode_all(raw.as_slice(), 3).map_err(|e| e.to_string())
}

fn decode_drawings(bytes: &[u8]) -> Result<std::collections::BTreeMap<u32, SparsePixels>, String> {
    let raw = zstd::stream::decode_all(bytes).map_err(|e| e.to_string())?;
    let mut rest = raw.as_slice();
    let mut drawings = std::collections::BTreeMap::new();
    while !rest.is_empty() {
        if rest.len() < 12 {
            return Err("Invalid drawing timeline".into());
        }
        let frame = u32::from_le_bytes(rest[..4].try_into().unwrap());
        let length = usize::try_from(u64::from_le_bytes(rest[4..12].try_into().unwrap()))
            .map_err(|_| "Invalid drawing length")?;
        rest = &rest[12..];
        if length > rest.len() || drawings.contains_key(&frame) {
            return Err("Invalid drawing timeline".into());
        }
        drawings.insert(frame, SparsePixels::decode(&rest[..length])?);
        rest = &rest[length..];
    }
    if !drawings.contains_key(&0) {
        return Err("Timeline has no initial drawing".into());
    }
    Ok(drawings)
}

fn read_preview(source: &ArchiveSource, path: Option<&str>) -> Result<Option<Vec<u8>>, String> {
    let Some(path) = path else {
        return Ok(None);
    };
    let bytes = source
        .0
        .read_all(path)
        .map_err(|e| format!("Cannot read preview {path}: {e}"))?;
    let rgba = image::load_from_memory(&bytes)
        .map_err(|e| format!("Cannot decode preview {path}: {e}"))?
        .to_rgba8();
    if rgba.dimensions() != (THUMB_SIZE, THUMB_SIZE) {
        return Err(format!("Invalid preview size: {path}"));
    }
    Ok(Some(rgba.into_raw()))
}

fn save_preview(
    path: &str,
    pixels: &[u8],
    previous: Option<&SavedResources>,
    next: &mut SavedResources,
    files: &mut Vec<WriteFile>,
) -> Result<(), String> {
    let hash = crate::safe_save::fingerprint(pixels);
    if previous.and_then(|saved| saved.previews.get(path)) != Some(&hash) {
        files.push(WriteFile::new(
            path,
            IMAGE_KIND,
            encode_webp_rgba(pixels, THUMB_SIZE, THUMB_SIZE)?,
        ));
    }
    next.previews.insert(path.to_string(), hash);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DocumentMeta {
    #[serde(default, rename = "type")]
    kind: DocumentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    animation: Option<crate::project::Animation>,
    id: String,
    name: String,
    width: u32,
    height: u32,
    active_layer: usize,
    layers: Vec<LayerMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum DocumentKind {
    #[default]
    Image,
    Animation,
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
    #[serde(default)]
    videos: Vec<VideoMeta>,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub id: String,
    pub name: String,
    pub project: Project,
    /// Cached library preview; rebuilt only when `thumb_dirty`.
    pub thumb: Vec<u8>,
    pub thumb_dirty: bool,
    deferred: Option<DeferredDocument>,
}

impl Document {
    pub fn new(id: String, name: String, project: Project) -> Self {
        let mut doc = Self {
            id,
            name,
            project,
            thumb: empty_thumb(),
            thumb_dirty: true,
            deferred: None,
        };
        doc.rebuild_thumb();
        doc
    }

    pub fn mark_thumb_dirty(&mut self) {
        self.thumb_dirty = true;
    }

    /// Full composite preview — call from debounced tick, not every frame.
    pub fn rebuild_thumb(&mut self) {
        if self.deferred.is_some() {
            return;
        }
        let p = &self.project;
        self.thumb = if p.layers.is_empty() || p.width == 0 || p.height == 0 {
            empty_thumb()
        } else {
            let pixels = crate::editing::composite_layers(&p.layers, p.width, p.height);
            make_thumb(&pixels, p.width, p.height)
        };
        self.thumb_dirty = false;
    }

    /// Pixel tiles currently resident for this document.
    pub fn memory_bytes(&self) -> u64 {
        if self.project.animation.is_none() {
            return self.project.layers.iter().map(|layer| layer.pixels.memory_bytes()).sum();
        }
        let mut seen = HashSet::new();
        let mut bytes = 0;
        for layer in &self.project.layers {
            for pixels in std::iter::once(&layer.pixels).chain(layer.drawings.values()) {
                for tile in pixels.tiles.values() {
                    if seen.insert(Arc::as_ptr(tile)) {
                        bytes += tile.len() as u64;
                    }
                }
            }
        }
        bytes
    }
}

fn empty_thumb() -> Vec<u8> {
    vec![0u8; (THUMB_SIZE * THUMB_SIZE * 4) as usize]
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub assets: Vec<ImageAsset>,
    pub videos: Vec<VideoAsset>,
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
    /// Bumped when a thumbnail or the library layout changes.
    thumb_revision: u64,
    thumb_packed_revision: u64,
    thumb_uploaded_revision: u64,
    pub thumb_atlas: Vec<u8>,
    pub thumb_atlas_size: (u32, u32),
    next_id: u64,
    saved: Option<SavedResources>,
}

/// Last successful disk state. Layers are identified by a content hash, not a second pixel buffer.
#[derive(Debug, Clone, Default)]
struct SavedResources {
    assets: HashMap<String, (u32, u32, Option<u64>)>,
    videos: HashSet<String>,
    layers: HashMap<String, (Option<u64>, Option<String>)>,
    previews: HashMap<String, u64>,
    manifest: Vec<u8>,
}

impl Default for Workspace {
    fn default() -> Self {
        let mut ws = Self {
            assets: Vec::new(),
            videos: Vec::new(),
            documents: Vec::new(),
            active_document: 0,
            path: None,
            tree_selected: None,
            asset_drag: None,
            asset_drag_origin: None,
            asset_dragging: false,
            thumb_rebuild_at: None,
            thumb_revision: 0,
            thumb_packed_revision: 0,
            thumb_uploaded_revision: 0,
            thumb_atlas: Vec::new(),
            thumb_atlas_size: (0, 0),
            next_id: 1,
            saved: None,
        };
        ws.create_document("Document 1");
        ws
    }
}

impl Workspace {
    pub fn add_video(&mut self, bytes: Vec<u8>) -> Result<usize, String> {
        if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
            return Err("ComfyUI returned invalid MP4 data".into());
        }
        let id = self.alloc_id("video");
        let index = self.videos.len();
        self.videos.push(VideoAsset {
            name: format!("Ltx 2.3 I2V {}.mp4", index + 1),
            id,
            byte_size: bytes.len() as u64,
            thumb: Vec::new(),
            poster_attempted: false,
            bytes: Some(Arc::new(bytes)),
            deferred: None,
        });
        self.invalidate_thumb_atlas();
        Ok(index)
    }

    pub fn set_video_thumb(&mut self, id: &str, thumb: Vec<u8>) -> bool {
        if thumb.len() != (THUMB_SIZE * THUMB_SIZE * 4) as usize {
            return false;
        }
        let Some(video) = self.videos.iter_mut().find(|v| v.id == id) else {
            return false;
        };
        video.thumb = thumb;
        video.poster_attempted = true;
        self.invalidate_thumb_atlas();
        true
    }

    pub fn delete_video(&mut self, index: usize) {
        if index < self.videos.len() {
            self.videos.remove(index);
            self.invalidate_thumb_atlas();
        }
    }

    pub fn ensure_document_loaded(&mut self, index: usize) -> Result<(), String> {
        let doc = self.documents.get_mut(index).ok_or("Document not found")?;
        let Some(deferred) = &doc.deferred else {
            return Ok(());
        };
        // Decode into a temporary vector: a failure must not leave a half-loaded document.
        let pixels = deferred
            .layers
            .iter()
            .map(|meta| {
                let bytes = deferred
                    .source
                    .0
                    .read_all(&meta.path)
                    .map_err(|e| format!("Cannot read layer '{}': {e}", meta.name))?;
                let pixels = decode_layer(&bytes, meta.compression.as_deref())
                    .map_err(|e| format!("Layer '{}': {e}", meta.name))?;
                let drawings = if let Some(path) = &meta.timeline {
                    let bytes = deferred
                        .source
                        .0
                        .read_all(path)
                        .map_err(|e| e.to_string())?;
                    decode_drawings(&bytes)?
                } else {
                    Default::default()
                };
                Ok((pixels, drawings))
            })
            .collect::<Result<Vec<_>, String>>()?;
        for (index, (layer, (pixels, drawings))) in
            doc.project.layers.iter_mut().zip(pixels).enumerate()
        {
            if let Some(saved) = self.saved.as_mut().and_then(|saved| {
                saved
                    .layers
                    .get_mut(&format!("/documents/{}/layers/{index}.tiles", doc.id))
            }) {
                saved.0 = Some(pixels.fingerprint());
            }
            layer.pixels = pixels;
            layer.drawings = drawings;
        }
        doc.deferred = None;
        Ok(())
    }

    pub fn ensure_asset_loaded(&mut self, index: usize) -> Result<(), String> {
        let asset = self.assets.get_mut(index).ok_or("Asset not found")?;
        let Some(deferred) = &asset.deferred else {
            return Ok(());
        };
        let bytes = deferred
            .source
            .0
            .read_all(&deferred.path)
            .map_err(|e| format!("Cannot read asset '{}': {e}", asset.name))?;
        let rgba = image::load_from_memory(&bytes)
            .map_err(|e| format!("Cannot decode asset '{}': {e}", asset.name))?
            .to_rgba8();
        if rgba.dimensions() != (asset.width, asset.height) {
            return Err(format!(
                "Asset '{}' dimensions do not match manifest",
                asset.name
            ));
        }
        if let Some(saved) = self
            .saved
            .as_mut()
            .and_then(|saved| saved.assets.get_mut(&deferred.path))
        {
            saved.2 = Some(crate::safe_save::fingerprint(rgba.as_raw()));
        }
        asset.pixels = rgba.into_raw();
        asset.deferred = None;
        Ok(())
    }

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
        self.select_active_document();
        self.invalidate_thumb_atlas();
        self.active_document
    }

    pub fn select_active_document(&mut self) {
        if let Some(doc) = self.documents.get(self.active_document) {
            self.tree_selected = Some(format!("doc_{}", doc.id));
        }
    }

    /// Insert a copy after `index` and return the new index. The copy is not activated.
    pub fn duplicate_document(&mut self, index: usize) -> Option<usize> {
        let mut doc = self.documents.get(index)?.clone();
        doc.id = self.alloc_id("doc");
        doc.name = format!("{} copy", doc.name);
        let insert_at = index + 1;
        self.documents.insert(insert_at, doc);
        self.invalidate_thumb_atlas();
        Some(insert_at)
    }

    pub fn mark_active_thumb_dirty(&mut self) {
        if let Some(doc) = self.documents.get_mut(self.active_document) {
            doc.mark_thumb_dirty();
        }
        // Debounce: reset 1s timer on every edit.
        self.thumb_rebuild_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(1));
    }

    pub fn rebuild_active_thumb_now(&mut self) {
        let rebuilt = self
            .documents
            .get_mut(self.active_document)
            .is_some_and(|doc| {
                doc.rebuild_thumb();
                true
            });
        if rebuilt {
            self.invalidate_thumb_atlas();
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
        let rebuilt = self
            .documents
            .get_mut(self.active_document)
            .is_some_and(|doc| {
                if !doc.thumb_dirty {
                    return false;
                }
                doc.rebuild_thumb();
                true
            });
        if rebuilt {
            self.invalidate_thumb_atlas();
        }
        self.thumb_rebuild_at = None;
        false
    }

    pub(crate) fn invalidate_thumb_atlas(&mut self) {
        self.thumb_revision = self.thumb_revision.wrapping_add(1);
    }

    /// Pack library thumbnails into one atlas. One quad per thumb, not one rect per pixel.
    pub fn prepare_thumb_atlas(&mut self) {
        if self.thumb_packed_revision == self.thumb_revision {
            return;
        }
        let cols = THUMB_ATLAS_COLS;
        let count = (self.assets.len() + self.documents.len() + self.videos.len()) as u32;
        let rows = count.div_ceil(cols).max(1);
        let width = cols * THUMB_SIZE;
        let height = rows * THUMB_SIZE;
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for (cell, thumb) in self
            .assets
            .iter()
            .map(|asset| &asset.thumb)
            .chain(self.documents.iter().map(|doc| &doc.thumb))
            .chain(self.videos.iter().map(|video| &video.thumb))
            .enumerate()
        {
            let col = (cell as u32) % cols;
            let row = (cell as u32) / cols;
            let row_bytes = (THUMB_SIZE * 4) as usize;
            for y in 0..THUMB_SIZE {
                let src = (y * THUMB_SIZE * 4) as usize;
                let dst = ((row * THUMB_SIZE + y) * width + col * THUMB_SIZE) as usize * 4;
                if src + row_bytes <= thumb.len() {
                    pixels[dst..dst + row_bytes].copy_from_slice(&thumb[src..src + row_bytes]);
                }
            }
        }
        self.thumb_atlas = pixels;
        self.thumb_atlas_size = (width, height);
        self.thumb_packed_revision = self.thumb_revision;
    }

    pub fn thumb_uv(&self, cell: usize) -> ([f32; 2], [f32; 2]) {
        let (width, height) = self.thumb_atlas_size;
        let col = (cell as u32) % THUMB_ATLAS_COLS;
        let row = (cell as u32) / THUMB_ATLAS_COLS;
        let u = |column: u32| column as f32 * THUMB_SIZE as f32 / width.max(1) as f32;
        let v = |row: u32| row as f32 * THUMB_SIZE as f32 / height.max(1) as f32;
        ([u(col), v(row)], [u(col + 1), v(row + 1)])
    }

    pub fn thumb_needs_upload(&self) -> bool {
        self.thumb_packed_revision == self.thumb_revision
            && self.thumb_uploaded_revision != self.thumb_packed_revision
            && self.thumb_atlas_size.0 > 0
    }

    pub fn mark_thumb_atlas_uploaded(&mut self) {
        self.thumb_uploaded_revision = self.thumb_packed_revision;
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

    /// Remove a document and keep it for undo. The last document stays.
    pub fn take_document(&mut self, index: usize) -> Option<Document> {
        if self.documents.len() <= 1 || index >= self.documents.len() {
            return None;
        }
        let doc = self.documents.remove(index);
        if self.active_document >= self.documents.len() {
            self.active_document = self.documents.len() - 1;
        } else if index < self.active_document {
            self.active_document -= 1;
        }
        self.select_active_document();
        self.invalidate_thumb_atlas();
        Some(doc)
    }

    pub fn set_active_document(&mut self, index: usize) {
        if index < self.documents.len() {
            self.active_document = index;
            self.select_active_document();
        }
    }

    pub fn import_asset_image(&mut self, path: &Path) -> Result<usize, String> {
        let bytes = fs::read(path).map_err(|e| format!("Cannot read image: {e}"))?;
        let byte_size = bytes.len() as u64;
        let decoded = image::load_from_memory(&bytes).map_err(|e| format!("Invalid image: {e}"))?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Asset")
            .to_string();
        self.add_asset_image(name, decoded, byte_size)
    }

    pub fn import_asset_rgba(
        &mut self,
        name: String,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> Result<usize, String> {
        let byte_size = pixels.len() as u64;
        let image = RgbaImage::from_raw(width, height, pixels.to_vec())
            .ok_or_else(|| "Invalid image size".to_string())?;
        self.add_asset_image(name, DynamicImage::ImageRgba8(image), byte_size)
    }

    fn add_asset_image(
        &mut self,
        name: String,
        image: DynamicImage,
        byte_size: u64,
    ) -> Result<usize, String> {
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
            byte_size,
            thumb,
            deferred: None,
        });
        self.invalidate_thumb_atlas();
        Ok(self.assets.len() - 1)
    }

    pub fn delete_asset(&mut self, index: usize) {
        if index < self.assets.len() {
            self.assets.remove(index);
            self.invalidate_thumb_atlas();
        }
    }

    /// Place asset pixels as a new layer on the active document (or create canvas).
    pub fn place_asset_on_active(&mut self, asset_index: usize) -> Result<(), String> {
        self.ensure_asset_loaded(asset_index)?;
        self.ensure_document_loaded(self.active_document)?;
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
        for doc in &mut self.documents {
            if doc.deferred.is_none() {
                doc.project.flush_drawings();
            }
        }
        let mut rebuilt = false;
        for doc in &mut self.documents {
            if doc.thumb_dirty && doc.deferred.is_none() {
                doc.rebuild_thumb();
                rebuilt = true;
            }
        }
        if rebuilt {
            self.invalidate_thumb_atlas();
        }
        let incremental = self.path.as_deref() == Some(path) && path.is_file();
        let previous = if incremental {
            self.saved.as_ref()
        } else {
            None
        };
        let mut next = SavedResources::default();
        let mut files = Vec::new();
        let mut asset_metas = Vec::new();
        for asset in &self.assets {
            let asset_path = asset
                .deferred
                .as_ref()
                .map(|source| source.path.clone())
                .unwrap_or_else(|| format!("/assets/{}.webp", asset.id));
            let signature = (
                asset.width,
                asset.height,
                asset
                    .deferred
                    .is_none()
                    .then(|| crate::safe_save::fingerprint(&asset.pixels)),
            );
            if let Some(deferred) = &asset.deferred {
                if !incremental
                    || previous.is_none_or(|saved| !saved.assets.contains_key(&asset_path))
                {
                    let bytes = deferred
                        .source
                        .0
                        .read_all(&deferred.path)
                        .map_err(|e| format!("Cannot copy asset '{}': {e}", asset.name))?;
                    let kind = deferred
                        .source
                        .0
                        .meta(&deferred.path)
                        .map_err(|e| e.to_string())?
                        .kind;
                    files.push(WriteFile::new(asset_path.clone(), kind, bytes));
                }
            } else if previous.and_then(|saved| saved.assets.get(&asset_path)) != Some(&signature) {
                files.push(WriteFile::new(
                    asset_path.clone(),
                    IMAGE_KIND,
                    encode_webp_rgba(&asset.pixels, asset.width, asset.height)?,
                ));
            }
            next.assets.insert(asset_path.clone(), signature);
            let preview = format!("/previews/assets/{}.webp", asset.id);
            save_preview(&preview, &asset.thumb, previous, &mut next, &mut files)?;
            asset_metas.push(AssetMeta {
                id: asset.id.clone(),
                name: asset.name.clone(),
                width: asset.width,
                height: asset.height,
                path: asset_path,
                preview: Some(preview),
            });
        }
        let mut video_metas = Vec::new();
        for video in &self.videos {
            let video_path = format!("/videos/{}.mp4", video.id);
            if previous.is_none_or(|saved| !saved.videos.contains(&video_path)) {
                files.push(WriteFile::new(&video_path, "video/mp4", video.read()?));
            }
            next.videos.insert(video_path.clone());
            let preview = if video.thumb.is_empty() {
                None
            } else {
                let path = format!("/previews/videos/{}.webp", video.id);
                save_preview(&path, &video.thumb, previous, &mut next, &mut files)?;
                Some(path)
            };
            video_metas.push(VideoMeta {
                id: video.id.clone(),
                name: video.name.clone(),
                path: video_path,
                preview,
            });
        }
        let mut doc_metas = Vec::new();
        for doc in &self.documents {
            let mut layers = Vec::new();
            for (i, layer) in doc.project.layers.iter().enumerate() {
                let layer_path = format!("/documents/{}/layers/{i}.tiles", doc.id);
                let fingerprint = doc.deferred.is_none().then(|| layer.pixels.fingerprint());
                let saved_layer = previous.and_then(|saved| saved.layers.get(&layer_path));
                let compression = if let Some(deferred) = &doc.deferred {
                    let meta = &deferred.layers[i];
                    if !incremental || saved_layer.is_none() {
                        let bytes = deferred
                            .source
                            .0
                            .read_all(&meta.path)
                            .map_err(|e| format!("Cannot copy layer '{}': {e}", layer.name))?;
                        files.push(WriteFile::new(
                            layer_path.clone(),
                            "application/x-rgba-tiles-64",
                            bytes,
                        ));
                    }
                    meta.compression.clone()
                } else if let Some((_, compression)) =
                    saved_layer.filter(|(hash, _)| *hash == fingerprint)
                {
                    compression.clone()
                } else {
                    files.push(WriteFile::new(
                        layer_path.clone(),
                        "application/x-rgba-tiles-64",
                        encode_layer(&layer.pixels)?,
                    ));
                    Some("zstd".to_string())
                };
                next.layers
                    .insert(layer_path.clone(), (fingerprint, compression.clone()));
                let timeline = if doc.project.animation.is_some() {
                    let path = format!("/documents/{}/layers/{i}.timeline", doc.id);
                    let bytes = if let Some(deferred) = &doc.deferred {
                        let source_path = deferred.layers[i]
                            .timeline
                            .as_ref()
                            .ok_or("Animation layer has no timeline")?;
                        deferred
                            .source
                            .0
                            .read_all(source_path)
                            .map_err(|e| e.to_string())?
                    } else {
                        encode_drawings(&layer.drawings)?
                    };
                    files.push(WriteFile::new(
                        &path,
                        "application/x-held-drawings-zstd-v1",
                        bytes,
                    ));
                    Some(path)
                } else {
                    None
                };
                layers.push(LayerMeta {
                    timeline,
                    name: layer.name.clone(),
                    visible: layer.visible,
                    opacity: layer.opacity,
                    path: layer_path,
                    compression,
                });
            }
            let preview = format!("/previews/documents/{}.webp", doc.id);
            save_preview(&preview, &doc.thumb, previous, &mut next, &mut files)?;
            doc_metas.push(DocumentMeta {
                kind: if doc.project.animation.is_some() {
                    DocumentKind::Animation
                } else {
                    DocumentKind::Image
                },
                animation: doc.project.animation.clone(),
                id: doc.id.clone(),
                name: doc.name.clone(),
                width: doc.project.width,
                height: doc.project.height,
                active_layer: doc.project.active_layer,
                layers,
                preview: Some(preview),
            });
        }
        let manifest = WorkspaceManifest {
            format: "mega-image-editor".into(),
            version: FORMAT_VERSION,
            active_document: self
                .active_document
                .min(self.documents.len().saturating_sub(1)),
            assets: asset_metas,
            documents: doc_metas,
            videos: video_metas,
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| format!("Cannot encode manifest: {e}"))?;
        if previous.map(|saved| saved.manifest.as_slice()) != Some(manifest_bytes.as_slice()) {
            files.push(WriteFile::new(
                MANIFEST_PATH,
                MANIFEST_KIND,
                manifest_bytes.clone(),
            ));
        }
        if incremental {
            crate::safe_save::append_archive(
                path,
                files,
                previous.map(|saved| saved.manifest.as_slice()),
            )?;
        } else {
            crate::safe_save::save_archive(path, files)?;
        }
        next.manifest = manifest_bytes;
        self.saved = Some(next);
        self.path = Some(path.to_path_buf());
        Ok(())
    }

    /// Compact only the saved revision. Unsaved in-memory edits are not committed.
    pub fn optimize(&mut self) -> Result<(u64, u64), String> {
        let path = self
            .path
            .as_deref()
            .ok_or("Save or open a project before optimizing")?;
        let before = fs::metadata(path)
            .map_err(|e| format!("Cannot inspect project: {e}"))?
            .len();
        // Read previews from the saved revision, never from unsaved memory edits.
        let saved_previews = Self::open(path)?;
        let (files, original_manifest, updated_manifest) = {
            let archive = MegaFile::open(path).map_err(|e| format!("Cannot open project: {e}"))?;
            let manifest_bytes = archive
                .read_all(MANIFEST_PATH)
                .map_err(|e| format!("Cannot read saved manifest: {e}"))?;
            let mut manifest: WorkspaceManifest = serde_json::from_slice(&manifest_bytes)
                .map_err(|e| format!("Invalid saved manifest: {e}"))?;
            if let Err(error) = check_manifest(&manifest) {
                return Err(error);
            }
            let mut paths = HashSet::from([MANIFEST_PATH.to_string()]);
            paths.extend(manifest.videos.iter().map(|v| v.path.clone()));
            paths.extend(manifest.videos.iter().filter_map(|v| v.preview.clone()));
            let mut replacements = HashMap::new();
            for (asset, preview) in manifest.assets.iter_mut().zip(&saved_previews.assets) {
                paths.insert(asset.path.clone());
                if asset.preview.is_none() {
                    let path = format!("/previews/assets/{}.webp", asset.id);
                    replacements.insert(
                        path.clone(),
                        encode_webp_rgba(&preview.thumb, THUMB_SIZE, THUMB_SIZE)?,
                    );
                    asset.preview = Some(path);
                }
                paths.insert(asset.preview.clone().unwrap());
            }
            for (document, preview) in manifest.documents.iter_mut().zip(&saved_previews.documents)
            {
                if document.preview.is_none() {
                    let path = format!("/previews/documents/{}.webp", document.id);
                    replacements.insert(
                        path.clone(),
                        encode_webp_rgba(&preview.thumb, THUMB_SIZE, THUMB_SIZE)?,
                    );
                    document.preview = Some(path);
                }
                paths.insert(document.preview.clone().unwrap());
                for layer in &mut document.layers {
                    if let Some(timeline) = &layer.timeline {
                        paths.insert(timeline.clone());
                    }
                    paths.insert(layer.path.clone());
                    match layer.compression.as_deref().unwrap_or("none") {
                        "zstd" => {}
                        "none" => {
                            let raw = archive.read_all(&layer.path).map_err(|e| {
                                format!("Cannot read saved layer '{}': {e}", layer.name)
                            })?;
                            let pixels = decode_layer(&raw, layer.compression.as_deref())?;
                            replacements.insert(layer.path.clone(), encode_layer(&pixels)?);
                            layer.compression = Some("zstd".into());
                        }
                        codec => return Err(format!("Unsupported layer compression: {codec}")),
                    }
                }
            }
            let updated_manifest = if replacements.is_empty() {
                manifest_bytes.clone()
            } else {
                serde_json::to_vec_pretty(&manifest)
                    .map_err(|e| format!("Cannot encode manifest: {e}"))?
            };
            replacements.insert(MANIFEST_PATH.to_string(), updated_manifest.clone());
            let mut paths: Vec<_> = paths.into_iter().collect();
            paths.sort();
            let mut files = Vec::new();
            for path in paths {
                let metadata = archive.meta(&path).ok();
                let bytes = match replacements.remove(&path) {
                    Some(bytes) => bytes,
                    None => archive
                        .read_all(&path)
                        .map_err(|e| format!("Cannot read saved resource {path}: {e}"))?,
                };
                let mut file = WriteFile::new(
                    path,
                    metadata
                        .as_ref()
                        .map(|meta| meta.kind.as_str())
                        .unwrap_or(IMAGE_KIND),
                    bytes,
                );
                if let Some(metadata) = metadata {
                    file.created_at = Some(metadata.created_at);
                    file.updated_at = Some(metadata.updated_at);
                    file.flags = metadata.flags;
                }
                files.push(file);
            }
            (files, manifest_bytes, updated_manifest)
        };
        crate::safe_save::save_archive(path, files)?;
        let after = fs::metadata(path)
            .map_err(|e| format!("Cannot inspect optimized project: {e}"))?
            .len();
        // Update only the disk snapshot, never fingerprints of unsaved memory edits.
        let same_revision = self
            .saved
            .as_ref()
            .is_some_and(|saved| saved.manifest == original_manifest);
        if let Some(saved) = self
            .saved
            .as_mut()
            .filter(|saved| saved.manifest == original_manifest)
        {
            saved.manifest = updated_manifest.clone();
            for (_, codec) in saved.layers.values_mut() {
                *codec = Some("zstd".into());
            }
            for asset in &saved_previews.assets {
                saved.previews.insert(
                    format!("/previews/assets/{}.webp", asset.id),
                    crate::safe_save::fingerprint(&asset.thumb),
                );
            }
            for doc in &saved_previews.documents {
                saved.previews.insert(
                    format!("/previews/documents/{}.webp", doc.id),
                    crate::safe_save::fingerprint(&doc.thumb),
                );
            }
        } else {
            self.saved = None;
        }
        if same_revision {
            let manifest: WorkspaceManifest =
                serde_json::from_slice(&updated_manifest).map_err(|e| e.to_string())?;
            let source = ArchiveSource(Arc::new(MegaFile::open(path).map_err(|e| e.to_string())?));
            for doc in &mut self.documents {
                if doc.deferred.is_some()
                    && let Some(meta) = manifest.documents.iter().find(|meta| meta.id == doc.id)
                {
                    doc.deferred = Some(DeferredDocument {
                        source: source.clone(),
                        layers: meta.layers.clone(),
                    });
                }
            }
            for video in &mut self.videos {
                if video.deferred.is_some()
                    && let Some(meta) = manifest.videos.iter().find(|meta| meta.id == video.id)
                {
                    video.deferred = Some(DeferredAsset {
                        source: source.clone(),
                        path: meta.path.clone(),
                    });
                }
            }
            for asset in &mut self.assets {
                if asset.deferred.is_some()
                    && let Some(meta) = manifest.assets.iter().find(|meta| meta.id == asset.id)
                {
                    asset.deferred = Some(DeferredAsset {
                        source: source.clone(),
                        path: meta.path.clone(),
                    });
                }
            }
        }
        Ok((before, after))
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let archive = MegaFile::open(path).map_err(|e| format!("Cannot open project: {e}"))?;
        let manifest_bytes = archive
            .read_all(MANIFEST_PATH)
            .map_err(|e| format!("Project has no valid manifest: {e}"))?;
        let value: serde_json::Value = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| format!("Invalid project manifest: {e}"))?;
        let manifest: WorkspaceManifest =
            serde_json::from_value(value).map_err(|e| format!("Invalid project manifest: {e}"))?;
        check_manifest(&manifest)?;
        let source = ArchiveSource(Arc::new(archive));
        let mut next_id = 1u64;
        let mut assets = Vec::new();
        for meta in &manifest.assets {
            let byte_size = source
                .0
                .meta(&meta.path)
                .map_err(|e| format!("Asset '{}' missing: {e}", meta.name))?;
            let thumb = match read_preview(&source, meta.preview.as_deref())? {
                Some(thumb) => thumb,
                None => {
                    // Legacy projects have no stored preview. Generate it once,
                    // retaining only the small thumbnail, not the decoded asset.
                    let bytes = source.0.read_all(&meta.path).map_err(|e| e.to_string())?;
                    let rgba = image::load_from_memory(&bytes)
                        .map_err(|e| e.to_string())?
                        .to_rgba8();
                    make_thumb(rgba.as_raw(), rgba.width(), rgba.height())
                }
            };
            bump_id(&meta.id, &mut next_id);
            assets.push(ImageAsset {
                id: meta.id.clone(),
                name: meta.name.clone(),
                width: meta.width,
                height: meta.height,
                pixels: Vec::new(),
                byte_size: byte_size.size,
                thumb,
                deferred: Some(DeferredAsset {
                    source: source.clone(),
                    path: meta.path.clone(),
                }),
            });
        }
        let mut documents = Vec::new();
        for meta in &manifest.documents {
            bump_id(&meta.id, &mut next_id);
            let mut layers = Vec::new();
            let width = meta.width;
            let height = meta.height;
            for layer_meta in &meta.layers {
                layers.push(Layer {
                    drawings: Default::default(),
                    name: layer_meta.name.clone(),
                    visible: layer_meta.visible,
                    opacity: layer_meta.opacity.clamp(0.0, 1.0),
                    pixels: SparsePixels::default(),
                });
            }
            let mut project = Project {
                animation: meta.animation.clone(),
                width,
                height,
                layers,
                active_layer: meta.active_layer,
            };
            if !project.layers.is_empty() {
                project.active_layer = project.active_layer.min(project.layers.len() - 1);
            }
            let thumb = match read_preview(&source, meta.preview.as_deref())? {
                Some(thumb) => thumb,
                None => {
                    let mut legacy = project.clone();
                    for (layer, layer_meta) in legacy.layers.iter_mut().zip(&meta.layers) {
                        let bytes = source
                            .0
                            .read_all(&layer_meta.path)
                            .map_err(|e| e.to_string())?;
                        layer.pixels = decode_layer(&bytes, layer_meta.compression.as_deref())?;
                    }
                    let doc = Document::new(meta.id.clone(), meta.name.clone(), legacy);
                    doc.thumb
                }
            };
            documents.push(Document {
                id: meta.id.clone(),
                name: meta.name.clone(),
                project,
                thumb,
                thumb_dirty: false,
                deferred: Some(DeferredDocument {
                    source: source.clone(),
                    layers: meta.layers.clone(),
                }),
            });
        }
        if documents.is_empty() {
            documents.push(Document::new(
                "doc_1".into(),
                "Document 1".into(),
                Project::default(),
            ));
        }
        let active_document = manifest.active_document.min(documents.len() - 1);
        let tree_selected = documents
            .get(active_document)
            .map(|doc| format!("doc_{}", doc.id));
        let videos = manifest
            .videos
            .iter()
            .map(|meta| {
                bump_id(&meta.id, &mut next_id);
                let byte_size = source
                    .0
                    .meta(&meta.path)
                    .map_err(|e| format!("Cannot read video metadata: {e}"))?
                    .size;
                let thumb = read_preview(&source, meta.preview.as_deref())?.unwrap_or_default();
                let poster_attempted = !thumb.is_empty();
                Ok(VideoAsset {
                    id: meta.id.clone(),
                    name: meta.name.clone(),
                    byte_size,
                    thumb,
                    poster_attempted,
                    bytes: None,
                    deferred: Some(DeferredAsset {
                        source: source.clone(),
                        path: meta.path.clone(),
                    }),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut workspace = Self {
            assets,
            videos,
            active_document,
            documents,
            path: Some(path.to_path_buf()),
            tree_selected,
            asset_drag: None,
            asset_drag_origin: None,
            asset_dragging: false,
            thumb_rebuild_at: None,
            thumb_revision: 1,
            thumb_packed_revision: 0,
            thumb_uploaded_revision: 0,
            thumb_atlas: Vec::new(),
            thumb_atlas_size: (0, 0),
            next_id: next_id.max(1),
            saved: None,
        };
        let mut saved = SavedResources {
            manifest: manifest_bytes,
            ..Default::default()
        };
        saved
            .videos
            .extend(manifest.videos.iter().map(|v| v.path.clone()));
        for (video, meta) in workspace.videos.iter().zip(&manifest.videos) {
            if let Some(preview) = &meta.preview {
                saved
                    .previews
                    .insert(preview.clone(), crate::safe_save::fingerprint(&video.thumb));
            }
        }
        for (asset, meta) in workspace.assets.iter().zip(&manifest.assets) {
            saved
                .assets
                .insert(meta.path.clone(), (asset.width, asset.height, None));
            if let Some(preview) = &meta.preview {
                saved
                    .previews
                    .insert(preview.clone(), crate::safe_save::fingerprint(&asset.thumb));
            }
        }
        for (document, meta) in workspace.documents.iter().zip(&manifest.documents) {
            for index in 0..document.project.layers.len() {
                saved.layers.insert(
                    format!("/documents/{}/layers/{index}.tiles", document.id),
                    (None, meta.layers[index].compression.clone()),
                );
            }
            if let Some(preview) = &meta.preview {
                saved.previews.insert(
                    preview.clone(),
                    crate::safe_save::fingerprint(&document.thumb),
                );
            }
        }
        workspace.saved = Some(saved);
        Ok(workspace)
    }
}

fn check_manifest(manifest: &WorkspaceManifest) -> Result<(), String> {
    if manifest.format == "mega-image-editor" && matches!(manifest.version, 5 | 6 | 7 | 8) {
        for doc in &manifest.documents {
            if (doc.kind == DocumentKind::Animation) != doc.animation.is_some() {
                return Err("Document type does not match its contents".into());
            }
            if let Some(animation) = &doc.animation {
                let (play_start, play_end) = animation.playback_range();
                if !(1..=120).contains(&animation.fps)
                    || animation.length == 0
                    || animation.frame >= animation.length
                    || play_start > play_end
                    || play_end >= animation.length
                    || doc.layers.iter().any(|l| l.timeline.is_none())
                {
                    return Err("Invalid animation settings".into());
                }
            }
        }
        Ok(())
    } else {
        Err("Unsupported project format or version".into())
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

    #[test]
    fn animation_survives_save_deferred_save_as_reorder_and_optimization() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("animation.mgimg");
        let copy = temp.path().join("copy.mgimg");
        let mut workspace = Workspace::default();
        let project = workspace.active_mut();
        project.resize_canvas(8, 8).unwrap();
        project.animation = Some(crate::project::Animation::new(12, 24));
        project.layers[0].pixels.write(0, 0, 0, [10, 0, 0, 255]);
        project.seek(7);
        project.new_drawing(true);
        project.layers[0].pixels.write(0, 0, 0, [20, 0, 0, 255]);
        project.add_layer();
        project.layers[1].pixels.write(0, 1, 0, [30, 0, 0, 255]);
        workspace.save(&path).unwrap();
        let archive = MegaFile::open(&path).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&archive.read_all(MANIFEST_PATH).unwrap()).unwrap();
        assert_eq!(manifest["documents"][0]["type"], "animation");
        drop(archive);
        let mut deferred = Workspace::open(&path).unwrap();
        assert!(deferred.documents[0].deferred.is_some());
        deferred.save(&copy).unwrap();
        deferred.optimize().unwrap();
        deferred.ensure_document_loaded(0).unwrap();
        assert_eq!(deferred.active().animation.as_ref().unwrap().frame, 7);
        assert_eq!(deferred.active().layers[0].pixels.read(0, 0, 0)[0], 20);
        deferred.active_mut().move_layer_to(0, 1);
        deferred.active_mut().seek(0);
        assert_eq!(deferred.active().layers[1].pixels.read(0, 0, 0)[0], 10);
        assert_eq!(deferred.active().layers[0].pixels.read(0, 1, 0)[0], 30);
        deferred.save(&copy).unwrap();
        deferred.optimize().unwrap();
        let mut loaded = Workspace::open(&copy).unwrap();
        loaded.ensure_document_loaded(0).unwrap();
        loaded.active_mut().seek(10);
        assert_eq!(loaded.active().layers[1].pixels.read(0, 0, 0)[0], 20);
        assert_eq!(loaded.active().animation.as_ref().unwrap().fps, 12);
    }

    // Existing raster round-trip tests explicitly request the content they inspect.
    fn open_loaded(path: &Path) -> Result<Workspace, String> {
        let mut workspace = Workspace::open(path)?;
        for index in 0..workspace.documents.len() {
            workspace.ensure_document_loaded(index)?;
        }
        for index in 0..workspace.assets.len() {
            workspace.ensure_asset_loaded(index)?;
        }
        Ok(workspace)
    }

    #[test]
    fn video_reader_reads_bounded_chunks_and_seeks_in_memory_and_archive() {
        use std::io::{Read, Seek, SeekFrom};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stream.mgimg");
        let mut workspace = Workspace::default();
        let mut bytes = b"\0\0\0\x18ftypisom".to_vec();
        bytes.extend((0..200_000).map(|i| (i % 251) as u8));
        workspace.add_video(bytes.clone()).unwrap();
        workspace.save(&path).unwrap();
        let loaded = Workspace::open(&path).unwrap();
        assert!(loaded.videos[0].bytes.is_none());
        for asset in [&workspace.videos[0], &loaded.videos[0]] {
            let mut reader = asset.reader();
            let mut buffer = vec![0; 128 * 1024];
            assert_eq!(reader.read(&mut buffer).unwrap(), 64 * 1024);
            assert_eq!(&buffer[..64 * 1024], &bytes[..64 * 1024]);
            reader.seek(SeekFrom::End(-5)).unwrap();
            let mut tail = Vec::new();
            reader.read_to_end(&mut tail).unwrap();
            assert_eq!(tail, bytes[bytes.len() - 5..]);
            assert!(reader.seek(SeekFrom::Start(0)).is_ok());
            assert!(reader.seek(SeekFrom::Current(-1)).is_err());
            let mut header = [0; 12];
            reader.read_exact(&mut header).unwrap();
            assert_eq!(header, bytes[..12]);
        }
    }

    #[test]
    fn videos_survive_save_as_lazy_open_and_optimization() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("videos.mgimg");
        let copy = dir.path().join("copy.mgimg");
        let bytes = b"\0\0\0\x18ftypisomtest video bytes".to_vec();
        let mut workspace = Workspace::default();
        assert!(workspace.add_video(vec![0; 20]).is_err());
        workspace.add_video(bytes.clone()).unwrap();
        let poster = vec![127; (THUMB_SIZE * THUMB_SIZE * 4) as usize];
        let id = workspace.videos[0].id.clone();
        assert!(workspace.set_video_thumb(&id, poster.clone()));
        workspace.save(&path).unwrap();
        let archive = MegaFile::open(&path).unwrap();
        assert_eq!(archive.read_all("/videos/video_2.mp4").unwrap(), bytes);
        let mut loaded = Workspace::open(&path).unwrap();
        assert!(loaded.videos[0].bytes.is_none());
        assert_eq!(loaded.videos[0].thumb, poster);
        assert!(loaded.videos[0].poster_attempted);
        let before = fs::read(&path).unwrap();
        loaded.save(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        loaded.optimize().unwrap();
        assert_eq!(loaded.videos[0].read().unwrap(), bytes);
        loaded.save(&copy).unwrap();
        let mut copied = Workspace::open(&copy).unwrap();
        assert_eq!(copied.videos[0].read().unwrap(), bytes);
        assert_eq!(copied.videos[0].thumb, poster);
        copied.videos.clear();
        copied.save(&copy).unwrap();
        copied.optimize().unwrap();
        assert!(Workspace::open(&copy).unwrap().videos.is_empty());
        assert!(
            MegaFile::open(&copy)
                .unwrap()
                .read_all("/videos/video_2.mp4")
                .is_err()
        );
    }

    fn lazy_fixture(path: &Path) -> Workspace {
        let mut workspace = saved_workspace(path);
        workspace
            .import_asset_rgba("second".into(), &[90, 80, 70, 128].repeat(64), 8, 8)
            .unwrap();
        workspace.create_document("Second");
        workspace.place_asset_on_active(1).unwrap();
        workspace.save(path).unwrap();
        workspace
    }

    #[test]
    fn open_reads_lossless_previews_and_only_requested_content_is_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lazy.mgimg");
        let original = lazy_fixture(&path);
        let mut workspace = Workspace::open(&path).unwrap();
        for (a, b) in workspace.assets.iter().zip(&original.assets) {
            assert_eq!(a.thumb, b.thumb);
            assert!(a.pixels.is_empty());
            assert!(a.deferred.is_some());
        }
        for (a, b) in workspace.documents.iter().zip(&original.documents) {
            assert_eq!(a.thumb, b.thumb);
            assert_eq!(a.memory_bytes(), 0);
            assert!(a.deferred.is_some());
        }
        let archive = MegaFile::open(&path).unwrap();
        let manifest: WorkspaceManifest =
            serde_json::from_slice(&archive.read_all(MANIFEST_PATH).unwrap()).unwrap();
        for preview in manifest
            .assets
            .iter()
            .filter_map(|a| a.preview.as_ref())
            .chain(manifest.documents.iter().filter_map(|d| d.preview.as_ref()))
        {
            let bytes = archive.read_all(preview).unwrap();
            assert_eq!(&bytes[8..12], b"WEBP");
            assert_eq!(&bytes[12..16], b"VP8L");
        }
        drop(archive);
        let saved_bytes = fs::read(&path).unwrap();
        workspace.save(&path).unwrap();
        assert!(fs::read(&path).unwrap() == saved_bytes);
        workspace.ensure_document_loaded(1).unwrap();
        assert_eq!(
            workspace.documents[1].project.layers[0].pixels,
            original.documents[1].project.layers[0].pixels
        );
        assert!(workspace.documents[0].deferred.is_some());
        assert!(workspace.assets.iter().all(|a| a.deferred.is_some()));
        workspace.ensure_asset_loaded(0).unwrap();
        assert_eq!(workspace.assets[0].pixels, original.assets[0].pixels);
        assert!(workspace.assets[1].deferred.is_some());
        // Loading by itself must not make any data dirty.
        workspace.save(&path).unwrap();
        assert!(fs::read(&path).unwrap() == saved_bytes);
    }

    #[test]
    fn deferred_content_survives_save_copy_compaction_and_deleted_document_undo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lazy.mgimg");
        let copy = dir.path().join("copy.mgimg");
        let original = lazy_fixture(&path);
        let mut workspace = Workspace::open(&path).unwrap();
        let duplicated = workspace.duplicate_document(0).unwrap();
        let removed = workspace.take_document(duplicated).unwrap();
        workspace.save(&path).unwrap();
        workspace.optimize().unwrap();
        assert!(workspace.documents.iter().all(|d| d.deferred.is_some()));
        assert!(workspace.assets.iter().all(|a| a.deferred.is_some()));
        workspace.save(&copy).unwrap();
        workspace.optimize().unwrap();
        workspace.documents.insert(1, removed);
        workspace.ensure_document_loaded(1).unwrap();
        assert_eq!(
            workspace.documents[1].project.layers[0].pixels,
            original.documents[0].project.layers[0].pixels
        );
        workspace.ensure_document_loaded(0).unwrap();
        workspace.active_document = 0;
        workspace.active_mut().layers[0]
            .pixels
            .write(0, -20, 10, [4, 3, 2, 255]);
        workspace.mark_active_thumb_dirty();
        workspace.save(&copy).unwrap();
        let loaded = open_loaded(&copy).unwrap();
        assert_eq!(
            loaded.documents[0].project.layers[0]
                .pixels
                .read(0, -20, 10),
            [4, 3, 2, 255]
        );
        assert_eq!(
            loaded.documents[2].project.layers[0].pixels,
            original.documents[1].project.layers[0].pixels
        );
        assert_eq!(loaded.assets[1].pixels, original.assets[1].pixels);
        assert!(workspace.documents[2].deferred.is_some());
        assert!(workspace.assets[1].deferred.is_some());
    }

    #[test]
    fn invalid_content_is_not_decoded_until_requested_and_failed_load_is_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.mgimg");
        lazy_fixture(&path);
        let archive = MegaFile::open(&path).unwrap();
        let manifest: WorkspaceManifest =
            serde_json::from_slice(&archive.read_all(MANIFEST_PATH).unwrap()).unwrap();
        archive
            .write_files(vec![
                WriteFile::new(
                    &manifest.documents[1].layers[0].path,
                    "application/x-rgba-tiles-64",
                    b"invalid zstd".to_vec(),
                )
                .overwrite(true),
                WriteFile::new(
                    &manifest.assets[1].path,
                    IMAGE_KIND,
                    b"invalid webp".to_vec(),
                )
                .overwrite(true),
            ])
            .unwrap();
        drop(archive);
        let mut workspace = Workspace::open(&path).unwrap();
        workspace.ensure_document_loaded(0).unwrap();
        assert!(workspace.ensure_document_loaded(1).is_err());
        assert!(workspace.documents[1].deferred.is_some());
        assert_eq!(workspace.documents[1].memory_bytes(), 0);
        assert!(workspace.ensure_asset_loaded(1).is_err());
        assert!(workspace.assets[1].deferred.is_some());
        assert!(workspace.assets[1].pixels.is_empty());
    }

    #[test]
    fn save_preserves_unchanged_raw_layers_and_optimize_compresses_them() {
        let dir = tempfile::tempdir().unwrap();
        for version in [5, 6] {
            let path = dir.path().join(format!("legacy-{version}.mgimg"));
            let mut pixels = SparsePixels::default();
            pixels.offset = (-10, 20);
            pixels.write(0, -15, 200, [1, 2, 3, 255]);
            let raw = pixels.encode();
            let manifest = serde_json::json!({
                "format": "mega-image-editor", "version": version,
                "documents": [{"id": "doc_1", "name": "Legacy", "width": 16,
                    "height": 16, "active_layer": 0, "layers": [
                        {"name": "implicit raw", "visible": true, "opacity": 0.5,
                         "path": "/documents/doc_1/layers/0.tiles"},
                        {"name": "explicit raw", "visible": true, "compression": "none",
                         "path": "/documents/doc_1/layers/1.tiles"}
                    ]}]
            });
            crate::safe_save::save_archive(
                &path,
                vec![
                    WriteFile::new(
                        MANIFEST_PATH,
                        MANIFEST_KIND,
                        serde_json::to_vec(&manifest).unwrap(),
                    ),
                    WriteFile::new(
                        "/documents/doc_1/layers/0.tiles",
                        "application/x-rgba-tiles-64",
                        raw.clone(),
                    ),
                    WriteFile::new(
                        "/documents/doc_1/layers/1.tiles",
                        "application/x-rgba-tiles-64",
                        raw.clone(),
                    ),
                ],
            )
            .unwrap();
            let optimize_path = dir.path().join(format!("optimize-legacy-{version}.mgimg"));
            fs::copy(&path, &optimize_path).unwrap();
            let mut optimized = open_loaded(&optimize_path).unwrap();
            optimized.active_mut().layers[0]
                .pixels
                .write(0, 1, 1, [7, 6, 5, 255]);
            let (before, after) = optimized.optimize().unwrap();
            assert!(after < before, "Optimize must compress existing raw layers");
            assert_eq!(
                open_loaded(&optimize_path).unwrap().active().layers[0].pixels,
                pixels
            );
            assert_eq!(
                optimized.active().layers[0].pixels.read(0, 1, 1),
                [7, 6, 5, 255]
            );
            let archive = MegaFile::open(&optimize_path).unwrap();
            let stored: WorkspaceManifest =
                serde_json::from_slice(&archive.read_all(MANIFEST_PATH).unwrap()).unwrap();
            assert!(
                stored.documents[0]
                    .layers
                    .iter()
                    .all(|layer| layer.compression.as_deref() == Some("zstd"))
            );
            drop(archive);
            optimized.save(&optimize_path).unwrap();
            let reopened = open_loaded(&optimize_path).unwrap();
            assert_eq!(
                reopened.active().layers[0].pixels.read(0, 1, 1),
                [7, 6, 5, 255]
            );
            assert_eq!(reopened.active().layers[1].pixels, pixels);
            let saved_bytes = fs::read(&optimize_path).unwrap();
            optimized.save(&optimize_path).unwrap();
            assert_eq!(fs::read(&optimize_path).unwrap(), saved_bytes);

            let mut workspace = open_loaded(&path).unwrap();
            assert_eq!(workspace.active().layers[0].pixels, pixels);
            assert_eq!(workspace.active().layers[0].opacity, 0.5);
            // The legacy manifest may be normalized/upgraded on the first save,
            // but neither unchanged raster payload should be rewritten.
            let original = MegaFile::open(&path).unwrap();
            let raw_meta = original.meta("/documents/doc_1/layers/0.tiles").unwrap();
            drop(original);
            workspace.save(&path).unwrap();
            let archive = MegaFile::open(&path).unwrap();
            assert_eq!(
                archive
                    .meta("/documents/doc_1/layers/0.tiles")
                    .unwrap()
                    .updated_at,
                raw_meta.updated_at
            );
            assert_eq!(
                archive.read_all("/documents/doc_1/layers/0.tiles").unwrap(),
                raw
            );
            drop(archive);
            let saved_bytes = fs::read(&path).unwrap();
            workspace.save(&path).unwrap();
            assert!(fs::read(&path).unwrap() == saved_bytes);
            workspace.active_mut().layers[0].name = "renamed".into();
            workspace.save(&path).unwrap();
            let archive = MegaFile::open(&path).unwrap();
            assert_eq!(
                archive.read_all("/documents/doc_1/layers/0.tiles").unwrap(),
                raw
            );
            let stored: WorkspaceManifest =
                serde_json::from_slice(&archive.read_all(MANIFEST_PATH).unwrap()).unwrap();
            assert_eq!(stored.documents[0].layers[0].compression, None);
            assert_eq!(
                stored.documents[0].layers[1].compression.as_deref(),
                Some("none")
            );
            drop(archive);

            workspace.active_mut().layers[1]
                .pixels
                .write(0, 0, 0, [9, 8, 7, 128]);
            workspace.save(&path).unwrap();
            let mut mixed = open_loaded(&path).unwrap();
            assert_eq!(mixed.active().layers[0].pixels, pixels);
            assert_eq!(
                mixed.active().layers[1].pixels.read(0, 0, 0),
                [9, 8, 7, 128]
            );
            let archive = MegaFile::open(&path).unwrap();
            let stored: WorkspaceManifest =
                serde_json::from_slice(&archive.read_all(MANIFEST_PATH).unwrap()).unwrap();
            assert_eq!(stored.documents[0].layers[0].compression, None);
            assert_eq!(
                stored.documents[0].layers[1].compression.as_deref(),
                Some("zstd")
            );
            drop(archive);
            mixed.optimize().unwrap();
            let optimized = open_loaded(&path).unwrap();
            for (a, b) in optimized.active().layers.iter().zip(&mixed.active().layers) {
                assert_eq!(a.pixels, b.pixels);
            }
            let compacted = fs::read(&path).unwrap();
            mixed.save(&path).unwrap();
            assert_eq!(fs::read(&path).unwrap(), compacted);

            let copy = dir.path().join(format!("copy-{version}.mgimg"));
            mixed.save(&copy).unwrap();
            let loaded = open_loaded(&copy).unwrap();
            for (a, b) in loaded.active().layers.iter().zip(&mixed.active().layers) {
                assert_eq!(a.pixels, b.pixels);
            }
            let archive = MegaFile::open(&copy).unwrap();
            let stored: WorkspaceManifest =
                serde_json::from_slice(&archive.read_all(MANIFEST_PATH).unwrap()).unwrap();
            assert!(
                stored.documents[0]
                    .layers
                    .iter()
                    .all(|layer| layer.compression.as_deref() == Some("zstd"))
            );
        }
    }

    #[test]
    fn layer_compression_round_trip_and_sizes() {
        let mut noise_seed = 42u32;
        for kind in ["solid", "gradient", "noise"] {
            let mut rgba = Vec::with_capacity(512 * 512 * 4);
            for y in 0..512 {
                for x in 0..512 {
                    let pixel = match kind {
                        "solid" => [40, 80, 120, 255],
                        "gradient" => [(x / 2) as u8, (y / 2) as u8, ((x + y) / 4) as u8, 255],
                        _ => {
                            noise_seed ^= noise_seed << 13;
                            noise_seed ^= noise_seed >> 17;
                            noise_seed ^= noise_seed << 5;
                            noise_seed.to_le_bytes()
                        }
                    };
                    rgba.extend_from_slice(&pixel);
                }
            }
            let pixels = SparsePixels::from_rgba(&rgba, 512, 512);
            let raw = pixels.encode();
            let compressed = encode_layer(&pixels).unwrap();
            assert_eq!(decode_layer(&compressed, Some("zstd")).unwrap(), pixels);
            println!(
                "{kind}: raw={} zstd={} savings={:.2}%",
                raw.len(),
                compressed.len(),
                100.0 * (1.0 - compressed.len() as f64 / raw.len() as f64)
            );
        }
        let empty = SparsePixels::default();
        assert_eq!(
            decode_layer(&encode_layer(&empty).unwrap(), Some("zstd")).unwrap(),
            empty
        );
    }

    #[test]
    fn invalid_layer_compression_is_an_error() {
        assert!(
            decode_layer(&[], Some("future-codec"))
                .unwrap_err()
                .contains("future-codec")
        );
        assert!(decode_layer(b"not zstd", Some("zstd")).is_err());
        let compressed = encode_layer(&SparsePixels::default()).unwrap();
        assert!(decode_layer(&compressed[..compressed.len() - 1], Some("zstd")).is_err());
        let invalid_tiles = zstd::stream::encode_all(&b"invalid tiles"[..], 3).unwrap();
        assert!(decode_layer(&invalid_tiles, Some("zstd")).is_err());
    }

    fn saved_workspace(path: &Path) -> Workspace {
        let mut workspace = Workspace::default();
        workspace
            .import_asset_rgba("reference".into(), &[10, 20, 30, 255].repeat(16), 4, 4)
            .unwrap();
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
        let mut reopened = open_loaded(&path).unwrap();
        reopened.save(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);

        reopened.active_mut().layers[0].name = "renamed".into();
        reopened.save(&path).unwrap();
        let renamed = fs::read(&path).unwrap();
        assert!(renamed.starts_with(&original));
        assert!(renamed.len() - original.len() < crate::raster::TILE_BYTES);
        assert_eq!(
            open_loaded(&path).unwrap().active().layers[0].name,
            "renamed"
        );

        reopened.active_mut().layers[0]
            .pixels
            .write(0, 2, 1, [100, 120, 140, 128]);
        reopened.save(&path).unwrap();
        let edited = fs::read(&path).unwrap();
        assert!(edited.starts_with(&renamed));
        assert_eq!(
            open_loaded(&path).unwrap().active().layers[0]
                .pixels
                .read(0, 2, 1),
            [100, 120, 140, 128]
        );
    }

    #[test]
    fn incremental_save_keeps_resource_paths_correct_after_reorder_and_removal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layers.mgimg");
        let mut workspace = saved_workspace(&path);
        workspace.active_mut().add_layer();
        workspace.active_mut().layers[1]
            .pixels
            .write(0, -20, 30, [80, 90, 100, 255]);
        workspace.save(&path).unwrap();
        workspace.active_mut().move_layer_to(1, 0);
        workspace.save(&path).unwrap();
        let loaded = open_loaded(&path).unwrap();
        assert_eq!(
            loaded.active().layers[0].pixels.read(0, -20, 30),
            [80, 90, 100, 255]
        );
        assert_eq!(
            loaded.active().layers[1].pixels.read(0, 0, 0),
            [10, 20, 30, 255]
        );
        workspace.active_mut().delete_layer(0);
        workspace.delete_asset(0);
        workspace.save(&path).unwrap();
        let loaded = open_loaded(&path).unwrap();
        assert_eq!(loaded.assets.len(), 0);
        assert_eq!(loaded.active().layers.len(), 1);
        assert_eq!(
            loaded.active().layers[0].pixels.read(0, 0, 0),
            [10, 20, 30, 255]
        );
    }

    #[test]
    fn interrupted_incremental_commit_reopens_previous_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.mgimg");
        let mut workspace = saved_workspace(&path);
        let original = fs::read(&path).unwrap();
        workspace.active_mut().layers[0]
            .pixels
            .write(0, 0, 0, [200, 201, 202, 255]);
        workspace.active_mut().layers[0].name = "changed".into();
        workspace.save(&path).unwrap();
        let committed = fs::read(&path).unwrap();
        assert!(committed.starts_with(&original));
        for cut in [original.len(), original.len() + 1, committed.len() - 1] {
            let interrupted = dir.path().join(format!("interrupted-{cut}.mgimg"));
            fs::write(&interrupted, &committed[..cut]).unwrap();
            let loaded = open_loaded(&interrupted).unwrap();
            assert_eq!(
                loaded.active().layers[0].pixels.read(0, 0, 0),
                [10, 20, 30, 255]
            );
            assert_ne!(loaded.active().layers[0].name, "changed");
        }
    }

    #[test]
    fn optimize_reclaims_removed_resources_and_does_not_save_memory_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("optimize.mgimg");
        let mut workspace = saved_workspace(&path);
        workspace.active_mut().add_layer();
        workspace.active_mut().layers[1]
            .pixels
            .write(0, 500, -500, [50, 60, 70, 255]);
        workspace.save(&path).unwrap();
        workspace.active_mut().delete_layer(1);
        workspace.delete_asset(0);
        workspace.save(&path).unwrap();
        let saved_manifest = workspace.saved.as_ref().unwrap().manifest.clone();
        workspace.active_mut().layers[0]
            .pixels
            .write(0, 0, 0, [200, 201, 202, 255]);
        let (before, after) = workspace.optimize().unwrap();
        assert!(after < before);
        assert_eq!(
            workspace.active().layers[0].pixels.read(0, 0, 0),
            [200, 201, 202, 255]
        );
        assert_eq!(workspace.saved.as_ref().unwrap().manifest, saved_manifest);
        let loaded = open_loaded(&path).unwrap();
        assert_eq!(
            loaded.active().layers[0].pixels.read(0, 0, 0),
            [10, 20, 30, 255]
        );
        let archive = MegaFile::open(&path).unwrap();
        assert_eq!(archive.list_all().unwrap().len(), 3); // Manifest, one live layer, document preview.
        drop(archive);
        workspace.save(&path).unwrap();
        assert_eq!(
            open_loaded(&path).unwrap().active().layers[0]
                .pixels
                .read(0, 0, 0),
            [200, 201, 202, 255]
        );
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
        workspace.active_mut().layers[0]
            .pixels
            .write(0, 0, 0, [200, 201, 202, 255]);
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(workspace.save(&path).is_err());
        assert!(workspace.optimize().is_err());
        drop(lock);
        assert_eq!(fs::read(&path).unwrap(), original);
        let saved_fp = workspace
            .saved
            .as_ref()
            .unwrap()
            .layers
            .values()
            .next()
            .unwrap()
            .0;
        assert_ne!(
            saved_fp,
            Some(workspace.active().layers[0].pixels.fingerprint())
        );
        workspace.save(&path).unwrap();
        assert_eq!(
            open_loaded(&path).unwrap().active().layers[0]
                .pixels
                .read(0, 0, 0),
            [200, 201, 202, 255]
        );
    }

    #[test]
    fn asset_byte_size_is_the_file_length_and_documents_report_tile_memory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dot.png");
        RgbaImage::from_raw(1, 1, vec![9, 8, 7, 255])
            .unwrap()
            .save(&path)
            .unwrap();
        let len = fs::metadata(&path).unwrap().len();
        let mut ws = Workspace::default();
        ws.import_asset_image(&path).unwrap();
        assert_eq!(ws.assets[0].byte_size, len);
        assert_eq!(ws.active_doc().memory_bytes(), 0);
        ws.place_asset_on_active(0).unwrap();
        assert!(ws.active_doc().memory_bytes() >= crate::raster::TILE_BYTES as u64);
        let copied = ws.duplicate_document(0).unwrap();
        assert_eq!(copied, 1);
        assert_ne!(ws.documents[0].id, ws.documents[1].id);
        assert!(ws.take_document(0).is_some());
        assert!(ws.take_document(0).is_none(), "the last document stays");
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

        let loaded = open_loaded(&path).unwrap();
        assert_eq!(loaded.assets.len(), 1);
        assert_eq!(loaded.documents.len(), 2);
        assert_eq!(loaded.assets[0].name, "tex");
        assert_eq!(loaded.documents[0].project.layers.len(), 1);
    }

    #[test]
    fn assets_save_as_lossless_webp_and_older_png_assets_still_open() {
        let pixels = vec![1, 2, 3, 4, 5, 6, 7, 128, 9, 10, 11, 0, 12, 13, 14, 255];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webp.mgimg");
        let mut workspace = Workspace::default();
        workspace
            .import_asset_rgba("tex".into(), &pixels, 2, 2)
            .unwrap();
        workspace.save(&path).unwrap();
        let archive = MegaFile::open(&path).unwrap();
        let stored = archive.read_all("/assets/img_2.webp").unwrap();
        assert_eq!(&stored[..4], b"RIFF");
        assert_eq!(&stored[8..12], b"WEBP");
        drop(archive);
        assert_eq!(open_loaded(&path).unwrap().assets[0].pixels, pixels);

        let legacy = dir.path().join("legacy.mgimg");
        let manifest = br#"{
            "format": "mega-image-editor",
            "version": 5,
            "active_document": 0,
            "assets": [{"id": "img_9", "name": "old", "width": 2, "height": 2, "path": "/assets/img_9.png"}],
            "documents": [{"id": "doc_1", "name": "Document 1", "width": 0, "height": 0, "active_layer": 0, "layers": []}]
        }"#;
        {
            let archive = MegaFile::create(&legacy).unwrap();
            archive
                .write_files(vec![
                    WriteFile::new("/manifest.json", MANIFEST_KIND, manifest.to_vec()),
                    WriteFile::new(
                        "/assets/img_9.png",
                        "image/png",
                        crate::project::encode_png_rgba(&pixels, 2, 2).unwrap(),
                    ),
                ])
                .unwrap();
        }
        let mut opened = open_loaded(&legacy).unwrap();
        assert_eq!(opened.assets[0].pixels, pixels);
        opened.save(&legacy).unwrap();
        let archive = MegaFile::open(&legacy).unwrap();
        let migrated = archive.read_all("/assets/img_9.webp").unwrap();
        assert_eq!(&migrated[8..12], b"WEBP");
        drop(archive);
        assert_eq!(open_loaded(&legacy).unwrap().assets[0].pixels, pixels);
    }
}
