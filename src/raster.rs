use std::collections::HashMap;
use std::sync::Arc;

use crate::tiles::TILE_SIZE;

pub const TILE_BYTES: usize = (TILE_SIZE * TILE_SIZE * 4) as usize;
pub type Tile = Arc<Vec<u8>>;

/// Pixel coordinates are signed and independent of the document bounds.
pub trait Raster {
    fn read(&self, width: u32, x: i32, y: i32) -> [u8; 4];
    fn write(&mut self, width: u32, x: i32, y: i32, pixel: [u8; 4]);
    fn packed_region(&self, stride: u32, x: i32, y: i32, width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity(width as usize * height as usize * 4);
        for py in y..y + height as i32 {
            for px in x..x + width as i32 {
                out.extend(self.read(stride, px, py));
            }
        }
        out
    }
}

impl Raster for [u8] {
    fn read(&self, width: u32, x: i32, y: i32) -> [u8; 4] {
        if x < 0 || y < 0 || x >= width as i32 {
            return [0; 4];
        }
        let i = (y as usize * width as usize + x as usize) * 4;
        self.get(i..i + 4)
            .map(|p| p.try_into().unwrap())
            .unwrap_or([0; 4])
    }
    fn write(&mut self, width: u32, x: i32, y: i32, pixel: [u8; 4]) {
        if x < 0 || y < 0 || x >= width as i32 {
            return;
        }
        let i = (y as usize * width as usize + x as usize) * 4;
        if let Some(p) = self.get_mut(i..i + 4) {
            p.copy_from_slice(&pixel);
        }
    }
}

impl Raster for Vec<u8> {
    fn read(&self, width: u32, x: i32, y: i32) -> [u8; 4] {
        self.as_slice().read(width, x, y)
    }
    fn write(&mut self, width: u32, x: i32, y: i32, pixel: [u8; 4]) {
        self.as_mut_slice().write(width, x, y, pixel);
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SparsePixels {
    pub offset: (i32, i32),
    pub(crate) tiles: HashMap<(i32, i32), Tile>,
}

impl SparsePixels {
    pub fn from_rgba(pixels: &[u8], width: u32, height: u32) -> Self {
        let mut result = Self::default();
        for ty in 0..height.div_ceil(TILE_SIZE) {
            for tx in 0..width.div_ceil(TILE_SIZE) {
                let mut tile = vec![0; TILE_BYTES];
                let x = tx * TILE_SIZE;
                let y = ty * TILE_SIZE;
                let w = (width - x).min(TILE_SIZE) as usize * 4;
                let h = (height - y).min(TILE_SIZE);
                for row in 0..h {
                    let src = ((y + row) as usize * width as usize + x as usize) * 4;
                    let dst = row as usize * TILE_SIZE as usize * 4;
                    if let Some(source) = pixels.get(src..src + w) {
                        tile[dst..dst + w].copy_from_slice(source);
                    }
                }
                if tile.iter().any(|&byte| byte != 0) {
                    result.tiles.insert((tx as i32, ty as i32), Arc::new(tile));
                }
            }
        }
        result
    }

    pub fn to_rgba(&self, width: u32, height: u32) -> Vec<u8> {
        self.region(0, 0, width, height)
    }

    /// Pack only the requested upload/export region, never the layer's full extent.
    pub fn region(&self, x: i32, y: i32, width: u32, height: u32) -> Vec<u8> {
        let mut out = vec![0; width as usize * height as usize * 4];
        if width == 0 || height == 0 {
            return out;
        }
        let lx = x - self.offset.0;
        let ly = y - self.offset.1;
        let side = TILE_SIZE as i32;
        for ty in ly.div_euclid(side)..=(ly + height as i32 - 1).div_euclid(side) {
            for tx in lx.div_euclid(side)..=(lx + width as i32 - 1).div_euclid(side) {
                let Some(tile) = self.tiles.get(&(tx, ty)) else {
                    continue;
                };
                let x0 = lx.max(tx * side);
                let x1 = (lx + width as i32).min((tx + 1) * side);
                let y0 = ly.max(ty * side);
                let y1 = (ly + height as i32).min((ty + 1) * side);
                for row in y0..y1 {
                    let src = ((row - ty * side) * side + x0 - tx * side) as usize * 4;
                    let dst = ((row - ly) as usize * width as usize + (x0 - lx) as usize) * 4;
                    let n = (x1 - x0) as usize * 4;
                    out[dst..dst + n].copy_from_slice(&tile[src..src + n]);
                }
            }
        }
        out
    }

    /// Exact nontransparent bounds in document coordinates.
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        for (&(tx, ty), tile) in &self.tiles {
            for (i, p) in tile.chunks_exact(4).enumerate() {
                if p[3] == 0 {
                    continue;
                }
                let x = tx * TILE_SIZE as i32 + (i % TILE_SIZE as usize) as i32 + self.offset.0;
                let y = ty * TILE_SIZE as i32 + (i / TILE_SIZE as usize) as i32 + self.offset.1;
                bounds = Some(match bounds {
                    Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x + 1), d.max(y + 1)),
                    None => (x, y, x + 1, y + 1),
                });
            }
        }
        bounds
    }

    pub fn prune(&mut self) {
        self.tiles
            .retain(|_, p| p.chunks_exact(4).any(|p| p[3] != 0));
    }

    /// Fixed-size physical tiles; no transparent bounding rectangle in the file.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + self.tiles.len() * (8 + TILE_BYTES));
        out.extend(self.offset.0.to_le_bytes());
        out.extend(self.offset.1.to_le_bytes());
        let mut keys: Vec<_> = self.tiles.keys().copied().collect();
        keys.sort_unstable();
        for (x, y) in keys {
            out.extend(x.to_le_bytes());
            out.extend(y.to_le_bytes());
            out.extend_from_slice(&self.tiles[&(x, y)]);
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 8 || (bytes.len() - 8) % (8 + TILE_BYTES) != 0 {
            return Err("Invalid tiled layer".into());
        }
        let int = |p: &[u8]| i32::from_le_bytes(p.try_into().unwrap());
        let mut result = Self {
            offset: (int(&bytes[..4]), int(&bytes[4..8])),
            ..Self::default()
        };
        for tile in bytes[8..].chunks_exact(8 + TILE_BYTES) {
            let key = (int(&tile[..4]), int(&tile[4..8]));
            // Keep coordinate arithmetic representable, including a full tile.
            let limit = i32::MAX / TILE_SIZE as i32 - 1;
            if key.0.abs_diff(0) > limit as u32
                || key.1.abs_diff(0) > limit as u32
                || result.tiles.contains_key(&key)
            {
                return Err("Invalid tile coordinates".into());
            }
            result.tiles.insert(key, Arc::new(tile[8..].to_vec()));
        }
        Ok(result)
    }
}

