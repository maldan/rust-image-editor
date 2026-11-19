use glam::Vec2;
use mega_ui::{LayoutOpts, ScrollAxes, Ui};

pub struct Preview {
    pub texture: u32,
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
}

pub fn show(
    ui: &mut Ui,
    active: &mut Option<String>,
    id: &str,
    label: &str,
    contents: impl FnOnce(&mut Ui, bool) -> Option<usize>,
) -> Option<usize> {
    let was_open = active.as_deref() == Some(id);
    let mut open = was_open;
    let width = ui.available_size().x / ui.scale();
    let max_chars = ((width - 28.0) / (ui.theme().metrics.font_size * 0.65)).max(1.0) as usize;
    let label = if label.chars().count() > max_chars {
        let mut short: String = label.chars().take(max_chars - 1).collect();
        short.push('…');
        short
    } else {
        label.to_owned()
    };
    let selected = ui.popover_button(id, &label, &mut open, Vec2::new(280.0, 305.0), |ui| {
        let mut choice = None;
        ui.scroll_area("options", ui.available_size(), ScrollAxes::Vertical, |ui| {
            choice = contents(ui, !was_open);
        });
        choice
    }).flatten();
    if let Some(index) = selected {
        *active = None;
        ui.request_repaint();
        return Some(index);
    }
    if open {
        if !was_open {
            *active = Some(id.to_owned());
        }
    } else if was_open {
        *active = None;
    }
    None
}

pub fn row(ui: &mut Ui, id: &str, name: &str, selected: bool, preview: Preview, hidden: bool) -> bool {
    const IMAGE_SIZE: f32 = 44.0;
    const PADDING_REDUCTION: f32 = 5.0;
    const GAP: f32 = 6.0;
    const ICON_SIZE: f32 = 16.0;
    let mut image_clicked = false;
    let response = ui.selectable(id, selected, |ui| {
        let scale = ui.scale();
        let spacing = GAP + PADDING_REDUCTION;
        let icon_width = if hidden { ICON_SIZE + spacing } else { 0.0 };
        let label_width = (ui.available_size().x / scale - IMAGE_SIZE - spacing - icon_width)
            .max(1.0);
        let label_offset = ((IMAGE_SIZE - ui.theme().metrics.font_size) * 0.5 - 2.0).max(0.0);
        ui.row_with(LayoutOpts {
            spacing: Some(spacing * scale),
            ..Default::default()
        }, |ui| {
            let image = ui.area("preview", Vec2::splat(IMAGE_SIZE * scale));
            // Use half of the selectable's 10-point inset to enlarge the preview.
            let preview_rect = image.rect.inset(-PADDING_REDUCTION * scale);
            ui.fill_rect(preview_rect, [0.12, 0.12, 0.14, 1.0]);
            ui.texture_rect(preview.texture, preview_rect, preview.uv_min, preview.uv_max);
            image_clicked = image.hovered && image.active && ui.pointer().released;
            ui.flex_width(label_width * scale, |ui| {
                ui.space(label_offset);
                ui.label_fixed(label_width, name);
            });
            if hidden {
                ui.flex_width(ICON_SIZE * scale, |ui| {
                    ui.space((IMAGE_SIZE - ICON_SIZE) * 0.5);
                    ui.icon("visibility_off", ICON_SIZE);
                });
            }
        });
    });
    response.clicked() || image_clicked
}
