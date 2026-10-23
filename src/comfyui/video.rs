use super::workflow::{LoraSelection, node_id};
use serde_json::Value;

#[derive(Clone)]
pub struct VideoSettings {
    pub width: i32,
    pub height: i32,
    pub fps: i32,
    pub seconds: i32,
    pub prompt: String,
    pub loras: [LoraSelection; 5],
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            width: 512,
            height: 512,
            fps: 25,
            seconds: 5,
            prompt: String::new(),
            loras: std::array::from_fn(|_| LoraSelection {
                name: None,
                strength: 1.0,
            }),
        }
    }
}

impl VideoSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.width < 64 || self.height < 64 || self.width % 64 != 0 || self.height % 64 != 0 {
            return Err("Video size must be at least 64 px and a multiple of 64".into());
        }
        super::scaled_size(self.width as u32, self.height as u32, 1.0)?;
        if !(1..=120).contains(&self.fps) || !(1..=120).contains(&self.seconds) {
            return Err("FPS and duration must be between 1 and 120".into());
        }
        if self.prompt.trim().is_empty() {
            return Err("Enter a video prompt".into());
        }
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

pub fn prepare(
    filename: &str,
    end_filename: Option<&str>,
    settings: &VideoSettings,
    seed: u64,
) -> Result<(Value, String), String> {
    settings.validate()?;
    let template = if end_filename.is_some() {
        include_str!("../../comfyui/video.ltx_between.json")
    } else {
        include_str!("../../comfyui/video.ltx_i2v.json")
    };
    let mut graph: Value =
        serde_json::from_str(template).map_err(|e| format!("Invalid LTX workflow: {e}"))?;
    if let Some(end_filename) = end_filename {
        let id = node_id(&graph, "Image2")?;
        graph[&id]["inputs"]["image"] = end_filename.into();
    } else {
        let id = node_id(&graph, "RandomNoise1")?;
        graph[&id]["inputs"]["noise_seed"] = seed.into();
    }
    for (title, key, value) in [
        ("Image1", "image", Value::from(filename)),
        ("VideoWidth", "value", settings.width.into()),
        ("VideoHeight", "value", settings.height.into()),
        ("FPS", "value", settings.fps.into()),
        ("VideoLength", "value", settings.seconds.into()),
        (
            "Positive",
            if end_filename.is_some() {
                "text"
            } else {
                "value"
            },
            settings.prompt.clone().into(),
        ),
        (
            "RandomNoise2",
            "noise_seed",
            ((seed + 1) & ((1_u64 << 53) - 1)).into(),
        ),
        ("SaveVideo", "format", "video/h264-mp4".into()),
    ] {
        let id = node_id(&graph, title)?;
        graph[&id]["inputs"][key] = value;
    }
    for (i, lora) in settings.loras.iter().enumerate() {
        let id = node_id(&graph, &format!("Lora{}", i + 1))?;
        if let Some(name) = &lora.name {
            graph[&id]["inputs"]["lora_name"] = name.clone().into();
            graph[&id]["inputs"]["strength_model"] = lora.strength.into();
        } else {
            let model = graph[&id]["inputs"]["model"].clone();
            graph.as_object_mut().unwrap().remove(&id);
            for node in graph.as_object_mut().unwrap().values_mut() {
                for input in node["inputs"]
                    .as_object_mut()
                    .ok_or("Missing node inputs")?
                    .values_mut()
                {
                    if input.is_array() && input[0] == id {
                        *input = model.clone();
                    }
                }
            }
        }
    }
    let output = node_id(&graph, "SaveVideo")?;
    Ok((graph, output))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn video_graph_patches_controls_and_bypasses_unused_loras() {
        let mut settings = VideoSettings {
            width: 768,
            height: 512,
            fps: 25,
            seconds: 4,
            prompt: "Camera pans across a forest".into(),
            ..Default::default()
        };
        settings.loras[0].name = Some("motion.safetensors".into());
        settings.loras[0].strength = 0.6;
        settings.loras[4].name = Some("last.safetensors".into());
        let (graph, save) = prepare("input.png", None, &settings, 42).unwrap();
        assert_eq!(save, "270");
        for (title, key, expected) in [
            ("Image1", "image", Value::from("input.png")),
            ("VideoWidth", "value", 768.into()),
            ("VideoHeight", "value", 512.into()),
            ("FPS", "value", 25.into()),
            ("VideoLength", "value", 4.into()),
            ("Positive", "value", settings.prompt.clone().into()),
            ("RandomNoise1", "noise_seed", 42.into()),
            ("RandomNoise2", "noise_seed", 43.into()),
        ] {
            assert_eq!(
                graph[node_id(&graph, title).unwrap()]["inputs"][key],
                expected
            );
        }
        assert_eq!(graph["271"]["inputs"]["lora_name"], "motion.safetensors");
        assert_eq!(
            graph["320:327"]["inputs"]["model"],
            serde_json::json!(["271", 0])
        );
        assert!(
            node_id(&graph, "Load LoRA").is_ok(),
            "retain built-in distilled LoRA"
        );
        assert!(node_id(&graph, "Lora2").is_err());
        assert_eq!(graph[&save]["inputs"]["format"], "video/h264-mp4");
        for node in graph.as_object().unwrap().values() {
            for input in node["inputs"].as_object().unwrap().values() {
                if let Some(link) = input.as_array() {
                    assert!(
                        graph.get(link[0].as_str().unwrap()).is_some(),
                        "dangling workflow link"
                    );
                }
            }
        }
        settings.loras.iter_mut().for_each(|l| l.name = None);
        let (graph, _) = prepare("input.png", None, &settings, 0).unwrap();
        assert_eq!(
            graph["320:314"]["inputs"]["model"],
            serde_json::json!(["320:285", 0])
        );
    }

    #[test]
    fn video_between_graph_patches_both_frames_controls_and_loras() {
        let mut settings = VideoSettings {
            width: 768,
            height: 512,
            fps: 30,
            seconds: 4,
            prompt: "Camera moves between two scenes".into(),
            ..Default::default()
        };
        for enabled in [false, true] {
            settings.loras[0].name = enabled.then(|| "motion.safetensors".into());
            settings.loras[0].strength = 0.6;
            let (graph, save) = prepare("first.png", Some("last.png"), &settings, 42).unwrap();
            assert_eq!(save, "69");
            for (title, key, expected) in [
                ("Image1", "image", Value::from("first.png")),
                ("Image2", "image", Value::from("last.png")),
                ("VideoWidth", "value", 768.into()),
                ("VideoHeight", "value", 512.into()),
                ("FPS", "value", 30.into()),
                ("VideoLength", "value", 4.into()),
                ("Positive", "text", settings.prompt.clone().into()),
                ("RandomNoise2", "noise_seed", 43.into()),
                ("SaveVideo", "format", "video/h264-mp4".into()),
            ] {
                assert_eq!(
                    graph[node_id(&graph, title).unwrap()]["inputs"][key],
                    expected
                );
            }
            assert_eq!(node_id(&graph, "Lora1").is_ok(), enabled);
            assert!(node_id(&graph, "Lora2").is_err());
            if enabled {
                assert_eq!(graph["71"]["inputs"]["lora_name"], "motion.safetensors");
                assert!(
                    (graph["71"]["inputs"]["strength_model"].as_f64().unwrap() - 0.6).abs() < 1e-6
                );
            }
            assert_eq!(graph["129:115"]["inputs"]["frame_idx"], 0);
            assert_eq!(graph["129:111"]["inputs"]["frame_idx"], -1);
            for node in graph.as_object().unwrap().values() {
                for input in node["inputs"].as_object().unwrap().values() {
                    if let Some(link) = input.as_array() {
                        assert!(
                            graph.get(link[0].as_str().unwrap()).is_some(),
                            "dangling workflow link"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn video_validation_rejects_invalid_settings() {
        let valid = VideoSettings {
            prompt: "Animate clouds".into(),
            ..Default::default()
        };
        assert_eq!(valid.fps, 25);
        assert!(valid.validate().is_ok());
        for settings in [
            VideoSettings {
                width: 65,
                ..valid.clone()
            },
            VideoSettings {
                height: 0,
                ..valid.clone()
            },
            VideoSettings {
                fps: 0,
                ..valid.clone()
            },
            VideoSettings {
                seconds: -1,
                ..valid.clone()
            },
            VideoSettings {
                prompt: String::new(),
                ..valid.clone()
            },
        ] {
            assert!(settings.validate().is_err());
        }
    }
}
