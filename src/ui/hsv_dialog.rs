use glam::Vec2;
use mega_ui::{Ui,Window};
use crate::{app::EditorApp,hsv::HsvAdjustment};

pub struct HsvDialog {
    pub open: bool,
    pub adjustment: HsvAdjustment,
    pub preview: bool,
    pub layer: usize,
    pub document_id: String,
}

impl Default for HsvDialog {
    fn default() -> Self {
        Self {open:false,adjustment:Default::default(),preview:true,layer:0,document_id:String::new()}
    }
}

fn row(ui:&mut Ui,id:&str,label:&str,value:&mut f32,min:f32,max:f32) {
    ui.label(label);
    let width = (ui.available_size().x-88.0*ui.scale()).max(100.0*ui.scale());
    ui.horizontal(|ui| {
        ui.flex_width(width,|ui| {ui.slider_stepped(id,value,min..=max,1.0);});
        ui.flex_width(80.0*ui.scale(),|ui| {ui.drag_float(&format!("{id}_number"),value,1.0);});
    });
    *value = value.round().clamp(min,max);
    ui.space(6.0);
}

pub fn draw(ui:&mut Ui,app:&mut EditorApp) {
    if !app.hsv.open { return; }
    let mut open = true;
    let mut apply = false;
    let mut cancel = false;
    let layer = app.project().layers.get(app.hsv.layer).map(|l|l.name.clone()).unwrap_or_default();
    // Keep the image's actual brightness visible while judging a color filter.
    let dim = ui.theme().window.modal_dim;
    ui.theme_mut().window.modal_dim = [0.0;4];
    ui.modal(Window::new("hsv_adjustment","HSV Adjustment").size(Vec2::new(540.0,390.0)).open(&mut open),|ui| {
        ui.label(&format!("Entire active layer: {layer}"));
        ui.space(8.0);
        row(ui,"hsv_hue","Hue (degrees)",&mut app.hsv.adjustment.hue,-180.0,180.0);
        row(ui,"hsv_saturation","Saturation (%)",&mut app.hsv.adjustment.saturation,-100.0,100.0);
        row(ui,"hsv_value","Value (%)",&mut app.hsv.adjustment.value,-100.0,100.0);
        ui.space(8.0);
        ui.horizontal(|ui| {
            ui.checkbox("hsv_preview","Preview",&mut app.hsv.preview);
            if ui.button("hsv_reset","Reset").clicked() { app.hsv.adjustment = Default::default(); }
            ui.spacer();
            apply = ui.button("hsv_apply","Apply").clicked();
            cancel = ui.button("hsv_cancel","Cancel").clicked();
        });
    });
    ui.theme_mut().window.modal_dim = dim;
    if !open || cancel || app.input.key_escape { app.hsv.open = false; }
    else if apply || app.input.key_enter { app.apply_hsv(); }
}
