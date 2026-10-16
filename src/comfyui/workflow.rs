use serde_json::Value;

pub const EMBEDDED: &str = include_str!("../../comfyui/image.qwen_edit_21.json");
pub const INPAINT: &str = include_str!("../../comfyui/image.sd_inpaint.json");
pub const REDRAW: &str = include_str!("../../comfyui/image.sd_redraw.json");
pub const DRAW: &str = include_str!("../../comfyui/image.sd_draw.json");

#[derive(Clone)]
pub struct LoraSelection {
    pub name: Option<String>,
    pub strength: f32,
}

#[derive(Clone)]
pub struct DrawSettings {
    pub model: Option<String>,
    pub loras: [LoraSelection; 3],
    pub positive: String,
    pub negative: String,
    pub width: i32,
    pub height: i32,
}

impl Default for DrawSettings {
    fn default() -> Self {
        Self {
            model: None,
            loras: std::array::from_fn(|_| LoraSelection {
                name: None,
                strength: 1.0,
            }),
            positive: String::new(),
            negative: String::new(),
            width: 512,
            height: 512,
        }
    }
}

impl DrawSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.width < 64 || self.height < 64 || self.width % 8 != 0 || self.height % 8 != 0 {
            return Err("Size must be at least 64 px and a multiple of 8".into());
        }
        super::scaled_size(self.width as u32, self.height as u32, 1.0)?;
        self.validate_loras()
    }

    pub fn validate_redraw(&self, denoise: f32) -> Result<(), String> {
        if !denoise.is_finite() || !(0.0..=1.0).contains(&denoise) {
            return Err("Denoise must be between 0 and 1".into());
        }
        self.validate_loras()
    }

    fn validate_loras(&self) -> Result<(), String> {
        if self
            .loras
            .iter()
            .any(|l| !l.strength.is_finite() || !(0.0..=1.0).contains(&l.strength))
        {
            return Err("LoRA strength must be between 0 and 1".into());
        }
        Ok(())
    }
}

pub fn prepare_draw(settings: &DrawSettings, seed: u64) -> Result<(Value, String), String> {
    settings.validate()?;
    let mut graph: Value =
        serde_json::from_str(DRAW).map_err(|e| format!("Invalid draw workflow: {e}"))?;
    patch(&mut graph, "Image", "width", settings.width.into())?;
    patch(&mut graph, "Image", "height", settings.height.into())?;
    prepare_sd(graph, settings, seed)
}

pub fn prepare_redraw(
    filename: &str,
    settings: &DrawSettings,
    denoise: f32,
    seed: u64,
) -> Result<(Value, String), String> {
    settings.validate_redraw(denoise)?;
    let mut graph: Value =
        serde_json::from_str(REDRAW).map_err(|e| format!("Invalid redraw workflow: {e}"))?;
    patch(&mut graph, "Image", "image", filename.into())?;
    patch(&mut graph, "Sampler", "denoise", denoise.into())?;
    prepare_sd(graph, settings, seed)
}

pub fn prepare_inpaint(
    filename: &str,
    mask_filename: &str,
    settings: &DrawSettings,
    denoise: f32,
    seed: u64,
) -> Result<(Value, String), String> {
    settings.validate_redraw(denoise)?;
    let mut graph: Value =
        serde_json::from_str(INPAINT).map_err(|e| format!("Invalid inpaint workflow: {e}"))?;
    patch(&mut graph, "Image", "image", filename.into())?;
    patch(&mut graph, "Mask", "image", mask_filename.into())?;
    patch(&mut graph, "Sampler", "denoise", denoise.into())?;
    prepare_sd(graph, settings, seed)
}

