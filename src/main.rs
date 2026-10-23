mod app;
mod canvas_gpu;
mod comfyui;
mod editing;
mod history;
mod host;
mod hsv;
mod layer_clipboard;
mod project;
mod safe_save;
mod tiles;
mod ui;
mod video;
mod view;
mod workspace;

use app::EditorApp;
use host::Host;

fn main() {
    Host::run(EditorApp::default());
}

mod raster;
