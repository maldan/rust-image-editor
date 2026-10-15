use std::path::{Path, PathBuf};

use glam::Vec2;
use mega_ui::wgpu::{DrawStats, UiRenderer};
use mega_ui::{Rect, Theme, Ui, Window};

use crate::host::{FrameInput, Scene};
use crate::project::Project;

const IMAGE_TEXTURE: u32 = 10;

pub struct EditorApp {
    project: Project,
    input: FrameInput,
    image_size: Option<(u32, u32)>,
    pending_texture: Option<(Vec<u8>, u32, u32)>,
    canvas_rect: Rect,
    pan: Vec2,
    zoom: f32,
    fit_image: bool,
    dirty: bool,
    status: String,
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
            status: "Create a project, then import a PNG".into(),
        }
    }
}

impl EditorApp {
    fn new_project(&mut self, ui: &mut Ui) {
        self.project = Project::default();
        self.image_size = None;
        self.pending_texture = None;
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        self.dirty = true;
        self.status = "New project created".into();
        ui.notify_success("New project created");
    }

    fn import_png(&mut self, ui: &mut Ui) {
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

    fn save_project(&mut self, ui: &mut Ui) {
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

    fn open_project(&mut self, ui: &mut Ui) {
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

    fn export_png(&mut self, ui: &mut Ui) {
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

    fn update_view(&mut self) {
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

    fn fit_to_canvas(&mut self) {
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

    fn draw_canvas(&mut self, ui: &mut Ui) {
        let available = ui.available_size().max(Vec2::splat(1.0));
        let origin = Vec2::new(8.0, 64.0);
        self.canvas_rect = Rect::from_min_size(
            origin,
            Vec2::new((available.x - 8.0).max(1.0), (available.y - 8.0).max(1.0)),
        );
        ui.fill_rect(self.canvas_rect, [0.075, 0.08, 0.095, 1.0]);
        if self.fit_image {
            self.fit_to_canvas();
        }
        self.update_view();
        if let Some((width, height)) = self.image_size {
            let size = Vec2::new(width as f32, height as f32) * self.zoom;
            let center = self.canvas_rect.min + rect_size(self.canvas_rect) * 0.5 + self.pan;
            let image_rect = Rect::from_min_size(center - size * 0.5, size);
            let shadow = Rect {
                min: image_rect.min - Vec2::splat(1.0),
                max: image_rect.max + Vec2::splat(1.0),
            };
            if let Some(visible_shadow) = shadow.intersect(self.canvas_rect) {
                ui.fill_rect(visible_shadow, [0.0, 0.0, 0.0, 0.8]);
            }
            ui.texture_rect(IMAGE_TEXTURE, image_rect, [0.0, 0.0], [1.0, 1.0]);
        } else {
            let message = "Import a PNG to begin";
            let pos =
                self.canvas_rect.min + rect_size(self.canvas_rect) * 0.5 - Vec2::new(75.0, 8.0);
            ui.text_at(pos, message);
        }
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
        ui.menu_bar(|ui| {
            ui.menu("file", "File", |ui| {
                if ui.menu_item("new", "New Project").clicked() {
                    self.new_project(ui);
                }
                if ui.menu_item("open", "Open Project…").clicked() {
                    self.open_project(ui);
                }
                if ui.menu_item("save", "Save Project").clicked() {
                    self.save_project(ui);
                }
                ui.separator();
                if ui.menu_item("import", "Import PNG…").clicked() {
                    self.import_png(ui);
                }
                if ui.menu_item("export", "Export PNG…").clicked() {
                    self.export_png(ui);
                }
            });
        });

        let side_width = 260.0;
        ui.window(
            Window::new("canvas", "Canvas")
                .pos(Vec2::new(0.0, 24.0))
                .size(Vec2::new(
                    (viewport.x - side_width).max(320.0),
                    (viewport.y - 48.0).max(300.0),
                )),
            |ui| self.draw_canvas(ui),
        );
        ui.window(
            Window::new("project", "Project")
                .pos(Vec2::new((viewport.x - side_width).max(320.0), 24.0))
                .size(Vec2::new(side_width, (viewport.y - 48.0).max(300.0))),
            |ui| {
                ui.label("Assets");
                ui.separator();
                match self.image_size {
                    Some((width, height)) => {
                        ui.label("  main/image.png");
                        ui.label(&format!("  {width} × {height}"));
                    }
                    None => ui.label("  No imported assets"),
                }
                ui.space(14.0);
                ui.label("Layers");
                ui.separator();
                if self.image_size.is_some() {
                    ui.label("  Image");
                } else {
                    ui.label("  Empty");
                }
                ui.space(14.0);
                if ui.button("fit", "Fit image").clicked() {
                    self.fit_image = true;
                }
                ui.label(&format!("Zoom: {:.0}%", self.zoom * 100.0));
                ui.space(14.0);
                let name = self
                    .project
                    .path
                    .as_deref()
                    .map(file_name)
                    .unwrap_or("Unsaved project");
                ui.label(name);
                if self.dirty {
                    ui.label("Modified");
                }
            },
        );
        ui.status_bar(|ui| {
            ui.label(&self.status);
            ui.spacer();
            ui.label("Middle drag: pan · Wheel: zoom");
        });
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

fn file_name(path: &Path) -> &str {
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

fn rect_size(rect: Rect) -> Vec2 {
    rect.max - rect.min
}
