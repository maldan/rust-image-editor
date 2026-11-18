use super::*;
use crate::nodes::{
    self, Destination, GraphOutput, NodeKind, PreparedOutput, Preview, PreviewResult, Run, Source,
};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, mpsc};

impl EditorApp {
    fn node_source_snapshot(&mut self, source: &Source) -> Result<Project, String> {
        let index = self
            .workspace
            .documents
            .iter()
            .position(|d| d.id == source.document)
            .ok_or("Source document was deleted; select another document")?;
        self.workspace.ensure_document_loaded(index)?;
        nodes::source_project(&self.workspace.documents[index].project, source)
    }

    fn node_asset_snapshot(&mut self, id: &str) -> Result<Project, String> {
        let index = self
            .workspace
            .assets
            .iter()
            .position(|asset| asset.id == id)
            .ok_or("Source asset was deleted; select another asset")?;
        self.workspace.ensure_asset_loaded(index)?;
        let asset = &self.workspace.assets[index];
        nodes::check_size(asset.width, asset.height)?;
        let mut project = Project::default();
        project.import_rgba(&asset.pixels, asset.width, asset.height)?;
        Ok(project)
    }

    fn node_kind_snapshot(&mut self, kind: &NodeKind) -> Result<Project, String> {
        match kind {
            NodeKind::ImageDocument(source) => self.node_source_snapshot(source),
            NodeKind::ImageAsset { id } => self.node_asset_snapshot(id),
            _ => Err("Choose an image source".into()),
        }
    }

    fn node_kind_signature(&mut self, kind: &NodeKind) -> Result<u64, String> {
        match kind {
            NodeKind::ImageDocument(source) => {
                Ok(preview_signature(&self.node_source_snapshot(source)?))
            }
            NodeKind::ImageAsset { id } => {
                let index = self
                    .workspace
                    .assets
                    .iter()
                    .position(|asset| asset.id == *id)
                    .ok_or("Source asset was deleted; select another asset")?;
                self.workspace.ensure_asset_loaded(index)?;
                let asset = &self.workspace.assets[index];
                nodes::check_size(asset.width, asset.height)?;
                Ok(asset_preview_signature(
                    &asset.id,
                    asset.width,
                    asset.height,
                    asset.pixels_hash,
                ))
            }
            _ => Err("Choose an image source".into()),
        }
    }

