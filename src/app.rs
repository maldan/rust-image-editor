use std::path::{Path, PathBuf};

use glam::Vec2;
use mega_ui::wgpu::{DrawStats, UiRenderer};
use mega_ui::{DockState, Rect, Theme, Ui};

use crate::host::{FrameInput, Scene};
use crate::project::Project;
use crate::ui;

pub const IMAGE_TEXTURE: u32 = 10;

pub struct EditorApp {
    pub project: Project,
    pub input: FrameInput,
    pub image_size: Option<(u32, u32)>,
    pub pending_texture: Option<(Vec<u8>, u32, u32)>,
    pub canvas_rect: Rect,
    pub pan: Vec2,
    pub zoom: f32,
    pub fit_image: bool,
    pub dirty: bool,
    pub status: String,
    pub dock: DockState,
}

impl Default for EditorApp {
    fn default() -> Self {
        Self {
            project: Project::default(),
            input: FrameInput::default(),
            image_size: None,
            pending_texture: None,
            canvas_rect: Rect::from_min_size(Vec2::ZERO, Vec2::ZERO),
            pan: Vec2::ZERO,
            zoom: 1.0,
            fit_image: false,
            dirty: false,
            status: "Create a project, then import or paste an image".into(),
            dock: ui::default_dock(),
        }
    }
}

impl EditorApp {
    pub fn new_project(&mut self, ui: &mut Ui) {
        self.project = Project::default();
        self.image_size = None;
        self.pending_texture = None;
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        self.dirty = true;
        self.status = "New project created".into();
        ui.notify_success("New project created");
    }

    pub fn import_png(&mut self, ui: &mut Ui) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .pick_file()
        else {
            return;
        };
        match self
            .project
            .import_png(&path)
            .and_then(|_| self.refresh_image())
        {
            Ok(()) => {
                self.dirty = true;
                self.status = format!("Imported {}", file_name(&path));
                ui.notify_success("PNG imported into the project");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn paste_image(&mut self, ui: &mut Ui) {
        match paste_image_from_clipboard().and_then(|(pixels, width, height)| {
            self.project.import_rgba(&pixels, width, height)?;
            self.refresh_image()
        }) {
            Ok(()) => {
                self.dirty = true;
                self.status = "Pasted image from clipboard".into();
                ui.notify_success("Image pasted from clipboard");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    pub fn save_project(&mut self, ui: &mut Ui) {
        let path = self.project.path.clone().or_else(|| {
            rfd::FileDialog::new()
                .add_filter("Mega Image Project", &["mgimg"])
                .set_file_name("project.mgimg")
                .save_file()
                .map(ensure_project_extension)
        });
        let Some(path) = path else { return };
        match self.project.save(&path) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", file_name(&path));
                ui.notify_success("Project saved");
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
        match Project::open(&path) {
            Ok(project) => {
                self.project = project;
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

    pub fn export_png(&mut self, ui: &mut Ui) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name("export.png")
            .save_file()
            .map(ensure_png_extension)
        else {
            return;
        };
        match self.project.export_png(&path) {
            Ok(()) => {
                self.status = format!("Exported {}", file_name(&path));
                ui.notify_success("PNG exported");
            }
            Err(error) => self.show_error(ui, error),
        }
    }

    fn refresh_image(&mut self) -> Result<(), String> {
        match self.project.decoded_rgba()? {
            Some((pixels, width, height)) => {
                self.image_size = Some((width, height));
                self.pending_texture = Some((pixels, width, height));
                self.fit_image = true;
            }
            None => {
                self.image_size = None;
                self.pending_texture = None;
            }
        }
        Ok(())
    }

    fn show_error(&mut self, ui: &mut Ui, error: String) {
        self.status = error.clone();
        ui.notify_error(&error);
    }

    pub fn update_view(&mut self) {
        if !self.canvas_rect.contains(self.input.mouse_pos) {
            return;
        }
        if self.input.mouse_middle_down {
            self.pan += self.input.mouse_delta;
        }
        if self.input.scroll_delta.y.abs() > 0.01 {
            let old_zoom = self.zoom;
            let factor = (self.input.scroll_delta.y * 0.002).exp();
            self.zoom = (self.zoom * factor).clamp(0.05, 32.0);
            let center = self.canvas_rect.min + rect_size(self.canvas_rect) * 0.5;
            let cursor_from_image_center = self.input.mouse_pos - center - self.pan;
            self.pan -= cursor_from_image_center * (self.zoom / old_zoom - 1.0);
        }
    }

    pub fn fit_to_canvas(&mut self) {
        let Some((width, height)) = self.image_size else {
            return;
        };
        let available = (rect_size(self.canvas_rect) - Vec2::splat(32.0)).max(Vec2::splat(1.0));
        self.zoom = (available.x / width as f32)
            .min(available.y / height as f32)
            .clamp(0.05, 1.0);
        self.pan = Vec2::ZERO;
        self.fit_image = false;
    }
}

impl Scene for EditorApp {
    fn title() -> &'static str {
        "Mega Image Editor"
    }

    fn init(ui: &mut Ui) {
        ui.set_theme(Theme::dark());
        ui.load_builtin_icons();
    }

    fn begin_frame(&mut self, input: &FrameInput) {
        self.input = input.clone();
    }

    fn build(&mut self, ui: &mut Ui, viewport: Vec2, _stats: DrawStats) -> bool {
        let paste = self.input.key_paste;
        ui::draw(ui, self, viewport);
        if paste {
            self.paste_image(ui);
        }
        false
    }

    fn sync_gpu(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, renderer: &mut UiRenderer) {
        if let Some((pixels, width, height)) = self.pending_texture.take() {
            renderer.set_texture_rgba(
                device,
                queue,
                IMAGE_TEXTURE,
                &pixels,
                width,
                height,
                "project image",
            );
        }
    }
}

fn paste_image_from_clipboard() -> Result<(Vec<u8>, u32, u32), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("Cannot open clipboard: {e}"))?;
    let image = clipboard
        .get_image()
        .map_err(|_| "Clipboard does not contain an image".to_string())?;
    let width = u32::try_from(image.width).map_err(|_| "Clipboard image is too large".to_string())?;
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

pub fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project")
}

fn ensure_project_extension(path: PathBuf) -> PathBuf {
    ensure_extension(path, "mgimg")
}

fn ensure_png_extension(path: PathBuf) -> PathBuf {
    ensure_extension(path, "png")
}

fn ensure_extension(path: PathBuf, extension: &str) -> PathBuf {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
    {
        path
    } else {
        path.with_extension(extension)
    }
}

pub fn rect_size(rect: Rect) -> Vec2 {
    rect.max - rect.min
}
