use super::*;
use crate::nodes::{GraphEntry, NodeEditor};

impl EditorApp {
    pub fn switch_graph(&mut self, index: usize) {
        if index >= self.workspace.graphs.entries.len() || index == self.workspace.graphs.active {
            return;
        }
        let Some(mut next) = self.workspace.graphs.entries[index].editor.take() else {
            return;
        };
        next.visible = self.nodes.visible;
        next.atlas_dirty = !next.atlas.is_empty();
        next.live_dirty = next.live_display.is_some();
        let old = std::mem::replace(&mut self.nodes, next);
        self.workspace.graphs.entries[self.workspace.graphs.active].editor = Some(old);
        self.workspace.graphs.active = index;
    }

    pub fn close_graph_view(&mut self) {
        self.session.preview_graph = false;
    }

    fn select_active_graph(&mut self) {
        if let Some(graph) = self.workspace.graphs.entries.get(self.workspace.graphs.active) {
            self.workspace.tree_selected = Some(format!("graph_{}", graph.id));
        }
    }

    pub(crate) fn stash_active_graph(&mut self) {
        let index = self.workspace.graphs.active;
        if self.workspace.graphs.entries.get(index).is_none() {
            return;
        }
        self.workspace.graphs.entries[index].editor = Some(std::mem::take(&mut self.nodes));
    }

    pub(crate) fn unstash_active_graph(&mut self) {
        let index = self.workspace.graphs.active;
        self.nodes = self
            .workspace
            .graphs
            .entries
            .get_mut(index)
            .and_then(|entry| entry.editor.take())
            .unwrap_or_default();
    }

    pub(crate) fn install_active_graph(&mut self) {
        if self.workspace.graphs.entries.is_empty() {
            self.workspace.graphs = Default::default();
        }
        self.workspace.graphs.active = self
            .workspace
            .graphs
            .active
            .min(self.workspace.graphs.entries.len().saturating_sub(1));
        self.unstash_active_graph();
    }

    pub fn show_graph(&mut self, index: usize) {
        self.close_asset_preview();
        self.switch_graph(index);
        self.session.preview_graph = true;
        self.select_active_graph();
    }

    pub fn new_graph(&mut self) {
        let n = self.workspace.graphs.next_id;
        self.workspace.graphs.next_id += 1;
        self.workspace.graphs.entries.push(GraphEntry {
            id: format!("g_{n}"),
            name: format!("Graph {n}"),
            editor: Some(NodeEditor::default()),
        });
        self.dirty = true;
        self.show_graph(self.workspace.graphs.entries.len() - 1);
    }

    pub fn clone_graph(&mut self, index: usize) {
        let Some(entry) = self.workspace.graphs.entries.get(index) else {
            return;
        };
        let copy = if index == self.workspace.graphs.active {
            self.nodes.duplicate()
        } else if let Some(editor) = &entry.editor {
            editor.duplicate()
        } else {
            return;
        };
        let name = format!("{} copy", entry.name);
        let n = self.workspace.graphs.next_id;
        self.workspace.graphs.next_id += 1;
        self.workspace.graphs.entries.push(GraphEntry {
            id: format!("g_{n}"),
            name,
            editor: Some(copy),
        });
        self.dirty = true;
        self.show_graph(self.workspace.graphs.entries.len() - 1);
    }

    pub fn delete_graph(&mut self, index: usize) {
        if index >= self.workspace.graphs.entries.len() {
            return;
        }
        self.dirty = true;
        if self.workspace.graphs.entries.len() == 1 {
            self.nodes = NodeEditor::default();
            self.workspace.graphs = Default::default();
            if self.session.preview_graph {
                self.select_active_graph();
            }
            return;
        }
        if index == self.workspace.graphs.active {
            let fallback = if index == 0 { 1 } else { index - 1 };
            self.switch_graph(fallback);
        }
        self.workspace.graphs.entries.remove(index);
        if index < self.workspace.graphs.active {
            self.workspace.graphs.active -= 1;
        }
        if self.session.preview_graph {
            self.select_active_graph();
        }
    }

