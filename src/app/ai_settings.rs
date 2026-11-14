use super::*;
use crate::{comfyui::HostConfig, ui::ai_settings::AiSettings};

impl EditorApp {
    pub fn open_ai_settings(&mut self) {
        if self.ai.open || self.hsv.open || self.rename.is_some() || self.ai_settings.is_some() || self.unsaved.is_some() {
            return;
        }
        self.commit_transform();
        self.workspace.clear_asset_drag();
        self.layer_drag = None;
        let mut dialog = AiSettings::default();
        match HostConfig::load() {
            Ok(config) => {
                dialog.host = config.host.clone();
                dialog.login = config.login.clone();
                dialog.password = config.password.clone();
                dialog.original = Some(config);
            }
            Err(_) => {}
        }
        self.ai_settings = Some(dialog);
    }

    pub fn save_ai_settings(&mut self) {
        let Some(dialog) = &mut self.ai_settings else {
            return;
        };
        let result = HostConfig::from_fields(
            dialog.host.clone(),
            dialog.login.clone(),
            dialog.password.clone(),
        )
        .and_then(|mut config| {
            if let Some(original) = &dialog.original {
                if original.host == config.host {
                    config.ws = original.ws.clone();
                }
            }
            config.save()?;
            Ok(config)
        });
        match result {
            Ok(config) => {
                self.ai.config = Some(config);
                self.ai.catalog_load = None;
                self.ai.catalog = Default::default();
                self.ai.catalog_message.clear();
                self.ai_settings = None;
                self.status = "ComfyUI settings saved".into();
            }
            Err(error) => dialog.message = error,
        }
    }
}
