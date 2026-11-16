//! Image graph evaluation and on-disk template (ids, not pixels).
use crate::project::{Layer, Project};
use glam::Vec2;
use image::RgbaImage;
use mega_ui::{NodeLink, NodeSpace};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};

pub const GRAPH_VERSION: u32 = 1;

pub const IMAGE_PORT: u16 = 100;
pub const SIZE_PORT: u16 = mega_ui::port_type::VEC2;
pub const PREVIEW_TEXTURE: u32 = 5;
pub const LIVE_PREVIEW_TEXTURE: u32 = 6;
pub const CELL: u32 = 512;
pub const ATLAS: u32 = CELL * 8;
pub const MAX_NODES: usize = 64;
const MAX_PIXELS: u64 = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct Source {
    pub document: String,
    pub layer: Option<Arc<()>>,
}

#[derive(Clone)]
pub enum Destination {
    NewDocument,
    Memory,
    Layer(String),
}

#[derive(Clone)]
pub enum NodeKind {
    ImageDocument(Source),
    ImageAsset { id: String },
    Resize { scale: f32 },
    Lineart { resolution: i32 },
    MinimaxRefs { prompt: String, megapixels: f32 },
    Output(Destination),
}

impl NodeKind {
    pub(crate) fn is_image_source(&self) -> bool {
        matches!(self, Self::ImageDocument(_) | Self::ImageAsset { .. })
    }
}

#[derive(Clone)]
pub struct Node {
    pub id: String,
    pub position: Vec2,
    pub kind: NodeKind,
    pub cell: usize,
}

pub struct Preview {
    // Keep tile Arcs alive so subsequent edits copy them and invalidate signature.
    pub _snapshot: Option<Project>,
    pub signature: u64,
    pub size: (u32, u32),
    pub ready: bool,
    pub error: String,
}

pub struct PreviewResult {
    pub id: String,
    pub signature: u64,
    pub size: (u32, u32),
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    Resize(f32),
    Lineart(i32),
}

pub struct LivePreview {
    pub node: String,
    pub size: (u32, u32),
    pub pixels: Vec<u8>,
    pub finished: bool,
}

#[derive(Clone, Default)]
pub struct NodeProgress {
    pub node: String,
    pub message: String,
    pub fraction: Option<f32>,
}

pub struct PreparedOutput {
    pub layer: Layer,
    pub size: (u32, u32),
    pub thumbnail: Vec<u8>,
}

pub struct Run {
    pub node: String,
    pub destination: Destination,
    pub destination_frame: Option<u32>,
    pub receiver: mpsc::Receiver<Result<PreparedOutput, String>>,
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    pub current_node: Arc<std::sync::Mutex<Option<String>>>,
    pub live_preview: Arc<std::sync::Mutex<Option<LivePreview>>>,
    pub progress: Arc<std::sync::Mutex<NodeProgress>>,
}

impl Drop for Run {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

pub struct NodeEditor {
    pub nodes: Vec<Node>,
    pub space: NodeSpace,
    pub visible: bool,
    pub message: String,
    pub run: Option<Run>,
    pub previews: HashMap<String, Preview>,
    pub memory: HashMap<String, Layer>,
    pub preview_job: Option<mpsc::Receiver<Vec<PreviewResult>>>,
    pub atlas: Vec<u8>,
    pub atlas_dirty: bool,
    pub live_display: Option<LivePreview>,
    pub live_dirty: bool,
    pub(crate) add_menu: AddMenu,
    next_id: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AddMenu {
    #[default]
    Root,
    Source,
    Ai,
    Utils,
}

pub struct GraphEntry {
    pub id: String,
    pub name: String,
    /// The active graph lives in EditorApp.nodes; all others live here.
    pub editor: Option<NodeEditor>,
}

pub struct Graphs {
    pub entries: Vec<GraphEntry>,
    pub active: usize,
    pub next_id: u64,
}

impl Default for Graphs {
    fn default() -> Self {
        Self {
            entries: vec![GraphEntry {
                id: "g_1".into(),
                name: "Graph 1".into(),
                editor: None,
            }],
            active: 0,
            next_id: 2,
        }
    }
}

impl Default for NodeEditor {
    fn default() -> Self {
        let mut space = NodeSpace::new();
        space.register_type(IMAGE_PORT, "Image", [0.4, 0.8, 0.5, 1.0]);
        Self {
            nodes: Vec::new(),
            space,
            visible: false,
            message: "Right-click the canvas to add nodes. Connect Image ports, then press Run."
                .into(),
            run: None,
            previews: HashMap::new(),
            memory: HashMap::new(),
            preview_job: None,
            atlas: Vec::new(),
            atlas_dirty: false,
            live_display: None,
            live_dirty: false,
            add_menu: AddMenu::Root,
            next_id: 1,
        }
    }
}

impl NodeEditor {
    pub fn duplicate(&self) -> Self {
        let mut copy = Self::default();
        copy.nodes = self.nodes.clone();
        copy.next_id = self.next_id;
        copy.space.pan = self.space.pan;
        copy.space.zoom = self.space.zoom;
        copy.space.snap = self.space.snap;
        copy.space.links = self.space.links.clone();
        copy.space.next_link_id = self.space.next_link_id;
        copy.space.frames = self.space.frames.clone();
        copy.space.next_frame_id = self.space.next_frame_id;
        copy.space.types = self.space.types.clone();
        copy.space.cast_pairs = self.space.cast_pairs.clone();
        copy
    }

