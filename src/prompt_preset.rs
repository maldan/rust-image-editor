use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptPreset {
    pub id: String,
    pub name: String,
    pub positive: String,
    #[serde(default)]
    pub negative: String,
}

impl PromptPreset {
    pub fn new(name: String, positive: String, negative: String) -> Self {
        Self {
            id: format!("prompt_{:032x}", rand::random::<u128>()),
            name,
            positive,
            negative,
        }
    }
}