fn prepare_sd(
    mut graph: Value,
    settings: &DrawSettings,
    seed: u64,
) -> Result<(Value, String), String> {
    if let Some(model) = &settings.model {
        patch(&mut graph, "Model", "ckpt_name", model.clone().into())?;
    }
    patch(
        &mut graph,
        "Positive",
        "text",
        settings.positive.clone().into(),
    )?;
    patch(
        &mut graph,
        "Negative",
        "text",
        settings.negative.clone().into(),
    )?;
    patch(&mut graph, "Sampler", "seed", seed.into())?;
    for (i, lora) in settings.loras.iter().enumerate() {
        let title = format!("Lora{}", i + 1);
        if let Some(name) = &lora.name {
            patch(&mut graph, &title, "lora_name", name.clone().into())?;
            for key in ["strength_model", "strength_clip"] {
                patch(&mut graph, &title, key, lora.strength.into())?;
            }
        } else {
            // Zero strength still validates the filename on the server. Bypass absent LoRAs.
            let id = node_id(&graph, &title)?;
            let model = graph[&id]["inputs"]["model"].clone();
            let clip = graph[&id]["inputs"]["clip"].clone();
            graph.as_object_mut().unwrap().remove(&id);
            for node in graph.as_object_mut().unwrap().values_mut() {
                if let Some(inputs) = node["inputs"].as_object_mut() {
                    for value in inputs.values_mut() {
                        if value.is_array() && value[0] == id {
                            *value = if value[1] == 0 {
                                model.clone()
                            } else {
                                clip.clone()
                            };
                        }
                    }
                }
            }
        }
    }
    let output = node_id(&graph, "ImageSaveWithFormat (maga)")?;
    Ok((graph, output))
}

pub fn node_id(graph: &Value, title: &str) -> Result<String, String> {
    let graph = graph
        .as_object()
        .ok_or("Workflow must be an API prompt object")?;
    let ids: Vec<_> = graph
        .iter()
        .filter(|(_, node)| node["_meta"]["title"] == title)
        .map(|(id, _)| id.clone())
        .collect();
    if ids.len() != 1 {
        return Err(format!("Workflow requires exactly one node titled {title}"));
    }
    Ok(ids[0].clone())
}

pub fn prepare(filename: &str, prompt: &str, seed: u64) -> Result<(Value, String), String> {
    let mut graph: Value =
        serde_json::from_str(EMBEDDED).map_err(|e| format!("Invalid embedded workflow: {e}"))?;
    patch(&mut graph, "LoadImage", "image", filename.into())?;
    patch(&mut graph, "Prompt", "prompt", prompt.into())?;
    patch(&mut graph, "Sampler", "seed", seed.into())?;
    let output = node_id(&graph, "SaveImage")?;
    Ok((graph, output))
}

