use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    pub ui_scale: f32,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self { ui_scale: 1.0 }
    }
}

impl EditorSettings {
    fn path() -> Result<PathBuf, String> {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("mega-image-editor").join("editor-settings.json"))
            .ok_or_else(|| "Cannot locate user settings directory (LOCALAPPDATA)".into())
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.ui_scale.is_finite() || !(0.5..=3.0).contains(&self.ui_scale) {
            return Err("UI Scale must be between 0.5 and 3".into());
        }
        Ok(())
    }

    pub fn load() -> Result<Self, String> {
        Self::load_from(&Self::path()?)
    }

    fn load_from(path: &Path) -> Result<Self, String> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("Cannot read editor settings: {error}")),
        };
        let settings: Self = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Invalid editor settings: {error}"))?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path()?)
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        let parent = path.parent().ok_or("Invalid editor settings path")?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let mut file =
            tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.as_file().sync_all())
            .map_err(|error| format!("Cannot write editor settings: {error}"))?;
        file.persist(path)
            .map_err(|error| format!("Cannot replace editor settings: {error}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_settings_default_roundtrip_and_replace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings/editor-settings.json");
        assert_eq!(
            EditorSettings::load_from(&path).unwrap(),
            EditorSettings::default()
        );
        for ui_scale in [1.5, 0.75] {
            let settings = EditorSettings { ui_scale };
            settings.save_to(&path).unwrap();
            assert_eq!(EditorSettings::load_from(&path).unwrap(), settings);
        }
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(EditorSettings::load_from(&path).unwrap().ui_scale, 1.0);
    }

    #[test]
    fn editor_settings_reject_invalid_values_without_overwriting_saved_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("editor-settings.json");
        let saved = EditorSettings { ui_scale: 1.25 };
        saved.save_to(&path).unwrap();
        for ui_scale in [0.0, 0.49, 3.1, f32::NAN, f32::INFINITY] {
            assert!(EditorSettings { ui_scale }.save_to(&path).is_err());
            assert_eq!(EditorSettings::load_from(&path).unwrap(), saved);
        }
        std::fs::write(&path, r#"{"ui_scale":0}"#).unwrap();
        assert!(EditorSettings::load_from(&path).is_err());
        std::fs::write(&path, "broken json").unwrap();
        assert!(EditorSettings::load_from(&path).is_err());
    }
}
