use super::*;
use crate::nodes::{
    self, Destination, NodeKind, PreparedOutput, Preview, PreviewResult, Run, Source,
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

    pub fn run_node_output(&mut self, id: &str) {
        if self.nodes.run.is_some() {
            return;
        }
        self.commit_transform();
        self.finish_deformation(false);
        let prepared = (|| {
            let (source, scales, destination) =
                nodes::plan(&self.nodes.nodes, &self.nodes.space.links, id)?;
            let mut destination_frame = None;
            if let Destination::Layer(document) = &destination {
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
            let snapshot = self.node_source_snapshot(&source)?;
            let config = if scales
                .iter()
                .any(|op| matches!(op, nodes::Operation::Lineart(_)))
            {
                Some(crate::comfyui::HostConfig::load()?)
            } else {
                None
            };
            Ok::<_, String>((snapshot, scales, destination, destination_frame, config))
        })();
        match prepared {
            Ok((snapshot, scales, destination, destination_frame, config)) => {
                let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let worker_cancel = cancel.clone();
                let (sender, receiver) = mpsc::channel();
                match std::thread::Builder::new()
                    .name("node-pipeline".into())
                    .spawn(move || {
                        let result = std::panic::catch_unwind(|| {
                            nodes::evaluate_operations(snapshot, scales, config, &worker_cancel)
                        })
                        .unwrap_or_else(|_| Err("Pipeline worker failed".into()));
                        let _ = sender.send(result);
                    }) {
                    Ok(_) => {
                        self.nodes.message = "Running…".into();
                        self.nodes.space.running_nodes.insert(id.into());
                        self.nodes.run = Some(Run {
                            node: id.into(),
                            destination,
                            destination_frame,
                            receiver,
                            cancel,
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
            let result = match run.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Pipeline worker stopped".into()))
                }
            };
            if let Some(result) = result {
                let run = self.nodes.run.take().unwrap();
                self.nodes.space.running_nodes.clear();
                let result = if !self.nodes.nodes.iter().any(|n| n.id == run.node) {
                    Err("Output was deleted; result discarded".into())
                } else {
                    result.and_then(|result| {
                        if matches!(run.destination, Destination::Memory) {
                            self.nodes
                                .memory
                                .insert(run.node.clone(), result.layer.clone());
                        } else {
                            self.nodes.memory.remove(&run.node);
                        }
                        self.store_node_result(&run.node, &result);
                        self.apply_node_output(
                            run.destination.clone(),
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
                        self.nodes.message = error.clone();
                        ui.notify_error(&error);
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
                .copy_from_slice(&result.thumbnail[src..src + (nodes::CELL * 4) as usize]);
        }
        self.nodes.previews.insert(
            id.into(),
            Preview {
                _snapshot: None,
                signature: 0,
                size: result.size,
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
        let target = match &destination {
            Destination::NewDocument => None,
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
                Some(index)
            }
        };
        self.finish_deformation(false);
        self.commit_transform();
        self.animation_clock = None;
        if let Some(index) = target {
            if index != self.workspace.active_document {
                self.switch_document(index);
            }
        } else {
            self.new_document();
        }
        self.close_asset_preview();
        self.history.bind_animation_edits(self.workspace.active());
        self.history
            .insert_layer(self.workspace.active_mut(), result.layer, result.size);
        self.sync_layer_structure();
        self.session.camera_selected = false;
        self.session.fit_image = true;
        Ok(format!(
            "Node output: {} × {} px → {}",
            result.size.0,
            result.size.1,
            self.workspace.active_doc().name
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
            .filter_map(|node| {
                if let NodeKind::Image(source) = &node.kind {
                    Some((node.id.clone(), source.clone()))
                } else {
                    None
                }
            })
            .collect();
        let mut pending = Vec::new();
        for (id, source) in sources {
            match self.node_source_snapshot(&source) {
                Ok(project) => {
                    let signature = preview_signature(&project);
                    if self
                        .nodes
                        .previews
                        .get(&id)
                        .is_some_and(|p| p.signature == signature && p.error.is_empty())
                    {
                        continue;
                    }
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
        app.nodes.add(NodeKind::Image(source));
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
        finish(&mut app);
        assert_eq!(app.workspace.documents.len(), 2);
        assert_eq!((app.project().width, app.project().height), (2, 1));
        assert_eq!(app.project().layers[0].pixels.read(0, 0, 0)[3], 128);
        assert!(app.dirty);
        app.run_node_output("node_3");
        finish(&mut app);
        assert_eq!(app.workspace.documents.len(), 3);
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
    fn nodes_layer_destination_is_captured_and_insertion_supports_undo_redo() {
        let mut app = fixture();
        let id = app.workspace.active_doc().id.clone();
        app.nodes.nodes[2].kind = NodeKind::Output(Destination::Layer(id.clone()));
        app.run_node_output("node_3");
        app.new_document();
        finish(&mut app);
        assert_eq!(app.workspace.active_doc().id, id);
        assert_eq!(app.project().layers.len(), 2);
        assert_eq!((app.project().width, app.project().height), (4, 2));
        assert_eq!(app.project().layers[1].pixels.bounds().unwrap().2, 2);
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
