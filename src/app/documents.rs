use super::*;

impl EditorApp {
    pub fn new_project(&mut self, ui: &mut Ui) {
        self.close_asset_preview();
        self.cancel_transform();
        self.workspace = Workspace::default();
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
        self.dirty = true;
        self.status = "New project created".into();
        ui.notify_success("New project created");
    }

    pub fn import_image(&mut self, ui: &mut Ui) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Images", &["png", "webp"])
            .pick_file()
        else {
            return;
        };
        match self.workspace.import_asset_image(&path) {
            Ok(_) => {
                self.dirty = true;
                self.status = format!("Imported asset {}", file_name(&path));
                ui.notify_success("Image imported as asset (not placed on document)");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub(in crate::app) fn copy_layer_text(&mut self, index: usize) -> Result<String, String> {
        self.commit_transform();
        let project = self.project();
        let layer = project.layers.get(index).ok_or("Choose a layer to copy")?;
        crate::layer_clipboard::encode(layer, (project.width, project.height))
    }

    pub fn copy_layer(&mut self, ui: &mut Ui) {
        self.copy_layer_at(self.project().active_layer, ui);
    }

    pub fn copy_layer_at(&mut self, index: usize, ui: &mut Ui) {
        match self.copy_layer_text(index).and_then(crate::layer_clipboard::write) {
            Ok(()) => self.status = "Layer copied with position and properties".into(),
            Err(error) => self.show_error(ui, error),
        }
    }

    pub(in crate::app) fn insert_copied_layer(&mut self, copied: crate::layer_clipboard::CopiedLayer) {
        self.commit_transform();
        self.close_asset_preview();
        let was_empty = self.image_size.is_none();
        self.history.insert_layer(self.workspace.active_mut(), copied.layer, copied.canvas_size);
        self.sync_layer_structure();
        if was_empty { self.session.fit_image = true; }
        self.status = "Layer pasted with position and properties".into();
    }

    pub fn paste_layer(&mut self, ui: &mut Ui) {
        match crate::layer_clipboard::read() {
            Ok(Some(copied)) => self.insert_copied_layer(copied),
            Ok(None) => self.show_error(ui, "Clipboard does not contain a copied layer".into()),
            Err(error) => self.show_error(ui, error),
        }
    }

    pub(in crate::app) fn paste_clipboard(&mut self, ui: &mut Ui) {
        match crate::layer_clipboard::read() {
            Ok(Some(copied)) => self.insert_copied_layer(copied),
            Ok(None) => self.paste_image(ui),
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn paste_image(&mut self, ui: &mut Ui) {
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
        let path = self.workspace.path.clone().or_else(|| {
            rfd::FileDialog::new()
                .add_filter("Mega Image Project", &["mgimg"])
                .set_file_name("project.mgimg")
                .save_file()
                .map(ensure_project_extension)
        });
        let Some(path) = path else { return };
        self.commit_transform();
        match self.workspace.save(&path) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", file_name(&path));
                ui.notify_success("Project saved");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn optimize_project(&mut self, ui: &mut Ui) {
        match self.workspace.optimize() {
            Ok((before, after)) => {
                self.status = format!("Project optimized: {} → {} KiB", before / 1024, after / 1024);
                ui.notify_success("Saved project optimized");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn open_project(&mut self, ui: &mut Ui) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Mega Image Project", &["mgimg"])
            .pick_file()
        else {
            return;
        };
        match Workspace::open(&path) {
            Ok(workspace) => {
                self.cancel_transform();
                self.workspace = workspace;
                match self.refresh_image() {
                    Ok(()) => {
                        self.dirty = false;
                        self.status = format!("Opened {}", file_name(&path));
                        ui.notify_success("Project opened");
                    }
                    Err(error) => self.show_error(ui, error),
                }
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn switch_document(&mut self, index: usize) {
        self.close_asset_preview();
        if index == self.workspace.active_document {
            return;
        }
        self.cancel_transform();
        self.workspace.set_active_document(index);
        self.history.clear();
        let _ = self.refresh_image();
        self.dirty = true;
        self.status = format!("Document: {}", self.workspace.active_doc().name);
    }

    pub fn place_asset(&mut self, asset_index: usize, ui: &mut Ui) {
        self.close_asset_preview();
        self.cancel_transform();
        match self.workspace.place_asset_on_active(asset_index) {
            Ok(()) => {
                self.dirty = true;
                let _ = self.refresh_image();
                self.status = "Asset placed on document".into();
                ui.notify_success("Asset placed as layer");
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
        let format = if webp { image::ImageFormat::WebP } else { image::ImageFormat::Png };
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
