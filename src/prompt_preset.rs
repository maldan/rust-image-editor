use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptKind {
    #[default]
    Image,
    Video,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_presets_default_to_images_and_typed_presets_roundtrip() {
        let legacy = r#"{"id":"old","name":"Style","positive":"Forest","negative":"Blur"}"#;
        let preset: PromptPreset = serde_json::from_str(legacy).unwrap();
        assert_eq!(preset.kind, PromptKind::Image);
        assert_eq!(preset.id, "old");
        assert_eq!(preset.positive, "Forest");
        assert_eq!(preset.negative, "Blur");
        for kind in [PromptKind::Image, PromptKind::Video] {
            let typed = preset.clone().with_kind(kind);
            let value = serde_json::to_value(&typed).unwrap();
            assert_eq!(
                value["kind"],
                if kind == PromptKind::Image {
                    "image"
                } else {
                    "video"
                }
            );
            assert_eq!(
                serde_json::from_value::<PromptPreset>(value).unwrap(),
                typed
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptPreset {
    #[serde(default)]
    pub kind: PromptKind,
    pub id: String,
    pub name: String,
    pub positive: String,
    #[serde(default)]
    pub negative: String,
}

impl PromptPreset {
    pub fn new(name: String, positive: String, negative: String) -> Self {
        Self {
            kind: PromptKind::Image,
            id: format!("prompt_{:032x}", rand::random::<u128>()),
            name,
            positive,
            negative,
        }
    }

    pub fn with_kind(mut self, kind: PromptKind) -> Self {
        self.kind = kind;
        self
    }
}
