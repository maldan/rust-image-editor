    use super::*;
    use crate::raster::SparsePixels;

    #[test]
    fn invert_selection_complements_mask_and_does_nothing_without_selection() {
        let mut app = EditorApp::default();
        app.session.selection_mask_dirty = false;
        let status = app.status.clone();
        app.invert_selection();
        assert!(app.session.selection.is_none());
        assert!(!app.session.selection_mask_dirty);
        assert_eq!(app.status, status);
        app.project_mut().import_rgba(&[1, 2, 3, 255].repeat(6), 3, 2).unwrap();
        let pixels = app.project().layers[0].pixels.clone();
        app.session.selection = Some(Selection { width: 3, height: 2, mask: vec![1, 0, 1, 0, 0, 1] });
        app.invert_selection();
        assert_eq!(app.session.selection.as_ref().unwrap().mask, [0, 1, 0, 1, 1, 0]);
        assert!(app.session.selection_mask_dirty);
        app.invert_selection();
        assert_eq!(app.session.selection.as_ref().unwrap().mask, [1, 0, 1, 0, 0, 1]);
        app.session.selection = Some(Selection::all(3, 2));
        app.invert_selection();
        assert!(app.session.selection.as_ref().unwrap().is_empty());
        app.invert_selection();
        assert_eq!(app.session.selection.as_ref().unwrap().mask, [1; 6]);
        assert_eq!(app.project().layers[0].pixels, pixels);
    }

    #[test]
    fn layer_clipboard_moves_between_documents_without_clipping_and_undoes() {
        use crate::raster::Raster;
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[10, 20, 30, 255].repeat(16), 4, 4).unwrap();
        let mut pixels = SparsePixels::default();
        pixels.offset = (-10, 20);
        pixels.write(0, -3, -5, [1, 2, 3, 128]);
        pixels.write(0, 1000, 800, [4, 5, 6, 255]);
        let source = &mut app.project_mut().layers[0];
        source.pixels = pixels.clone();
        source.name = "Reference".into();
        source.visible = false;
        source.opacity = 0.25;
        let text = app.copy_layer_text(0).unwrap();
        let index = app.workspace.create_document("Target");
        app.switch_document(index);
        app.project_mut().import_rgba(&[80, 90, 100, 255], 1, 1).unwrap();
        app.refresh_image().unwrap();
        let destination = app.project().layers[0].pixels.clone();
        app.insert_copied_layer(crate::layer_clipboard::decode(&text).unwrap().unwrap());
        assert_eq!(app.image_size, Some((1, 1)));
        assert_eq!(app.project().active_layer, 1);
        let pasted = &app.project().layers[1];
        assert_eq!(pasted.name, "Reference");
        assert!(!pasted.visible);
        assert_eq!(pasted.opacity, 0.25);
        assert_eq!(pasted.pixels, pixels);
        assert_eq!(app.project().layers[0].pixels, destination);
        app.undo();
        assert_eq!(app.project().layers.len(), 1);
        assert_eq!(app.project().active_layer, 0);
        app.redo();
        assert_eq!(app.project().layers.len(), 2);
        assert_eq!(app.project().layers[1].pixels, pixels);
        app.switch_document(0);
        assert_eq!(app.project().layers.len(), 1);
        assert_eq!(app.project().layers[0].pixels, pixels);
    }

    #[test]
    fn pasting_layer_into_empty_document_restores_source_canvas_and_undoes() {
        let mut source = EditorApp::default();
        source.project_mut().import_rgba(&[1, 2, 3, 255].repeat(8), 4, 2).unwrap();
        let text = source.copy_layer_text(0).unwrap();
        let mut target = EditorApp::default();
        target.insert_copied_layer(crate::layer_clipboard::decode(&text).unwrap().unwrap());
        assert_eq!(target.image_size, Some((4, 2)));
        assert_eq!(target.project().layers.len(), 1);
        target.undo();
        assert!(target.project().layers.is_empty());
        assert!(target.image_size.is_none());
        assert_eq!((target.project().width, target.project().height), (0, 0));
        target.redo();
        assert_eq!(target.image_size, Some((4, 2)));
        assert_eq!(target.project().layers[0].pixels, source.project().layers[0].pixels);
    }

    #[test]
    fn selection_cut_and_copy_paste_onto_a_new_layer_and_start_transform() {
        use crate::raster::Raster;
        let mut pixels = vec![0u8; 4 * 2 * 4];
        for i in 0..8u8 {
            pixels[i as usize * 4..i as usize * 4 + 4].copy_from_slice(&[i, 1, 2, 255]);
        }
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&pixels, 4, 2).unwrap();
        app.refresh_image().unwrap();
        let original = app.project().layers[0].pixels.clone();
        app.session.selection = Some(Selection::from_rect(
            PixelRect { min_x: 1, min_y: 0, max_x: 3, max_y: 2 }, 4, 2));
        let text = app.selection_clipboard_text().unwrap();
        let copied = crate::layer_clipboard::decode(&text).unwrap().unwrap();
        assert!(copied.fragment);
        assert_eq!(copied.layer.pixels.read(4, 1, 0), [1, 1, 2, 255]);
        assert_eq!(copied.layer.pixels.read(4, 0, 0), [0; 4]);
        assert_eq!(app.project().layers[0].pixels, original);

        app.erase_active_selection().unwrap();
        assert_eq!(app.project().layers[0].pixels.read(4, 1, 0), [0; 4]);
        assert_eq!(app.project().layers[0].pixels.read(4, 0, 0), [0, 1, 2, 255]);
        app.undo();
        assert_eq!(app.project().layers[0].pixels, original);
        app.redo();
        assert_eq!(app.project().layers[0].pixels.read(4, 2, 1), [0; 4]);

        app.insert_copied_layer(copied);
        assert_eq!(app.project().layers.len(), 2);
        assert_eq!(app.project().active_layer, 1);
        assert_eq!(app.session.tool, Tool::Transform);
        let floating = app.session.floating.as_mut().unwrap();
        assert!(floating.whole_layer);
        assert_eq!(floating.layer, 1);
        floating.center.x += 1.0;
        app.commit_transform();
        assert!(app.session.floating.is_none());
        assert_eq!(app.project().layers[1].pixels.read(4, 2, 0), [1, 1, 2, 255]);
        assert_eq!(app.project().layers[1].pixels.read(4, 1, 0), [0; 4]);
        assert_eq!(app.project().layers[0].pixels.read(4, 1, 0), [0; 4]);
        assert_eq!(app.project().layers[0].pixels.read(4, 0, 0), [0, 1, 2, 255]);
    }

    #[test]
    fn invert_layer_preserves_alpha_other_layers_and_supports_undo_redo() {
        let mut app = EditorApp::default();
        app.invert_layer();
        assert!(!app.dirty);
        app.project_mut().import_rgba(&[10, 20, 30, 255], 1, 1).unwrap();
        app.project_mut().add_layer();
        app.invert_layer();
        assert!(!app.dirty);
        let bytes = [0, 127, 255, 255, 10, 20, 30, 128, 40, 50, 60, 0];
        let mut pixels = SparsePixels::from_rgba(&bytes, 3, 1);
        pixels.offset = (-2, 1000);
        app.project_mut().layers[1].pixels = pixels.clone();
        app.project_mut().layers[1].visible = false;
        app.session.selection = Some(Selection::all(1, 1));
        app.invert_layer();
        let inverted = app.project().layers[1].pixels.clone();
        assert_eq!(inverted.region(-2, 1000, 3, 1),
            [255, 128, 0, 255, 245, 235, 225, 128, 40, 50, 60, 0]);
        assert_eq!(inverted.offset, pixels.offset);
        assert_eq!(inverted.tiles.len(), pixels.tiles.len());
        assert_eq!(app.project().layers[0].pixels.to_rgba(1, 1), [10, 20, 30, 255]);
        assert!(app.dirty);
        assert!(app.layers_reload);
        app.undo();
        assert_eq!(app.project().layers[1].pixels, pixels);
        app.redo();
        assert_eq!(app.project().layers[1].pixels, inverted);
        app.invert_layer();
        assert_eq!(app.project().layers[1].pixels, pixels);
    }

    #[test]
    fn fix_alpha_cleans_only_active_layer_and_supports_undo_redo() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[10, 20, 30, 4], 1, 1).unwrap();
        app.project_mut().add_layer();
        let bytes = [10, 20, 30, 0, 40, 50, 60, 1, 70, 80, 90, 4, 100, 110, 120, 5, 130, 140, 150, 255];
        let mut pixels = SparsePixels::from_rgba(&bytes, 5, 1);
        pixels.offset = (-2, 1000);
        app.project_mut().layers[1].pixels = pixels.clone();
        app.project_mut().layers[1].visible = false;
        app.session.selection = Some(Selection::all(1, 1));
        app.fix_alpha();
        let fixed = app.project().layers[1].pixels.clone();
        let mut expected = bytes;
        expected[4..12].fill(0);
        assert_eq!(fixed.region(-2, 1000, 5, 1), expected);
        assert_eq!(fixed.offset, pixels.offset);
        assert_eq!(app.project().layers[0].pixels.to_rgba(1, 1), [10, 20, 30, 4]);
        assert!(app.dirty);
        assert!(app.ai.job.is_none());
        app.fix_alpha(); // No change must not create a second Undo step.
        app.undo();
        assert_eq!(app.project().layers[1].pixels, pixels);
        app.redo();
        assert_eq!(app.project().layers[1].pixels, fixed);
    }

    #[test]
    fn fix_alpha_prunes_empty_tiles_and_leaves_clean_document_unchanged() {
        let mut app = EditorApp::default();
        app.fix_alpha();
        app.project_mut().import_rgba(&[10, 20, 30, 5], 1, 1).unwrap();
        app.dirty = false;
        app.fix_alpha();
        assert!(!app.dirty);
        app.project_mut().layers[0].pixels = SparsePixels::from_rgba(&[10, 20, 30, 4], 1, 1);
        let original = app.project().layers[0].pixels.clone();
        app.fix_alpha();
        assert!(app.project().layers[0].pixels.tiles.is_empty());
        app.undo();
        assert_eq!(app.project().layers[0].pixels, original);
    }

    #[test]
    fn layer_source_crops_alpha_with_padding_in_every_image_workflow() {
        use crate::raster::Raster;
        for mode in [AiMode::Edit, AiMode::Redraw, AiMode::Inpaint, AiMode::Lineart, AiMode::DrawCn, AiMode::Upscale] {
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&vec![0; 1000 * 1000 * 4], 1000, 1000).unwrap();
            let mut pixels = SparsePixels::from_rgba(&[20, 40, 60, 128].repeat(200 * 200), 200, 200);
            pixels.offset = (300, 400);
            pixels.write(0, 5, 5, [255, 0, 0, 0]);
            app.project_mut().layers[0].pixels = pixels;
            app.project_mut().layers[0].visible = false;
            app.ai.mode = mode;
            app.ai.layer_padding = 12;
            app.ai.scale = 0.5;
            let (image, region) = app.ai_input().unwrap();
            assert_eq!(image.dimensions(), (224, 224));
            assert_eq!((region.min_x, region.min_y), (288, 388));
            assert_eq!(image.get_pixel(0, 0).0, [0; 4]);
            assert_eq!(image.get_pixel(12, 12).0, [20, 40, 60, 128]);
            assert_eq!(image.get_pixel(211, 211).0, [20, 40, 60, 128]);
            assert_eq!(image.get_pixel(212, 212).0, [0; 4]);
            app.ai.layer_padding = 0;
            assert_eq!(app.ai_input().unwrap().0.dimensions(), (200, 200));
            app.ai.layer_canvas_size = true;
            app.ai.layer_padding = -1; // Canvas mode ignores the padding setting.
            let (image, region) = app.ai_input().unwrap();
            assert_eq!(image.dimensions(), (1000, 1000));
            assert_eq!((region.min_x, region.min_y), (0, 0));
            assert_eq!(image.get_pixel(300, 400).0, [20, 40, 60, 128]);
        }
    }

    #[test]
    fn cropped_layer_mask_and_accepted_result_keep_negative_origin_and_padding() {
        for fit in [true, false] {
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&[0; 4].repeat(16), 4, 4).unwrap();
            let mut pixels = SparsePixels::from_rgba(&[20, 40, 60, 128].repeat(6), 2, 3);
            pixels.offset = (-1, -2);
            app.project_mut().layers[0].pixels = pixels;
            app.ai.mode = AiMode::Inpaint;
            app.ai.layer_padding = 2;
            app.session.selection = Some(Selection::all(4, 4));
            let (image, region) = app.ai_input().unwrap();
            assert_eq!(image.dimensions(), (6, 7));
            assert_eq!((region.min_x, region.min_y), (-3, -4));
            let mask = app.ai_mask().unwrap();
            assert_eq!(mask.dimensions(), image.dimensions());
            for (x, y, pixel) in mask.enumerate_pixels() {
                assert_eq!(pixel.0, if x >= 3 && y >= 4 { [255; 4] } else { [0, 0, 0, 255] });
            }
            app.ai.input_region = Some(region);
            app.ai.document_id = app.workspace.active_doc().id.clone();
            app.ai.fit_image = fit;
            app.ai.result = Some(image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255])));
            app.accept_ai();
            assert_eq!(app.project().layers.len(), 2);
            let inserted = &app.project().layers[1].pixels;
            assert_eq!(inserted.offset, (-3, -4));
            assert_eq!(inserted.bounds(), Some(if fit { (-3, -4, 3, 3) } else { (-3, -4, -1, -2) }));
        }
    }

    #[test]
    fn empty_layers_and_invalid_padding_are_rejected_before_packing() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[0; 4].repeat(16), 4, 4).unwrap();
        assert!(app.ai_input().is_err());
        app.ai.layer_canvas_size = true;
        assert_eq!(app.ai_input().unwrap().0.dimensions(), (4, 4));
        app.project_mut().layers[0].pixels = SparsePixels::from_rgba(&[1, 2, 3, 1], 1, 1);
        app.ai.layer_canvas_size = false;
        for padding in [-1, 100000, i32::MAX] {
            app.ai.layer_padding = padding;
            assert!(app.ai_input().is_err());
        }
    }

    #[test]
    fn asset_preview_is_read_only_and_switching_back_preserves_document_state() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[10, 20, 30, 128].repeat(16), 4, 4).unwrap();
        app.refresh_image().unwrap();
        app.session.selection = Some(Selection::all(4, 4));
        let asset = app.workspace.import_asset_rgba("preview".into(), &[255, 0, 0, 255].repeat(6), 3, 2).unwrap();
        app.session.pan = Vec2::new(12.0, 34.0);
        app.session.zoom = 2.0;
        app.session.rotation = 37.0;
        app.show_asset_preview(asset);
        assert_eq!(app.display_size(), Some((3, 2)));
        let view = app.canvas_view().unwrap();
        app.input.mouse_pressed = true;
        app.input.mouse_down = true;
        app.input.mouse_pos = view.image_to_screen(Vec2::splat(0.5));
        app.handle_canvas_edit(view);
        assert_eq!(app.project().layers[0].pixels.to_rgba(app.project().width, app.project().height), [10, 20, 30, 128].repeat(16));
        assert_eq!(app.workspace.assets[asset].pixels, [255, 0, 0, 255].repeat(6));
        app.switch_document(0);
        assert_eq!(app.display_size(), Some((4, 4)));
        assert_eq!((app.session.pan, app.session.zoom, app.session.rotation), (Vec2::new(12.0, 34.0), 2.0, 37.0));
        assert_eq!(app.session.selection.as_ref().unwrap().mask, vec![1; 16]);
        assert!(app.layers_reload && app.session.selection_mask_dirty);
    }

    #[test]
    fn inpaint_softness_blurs_mask_without_changing_selection_or_alpha() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[1, 2, 3, 255].repeat(9 * 9), 9, 9).unwrap();
        app.session.selection = Some(Selection::from_rect(PixelRect { min_x: 3, min_y: 3, max_x: 6, max_y: 6 }, 9, 9));
        let original = app.session.selection.as_ref().unwrap().mask.clone();
        let sharp = app.ai_mask().unwrap();
        assert_eq!(sharp.get_pixel(2, 4).0, [0, 0, 0, 255]);
        app.ai.mask_softness = 1.5;
        let soft = app.ai_mask().unwrap();
        assert!(soft.get_pixel(2, 4)[0] > 0);
        assert!(soft.get_pixel(4, 4)[0] < 255);
        assert!(soft.pixels().all(|p| p[3] == 255 && p[0] == p[1] && p[1] == p[2]));
        assert_eq!(app.session.selection.as_ref().unwrap().mask, original);
    }

    #[test]
    fn selection_tools_handle_rotated_view_modifiers_and_release_outside_canvas() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[0, 0, 0, 255].repeat(100), 10, 10).unwrap();
        app.refresh_image().unwrap();
        app.session.canvas_rect = Rect::from_min_size(Vec2::ZERO, Vec2::splat(200.0));
        app.session.rotation = 45.0;
        app.session.zoom = 4.0;
        app.set_tool(Tool::Lasso);
        let view = app.canvas_view().unwrap();
        for (i, point) in [Vec2::new(1.0, 1.0), Vec2::new(5.0, 1.0), Vec2::new(1.0, 5.0)].into_iter().enumerate() {
            app.input = FrameInput::default();
            app.input.mouse_pressed = i == 0;
            app.input.mouse_down = true;
            app.input.mouse_pos = view.image_to_screen(point);
            app.handle_canvas_edit(view);
        }
        app.input.mouse_down = false;
        app.input.mouse_released = true;
        app.handle_canvas_edit(view);
        assert!(app.session.selection.as_ref().unwrap().contains(1, 1));
        assert!(!app.session.selection.as_ref().unwrap().contains(4, 4));
        app.set_tool(Tool::SelectionBrush);
        app.session.brush_size = 1.0;
        app.input = FrameInput::default();
        app.input.mouse_pressed = true;
        app.input.mouse_down = true;
        app.input.mouse_pos = view.image_to_screen(Vec2::new(8.5, 8.5));
        app.handle_canvas_edit(view);
        assert!(app.session.selection.as_ref().unwrap().contains(1, 1));
        assert!(app.session.selection.as_ref().unwrap().contains(8, 8));
        app.session.selection_brush_erase = true;
        app.handle_canvas_edit(view);
        assert!(!app.session.selection.as_ref().unwrap().contains(8, 8));
        app.input = FrameInput::default();
        app.input.mouse_released = true;
        app.input.mouse_pos = Vec2::splat(300.0);
        app.handle_canvas_edit(view);
        assert!(!app.brush_last_active());
        assert!(app.project().layers[0].pixels.to_rgba(app.project().width, app.project().height).chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
    }

    #[test]
    fn inpaint_requires_nonempty_matching_selection_and_full_image_source() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[1, 2, 3, 255].repeat(8), 4, 2).unwrap();
        app.ai.mode = AiMode::Inpaint;
        assert!(app.ai_mask().is_err());
        app.session.selection = Some(Selection::from_rect(PixelRect { min_x: 0, min_y: 0, max_x: 0, max_y: 0 }, 4, 2));
        assert!(app.ai_mask().is_err());
        app.session.selection = Some(Selection::all(5, 2));
        assert!(app.ai_mask().is_err());
        app.session.selection = Some(Selection::all(4, 2));
        assert!(app.ai_mask().unwrap().pixels().all(|p| p.0 == [255, 255, 255, 255]));
        app.ai.source = AiSource::Selection;
        assert!(app.ai_mask().is_err());
    }

    #[test]
    fn selection_source_crops_masks_and_preserves_alpha_for_edit_and_redraw() {
        for mode in [AiMode::Edit, AiMode::Redraw] {
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&[20, 40, 60, 128].repeat(6 * 5), 6, 5).unwrap();
            app.project_mut().add_layer();
            app.project_mut().layers[1].pixels = SparsePixels::from_rgba(&[255, 0, 0, 255].repeat(6 * 5), 6, 5);
            app.project_mut().layers[1].visible = false;
            app.ai.mode = mode;
            app.ai.source = AiSource::Selection;
            assert!(app.ai_input().is_err());
            app.session.selection = Some(Selection::from_rect(PixelRect { min_x: 0, min_y: 0, max_x: 0, max_y: 0 }, 6, 5));
            assert!(app.ai_input().is_err());
            let bounds = PixelRect { min_x: 1, min_y: 1, max_x: 5, max_y: 4 };
            let mut selection = Selection::from_rect(bounds, 6, 5);
            selection.subtract(&Selection::from_rect(PixelRect { min_x: 2, min_y: 2, max_x: 4, max_y: 3 }, 6, 5));
            app.session.selection = Some(selection);
            app.ai.scale = 2.0;
            let (image, region) = app.ai_input().unwrap();
            assert_eq!(region, bounds.into());
            assert_eq!(image.dimensions(), (4, 3));
            for y in 0..3 {
                for x in 0..4 {
                    assert_eq!(image.get_pixel(x, y).0, if y == 1 && (1..3).contains(&x) { [0, 0, 0, 0] } else { [20, 40, 60, 128] });
                }
            }
            assert_eq!(crate::comfyui::scaled_size(image.width(), image.height(), app.ai.scale).unwrap(), (8, 6));
            // A plain rectangle retains the complete cropped image.
            app.session.selection = Some(Selection::from_rect(bounds, 6, 5));
            let (image, _) = app.ai_input().unwrap();
            assert!(image.pixels().all(|p| p.0 == [20, 40, 60, 128]));
            app.session.selection = Some(Selection::all(7, 5));
            assert!(app.ai_input().is_err());
        }
    }

    #[test]
    fn input_workflows_upload_selected_source_and_accept_as_new_layer() {
        use std::{io::{BufRead, BufReader, Read, Write}, net::TcpListener, time::{Duration, Instant}};
        for (mode, source) in [(AiMode::AnimeRedraw, AiSource::Layer),
            (AiMode::AnimeRedraw, AiSource::Document), (AiMode::AnimeRedraw, AiSource::Selection),
            (AiMode::Redraw, AiSource::Layer), (AiMode::Redraw, AiSource::Document),
            (AiMode::Redraw, AiSource::Selection), (AiMode::Inpaint, AiSource::Layer), (AiMode::Inpaint, AiSource::Document),
            (AiMode::Lineart, AiSource::Layer), (AiMode::Lineart, AiSource::Document), (AiMode::Lineart, AiSource::Selection),
            (AiMode::DrawCn, AiSource::Layer), (AiMode::DrawCn, AiSource::Document), (AiMode::DrawCn, AiSource::Selection),
            (AiMode::Upscale, AiSource::Layer), (AiMode::Upscale, AiSource::Document), (AiMode::Upscale, AiSource::Selection)] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let host = format!("http://{}", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let mut routes = vec![("/upload/image", false)];
                if mode == AiMode::Inpaint { routes.push(("/upload/image", true)); }
                routes.extend([("/prompt", false), ("/history/job", false), ("/view?", false)]);
                for (route, is_mask) in routes {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut reader = BufReader::new(&mut stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    assert!(line.contains(route), "{line}");
                    let mut length = 0;
                    loop {
                        line.clear();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" { break; }
                        if let Some((key, value)) = line.split_once(':') {
                            if key.eq_ignore_ascii_case("content-length") { length = value.trim().parse::<usize>().unwrap(); }
                        }
                    }
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    let response = match route {
                        "/upload/image" => {
                            let multipart = String::from_utf8_lossy(&body);
                            assert!(multipart.contains(if is_mask { "filename=\"mega-image-editor-mask.png\"" } else { "filename=\"mega-image-editor-input.png\"" }));
                            assert!(multipart.contains("name=\"overwrite\"\r\n\r\ntrue\r\n"));
                            let start = body.windows(8).position(|w| w == b"\x89PNG\r\n\x1a\n").unwrap();
                            let image = image::load_from_memory_with_format(&body[start..], image::ImageFormat::Png).unwrap().to_rgba8();
                            assert_eq!(image.dimensions(), if source == AiSource::Selection { (48, 40) } else { (64, 64) });
                            if is_mask {
                                assert_eq!(image.get_pixel(0, 0).0, [0, 0, 0, 255]);
                                assert_eq!(image.get_pixel(10, 14).0, [255, 255, 255, 255]);
                                assert_eq!(image.get_pixel(32, 30).0, [0, 0, 0, 255]);
                                assert!(image.pixels().all(|p| p.0 == [0, 0, 0, 255] || p.0 == [255, 255, 255, 255]));
                            } else {
                                assert_eq!(image.get_pixel(0, 0).0, if source == AiSource::Layer { [200, 100, 50, 255] } else { [10, 20, 30, 255] });
                            }
                            if source == AiSource::Selection { assert_eq!(image.get_pixel(24, 20).0, [0, 0, 0, 0]); }
                            if is_mask { br#"{"name":"mask.png","subfolder":"test"}"#.to_vec() }
                            else { br#"{"name":"source.png","subfolder":"test"}"#.to_vec() }
                        }
                        "/prompt" => {
                            let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                            let graph = request["prompt"].as_object().unwrap();
                            let title = if mode == AiMode::AnimeRedraw { "LoadImage" } else if matches!(mode, AiMode::DrawCn | AiMode::Upscale) { "ReferenceImage" } else { "Image" };
                            let image = graph.values().find(|n| n["_meta"]["title"] == title).unwrap();
                            assert_eq!(image["inputs"]["image"], "test/source.png");
                            if mode == AiMode::Lineart {
                                let lineart = graph.values().find(|n| n["class_type"] == "AnimeLineArtPreprocessor").unwrap();
                                assert_eq!(lineart["inputs"]["resolution"], 1024);
                                assert_eq!(graph.len(), 3);
                            } else if matches!(mode, AiMode::DrawCn | AiMode::Upscale) {
                                let cn = graph.values().find(|n| n["_meta"]["title"] == "ControlNet").unwrap();
                                assert_eq!(cn["inputs"]["strength"], 0.5);
                                assert_eq!(cn["inputs"]["start_percent"], 0.25);
                                assert_eq!(cn["inputs"]["end_percent"], 0.75);
                                if mode == AiMode::Upscale {
                                    let upscale = graph.values().find(|n| n["class_type"] == "ImageScaleBy").unwrap();
                                    assert_eq!(upscale["inputs"]["scale_by"], 2.5);
                                    let sampler = graph.values().find(|n| n["_meta"]["title"] == "Sampler").unwrap();
                                    assert_eq!(sampler["inputs"]["denoise"], 0.5);
                                    assert!(graph.values().any(|n| n["class_type"] == "DepthAnythingV2Preprocessor"));
                                } else {
                                    let latent = graph.values().find(|n| n["class_type"] == "EmptyLatentImage").unwrap();
                                    assert!(latent["inputs"]["width"].is_array());
                                    assert!(latent["inputs"]["height"].is_array());
                                }
                            } else {
                                let sampler = graph.values().find(|n| n["_meta"]["title"] == "Sampler").unwrap();
                                assert_eq!(sampler["inputs"]["denoise"], 0.5);
                                assert!(graph.values().any(|n| n["class_type"] == "VAEEncode"));
                            }
                            if mode == AiMode::Inpaint {
                                let mask = graph.values().find(|n| n["_meta"]["title"] == "Mask").unwrap();
                                assert_eq!(mask["inputs"]["image"], "test/mask.png");
                                assert_eq!(mask["inputs"]["channel"], "red");
                                assert!(graph.values().any(|n| n["class_type"] == "SetLatentNoiseMask"));
                            }
                            br#"{"prompt_id":"job"}"#.to_vec()
                        }
                        "/history/job" => {
                            let save = if mode == AiMode::Lineart { "3" } else if mode == AiMode::AnimeRedraw { "62" } else { "12" };
                            serde_json::to_vec(&serde_json::json!({"job": {"outputs": {save: {"images": [{"filename": "result.png"}]}}, "status": {"completed": true}}})).unwrap()
                        }
                        _ => crate::project::encode_png_rgba(&[1, 2, 3, 255].repeat(64 * 64), 64, 64).unwrap(),
                    };
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).unwrap();
                    stream.write_all(&response).unwrap();
                }
            });
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&[10, 20, 30, 255].repeat(128 * 128), 128, 128).unwrap();
            app.project_mut().add_layer();
            app.project_mut().layers[1].pixels = SparsePixels::from_rgba(&[200, 100, 50, 255].repeat(128 * 128), 128, 128);
            // Active layer source ignores visibility; composite source honors it.
            app.project_mut().layers[1].visible = false;
            app.refresh_image().unwrap();
            app.ai.open = true;
            app.ai.mode = mode;
            app.ai.source = source;
            if source == AiSource::Selection || mode == AiMode::Inpaint {
                let mut selection = Selection::from_rect(PixelRect { min_x: 16, min_y: 24, max_x: 112, max_y: 104 }, 128, 128);
                selection.subtract(&Selection::from_rect(PixelRect { min_x: 48, min_y: 48, max_x: 80, max_y: 72 }, 128, 128));
                app.ai.selection_bounds = selection.bounds();
                app.session.selection = Some(selection);
            }
            app.ai.scale = 0.5;
            app.ai.denoise = 0.5;
            app.ai.lineart_resolution = 1024;
            app.ai.control_net = crate::comfyui::ControlNetSettings {
                strength: 0.5, start_percent: 0.25, end_percent: 0.75,
            };
            app.ai.upscale = crate::comfyui::UpscaleSettings {
                scale_by: 2.5, denoise: 0.5, control: app.ai.control_net,
            };
            app.ai.config = Some(crate::comfyui::HostConfig { host, ws: String::new(), login: String::new(), password: String::new() });
            app.start_ai();
            let started = Instant::now();
            let mut ui = Ui::new();
            while app.ai.job.is_some() {
                assert!(started.elapsed() < Duration::from_secs(10), "Redraw timed out");
                app.poll_ai(&mut ui);
                std::thread::sleep(Duration::from_millis(10));
            }
            server.join().unwrap();
            assert!(app.ai.result.is_some(), "{}", app.ai.message);
            assert_eq!(app.project().layers.len(), 2);
            assert_eq!(&app.project().layers[1].pixels.to_rgba(app.project().width, app.project().height)[..4], &[200, 100, 50, 255]);
            assert_eq!(app.ai.input_region.unwrap().max_x, if source == AiSource::Selection { 112 } else { 128 });
            // Accept must retain the submitted region even if selection changes afterwards.
            app.session.selection = Some(Selection::all(128, 128));
            // Render the Redraw modal with its source, denoise and cached selectors.
            EditorApp::init(&mut ui);
            let viewport = Vec2::new(1280.0, 800.0);
            ui.begin_frame(mega_ui::UiInput { viewport, ..Default::default() });
            app.build(&mut ui, viewport, DrawStats::default());
            ui.end_frame();
            app.accept_ai();
            assert_eq!(app.image_size, Some((128, 128)));
            assert_eq!(app.project().layers.len(), 3);
            assert_eq!(app.project().layers[2].name, format!("{} 3", mode.layer_prefix()));
            let inserted = &app.project().layers[2].pixels.to_rgba(app.project().width, app.project().height);
            if source == AiSource::Selection {
                for y in 0..128 {
                    for x in 0..128 {
                        let index = (y * 128 + x) * 4;
                        assert_eq!(&inserted[index..index + 4], if (16..112).contains(&x) && (24..104).contains(&y) { &[1, 2, 3, 255] } else { &[0, 0, 0, 0] });
                    }
                }
            } else { assert_eq!(&inserted[..4], &[1, 2, 3, 255]); }
            assert!(app.ai.open);
        }
    }

    #[test]
    fn draw_on_empty_document_waits_for_acceptance_then_creates_canvas() {
        for mode in [AiMode::Draw, AiMode::AnimeDraw, AiMode::QwenDraw] {
            let mut app = EditorApp::default();
            app.ai.open = true;
            app.ai.mode = mode;
            assert!(app.project().layers.is_empty());
            app.ai.document_id = app.workspace.active_doc().id.clone();
            app.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: 64, max_y: 128 }.into());
            let image = image::RgbaImage::from_pixel(64, 128, image::Rgba([1, 2, 3, 255]));
            app.ai.job = Some(crate::comfyui::Job::with_result(crate::comfyui::JobResult::Done(image)));
            let mut ui = Ui::new();
            app.poll_ai(&mut ui);
            assert!(app.project().layers.is_empty());
            EditorApp::init(&mut ui);
            let viewport = Vec2::new(1280.0, 800.0);
            ui.begin_frame(mega_ui::UiInput { viewport, ..Default::default() });
            app.build(&mut ui, viewport, DrawStats::default());
            let output = ui.end_frame();
            assert!(output.draw_list.iter().any(|c| c.kind == 1.0 && c.tex == crate::host::AI_PREVIEW_TEXTURE));
            app.accept_ai();
            assert_eq!(app.image_size, Some((64, 128)));
            assert_eq!(app.project().layers.len(), 1);
            assert_eq!(app.project().layers[0].name, format!("{} 1", mode.layer_prefix()));
            assert_eq!(&app.project().layers[0].pixels.to_rgba(app.project().width, app.project().height)[..4], &[1, 2, 3, 255]);
            assert!(app.ai.open);
            app.accept_ai();
            assert_eq!(app.project().layers.len(), 1);
        }
    }

    #[test]
    fn completed_job_waits_for_acceptance_and_adds_exactly_once() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[20, 40, 60, 255], 1, 1).unwrap();
        app.refresh_image().unwrap();
        app.ai.open = true;
        app.ai.document_id = app.workspace.active_doc().id.clone();
        app.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: 1, max_y: 1 }.into());
        let result = image::RgbaImage::from_pixel(2, 3, image::Rgba([100, 120, 140, 255]));
        app.ai.job = Some(crate::comfyui::Job::with_result(crate::comfyui::JobResult::Done(result.clone())));
        let mut ui = Ui::new();
        app.poll_ai(&mut ui);
        assert!(app.ai.open);
        assert!(app.ai.job.is_none());
        assert_eq!(app.image_size, Some((1, 1)));
        assert_eq!(app.project().layers.len(), 1);
        assert_eq!(app.ai.result.as_ref(), Some(&result));
        assert!(app.ai.preview_dirty);
        assert!(!app.dirty);

        app.accept_ai();
        assert!(app.ai.open);
        assert!(app.ai.result.is_none());
        assert_eq!(app.ai.preview.as_ref(), Some(&result));
        assert_eq!(app.image_size, Some((1, 1)));
        assert_eq!(app.project().active_layer, 1);
        assert_eq!(app.project().layers[1].pixels.to_rgba(app.project().width, app.project().height), [100, 120, 140, 255]);
        assert_eq!(&app.project().layers[0].pixels.to_rgba(app.project().width, app.project().height)[..4], &[20, 40, 60, 255]);
        assert!(app.dirty);
        app.accept_ai();
        assert_eq!(app.project().layers.len(), 2);
    }

    #[test]
    fn closing_unaccepted_result_does_not_change_document() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[20, 40, 60, 255], 1, 1).unwrap();
        app.refresh_image().unwrap();
        app.ai.open = true;
        app.ai.result = Some(image::RgbaImage::new(2, 3));
        app.close_ai();
        assert!(!app.ai.open);
        assert!(app.ai.result.is_none());
        assert_eq!(app.image_size, Some((1, 1)));
        assert_eq!(app.project().layers.len(), 1);
        assert!(!app.dirty);
    }

    #[test]
    fn hung_generation_dialog_closes_without_waiting_for_the_worker() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[20,40,60,255],1,1).unwrap();
        app.refresh_image().unwrap();
        let original = app.project().layers[0].pixels.clone();
        let (job,sender) = crate::comfyui::Job::pending();
        assert!(job.poll().is_none());
        app.ai.open = true;
        app.ai.job = Some(job);
        app.close_ai();
        assert!(!app.ai.open,"dialog close must not depend on a network response");
        assert!(app.ai.job.is_none());
        assert!(sender.send(crate::comfyui::JobResult::Done(image::RgbaImage::new(1,1))).is_err(),
            "late result must be detached from the editor");
        assert_eq!(app.project().layers[0].pixels,original);
        assert!(!app.dirty);
    }

    #[test]
    fn hung_generation_does_not_prevent_application_exit() {
        let mut app = EditorApp::default();
        let (job,_sender) = crate::comfyui::Job::pending();
        app.ai.job = Some(job);
        assert!(app.request_close(),"window close must not wait for a stalled worker");
        assert!(app.ai.job.is_none());
    }

    #[test]
    fn cancelling_hung_generation_frees_dialog_and_rejects_late_results() {
        let mut app = EditorApp::default();
        let (job,old_sender) = crate::comfyui::Job::pending();
        app.ai.open = true;
        app.ai.job = Some(job);
        app.cancel_ai();
        assert!(app.ai.open && app.ai.job.is_none());
        let (replacement,_new_sender) = crate::comfyui::Job::pending();
        app.ai.job = Some(replacement);
        assert!(old_sender.send(crate::comfyui::JobResult::Done(image::RgbaImage::new(1,1))).is_err());
        app.poll_ai(&mut Ui::new());
        assert!(app.ai.job.is_some() && app.ai.result.is_none());
    }

    #[test]
    fn fit_uses_unscaled_input_bounds_and_can_be_disabled() {
        for fit in [true, false] {
            let mut app = EditorApp::default();
            app.project_mut().import_rgba(&vec![20, 40, 60, 255].repeat(8), 4, 2).unwrap();
            app.refresh_image().unwrap();
            app.ai.open = true;
            app.ai.document_id = app.workspace.active_doc().id.clone();
            app.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: 4, max_y: 2 }.into());
            app.ai.scale = 0.5;
            app.ai.fit_image = fit;
            app.ai.result = Some(image::RgbaImage::from_pixel(2, 1, image::Rgba([100, 120, 140, 255])));
            app.accept_ai();
            assert_eq!(app.image_size, Some((4, 2)));
            assert_eq!(app.project().layers.len(), 2);
            let inserted = &app.project().layers[1].pixels.to_rgba(app.project().width, app.project().height);
            if fit {
                assert_eq!(*inserted, vec![100, 120, 140, 255].repeat(8));
            } else {
                assert_eq!(&inserted[..8], &[100, 120, 140, 255, 100, 120, 140, 255]);
                assert!(inserted[8..].iter().all(|b| *b == 0));
            }
        }
    }

    #[test]
    fn only_latest_result_is_accepted_and_destination_is_checked_again() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[20, 40, 60, 255], 1, 1).unwrap();
        app.refresh_image().unwrap();
        app.ai.open = true;
        app.ai.document_id = app.workspace.active_doc().id.clone();
        app.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: 1, max_y: 1 }.into());
        let mut ui = Ui::new();
        for color in [10, 80] {
            let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([color, 0, 0, 255]));
            app.ai.job = Some(crate::comfyui::Job::with_result(crate::comfyui::JobResult::Done(image)));
            app.poll_ai(&mut ui);
        }
        assert_eq!(app.project().layers.len(), 1);
        app.workspace.create_document("Other document");
        app.accept_ai();
        assert!(app.project().layers.is_empty());
        assert!(app.ai.result.is_some());
        assert!(app.ai.open);
        app.workspace.set_active_document(0);
        app.accept_ai();
        assert_eq!(app.project().layers[1].pixels.to_rgba(app.project().width, app.project().height), [80, 0, 0, 255]);
    }

    #[test]
    fn modal_blocks_editor_input_and_composes_preview_above_canvas() {
        let mut app = EditorApp::default();
        app.project_mut().import_rgba(&[20, 40, 60, 255], 1, 1).unwrap();
        app.refresh_image().unwrap();
        app.ai.open = true;
        app.ai.preview = Some(image::RgbaImage::from_pixel(10, 10, image::Rgba([1, 2, 3, 255])));
        app.ai.fraction = Some(0.4);
        let mut input = FrameInput::default();
        input.mouse_down = true;
        input.key_undo = true;
        input.key_paste = true;
        input.mouse_pos = Vec2::new(600.0, 400.0);
        app.begin_frame(&input);
        assert!(!app.input.mouse_down && !app.input.key_undo && !app.input.key_paste);

        let mut ui = Ui::new();
        EditorApp::init(&mut ui);
        let viewport = Vec2::new(1280.0, 800.0);
        ui.begin_frame(mega_ui::UiInput { viewport, ..Default::default() });
        assert!(app.build(&mut ui, viewport, DrawStats::default()));
        let output = ui.end_frame();
        let canvas = output.draw_list.iter().position(|c| c.kind == 1.0 && c.tex == crate::host::CANVAS_TEXTURE).unwrap();
        let preview = output.draw_list.iter().position(|c| c.kind == 1.0 && c.tex == crate::host::AI_PREVIEW_TEXTURE).unwrap();
        assert!(preview > canvas);
        assert!(ui.input_debug().modal_open);
        assert_eq!(app.project().layers[0].pixels.to_rgba(app.project().width, app.project().height), [20, 40, 60, 255]);
    }
