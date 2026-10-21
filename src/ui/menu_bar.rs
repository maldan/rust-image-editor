use mega_ui::Ui;

use crate::app::EditorApp;

pub fn draw(ui: &mut Ui, app: &mut EditorApp) {
    ui.menu("file", "File", |ui| {
        if ui.menu_item("new", "New Project").clicked() {
            app.new_project(ui);
        }
        if ui.menu_item("open", "Open Project…").clicked() {
            app.open_project(ui);
        }
        if ui.menu_item("save", "Save Project").clicked() {
            app.save_project(ui);
        }
        if ui.menu_item_enabled("optimize", "Optimize", app.workspace.path.is_some()).clicked() {
            app.optimize_project(ui);
        }
        ui.separator();
        if ui.menu_item("import", "Import Asset…").clicked() {
            app.import_image(ui);
        }
        if ui.menu_item("export", "Export PNG…").clicked() {
            app.export_image(ui, false);
        }
        if ui.menu_item("export_webp", "Export WebP…").clicked() {
            app.export_image(ui, true);
        }
    });

    ui.menu("edit", "Edit", |ui| {
        if ui.menu_item("undo", "Undo").clicked() {
            app.undo();
        }
        if ui.menu_item("redo", "Redo").clicked() {
            app.redo();
        }
        ui.separator();
        ui.menu("layer", "Layer", |ui| {
            let index = app.project().active_layer;
            let can_merge = index > 0 && index < app.project().layers.len();
            if ui.menu_item_enabled("merge_down", "Merge Down", can_merge).clicked() {
                app.merge_layer_down();
            }
        });
        ui.menu("mask", "Mask", |ui| {
            if ui.menu_item_enabled("invert", "Invert", app.session.selection.is_some()).clicked() {
                app.invert_selection();
            }
        });
        ui.add_enabled(app.session.selection.is_some(), |ui| {
            if ui.menu_item("cut_selection", "Cut (Ctrl+X)").clicked() {
                app.cut_selection(ui);
            }
        });
        ui.add_enabled(!app.project().layers.is_empty(), |ui| {
            if ui.menu_item("copy_layer", "Copy (Ctrl+C)").clicked() {
                app.copy_shortcut(ui);
            }
        });
        if ui.menu_item("paste_layer", "Paste (Ctrl+V)").clicked() {
            app.paste_clipboard(ui);
        }
        if ui.menu_item("paste_image", "Paste Image").clicked() {
            app.paste_image(ui);
        }
    });

    ui.menu("filter", "Filter", |ui| {
        if ui.menu_item_enabled("invert_colors", "Invert", !app.project().layers.is_empty()).clicked() {
            app.invert_layer();
        }
        if ui.menu_item_enabled("hsv_adjustment","HSV Adjustment…",!app.project().layers.is_empty()).clicked() {
            app.open_hsv();
        }
    });

    ui.menu("ai", "AI", |ui| {
        ui.menu("anime", "Anime", |ui| {
            if ui.menu_item("anime_draw", "Draw...").clicked() {
                app.open_ai_anime_draw();
            }
            if ui.menu_item_enabled("anime_redraw", "Redraw...", app.image_size.is_some()).clicked() {
                app.open_ai_anime_redraw();
            }
        });
        ui.menu("qwen_21", "Qwen 2.1", |ui| {
            if ui.menu_item("qwen_draw", "Draw...").clicked() {
                app.open_ai_qwen_draw();
            }
            if ui.menu_item_enabled("qwen_redraw", "Redraw...", app.image_size.is_some()).clicked() {
                app.open_ai();
            }
        });
        ui.add_enabled(app.image_size.is_some(), |ui| {
            if ui.menu_item("redraw_image", "Redraw Image...").clicked() {
                app.open_ai_redraw();
            }
            if ui.menu_item("inpaint_image", "Inpaint Image...").clicked() {
                app.open_ai_inpaint();
            }
            if ui.menu_item("to_lineart", "To Lineart...").clicked() {
                app.open_ai_lineart();
            }
            if ui.menu_item("draw_cn", "Draw with ControlNet...").clicked() {
                app.open_ai_draw_cn();
            }
            if ui.menu_item("upscale_image", "Upscale Image...").clicked() {
                app.open_ai_upscale();
            }
            ui.separator();
            if ui.menu_item("fix_alpha", "Fix Alpha").clicked() {
                app.fix_alpha();
            }
        });
        if ui.menu_item("draw_image", "Draw Image...").clicked() {
            app.open_ai_draw();
        }
    });
}