    pub fn run_node_output(&mut self, id: &str) {
        if self.nodes.run.is_some() {
            return;
        }
        self.commit_transform();
        self.finish_deformation(false);
        let prepared = (|| {
            let (order, destination) =
                nodes::graph_plan(&self.nodes.nodes, &self.nodes.space.links, id)?;
            let mut destination_frame = None;
            if let Some(Destination::Layer(document)) = &destination {
                let index = self
                    .workspace
                    .documents
                    .iter()
                    .position(|d| &d.id == document)
                    .ok_or("Choose an existing destination document")?;
                self.workspace.ensure_document_loaded(index)?;
                destination_frame = self.workspace.documents[index]
                    .project
                    .animation
                    .as_ref()
                    .map(|a| a.frame);
            }
            let mut projects = std::collections::HashMap::new();
            for node in &order {
                if node.kind.is_image_source() {
                    projects.insert(node.id.clone(), self.node_kind_snapshot(&node.kind)?);
                }
            }
            let config = if order.iter().any(|node| node.kind.uses_comfy()) {
                Some(crate::comfyui::HostConfig::load()?)
            } else {
                None
            };
            Ok::<_, String>((
                order,
                self.nodes.space.links.clone(),
                projects,
                destination,
                destination_frame,
                config,
            ))
        })();
        match prepared {
            Ok((order, links, projects, destination, destination_frame, config)) => {
                let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let worker_cancel = cancel.clone();
                let current_node = Arc::new(std::sync::Mutex::new(None));
                let worker_current_node = current_node.clone();
                let live_preview = Arc::new(std::sync::Mutex::new(None));
                let worker_live_preview = live_preview.clone();
                let progress = Arc::new(std::sync::Mutex::new(nodes::NodeProgress::default()));
                let worker_progress = progress.clone();
                let times = Arc::new(std::sync::Mutex::new(nodes::ExecutionTimes::default()));
                let worker_times = times.clone();
                self.nodes.live_display = None;
                for node in &order {
                    self.nodes.space.set_node_annotation(&node.id, None);
                    self.nodes.space.set_node_progress(&node.id, None);
                    if node.kind.uses_comfy() {
                        self.nodes.previews.remove(&node.id);
                    }
                }
                let (sender, receiver) = mpsc::channel();
                match std::thread::Builder::new()
                    .name("node-pipeline".into())
                    .spawn(move || {
                        let result = std::panic::catch_unwind(|| {
                            nodes::evaluate_graph(
                                order,
                                links,
                                projects,
                                config,
                                &worker_cancel,
                                &worker_current_node,
                                &worker_live_preview,
                                &worker_progress,
                                &worker_times,
                            )
                        })
                        .unwrap_or_else(|_| Err("Pipeline worker failed".into()));
                        let _ = sender.send(result);
                    }) {
                    Ok(_) => {
                        self.nodes.message = "Running…".into();
                        self.nodes.run = Some(Run {
                            node: id.into(),
                            destination,
                            destination_frame,
                            receiver,
                            cancel,
                            current_node,
                            live_preview,
                            progress,
                            times,
                        });
                    }
                    Err(error) => self.nodes.message = format!("Cannot start pipeline: {error}"),
                }
            }
            Err(error) => self.nodes.message = error,
        }
    }

