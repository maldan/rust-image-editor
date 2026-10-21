use super::*;

impl EditorApp {
    pub fn open_ai(&mut self) {
        self.open_ai_mode(AiMode::Edit);
    }

    pub fn open_ai_qwen_draw(&mut self) {
        self.open_ai_mode(AiMode::QwenDraw);
    }

    pub fn open_ai_draw(&mut self) {
        self.open_ai_mode(AiMode::Draw);
    }

    pub fn open_ai_anime_draw(&mut self) {
        self.open_ai_mode(AiMode::AnimeDraw);
    }

    pub fn open_ai_anime_redraw(&mut self) {
        self.open_ai_mode(AiMode::AnimeRedraw);
    }

    pub fn open_ai_redraw(&mut self) {
        self.open_ai_mode(AiMode::Redraw);
    }

    pub fn open_ai_inpaint(&mut self) {
        self.open_ai_mode(AiMode::Inpaint);
    }

    pub fn open_ai_lineart(&mut self) {
        self.open_ai_mode(AiMode::Lineart);
    }

    pub fn open_ai_draw_cn(&mut self) {
        self.open_ai_mode(AiMode::DrawCn);
    }

    pub fn open_ai_upscale(&mut self) {
        self.open_ai_mode(AiMode::Upscale);
    }

    pub(in crate::app) fn open_ai_mode(&mut self, mode: AiMode) {
        if (mode.needs_input() && self.image_size.is_none()) || self.ai.job.is_some() { return; }
        self.commit_transform();
        self.ai.selection_bounds = self.selection.as_ref().and_then(Selection::bounds);
        self.ai.layer_bounds = self.project().active_pixels().and_then(|pixels| pixels.bounds());
        self.workspace.clear_asset_drag();
        self.layer_drag = None;
        self.ai.open = true;
        self.ai.preview_view = Default::default();
        self.ai.mode = mode;
        if mode == AiMode::Upscale { self.ai.fit_image = false; }
        if mode == AiMode::Inpaint && self.ai.source == AiSource::Selection { self.ai.source = AiSource::Layer; }
        self.ai.catalog_load = None;
        if mode.uses_sd() { self.ai.catalog = Default::default(); }
        self.ai.preview = None;
        self.ai.result = None;
        self.ai.input_region = None;
        self.ai.preview_dirty = false;
        self.ai.fraction = None;
        match crate::comfyui::HostConfig::from_env() {
            Ok(config) => {
                if mode.uses_sd() {
                    self.ai.catalog_load = Some(crate::comfyui::CatalogLoad::start(config.clone(), false));
                    self.ai.catalog_message = "Loading models / LoRAs...".into();
                }
                self.ai.config = Some(config); self.ai.message.clear();
            }
            Err(error) => { self.ai.config = None; self.ai.message = error; }
        }
    }

