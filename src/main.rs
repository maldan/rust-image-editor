//! No console window when opened via double-click / Explorer.
#![windows_subsystem = "windows"]

mod app;
mod brightness_contrast;
mod camera;
mod canvas_gpu;
mod comfyui;
mod editing;
mod editor_settings;
mod history;
mod host;
mod hsv;
mod layer_clipboard;
mod project;
mod nodes;
mod prompt_preset;
mod safe_save;
mod tiles;
mod ui;
mod video;
mod video_document;
mod video_effects;
mod view;
mod win_assoc;
mod workspace;

use std::path::PathBuf;

use app::EditorApp;
use host::Host;

fn main() {
    #[cfg(windows)]
    win_assoc::set_app_user_model_id();

    let mut args = std::env::args().skip(1);
    let mut open_path: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--register" => match win_assoc::register() {
                Ok(()) => {
                    win_assoc::report(
                        "Mega Image Editor",
                        "Registered .mgimg association and icons.",
                    );
                    return;
                }
                Err(e) => {
                    win_assoc::report("Mega Image Editor", &format!("Register failed: {e}"));
                    std::process::exit(1);
                }
            },
            "--unregister" => match win_assoc::unregister() {
                Ok(()) => {
                    win_assoc::report("Mega Image Editor", "Unregistered .mgimg association.");
                    return;
                }
                Err(e) => {
                    win_assoc::report("Mega Image Editor", &format!("Unregister failed: {e}"));
                    std::process::exit(1);
                }
            },
            "-h" | "--help" => {
                win_assoc::report(
                    "Mega Image Editor",
                    "Usage: image-editor [path.mgimg] | --register | --unregister",
                );
                return;
            }
            other if other.starts_with('-') => {
                win_assoc::report("Mega Image Editor", &format!("Unknown flag: {other}"));
                std::process::exit(2);
            }
            other => {
                open_path = Some(PathBuf::from(other));
            }
        }
    }

    Host::run(EditorApp::new(open_path));
}

mod raster;