    pub(in crate::app) fn poll_nodes(&mut self, ui: &mut Ui) {
        if let Some(run) = &self.nodes.run {
            let current = run.current_node.lock().unwrap().clone();
            let live_preview = run.live_preview.lock().unwrap().take();
            let progress = run.progress.lock().unwrap().clone();
            let stopping = run.cancel.load(std::sync::atomic::Ordering::Relaxed);
            let (finished, running) = {
                let times = run.times.lock().unwrap();
                (times.finished.clone(), times.running.clone())
            };
            for id in std::mem::take(&mut self.nodes.space.running_nodes) {
                self.nodes.space.set_node_progress(&id, None);
            }
            if let Some(id) =
                current.filter(|id| self.nodes.nodes.iter().any(|node| node.id == *id))
            {
                if !stopping && progress.node == id {
                    self.nodes.space.set_node_progress(&id, progress.fraction);
                }
                self.nodes.space.running_nodes.insert(id);
            }
            for (id, duration) in finished {
                if self.nodes.nodes.iter().any(|node| node.id == id) {
                    self.nodes.space.set_node_annotation(&id, Some(&format_node_duration(duration)));
                }
            }
            if let Some((id, started)) = running {
                if self.nodes.nodes.iter().any(|node| node.id == id) {
                    self.nodes.space.set_node_annotation(&id, Some(&format_node_duration(started.elapsed())));
                }
            }
            let result = match run.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Pipeline worker stopped".into()))
                }
            };
            if let Some(preview) = live_preview {
                if self.nodes.nodes.iter().any(|node| node.id == preview.node) {
                    if preview.finished {
                        self.store_node_thumbnail(&preview.node, preview.size, &preview.pixels);
                        self.nodes.live_display = None;
                    } else {
                        self.nodes.live_display = Some(preview);
                        self.nodes.live_dirty = true;
                    }
                    ui.request_repaint();
                }
            }
            if let Some(result) = result {
                let run = self.nodes.run.take().unwrap();
                // The worker can finish between the timing snapshot and receiving
                // its result; read completed times once more before dropping Run.
                for (id, duration) in &run.times.lock().unwrap().finished {
                    if self.nodes.nodes.iter().any(|node| &node.id == id) {
                        self.nodes.space.set_node_annotation(id, Some(&format_node_duration(*duration)));
                    }
                }
                for id in std::mem::take(&mut self.nodes.space.running_nodes) {
                    self.nodes.space.set_node_progress(&id, None);
                }
                let result = if run.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    Err("Cancelled".into())
                } else if !self.nodes.nodes.iter().any(|n| n.id == run.node) {
                    Err("Output was deleted; result discarded".into())
                } else {
                    result.and_then(|result| {
                        let result = match result {
                            GraphOutput::Image(image) => image,
                            GraphOutput::Video(bytes) => {
                                let index = self.workspace.add_video_named(bytes, "Ltx 2.3 I2V")?;
                                self.dirty = true;
                                return Ok(format!("Video saved: {}", self.workspace.videos[index].name));
                            }
                        };
                        let destination = run.destination.clone().ok_or("Expected a video")?;
                        if matches!(destination, Destination::Memory) {
                            self.nodes
                                .memory
                                .insert(run.node.clone(), result.layer.clone());
                        } else {
                            self.nodes.memory.remove(&run.node);
                        }
                        self.store_node_result(&run.node, &result);
                        self.apply_node_output(
                            destination,
                            run.destination_frame,
                            result,
                        )
                    })
                };
                match result {
                    Ok(message) => {
                        self.nodes.message = message.clone();
                        self.status = message;
                    }
                    Err(error) => {
                        if run.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                            self.nodes.message = "Graph stopped".into();
                            self.status = self.nodes.message.clone();
                        } else {
                            self.nodes.message = error.clone();
                            ui.notify_error(&error);
                        }
                    }
                }
            }
        }
        if let Some(receiver) = &self.nodes.preview_job {
            match receiver.try_recv() {
                Ok(results) => {
                    self.nodes.preview_job = None;
                    self.nodes
                        .atlas
                        .resize((nodes::ATLAS * nodes::ATLAS * 4) as usize, 0);
                    for result in results {
                        let Some(preview) = self.nodes.previews.get_mut(&result.id) else {
                            continue;
                        };
                        if preview.signature != result.signature {
                            continue;
                        }
                        let Some(node) = self.nodes.nodes.iter().find(|n| n.id == result.id) else {
                            continue;
                        };
                        let ox = node.cell as u32 % 8 * nodes::CELL;
                        let oy = node.cell as u32 / 8 * nodes::CELL;
                        for y in 0..nodes::CELL {
                            let dst = (((oy + y) * nodes::ATLAS + ox) * 4) as usize;
                            let src = (y * nodes::CELL * 4) as usize;
                            self.nodes.atlas[dst..dst + (nodes::CELL * 4) as usize]
                                .copy_from_slice(
                                    &result.pixels[src..src + (nodes::CELL * 4) as usize],
                                );
                        }
                        preview.ready = true;
                        preview.size = result.size;
                        self.nodes.atlas_dirty = true;
                        ui.request_repaint();
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.nodes.preview_job = None;
                    self.nodes.previews.clear();
                }
            }
        }
    }

    fn store_node_result(&mut self, id: &str, result: &PreparedOutput) {
        self.store_node_thumbnail(id, result.size, &result.thumbnail);
    }

    fn store_node_thumbnail(&mut self, id: &str, size: (u32, u32), pixels: &[u8]) {
        let Some(node) = self.nodes.nodes.iter().find(|n| n.id == id) else {
            return;
        };
        let cell = node.cell;
        self.nodes
            .atlas
            .resize((nodes::ATLAS * nodes::ATLAS * 4) as usize, 0);
        let ox = cell as u32 % 8 * nodes::CELL;
        let oy = cell as u32 / 8 * nodes::CELL;
        for y in 0..nodes::CELL {
            let dst = (((oy + y) * nodes::ATLAS + ox) * 4) as usize;
            let src = (y * nodes::CELL * 4) as usize;
            self.nodes.atlas[dst..dst + (nodes::CELL * 4) as usize]
                .copy_from_slice(&pixels[src..src + (nodes::CELL * 4) as usize]);
        }
        self.nodes.previews.insert(
            id.into(),
            Preview {
                _snapshot: None,
                signature: 0,
                size,
                ready: true,
                error: String::new(),
            },
        );
        self.nodes.atlas_dirty = true;
    }

    fn apply_node_output(
        &mut self,
        destination: Destination,
        frame: Option<u32>,
        result: PreparedOutput,
    ) -> Result<String, String> {
        if matches!(destination, Destination::Memory) {
            return Ok(format!(
                "Node output: {} × {} px → Memory",
                result.size.0, result.size.1
            ));
        }
        // Destination is captured at Run, never inferred from the active document.
        let index = match &destination {
            Destination::NewDocument => {
                let name = format!("Document {}", self.workspace.documents.len() + 1);
                self.workspace.add_document(&name)
            }
            Destination::Memory => unreachable!(),
            Destination::Layer(id) => {
                let index = self
                    .workspace
                    .documents
                    .iter()
                    .position(|d| &d.id == id)
                    .ok_or("Destination document was deleted; result discarded")?;
                self.workspace.ensure_document_loaded(index)?;
                if self.workspace.documents[index]
                    .project
                    .animation
                    .as_ref()
                    .map(|a| a.frame)
                    != frame
                {
                    return Err("Destination animation frame changed; run again".into());
                }
                index
            }
        };
        if index == self.workspace.active_document {
            self.finish_deformation(false);
            self.commit_transform();
            self.animation_clock = None;
            self.history.bind_animation_edits(self.workspace.active());
            self.history
                .insert_layer(self.workspace.active_mut(), result.layer, result.size);
            self.sync_layer_structure();
        } else {
            let document = &mut self.workspace.documents[index];
            let mut layer = result.layer;
            layer.identity = Default::default();
            if document.project.width == 0 || document.project.height == 0 {
                (document.project.width, document.project.height) = result.size;
            }
            document.project.layers.push(layer);
            document.project.active_layer = document.project.layers.len() - 1;
            document.mark_thumb_dirty();
            self.dirty = true;
        }
        Ok(format!(
            "Node output: {} × {} px → {}",
            result.size.0,
            result.size.1,
            self.workspace.documents[index].name
        ))
    }

    pub fn refresh_node_previews(&mut self) {
        if self.nodes.preview_job.is_some() {
            return;
        }
        let sources: Vec<_> = self
            .nodes
            .nodes
            .iter()
            .filter(|node| node.kind.is_image_source())
            .map(|node| (node.id.clone(), node.kind.clone()))
            .collect();
        let mut pending = Vec::new();
        for (id, kind) in sources {
            match self.node_kind_signature(&kind) {
                Ok(signature)
                    if self
                        .nodes
                        .previews
                        .get(&id)
                        .is_some_and(|p| p.signature == signature && p.error.is_empty()) =>
                {
                    continue;
                }
                Ok(signature) => match self.node_kind_snapshot(&kind) {
                    Ok(project) => {
                        self.nodes.previews.insert(
                            id.clone(),
                            Preview {
                                _snapshot: Some(project.clone()),
                                signature,
                                size: (project.width, project.height),
                                ready: false,
                                error: String::new(),
                            },
                        );
                        pending.push((id, signature, project));
                    }
                    Err(error) => {
                        self.nodes.previews.insert(
                            id,
                            Preview {
                                _snapshot: None,
                                signature: 0,
                                size: (0, 0),
                                ready: false,
                                error,
                            },
                        );
                    }
                },
                Err(error) => {
                    self.nodes.previews.insert(
                        id,
                        Preview {
                            _snapshot: None,
                            signature: 0,
                            size: (0, 0),
                            ready: false,
                            error,
                        },
                    );
                }
            }
        }
        if pending.is_empty() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        if std::thread::Builder::new()
            .name("node-previews".into())
            .spawn(move || {
                let results = pending
                    .into_iter()
                    .map(|(id, signature, project)| PreviewResult {
                        id,
                        signature,
                        size: (project.width, project.height),
                        pixels: nodes::preview(&project),
                    })
                    .collect();
                let _ = sender.send(results);
            })
            .is_ok()
        {
            self.nodes.preview_job = Some(receiver);
        } else {
            self.nodes.previews.clear();
            self.nodes.message = "Cannot start preview worker".into();
        }
    }
}

