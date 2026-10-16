mod app;
mod canvas_gpu;
mod editing;
mod history;
mod host;
mod project;
mod tiles;
mod ui;
mod view;

use app::EditorApp;
use host::Host;

fn main() {
    Host::run(EditorApp::default());
}
