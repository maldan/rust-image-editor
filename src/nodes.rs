//! Session-only image graph and pure evaluation. No workspace writes happen here.
use crate::project::{Layer, Project};
use glam::Vec2;
use image::RgbaImage;
use mega_ui::{NodeLink, NodeSpace};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};

pub const IMAGE_PORT: u16 = 100;
pub const PREVIEW_TEXTURE: u32 = 5;
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
    Image(Source),
    Resize { scale: f32 },
    Lineart { resolution: i32 },
    Output(Destination),
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
    next_id: u64,
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
            next_id: 1,
        }
    }
}

impl NodeEditor {
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
        if matches!(from.kind, NodeKind::Output(_))
            || matches!(to.kind, NodeKind::Image(_))
            || link.from_port != "image"
            || link.to_port != "image"
            || link.ty != IMAGE_PORT
        {
            return Err("Connect an Image output to an Image input".into());
        }
        if !inputs.insert(&link.to_node) {
            return Err("An input accepts one image".into());
        }
    }
    for node in nodes {
        let mut seen = HashSet::new();
        let mut current = node.id.as_str();
        while let Some(link) = links.iter().find(|l| l.to_node == current) {
            if !seen.insert(current) {
                return Err("Cycles are not supported".into());
            }
            current = &link.from_node;
        }
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
            .find(|l| l.to_node == current)
            .ok_or("Connect the Image input first")?;
        let node = nodes
            .iter()
            .find(|n| n.id == link.from_node)
            .ok_or("Source node was deleted")?;
        match &node.kind {
            NodeKind::Image(source) => {
                scales.reverse();
                return Ok((source.clone(), scales, destination.clone()));
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
            NodeKind::Output(_) => return Err("Output cannot be an image source".into()),
        }
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
                if !scale.is_finite() || scale <= 0.0 {
                    return Err("Scale must be greater than zero".into());
                }
                let w = (f64::from(image.width()) * f64::from(scale))
                    .round()
                    .max(1.0);
                let h = (f64::from(image.height()) * f64::from(scale))
                    .round()
                    .max(1.0);
                if w > 16384.0 || h > 16384.0 {
                    return Err("Resized image exceeds 16384 px".into());
                }
                let (w, h) = (w as u32, h as u32);
                check_size(w, h)?;
                if image.dimensions() != (w, h) {
                    image = image::imageops::resize(
                        &image,
                        w,
                        h,
                        image::imageops::FilterType::Lanczos3,
                    );
                }
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
    let ratio = (CELL as f32 / project.width as f32).min(CELL as f32 / project.height as f32);
    let w = (project.width as f32 * ratio).round().max(1.0) as u32;
    let h = (project.height as f32 * ratio).round().max(1.0) as u32;
    // Reuse the editor's alpha-aware box filter. Composite first so overlapping
    // layers are blended before averaging, and transparent RGB cannot bleed.
    let thumbnail = crate::editing::resize_rgba(
        &crate::editing::composite_layers(&project.layers, project.width, project.height),
        project.width,
        project.height,
        w,
        h,
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Raster;

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
        graph.add(NodeKind::Image(Source {
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
}
