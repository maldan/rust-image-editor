use super::*;

#[test]
fn merge_down_restores_both_layers_and_prior_edits_through_undo_redo() {
    let mut app = app_with_layers(3);
    paint_pixel(&mut app, 2, 80);
    let before = app.project().layers.clone();
    app.project_mut().active_layer = 2;
    app.merge_layer_down();
    assert_eq!(app.project().layers.len(), 2);
    assert_eq!(app.project().active_layer, 1);
    assert_eq!(app.project().layers[0].pixels, before[0].pixels);
    assert!(app.layers_reload && app.dirty);
    let merged = app.project().layers[1].pixels.clone();
    app.undo();
    assert_eq!(app.project().layers.len(), 3);
    assert_eq!(app.project().active_layer, 2);
    for (restored, original) in app.project().layers.iter().zip(&before) {
        assert_eq!(restored.name, original.name);
        assert_eq!(restored.opacity, original.opacity);
        assert_eq!(restored.visible, original.visible);
        assert_eq!(restored.pixels, original.pixels);
    }
    app.undo();
    app.redo();
    app.redo();
    assert_eq!(app.project().layers.len(), 2);
    assert_eq!(app.project().layers[1].pixels, merged);
    app.undo();
    paint_pixel(&mut app, 2, 90);
    app.redo();
    assert_eq!(app.project().layers.len(), 3);
}

#[test]
fn merge_down_composites_animation_keys_and_restores_them() {
    use crate::raster::Raster;
    let mut app = EditorApp::default();
    app.new_animation();
    app.project_mut().resize_canvas(2, 2).unwrap();
    app.project_mut().layers[0]
        .pixels
        .write(0, 0, 0, [10, 0, 0, 255]);
    app.project_mut().add_layer();
    app.seek_animation(4);
    app.animation_drawing(false);
    app.project_mut().layers[1]
        .pixels
        .write(0, 0, 0, [0, 20, 0, 255]);
    app.project_mut().active_layer = 1;
    app.merge_layer_down();
    assert_eq!(app.project().layers.len(), 1);
    assert_eq!(app.project().animation.as_ref().unwrap().frame, 4);
    let layer = &app.project().layers[0];
    assert_eq!(layer.drawings[&0].read(0, 0, 0), [10, 0, 0, 255]);
    assert!(!layer.drawings.contains_key(&1));
    assert_eq!(layer.drawings[&4].read(0, 0, 0), [0, 20, 0, 255]);
    assert_eq!(layer.pixels.read(0, 0, 0), [0, 20, 0, 255]);
    app.undo();
    assert_eq!(app.project().layers.len(), 2);
    assert!(app.project().layers[1].drawings.contains_key(&4));
    assert_eq!(
        app.project().layers[0].drawings[&0].read(0, 0, 0),
        [10, 0, 0, 255]
    );
    app.redo();
    assert_eq!(app.project().layers.len(), 1);
    assert_eq!(
        app.project().layers[0].drawings[&4].read(0, 0, 0),
        [0, 20, 0, 255]
    );
}

#[test]
fn merge_down_does_nothing_for_empty_document_or_bottom_layer() {
    let mut app = EditorApp::default();
    app.merge_layer_down();
    assert!(app.project().layers.is_empty());
    let mut app = app_with_layers(2);
    app.project_mut().active_layer = 0;
    let before = app.project().layers[0].pixels.clone();
    app.merge_layer_down();
    assert_eq!(app.project().layers.len(), 2);
    assert_eq!(app.project().layers[0].pixels, before);
}
use crate::raster::{Raster, SparsePixels};

fn app_with_layers(count: usize) -> EditorApp {
    let mut app = EditorApp::default();
    app.project_mut()
        .import_rgba(&[1, 2, 3, 255].repeat(8), 4, 2)
        .unwrap();
    for _ in 1..count {
        app.project_mut().add_layer();
    }
    for (i, layer) in app.project_mut().layers.iter_mut().enumerate() {
        layer.name = format!("Layer {i}");
        layer.visible = i % 2 == 0;
        layer.opacity = 0.25 * (i + 1) as f32;
        layer.pixels = SparsePixels::from_rgba(&vec![(i + 1) as u8; 4 * 2 * 4], 4, 2);
    }
    app.refresh_image().unwrap();
    app
}