fn patch(graph: &mut Value, title: &str, key: &str, value: Value) -> Result<(), String> {
    let id = node_id(graph, title)?;
    let inputs = graph[&id]["inputs"]
        .as_object_mut()
        .ok_or_else(|| format!("{title} has no inputs"))?;
    if !inputs.contains_key(key) {
        return Err(format!("{title} has no {key} input"));
    }
    inputs.insert(key.into(), value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inpaint_patches_both_inputs_and_preserves_mask_latent_connections() {
        let settings = DrawSettings {
            model: Some("chosen".into()),
            positive: "forest".into(),
            negative: "text".into(),
            ..Default::default()
        };
        let (graph, save) = prepare_inpaint("source.png", "mask.png", &settings, 0.5, 42).unwrap();
        let original: Value = serde_json::from_str(INPAINT).unwrap();
        assert_eq!(graph[&save], original[&save]);
        let input = node_id(&graph, "Image").unwrap();
        let mask = node_id(&graph, "Mask").unwrap();
        assert_eq!(graph[&input]["inputs"]["image"], "source.png");
        assert_eq!(graph[&mask]["inputs"]["image"], "mask.png");
        assert_eq!(graph[&mask]["inputs"]["channel"], "red");
        let sampler = node_id(&graph, "Sampler").unwrap();
        assert_eq!(graph[&sampler]["inputs"]["denoise"], 0.5);
        assert_eq!(graph[&sampler]["inputs"]["seed"], 42);
        assert_eq!(
            graph[&sampler]["inputs"]["latent_image"],
            original[&sampler]["inputs"]["latent_image"]
        );
        for title in ["VAE Encode", "Set Latent Noise Mask"] {
            let id = node_id(&graph, title).unwrap();
            assert_eq!(graph[&id], original[&id]);
        }
        assert_eq!(
            graph[node_id(&graph, "Model").unwrap()]["inputs"]["ckpt_name"],
            "chosen"
        );
        assert_eq!(
            graph[node_id(&graph, "Positive").unwrap()]["inputs"]["text"],
            "forest"
        );
        assert_eq!(
            graph[node_id(&graph, "Negative").unwrap()]["inputs"]["text"],
            "text"
        );
        assert_eq!(
            graph
                .as_object()
                .unwrap()
                .values()
                .filter(|n| n["class_type"] == "LoraLoader")
                .count(),
            0
        );
        assert!(prepare_inpaint("i", "m", &settings, f32::NAN, 0).is_err());
    }

    #[test]
    fn redraw_uses_input_denoise_and_shared_sd_settings() {
        let original: Value = serde_json::from_str(REDRAW).unwrap();
        for denoise in [0.0, 0.5, 1.0] {
            for mask in 0..8 {
                let mut settings = DrawSettings {
                    model: Some("chosen.safetensors".into()),
                    positive: "forest".into(),
                    negative: "watermark".into(),
                    // Draw dimensions must not affect the image input workflow.
                    width: -1,
                    height: 0,
                    ..Default::default()
                };
                for (i, lora) in settings.loras.iter_mut().enumerate() {
                    if mask & (1 << i) != 0 {
                        lora.name = Some(format!("lora{i}"));
                        lora.strength = 0.5;
                    }
                }
                let (graph, save) =
                    prepare_redraw("folder/input.png", &settings, denoise, 42).unwrap();
                assert_eq!(graph[&save], original[&save]);
                assert_eq!(
                    graph[node_id(&graph, "Image").unwrap()]["inputs"]["image"],
                    "folder/input.png"
                );
                let sampler = node_id(&graph, "Sampler").unwrap();
                assert_eq!(graph[&sampler]["inputs"]["denoise"], denoise);
                assert_eq!(graph[&sampler]["inputs"]["seed"], 42);
                assert_eq!(
                    graph[&sampler]["inputs"]["latent_image"],
                    original[&sampler]["inputs"]["latent_image"]
                );
                assert_eq!(
                    graph[node_id(&graph, "VAE Encode").unwrap()],
                    original[node_id(&original, "VAE Encode").unwrap()]
                );
                assert_eq!(
                    graph[node_id(&graph, "Positive").unwrap()]["inputs"]["text"],
                    "forest"
                );
                assert_eq!(
                    graph[node_id(&graph, "Negative").unwrap()]["inputs"]["text"],
                    "watermark"
                );
                let mut upstream = node_id(&graph, "Model").unwrap();
                assert_eq!(
                    graph[&upstream]["inputs"]["ckpt_name"],
                    "chosen.safetensors"
                );
                for (i, lora) in settings.loras.iter().enumerate() {
                    let id = node_id(&graph, &format!("Lora{}", i + 1));
                    if let Some(name) = &lora.name {
                        let id = id.unwrap();
                        assert_eq!(graph[&id]["inputs"]["lora_name"], *name);
                        assert_eq!(graph[&id]["inputs"]["strength_model"], 0.5);
                        assert_eq!(graph[&id]["inputs"]["strength_clip"], 0.5);
                        assert_eq!(
                            graph[&id]["inputs"]["model"],
                            serde_json::json!([upstream, 0])
                        );
                        assert_eq!(
                            graph[&id]["inputs"]["clip"],
                            serde_json::json!([upstream, 1])
                        );
                        upstream = id;
                    } else {
                        assert!(id.is_err());
                    }
                }
                assert_eq!(
                    graph[&sampler]["inputs"]["model"],
                    serde_json::json!([upstream, 0])
                );
                for title in ["Positive", "Negative"] {
                    assert_eq!(
                        graph[node_id(&graph, title).unwrap()]["inputs"]["clip"],
                        serde_json::json!([upstream, 1])
                    );
                }
            }
        }
        for denoise in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            assert!(prepare_redraw("input.png", &DrawSettings::default(), denoise, 0).is_err());
        }
    }

    #[test]
    fn draw_patches_settings_and_bypasses_every_combination_of_unused_loras() {
        let original: Value = serde_json::from_str(DRAW).unwrap();
        for mask in 0..8 {
            let mut settings = DrawSettings {
                positive: "a forest".into(),
                negative: "watermark".into(),
                width: 768,
                height: 1024,
                ..Default::default()
            };
            for (i, lora) in settings.loras.iter_mut().enumerate() {
                if mask & (1 << i) != 0 {
                    lora.name = Some(format!("lora{i}.safetensors"));
                    lora.strength = [0.0, 0.5, 1.0][i];
                }
            }
            let (graph, output) = prepare_draw(&settings, 42).unwrap();
            assert_eq!(graph[&output], original[&output]);
            let model = node_id(&graph, "Model").unwrap();
            assert_eq!(graph[&model], original[&model]);
            assert_eq!(
                graph[node_id(&graph, "Positive").unwrap()]["inputs"]["text"],
                "a forest"
            );
            assert_eq!(
                graph[node_id(&graph, "Negative").unwrap()]["inputs"]["text"],
                "watermark"
            );
            let image = node_id(&graph, "Image").unwrap();
            assert_eq!(graph[&image]["inputs"]["width"], 768);
            assert_eq!(graph[&image]["inputs"]["height"], 1024);
            let sampler = node_id(&graph, "Sampler").unwrap();
            assert_eq!(graph[&sampler]["inputs"]["seed"], 42);
            assert_eq!(
                graph[&sampler]["inputs"]["steps"],
                original[&sampler]["inputs"]["steps"]
            );
            let mut upstream = model;
            for (i, lora) in settings.loras.iter().enumerate() {
                let id = node_id(&graph, &format!("Lora{}", i + 1));
                if let Some(name) = &lora.name {
                    let id = id.unwrap();
                    let inputs = &graph[&id]["inputs"];
                    assert_eq!(inputs["lora_name"], *name);
                    assert_eq!(inputs["strength_model"], lora.strength);
                    assert_eq!(inputs["strength_clip"], lora.strength);
                    assert_eq!(inputs["model"], serde_json::json!([upstream, 0]));
                    assert_eq!(inputs["clip"], serde_json::json!([upstream, 1]));
                    upstream = id;
                } else {
                    assert!(id.is_err());
                }
            }
            assert_eq!(
                graph[&sampler]["inputs"]["model"],
                serde_json::json!([upstream, 0])
            );
            assert_eq!(
                graph[node_id(&graph, "Positive").unwrap()]["inputs"]["clip"],
                serde_json::json!([upstream, 1])
            );
        }
        let settings = DrawSettings {
            model: Some("chosen.safetensors".into()),
            ..Default::default()
        };
        let (graph, _) = prepare_draw(&settings, 1).unwrap();
        assert_eq!(
            graph[node_id(&graph, "Model").unwrap()]["inputs"]["ckpt_name"],
            "chosen.safetensors"
        );
    }

    #[test]
    fn draw_rejects_invalid_sizes_and_strengths() {
        for (width, height) in [
            (0, 512),
            (-8, 512),
            (65, 512),
            (512, 63),
            (16392, 512),
            (16384, 16384),
        ] {
            assert!(
                prepare_draw(
                    &DrawSettings {
                        width,
                        height,
                        ..Default::default()
                    },
                    0
                )
                .is_err()
            );
        }
        for strength in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            let mut settings = DrawSettings::default();
            settings.loras[0].strength = strength;
            assert!(prepare_draw(&settings, 0).is_err());
        }
    }

    #[test]
    fn patches_only_requested_inputs() {
        let original: Value = serde_json::from_str(EMBEDDED).unwrap();
        let (mut graph, save) = prepare("folder/input.png", "Keep colors", 123456789).unwrap();
        assert_eq!(graph[&save]["inputs"]["image_format"], "webp");
        let load = node_id(&graph, "LoadImage").unwrap();
        let prompt = node_id(&graph, "Prompt").unwrap();
        let sampler = node_id(&graph, "Sampler").unwrap();
        assert_eq!(graph[&sampler]["inputs"]["seed"].as_u64(), Some(123456789));
        graph[&load]["inputs"]["image"] = original[&load]["inputs"]["image"].clone();
        graph[&prompt]["inputs"]["prompt"] = original[&prompt]["inputs"]["prompt"].clone();
        graph[&sampler]["inputs"]["seed"] = original[&sampler]["inputs"]["seed"].clone();
        assert_eq!(graph, original);
    }
}