    pub fn add(&mut self, kind: NodeKind) {
        if self.nodes.len() >= MAX_NODES {
            self.message = "This graph supports up to 64 nodes".into();
            return;
        }
        let cell = (0..MAX_NODES)
            .find(|cell| !self.nodes.iter().any(|n| n.cell == *cell))
            .unwrap();
        let i = self.nodes.len();
        let position = self.space.context_world.take().unwrap_or_else(|| {
            Vec2::new(30.0 + (i % 3) as f32 * 260.0, 30.0 + (i / 3) as f32 * 340.0)
        });
        self.nodes.push(Node {
            id: format!("node_{}", self.next_id),
            position,
            kind,
            cell,
        });
        self.next_id += 1;
    }

    pub fn remove_requested(&mut self) {
        let deleted = self.space.take_delete_nodes();
        if self
            .run
            .as_ref()
            .is_some_and(|run| deleted.contains(&run.node))
        {
            self.run = None;
            self.space.running_nodes.clear();
            self.message = "Run cancelled: output was deleted".into();
        }
        for id in &deleted {
            self.space.detach_node(id);
        }
        self.nodes.retain(|n| !deleted.contains(&n.id));
        self.previews.retain(|id, _| !deleted.contains(id));
        self.memory.retain(|id, _| !deleted.contains(id));
        self.space
            .links
            .retain(|l| !deleted.contains(&l.from_node) && !deleted.contains(&l.to_node));
        self.space.selected_nodes.retain(|id| !deleted.contains(id));
    }
}

/// Validate the complete graph, including disconnected cycles. Missing inputs are
/// allowed while editing; the execution planner requires them on the chosen path.
pub fn validate(nodes: &[Node], links: &[NodeLink]) -> Result<(), String> {
    let mut inputs = HashSet::new();
    for link in links {
        let from = nodes
            .iter()
            .find(|n| n.id == link.from_node)
            .ok_or("Source node was deleted")?;
        let to = nodes
            .iter()
            .find(|n| n.id == link.to_node)
            .ok_or("Destination node was deleted")?;
        let ok = match (&from.kind, link.from_port.as_str(), &to.kind, link.to_port.as_str(), link.ty) {
            (NodeKind::Output(_), ..) => false,
            (from, "size", NodeKind::Resize { .. }, "size", SIZE_PORT) if from.is_image_source() => true,
            (_, "image", NodeKind::MinimaxRefs { .. }, port, IMAGE_PORT)
                if ["image_1", "image_2", "image_3", "image_4"].contains(&port) =>
            {
                true
            }
            (_, "image", NodeKind::Resize { .. } | NodeKind::Lineart { .. } | NodeKind::Output(_), "image", IMAGE_PORT) => {
                true
            }
            _ => false,
        };
        if !ok {
            return Err("Connect compatible ports".into());
        }
        if !inputs.insert((&link.to_node, &link.to_port)) {
            return Err("An input accepts one connection".into());
        }
    }
    fn visit<'a>(
        id: &'a str,
        links: &'a [NodeLink],
        visiting: &mut HashSet<&'a str>,
        done: &mut HashSet<&'a str>,
    ) -> Result<(), String> {
        if done.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err("Cycles are not supported".into());
        }
        for link in links.iter().filter(|link| link.to_node == id) {
            visit(&link.from_node, links, visiting, done)?;
        }
        visiting.remove(id);
        done.insert(id);
        Ok(())
    }
    let mut done = HashSet::new();
    for node in nodes {
        visit(&node.id, links, &mut HashSet::new(), &mut done)?;
    }
    Ok(())
}

/// The initial operations have one input each, so a topological execution plan
/// is simply the selected output's upstream chain, in reverse order.
pub fn plan(
    nodes: &[Node],
    links: &[NodeLink],
    output: &str,
) -> Result<(Source, Vec<Operation>, Destination), String> {
    validate(nodes, links)?;
    let node = nodes
        .iter()
        .find(|n| n.id == output)
        .ok_or("Output node was deleted")?;
    let NodeKind::Output(destination) = &node.kind else {
        return Err("Choose an Output node".into());
    };
    let mut current = output;
    let mut scales = Vec::new();
    loop {
        let link = links
            .iter()
            .find(|l| l.to_node == current && l.to_port == "image")
            .ok_or("Connect the Image input first")?;
        let node = nodes
            .iter()
            .find(|n| n.id == link.from_node)
            .ok_or("Source node was deleted")?;
        match &node.kind {
            NodeKind::ImageDocument(source) => {
                scales.reverse();
                return Ok((source.clone(), scales, destination.clone()));
            }
            NodeKind::ImageAsset { .. } => {
                scales.reverse();
                return Ok((
                    Source {
                        document: String::new(),
                        layer: None,
                    },
                    scales,
                    destination.clone(),
                ));
            }
            NodeKind::Resize { scale } => {
                if !scale.is_finite() || *scale <= 0.0 {
                    return Err("Scale must be a finite number greater than zero".into());
                }
                scales.push(Operation::Resize(*scale));
                current = &node.id;
            }
            NodeKind::Lineart { resolution } => {
                crate::comfyui::validate_lineart_resolution(*resolution)?;
                scales.push(Operation::Lineart(*resolution));
                current = &node.id;
            }
            NodeKind::MinimaxRefs { .. } => {
                return Err("Multi-input node requires graph execution".into());
            }
            NodeKind::Output(_) => return Err("Output cannot be an image source".into()),
        }
    }
}