#[test]
fn deletion_restores_each_layers_order_metadata_pixels_and_active_layer() {
    for index in 0..3 {
        let mut app = app_with_layers(3);
        let before = app.project().layers.clone();
        for _ in 0..3 {
            app.delete_layer(index);
            assert_eq!(app.project().layers.len(), 2);
            assert_eq!(app.project().active_layer, 1);
            assert!(app.layers_reload);
            assert!(app.workspace.active_doc().thumb_dirty);
            app.undo();
            assert_eq!(app.project().active_layer, 2);
            for (restored, original) in app.project().layers.iter().zip(&before) {
                assert_eq!(restored.name, original.name);
                assert_eq!(restored.visible, original.visible);
                assert_eq!(restored.opacity, original.opacity);
                assert_eq!(restored.pixels, original.pixels);
            }
            app.redo();
            assert_eq!(app.project().layers.len(), 2);
            app.undo();
        }
    }
}

fn paint_pixel(app: &mut EditorApp, layer: usize, value: u8) {
    let p = app.project_mut();
    let mut capture = EditCapture::new(layer, p.width, p.height);
    capture.touch_rect(
        &p.layers[layer].pixels,
        PixelRect {
            min_x: 0,
            min_y: 0,
            max_x: 1,
            max_y: 1,
        },
    );
    let mut pixel = p.layers[layer].pixels.read(p.width, 0, 0);
    pixel[0] = value;
    p.layers[layer].pixels.write(p.width, 0, 0, pixel);
    let edit = capture.finish(&mut p.layers[layer].pixels).unwrap();
    app.history.push(edit);
}

#[test]
fn last_layer_deletion_restores_canvas_and_keeps_pixel_history() {
    let mut app = app_with_layers(1);
    paint_pixel(&mut app, 0, 99);
    app.delete_layer(0);
    assert!(app.image_size.is_none());
    assert_eq!((app.project().width, app.project().height), (0, 0));
    app.undo();
    assert_eq!(app.image_size, Some((4, 2)));
    assert_eq!(app.stroke_marks.len(), 8);
    assert!(app.layers_reload);
    assert_eq!(
        app.project().layers[0]
            .pixels
            .read(app.project().width, 0, 0)[0],
        99
    );
    app.undo();
    assert_eq!(
        app.project().layers[0]
            .pixels
            .read(app.project().width, 0, 0)[0],
        1
    );
    app.redo();
    assert_eq!(
        app.project().layers[0]
            .pixels
            .read(app.project().width, 0, 0)[0],
        99
    );
    app.redo();
    assert!(app.image_size.is_none());
    app.undo();
    assert_eq!(
        app.project().layers[0]
            .pixels
            .read(app.project().width, 0, 0)[0],
        99
    );
}

#[test]
fn edits_before_and_after_deletion_target_correct_layers_and_new_edit_clears_redo() {
    let mut app = app_with_layers(3);
    paint_pixel(&mut app, 2, 80);
    app.delete_layer(0);
    paint_pixel(&mut app, 1, 90);
    app.project_mut().active_layer = 0;
    app.undo();
    assert_eq!(
        app.project().layers[1]
            .pixels
            .read(app.project().width, 0, 0)[0],
        80
    );
    assert_eq!(app.dirty_layer, Some(1));
    app.undo();
    assert_eq!(app.project().layers.len(), 3);
    app.undo();
    assert_eq!(
        app.project().layers[2]
            .pixels
            .read(app.project().width, 0, 0)[0],
        3
    );
    app.redo();
    app.redo();
    app.redo();
    assert_eq!(
        app.project().layers[1]
            .pixels
            .read(app.project().width, 0, 0)[0],
        90
    );
    app.undo();
    paint_pixel(&mut app, 1, 100);
    app.redo();
    assert_eq!(
        app.project().layers[1]
            .pixels
            .read(app.project().width, 0, 0)[0],
        100
    );
    app.delete_layer(1);
    app.undo();
    app.add_layer();
    app.redo();
    assert_eq!(app.project().layers.len(), 3);
}

