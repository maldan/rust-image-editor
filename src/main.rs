mod app;
mod host;
mod project;

use app::EditorApp;
use host::Host;

fn main() {
    Host::run(EditorApp::default());
}
