    use super::*;

    fn document() -> EditorApp {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[100,150,200,255].repeat(100*80),100,80).unwrap();
        app.sync_layer_structure();
        app.session.canvas_rect = Rect::from_min_size(Vec2::ZERO,Vec2::splat(500.0));
        app.set_tool(Tool::Transform);
        app
    }

    fn pointer(app: &mut EditorApp, point: Vec2, pressed: bool, down: bool) {
        let view = app.canvas_view().unwrap();
        app.input = FrameInput::default();
        app.input.mouse_pos = view.image_to_screen(point);
        app.input.mouse_pressed = pressed;
        app.input.mouse_down = down;
        app.input.mouse_released = !down;
        app.handle_canvas_edit(view);
    }

    #[test]
    fn transform_flip_buttons_preview_cancel_commit_and_undo() {
        let bytes = [10,20,30,255, 40,50,60,255, 70,80,90,255, 100,110,120,255];
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&bytes,2,2).unwrap();
        app.sync_layer_structure();
        app.set_tool(Tool::Transform);
        let original = app.project().layers[0].pixels.clone();
        let corners = app.session.floating.as_ref().unwrap().corners();
        app.flip_transform(true);
        app.flip_transform(false);
        assert_eq!(app.project().layers[0].pixels,original);
        assert_eq!(app.session.floating.as_ref().unwrap().corners(),corners);
        app.cancel_transform();
        assert_eq!(app.project().layers[0].pixels,original);
        app.flip_transform(true); // Also starts a new floating transform after Cancel.
        app.flip_transform(true);
        assert!(!app.session.floating.as_ref().unwrap().flip_x);
        app.flip_transform(true);
        app.flip_transform(false);
        app.commit_transform();
        let expected: Vec<u8> = bytes.chunks_exact(4).rev().flatten().copied().collect();
        assert_eq!(app.project().layers[0].pixels.to_rgba(2,2),expected);
        app.undo();
        assert_eq!(app.project().layers[0].pixels,original);
        app.redo();
        assert_eq!(app.project().layers[0].pixels.to_rgba(2,2),expected);
    }

    #[test]
    fn transform_aspect_checkbox_controls_pointer_scaling() {
        for locked in [true,false] {
            let mut app = document();
            assert!(app.session.transform_keep_aspect_ratio);
            app.session.transform_keep_aspect_ratio = locked;
            let floating = app.session.floating.as_ref().unwrap();
            let pivot = floating.pivot;
            let corner = floating.corners()[2];
            pointer(&mut app,corner,true,true);
            pointer(&mut app,pivot+(corner-pivot)*Vec2::new(2.0,1.0),false,true);
            let scale = app.session.floating.as_ref().unwrap().scale;
            if locked { assert!((scale.x-scale.y).abs()<1.0e-5); }
            else { assert!((scale-Vec2::new(2.0,1.0)).length()<1.0e-5); }
        }
    }

    #[test]
    fn outside_drag_rotates_without_committing_and_escape_restores_pixels() {
        let mut app = document();
        let original = app.project().layers[0].pixels.clone();
        pointer(&mut app,Vec2::new(-20.0,40.0),true,true);
        assert_eq!(app.session.transform_drag,Some(TransformHit::Rotate));
        pointer(&mut app,Vec2::new(50.0,-30.0),false,true);
        assert!((app.session.floating.as_ref().unwrap().rotation-std::f32::consts::FRAC_PI_2).abs()<1.0e-5);
        let view = app.canvas_view().unwrap();
        assert_eq!(app.transform_cursor(view),Some(TransformCursor::Rotate));
        pointer(&mut app,Vec2::new(50.0,-30.0),false,false);
        assert!(app.session.transform_drag.is_none() && app.session.floating.is_some());
        app.input.key_escape = true;
        app.handle_canvas_edit(view);
        assert!(app.session.floating.is_none());
        assert_eq!(app.project().layers[0].pixels,original);
    }

    #[test]
    fn pivot_drag_keeps_image_stationary_and_body_drag_carries_pivot() {
        let mut app = document();
        let before = app.session.floating.as_ref().unwrap().corners();
        pointer(&mut app,Vec2::new(50.0,40.0),true,true);
        assert_eq!(app.session.transform_drag,Some(TransformHit::Pivot));
        pointer(&mut app,Vec2::new(35.0,55.0),false,true);
        assert_eq!(app.session.floating.as_ref().unwrap().corners(),before);
        pointer(&mut app,Vec2::new(35.0,55.0),false,false);
        pointer(&mut app,Vec2::new(75.0,20.0),true,true);
        assert_eq!(app.session.transform_drag,Some(TransformHit::Body));
        pointer(&mut app,Vec2::new(85.0,25.0),false,true);
        assert_eq!(app.session.floating.as_ref().unwrap().pivot,Vec2::new(45.0,60.0));
        assert_eq!(app.session.floating.as_ref().unwrap().center,Vec2::new(60.0,45.0));
        assert_eq!(app.transform_cursor(app.canvas_view().unwrap()),Some(TransformCursor::Move));
    }

    #[test]
    fn resize_cursors_follow_canvas_rotation_with_a_constant_screen_hit_radius() {
        let mut app = document();
        let corner = app.session.floating.as_ref().unwrap().corners()[0];
        for zoom in [0.25,1.0,32.0] {
            app.session.zoom = zoom;
            let view = app.canvas_view().unwrap();
            app.input.mouse_pos = view.image_to_screen(corner)+Vec2::X*7.0;
            assert_eq!(app.transform_cursor(view),Some(TransformCursor::ResizeNwse));
            app.input.mouse_pos = view.image_to_screen(corner)+Vec2::X*12.0;
            assert_eq!(app.transform_cursor(view),Some(TransformCursor::Move));
        }
        app.session.rotation = 90.0;
        let view = app.canvas_view().unwrap();
        app.input.mouse_pos = view.image_to_screen(corner);
        assert_eq!(app.transform_cursor(view),Some(TransformCursor::ResizeNesw));
        app.session.transform_drag = Some(TransformHit::Scale(0));
        app.input.mouse_pos = view.image_to_screen(Vec2::splat(-100.0));
        assert_eq!(app.transform_cursor(view),Some(TransformCursor::ResizeNesw));
    }