#[test]
fn deleting_a_document_restores_its_pixels_and_keeps_later_edits_above_it() {
    let mut app = app_with_layers(1);
    let original = app.project().layers[0].pixels.clone();
    app.workspace.active_mut().layers[0].name = "Kept".into();
    app.new_document();
    app.project_mut()
        .import_rgba(&[4, 5, 6, 255].repeat(4), 2, 2)
        .unwrap();
    let survivor = app.workspace.active_doc().id.clone();
    app.delete_document(0);
    paint_pixel(&mut app, 0, 90);
    assert_eq!(app.workspace.documents.len(), 1);
    assert_eq!(app.workspace.active_doc().id, survivor);
    app.undo();
    assert_eq!(
        app.project().layers[0].pixels.read(2, 0, 0)[0],
        4,
        "paint undo happens before the document returns"
    );
    assert_eq!(app.workspace.documents.len(), 1);
    app.undo();
    assert_eq!(app.workspace.documents.len(), 2);
    assert_eq!(app.workspace.documents[0].name, "Document 1");
    assert_eq!(app.workspace.documents[0].project.layers[0].name, "Kept");
    assert_eq!(
        app.workspace.documents[0].project.layers[0].pixels,
        original
    );
    assert_eq!(app.workspace.active_doc().id, survivor);
    app.redo();
    assert_eq!(app.workspace.documents.len(), 1);
    app.undo();
    assert_eq!(
        app.workspace.documents[0].project.layers[0].pixels,
        original
    );
}

#[test]
fn deleting_the_open_document_comes_back_on_screen() {
    let mut app = app_with_layers(1);
    app.workspace.active_mut().layers[0].name = "Open".into();
    let id = app.workspace.active_doc().id.clone();
    app.new_document();
    app.switch_document(0);
    app.delete_document(0);
    assert_eq!(app.workspace.documents.len(), 1);
    assert_ne!(app.workspace.active_doc().id, id);
    app.undo();
    assert_eq!(app.workspace.active_doc().id, id);
    assert_eq!(app.workspace.active_doc().project.layers[0].name, "Open");
    assert_eq!(app.image_size, Some((4, 2)));
    app.redo();
    assert_eq!(app.workspace.documents.len(), 1);
    assert_ne!(app.workspace.active_doc().id, id);
}

#[test]
fn switching_documents_still_undoes_a_deletion() {
    let mut app = app_with_layers(1);
    app.new_document();
    app.new_document();
    assert_eq!(app.workspace.documents.len(), 3);
    app.delete_document(1);
    app.switch_document(0);
    app.undo();
    assert_eq!(app.workspace.documents.len(), 3);
    assert_eq!(app.workspace.documents[1].name, "Document 2");
}

#[test]
fn last_document_cannot_be_deleted_and_duplicate_copies_pixels() {
    let mut app = app_with_layers(1);
    app.delete_document(0);
    assert_eq!(app.workspace.documents.len(), 1);
    app.undo();
    assert_eq!(app.workspace.documents.len(), 1);
    let pixels = app.project().layers[0].pixels.clone();
    app.duplicate_document(0);
    assert_eq!(app.workspace.documents.len(), 2);
    assert_ne!(app.workspace.documents[0].id, app.workspace.documents[1].id);
    assert_eq!(app.workspace.documents[1].project.layers[0].pixels, pixels);
    assert_eq!(app.workspace.active_document, 1);
    assert!(app.workspace.active_doc().name.ends_with("copy"));
    assert!(app.workspace.active_doc().memory_bytes() > 0);
}