impl Raster for SparsePixels {
    fn packed_region(&self, _stride: u32, x: i32, y: i32, width: u32, height: u32) -> Vec<u8> {
        self.region(x, y, width, height)
    }
    fn read(&self, _width: u32, x: i32, y: i32) -> [u8; 4] {
        let x = x - self.offset.0;
        let y = y - self.offset.1;
        let side = TILE_SIZE as i32;
        let Some(tile) = self.tiles.get(&(x.div_euclid(side), y.div_euclid(side))) else {
            return [0; 4];
        };
        let i = (y.rem_euclid(side) * side + x.rem_euclid(side)) as usize * 4;
        tile[i..i + 4].try_into().unwrap()
    }
    fn write(&mut self, _width: u32, x: i32, y: i32, pixel: [u8; 4]) {
        let x = x - self.offset.0;
        let y = y - self.offset.1;
        let side = TILE_SIZE as i32;
        let key = (x.div_euclid(side), y.div_euclid(side));
        if pixel == [0; 4] && !self.tiles.contains_key(&key) {
            return;
        }
        let tile = self
            .tiles
            .entry(key)
            .or_insert_with(|| Arc::new(vec![0; TILE_BYTES]));
        let i = (y.rem_euclid(side) * side + x.rem_euclid(side)) as usize * 4;
        if tile[i..i + 4] != pixel {
            Arc::make_mut(tile)[i..i + 4].copy_from_slice(&pixel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distant_and_negative_pixels_allocate_only_physical_tiles() {
        let mut raster = SparsePixels::default();
        assert!(raster.tiles.is_empty());
        raster.write(0, -1, -65, [1, 2, 3, 255]);
        raster.write(0, 1_000_000, 1_000_000, [4, 5, 6, 255]);
        assert_eq!(raster.tiles.len(), 2);
        assert_eq!(raster.read(0, -1, -65), [1, 2, 3, 255]);
        assert_eq!(
            raster.region(-2, -66, 2, 2),
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 255]
        );
        let bytes = raster.encode();
        assert_eq!(bytes.len(), 8 + 2 * (8 + TILE_BYTES));
        assert_eq!(SparsePixels::decode(&bytes).unwrap(), raster);
    }

    #[test]
    fn snapshot_copies_only_the_modified_tile_and_offsets_do_not_copy_pixels() {
        let mut raster = SparsePixels::default();
        raster.write(0, 0, 0, [1, 2, 3, 255]);
        raster.write(0, 64, 0, [4, 5, 6, 255]);
        let before = raster.clone();
        raster.offset = (-10, 20);
        assert!(Arc::ptr_eq(&before.tiles[&(0, 0)], &raster.tiles[&(0, 0)]));
        raster.write(0, -10, 20, [7, 8, 9, 255]);
        assert!(!Arc::ptr_eq(&before.tiles[&(0, 0)], &raster.tiles[&(0, 0)]));
        assert!(Arc::ptr_eq(&before.tiles[&(1, 0)], &raster.tiles[&(1, 0)]));
        assert_eq!(before.read(0, 0, 0), [1, 2, 3, 255]);
        assert_eq!(raster.read(0, -10, 20), [7, 8, 9, 255]);
    }
}
