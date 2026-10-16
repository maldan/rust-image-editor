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

    pub fn pixel_corners(self, rect: crate::editing::PixelRect) -> [Vec2; 4] {
        [
            self.image_to_screen(Vec2::new(rect.min_x as f32, rect.min_y as f32)),
            self.image_to_screen(Vec2::new(rect.max_x as f32, rect.min_y as f32)),
            self.image_to_screen(Vec2::new(rect.max_x as f32, rect.max_y as f32)),
            self.image_to_screen(Vec2::new(rect.min_x as f32, rect.max_y as f32)),
        ]
    }
}

pub fn rotate(v: Vec2, radians: f32) -> Vec2 {
    let (sin, cos) = radians.sin_cos();
    Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}
