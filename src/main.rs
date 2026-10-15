mod app;
mod host;
mod project;
mod ui;

use app::EditorApp;
use host::Host;

fn main() {
    Host::run(EditorApp::default());
}
