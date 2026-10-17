use std::sync::Arc;
use std::time::Instant;

use glam::Vec2;
use mega_ui::wgpu::{DrawStats, UiRenderer};
use mega_ui::{CursorIcon, Rect, Ui, UiInput};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::canvas_gpu::{CanvasOverlay, CanvasRenderer};
use crate::view::CanvasView;

pub const CANVAS_TEXTURE: u32 = 1;
pub const AI_PREVIEW_TEXTURE: u32 = 2;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: UiRenderer,
    canvas: CanvasRenderer,
    canvas_target: wgpu::Texture,
    canvas_target_view: wgpu::TextureView,
}

#[derive(Clone, Default)]
pub struct FrameInput {
    pub mouse_pos: Vec2,
    pub mouse_delta: Vec2,
    pub mouse_down: bool,
    pub mouse_pressed: bool,
    pub mouse_released: bool,
    pub mouse_right_down: bool,
    pub mouse_right_pressed: bool,
    pub mouse_right_released: bool,
    pub mouse_middle_down: bool,
    pub mouse_middle_pressed: bool,
    pub mouse_middle_released: bool,
    pub scroll_delta: Vec2,
    pub text: String,
    pub key_backspace: bool,
    pub key_delete: bool,
    pub key_enter: bool,
    pub key_escape: bool,
    pub key_left: bool,
    pub key_right: bool,
    pub key_up: bool,
    pub key_down: bool,
    pub key_home: bool,
    pub key_end: bool,
    pub key_copy: bool,
    pub key_cut: bool,
    pub key_select_all: bool,
    pub key_paste: bool,
    pub key_undo: bool,
    pub key_redo: bool,
    pub key_ctrl: bool,
    pub key_shift: bool,
    modifiers: ModifiersState,
}

impl FrameInput {
    fn press_key(&mut self, logical: &Key, physical: PhysicalKey) {
        match logical {
            Key::Named(NamedKey::Backspace) => self.key_backspace = true,
            Key::Named(NamedKey::Delete) => self.key_delete = true,
            Key::Named(NamedKey::Enter) => self.key_enter = true,
            Key::Named(NamedKey::Escape) => self.key_escape = true,
            Key::Named(NamedKey::ArrowLeft) => self.key_left = true,
            Key::Named(NamedKey::ArrowRight) => self.key_right = true,
            Key::Named(NamedKey::ArrowUp) => self.key_up = true,
            Key::Named(NamedKey::ArrowDown) => self.key_down = true,
            Key::Named(NamedKey::Home) => self.key_home = true,
            Key::Named(NamedKey::End) => self.key_end = true,
            _ => {}
        }
        if !self.shortcut_mod() { return; }
        // Physical keys keep editing shortcuts working with Cyrillic layouts too.
        let code = match physical {
            PhysicalKey::Code(code) => Some(code),
            _ => None,
        };
        let character = match logical {
            Key::Character(c) => c.to_lowercase(),
            _ => String::new(),
        };
        match (code, character.as_str()) {
            (Some(KeyCode::KeyA), _) | (_, "a") => self.key_select_all = true,
            (Some(KeyCode::KeyC), _) | (_, "c") => self.key_copy = true,
            (Some(KeyCode::KeyX), _) | (_, "x") => self.key_cut = true,
            (Some(KeyCode::KeyV), _) | (_, "v") => self.key_paste = true,
            (Some(KeyCode::KeyZ), _) | (_, "z") if self.key_shift => self.key_redo = true,
            (Some(KeyCode::KeyZ), _) | (_, "z") => self.key_undo = true,
            (Some(KeyCode::KeyY), _) | (_, "y") => self.key_redo = true,
            _ => {}
        }
    }

    fn for_scene(&self, keyboard_captured: bool) -> Self {
        let mut input = self.clone();
        if keyboard_captured {
            input.key_paste = false;
            input.key_undo = false;
            input.key_redo = false;
            input.key_enter = false;
        }
        input
    }

