mod app;
mod comfyui;
mod canvas_gpu;
mod editing;
mod history;
mod layer_clipboard;
mod host;
mod project;
mod tiles;
mod ui;
mod view;
mod workspace;

use app::EditorApp;
use host::Host;

fn main() {
    Host::run(EditorApp::default());
}

mod raster;