pub fn graph_plan(
    nodes: &[Node],
    links: &[NodeLink],
    output: &str,
) -> Result<(Vec<Node>, Destination), String> {
    validate(nodes, links)?;
    let root = nodes
        .iter()
        .find(|n| n.id == output)
        .ok_or("Output node was deleted")?;
    let NodeKind::Output(destination) = &root.kind else {
        return Err("Choose an Output node".into());
    };
    fn visit(
        id: &str,
        nodes: &[Node],
        links: &[NodeLink],
        seen: &mut HashSet<String>,
        order: &mut Vec<Node>,
    ) -> Result<(), String> {
        if !seen.insert(id.to_owned()) {
            return Ok(());
        }
        let node = nodes
            .iter()
            .find(|n| n.id == id)
            .ok_or("Node was deleted")?;
        if matches!(
            node.kind,
            NodeKind::Resize { .. } | NodeKind::Lineart { .. } | NodeKind::Output(_)
        ) && !links
            .iter()
            .any(|l| l.to_node == id && l.to_port == "image")
        {
            return Err("Connect the Image input first".into());
        }
        for link in links.iter().filter(|l| l.to_node == id) {
            visit(&link.from_node, nodes, links, seen, order)?;
        }
        order.push(node.clone());
        Ok(())
    }
    let mut order = Vec::new();
    visit(output, nodes, links, &mut HashSet::new(), &mut order)?;
    Ok((order, destination.clone()))
}

pub fn evaluate_graph(
    order: Vec<Node>,
    links: Vec<NodeLink>,
    projects: HashMap<String, Project>,
    config: Option<crate::comfyui::HostConfig>,
    cancel: &std::sync::atomic::AtomicBool,
    current_node: &std::sync::Mutex<Option<String>>,
    live_preview: &std::sync::Mutex<Option<LivePreview>>,
    node_progress: &std::sync::Mutex<NodeProgress>,
) -> Result<PreparedOutput, String> {
    use std::sync::atomic::Ordering;
    let mut images: HashMap<String, Arc<RgbaImage>> = HashMap::new();
    for node in order {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        *current_node.lock().unwrap() = Some(node.id.clone());
        *node_progress.lock().unwrap() = NodeProgress {
            node: node.id.clone(),
            message: String::new(),
            fraction: None,
        };
        let input = |port: &str| -> Option<Arc<RgbaImage>> {
            links
                .iter()
                .find(|l| l.to_node == node.id && l.to_port == port)
                .and_then(|l| images.get(&l.from_node))
                .cloned()
        };
        let image = match node.kind {
            NodeKind::ImageDocument(_) | NodeKind::ImageAsset { .. } => {
                let project = projects.get(&node.id).ok_or("Source document is missing")?;
                RgbaImage::from_raw(
                    project.width,
                    project.height,
                    crate::editing::composite_layers(
                        &project.layers,
                        project.width,
                        project.height,
                    ),
                )
                .ok_or("Invalid source pixels")?
            }
            NodeKind::Resize { scale } => {
                let image = input("image").ok_or("Connect the Image input first")?;
                let (width, height) = input("size")
                    .map(|source| source.dimensions())
                    .unwrap_or_else(|| image.dimensions());
                resize_rgba(&image, width, height, scale)?
            }
            NodeKind::Lineart { resolution } => {
                crate::comfyui::validate_lineart_resolution(resolution)?;
                let image = input("image").ok_or("Connect the Image input first")?;
                let job = crate::comfyui::Job::start_lineart(
                    config.clone().ok_or("ComfyUI is not configured")?,
                    (*image).clone(),
                    1.0,
                    resolution,
                );
                wait_graph_job(job, cancel, &node.id, live_preview, node_progress)?
            }
            NodeKind::MinimaxRefs { prompt, megapixels } => {
                let inputs = ["image_1", "image_2", "image_3", "image_4"]
                    .map(|port| input(port).map(|img| (*img).clone()));
                let job = crate::comfyui::Job::start_minimax_refs(
                    config.clone().ok_or("ComfyUI is not configured")?,
                    inputs,
                    None,
                    prompt,
                    megapixels,
                );
                wait_graph_job(job, cancel, &node.id, live_preview, node_progress)?
            }
            NodeKind::Output(_) => {
                let image = input("image").ok_or("Connect the Image input first")?;
                return prepare_image_output(&image);
            }
        };
        check_size(image.width(), image.height())?;
        images.insert(node.id, Arc::new(image));
    }
    Err("Output node is missing".into())
}

