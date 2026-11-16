use super::*;
use crate::raster::Raster;

impl EditorApp {
    pub fn request_new_project(&mut self, ui: &mut Ui) {
        if self.prompt_if_unsaved(UnsavedAction::NewProject) {
            return;
        }
        self.new_project(ui);
    }

    pub fn new_project(&mut self, ui: &mut Ui) {
        self.animation_clock = None;
        self.close_asset_preview();
        self.close_graph_view();
        self.video_poster = None;
        self.video_trim = Default::default();
        self.cancel_transform();
        self.workspace = Workspace::default();
        self.nodes = Default::default();
        self.ai.presets = Default::default();
        self.image_size = None;
        self.dirty_tiles.clear();
        self.dirty_layer = None;
        self.layers_reload = false;
        self.floating_reload = false;
        self.canvas_cleared = true;
        self.history.clear();
        self.clear_selection();
        self.session.pan = Vec2::ZERO;
        self.session.zoom = 1.0;
        self.session.rotation = 0.0;
        self.dirty = false;
        self.status = "New project created".into();
        ui.notify_success("New project created");
    }

    pub fn import_image(&mut self, ui: &mut Ui) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Assets", &["png", "webp", "mp4"])
            .add_filter("Images", &["png", "webp"])
            .add_filter("Videos", &["mp4"])
            .pick_file()
        else {
            return;
        };
        let video = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"));
        let result = if video {
            self.workspace.import_asset_video(&path)
        } else {
            self.workspace.import_asset_image(&path)
        };
        match result {
            Ok(_) => {
                self.dirty = true;
                self.status = format!("Imported asset {}", file_name(&path));
                ui.notify_success(if video {
                    "Video imported as asset"
                } else {
                    "Image imported as asset (not placed on document)"
                });
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub(in crate::app) fn copy_layer_data(&mut self, index: usize) -> Result<crate::layer_clipboard::CopiedLayer, String> {
        self.commit_transform();
        let project = self.workspace.active_mut();
        project.flush_drawings();
        let layer = project.layers.get(index).ok_or("Choose a layer to copy")?;
        Ok(crate::layer_clipboard::CopiedLayer {
            layer: layer.clone(),
            canvas_size: (project.width, project.height),
            fragment: false,
        })
    }

    pub fn copy_layer(&mut self, ui: &mut Ui) {
        self.copy_layer_at(self.project().active_layer, ui);
    }

    /// Ctrl+C: copy the selection when one exists, otherwise the active layer.
    pub fn copy_shortcut(&mut self, ui: &mut Ui) {
        if self
            .session
            .selection
            .as_ref()
            .is_some_and(|selection| !selection.is_empty())
        {
            self.copy_selection(ui);
        } else {
            self.copy_layer(ui);
        }
    }

    pub fn copy_selection(&mut self, ui: &mut Ui) {
        if self.session.floating.is_some() {
            self.status = "Apply or cancel the transform before copying".into();
            return;
        }
        match self.selection_clipboard_data() {
            Ok(copied) => {
                self.clipboard = Some(copied);
                self.status = "Selection copied".into();
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn cut_selection(&mut self, ui: &mut Ui) {
        if self.camera_visible() { return; }
        if self.session.floating.is_some() {
            self.status = "Apply or cancel the transform before cutting".into();
            return;
        }
        if self
            .session
            .selection
            .as_ref()
            .is_none_or(|selection| selection.is_empty())
        {
            self.status = "Select an area to cut".into();
            return;
        }
        let copied = match self.selection_clipboard_data() {
            Ok(copied) => copied,
            Err(error) => {
                self.show_error(ui, error);
                return;
            }
        };
        self.clipboard = Some(copied);
        if let Err(error) = self.erase_active_selection() {
            self.show_error(ui, error);
            return;
        }
        self.status = "Selection cut".into();
    }

    pub fn delete_selection(&mut self) {
        if self.camera_visible() { return; }
        if self.session.floating.is_some() {
            self.status = "Apply or cancel the transform before deleting".into();
            return;
        }
        if self
            .session
            .selection
            .as_ref()
            .is_none_or(|selection| selection.is_empty())
        {
            return;
        }
        match self.erase_active_selection() {
            Ok(()) => self.status = "Selection erased".into(),
            Err(error) => self.status = error,
        }
    }

    pub(in crate::app) fn selection_clipboard_data(&self) -> Result<crate::layer_clipboard::CopiedLayer, String> {
        let layer = self.capture_selection_layer()?;
        let (width, height) = self.image_size.ok_or("No image")?;
        Ok(crate::layer_clipboard::CopiedLayer {
            layer,
            canvas_size: (width, height),
            fragment: true,
        })
    }

    pub fn copy_layer_at(&mut self, index: usize, ui: &mut Ui) {
        match self.copy_layer_data(index) {
            Ok(copied) => {
                self.clipboard = Some(copied);
                self.status = "Layer copied with position and properties".into();
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn copy_layer_as_image(&mut self, index: usize, ui: &mut Ui) {
        let result = self.layer_clipboard_image(index).and_then(|image| {
            arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.set_image(image))
                .map_err(|error| format!("Cannot copy image to clipboard: {error}"))
        });
        match result {
            Ok(()) => self.status = "Layer copied as image to system clipboard".into(),
            Err(error) => self.show_error(ui, error),
        }
    }

    pub(in crate::app) fn layer_clipboard_image(
        &mut self,
        index: usize,
    ) -> Result<arboard::ImageData<'static>, String> {
        self.commit_transform();
        let layer = self.project().layers.get(index).ok_or("Choose a layer to copy")?;
        let (x, y, right, bottom) = layer.pixels.bounds().ok_or("Layer has no pixels")?;
        let width = x.abs_diff(right);
        let height = y.abs_diff(bottom);
        let bytes = (width as usize).checked_mul(height as usize)
            .and_then(|count| count.checked_mul(4))
            .ok_or("Layer image is too large for the clipboard")?;
        if bytes > 256 * 1024 * 1024 {
            return Err("Layer image is too large for the clipboard".into());
        }
        let mut pixels = layer.pixels.region(x, y, width, height);
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = (pixel[3] as f32 * layer.opacity).round() as u8;
        }
        Ok(arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: std::borrow::Cow::Owned(pixels),
        })
    }

    pub(in crate::app) fn insert_copied_layer(
        &mut self,
        copied: crate::layer_clipboard::CopiedLayer,
    ) {
        let fragment = copied.fragment;
        self.commit_transform();
        self.close_asset_preview();
        let was_empty = self.image_size.is_none();
        self.history.insert_layer(
            self.workspace.active_mut(),
            copied.layer,
            copied.canvas_size,
        );
        self.sync_layer_structure();
        if was_empty {
            self.session.fit_image = true;
        }
        self.status = "Layer pasted with position and properties".into();
        if fragment {
            self.clear_selection();
            if self.session.tool == Tool::Transform {
                self.begin_transform();
            } else {
                self.set_tool(Tool::Transform);
            }
            if self.session.floating.is_some() {
                self.status = "Pasted onto a new layer — drag inside to move, corners to scale, outside to rotate. Enter apply, Esc cancel".into();
            }
        }
    }

    fn capture_selection_layer(&self) -> Result<crate::project::Layer, String> {
        let (width, _) = self.image_size.ok_or("No image")?;
        let selection = self
            .session
            .selection
            .as_ref()
            .ok_or("Select an area first")?;
        let bounds = selection.bounds().ok_or("Selection is empty")?;
        let project = self.project();
        let source = project
            .layers
            .get(project.active_layer)
            .ok_or("Choose a layer")?;
        let mut pixels = crate::raster::SparsePixels::default();
        let mut any = false;
        for y in bounds.min_y..bounds.max_y {
            for x in bounds.min_x..bounds.max_x {
                if !selection.contains(x, y) {
                    continue;
                }
                let pixel = source.pixels.read(width, x as i32, y as i32);
                if pixel[3] == 0 {
                    continue;
                }
                pixels.write(width, x as i32, y as i32, pixel);
                any = true;
            }
        }
        if !any {
            return Err("Selection has no pixels".into());
        }
        Ok(crate::project::Layer {
            identity: Default::default(),
            ai_metadata: source.ai_metadata.clone(),
            z: source.z,
            drawings: Default::default(),
            name: "Selection".into(),
            visible: true,
            opacity: source.opacity,
            pixels,
        })
    }

    pub(in crate::app) fn erase_active_selection(&mut self) -> Result<(), String> {
        let (width, _) = self.image_size.ok_or("No image")?;
        let selection = self
            .session
            .selection
            .clone()
            .ok_or("Select an area to cut")?;
        let bounds = selection.bounds().ok_or("Selection is empty")?;
        let index = self.project().active_layer;
        {
            let pixels = &mut self
                .workspace
                .active_mut()
                .layers
                .get_mut(index)
                .ok_or("Choose a layer")?
                .pixels;
            let before = pixels.clone();
            let mut changed = false;
            for y in bounds.min_y..bounds.max_y {
                for x in bounds.min_x..bounds.max_x {
                    if !selection.contains(x, y) {
                        continue;
                    }
                    if pixels.read(width, x as i32, y as i32) == [0; 4] {
                        continue;
                    }
                    pixels.write(width, x as i32, y as i32, [0; 4]);
                    changed = true;
                }
            }
            if !changed {
                return Ok(());
            }
            pixels.prune();
            self.history.push_diff(index, &before, pixels);
        }
        self.mark_dirty_rect(bounds);
        self.dirty = true;
        Ok(())
    }

    pub fn paste_clipboard(&mut self, ui: &mut Ui) {
        let selected_document = self.workspace.documents.get(self.workspace.active_document)
            .is_some_and(|doc| {
                self.workspace.tree_selected.as_deref() == Some(format!("doc_{}", doc.id).as_str())
            });
        if !selected_document || self.session.preview_asset.is_some() {
            return;
        }
        if let Some(copied) = self.clipboard.clone() {
            self.insert_copied_layer(copied);
            return;
        }
        match paste_image_from_clipboard().and_then(|(pixels, width, height)| {
            self.insert_clipboard_image(&pixels, width, height)
        }) {
            Ok(()) => {},
            Err(error) => self.show_error(ui, error),
        }
    }

    pub(in crate::app) fn insert_clipboard_image(
        &mut self, pixels: &[u8], width: u32, height: u32,
    ) -> Result<(), String> {
        let mut image = Project::default();
        image.import_rgba(pixels, width, height)?;
        let mut layer = image.layers.remove(0);
        layer.name = "Clipboard".into();
        self.insert_copied_layer(crate::layer_clipboard::CopiedLayer {
            layer,
            canvas_size: (width, height),
            fragment: true,
        });
        Ok(())
    }

    pub fn import_clipboard_image(&mut self, ui: &mut Ui) {
        match paste_image_from_clipboard().and_then(|(pixels, width, height)| {
            self.workspace
                .import_asset_rgba("Clipboard".into(), &pixels, width, height)?;
            Ok(())
        }) {
            Ok(()) => {
                self.dirty = true;
                self.status = "Pasted image as asset".into();
                ui.notify_success("Clipboard image imported as asset");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn save_project(&mut self, ui: &mut Ui) {
        let _ = self.try_save_project(ui);
    }

    /// Returns `true` when the project was written and `dirty` cleared.
    pub fn try_save_project(&mut self, ui: &mut Ui) -> bool {
        let path = self.workspace.path.clone().or_else(|| {
            rfd::FileDialog::new()
                .add_filter("Mega Image Project", &["mgimg"])
                .set_file_name("project.mgimg")
                .save_file()
                .map(ensure_project_extension)
        });
        let Some(path) = path else {
            return false;
        };
        self.commit_transform();
        self.stash_active_graph();
        let result = self.workspace.save(&path);
        self.unstash_active_graph();
        match result {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", file_name(&path));
                ui.notify_success("Project saved");
                true
            }
            Err(error) => {
                self.show_error(ui, error);
                false
            }
        }
    }

    pub fn optimize_project(&mut self, ui: &mut Ui) {
        match self.workspace.optimize() {
            Ok((before, after)) => {
                self.status = format!(
                    "Project optimized: {} → {} KiB",
                    before / 1024,
                    after / 1024
                );
                ui.notify_success("Saved project optimized");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn open_path(&mut self, path: &Path) -> Result<(), String> {
        let mut workspace = Workspace::open(path)?;
        workspace.ensure_document_loaded(workspace.active_document)?;
        self.close_asset_preview();
        self.close_graph_view();
        self.video_poster = None;
        self.cancel_transform();
        self.workspace = workspace;
        self.install_active_graph();
        self.video_trim = Default::default();
        self.ai.presets = Default::default();
        self.refresh_image()?;
        self.dirty = false;
        self.status = format!("Opened {}", file_name(path));
        Ok(())
    }

    pub fn request_open_project(&mut self, ui: &mut Ui) {
        if self.prompt_if_unsaved(UnsavedAction::OpenProject) {
            return;
        }
        self.open_project(ui);
    }

    pub fn open_project(&mut self, ui: &mut Ui) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Mega Image Project", &["mgimg"])
            .pick_file()
        else {
            return;
        };
        match self.open_path(&path) {
            Ok(()) => ui.notify_success("Project opened"),
            Err(error) => self.show_error(ui, error),
        }
    }

    fn prompt_if_unsaved(&mut self, action: UnsavedAction) -> bool {
        if !self.dirty {
            return false;
        }
        if self.unsaved.is_none() {
            self.unsaved = Some(action);
        }
        true
    }

    pub fn cancel_unsaved_prompt(&mut self) {
        if self.unsaved == Some(UnsavedAction::Close) {
            self.close_aborted = true;
        }
        self.unsaved = None;
    }

    pub fn finish_unsaved_prompt(&mut self, action: UnsavedAction, ui: &mut Ui) {
        self.unsaved = None;
        match action {
            UnsavedAction::Close => {
                self.close_ready = true;
            }
            UnsavedAction::NewProject => self.new_project(ui),
            UnsavedAction::OpenProject => self.open_project(ui),
        }
    }

    pub fn switch_document(&mut self, index: usize) {
        self.animation_clock = None;
        if let Err(error) = self.workspace.ensure_document_loaded(index) {
            self.status = error;
            return;
        }
        self.close_asset_preview();
        self.close_graph_view();
        if index == self.workspace.active_document {
            return;
        }
        self.cancel_transform();
        self.workspace.set_active_document(index);
        self.history.discard_project_edits();
        if let Err(error) = self.reload_active_view() {
            self.status = error;
            return;
        }
        self.dirty = true;
        self.status = format!("Document: {}", self.workspace.active_doc().name);
    }

    pub fn new_document(&mut self) {
        self.animation_clock = None;
        self.cancel_transform();
        self.close_asset_preview();
        self.close_graph_view();
        let name = format!("Document {}", self.workspace.documents.len() + 1);
        self.workspace.create_document(&name);
        self.history.discard_project_edits();
        let _ = self.reload_active_view();
        self.dirty = true;
        self.status = format!("Document: {}", self.workspace.active_doc().name);
    }

    pub fn duplicate_document(&mut self, index: usize) {
        let Some(index) = self.workspace.duplicate_document(index) else {
            return;
        };
        self.switch_document(index);
        self.status = format!("Duplicated {}", self.workspace.active_doc().name);
    }

    pub fn delete_document(&mut self, index: usize) {
        if index >= self.workspace.documents.len() {
            return;
        }
        if index == self.workspace.active_document && self.workspace.documents.len() > 1 {
            let fallback = if index + 1 < self.workspace.documents.len() {
                index + 1
            } else {
                index - 1
            };
            if let Err(error) = self.workspace.ensure_document_loaded(fallback) {
                self.status = error;
                return;
            }
        }
        let active_id = self.workspace.documents[self.workspace.active_document]
            .id
            .clone();
        self.cancel_transform();
        self.close_asset_preview();
        let Some(document) = self.workspace.take_document(index) else {
            return;
        };
        let switched = document.id == active_id;
        let fallback_id = self.workspace.documents[self.workspace.active_document]
            .id
            .clone();
        self.history
            .push_delete_document(index, document, fallback_id, switched);
        if switched {
            let _ = self.reload_active_view();
        }
        self.dirty = true;
        self.status = "Document deleted".into();
    }

    pub fn place_asset(&mut self, asset_index: usize, ui: &mut Ui) {
        self.close_asset_preview();
        self.close_graph_view();
        self.cancel_transform();
        match self.workspace.place_asset_on_active(asset_index) {
            Ok(()) => {
                self.dirty = true;
                self.history.discard_project_edits();
                let _ = self.reload_active_view();
                self.status = "Asset placed on document".into();
                ui.notify_success("Asset placed as layer");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn create_document_from_asset(&mut self, asset_index: usize, ui: &mut Ui) {
        self.cancel_transform();
        match self.workspace.create_document_from_asset(asset_index) {
            Ok(_) => {
                self.animation_clock = None;
                self.close_asset_preview();
                self.history.discard_project_edits();
                let _ = self.reload_active_view();
                self.dirty = true;
                self.status = format!("Document: {}", self.workspace.active_doc().name);
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn export_image(&mut self, ui: &mut Ui, webp: bool) {
        let extension = if webp { "webp" } else { "png" };
        let Some(path) = rfd::FileDialog::new()
            .add_filter(if webp { "WebP image" } else { "PNG image" }, &[extension])
            .set_file_name(format!("export.{extension}"))
            .save_file()
            .map(|p| ensure_extension(p, extension))
        else {
            return;
        };
        let format = if webp {
            image::ImageFormat::WebP
        } else {
            image::ImageFormat::Png
        };
        match self.workspace.active_mut().export_image(&path, format) {
            Ok(()) => {
                self.status = format!("Exported {}", file_name(&path));
                ui.notify_success("Image exported");
            }
            Err(error) => self.show_error(ui, error),
        }
    }
}

fn paste_image_from_clipboard() -> Result<(Vec<u8>, u32, u32), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("Cannot open clipboard: {e}"))?;
    let image = clipboard
        .get_image()
        .map_err(|_| "Clipboard does not contain an image".to_string())?;
    let width =
        u32::try_from(image.width).map_err(|_| "Clipboard image is too large".to_string())?;
    let height =
        u32::try_from(image.height).map_err(|_| "Clipboard image is too large".to_string())?;
    if width == 0 || height == 0 {
        return Err("Clipboard image is empty".into());
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| "Clipboard image is too large".to_string())?;
    if image.bytes.len() < expected {
        return Err("Clipboard image data is incomplete".into());
    }
    Ok((image.bytes[..expected].to_vec(), width, height))
}
