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

pub const LTX_FPS: i32 = 25;
pub const MAX_DURATION_SCALE: i32 = 8;

/// Seconds sent to LTX. The frame count is rounded up to a whole second, then multiplied by `scale`.
pub fn generated_seconds(frame_count: u32, scale: i32) -> Result<i32, String> {
    if frame_count == 0 {
        return Err("Choose a frame range".into());
    }
    if !(1..=MAX_DURATION_SCALE).contains(&scale) {
        return Err(format!(
            "Duration scale must be between 1 and {MAX_DURATION_SCALE}"
        ));
    }
    let seconds = ((frame_count as i32 + LTX_FPS - 1) / LTX_FPS) * scale;
    if !(1..=120).contains(&seconds) {
        return Err("Generated duration must be between 1 and 120 seconds".into());
    }
    Ok(seconds)
}

/// Map a timeline slot onto a decoded frame. The first and last frames stay put;
/// extra frames are skipped and a short clip repeats frames to fill the range.
pub fn frame_sample(slot: u32, slots: u32, sources: usize) -> usize {
    if sources <= 1 || slots <= 1 || slot == 0 {
        return 0;
    }
    let slot = slot.min(slots - 1) as u64;
    let last = sources as u64 - 1;
    (slot * last / (slots as u64 - 1)) as usize
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
    fn animation_duration_rounds_up_and_scale_stretches_it() {
        assert_eq!(generated_seconds(25, 1).unwrap(), 1);
        assert_eq!(generated_seconds(25, 3).unwrap(), 3);
        assert_eq!(generated_seconds(1, 1).unwrap(), 1);
        assert_eq!(generated_seconds(26, 1).unwrap(), 2);
        assert_eq!(generated_seconds(50, 2).unwrap(), 4);
        assert!(generated_seconds(0, 1).is_err());
        assert!(generated_seconds(25, 0).is_err());
        assert!(generated_seconds(25, MAX_DURATION_SCALE + 1).is_err());
        assert_eq!(generated_seconds(25 * 120, 1).unwrap(), 120);
        assert!(generated_seconds(25 * 120 + 1, 1).is_err());
    }

    #[test]
    fn frame_sampling_drops_extras_and_repeats_short_clips() {
        assert_eq!(
            (0..25u32)
                .map(|slot| frame_sample(slot, 25, 25))
                .collect::<Vec<_>>(),
            (0..25usize).collect::<Vec<_>>()
        );
        assert_eq!(
            (0..25u32)
                .map(|slot| frame_sample(slot, 25, 76))
                .collect::<Vec<_>>(),
            (0..25u32).map(|slot| (slot * 75 / 24) as usize).collect::<Vec<_>>()
        );
        assert_eq!(frame_sample(0, 2, 4), 0);
        assert_eq!(frame_sample(1, 2, 4), 3);
        assert_eq!(
            (0..4)
                .map(|slot| frame_sample(slot, 4, 2))
                .collect::<Vec<_>>(),
            vec![0, 0, 0, 1]
        );
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