fn wait_graph_job(
    job: crate::comfyui::Job,
    cancel: &std::sync::atomic::AtomicBool,
    node_id: &str,
    live_preview: &std::sync::Mutex<Option<LivePreview>>,
    node_progress: &std::sync::Mutex<NodeProgress>,
) -> Result<RgbaImage, String> {
    use std::sync::atomic::Ordering;
    loop {
        if cancel.load(Ordering::Relaxed) {
            job.cancel();
            return Err("Cancelled".into());
        }
        if let Ok(mut progress) = job.progress.try_lock() {
            let message = progress.message.clone();
            let fraction = progress
                .fraction
                .filter(|value| value.is_finite())
                .map(|value| value.clamp(0.0, 1.0));
            let image = progress.preview.take();
            drop(progress);
            *node_progress.lock().unwrap() = NodeProgress {
                node: node_id.into(),
                message,
                fraction,
            };
            if let Some(image) = image {
                publish_live_preview(node_id, &image, live_preview, false);
            }
        }
        match job.poll() {
            Some(crate::comfyui::JobResult::Done(image)) => {
                publish_live_preview(node_id, &image, live_preview, true);
                return Ok(image);
            }
            Some(crate::comfyui::JobResult::Failed(error)) => return Err(error),
            Some(crate::comfyui::JobResult::Cancelled) => return Err("Cancelled".into()),
            Some(crate::comfyui::JobResult::Video(_)) => return Err("Expected an image".into()),
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
}

fn publish_live_preview(
    node_id: &str,
    image: &RgbaImage,
    slot: &std::sync::Mutex<Option<LivePreview>>,
    finished: bool,
) {
    if image.width() == 0 || image.height() == 0 {
        return;
    }
    let pixels = preview_rgba(image.as_raw(), image.width(), image.height());
    *slot.lock().unwrap() = Some(LivePreview {
        node: node_id.into(),
        size: image.dimensions(),
        pixels,
        finished,
    });
}

fn resize_rgba(image: &RgbaImage, width: u32, height: u32, scale: f32) -> Result<RgbaImage, String> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err("Scale must be a finite number greater than zero".into());
    }
    let w = (f64::from(width) * f64::from(scale)).round().max(1.0);
    let h = (f64::from(height) * f64::from(scale)).round().max(1.0);
    if w > 16384.0 || h > 16384.0 {
        return Err("Resized image exceeds 16384 px".into());
    }
    let (w, h) = (w as u32, h as u32);
    check_size(w, h)?;
    if image.dimensions() == (w, h) {
        Ok(image.clone())
    } else {
        Ok(image::imageops::resize(
            image,
            w,
            h,
            image::imageops::FilterType::Lanczos3,
        ))
    }
}

pub fn check_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("Source document is empty".into());
    }
    if width > 16384 || height > 16384 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("Image exceeds the limit: 16384 px per side / 64 megapixels".into());
    }
    Ok(())
}

pub fn source_project(project: &Project, source: &Source) -> Result<Project, String> {
    check_size(project.width, project.height)?;
    let mut snapshot = project.clone();
    if let Some(identity) = &source.layer {
        let layer = project
            .layers
            .iter()
            .find(|l| Arc::ptr_eq(&l.identity, identity))
            .ok_or("Source layer was deleted; select another layer")?;
        let mut layer = layer.clone();
        // A layer source means its raw pixels, independent of document visibility.
        layer.visible = true;
        layer.opacity = 1.0;
        snapshot.layers = vec![layer];
    }
    Ok(snapshot)
}

#[cfg(test)]
pub fn evaluate(project: Project, scales: Vec<f32>) -> Result<PreparedOutput, String> {
    evaluate_operations(
        project,
        scales.into_iter().map(Operation::Resize).collect(),
        None,
        &std::sync::atomic::AtomicBool::new(false),
    )
}

