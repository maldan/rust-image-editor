    use super::*;
    use crate::raster::Raster;
    use std::sync::Arc;

    #[test]
    fn trim_canvas_commits_pending_transform_updates_fields_and_preserves_undo() {
        let mut app = EditorApp::default();
        app.project_mut().resize_canvas(100,80).unwrap();
        app.project_mut().layers[0].pixels.write(0,20,30,[100,150,200,255]);
        app.project_mut().layers[0].pixels.write(0,24,32,[200,150,100,255]);
        app.sync_layer_structure();
        app.begin_transform();
        app.session.floating.as_mut().unwrap().center += glam::Vec2::new(10.0,5.0);
        app.trim_canvas(&mut Ui::new());
        assert!(app.session.floating.is_none());
        assert_eq!(app.image_size,Some((5,3)));
        assert_eq!((app.resize_w,app.resize_h),(5,3));
        assert_eq!(app.project().layers[0].pixels.read(0,0,0),[100,150,200,255]);
        assert_eq!(app.project().layers[0].pixels.read(0,4,2),[200,150,100,255]);
        assert!(app.dirty && app.session.fit_image);
        app.undo();
        assert_eq!(app.image_size,Some((100,80)));
        assert_eq!((app.resize_w,app.resize_h),(100,80));
        assert_eq!(app.project().layers[0].pixels.read(0,30,35),[100,150,200,255]);
        app.undo();
        assert_eq!(app.project().layers[0].pixels.read(0,20,30),[100,150,200,255]);
        app.redo();
        app.redo();
        assert_eq!(app.image_size,Some((5,3)));
    }

    #[test]
    fn soft_brush_setting_reaches_raster_and_undo_restores_original_tiles() {
        let mut app = document();
        let original = app.project().layers[0].pixels.clone();
        app.session.brush_hardness = 0.0;
        app.session.brush_size = 8.0;
        app.session.brush_opacity = 0.5;
        app.session.brush_color = [1.0, 0.0, 0.0, 1.0];
        app.stroke_capture = Some(EditCapture::new(0, 10, 10));
        app.apply_brush_segment((4.5, 4.5), (4.5, 4.5), 10, 10, false);
        assert_eq!(app.project().layers[0].pixels.read(0, 4, 4), [255, 0, 0, 128]);
        assert_eq!(app.project().layers[0].pixels.read(0, 6, 4), [255, 0, 0, 64]);
        let painted = app.project().layers[0].pixels.clone();
        app.apply_brush_segment((4.5, 4.5), (4.5, 4.5), 10, 10, false);
        assert_eq!(app.project().layers[0].pixels, painted);
        let edit = app.stroke_capture.take().unwrap().finish(&mut app.project_mut().layers[0].pixels).unwrap();
        app.history.push(edit);
        app.undo();
        assert_eq!(app.project().layers[0].pixels, original);
        app.redo();
        assert_eq!(app.project().layers[0].pixels, painted);
    }

    #[test]
    fn ctrl_click_picks_the_composite_and_leaves_the_layer_unchanged() {
        let mut app = document();
        app.set_tool(Tool::Brush);
        app.project_mut().layers[0].opacity = 0.5;
        app.project_mut().add_layer();
        app.project_mut().layers[1].pixels.write(0, 4, 4, [0, 0, 255, 255]);
        let before = app.project().layers.clone();
        app.session.canvas_rect = Rect::from_min_size(Vec2::ZERO, Vec2::splat(400.0));
        app.session.brush_color = [1.0, 1.0, 1.0, 1.0];
        let view = app.canvas_view().unwrap();
        app.input.mouse_pos = view.image_to_screen(Vec2::new(2.5, 3.5));
        app.input.mouse_pressed = true;
        app.input.mouse_down = true;
        app.input.key_ctrl = true;
        assert!(app.eyedropper_armed());
        app.handle_canvas_edit(view);
        let covered = composite_pixel(&app.project().layers, 2, 3);
        assert_ne!(covered, [0, 0, 0, 0]);
        assert_eq!(app.session.brush_color, covered.map(|c| c as f32 / 255.0));
        app.input.mouse_pos = view.image_to_screen(Vec2::new(4.5, 4.5));
        app.handle_canvas_edit(view);
        assert_eq!(app.session.brush_color, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(app.project().layers[0].pixels, before[0].pixels);
        assert_eq!(app.project().layers[1].pixels, before[1].pixels);
        assert!(app.brush_last.is_none());
        assert!(app.canvas_present().unwrap().2.brush_center.is_none());
        app.input.key_ctrl = false;
        app.input.mouse_pressed = true;
        app.handle_canvas_edit(view);
        assert_ne!(app.project().layers[1].pixels, before[1].pixels);
        app.input.mouse_pressed = false;
        app.input.key_ctrl = true;
        assert!(!app.eyedropper_armed());
        app.set_tool(Tool::Fill);
        app.input.mouse_pos = view.image_to_screen(Vec2::new(2.5, 3.5));
        app.input.mouse_pressed = true;
        app.session.brush_color = [1.0, 1.0, 1.0, 1.0];
        let filled = app.project().layers.clone();
        let picked = composite_pixel(&app.project().layers, 2, 3);
        app.handle_canvas_edit(view);
        assert_eq!(app.session.brush_color, picked.map(|c| c as f32 / 255.0));
        assert_eq!(app.project().layers[0].pixels, filled[0].pixels);
        assert_eq!(app.project().layers[1].pixels, filled[1].pixels);
    }

    fn document() -> EditorApp {
        let mut app = EditorApp::default();
        app.project_mut().resize_canvas(10,10).unwrap();
        app.project_mut().layers[0].pixels.write(0,2,3,[100,20,30,255]);
        app.refresh_image().unwrap();
        app
    }

    #[test]
    fn whole_layer_moves_out_and_back_without_copying_or_losing_tiles_and_undoes() {
        let mut app = document();
        let original = app.project().layers[0].pixels.clone();
        for delta in [Vec2::new(-30.0,40.0),Vec2::new(30.0,-40.0)] {
            app.begin_transform();
            let floating = app.session.floating.as_mut().unwrap();
            assert!(floating.pixels.is_empty() && floating.mask.is_empty());
            floating.center += delta;
            app.commit_transform();
            assert!(Arc::ptr_eq(&original.tiles[&(0,0)],&app.project().layers[0].pixels.tiles[&(0,0)]));
        }
        assert_eq!(app.project().layers[0].pixels,original);
        app.undo();
        assert_eq!(app.project().layers[0].pixels.read(0,-28,43),[100,20,30,255]);
        app.redo();
        assert_eq!(app.project().layers[0].pixels,original);
    }

    #[test]
    fn selected_fragment_survives_negative_transform_save_and_canvas_resize() {
        let mut app = document();
        app.set_selection(Selection::from_rect(PixelRect { min_x:2,min_y:3,max_x:3,max_y:4 },10,10));
        app.begin_transform();
        app.session.floating.as_mut().unwrap().center -= Vec2::new(20.0,20.0);
        app.commit_transform();
        assert_eq!(app.project().layers[0].pixels.read(0,-18,-17),[100,20,30,255]);
        assert_eq!(app.project().layers[0].pixels.read(0,2,3),[0;4]);
        app.undo();
        assert_eq!(app.project().layers[0].pixels.read(0,2,3),[100,20,30,255]);
        assert_eq!(app.project().layers[0].pixels.read(0,-18,-17),[0;4]);
        app.redo();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("offcanvas.mgimg");
        app.workspace.save(&path).unwrap();
        let mut loaded = Workspace::open(&path).unwrap();
        loaded.ensure_document_loaded(loaded.active_document).unwrap();
        assert_eq!(loaded.active().layers[0].pixels,app.project().layers[0].pixels);
        app.project_mut().resize_canvas(1,1).unwrap();
        app.project_mut().resize_canvas(100,100).unwrap();
        assert_eq!(app.project().layers[0].pixels.read(0,-18,-17),[100,20,30,255]);
    }

    #[test]
    fn brush_grows_moved_layer_and_undo_removes_new_tile() {
        let mut app = document();
        app.project_mut().layers[0].pixels.offset = (-100,0);
        app.stroke_capture = Some(EditCapture::new(0,10,10));
        app.session.brush_size = 1.0;
        app.session.brush_opacity = 1.0;
        app.session.brush_color = [1.0,0.0,0.0,1.0];
        app.stroke_marks = vec![0;100];
        app.stroke_id = 1;
        app.apply_brush_segment((5.5,5.5),(5.5,5.5),10,10,false);
        let pixels = &app.project().layers[0].pixels;
        assert_eq!(pixels.tiles.len(),2);
        assert_eq!(pixels.read(0,5,5),[255,0,0,255]);
        assert_eq!(pixels.read(0,-98,3),[100,20,30,255]);
        let edit = app.stroke_capture.take().unwrap().finish(&mut app.project_mut().layers[0].pixels).unwrap();
        app.history.push(edit);
        app.undo();
        assert_eq!(app.project().layers[0].pixels.tiles.len(),1);
        app.redo();
        assert_eq!(app.project().layers[0].pixels.read(0,5,5),[255,0,0,255]);
    }

    #[test]
    fn whole_layer_scale_preserves_sparse_gaps_and_offcanvas_pixels() {
        let mut app = document();
        app.project_mut().layers[0].pixels.write(0,100_000,3,[10,200,30,255]);
        app.begin_transform();
        let floating = app.session.floating.as_mut().unwrap();
        assert!(floating.pixels.is_empty());
        floating.scale = Vec2::splat(2.0);
        app.commit_transform();
        let raster = &app.project().layers[0].pixels;
        assert!(raster.tiles.len() <= 8);
        assert!(raster.bounds().unwrap().0 < 0);
        assert!(raster.bounds().unwrap().2 > 100_000);
        app.undo();
        assert_eq!(app.project().layers[0].pixels.read(0,100_000,3),[10,200,30,255]);
    }