    fn to_ui(&self, viewport: Vec2, dt: f32) -> UiInput {
        UiInput {
            mouse_pos: self.mouse_pos,
            mouse_delta: self.mouse_delta,
            mouse_down: self.mouse_down,
            mouse_pressed: self.mouse_pressed,
            mouse_released: self.mouse_released,
            mouse_right_down: self.mouse_right_down,
            mouse_right_pressed: self.mouse_right_pressed,
            mouse_right_released: self.mouse_right_released,
            mouse_middle_down: self.mouse_middle_down,
            mouse_middle_pressed: self.mouse_middle_pressed,
            mouse_middle_released: self.mouse_middle_released,
            viewport,
            scroll_delta: self.scroll_delta,
            text: self.text.clone(),
            key_backspace: self.key_backspace,
            key_delete: self.key_delete,
            key_enter: self.key_enter,
            key_left: self.key_left,
            key_right: self.key_right,
            key_up: self.key_up,
            key_down: self.key_down,
            key_home: self.key_home,
            key_end: self.key_end,
            key_copy: self.key_copy,
            key_cut: self.key_cut,
            key_select_all: self.key_select_all,
            key_paste: self.key_paste,
            key_ctrl: self.key_ctrl,
            key_shift: self.key_shift,
            dt,
            ..UiInput::default()
        }
    }

    fn shortcut_mod(&self) -> bool {
        self.modifiers.control_key() || self.modifiers.super_key()
    }

    fn clear_edges(&mut self) {
        self.mouse_delta = Vec2::ZERO;
        self.mouse_pressed = false;
        self.mouse_released = false;
        self.mouse_right_pressed = false;
        self.mouse_right_released = false;
        self.mouse_middle_pressed = false;
        self.mouse_middle_released = false;
        self.scroll_delta = Vec2::ZERO;
        self.text.clear();
        self.key_backspace = false;
        self.key_delete = false;
        self.key_enter = false;
        self.key_escape = false;
        self.key_left = false;
        self.key_right = false;
        self.key_up = false;
        self.key_down = false;
        self.key_home = false;
        self.key_end = false;
        self.key_copy = false;
        self.key_cut = false;
        self.key_select_all = false;
        self.key_paste = false;
        self.key_undo = false;
        self.key_redo = false;
    }
}

pub trait Scene {
    fn title() -> &'static str;
    fn init(ui: &mut Ui);
    fn begin_frame(&mut self, input: &FrameInput);
    fn build(&mut self, ui: &mut Ui, viewport: Vec2, stats: DrawStats) -> bool;
    fn sync_canvas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        canvas: &mut CanvasRenderer,
    );
    fn canvas_present(&self) -> Option<(Rect, CanvasView, CanvasOverlay)>;
    fn sync_ui_textures(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, renderer: &mut UiRenderer);
    /// Return false while outstanding work is being cancelled before exit.
    fn request_close(&mut self) -> bool;
}