pub fn evaluate_operations(
    project: Project,
    operations: Vec<Operation>,
    config: Option<crate::comfyui::HostConfig>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<PreparedOutput, String> {
    use std::sync::atomic::Ordering;
    check_size(project.width, project.height)?;
    let pixels = crate::editing::composite_layers(&project.layers, project.width, project.height);
    let mut image = RgbaImage::from_raw(project.width, project.height, pixels)
        .ok_or("Invalid source pixels")?;
    for operation in operations {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        match operation {
            Operation::Resize(scale) => {
                let (width, height) = image.dimensions();
                image = resize_rgba(&image, width, height, scale)?;
            }
            Operation::Lineart(resolution) => {
                crate::comfyui::validate_lineart_resolution(resolution)?;
                let job = crate::comfyui::Job::start_lineart(
                    config.clone().ok_or("ComfyUI is not configured")?,
                    image,
                    1.0,
                    resolution,
                );
                image = loop {
                    if cancel.load(Ordering::Relaxed) {
                        job.cancel();
                        return Err("Cancelled".into());
                    }
                    match job.poll() {
                        Some(crate::comfyui::JobResult::Done(image)) => break image,
                        Some(crate::comfyui::JobResult::Failed(error)) => return Err(error),
                        Some(crate::comfyui::JobResult::Cancelled) => {
                            return Err("Cancelled".into());
                        }
                        Some(crate::comfyui::JobResult::Video(_)) => {
                            return Err("Expected an image from Lineart".into());
                        }
                        None => std::thread::sleep(std::time::Duration::from_millis(50)),
                    }
                };
                check_size(image.width(), image.height())?;
            }
        }
    }
    prepare_image_output(&image)
}

fn prepare_image_output(image: &RgbaImage) -> Result<PreparedOutput, String> {
    let (w, h) = image.dimensions();
    let layer = Layer {
        identity: Default::default(),
        ai_metadata: None,
        z: 0.0,
        drawings: Default::default(),
        name: "Node output".into(),
        visible: true,
        opacity: 1.0,
        pixels: crate::raster::SparsePixels::from_rgba(image.as_raw(), w, h),
    };
    let thumbnail = preview(&Project {
        width: w,
        height: h,
        layers: vec![layer.clone()],
        ..Default::default()
    });
    Ok(PreparedOutput {
        layer,
        size: (w, h),
        thumbnail,
    })
}

/// Area-average thumbnail, evaluated in the cached background preview worker.
pub fn preview(project: &Project) -> Vec<u8> {
    let pixels = crate::editing::composite_layers(&project.layers, project.width, project.height);
    preview_rgba(&pixels, project.width, project.height)
}

fn preview_rgba(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let ratio = (CELL as f32 / width as f32).min(CELL as f32 / height as f32);
    let w = (width as f32 * ratio).round().max(1.0) as u32;
    let h = (height as f32 * ratio).round().max(1.0) as u32;
    let thumbnail = crate::editing::resize_rgba(pixels, width, height, w, h);
    let mut pixels = vec![0; (CELL * CELL * 4) as usize];
    for y in 0..CELL {
        for x in 0..CELL {
            let bg = if (x / 8 + y / 8) % 2 == 0 { 56.0 } else { 76.0 };
            let mut color = [bg as u8, bg as u8, bg as u8, 255];
            let (ox, oy) = ((CELL - w) / 2, (CELL - h) / 2);
            if x >= ox && x < ox + w && y >= oy && y < oy + h {
                let src = (((y - oy) * w + x - ox) * 4) as usize;
                let rgba = &thumbnail[src..src + 4];
                let a = rgba[3] as f32 / 255.0;
                for c in 0..3 {
                    color[c] = (rgba[c] as f32 * a + bg * (1.0 - a)).round() as u8;
                }
            }
            let i = ((y * CELL + x) * 4) as usize;
            pixels[i..i + 4].copy_from_slice(&color);
        }
    }
    pixels
}

pub fn encode_graph(editor: &NodeEditor) -> Result<Vec<u8>, String> {
    let file = GraphFile {
        version: GRAPH_VERSION,
        view: GraphViewFile {
            pan: [editor.space.pan.x, editor.space.pan.y],
            zoom: editor.space.zoom,
            snap: editor.space.snap,
        },
        nodes: editor
            .nodes
            .iter()
            .map(|node| GraphNodeFile {
                id: node.id.clone(),
                pos: [node.position.x, node.position.y],
                kind: node_kind_name(&node.kind).into(),
                data: node_kind_data(&node.kind),
            })
            .collect(),
        links: editor
            .space
            .links
            .iter()
            .map(|link| GraphLinkFile {
                id: link.id,
                from: [link.from_node.clone(), link.from_port.clone()],
                to: [link.to_node.clone(), link.to_port.clone()],
            })
            .collect(),
    };
    serde_json::to_vec_pretty(&file).map_err(|e| format!("Cannot encode graph: {e}"))
}

pub fn decode_graph(bytes: &[u8]) -> Result<NodeEditor, String> {
    let file: GraphFile =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid graph: {e}"))?;
    if file.version != GRAPH_VERSION {
        return Err(format!("Unsupported graph version {}", file.version));
    }
    let mut editor = NodeEditor::default();
    editor.space.pan = Vec2::new(file.view.pan[0], file.view.pan[1]);
    editor.space.zoom = file.view.zoom;
    editor.space.snap = file.view.snap;
    let mut next_id = 1u64;
    let mut used = HashSet::new();
    for (index, node) in file.nodes.into_iter().enumerate() {
        if !used.insert(node.id.clone()) {
            return Err(format!("Duplicate node '{}'", node.id));
        }
        bump_node_id(&node.id, &mut next_id);
        if index >= MAX_NODES {
            return Err("This graph supports up to 64 nodes".into());
        }
        editor.nodes.push(Node {
            id: node.id,
            position: Vec2::new(node.pos[0], node.pos[1]),
            kind: node_kind_from_file(&node.kind, node.data)?,
            cell: index,
        });
    }
    editor.next_id = next_id;
    let mut next_link = 1u64;
    let mut link_ids = HashSet::new();
    for link in file.links {
        if !link_ids.insert(link.id) {
            return Err(format!("Duplicate link {}", link.id));
        }
        next_link = next_link.max(link.id.saturating_add(1));
        let from_ok = editor.nodes.iter().any(|n| n.id == link.from[0]);
        let to_ok = editor.nodes.iter().any(|n| n.id == link.to[0]);
        if !from_ok || !to_ok {
            return Err("Graph link points to a missing node".into());
        }
        editor.space.links.push(NodeLink {
            id: link.id,
            from_node: link.from[0].clone(),
            from_port: link.from[1].clone(),
            to_node: link.to[0].clone(),
            to_port: link.to[1].clone(),
            ty: if link.from[1] == "size" {
                SIZE_PORT
            } else {
                IMAGE_PORT
            },
        });
    }
    editor.space.next_link_id = next_link;
    Ok(editor)
}

fn bump_node_id(id: &str, next: &mut u64) {
    if let Some(n) = id.strip_prefix("node_").and_then(|s| s.parse::<u64>().ok()) {
        *next = (*next).max(n.saturating_add(1));
    }
}

fn node_kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::ImageDocument(_) => "image_document",
        NodeKind::ImageAsset { .. } => "image_asset",
        NodeKind::Resize { .. } => "resize",
        NodeKind::Lineart { .. } => "lineart",
        NodeKind::MinimaxRefs { .. } => "minimax_refs",
        NodeKind::Output(_) => "output",
    }
}