fn format_node_duration(duration: std::time::Duration) -> String {
    if duration.as_secs_f64() < 1.0 {
        format!("{} ms", duration.as_millis())
    } else {
        format!("{:.1} s", duration.as_secs_f64())
    }
}

fn asset_preview_signature(id: &str, width: u32, height: u32, pixels_hash: u64) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hash);
    (width, height, pixels_hash).hash(&mut hash);
    hash.finish()
}

fn preview_signature(project: &Project) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (project.width, project.height).hash(&mut hash);
    for layer in &project.layers {
        Arc::as_ptr(&layer.identity).hash(&mut hash);
        layer.visible.hash(&mut hash);
        layer.opacity.to_bits().hash(&mut hash);
        layer.pixels.offset.hash(&mut hash);
        // Tile buffers are copy-on-write; pointer identity changes after edits.
        // Sort coordinates so HashMap traversal order cannot invalidate the cache.
        let mut tiles: Vec<_> = layer.pixels.tiles.iter().collect();
        tiles.sort_unstable_by_key(|(key, _)| **key);
        for (key, tile) in tiles {
            key.hash(&mut hash);
            Arc::as_ptr(tile).hash(&mut hash);
        }
    }
    hash.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::IMAGE_PORT;
    use crate::raster::Raster;

    fn fixture() -> EditorApp {
        let mut app = EditorApp::default();
        app.project_mut()
            .import_rgba(&[50, 100, 150, 128].repeat(8), 4, 2)
            .unwrap();
        let source = Source {
            document: app.workspace.active_doc().id.clone(),
            layer: None,
        };
        app.nodes.add(NodeKind::ImageDocument(source));
        app.nodes.add(NodeKind::Resize { scale: 0.5 });
        app.nodes.add(NodeKind::Output(Destination::NewDocument));
        for (from, to) in [("node_1", "node_2"), ("node_2", "node_3")] {
            app.nodes.space.links.push(mega_ui::NodeLink {
                id: app.nodes.space.links.len() as u64 + 1,
                from_node: from.into(),
                to_node: to.into(),
                from_port: "image".into(),
                to_port: "image".into(),
                ty: IMAGE_PORT,
            });
        }
        app
    }

    fn finish(app: &mut EditorApp) {
        let mut ui = Ui::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.nodes.run.is_some() && std::time::Instant::now() < deadline {
            app.poll_nodes(&mut ui);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(app.nodes.run.is_none(), "Worker did not complete");
    }

    #[test]
    fn nodes_run_uses_snapshot_and_repeated_runs_create_separate_documents() {
        let mut app = fixture();
        app.run_node_output("node_3");
        assert!(app.nodes.run.is_some());
        app.project_mut().layers[0].pixels.write(0, 0, 0, [255; 4]);
        app.session.preview_graph = true;
        finish(&mut app);
        for id in ["node_1", "node_2", "node_3"] {
            assert!(app.nodes.space.node_annotation(id).is_some(), "missing timing for {id}");
            assert_eq!(app.nodes.space.node_progress(id), None);
        }
        assert!(app.session.preview_graph);
        assert_eq!(app.workspace.documents.len(), 2);
        assert_eq!((app.project().width, app.project().height), (4, 2));
        let created = &app.workspace.documents[1].project;
        assert_eq!((created.width, created.height), (2, 1));
        assert_eq!(created.layers[0].pixels.read(0, 0, 0)[3], 128);
        assert!(app.dirty);
        app.run_node_output("node_3");
        finish(&mut app);
        assert_eq!(app.workspace.documents.len(), 3);
        assert!(app.session.preview_graph);
    }

    #[test]
    fn nodes_highlight_tracks_worker_and_clears_after_failure() {
        let mut app = fixture();
        let (sender, receiver) = mpsc::channel();
        let current_node = Arc::new(std::sync::Mutex::new(None));
        let progress = Arc::new(std::sync::Mutex::new(nodes::NodeProgress::default()));
        let times = Arc::new(std::sync::Mutex::new(nodes::ExecutionTimes::default()));
        app.nodes.run = Some(Run {
            node: "node_3".into(),
            destination: Some(Destination::Memory),
            destination_frame: None,
            receiver,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            current_node: current_node.clone(),
            live_preview: Arc::new(std::sync::Mutex::new(None)),
            progress: progress.clone(),
            times: times.clone(),
        });
        let mut ui = Ui::new();
        for id in ["node_1", "node_2"] {
            *current_node.lock().unwrap() = Some(id.into());
            *progress.lock().unwrap() = nodes::NodeProgress { node: id.into(), message: "Generating".into(), fraction: Some(0.4) };
            times.lock().unwrap().running = Some((id.into(), std::time::Instant::now()));
            app.poll_nodes(&mut ui);
            assert_eq!(app.nodes.space.running_nodes.len(), 1);
            assert!(app.nodes.space.running_nodes.contains(id));
            assert_eq!(app.nodes.space.node_progress(id), Some(0.4));
            assert!(app.nodes.space.node_annotation(id).is_some());
        }
        assert_eq!(app.nodes.space.node_progress("node_1"), None);
        sender.send(Err("test failure".into())).unwrap();
        app.poll_nodes(&mut ui);
        assert!(app.nodes.space.running_nodes.is_empty());
        assert_eq!(app.nodes.space.node_progress("node_2"), None);
        assert!(app.nodes.run.is_none());
    }

    #[test]
    fn nodes_failed_operation_keeps_its_execution_time() {
        let mut app = fixture();
        app.nodes.nodes[1].kind = NodeKind::Resize { scale: -1.0 };
        app.run_node_output("node_3");
        finish(&mut app);
        assert!(app.nodes.space.node_annotation("node_1").is_some());
        assert!(app.nodes.space.node_annotation("node_2").is_some());
        assert!(app.nodes.space.node_annotation("node_3").is_none());
        assert!(app.nodes.space.running_nodes.is_empty());
    }

    #[test]
    fn nodes_save_video_worker_result_as_asset() {
        let mut app = fixture();
        app.nodes.nodes[2].kind = NodeKind::Generate(nodes::Generation::LtxVideo {
            settings: Default::default(),
            scale: 1.0,
        });
        let bytes = include_bytes!("../../tests/fixtures/video-preview.mp4");
        let (sender, receiver) = mpsc::channel();
        sender.send(Ok(GraphOutput::Video(bytes.to_vec()))).unwrap();
        app.nodes.run = Some(Run {
            node: "node_3".into(),
            destination: None,
            destination_frame: None,
            receiver,
            cancel: Arc::default(),
            current_node: Arc::default(),
            live_preview: Arc::default(),
            progress: Arc::default(),
            times: Arc::default(),
        });
        app.poll_nodes(&mut Ui::new());
        assert_eq!(app.workspace.videos[0].read().unwrap(), bytes);
        assert_eq!(app.workspace.documents.len(), 1);
        assert!(app.dirty);
    }

    #[test]
    fn nodes_stop_discards_a_result_that_finished_before_polling() {
        let mut app = fixture();
        let result = nodes::evaluate(app.project().clone(), vec![]).unwrap();
        let (sender, receiver) = mpsc::channel();
        sender.send(Ok(GraphOutput::Image(result))).unwrap();
        app.nodes.run = Some(Run {
            node: "node_3".into(),
            destination: Some(Destination::NewDocument),
            destination_frame: None,
            receiver,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            current_node: Arc::new(std::sync::Mutex::new(Some("node_2".into()))),
            live_preview: Arc::new(std::sync::Mutex::new(None)),
            progress: Arc::new(std::sync::Mutex::new(nodes::NodeProgress::default())),
            times: Arc::new(std::sync::Mutex::new(nodes::ExecutionTimes::default())),
        });
        app.poll_nodes(&mut Ui::new());
        assert_eq!(app.workspace.documents.len(), 1);
        assert_eq!(app.nodes.message, "Graph stopped");
        assert!(app.nodes.space.running_nodes.is_empty());
    }

    #[test]
    fn nodes_memory_retains_result_and_preview_without_modifying_workspace() {
        let mut app = fixture();
        app.nodes.nodes[2].kind = NodeKind::Output(Destination::Memory);
        let before = app.workspace.active_doc().id.clone();
        app.dirty = false;
        app.run_node_output("node_3");
        finish(&mut app);
        assert_eq!(app.workspace.documents.len(), 1);
        assert_eq!(app.workspace.active_doc().id, before);
        assert!(!app.dirty);
        assert_eq!(app.nodes.memory["node_3"].pixels.read(0, 0, 0)[3], 128);
        assert_eq!(app.nodes.previews["node_3"].size, (2, 1));
        assert!(app.nodes.previews["node_3"].ready);
        app.nodes.space.request_delete_nodes.push("node_3".into());
        app.nodes.remove_requested();
        assert!(!app.nodes.memory.contains_key("node_3"));
    }

    #[test]
    fn nodes_layer_destination_is_captured_without_opening_the_document() {
        let mut app = fixture();
        let id = app.workspace.active_doc().id.clone();
        app.nodes.nodes[2].kind = NodeKind::Output(Destination::Layer(id.clone()));
        app.run_node_output("node_3");
        app.new_document();
        let opened = app.workspace.active_doc().id.clone();
        finish(&mut app);
        assert_eq!(app.workspace.active_doc().id, opened);
        let dest = app
            .workspace
            .documents
            .iter()
            .find(|d| d.id == id)
            .unwrap();
        assert_eq!(dest.project.layers.len(), 2);
        assert_eq!((dest.project.width, dest.project.height), (4, 2));
        assert_eq!(dest.project.layers[1].pixels.bounds().unwrap().2, 2);
    }

    #[test]
    fn nodes_layer_destination_on_open_document_supports_undo_redo() {
        let mut app = fixture();
        let id = app.workspace.active_doc().id.clone();
        app.nodes.nodes[2].kind = NodeKind::Output(Destination::Layer(id.clone()));
        app.run_node_output("node_3");
        finish(&mut app);
        assert_eq!(app.workspace.active_doc().id, id);
        assert_eq!(app.project().layers.len(), 2);
        app.undo();
        assert_eq!(app.project().layers.len(), 1);
        app.redo();
        assert_eq!(app.project().layers.len(), 2);
    }

    #[test]
    fn nodes_deleted_destination_and_output_do_not_write_a_result() {
        let mut app = fixture();
        let id = app.workspace.active_doc().id.clone();
        app.nodes.nodes[2].kind = NodeKind::Output(Destination::Layer(id));
        app.run_node_output("node_3");
        app.new_document();
        app.delete_document(0);
        finish(&mut app);
        assert!(app.nodes.message.contains("deleted"));
        assert!(app.project().layers.is_empty());

        let mut app = fixture();
        app.run_node_output("node_3");
        app.nodes.nodes.pop();
        finish(&mut app);
        assert_eq!(app.workspace.documents.len(), 1);
        assert!(app.nodes.message.contains("discarded"));
    }

    #[test]
    fn nodes_cached_preview_detects_pixel_edits_with_unchanged_tile_coordinates() {
        let mut app = fixture();
        let project = app.project().clone();
        let signature = preview_signature(&project);
        app.nodes.previews.insert(
            "node_1".into(),
            Preview {
                _snapshot: Some(project),
                signature,
                size: (4, 2),
                ready: true,
                error: String::new(),
            },
        );
        app.project_mut().layers[0].pixels.write(0, 0, 0, [255; 4]);
        assert_ne!(signature, preview_signature(app.project()));
        app.refresh_node_previews();
        assert!(!app.nodes.previews["node_1"].ready);
        assert!(app.nodes.preview_job.is_some());
        // Resetting the project drops in-flight work and every graph reference.
        let mut ui = Ui::new();
        app.new_project(&mut ui);
        assert!(app.nodes.nodes.is_empty());
        assert!(app.nodes.preview_job.is_none());
    }

    #[test]
    fn nodes_asset_preview_keeps_signature_until_pixels_change() {
        let mut app = EditorApp::default();
        app.workspace
            .import_asset_rgba("Ref".into(), &[9, 8, 7, 255].repeat(8), 4, 2)
            .unwrap();
        let id = app.workspace.assets[0].id.clone();
        let signature = asset_preview_signature(
            &id,
            app.workspace.assets[0].width,
            app.workspace.assets[0].height,
            app.workspace.assets[0].pixels_hash,
        );
        app.nodes.add(NodeKind::ImageAsset { id });
        app.nodes.previews.insert(
            "node_1".into(),
            Preview {
                _snapshot: None,
                signature,
                size: (4, 2),
                ready: true,
                error: String::new(),
            },
        );
        app.refresh_node_previews();
        assert!(app.nodes.previews["node_1"].ready);
        assert!(app.nodes.preview_job.is_none());
        app.workspace.assets[0].pixels.splice(0..4, [1, 2, 3, 255]);
        app.workspace.assets[0].pixels_hash =
            crate::safe_save::fingerprint(&app.workspace.assets[0].pixels);
        app.refresh_node_previews();
        assert!(!app.nodes.previews["node_1"].ready);
        assert!(app.nodes.preview_job.is_some());
    }

    #[test]
    fn nodes_image_asset_runs_from_library_pixels_without_touching_documents() {
        let mut app = EditorApp::default();
        app.workspace
            .import_asset_rgba("Ref".into(), &[9, 8, 7, 255].repeat(8), 4, 2)
            .unwrap();
        let id = app.workspace.assets[0].id.clone();
        app.nodes.add(NodeKind::ImageAsset { id });
        app.nodes.add(NodeKind::Output(Destination::Memory));
        app.nodes.space.links.push(mega_ui::NodeLink {
            id: 1,
            from_node: "node_1".into(),
            to_node: "node_2".into(),
            from_port: "image".into(),
            to_port: "image".into(),
            ty: IMAGE_PORT,
        });
        let documents = app.workspace.documents.len();
        app.run_node_output("node_2");
        finish(&mut app);
        assert_eq!(app.workspace.documents.len(), documents);
        assert_eq!(app.nodes.previews["node_2"].size, (4, 2));
        assert_eq!(app.nodes.memory["node_2"].pixels.read(0, 0, 0), [9, 8, 7, 255]);
    }

    #[test]
    fn nodes_pasted_layer_cannot_alias_the_original_source_identity() {
        let mut app = fixture();
        let original = app.project().layers[0].identity.clone();
        let copied = app.copy_layer_data(0).unwrap();
        app.insert_copied_layer(copied);
        assert!(!Arc::ptr_eq(&original, &app.project().layers[1].identity));
        app.project_mut().layers.remove(0);
        let source = Source {
            document: app.workspace.active_doc().id.clone(),
            layer: Some(original),
        };
        assert!(app.node_source_snapshot(&source).is_err());
    }
}