    /// Poll inactive graphs too: switching views must not discard an AI job.
    pub(in crate::app) fn poll_all_graphs(&mut self, ui: &mut Ui) {
        self.poll_nodes(ui);
        for index in 0..self.workspace.graphs.entries.len() {
            if index == self.workspace.graphs.active {
                continue;
            }
            let Some(mut inactive) = self.workspace.graphs.entries[index].editor.take() else {
                continue;
            };
            std::mem::swap(&mut self.nodes, &mut inactive);
            self.poll_nodes(ui);
            std::mem::swap(&mut self.nodes, &mut inactive);
            self.workspace.graphs.entries[index].editor = Some(inactive);
        }
    }

    pub(in crate::app) fn graphs_need_repaint(&self) -> bool {
        self.nodes.run.is_some()
            || self.nodes.preview_job.is_some()
            || self
                .workspace
                .graphs
                .entries
                .iter()
                .filter_map(|g| g.editor.as_ref())
                .any(|g| g.run.is_some() || g.preview_job.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::{Destination, NodeKind, Source};

    #[test]
    fn graphs_create_switch_clone_and_delete_independently() {
        let mut app = EditorApp::default();
        let document = app.workspace.active_doc().id.clone();
        app.nodes.add(NodeKind::ImageDocument(Source { document: document.clone(), layer: None }));
        app.nodes.add(NodeKind::Resize { scale: 0.5 });
        app.nodes.space.pan = Vec2::new(100.0, 50.0);
        app.nodes.space.links.push(mega_ui::NodeLink {
            id: 1, from_node: "node_1".into(), from_port: "image".into(),
            to_node: "node_2".into(), to_port: "image".into(), ty: crate::nodes::IMAGE_PORT,
        });
        app.new_graph();
        assert_eq!(app.workspace.graphs.active, 1);
        assert!(app.nodes.nodes.is_empty());
        app.nodes.add(NodeKind::Output(Destination::Memory));
        app.switch_graph(0);
        assert_eq!(app.nodes.nodes.len(), 2);
        assert_eq!(app.nodes.space.pan, Vec2::new(100.0, 50.0));
        app.clone_graph(0);
        assert_eq!(app.workspace.graphs.entries.len(), 3);
        assert_eq!(app.nodes.space.links.len(), 1);
        assert_eq!(app.nodes.nodes.len(), 2);
        assert!(matches!(&app.nodes.nodes[0].kind, NodeKind::ImageDocument(s) if s.document == document));
        app.nodes.nodes.pop();
        app.switch_graph(0);
        assert_eq!(app.nodes.nodes.len(), 2);
        app.delete_graph(2);
        assert_eq!(app.workspace.graphs.entries.len(), 2);
        app.delete_graph(0);
        assert_eq!(app.workspace.graphs.active, 0);
        assert_eq!(app.nodes.nodes.len(), 1);
        app.delete_graph(0);
        assert_eq!(app.workspace.graphs.entries.len(), 1);
        assert!(app.nodes.nodes.is_empty());
    }

    #[test]
    fn graphs_switch_keeps_running_job_and_polls_inactive_graph() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[10, 20, 30, 255], 1, 1).unwrap();
        let document = app.workspace.active_doc().id.clone();
        app.nodes.add(NodeKind::ImageDocument(Source { document, layer: None }));
        app.nodes.add(NodeKind::Output(Destination::Memory));
        app.nodes.space.links.push(mega_ui::NodeLink {
            id: 1, from_node: "node_1".into(), from_port: "image".into(),
            to_node: "node_2".into(), to_port: "image".into(), ty: crate::nodes::IMAGE_PORT,
        });
        app.run_node_output("node_2");
        assert!(app.nodes.run.is_some());
        app.new_graph();
        assert!(app.nodes.run.is_none());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.workspace.graphs.entries[0].editor.as_ref().unwrap().run.is_some() && std::time::Instant::now() < deadline {
            app.poll_all_graphs(&mut Ui::new());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(app.workspace.graphs.entries[0].editor.as_ref().unwrap().run.is_none());
        assert!(app.nodes.memory.is_empty());
        app.switch_graph(0);
        assert!(app.nodes.memory.contains_key("node_2"));
        assert!(app.nodes.previews["node_2"].ready);
        assert!(app.nodes.atlas_dirty);
        assert_eq!(app.workspace.documents.len(), 1);
    }

    #[test]
    fn graph_rename_targets_id_and_marks_the_project_dirty() {
        let mut app = EditorApp::default();
        app.new_graph();
        let id = app.workspace.graphs.entries[0].id.clone();
        app.open_rename(crate::app::RenameTarget::Graph(id.clone()));
        app.rename.as_mut().unwrap().name = "Character Transfer".into();
        app.dirty = false;
        app.apply_rename();
        assert!(app.dirty);
        assert!(app.rename.is_none());
        assert_eq!(app.workspace.graphs.entries[0].name, "Character Transfer");
        assert_eq!(app.workspace.graphs.entries[1].name, "Graph 2");
        app.open_rename(crate::app::RenameTarget::Graph(id.clone()));
        app.rename.as_mut().unwrap().name = "   ".into();
        app.apply_rename();
        assert_eq!(app.rename.as_ref().unwrap().name, "   ");
        app.rename.as_mut().unwrap().name = "Should not apply".into();
        app.delete_graph(0);
        app.dirty = false;
        app.apply_rename();
        assert!(app.rename.is_none());
        assert!(!app.dirty);
        assert!(app.workspace.graphs.entries.iter().all(|g| g.name != "Should not apply"));
    }

    #[test]
    fn library_graph_opens_workspace_view_without_changing_the_document() {
        let mut app = EditorApp::default();
        let document = app.workspace.active_doc().id.clone();
        app.workspace
            .import_asset_rgba("Ref".into(), &[1, 2, 3, 255], 1, 1)
            .unwrap();
        app.show_asset_preview(0);
        assert!(app.session.preview_asset.is_some());
        app.show_graph(0);
        assert!(app.session.preview_graph);
        assert!(app.session.preview_asset.is_none());
        assert_eq!(app.workspace.active_doc().id, document);
        assert_eq!(app.workspace.tree_selected.as_deref(), Some("graph_g_1"));
        app.switch_document(0);
        assert!(!app.session.preview_graph);
        assert_eq!(app.workspace.active_doc().id, document);
    }

    #[test]
    fn graphs_save_and_open_restore_node_document_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graphs.mgimg");
        let mut app = EditorApp::default();
        let document = app.workspace.active_doc().id.clone();
        app.nodes.add(NodeKind::ImageDocument(Source {
            document: document.clone(),
            layer: None,
        }));
        app.nodes.add(NodeKind::Output(Destination::Memory));
        app.nodes.space.pan = Vec2::new(40.0, 8.0);
        app.nodes.space.links.push(mega_ui::NodeLink {
            id: 1,
            from_node: "node_1".into(),
            from_port: "image".into(),
            to_node: "node_2".into(),
            to_port: "image".into(),
            ty: crate::nodes::IMAGE_PORT,
        });
        app.workspace.path = Some(path.clone());
        app.stash_active_graph();
        app.workspace.save(&path).unwrap();
        app.unstash_active_graph();
        app.open_path(&path).unwrap();
        assert_eq!(app.workspace.graphs.entries.len(), 1);
        assert_eq!(app.workspace.graphs.entries[0].id, "g_1");
        assert_eq!(app.nodes.space.pan, Vec2::new(40.0, 8.0));
        assert_eq!(app.nodes.nodes.len(), 2);
        assert!(matches!(
            &app.nodes.nodes[0].kind,
            NodeKind::ImageDocument(source) if source.document == document
        ));
        assert_eq!(app.nodes.space.links.len(), 1);
    }
}
