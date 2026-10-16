use std::collections::HashSet;

pub const TILE_SIZE: u32 = 64;

#[derive(Debug, Clone, Default)]
pub struct DirtyTiles {
    tiles: HashSet<(u32, u32)>,
    width: u32,
    height: u32,
}

impl DirtyTiles {
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.tiles.clear();
        self.mark_all();
    }

    pub fn clear(&mut self) {
        self.tiles.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn mark_all(&mut self) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let tiles_x = self.width.div_ceil(TILE_SIZE);
        let tiles_y = self.height.div_ceil(TILE_SIZE);
        self.tiles.reserve((tiles_x * tiles_y) as usize);
        for ty in 0..tiles_y {
            for tx in 0..tiles_x {
                self.tiles.insert((tx, ty));
            }
        }
    }

    pub fn mark_rect(&mut self, min_x: u32, min_y: u32, max_x: u32, max_y: u32) {
        if self.width == 0 || self.height == 0 || min_x >= max_x || min_y >= max_y {
            return;
        }
        let max_x = max_x.min(self.width);
        let max_y = max_y.min(self.height);
        let tx0 = min_x / TILE_SIZE;
        let ty0 = min_y / TILE_SIZE;
        let tx1 = (max_x - 1) / TILE_SIZE;
        let ty1 = (max_y - 1) / TILE_SIZE;
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                self.tiles.insert((tx, ty));
            }
        }
    }

    pub fn mark_brush_segment(&mut self, from: (f32, f32), to: (f32, f32), size: f32) {
        let radius = (size * 0.5).max(0.5);
        let min_x = from.0.min(to.0) - radius;
        let min_y = from.1.min(to.1) - radius;
        let max_x = from.0.max(to.0) + radius;
        let max_y = from.1.max(to.1) + radius;
        self.mark_rect(
            min_x.floor().max(0.0) as u32,
            min_y.floor().max(0.0) as u32,
            max_x.ceil().max(0.0) as u32,
            max_y.ceil().max(0.0) as u32,
        );
    }

    pub fn drain(&mut self) -> impl Iterator<Item = (u32, u32, u32, u32)> + '_ {
        let width = self.width;
        let height = self.height;
        self.tiles.drain().map(move |(tx, ty)| {
            let x = tx * TILE_SIZE;
            let y = ty * TILE_SIZE;
            let w = (width - x).min(TILE_SIZE);
            let h = (height - y).min(TILE_SIZE);
            (x, y, w, h)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brush_marks_overlapping_tiles_only() {
        let mut dirty = DirtyTiles::default();
        dirty.resize(200, 200);
        dirty.clear();
        dirty.mark_brush_segment((70.0, 70.0), (80.0, 80.0), 10.0);
        let rects: Vec<_> = dirty.drain().collect();
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0], (64, 64, 64, 64));
    }
}
