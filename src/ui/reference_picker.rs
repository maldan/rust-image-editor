use crate::{host::THUMB_ATLAS, raster::Raster, workspace::Workspace};

use super::preview_select::Preview;

const LAYER_ATLAS: u32 = 1 << 24;
const EDGE: u32 = 48;

#[derive(Default)]
pub struct ReferencePicker {
    pub open: Option<String>,
    pub layers: Option<Result<LayerPreviews, String>>,
}

impl ReferencePicker {
    pub fn sync_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut mega_ui::wgpu::UiRenderer,
    ) {
        if let Some(Ok(atlas)) = &mut self.layers
            && let Some(pixels) = atlas.pixels.take()
        {
            renderer.set_texture_rgba(
                device, queue, LAYER_ATLAS, &pixels, atlas.width, atlas.height, "reference layers",
            );
        }
    }
}

pub fn document_preview(workspace: &Workspace, index: usize) -> Preview {
    let (uv_min, uv_max) = workspace.thumb_uv(workspace.assets.len() + index);
    Preview { texture: THUMB_ATLAS, uv_min, uv_max }
}

pub struct LayerPreviews {
    pixels: Option<Vec<u8>>,
    width: u32,
    height: u32,
}

impl LayerPreviews {
    pub fn load(workspace: &mut Workspace, index: usize) -> Result<Self, String> {
        workspace.ensure_document_loaded(index)?;
        let project = &workspace.documents[index].project;
        let count = project.layers.len() as u32;
        let columns = count.clamp(1, 32);
        let width = columns * EDGE;
        let height = count.div_ceil(columns).max(1) * EDGE;
        let mut pixels = vec![0; (width * height * 4) as usize];

        let (source_w, source_h) = (project.width, project.height);
        if source_w > 0 && source_h > 0 {
            let scale = EDGE as f32 / source_w.max(source_h) as f32;
            let w = (source_w as f32 * scale).round().clamp(1.0, EDGE as f32) as u32;
            let h = (source_h as f32 * scale).round().clamp(1.0, EDGE as f32) as u32;
            for (index, layer) in project.layers.iter().enumerate() {
                let (left, top) = cell_origin(index, columns);
                let x = left + (EDGE - w) / 2;
                let y = top + (EDGE - h) / 2;
                for row in 0..h {
                    let sy = ((f64::from(row) + 0.5) * f64::from(source_h) / f64::from(h)) as i32;
                    for col in 0..w {
                        let sx = ((f64::from(col) + 0.5) * f64::from(source_w) / f64::from(w)) as i32;
                        let rgba = layer.pixels.read(source_w, sx, sy);
                        let alpha = f32::from(rgba[3]) / 255.0;
                        let checker = if (col / 5 + row / 5) % 2 == 0 { 112.0 } else { 146.0 };
                        let dst = (((y + row) * width + x + col) * 4) as usize;
                        for channel in 0..3 {
                            pixels[dst + channel] = (checker * (1.0 - alpha)
                                + f32::from(rgba[channel]) * alpha).round() as u8;
                        }
                        pixels[dst + 3] = 255;
                    }
                }
            }
        }
        Ok(Self { pixels: Some(pixels), width, height })
    }

    pub fn preview(&self, index: usize) -> Preview {
        let (x, y) = cell_origin(index, self.width / EDGE);
        let uv = |x: u32, y: u32| [x as f32 / self.width as f32, y as f32 / self.height as f32];
        Preview { texture: LAYER_ATLAS, uv_min: uv(x, y), uv_max: uv(x + EDGE, y + EDGE) }
    }
}

fn cell_origin(index: usize, columns: u32) -> (u32, u32) {
    let index = index as u32;
    (index % columns * EDGE, index / columns * EDGE)
}
