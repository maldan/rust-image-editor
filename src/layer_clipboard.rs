use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

use crate::{project::Layer, raster::SparsePixels};

const PREFIX: &str = "mega-image-editor-layer\n";
const MAX_BYTES: usize = 256 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Payload {
    version: u32,
    canvas_width: u32,
    canvas_height: u32,
    name: String,
    visible: bool,
    opacity: f32,
    tiles: String,
    /// Selected pixels only. Paste starts a transform on the new layer.
    #[serde(default)]
    fragment: bool,
}

pub struct CopiedLayer {
    pub layer: Layer,
    pub canvas_size: (u32, u32),
    pub fragment: bool,
}

pub fn encode(layer: &Layer, canvas_size: (u32, u32)) -> Result<String, String> {
    encode_kind(layer, canvas_size, false)
}

pub fn encode_fragment(layer: &Layer, canvas_size: (u32, u32)) -> Result<String, String> {
    encode_kind(layer, canvas_size, true)
}

fn encode_kind(layer: &Layer, canvas_size: (u32, u32), fragment: bool) -> Result<String, String> {
    let tiles = layer.pixels.encode();
    if tiles.len() > MAX_BYTES {
        return Err("Layer is too large for the clipboard".into());
    }
    let payload = Payload {
        version: 1,
        canvas_width: canvas_size.0,
        canvas_height: canvas_size.1,
        name: layer.name.clone(),
        visible: layer.visible,
        opacity: layer.opacity,
        tiles: STANDARD.encode(tiles),
        fragment,
    };
    serde_json::to_string(&payload)
        .map(|json| format!("{PREFIX}{json}"))
        .map_err(|e| format!("Cannot copy layer: {e}"))
}

/// Ordinary text is not a layer; a recognized but corrupt layer reports an error.
pub fn decode(text: &str) -> Result<Option<CopiedLayer>, String> {
    let Some(json) = text.strip_prefix(PREFIX) else {
        return Ok(None);
    };
    if json.len() > MAX_BYTES / 3 * 4 + 1024 * 1024 {
        return Err("Clipboard layer is too large".into());
    }
    let payload: Payload =
        serde_json::from_str(json).map_err(|e| format!("Invalid clipboard layer: {e}"))?;
    if payload.version != 1 {
        return Err("Unsupported clipboard layer version".into());
    }
    crate::comfyui::scaled_size(payload.canvas_width, payload.canvas_height, 1.0)?;
    if !payload.opacity.is_finite() || !(0.0..=1.0).contains(&payload.opacity) {
        return Err("Invalid clipboard layer opacity".into());
    }
    let bytes = STANDARD
        .decode(&payload.tiles)
        .map_err(|_| "Invalid clipboard layer tiles")?;
    if bytes.len() > MAX_BYTES {
        return Err("Clipboard layer is too large".into());
    }
    let pixels = SparsePixels::decode(&bytes)?;
    // A foreign clipboard must not overflow signed coordinates during rendering.
    let side = crate::tiles::TILE_SIZE as i64;
    for &(tx, ty) in pixels.tiles.keys() {
        for (tile, offset) in [(tx, pixels.offset.0), (ty, pixels.offset.1)] {
            let start = tile as i64 * side + offset as i64;
            if start < i32::MIN as i64 || start + side > i32::MAX as i64 {
                return Err("Clipboard layer coordinates are out of range".into());
            }
        }
    }
    Ok(Some(CopiedLayer {
        layer: Layer {
            name: payload.name,
            visible: payload.visible,
            opacity: payload.opacity,
            pixels,
        },
        canvas_size: (payload.canvas_width, payload.canvas_height),
        fragment: payload.fragment,
    }))
}

pub fn write(text: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .map_err(|e| format!("Cannot copy layer to clipboard: {e}"))
}

pub fn read() -> Result<Option<CopiedLayer>, String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("Cannot open clipboard: {e}"))?;
    match clipboard.get_text() {
        Ok(text) => decode(&text),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(error) => Err(format!("Cannot read clipboard: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Raster;

    #[test]
    fn clipboard_round_trip_preserves_sparse_layer_and_metadata() {
        let mut pixels = SparsePixels::default();
        pixels.offset = (-120, 45);
        pixels.write(0, -100, -30, [1, 2, 3, 4]);
        pixels.write(0, 20000, 20000, [5, 6, 7, 255]);
        let layer = Layer {
            name: "Слой 🎨".into(),
            visible: false,
            opacity: 0.375,
            pixels,
        };
        let text = encode(&layer, (1000, 800)).unwrap();
        let copied = decode(&text).unwrap().unwrap();
        assert_eq!(copied.canvas_size, (1000, 800));
        assert_eq!(copied.layer.name, layer.name);
        assert_eq!(copied.layer.visible, layer.visible);
        assert_eq!(copied.layer.opacity, layer.opacity);
        assert_eq!(copied.layer.pixels, layer.pixels);
        assert_eq!(copied.layer.pixels.bounds(), layer.pixels.bounds());
        assert!(!copied.fragment);
        assert!(text.len() < 100000); // Pack physical tiles, not the huge bounding rectangle.
        let fragment = encode_fragment(&layer, (1000, 800)).unwrap();
        assert!(decode(&fragment).unwrap().unwrap().fragment);
    }

    #[test]
    fn plain_text_is_ignored_but_invalid_layer_payload_is_rejected() {
        assert!(decode("ordinary copied text").unwrap().is_none());
        assert!(decode(&format!("{PREFIX}broken")).is_err());
        let layer = Layer {
            name: "empty".into(),
            visible: true,
            opacity: 1.0,
            pixels: SparsePixels::default(),
        };
        let text = encode(&layer, (64, 64)).unwrap();
        let copied = decode(&text).unwrap().unwrap();
        assert_eq!(copied.layer.pixels, layer.pixels);
        let original: serde_json::Value =
            serde_json::from_str(text.strip_prefix(PREFIX).unwrap()).unwrap();
        for (key, value) in [
            ("version", serde_json::json!(2)),
            ("canvas_width", serde_json::json!(0)),
            ("opacity", serde_json::json!(2)),
            ("tiles", serde_json::json!("invalid")),
        ] {
            let mut payload = original.clone();
            payload[key] = value;
            assert!(decode(&format!("{PREFIX}{payload}")).is_err());
        }
    }
}
