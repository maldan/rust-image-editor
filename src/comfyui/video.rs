use super::workflow::{LoraSelection, node_id, node_ids};
use serde_json::Value;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
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
    pub fn with_source_size(mut self, width: u32, height: u32, scale: f32) -> Result<Self, String> {
        let (width, height) = super::scaled_size(width, height, scale)?;
        let width = ((width + 32) / 64 * 64).max(64);
        let height = ((height + 32) / 64 * 64).max(64);
        super::scaled_size(width, height, 1.0)?;
        self.width = width as i32;
        self.height = height as i32;
        Ok(self)
    }

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

pub fn validate_minimax_i2v(megapixels: f32, seconds: i32, prompt: &str) -> Result<(), String> {
    if !megapixels.is_finite() || !(0.2..=1.0).contains(&megapixels) {
        return Err("Megapixels must be between 0.2 and 1".into());
    }
    if !(1..=120).contains(&seconds) {
        return Err("Duration must be between 1 and 120 seconds".into());
    }
    if prompt.trim().is_empty() {
        return Err("Enter a video prompt".into());
    }
    Ok(())
}

pub fn prepare_minimax_i2v(
    filename: &str,
    last_filename: Option<&str>,
    prompt: &str,
    megapixels: f32,
    seconds: i32,
    turbo: bool,
    seed: u64,
) -> Result<(Value, String), String> {
    validate_minimax_i2v(megapixels, seconds, prompt)?;
    let mut graph: Value = serde_json::from_str(include_str!("../../comfyui/video.minimax_h3_i2v.json"))
        .map_err(|e| format!("Invalid Minimax I2V workflow: {e}"))?;
    for (title, key, value) in [
        ("LoadImage", "image", Value::from(filename)),
        ("Megapixels", "megapixels", megapixels.into()),
        ("Prompt", "prompt", prompt.into()),
        ("Float (duration)", "value", seconds.into()),
        ("RandomNoise", "noise_seed", seed.into()),
        ("Boolean (Enable Lightning LoRA)", "value", turbo.into()),
    ] {
        let id = node_id(&graph, title)?;
        graph[&id]["inputs"][key] = value;
    }
    let last_id = node_id(&graph, "LoadImage2")?;
    if let Some(last_filename) = last_filename {
        graph[&last_id]["inputs"]["image"] = last_filename.into();
    } else {
        let prompt_id = node_id(&graph, "Prompt")?;
        graph.as_object_mut().unwrap().remove(&last_id);
        graph[&prompt_id]["inputs"]
            .as_object_mut()
            .ok_or("Prompt has no inputs")?
            .remove("last_frame");
    }
    let output = node_id(&graph, "Save Video")?;
    Ok((graph, output))
}

pub fn validate_minimax_r2v(
    megapixels: f32,
    seconds: i32,
    prompt: &str,
    has_image: bool,
    has_video: bool,
    custom_size: Option<(i32, i32)>,
) -> Result<(), String> {
    validate_minimax_i2v(megapixels, seconds, prompt)?;
    if !has_image && !has_video {
        return Err("Choose a reference image or video".into());
    }
    match custom_size {
        Some((width, height)) => validate_r2v_custom_size(width, height),
        None => Ok(()),
    }
}

fn validate_r2v_custom_size(width: i32, height: i32) -> Result<(), String> {
    if width < 64 || height < 64 || width % 8 != 0 || height % 8 != 0 {
        return Err("Custom size must be at least 64 px and a multiple of 8".into());
    }
    super::scaled_size(width as u32, height as u32, 1.0)?;
    Ok(())
}

/// Drops pixel scalers fed by a removed node so no dangling link is left behind.
fn remove_scalers_of(graph: &mut Value, source: &str) {
    let orphans: Vec<String> = graph
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, node)| {
            node["class_type"] == "ImageScaleToTotalPixels" && node["inputs"]["image"][0] == source
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in orphans {
        graph.as_object_mut().unwrap().remove(&id);
    }
}

