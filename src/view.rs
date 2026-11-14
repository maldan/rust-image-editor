use glam::Vec2;

#[derive(Clone, Copy)]
pub struct CanvasView {
    pub center: Vec2,
    pub zoom: f32,
    pub rotation: f32,
    pub width: u32,
    pub height: u32,
}

impl CanvasView {
    pub fn image_center(self) -> Vec2 {
        Vec2::new(self.width as f32, self.height as f32) * 0.5
    }

    pub fn image_to_screen(self, image: Vec2) -> Vec2 {
        let local = (image - self.image_center()) * self.zoom;
        self.center + rotate(local, self.rotation)
    }

    pub fn screen_to_image(self, screen: Vec2) -> Vec2 {
        let local = rotate(screen - self.center, -self.rotation) / self.zoom.max(1e-6);
        local + self.image_center()
    }

    /// Rubber-band corners for a drag that may extend outside the document.
    pub fn signed_pixel_corners(self, a: (i32, i32), b: (i32, i32)) -> [Vec2; 4] {
        let min_x = a.0.min(b.0) as f32;
        let min_y = a.1.min(b.1) as f32;
        let max_x = a.0.max(b.0).saturating_add(1) as f32;
        let max_y = a.1.max(b.1).saturating_add(1) as f32;
        [
            self.image_to_screen(Vec2::new(min_x, min_y)),
            self.image_to_screen(Vec2::new(max_x, min_y)),
            self.image_to_screen(Vec2::new(max_x, max_y)),
            self.image_to_screen(Vec2::new(min_x, max_y)),
        ]
    }
}

pub fn rotate(v: Vec2, radians: f32) -> Vec2 {
    let (sin, cos) = radians.sin_cos();
    Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}
