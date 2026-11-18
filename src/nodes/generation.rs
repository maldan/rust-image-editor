use crate::comfyui::{self, ControlNetSettings, DrawSettings, HostConfig, Job, UpscaleSettings, VideoSettings};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "workflow", rename_all = "snake_case")]
pub(crate) enum Generation {
    QwenDraw { settings: DrawSettings },
    QwenRedraw { prompt: String },
    MinimaxRedraw { prompt: String, megapixels: f32 },
    SdxlDraw { settings: DrawSettings },
    ControlNet { settings: DrawSettings, control: ControlNetSettings },
    Upscale { settings: DrawSettings, upscale: UpscaleSettings },
    LtxVideo { settings: VideoSettings, scale: f32 },
}

impl Generation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::QwenDraw { .. } => "Qwen 2.1 Draw",
            Self::QwenRedraw { .. } => "Qwen 2.1 Redraw",
            Self::MinimaxRedraw { .. } => "Minimax H3 Redraw",
            Self::SdxlDraw { .. } => "SDXL Draw",
            Self::ControlNet { .. } => "ControlNet",
            Self::Upscale { .. } => "Upscale",
            Self::LtxVideo { .. } => "LTX 2.3 Video",
        }
    }

    pub fn needs_image(&self) -> bool {
        !matches!(self, Self::QwenDraw { .. } | Self::SdxlDraw { .. })
    }

    pub fn is_video(&self) -> bool {
        matches!(self, Self::LtxVideo { .. })
    }

    pub fn uses_catalog(&self) -> bool {
        matches!(self, Self::SdxlDraw { .. } | Self::ControlNet { .. } | Self::Upscale { .. } | Self::LtxVideo { .. })
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::QwenDraw { settings } | Self::SdxlDraw { settings } => settings.validate(),
            Self::QwenRedraw { .. } => Ok(()),
            Self::MinimaxRedraw { megapixels, .. } => comfyui::validate_megapixels(*megapixels),
            Self::ControlNet { settings, control } => control.validate(settings),
            Self::Upscale { settings, upscale } => upscale.validate(settings),
            Self::LtxVideo { settings, scale } => {
                comfyui::scaled_size(64, 64, *scale)?;
                settings.validate()
            }
        }
    }

    pub fn start(
        &self,
        config: HostConfig,
        input: Option<Arc<RgbaImage>>,
        end_image: Option<Arc<RgbaImage>>,
    ) -> Result<Job, String> {
        let image = || input.as_deref().cloned().ok_or("Connect the Image input first");
        Ok(match self {
            Self::QwenDraw { settings } => Job::start_qwen_draw(config, settings.clone()),
            Self::QwenRedraw { prompt } => Job::start(config, image()?, 1.0, prompt.clone()),
            Self::MinimaxRedraw { prompt, megapixels } => {
                Job::start_minimax_redraw(config, image()?, 1.0, prompt.clone(), *megapixels)
            }
            Self::SdxlDraw { settings } => Job::start_draw(config, settings.clone()),
            Self::ControlNet { settings, control } => {
                Job::start_draw_cn(config, image()?, 1.0, settings.clone(), *control)
            }
            Self::Upscale { settings, upscale } => {
                Job::start_upscale(config, image()?, 1.0, settings.clone(), *upscale)
            }
            Self::LtxVideo { settings, scale } => {
                let pixels = image()?;
                let settings = settings.clone().with_source_size(pixels.width(), pixels.height(), *scale)?;
                settings.validate()?;
                Job::start_video(config, pixels, end_image.as_deref().cloned(), settings)
            }
        })
    }
}
