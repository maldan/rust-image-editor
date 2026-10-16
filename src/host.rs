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

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: UiRenderer,
    canvas: CanvasRenderer,
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
    pub key_paste: bool,
    pub key_undo: bool,
    pub key_redo: bool,
    pub key_ctrl: bool,
    pub key_shift: bool,
    modifiers: ModifiersState,
}

impl FrameInput {
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
        let renderer = UiRenderer::new(&device, &queue, format, &self.ui);
        renderer.set_viewport(&queue, config.width as f32, config.height as f32);
        let canvas = CanvasRenderer::new(&device, format);
        self.gpu = Some(Gpu {
            device,
            queue,
            surface,
            config,
            renderer,
            canvas,
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
        self.scene.begin_frame(&self.input);
        self.ui.begin_frame(self.input.to_ui(viewport, dt));
        let keep_redrawing = self.scene.build(&mut self.ui, viewport, self.draw_stats);
        let output = self.ui.end_frame();
        let canvas_present = self.scene.canvas_present();

        let Some(gpu) = self.gpu.as_mut() else { return };
        self.scene
            .sync_canvas(&gpu.device, &gpu.queue, &mut gpu.canvas);
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
            // UI chrome first, then canvas image (scissored) on top of the canvas dock pane.
            self.draw_stats = gpu.renderer.draw(&gpu.queue, &mut pass, &output.draw_list);
            if let Some((canvas_rect, canvas_view, overlay)) = canvas_present {
                gpu.canvas.render(
                    &gpu.queue,
                    &mut pass,
                    viewport,
                    canvas_rect,
                    canvas_view,
                    overlay,
                    self.elapsed,
                );
            }
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
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.resize(size.width, size.height);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
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
                match &event.logical_key {
                    Key::Named(NamedKey::Backspace) => self.input.key_backspace = true,
                    Key::Named(NamedKey::Delete) => self.input.key_delete = true,
                    Key::Named(NamedKey::Enter) => self.input.key_enter = true,
                    Key::Named(NamedKey::Escape) => self.input.key_escape = true,
                    Key::Character(c) if shortcut => match c.to_lowercase().as_str() {
                        "v" => self.input.key_paste = true,
                        "z" if self.input.key_shift => self.input.key_redo = true,
                        "z" => self.input.key_undo = true,
                        "y" => self.input.key_redo = true,
                        _ => {}
                    },
                    _ => {}
                }
                if shortcut {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        match code {
                            KeyCode::KeyV => self.input.key_paste = true,
                            KeyCode::KeyZ if self.input.key_shift => self.input.key_redo = true,
                            KeyCode::KeyZ => self.input.key_undo = true,
                            KeyCode::KeyY => self.input.key_redo = true,
                            _ => {}
                        }
                    }
                }
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