pub fn prepare_minimax_r2v(
    images: [Option<String>; 4],
    videos: [Option<(String, bool)>; 2],
    size_filename: &str,
    prompt: &str,
    megapixels: f32,
    seconds: i32,
    lightning: bool,
    seed: u64,
) -> Result<(Value, String), String> {
    validate_minimax_i2v(megapixels, seconds, prompt)?;
    if images.iter().all(Option::is_none) && videos.iter().all(Option::is_none) {
        return Err("Choose a reference image or video".into());
    }
    if size_filename.is_empty() {
        return Err("Cannot read reference size".into());
    }
    let mut graph: Value = serde_json::from_str(include_str!("../../comfyui/video.minimax_h3_r2v.json"))
        .map_err(|e| format!("Invalid Minimax R2V workflow: {e}"))?;
    let r2v = node_id(&graph, "MiniMax H3 Reference to Video")?;
    for i in 0..4 {
        let id = node_id(&graph, &format!("LoadImage{}", i + 1))?;
        if let Some(filename) = &images[i] {
            graph[&id]["inputs"]["image"] = filename.clone().into();
        } else {
            graph.as_object_mut().unwrap().remove(&id);
            graph[&r2v]["inputs"]
                .as_object_mut()
                .ok_or("MiniMax H3 Reference to Video has no inputs")?
                .remove(&format!("ref_images.ref_image_{i}"));
        }
    }
    for i in 0..2 {
        let id = node_id(&graph, &format!("LoadVideo{}", i + 1))?;
        if let Some((filename, has_audio)) = &videos[i] {
            graph[&id]["inputs"]["video"] = filename.clone().into();
            if !has_audio {
                graph[&r2v]["inputs"]
                    .as_object_mut()
                    .ok_or("MiniMax H3 Reference to Video has no inputs")?
                    .remove(&format!("ref_video_audios.ref_video_audio_{i}"));
            }
        } else {
            graph.as_object_mut().unwrap().remove(&id);
            remove_scalers_of(&mut graph, &id);
            let inputs = graph[&r2v]["inputs"]
                .as_object_mut()
                .ok_or("MiniMax H3 Reference to Video has no inputs")?;
            inputs.remove(&format!("ref_videos.ref_video_{i}"));
            inputs.remove(&format!("ref_video_audios.ref_video_audio_{i}"));
        }
    }
    let size_id = node_id(&graph, "RefImageSize")?;
    graph[&size_id]["inputs"]["image"] = size_filename.into();
    // The workflow scales the size reference and each reference video with its own
    // "Megapixels" node; all of them share the one requested value.
    for id in node_ids(&graph, "Megapixels")? {
        graph[&id]["inputs"]["megapixels"] = megapixels.into();
    }
    for (title, key, value) in [
        ("Prompt", "value", prompt.into()),
        ("Duration", "value", seconds.into()),
        ("RandomNoise", "noise_seed", seed.into()),
        ("Boolean (Enable Lightning LoRA)", "value", lightning.into()),
    ] {
        let id = node_id(&graph, title)?;
        graph[&id]["inputs"][key] = value;
    }
    let output = node_id(&graph, "Save Video")?;
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
    fn minimax_i2v_patches_image_prompt_megapixels_duration_and_seed() {
        let original: Value =
            serde_json::from_str(include_str!("../../comfyui/video.minimax_h3_i2v.json")).unwrap();
        let (graph, save) =
            prepare_minimax_i2v("folder/input.png", None, "Mouse orbits", 0.5, 6, false, 42)
                .unwrap();
        assert_eq!(save, node_id(&original, "Save Video").unwrap());
        assert_eq!(
            graph[node_id(&graph, "LoadImage").unwrap()]["inputs"]["image"],
            "folder/input.png"
        );
        assert!(node_id(&graph, "LoadImage2").is_err());
        assert!(graph[node_id(&graph, "Prompt").unwrap()]["inputs"]
            .get("last_frame")
            .is_none());
        let (mut with_last, _) = prepare_minimax_i2v(
            "folder/input.png",
            Some("last.png"),
            "Mouse orbits",
            0.5,
            6,
            false,
            42,
        )
        .unwrap();
        for (title, key, expected) in [
            ("LoadImage", "image", Value::from("folder/input.png")),
            ("LoadImage2", "image", Value::from("last.png")),
            ("Megapixels", "megapixels", 0.5.into()),
            ("Prompt", "prompt", "Mouse orbits".into()),
            ("Float (duration)", "value", 6.into()),
            ("RandomNoise", "noise_seed", 42.into()),
            ("Boolean (Enable Lightning LoRA)", "value", false.into()),
        ] {
            let id = node_id(&with_last, title).unwrap();
            assert_eq!(with_last[&id]["inputs"][key], expected);
            with_last[&id]["inputs"][key] = original[&id]["inputs"][key].clone();
        }
        assert_eq!(with_last, original);
        let turbo =
            prepare_minimax_i2v("input.png", None, "x", 0.5, 5, true, 0).unwrap().0;
        assert_eq!(
            turbo[node_id(&turbo, "Boolean (Enable Lightning LoRA)").unwrap()]["inputs"]["value"],
            true
        );
        for megapixels in [0.19, 1.01, f32::NAN, f32::INFINITY] {
            assert!(prepare_minimax_i2v("input.png", None, "x", megapixels, 5, false, 0).is_err());
        }
        assert!(prepare_minimax_i2v("input.png", None, "x", 0.5, 0, false, 0).is_err());
        assert!(prepare_minimax_i2v("input.png", None, "   ", 0.5, 5, false, 0).is_err());
        assert!(validate_minimax_i2v(0.2, 1, "ok").is_ok());
        assert!(validate_minimax_i2v(1.0, 120, "ok").is_ok());
    }

    #[test]
    fn minimax_r2v_drops_unused_load_images_and_keeps_enabled_slots() {
        let original: Value =
            serde_json::from_str(include_str!("../../comfyui/video.minimax_h3_r2v.json")).unwrap();
        let (graph, save) = prepare_minimax_r2v(
            [
                None,
                Some("b.png".into()),
                Some("c.png".into()),
                None,
            ],
            [None, None],
            "size.webp",
            "Two refs",
            0.4,
            7,
            true,
            9,
        )
        .unwrap();
        assert_eq!(save, node_id(&original, "Save Video").unwrap());
        let r2v = node_id(&graph, "MiniMax H3 Reference to Video").unwrap();
        assert!(node_id(&graph, "LoadImage1").is_err());
        assert_eq!(graph[node_id(&graph, "LoadImage2").unwrap()]["inputs"]["image"], "b.png");
        assert_eq!(graph[node_id(&graph, "LoadImage3").unwrap()]["inputs"]["image"], "c.png");
        assert!(node_id(&graph, "LoadImage4").is_err());
        assert_eq!(graph[node_id(&graph, "RefImageSize").unwrap()]["inputs"]["image"], "size.webp");
        assert!(graph[&r2v]["inputs"].get("ref_images.ref_image_0").is_none());
        assert_eq!(graph[&r2v]["inputs"]["ref_images.ref_image_1"], serde_json::json!(["139", 0]));
        assert_eq!(graph[&r2v]["inputs"]["ref_images.ref_image_2"], serde_json::json!(["149", 0]));
        assert!(graph[&r2v]["inputs"].get("ref_images.ref_image_3").is_none());
        assert!(node_id(&graph, "LoadVideo1").is_err());
        assert!(node_id(&graph, "LoadVideo2").is_err());
        assert!(graph[&r2v]["inputs"].get("ref_videos.ref_video_0").is_none());
        assert!(graph[&r2v]["inputs"].get("ref_videos.ref_video_1").is_none());
        assert!(graph[&r2v]["inputs"].get("ref_video_audios.ref_video_audio_0").is_none());
        assert!(graph[&r2v]["inputs"].get("ref_video_audios.ref_video_audio_1").is_none());
        assert_eq!(graph[node_id(&graph, "Boolean (Enable Lightning LoRA)").unwrap()]["inputs"]["value"], true);
        let scalers = node_ids(&graph, "Megapixels").unwrap();
        assert_eq!(scalers.len(), 1, "scalers of removed videos are dropped");
        assert_eq!(graph[&scalers[0]]["inputs"]["megapixels"], Value::from(0.4_f32));
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
        let (graph, _) = prepare_minimax_r2v(
            [None, None, None, None],
            [Some(("clip.mp4".into(), false)), None],
            "size.webp",
            "x",
            0.9,
            5,
            false,
            1,
        )
        .unwrap();
        assert_eq!(
            graph[node_id(&graph, "LoadVideo1").unwrap()]["inputs"]["video"],
            "clip.mp4"
        );
        assert!(node_id(&graph, "LoadVideo2").is_err());
        let scalers = node_ids(&graph, "Megapixels").unwrap();
        assert_eq!(scalers.len(), 2);
        assert!(scalers.iter().all(|id| graph[id]["inputs"]["megapixels"] == Value::from(0.9_f32)));
        assert_eq!(
            graph[node_id(&graph, "RefImageSize").unwrap()]["inputs"]["image"],
            "size.webp"
        );
        assert_eq!(
            graph[node_id(&graph, "MiniMax H3 Reference to Video").unwrap()]["inputs"]
                ["ref_videos.ref_video_0"],
            serde_json::json!(["157", 0])
        );
        assert!(
            graph[node_id(&graph, "MiniMax H3 Reference to Video").unwrap()]["inputs"]
                .get("ref_video_audios.ref_video_audio_0")
                .is_none()
        );
        let (mut full, _) = prepare_minimax_r2v(
            [
                Some("a.png".into()),
                Some("b.png".into()),
                Some("c.png".into()),
                Some("d.png".into()),
            ],
            [Some(("0001-0015.mp4".into(), true)), Some(("clip-2.mp4".into(), true))],
            "size.webp",
            "x",
            0.9,
            5,
            false,
            1,
        )
        .unwrap();
        for id in node_ids(&full, "Megapixels").unwrap() {
            assert_eq!(full[&id]["inputs"]["megapixels"], Value::from(0.9_f32));
            full[&id]["inputs"]["megapixels"] = original[&id]["inputs"]["megapixels"].clone();
        }
        for (title, key) in [
            ("LoadImage1", "image"),
            ("LoadImage2", "image"),
            ("LoadImage3", "image"),
            ("LoadImage4", "image"),
            ("RefImageSize", "image"),
            ("LoadVideo1", "video"),
            ("LoadVideo2", "video"),
            ("Prompt", "value"),
            ("Duration", "value"),
            ("RandomNoise", "noise_seed"),
            ("Boolean (Enable Lightning LoRA)", "value"),
        ] {
            let id = node_id(&full, title).unwrap();
            full[&id]["inputs"][key] = original[&id]["inputs"][key].clone();
        }
        assert_eq!(full, original);
        assert!(prepare_minimax_r2v(
            [None, None, None, None],
            [None, None],
            "size.webp",
            "x",
            0.5,
            5,
            true,
            0
        )
        .is_err());
        assert!(validate_minimax_r2v(0.5, 5, "x", false, true, None).is_ok());
        assert!(validate_minimax_r2v(0.5, 5, "x", false, true, Some((512, 288))).is_ok());
        assert!(validate_minimax_r2v(0.5, 5, "x", true, false, None).is_ok());
        assert!(validate_minimax_r2v(0.5, 5, "x", false, false, Some((512, 512))).is_err());
        assert!(validate_minimax_r2v(0.5, 5, "x", true, false, Some((63, 512))).is_err());
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