fn canvas_target(device: &wgpu::Device, width: u32, height: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("canvas composition target"),
        size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

pub struct Host<S: Scene> {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    ui: Ui,
    scene: S,
    input: FrameInput,
    last_mouse: Option<Vec2>,
    last_frame: Instant,
    elapsed: f32,
    cursor: CursorIcon,
    draw_stats: DrawStats,
    closing: bool,
    keyboard_captured: bool,
}

impl<S: Scene> Host<S> {
    pub fn run(scene: S) {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
        let event_loop = EventLoop::new().expect("event loop");
        event_loop.set_control_flow(ControlFlow::Wait);
        let mut ui = Ui::new();
        S::init(&mut ui);
        let mut host = Self {
            window: None,
            gpu: None,
            ui,
            scene,
            input: FrameInput::default(),
            last_mouse: None,
            last_frame: Instant::now(),
            elapsed: 0.0,
            cursor: CursorIcon::Default,
            draw_stats: DrawStats::default(),
            closing: false,
            keyboard_captured: false,
        };
        event_loop.run_app(&mut host).expect("run application");
    }

    fn init_gpu(&mut self, window: Arc<Window>) {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("no suitable GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Mega Image Editor"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::default(),
        }))
        .expect("request GPU device");
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let mut renderer = UiRenderer::new(&device, &queue, format, &self.ui);
        renderer.set_viewport(&queue, config.width as f32, config.height as f32);
        let canvas = CanvasRenderer::new(&device, format);
        let canvas_target = canvas_target(&device, config.width, config.height, format);
        let canvas_target_view = canvas_target.create_view(&Default::default());
        renderer.bind_texture_view(&device, CANVAS_TEXTURE, canvas_target.create_view(&Default::default()));
        self.gpu = Some(Gpu {
            device,
            queue,
            surface,
            config,
            renderer,
            canvas,
            canvas_target,
            canvas_target_view,
        });
        self.window = Some(window);
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.config.width = width;
            gpu.config.height = height;
            gpu.surface.configure(&gpu.device, &gpu.config);
            gpu.canvas_target = canvas_target(&gpu.device, width, height, gpu.config.format);
            gpu.canvas_target_view = gpu.canvas_target.create_view(&Default::default());
            gpu.renderer.bind_texture_view(&gpu.device, CANVAS_TEXTURE, gpu.canvas_target.create_view(&Default::default()));
            gpu.renderer
                .set_viewport(&gpu.queue, width as f32, height as f32);
        }
    }

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.elapsed += dt;
        let viewport = Vec2::new(size.width as f32, size.height as f32);
        self.scene.begin_frame(&self.input.for_scene(self.keyboard_captured));
        let mut ui_input = self.input.to_ui(viewport, dt);
        if ui_input.key_paste {
            ui_input.clipboard = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
        }
        self.ui.begin_frame(ui_input);
        let keep_redrawing = self.scene.build(&mut self.ui, viewport, self.draw_stats);
        let output = self.ui.end_frame();
        self.keyboard_captured = output.want_capture_keyboard;
        if let Some(text) = &output.clipboard {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(text.clone());
            }
        }
        let canvas_present = self.scene.canvas_present();

        let Some(gpu) = self.gpu.as_mut() else { return };
        self.scene
            .sync_canvas(&gpu.device, &gpu.queue, &mut gpu.canvas);
        self.scene.sync_ui_textures(&gpu.device, &gpu.queue, &mut gpu.renderer);
        gpu.renderer
            .sync_atlases(&gpu.device, &gpu.queue, &mut self.ui);

        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                window.request_redraw();
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("editor frame"),
            });
        let draw_canvas = canvas_present.as_ref().is_some_and(|(rect, view, _)| {
            rect.width() >= 1.0 && rect.height() >= 1.0 && view.width > 0
        });
        if draw_canvas {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("canvas offscreen"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.canvas_target_view, resolve_target: None, depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            if let Some((canvas_rect, canvas_view, overlay)) = canvas_present {
                gpu.canvas.render(&gpu.queue, &mut pass, viewport, canvas_rect, canvas_view, overlay, self.elapsed);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("editor frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.025,
                            g: 0.027,
                            b: 0.032,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // Canvas is sampled by its UI pane; menus and modal windows compose above it.
            self.draw_stats = gpu.renderer.draw(&gpu.queue, &mut pass, &output.draw_list);
        }
        gpu.queue.submit(Some(encoder.finish()));
        gpu.queue.present(frame);
        self.input.clear_edges();

        if output.cursor != self.cursor {
            self.cursor = output.cursor;
            window.set_cursor(match output.cursor {
                CursorIcon::Default => winit::window::CursorIcon::Default,
                CursorIcon::Pointer => winit::window::CursorIcon::Pointer,
                CursorIcon::Move => winit::window::CursorIcon::Move,
                CursorIcon::ResizeNwse => winit::window::CursorIcon::NwseResize,
                CursorIcon::ResizeEw => winit::window::CursorIcon::EwResize,
                CursorIcon::ResizeNs => winit::window::CursorIcon::NsResize,
                CursorIcon::Text => winit::window::CursorIcon::Text,
            });
        }
        if keep_redrawing || output.needs_repaint {
            window.request_redraw();
        }
    }
}

impl<S: Scene> ApplicationHandler for Host<S> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(S::title())
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.init_gpu(window.clone());
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                if self.scene.request_close() { event_loop.exit(); }
                else {
                    self.closing = true;
                    if let Some(window) = &self.window { window.request_redraw(); }
                }
            }
            WindowEvent::Resized(size) => {
                self.resize(size.width, size.height);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                if self.closing && self.scene.request_close() { event_loop.exit(); }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let pos = Vec2::new(position.x as f32, position.y as f32);
                if let Some(previous) = self.last_mouse {
                    self.input.mouse_delta += pos - previous;
                }
                self.last_mouse = Some(pos);
                self.input.mouse_pos = pos;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                let (held, pressed, released) = match button {
                    MouseButton::Left => (
                        &mut self.input.mouse_down,
                        &mut self.input.mouse_pressed,
                        &mut self.input.mouse_released,
                    ),
                    MouseButton::Right => (
                        &mut self.input.mouse_right_down,
                        &mut self.input.mouse_right_pressed,
                        &mut self.input.mouse_right_released,
                    ),
                    MouseButton::Middle => (
                        &mut self.input.mouse_middle_down,
                        &mut self.input.mouse_middle_pressed,
                        &mut self.input.mouse_middle_released,
                    ),
                    _ => return,
                };
                if down && !*held {
                    *pressed = true;
                }
                if !down && *held {
                    *released = true;
                }
                *held = down;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.input.scroll_delta = match delta {
                    MouseScrollDelta::LineDelta(x, y) => Vec2::new(x * 40.0, y * 40.0),
                    MouseScrollDelta::PixelDelta(p) => Vec2::new(p.x as f32, p.y as f32),
                };
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.input.modifiers = mods.state();
                self.input.key_ctrl = mods.state().control_key() || mods.state().super_key();
                self.input.key_shift = mods.state().shift_key();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                let shortcut = self.input.shortcut_mod();
                self.input.press_key(&event.logical_key, event.physical_key);
                if !shortcut {
                    if let Some(text) = event.text.as_ref() {
                        for ch in text.chars() {
                            if !ch.is_control() {
                                self.input.text.push(ch);
                            }
                        }
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                self.input.text.push_str(&text);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod text_edit_tests {
    use super::*;

    #[test]
    fn navigation_shortcuts_reach_text_fields_and_edges_reset() {
        let mut input = FrameInput::default();
        for key in [NamedKey::ArrowLeft, NamedKey::ArrowRight, NamedKey::ArrowUp,
            NamedKey::ArrowDown, NamedKey::Home, NamedKey::End] {
            input.press_key(&Key::Named(key), PhysicalKey::Code(KeyCode::Space));
        }
        input.modifiers = ModifiersState::CONTROL | ModifiersState::SHIFT;
        input.key_ctrl = true;
        input.key_shift = true;
        for code in [KeyCode::KeyA, KeyCode::KeyC, KeyCode::KeyX, KeyCode::KeyV] {
            input.press_key(&Key::Character("я".into()), PhysicalKey::Code(code));
        }
        let ui_input = input.to_ui(Vec2::splat(400.0), 0.016);
        assert!(ui_input.key_left && ui_input.key_right && ui_input.key_up && ui_input.key_down);
        assert!(ui_input.key_home && ui_input.key_end && ui_input.key_shift);
        assert!(ui_input.key_select_all && ui_input.key_copy && ui_input.key_cut && ui_input.key_paste);
        input.key_undo = true;
        input.key_redo = true;
        input.key_enter = true;
        let scene = input.for_scene(true);
        assert!(!scene.key_paste && !scene.key_undo && !scene.key_redo && !scene.key_enter);
        assert!(input.for_scene(false).key_paste);
        input.clear_edges();
        let ui_input = input.to_ui(Vec2::splat(400.0), 0.016);
        assert!(!ui_input.key_left && !ui_input.key_right && !ui_input.key_up && !ui_input.key_down);
        assert!(!ui_input.key_home && !ui_input.key_end && !ui_input.key_copy && !ui_input.key_cut && !ui_input.key_select_all);
        assert!(ui_input.key_shift && ui_input.key_ctrl);
    }

    #[test]
    fn keyboard_selection_replaces_text_and_delete_handles_unicode() {
        let mut ui = Ui::new();
        let mut text = "привет\nмир".to_string();
        let out = frame(&mut ui, &mut text, FrameInput::default());
        let mut input = FrameInput::default();
        input.mouse_pos = out.draw_list[0].rect.min + Vec2::splat(8.0);
        input.mouse_pressed = true;
        input.mouse_down = true;
        frame(&mut ui, &mut text, input);
        let mut input = FrameInput::default();
        input.mouse_released = true;
        frame(&mut ui, &mut text, input);
        let mut input = FrameInput::default();
        input.key_end = true;
        input.key_shift = true;
        frame(&mut ui, &mut text, input);
        let mut input = FrameInput::default();
        input.text = "hello".into();
        frame(&mut ui, &mut text, input);
        assert_eq!(text, "hello\nмир");
        let mut input = FrameInput::default();
        input.key_down = true;
        frame(&mut ui, &mut text, input);
        let mut input = FrameInput::default();
        input.key_home = true;
        frame(&mut ui, &mut text, input);
        let mut input = FrameInput::default();
        input.key_delete = true;
        frame(&mut ui, &mut text, input);
        assert_eq!(text, "hello\nир");
        let mut input = FrameInput::default();
        input.key_select_all = true;
        input.key_cut = true;
        let out = frame(&mut ui, &mut text, input);
        assert_eq!(out.clipboard.as_deref(), Some("hello\nир"));
        assert!(text.is_empty());
    }

    fn frame(ui: &mut Ui, text: &mut String, input: FrameInput) -> mega_ui::UiOutput {
        ui.begin_frame(input.to_ui(Vec2::new(400.0, 200.0), 0.016));
        ui.text_area("prompt", text, Vec2::new(300.0, 100.0));
        ui.end_frame()
    }

    #[test]
    fn first_click_places_caret_before_typing() {
        let mut ui = Ui::new();
        let mut text = "abcdef".to_string();
        let out = frame(&mut ui, &mut text, FrameInput::default());
        let position = out.draw_list[0].rect.min + Vec2::splat(8.0);
        let mut input = FrameInput::default();
        input.mouse_pos = position;
        input.mouse_pressed = true;
        input.mouse_down = true;
        frame(&mut ui, &mut text, input);
        let mut input = FrameInput::default();
        input.mouse_pos = position;
        input.mouse_released = true;
        frame(&mut ui, &mut text, input);
        let mut input = FrameInput::default();
        input.text = "X".into();
        frame(&mut ui, &mut text, input);
        assert_eq!(text, "Xabcdef");
    }

    #[test]
    fn drag_selection_starts_at_pressed_position() {
        let mut ui = Ui::new();
        let mut text = "привет мир".to_string();
        let out = frame(&mut ui, &mut text, FrameInput::default());
        let start = out.draw_list[0].rect.min + Vec2::splat(8.0);
        let mut input = FrameInput::default();
        input.mouse_pos = start;
        input.mouse_pressed = true;
        input.mouse_down = true;
        frame(&mut ui, &mut text, input);
        let mut input = FrameInput::default();
        input.mouse_pos = start + Vec2::new(200.0, 0.0);
        input.mouse_down = true;
        frame(&mut ui, &mut text, input);
        let mut input = FrameInput::default();
        input.key_copy = true;
        let out = frame(&mut ui, &mut text, input);
        assert_eq!(out.clipboard.as_deref(), Some("привет мир"));
    }
}
