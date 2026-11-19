use glam::Vec2;
use mega_ui::Ui;

use crate::{app::EditorApp, project::Project, view::{CanvasView, rotate}};

#[derive(Default)]
pub struct LayerGrid {
    pub document: Option<String>,
    pub pan: Vec2,
    pub zoom: Option<f32>,
    pub rotation: f32,
    pub max_per_row: i32,
}

impl LayerGrid {
    pub fn layout(&self, project: &Project) -> Layout {
        Layout::new(project, self.max_per_row.max(0) as usize)
    }
}

pub struct Layout {
    indices: Vec<usize>,
    columns: usize,
    rows: usize,
    image_size: Vec2,
    gap: Vec2,
}

impl Layout {
    fn new(project: &Project, max_per_row: usize) -> Self {
        let indices: Vec<_> = project.layers.iter().enumerate().rev()
            .filter_map(|(index, layer)| layer.visible.then_some(index))
            .collect();
        let columns = if max_per_row == 0 {
            indices.len()
        } else {
            indices.len().min(max_per_row)
        }.max(1);
        let rows = indices.len().div_ceil(columns).max(1);
        let image_size = Vec2::new(project.width.max(1) as f32, project.height.max(1) as f32);
        let gap = Vec2::splat((image_size.min_element() * 0.1).max(48.0));
        Self { indices, columns, rows, image_size, gap }
    }

    pub fn bounds(&self) -> Vec2 {
        self.stride() * Vec2::new(self.columns as f32, self.rows as f32)
            - Vec2::new(self.gap.x, 0.0)
    }

    fn stride(&self) -> Vec2 {
        self.image_size + self.gap
    }

    pub fn centers(&self, view: CanvasView) -> impl Iterator<Item = (usize, Vec2)> + '_ {
        let origin = (self.image_size - self.bounds()) * (0.5 * view.zoom);
        let step = self.stride() * view.zoom;
        self.indices.iter().enumerate().map(move |(cell, &index)| {
            let position = Vec2::new((cell % self.columns) as f32, (cell / self.columns) as f32);
            (index, view.center + rotate(origin + position * step, view.rotation))
        })
    }
}

pub fn draw_labels(ui: &mut Ui, app: &EditorApp) {
    let layout = app.layer_grid.layout(app.project());
    if layout.indices.is_empty() {
        ui.text_at(app.session.canvas_rect.min + Vec2::splat(16.0), "No visible layers");
        return;
    }
    let Some(view) = app.canvas_view() else { return };
    let half_size = layout.image_size * (0.5 * view.zoom);
    let font_size = (layout.gap.y * view.zoom * 0.6)
        .min(ui.theme().metrics.font_size * ui.scale());
    if font_size < 6.0 {
        return;
    }
    for (index, center) in layout.centers(view) {
        let offset = Vec2::new(-half_size.x, half_size.y + font_size * 0.25);
        let position = center + rotate(offset, view.rotation);
        ui.text_at_size(position, &app.project().layers[index].name, font_size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editing::Tool;
    use mega_ui::Rect;

    #[test]
    fn tools_can_be_selected_without_editing_the_grid_canvas() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[255; 4], 1, 1).unwrap();
        app.refresh_image().unwrap();
        let pixels = app.project().layers[0].pixels.clone();
        app.open_layer_grid();
        app.layer_grid.zoom = Some(1.0);
        app.session.canvas_rect = Rect::from_min_size(Vec2::ZERO, Vec2::splat(100.0));

        app.set_tool(Tool::Transform);
        assert_eq!(app.session.tool, Tool::Transform);
        assert!(app.session.floating.is_none());

        app.set_tool(Tool::Brush);
        let view = app.canvas_view().unwrap();
        app.input.mouse_pos = view.center;
        app.input.mouse_pressed = true;
        app.input.mouse_down = true;
        app.handle_canvas_edit(view);
        assert_eq!(app.project().layers[0].pixels, pixels);

        app.layer_grid.document = None;
        app.handle_canvas_edit(app.canvas_view().unwrap());
        assert_ne!(app.project().layers[0].pixels, pixels);
    }

    #[test]
    fn visible_layers_share_the_workspace_camera() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[255; 4], 1, 1).unwrap();
        app.project_mut().add_layer();
        app.project_mut().add_layer();
        app.project_mut().layers[1].visible = false;
        app.refresh_image().unwrap();
        app.open_layer_grid();
        app.layer_grid.zoom = Some(1.0);
        app.session.canvas_rect = Rect::from_min_size(Vec2::ZERO, Vec2::splat(500.0));

        let layout = app.layer_grid.layout(app.project());
        let before: Vec<_> = layout.centers(app.canvas_view().unwrap()).collect();
        assert_eq!([before[0].0, before[1].0], [2, 0]);
        assert_eq!(before[0].1.y, before[1].1.y);
        assert!(before[0].1.x < before[1].1.x);

        app.input.mouse_pos = before[0].1;
        app.input.scroll_delta.y = 350.0;
        app.update_view();
        let zoomed: Vec<_> = layout.centers(app.canvas_view().unwrap()).collect();
        assert!(zoomed[0].1.abs_diff_eq(before[0].1, 0.0001));
        assert!(zoomed[1].1.x > before[1].1.x);

        app.input.scroll_delta = Vec2::ZERO;
        app.input.mouse_middle_down = true;
        let delta = Vec2::new(30.0, -12.0);
        app.input.mouse_delta = delta;
        app.update_view();
        for ((_, moved), (_, old)) in layout.centers(app.canvas_view().unwrap()).zip(zoomed) {
            assert!(moved.abs_diff_eq(old + delta, 0.0001));
        }
        assert_eq!(app.session.zoom, 1.0);
        assert_eq!(app.session.pan, Vec2::ZERO);
    }

    #[test]
    fn max_per_row_wraps_visible_layers_only() {
        let mut project = Project::default();
        project.import_rgba(&[255; 4], 1, 1).unwrap();
        for _ in 0..4 {
            project.add_layer();
        }
        project.layers[3].visible = false;
        let grid = LayerGrid::default();
        assert_eq!(grid.max_per_row, 0);
        let view = CanvasView {
            center: Vec2::ZERO,
            zoom: 1.0,
            rotation: 0.0,
            width: 1,
            height: 1,
        };
        let unlimited: Vec<_> = grid.layout(&project).centers(view).collect();
        assert_eq!(unlimited.len(), 4);
        assert!(unlimited.iter().all(|(_, center)| center.y == unlimited[0].1.y));

        let grid = LayerGrid { max_per_row: 2, ..grid };
        let wrapped: Vec<_> = grid.layout(&project).centers(view).collect();
        assert_eq!(wrapped.iter().map(|&(index, _)| index).collect::<Vec<_>>(), [4, 2, 1, 0]);
        assert_eq!(wrapped[0].1.y, wrapped[1].1.y);
        assert_eq!(wrapped[2].1.y, wrapped[3].1.y);
        assert!(wrapped[2].1.y > wrapped[0].1.y);
        assert_eq!(wrapped[0].1.x, wrapped[2].1.x);
    }
}
