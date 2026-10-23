use super::*;
use crate::raster::Raster;

#[test]
fn opening_switching_previewing_and_dropping_load_only_the_required_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lazy.mgimg");
    let mut original = Workspace::default();
    original
        .import_asset_rgba("first".into(), &[10, 20, 30, 255].repeat(16), 4, 4)
        .unwrap();
    original
        .import_asset_rgba("second".into(), &[90, 80, 70, 128].repeat(6), 3, 2)
        .unwrap();
    original.place_asset_on_active(0).unwrap();
    original.create_document("Second");
    original.place_asset_on_active(1).unwrap();
    original.save(&path).unwrap();

    let mut app = EditorApp::default();
    app.workspace = Workspace::open(&path).unwrap();
    assert!(
        app.workspace
            .documents
            .iter()
            .all(|doc| doc.memory_bytes() == 0)
    );
    app.refresh_image().unwrap();
    assert_eq!(app.image_size, Some((3, 2)));
    assert!(app.workspace.documents[1].memory_bytes() > 0);
    assert_eq!(app.workspace.documents[0].memory_bytes(), 0);
    assert!(
        app.workspace
            .assets
            .iter()
            .all(|asset| asset.pixels.is_empty())
    );

    app.show_asset_preview(0);
    assert_eq!(app.display_size(), Some((4, 4)));
    assert_eq!(app.workspace.assets[0].pixels, original.assets[0].pixels);
    assert!(app.workspace.assets[1].pixels.is_empty());
    app.switch_document(0);
    assert!(app.session.preview_asset.is_none());
    assert_eq!(
        app.project().layers[0].pixels,
        original.documents[0].project.layers[0].pixels
    );
    let mut ui = Ui::new();
    // This is the same entry point used when an asset is dropped on the canvas.
    app.place_asset(1, &mut ui);
    assert_eq!(app.project().layers.len(), 2);
    assert_eq!(
        app.project().layers[1].pixels.read(0, 0, 0),
        [90, 80, 70, 128]
    );
    assert_eq!(app.workspace.assets[1].pixels, original.assets[1].pixels);
}

#[test]
fn deleting_active_document_loads_fallback_and_undo_restores_original() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delete.mgimg");
    let mut original = Workspace::default();
    original
        .active_mut()
        .import_rgba(&[10, 20, 30, 255], 1, 1)
        .unwrap();
    original.create_document("Second");
    original
        .active_mut()
        .import_rgba(&[90, 80, 70, 255], 1, 1)
        .unwrap();
    original.mark_active_thumb_dirty();
    original.save(&path).unwrap();
    let mut app = EditorApp::default();
    app.workspace = Workspace::open(&path).unwrap();
    app.refresh_image().unwrap();
    app.delete_document(1);
    assert_eq!(
        app.project().layers[0].pixels.read(0, 0, 0),
        [10, 20, 30, 255]
    );
    app.undo();
    assert_eq!(
        app.project().layers[0].pixels.read(0, 0, 0),
        [90, 80, 70, 255]
    );
}