fn node_kind_data(kind: &NodeKind) -> serde_json::Value {
    match kind {
        NodeKind::ImageDocument(source) => serde_json::json!({ "document": source.document }),
        NodeKind::ImageAsset { id } => serde_json::json!({ "asset": id }),
        NodeKind::Resize { scale } => serde_json::json!({ "scale": scale }),
        NodeKind::Lineart { resolution } => serde_json::json!({ "resolution": resolution }),
        NodeKind::MinimaxRefs { prompt, megapixels } => {
            serde_json::json!({ "prompt": prompt, "megapixels": megapixels })
        }
        NodeKind::Output(Destination::Memory) => serde_json::json!({ "destination": "memory" }),
        NodeKind::Output(Destination::NewDocument) => {
            serde_json::json!({ "destination": "new_document" })
        }
        NodeKind::Output(Destination::Layer(document)) => {
            serde_json::json!({ "destination": { "layer": document } })
        }
    }
}

fn node_kind_from_file(kind: &str, data: serde_json::Value) -> Result<NodeKind, String> {
    match kind {
        "image_document" => {
            let document = data
                .get("document")
                .and_then(serde_json::Value::as_str)
                .ok_or("Image document node is missing document")?
                .to_string();
            Ok(NodeKind::ImageDocument(Source {
                document,
                layer: None,
            }))
        }
        "image_asset" => {
            let id = data
                .get("asset")
                .and_then(serde_json::Value::as_str)
                .ok_or("Image asset node is missing asset")?
                .to_string();
            Ok(NodeKind::ImageAsset { id })
        }
        "resize" => {
            let scale = data
                .get("scale")
                .and_then(serde_json::Value::as_f64)
                .ok_or("Resize node is missing scale")? as f32;
            Ok(NodeKind::Resize { scale })
        }
        "lineart" => {
            let resolution = data
                .get("resolution")
                .and_then(serde_json::Value::as_i64)
                .ok_or("Lineart node is missing resolution")? as i32;
            Ok(NodeKind::Lineart { resolution })
        }
        "minimax_refs" => {
            let prompt = data
                .get("prompt")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string();
            let megapixels = data
                .get("megapixels")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(4.0) as f32;
            Ok(NodeKind::MinimaxRefs { prompt, megapixels })
        }
        "output" => {
            let destination = data.get("destination").ok_or("Output node is missing destination")?;
            let dest = if let Some(name) = destination.as_str() {
                match name {
                    "memory" => Destination::Memory,
                    "new_document" => Destination::NewDocument,
                    _ => return Err("Unknown output destination".into()),
                }
            } else if let Some(document) = destination.get("layer").and_then(serde_json::Value::as_str)
            {
                Destination::Layer(document.to_string())
            } else {
                return Err("Unknown output destination".into());
            };
            Ok(NodeKind::Output(dest))
        }
        _ => Err(format!("Unknown node kind '{kind}'")),
    }
}

#[derive(Serialize, Deserialize)]
struct GraphFile {
    version: u32,
    view: GraphViewFile,
    #[serde(default)]
    nodes: Vec<GraphNodeFile>,
    #[serde(default)]
    links: Vec<GraphLinkFile>,
}

#[derive(Serialize, Deserialize)]
struct GraphViewFile {
    pan: [f32; 2],
    zoom: f32,
    #[serde(default = "default_snap")]
    snap: f32,
}

fn default_snap() -> f32 {
    5.0
}

#[derive(Serialize, Deserialize)]
struct GraphNodeFile {
    id: String,
    pos: [f32; 2],
    kind: String,
    data: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
struct GraphLinkFile {
    id: u64,
    from: [String; 2],
    to: [String; 2],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Raster;