    pub fn start_ai(&mut self) {
        if self.ai.job.is_some() { return; }
        self.ai.preview_view = Default::default();
        let Some(config) = self.ai.config.clone() else { return; };
        if !self.ai.mode.needs_input() {
            if let Err(error) = self.ai.draw_settings.validate() {
                self.ai.message = error;
                return;
            }
            self.ai.document_id = self.workspace.active_doc().id.clone();
            let (width, height) = if self.project().width == 0 || self.project().height == 0 {
                (self.ai.draw_settings.width as u32, self.ai.draw_settings.height as u32)
            } else { (self.project().width, self.project().height) };
            self.ai.input_region = Some(PixelRect { min_x: 0, min_y: 0, max_x: width, max_y: height }.into());
            self.ai.message = "Preparing workflow...".into();
            self.ai.preview = None;
            self.ai.result = None;
            self.ai.preview_dirty = false;
            self.ai.fraction = None;
            self.ai.job = Some(if self.ai.mode == AiMode::QwenDraw {
                crate::comfyui::Job::start_qwen_draw(config, self.ai.draw_settings.clone())
            } else if self.ai.mode == AiMode::AnimeDraw {
                crate::comfyui::Job::start_anime_draw(config, self.ai.draw_settings.clone())
            } else {
                crate::comfyui::Job::start_draw(config, self.ai.draw_settings.clone())
            });
            return;
        }
        if matches!(self.ai.mode, AiMode::Redraw | AiMode::AnimeRedraw | AiMode::Inpaint) {
            if let Err(error) = self.ai.draw_settings.validate_redraw(self.ai.denoise) {
                self.ai.message = error;
                return;
            }
        }
        if self.ai.mode == AiMode::Upscale {
            if let Err(error) = self.ai.upscale.validate(&self.ai.draw_settings) {
                self.ai.message = error;
                return;
            }
        }
        if self.ai.mode == AiMode::DrawCn {
            if let Err(error) = self.ai.control_net.validate(&self.ai.draw_settings) {
                self.ai.message = error;
                return;
            }
        }
        if self.ai.mode == AiMode::Lineart {
            if let Err(error) = crate::comfyui::validate_lineart_resolution(self.ai.lineart_resolution) {
                self.ai.message = error;
                return;
            }
        }
        let mask = if self.ai.mode == AiMode::Inpaint {
            match self.ai_mask() {
                Ok(mask) => Some(mask),
                Err(error) => { self.ai.message = error; return; }
            }
        } else { None };
        let (image, input_region) = match self.ai_input() {
            Ok(input) => input,
            Err(error) => { self.ai.message = error; return; }
        };
        self.ai.document_id = self.workspace.active_doc().id.clone();
        self.ai.input_region = Some(input_region);
        self.ai.message = "Preparing input…".into();
        self.ai.preview = None;
        self.ai.result = None;
        self.ai.preview_dirty = false;
        self.ai.fraction = None;
        self.ai.job = Some(if let Some(mask) = mask {
            crate::comfyui::Job::start_inpaint(config, image, mask, self.ai.scale, self.ai.draw_settings.clone(), self.ai.denoise)
        } else if self.ai.mode == AiMode::AnimeRedraw {
            crate::comfyui::Job::start_anime_redraw(config, image, self.ai.scale, self.ai.draw_settings.clone(), self.ai.denoise)
        } else if self.ai.mode == AiMode::Redraw {
            crate::comfyui::Job::start_redraw(config, image, self.ai.scale, self.ai.draw_settings.clone(), self.ai.denoise)
        } else if self.ai.mode == AiMode::Lineart {
            crate::comfyui::Job::start_lineart(config, image, self.ai.scale, self.ai.lineart_resolution)
        } else if self.ai.mode == AiMode::DrawCn {
            crate::comfyui::Job::start_draw_cn(config, image, self.ai.scale, self.ai.draw_settings.clone(), self.ai.control_net)
        } else if self.ai.mode == AiMode::Upscale {
            crate::comfyui::Job::start_upscale(config, image, self.ai.scale, self.ai.draw_settings.clone(), self.ai.upscale)
        } else {
            crate::comfyui::Job::start(config, image, self.ai.scale, self.ai.prompt.clone())
        });
    }

    pub(in crate::app) fn ai_mask(&self) -> Result<image::RgbaImage, String> {
        if self.ai.source == AiSource::Selection { return Err("Inpaint source must be a layer or the whole document".into()); }
        let p = self.project();
        let selection = self.selection.as_ref().ok_or("Select an area to use as the inpaint mask")?;
        if selection.width != p.width || selection.height != p.height {
            return Err("Selection does not match the current canvas".into());
        }
        if selection.is_empty() { return Err("Select an area to use as the inpaint mask".into()); }
        let region = self.ai_source_region()?;
        let (width, height) = region.size();
        crate::comfyui::scaled_size(width, height, 1.0)?;
        crate::comfyui::scaled_size(width, height, self.ai.scale)?;
        let mask = image::RgbaImage::from_fn(width, height, |x, y| {
            let x = region.min_x as i64 + x as i64;
            let y = region.min_y as i64 + y as i64;
            let value = if x >= 0 && y >= 0 && selection.contains(x as u32, y as u32) { 255 } else { 0 };
            image::Rgba([value, value, value, 255])
        });
        let softness = self.ai.mask_softness;
        if !softness.is_finite() || !(0.0..=256.0).contains(&softness) {
            return Err("Mask softness must be between 0 and 256 px".into());
        }
        Ok(if softness > 0.0 { image::imageops::blur(&mask, softness) } else { mask })
    }

    /// Crop before worker-side scaling; retain document coordinates for Accept / Fit.
    pub(in crate::app) fn ai_source_region(&self) -> Result<crate::raster::RasterRect, String> {
        let p = self.project();
        if self.ai.source == AiSource::Selection {
            let selection = self.selection.as_ref().ok_or("Select an area on the canvas first")?;
            if selection.width != p.width || selection.height != p.height {
                return Err("Selection does not match the current canvas".into());
            }
            return selection.bounds().map(Into::into).ok_or("Select an area on the canvas first".into());
        }
        if self.ai.source == AiSource::Layer {
            let pixels = p.active_pixels().ok_or("Choose a raster layer first")?;
            return self.ai.source_region(p.width, p.height, pixels.bounds());
        }
        self.ai.source_region(p.width, p.height, None)
    }

