use serde_json::Value;

pub const EMBEDDED: &str = include_str!("../../comfyui/image.qwen_edit_21.json");

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