    #[test]
    fn nodes_receive_comfy_progress_preview_and_cancel_wait() {
        let (job, _sender) = crate::comfyui::Job::pending();
        job.progress.lock().unwrap().preview =
            Some(RgbaImage::from_pixel(2, 1, image::Rgba([20, 40, 60, 255])));
        {
            let mut progress = job.progress.lock().unwrap();
            progress.message = "Uploading reference 2...".into();
            progress.fraction = Some(0.4);
        }
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let slot = Arc::new(std::sync::Mutex::new(None));
        let progress = Arc::new(std::sync::Mutex::new(NodeProgress::default()));
        let worker_cancel = cancel.clone();
        let worker_slot = slot.clone();
        let worker_progress = progress.clone();
        let worker = std::thread::spawn(move || {
            wait_graph_job(
                job,
                &worker_cancel,
                "minimax",
                &worker_slot,
                &worker_progress,
            )
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(preview) = slot.lock().unwrap().take() {
                assert_eq!(preview.node, "minimax");
                assert_eq!(preview.size, (2, 1));
                assert!(!preview.finished);
                assert_eq!(preview.pixels.len(), (CELL * CELL * 4) as usize);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Preview was not delivered"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let progress = progress.lock().unwrap().clone();
        assert_eq!(progress.node, "minimax");
        assert_eq!(progress.message, "Uploading reference 2...");
        assert_eq!(progress.fraction, Some(0.4));
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(worker.join().unwrap().unwrap_err(), "Cancelled");
    }

    #[test]
    fn nodes_preview_averages_fine_detail_without_transparent_color_bleed() {
        let side = CELL * 2;
        for transparent in [false, true] {
            let mut pixels = Vec::with_capacity((side * side * 4) as usize);
            for y in 0..side {
                for x in 0..side {
                    let color = match (transparent, (x + y) % 2 == 0) {
                        (false, true) => [0, 0, 0, 255],
                        (false, false) => [255; 4],
                        (true, true) => [255, 0, 0, 0],
                        (true, false) => [0, 0, 255, 255],
                    };
                    pixels.extend(color);
                }
            }
            let mut project = Project::default();
            project.import_rgba(&pixels, side, side).unwrap();
            let output = preview(&project);
            if transparent {
                assert_eq!(&output[..4], &[28, 28, 156, 255]);
            } else {
                assert!(output.chunks_exact(4).all(|p| p == [128, 128, 128, 255]));
            }
        }
    }

    fn graph() -> NodeEditor {
        let mut graph = NodeEditor::default();
        graph.add(NodeKind::ImageDocument(Source {
            document: "doc".into(),
            layer: None,
        }));
        graph.add(NodeKind::Resize { scale: 0.5 });
        graph.add(NodeKind::Output(Destination::NewDocument));
        graph.space.links = vec![link("node_1", "node_2"), link("node_2", "node_3")];
        graph
    }

    fn link(from: &str, to: &str) -> NodeLink {
        NodeLink {
            id: 1,
            from_node: from.into(),
            to_node: to.into(),
            from_port: "image".into(),
            to_port: "image".into(),
            ty: IMAGE_PORT,
        }
    }

    #[test]
    fn nodes_plan_only_the_selected_output_and_preserve_operation_order() {
        let mut graph = graph();
        graph.add(NodeKind::Resize { scale: 2.0 });
        graph.add(NodeKind::Resize { scale: 3.0 }); // disconnected input is allowed
        graph.space.links = vec![
            link("node_1", "node_2"),
            link("node_2", "node_4"),
            link("node_4", "node_3"),
        ];
        assert_eq!(
            plan(&graph.nodes, &graph.space.links, "node_3").unwrap().1,
            [Operation::Resize(0.5), Operation::Resize(2.0)]
        );
        graph.space.links = vec![link("node_1", "node_3")];
        assert!(
            plan(&graph.nodes, &graph.space.links, "node_3")
                .unwrap()
                .1
                .is_empty()
        );
    }

    #[test]
    fn nodes_plan_lineart_in_chain_and_validate_resolution() {
        let mut graph = graph();
        graph.add(NodeKind::Lineart { resolution: 512 });
        graph.space.links = vec![
            link("node_1", "node_2"),
            link("node_2", "node_4"),
            link("node_4", "node_3"),
        ];
        assert_eq!(
            plan(&graph.nodes, &graph.space.links, "node_3").unwrap().1,
            [Operation::Resize(0.5), Operation::Lineart(512)]
        );
        graph.nodes[3].kind = NodeKind::Lineart { resolution: 513 };
        assert!(plan(&graph.nodes, &graph.space.links, "node_3").is_err());
    }

    #[test]
    fn nodes_multi_input_graph_orders_shared_dependencies_and_rejects_cycles() {
        let mut graph = graph();
        graph.add(NodeKind::MinimaxRefs {
            prompt: "test".into(),
            megapixels: 4.0,
        });
        let mut first = link("node_2", "node_4");
        first.to_port = "image_1".into();
        let mut second = link("node_2", "node_4");
        second.to_port = "image_3".into();
        graph.space.links = vec![
            link("node_1", "node_2"),
            first,
            second,
            link("node_4", "node_3"),
        ];
        let (order, _) = graph_plan(&graph.nodes, &graph.space.links, "node_3").unwrap();
        assert_eq!(
            order.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            ["node_1", "node_2", "node_4", "node_3"]
        );
        let mut cycle = link("node_4", "node_4");
        cycle.to_port = "image_4".into();
        graph.space.links.push(cycle);
        assert!(
            validate(&graph.nodes, &graph.space.links)
                .unwrap_err()
                .contains("Cycles")
        );
        graph.space.links = vec![link("node_4", "node_3")];
        assert!(graph_plan(&graph.nodes, &graph.space.links, "node_3").is_ok());
    }

    #[test]
    fn nodes_reject_cycles_missing_inputs_and_invalid_scales() {
        let mut graph = graph();
        graph.space.links = vec![link("node_2", "node_2"), link("node_2", "node_3")];
        assert!(
            validate(&graph.nodes, &graph.space.links)
                .unwrap_err()
                .contains("Cycles")
        );
        assert!(plan(&graph.nodes, &[], "node_3").is_err());
        graph.space.links = vec![link("node_1", "node_2"), link("node_2", "node_3")];
        graph.nodes[1].kind = NodeKind::Resize { scale: f32::NAN };
        assert!(plan(&graph.nodes, &graph.space.links, "node_3").is_err());
        graph.nodes[1].kind = NodeKind::Resize { scale: 0.5 };
        graph.space.links.push(NodeLink {
            id: 9,
            from_node: "node_1".into(),
            from_port: "size".into(),
            to_node: "node_2".into(),
            to_port: "size".into(),
            ty: SIZE_PORT,
        });
        assert!(validate(&graph.nodes, &graph.space.links).is_ok());
        assert_eq!(
            plan(&graph.nodes, &graph.space.links, "node_3").unwrap().1,
            [Operation::Resize(0.5)]
        );
        graph.space.links.last_mut().unwrap().from_port = "image".into();
        assert!(validate(&graph.nodes, &graph.space.links).is_err());
    }

    #[test]
    fn nodes_resize_uses_connected_size_then_scale() {
        let mut source = Project::default();
        source.import_rgba(&[10, 20, 30, 255].repeat(4), 2, 2).unwrap();
        let mut size = Project::default();
        size.import_rgba(&[1, 2, 3, 255].repeat(8), 4, 2).unwrap();
        let mut graph = NodeEditor::default();
        graph.add(NodeKind::ImageDocument(Source { document: "pixels".into(), layer: None }));
        graph.add(NodeKind::ImageDocument(Source { document: "size".into(), layer: None }));
        graph.add(NodeKind::Resize { scale: 2.0 });
        graph.add(NodeKind::Output(Destination::Memory));
        graph.space.links = vec![
            link("node_1", "node_3"),
            NodeLink {
                id: 2,
                from_node: "node_2".into(),
                from_port: "size".into(),
                to_node: "node_3".into(),
                to_port: "size".into(),
                ty: SIZE_PORT,
            },
            link("node_3", "node_4"),
        ];
        let (order, _) = graph_plan(&graph.nodes, &graph.space.links, "node_4").unwrap();
        assert_eq!(
            order.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            ["node_1", "node_2", "node_3", "node_4"]
        );
        let mut projects = HashMap::new();
        projects.insert("node_1".into(), source);
        projects.insert("node_2".into(), size);
        let result = evaluate_graph(
            order,
            graph.space.links,
            projects,
            None,
            &std::sync::atomic::AtomicBool::new(false),
            &std::sync::Mutex::new(None),
            &std::sync::Mutex::new(None),
            &std::sync::Mutex::new(NodeProgress::default()),
        )
        .unwrap();
        assert_eq!(result.size, (8, 4));
    }

    #[test]
    fn nodes_layer_reference_survives_rename_reorder_and_detects_deletion() {
        let mut project = Project::default();
        project
            .import_rgba(&[255, 0, 0, 255, 0, 0, 0, 0], 2, 1)
            .unwrap();
        let identity = project.layers[0].identity.clone();
        project.layers[0].visible = false;
        project.layers[0].opacity = 0.0;
        project.layers[0].pixels.offset = (1, 0);
        project.add_layer();
        project.layers.swap(0, 1);
        project.layers[1].name = "Renamed".into();
        let source = Source {
            document: "doc".into(),
            layer: Some(identity),
        };
        let snapshot = source_project(&project, &source).unwrap();
        let result = evaluate(snapshot, vec![]).unwrap();
        assert_eq!(result.size, (2, 1));
        assert_eq!(result.layer.pixels.read(0, 0, 0), [0, 0, 0, 0]);
        assert_eq!(result.layer.pixels.read(0, 1, 0), [255, 0, 0, 255]);
        project.layers.pop();
        assert!(source_project(&project, &source).is_err());
    }

    #[test]
    fn nodes_resize_snapshot_preserves_alpha_and_checks_intermediate_limits() {
        let mut project = Project::default();
        project
            .import_rgba(&[20, 40, 60, 128].repeat(8), 4, 2)
            .unwrap();
        let snapshot = project.clone();
        project.layers[0].pixels.write(0, 0, 0, [255; 4]);
        let result = evaluate(snapshot.clone(), vec![0.5]).unwrap();
        assert_eq!(result.size, (2, 1));
        let pixel = result.layer.pixels.read(0, 0, 0);
        assert_eq!(pixel[3], 128);
        assert!((pixel[0] as i32 - 20).abs() <= 1);
        assert!(evaluate(snapshot, vec![10000.0, 0.0001]).is_err());
        assert!(check_size(16384, 16384).is_err());
        assert!(check_size(0, 10).is_err());
    }

    #[test]
    fn graph_template_roundtrips_ids_without_pixels() {
        let mut editor = NodeEditor::default();
        editor.add(NodeKind::ImageDocument(Source {
            document: "doc_9".into(),
            layer: Some(Default::default()),
        }));
        editor.add(NodeKind::ImageAsset {
            id: "img_2".into(),
        });
        editor.add(NodeKind::Output(Destination::Layer("doc_9".into())));
        editor.space.pan = Vec2::new(12.0, -4.0);
        editor.space.zoom = 0.5;
        editor.space.links.push(mega_ui::NodeLink {
            id: 7,
            from_node: "node_1".into(),
            from_port: "image".into(),
            to_node: "node_3".into(),
            to_port: "image".into(),
            ty: IMAGE_PORT,
        });
        let loaded = decode_graph(&encode_graph(&editor).unwrap()).unwrap();
        assert_eq!(loaded.space.pan, Vec2::new(12.0, -4.0));
        assert_eq!(loaded.space.zoom, 0.5);
        assert_eq!(loaded.nodes.len(), 3);
        assert!(matches!(
            &loaded.nodes[0].kind,
            NodeKind::ImageDocument(source) if source.document == "doc_9" && source.layer.is_none()
        ));
        assert!(matches!(
            &loaded.nodes[1].kind,
            NodeKind::ImageAsset { id } if id == "img_2"
        ));
        assert!(matches!(
            &loaded.nodes[2].kind,
            NodeKind::Output(Destination::Layer(document)) if document == "doc_9"
        ));
        assert_eq!(loaded.space.links[0].id, 7);
        assert_eq!(loaded.next_id, 4);
        assert_eq!(loaded.space.next_link_id, 8);
    }
}