    pub(in crate::app) fn ai_input(&self) -> Result<(image::RgbaImage, crate::raster::RasterRect), String> {
        let p = self.project();
        let region = self.ai_source_region()?;
        let (width, height) = region.size();
        crate::comfyui::scaled_size(width, height, 1.0)?;
        crate::comfyui::scaled_size(width, height, self.ai.scale)?;
        let pixels = match self.ai.source {
            AiSource::Layer => p.active_pixels().ok_or("Choose a raster layer first")?.region(region.min_x, region.min_y, width, height),
            AiSource::Document | AiSource::Selection => crate::editing::composite_layers(&p.layers, p.width, p.height),
        };
        let image = if self.ai.source == AiSource::Selection {
            let selection = self.selection.as_ref().unwrap();
            let mut cropped = image::RgbaImage::new(width, height);
            for y in region.min_y as u32..region.max_y as u32 {
                for x in region.min_x as u32..region.max_x as u32 {
                    if selection.contains(x, y) {
                        let index = ((y as usize * p.width as usize) + x as usize) * 4;
                        cropped.put_pixel(x - region.min_x as u32, y - region.min_y as u32,
                            image::Rgba(pixels[index..index + 4].try_into().unwrap()));
                    }
                }
            }
            cropped
        } else {
            image::RgbaImage::from_raw(width, height, pixels).ok_or("Invalid input raster")?
        };
        Ok((image, region))
    }

    pub fn cancel_ai(&mut self) {
        if let Some(job) = self.ai.job.take() {
            job.cancel();
            self.ai.result = None;
            self.ai.fraction = None;
            self.ai.message = "Cancelled locally; remote stop requested in background".into();
        }
    }

    pub fn close_ai(&mut self) {
        self.cancel_ai();
        self.ai.open = false;
        self.ai.catalog_load = None;
        self.ai.result = None;
        self.ai.input_region = None;
        self.ai.preview = None;
        self.ai.preview_dirty = false;
        self.ai.fraction = None;
    }

    pub fn accept_ai(&mut self) {
        if self.ai.job.is_some() || self.ai.result.is_none() { return; }
        if self.workspace.active_doc().id != self.ai.document_id {
            self.ai.message = "Destination document changed; result was not inserted".into();
            return;
        }
        let image = self.ai.result.as_ref().unwrap();
        let Some(region) = self.ai.input_region.filter(|r| !r.is_empty()) else {
            self.ai.message = "Missing source bounds; regenerate the image".into();
            return;
        };
        let size = if self.ai.fit_image { region.size() } else { image.dimensions() };
        let action = self.ai.mode.layer_prefix();
        let name = format!("{action} {}", self.workspace.active().layers.len() + 1);
        let project = self.workspace.active_mut();
        let insertion = if !self.ai.mode.needs_input() && (project.width == 0 || project.height == 0) {
            project.import_rgba(image.as_raw(), image.width(), image.height()).map(|()| {
                project.layers[0].name = name;
                project.sync_manifest_pub();
            })
        } else {
            project.add_raster_layer(image, name, region, self.ai.fit_image)
        };
        match insertion {
            Ok(()) => {
                let needs_fit = self.image_size.is_none();
                self.history.clear();
                self.sync_layer_structure();
                self.ai.layer_bounds = self.project().active_pixels().and_then(|pixels| pixels.bounds());
                if needs_fit { self.fit_image = true; }
                self.status = format!("{action} added: {} × {} px", size.0, size.1);
                self.ai.preview = self.ai.result.take();
                self.ai.message = format!("{} — regenerate for another result", self.status);
            }
            Err(error) => self.ai.message = error,
        }
    }

    pub(in crate::app) fn poll_ai(&mut self, ui: &mut Ui) {
        let Some(job) = self.ai.job.as_ref() else { return; };
        if let Ok(mut progress) = job.progress.try_lock() {
            if !job.is_cancelling() { self.ai.message = progress.message.clone(); }
            self.ai.fraction = progress.fraction;
            if let Some(preview) = progress.preview.take() {
                self.ai.preview = Some(preview);
                self.ai.preview_dirty = true;
            }
        }
        let Some(result) = job.poll() else { return; };
        self.ai.job = None;
        match result {
            crate::comfyui::JobResult::Done(image) => {
                if self.workspace.active_doc().id != self.ai.document_id {
                    self.ai.message = "Destination document changed; result was not inserted".into();
                    return;
                }
                self.ai.message = format!("Result: {} × {} px — accept or regenerate", image.width(), image.height());
                self.ai.preview = Some(image::imageops::thumbnail(&image, 1024, 1024));
                self.ai.preview_dirty = true;
                self.ai.fraction = None;
                self.ai.result = Some(image);
            }
            crate::comfyui::JobResult::Failed(error) => {
                self.ai.message = error.clone();
                self.status = error.clone();
                ui.notify_error(&error);
            }
            crate::comfyui::JobResult::Cancelled => {
                self.ai.message = "Cancelled".into();
                self.ai.fraction = None;
            }
        }
    }
}
