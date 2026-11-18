use super::*;

impl EditorApp {
    pub(crate) fn poll_catalog(&mut self, ui: &mut Ui) {
        if let Some(load) = &self.ai.catalog_load {
            if let Some(result) = load.poll() {
                match result {
                    Ok((catalog, message)) => {
                        self.ai.catalog = catalog;
                        self.ai.catalog_message = message;
                    }
                    Err(error) => self.ai.catalog_message = error,
                }
                self.ai.catalog_load = None;
            } else {
                ui.request_repaint();
            }
        }
    }

    pub(crate) fn ensure_sd_catalog(&mut self) {
        if self.ai.catalog_load.is_some()
            || !self.ai.catalog.models.is_empty()
            || !self.ai.catalog.loras.is_empty()
            || !self.ai.catalog_message.is_empty()
        {
            return;
        }
        let Ok(config) = crate::comfyui::HostConfig::load() else {
            return;
        };
        self.ai.config = Some(config.clone());
        self.ai.catalog_load = Some(crate::comfyui::CatalogLoad::start(config, false));
        self.ai.catalog_message = "Loading models / LoRAs...".into();
    }

    pub fn open_ai_video(&mut self) {
        self.open_ai_mode(AiMode::Video);
    }

    pub fn open_ai_animation(&mut self) {
        if self.project().animation.is_none() || self.session.preview_asset.is_some() {
            return;
        }
        self.open_ai_mode(AiMode::Animation);
    }

    pub fn open_ai(&mut self) {
        self.open_ai_mode(AiMode::Edit);
    }

    pub fn open_ai_minimax_redraw(&mut self) {
        self.open_ai_mode(AiMode::MinimaxRedraw);
    }

    pub fn open_ai_minimax_i2v(&mut self) {
        self.open_ai_mode(AiMode::MinimaxI2V);
    }

    pub fn open_ai_minimax_refs(&mut self) {
        self.open_ai_mode(AiMode::MinimaxRefs);
    }

    pub fn open_ai_minimax_r2v(&mut self) {
        self.open_ai_mode(AiMode::MinimaxR2V);
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
        if self.camera_visible() { return; }
        if (!mode.uses_video() && mode.needs_input() && self.image_size.is_none())
            || self.ai.job.is_some()
        {
            return;
        }
        self.commit_transform();
        self.ai.selection_bounds = self.session.selection.as_ref().and_then(Selection::bounds);
        self.ai.layer_bounds = self
            .project()
            .active_pixels()
            .and_then(|pixels| pixels.bounds());
        self.workspace.clear_asset_drag();
        self.layer_drag = None;
        self.ai.open = true;
        self.ai.preview_view = Default::default();
        self.ai.mode = mode;
        if matches!(mode, AiMode::Video | AiMode::MinimaxI2V | AiMode::MinimaxR2V) {
            self.ai.document_id = self.workspace.active_doc().id.clone();
            self.ai.video_layer = None;
            self.ai.video_end_document = None;
            self.ai.video_end_layer = None;
        }
        if matches!(
            mode,
            AiMode::MinimaxI2V | AiMode::MinimaxR2V | AiMode::MinimaxRefs
        ) {
            let refs: &mut [crate::ui::ai_dialog::VideoRef] = if mode == AiMode::MinimaxI2V {
                &mut self.ai.i2v_refs
            } else if mode == AiMode::MinimaxRefs {
                &mut self.ai.minimax_refs
            } else {
                &mut self.ai.r2v_refs
            };
            if refs[0].document_id.is_empty() {
                let id = self.workspace.active_doc().id.clone();
                for (i, reference) in refs.iter_mut().enumerate() {
                    reference.document_id = id.clone();
                    reference.layer = None;
                    reference.enabled = i == 0;
                }
            }
        }
        if mode == AiMode::Animation {
            if let Some(animation) = self.project().animation.clone() {
                self.ai.document_id = self.workspace.active_doc().id.clone();
                self.ai.anim_start = 0;
                self.ai.anim_end = animation.length.saturating_sub(1);
                self.ai.anim_scale = 1;
            }
        }
        if mode == AiMode::Upscale {
            self.ai.fit_image = false;
        }
        if mode == AiMode::Inpaint && self.ai.source == AiSource::Selection {
            self.ai.source = AiSource::Layer;
        }
        self.ai.catalog_load = None;
        if mode.uses_sd() || matches!(mode, AiMode::Video | AiMode::Animation) {
            self.ai.catalog = Default::default();
        }
        self.ai.preview = None;
        self.ai.result = None;
        self.ai.result_metadata = None;
        self.ai.input_region = None;
        self.ai.anim_frames = None;
        self.ai.anim_play = 0.0;
        self.ai.anim_shown = 0;
        self.ai.preview_dirty = false;
        self.ai.fraction = None;
        match crate::comfyui::HostConfig::load() {
            Ok(config) => {
                if mode.uses_sd() || matches!(mode, AiMode::Video | AiMode::Animation) {
                    self.ai.catalog_load =
                        Some(crate::comfyui::CatalogLoad::start(config.clone(), false));
                    self.ai.catalog_message = "Loading models / LoRAs...".into();
                }
                self.ai.config = Some(config);
                self.ai.message.clear();
            }
            Err(error) => {
                self.ai.config = None;
                self.ai.message = error;
            }
        }
    }

    pub fn start_ai(&mut self) {
        if self.ai.job.is_some() {
            return;
        }
        if matches!(
            self.ai.mode,
            AiMode::Video | AiMode::MinimaxI2V | AiMode::MinimaxR2V
        ) {
            self.start_video();
            return;
        }
        if self.ai.mode == AiMode::Animation {
            self.start_animation();
            return;
        }
        self.ai.preview_view = Default::default();
        let Some(config) = self.ai.config.clone() else {
            return;
        };
        if self.ai.mode == AiMode::MinimaxRefs {
            self.start_minimax_refs(config);
            return;
        }
        if !self.ai.mode.needs_input() {
            if let Err(error) = self.ai.draw_settings.validate() {
                self.ai.message = error;
                return;
            }
            self.ai.document_id = self.workspace.active_doc().id.clone();
            let (width, height) = if self.project().width == 0 || self.project().height == 0 {
                (
                    self.ai.draw_settings.width as u32,
                    self.ai.draw_settings.height as u32,
                )
            } else {
                (self.project().width, self.project().height)
            };
            self.ai.input_region = Some(
                PixelRect {
                    min_x: 0,
                    min_y: 0,
                    max_x: width,
                    max_y: height,
                }
                .into(),
            );
            self.ai.message = "Preparing workflow...".into();
            self.ai.preview = None;
            self.ai.result = None;
            self.ai.result_metadata = None;
            self.ai.preview_dirty = false;
            self.ai.fraction = None;
            self.ai.result_metadata = Some(self.ai.generation_metadata());
            self.ai.job = Some(if self.ai.mode == AiMode::QwenDraw {
                crate::comfyui::Job::start_qwen_draw(config, self.ai.draw_settings.clone())
            } else if self.ai.mode == AiMode::AnimeDraw {
                crate::comfyui::Job::start_anime_draw(config, self.ai.draw_settings.clone())
            } else {
                crate::comfyui::Job::start_draw(config, self.ai.draw_settings.clone())
            });
            return;
        }
        if matches!(
            self.ai.mode,
            AiMode::Redraw | AiMode::AnimeRedraw | AiMode::Inpaint
        ) {
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
            if let Err(error) =
                crate::comfyui::validate_lineart_resolution(self.ai.lineart_resolution)
            {
                self.ai.message = error;
                return;
            }
        }
        if self.ai.mode == AiMode::MinimaxRedraw {
            if let Err(error) = crate::comfyui::validate_megapixels(self.ai.megapixels) {
                self.ai.message = error;
                return;
            }
        }
        let mask = if self.ai.mode == AiMode::Inpaint {
            match self.ai_mask() {
                Ok(mask) => Some(mask),
                Err(error) => {
                    self.ai.message = error;
                    return;
                }
            }
        } else {
            None
        };
        let (image, input_region) = match self.ai_input() {
            Ok(input) => input,
            Err(error) => {
                self.ai.message = error;
                return;
            }
        };
        self.ai.document_id = self.workspace.active_doc().id.clone();
        self.ai.input_region = Some(input_region);
        self.ai.message = "Preparing input…".into();
        self.ai.preview = None;
        self.ai.result = None;
        self.ai.result_metadata = None;
        self.ai.preview_dirty = false;
        self.ai.fraction = None;
        self.ai.result_metadata = Some(self.ai.generation_metadata());
        self.ai.job = Some(if let Some(mask) = mask {
            crate::comfyui::Job::start_inpaint(
                config,
                image,
                mask,
                self.ai.scale,
                self.ai.draw_settings.clone(),
                self.ai.denoise,
            )
        } else if self.ai.mode == AiMode::AnimeRedraw {
            crate::comfyui::Job::start_anime_redraw(
                config,
                image,
                self.ai.scale,
                self.ai.draw_settings.clone(),
                self.ai.denoise,
            )
        } else if self.ai.mode == AiMode::Redraw {
            crate::comfyui::Job::start_redraw(
                config,
                image,
                self.ai.scale,
                self.ai.draw_settings.clone(),
                self.ai.denoise,
                0,
            )
        } else if self.ai.mode == AiMode::Lineart {
            crate::comfyui::Job::start_lineart(
                config,
                image,
                self.ai.scale,
                self.ai.lineart_resolution,
            )
        } else if self.ai.mode == AiMode::DrawCn {
            crate::comfyui::Job::start_draw_cn(
                config,
                image,
                self.ai.scale,
                self.ai.draw_settings.clone(),
                self.ai.control_net,
            )
        } else if self.ai.mode == AiMode::Upscale {
            crate::comfyui::Job::start_upscale(
                config,
                image,
                self.ai.scale,
                self.ai.draw_settings.clone(),
                self.ai.upscale,
            )
        } else if self.ai.mode == AiMode::MinimaxRedraw {
            crate::comfyui::Job::start_minimax_redraw(
                config,
                image,
                self.ai.scale,
                self.ai.prompt.clone(),
                self.ai.megapixels,
            )
        } else {
            crate::comfyui::Job::start(config, image, self.ai.scale, self.ai.prompt.clone())
        });
    }

    pub(crate) fn video_settings(&self) -> Result<crate::comfyui::VideoSettings, String> {
        let doc = self.workspace.documents.iter()
            .find(|d| d.id == self.ai.document_id)
            .ok_or("Choose a source document")?;
        self.ai.video_settings.clone().with_source_size(
            doc.project.width, doc.project.height, self.ai.video_scale,
        )
    }

    fn start_video(&mut self) {
        let Some(config) = self.ai.config.clone() else {
            return;
        };
        if self.ai.mode == AiMode::MinimaxR2V {
            self.start_minimax_r2v(config);
            return;
        }
        if self.ai.mode == AiMode::MinimaxI2V {
            self.start_minimax_i2v(config);
            return;
        }
        let image = match self.video_input() {
            Ok(image) => image,
            Err(error) => {
                self.ai.message = error;
                return;
            }
        };
        let settings = match self.video_settings() {
            Ok(settings) => settings,
            Err(error) => {
                self.ai.message = error;
                return;
            }
        };
        if let Err(error) = settings.validate() {
            self.ai.message = error;
            return;
        }
        let end_image = match self.ai.video_end_document.clone() {
            Some(document) => match self.video_frame_input(&document, self.ai.video_end_layer) {
                Ok(image) => Some(image),
                Err(error) => {
                    self.ai.message = error;
                    return;
                }
            },
            None => None,
        };
        self.ai.message = "Preparing video input…".into();
        self.ai.fraction = None;
        self.ai.preview = None;
        self.ai.preview_dirty = false;
        self.ai.job = Some(crate::comfyui::Job::start_video(
            config,
            image,
            end_image,
            settings,
        ));
    }

    fn start_minimax_i2v(&mut self, config: crate::comfyui::HostConfig) {
        if let Err(error) = crate::comfyui::validate_minimax_i2v(
            self.ai.minimax_i2v_megapixels,
            self.ai.video_settings.seconds,
            &self.ai.video_settings.prompt,
        ) {
            self.ai.message = error;
            return;
        }
        self.ai.i2v_refs[0].enabled = true;
        let first = match self.video_frame_input(
            &self.ai.i2v_refs[0].document_id.clone(),
            self.ai.i2v_refs[0].layer,
        ) {
            Ok(image) => image,
            Err(error) => {
                self.ai.message = error;
                return;
            }
        };
        let last = if self.ai.i2v_refs[1].enabled {
            match self.video_frame_input(
                &self.ai.i2v_refs[1].document_id.clone(),
                self.ai.i2v_refs[1].layer,
            ) {
                Ok(image) => Some(image),
                Err(error) => {
                    self.ai.message = error;
                    return;
                }
            }
        } else {
            None
        };
        self.ai.message = "Preparing video input…".into();
        self.ai.fraction = None;
        self.ai.preview = None;
        self.ai.preview_dirty = false;
        self.ai.job = Some(crate::comfyui::Job::start_minimax_i2v(
            config,
            first,
            last,
            self.ai.video_settings.prompt.clone(),
            self.ai.minimax_i2v_megapixels,
            self.ai.video_settings.seconds,
            self.ai.minimax_i2v_turbo,
        ));
    }

    fn start_minimax_refs(&mut self, config: crate::comfyui::HostConfig) {
        let has_image = self.ai.minimax_refs.iter().any(|r| r.enabled);
        let custom_size = self
            .ai
            .refs_custom_size
            .then_some((self.ai.refs_width, self.ai.refs_height));
        if let Err(error) = crate::comfyui::validate_minimax_refs(
            self.ai.megapixels,
            &self.ai.prompt,
            has_image,
            custom_size,
        ) {
            self.ai.message = error;
            return;
        }
        let mut images = [None, None, None, None];
        for i in 0..4 {
            if !self.ai.minimax_refs[i].enabled {
                continue;
            }
            let document = self.ai.minimax_refs[i].document_id.clone();
            let layer = self.ai.minimax_refs[i].layer;
            match self.video_frame_input(&document, layer) {
                Ok(image) => images[i] = Some(image),
                Err(error) => {
                    self.ai.message = error;
                    return;
                }
            }
        }
        let size = custom_size.map(|(w, h)| (w as u32, h as u32));
        let (region_w, region_h) = size.or_else(|| {
            images
                .iter()
                .flatten()
                .next()
                .map(|image| (image.width(), image.height()))
        }).unwrap_or((self.ai.refs_width as u32, self.ai.refs_height as u32));
        self.ai.document_id = self.workspace.active_doc().id.clone();
        self.ai.input_region = Some(
            crate::editing::PixelRect {
                min_x: 0,
                min_y: 0,
                max_x: region_w,
                max_y: region_h,
            }
            .into(),
        );
        self.ai.message = "Preparing input…".into();
        self.ai.preview = None;
        self.ai.result = None;
        self.ai.result_metadata = None;
        self.ai.preview_dirty = false;
        self.ai.fraction = None;
        self.ai.result_metadata = Some(self.ai.generation_metadata());
        self.ai.job = Some(crate::comfyui::Job::start_minimax_refs(
            config,
            images,
            size,
            self.ai.prompt.clone(),
            self.ai.megapixels,
            0,
        ));
    }

    fn start_minimax_r2v(&mut self, config: crate::comfyui::HostConfig) {
        let has_image = self.ai.r2v_refs.iter().any(|r| r.enabled);
        let has_video = self.ai.r2v_video.iter().any(|on| *on);
        let custom_size = self
            .ai
            .r2v_custom_size
            .then_some((self.ai.r2v_width, self.ai.r2v_height));
        if let Err(error) = crate::comfyui::validate_minimax_r2v(
            self.ai.minimax_i2v_megapixels,
            self.ai.video_settings.seconds,
            &self.ai.video_settings.prompt,
            has_image,
            has_video,
            custom_size,
        ) {
            self.ai.message = error;
            return;
        }
        let mut images = [None, None, None, None];
        for i in 0..4 {
            if !self.ai.r2v_refs[i].enabled {
                continue;
            }
            let document = self.ai.r2v_refs[i].document_id.clone();
            let layer = self.ai.r2v_refs[i].layer;
            match self.video_frame_input(&document, layer) {
                Ok(image) => images[i] = Some(image),
                Err(error) => {
                    self.ai.message = error;
                    return;
                }
            }
        }
        let mut videos = [None, None];
        for i in 0..2 {
            if !self.ai.r2v_video[i] {
                continue;
            }
            let id = self.ai.r2v_video_id[i].clone();
            match self.workspace.videos.iter().find(|v| v.id == id) {
                Some(asset) => match asset.read() {
                    Ok(bytes) => videos[i] = Some(bytes),
                    Err(error) => {
                        self.ai.message = error;
                        return;
                    }
                },
                None => {
                    self.ai.message = "Choose a reference video".into();
                    return;
                }
            }
        }
        let size = custom_size.map(|(w, h)| (w as u32, h as u32));
        self.ai.message = "Preparing video input…".into();
        self.ai.fraction = None;
        self.ai.preview = None;
        self.ai.preview_dirty = false;
        self.ai.job = Some(crate::comfyui::Job::start_minimax_r2v(
            config,
            images,
            videos,
            size,
            self.ai.video_settings.prompt.clone(),
            self.ai.minimax_i2v_megapixels,
            self.ai.video_settings.seconds,
            self.ai.minimax_r2v_lightning,
        ));
    }

    fn start_animation(&mut self) {
        let Some(config) = self.ai.config.clone() else {
            return;
        };
        let settings = match self.animation_settings() {
            Ok(settings) => settings,
            Err(error) => {
                self.ai.message = error;
                return;
            }
        };
        let start = self.ai.anim_start;
        let end = self.ai.anim_end;
        let image = match self.animation_frame_image(start) {
            Ok(image) => image,
            Err(error) => {
                self.ai.message = error;
                return;
            }
        };
        let end_image = if end == start {
            None
        } else {
            match self.animation_frame_image(end) {
                Ok(image) => Some(image),
                Err(error) => {
                    self.ai.message = error;
                    return;
                }
            }
        };
        self.ai.result_metadata = Some(self.ai.generation_metadata());
        self.ai.anim_apply = (start, end);
        self.ai.anim_frames = None;
        self.ai.anim_play = 0.0;
        self.ai.anim_shown = 0;
        self.ai.message = "Preparing animation input…".into();
        self.ai.fraction = None;
        self.ai.preview = None;
        self.ai.preview_dirty = false;
        self.ai.frame_decode = None;
        self.ai.job = Some(crate::comfyui::Job::start_video(
            config,
            image,
            end_image,
            settings,
        ));
    }

    pub(crate) fn animation_settings(&self) -> Result<crate::comfyui::VideoSettings, String> {
        let doc = self
            .workspace
            .documents
            .iter()
            .find(|d| d.id == self.ai.document_id)
            .ok_or("Choose an animation document")?;
        let animation = doc
            .project
            .animation
            .as_ref()
            .ok_or("Choose an animation document")?;
        if doc.project.width == 0 || doc.project.height == 0 {
            return Err("Animation canvas is empty".into());
        }
        if self.ai.anim_end < self.ai.anim_start || self.ai.anim_end >= animation.length {
            return Err("Frame range is outside the animation".into());
        }
        let span = self.ai.anim_end - self.ai.anim_start + 1;
        let mut settings = self.ai.video_settings.clone();
        settings.fps = crate::comfyui::LTX_FPS;
        settings.seconds = crate::comfyui::generated_seconds(span, self.ai.anim_scale)?;
        settings.validate()?;
        Ok(settings)
    }

    pub(in crate::app) fn animation_frame_image(
        &mut self,
        frame: u32,
    ) -> Result<image::RgbaImage, String> {
        let id = self.ai.document_id.clone();
        let index = self
            .workspace
            .documents
            .iter()
            .position(|d| d.id == id)
            .ok_or("Choose an animation document")?;
        self.workspace.ensure_document_loaded(index)?;
        let project = &mut self.workspace.documents[index].project;
        let length = project
            .animation
            .as_ref()
            .ok_or("Choose an animation document")?
            .length;
        if frame >= length {
            return Err("Frame is outside the animation".into());
        }
        project.flush_drawings();
        let width = project.width;
        let height = project.height;
        crate::comfyui::scaled_size(width, height, 1.0)?;
        let saved: Vec<_> = project.layers.iter().map(|layer| layer.pixels.clone()).collect();
        for layer in &mut project.layers {
            layer.pixels = layer
                .drawings
                .range(..=frame)
                .next_back()
                .map(|(_, pixels)| pixels.clone())
                .unwrap_or_default();
        }
        let pixels = crate::editing::composite_layers(&project.layers, width, height);
        for (layer, pixels) in project.layers.iter_mut().zip(saved) {
            layer.pixels = pixels;
        }
        image::RgbaImage::from_raw(width, height, pixels).ok_or("Invalid animation frame".into())
    }

    pub fn accept_animation(&mut self) {
        if self.ai.job.is_some() || self.ai.frame_decode.is_some() {
            return;
        }
        let Some(frames) = self.ai.anim_frames.take() else {
            return;
        };
        if !self.place_animation_frames(&frames) {
            self.ai.anim_frames = Some(frames);
        }
    }

    pub(in crate::app) fn place_animation_frames(&mut self, frames: &[image::RgbaImage]) -> bool {
        if frames.is_empty() {
            self.ai.message = "Generated video has no frames".into();
            return false;
        }
        let (start, end) = self.ai.anim_apply;
        let id = self.ai.document_id.clone();
        let Some(index) = self.workspace.documents.iter().position(|d| d.id == id) else {
            self.ai.message = "Destination document changed; result was not inserted".into();
            return false;
        };
        if let Err(error) = self.workspace.ensure_document_loaded(index) {
            self.ai.message = error;
            return false;
        }
        let project = &self.workspace.documents[index].project;
        let Some(animation) = project.animation.clone() else {
            self.ai.message = "Destination is not an animation".into();
            return false;
        };
        if start > end || end >= animation.length || project.width == 0 || project.height == 0 {
            self.ai.message = "Frame range no longer fits the animation".into();
            return false;
        }
        let width = project.width;
        let height = project.height;
        let current = animation.frame;
        let length = animation.length;
        let name = format!("Ltx 2.3 {}", project.layers.len() + 1);
        let span = end - start + 1;
        let mut cached = vec![None; frames.len()];
        let mut drawings = std::collections::BTreeMap::new();
        if start > 0 {
            drawings.insert(0, crate::raster::SparsePixels::default());
        }
        for slot in 0..span {
            let source = crate::comfyui::frame_sample(slot, span, frames.len());
            if cached[source].is_none() {
                let image = &frames[source];
                let mut pixels = crate::raster::SparsePixels::from_rgba(
                    image.as_raw(),
                    image.width(),
                    image.height(),
                );
                pixels.offset = (
                    (width as i32 - image.width() as i32) / 2,
                    (height as i32 - image.height() as i32) / 2,
                );
                cached[source] = Some(pixels);
            }
            drawings.insert(start + slot, cached[source].clone().unwrap());
        }
        if end + 1 < length {
            drawings.insert(end + 1, crate::raster::SparsePixels::default());
        }
        let pixels = drawings
            .range(..=current)
            .next_back()
            .map(|(_, pixels)| pixels.clone())
            .unwrap_or_default();
        let project = &mut self.workspace.documents[index].project;
        project.add_layer();
        let layer = &mut project.layers[project.active_layer];
        layer.name = name.clone();
        layer.ai_metadata = self.ai.result_metadata.clone();
        layer.drawings = drawings;
        layer.pixels = pixels;
        self.workspace.documents[index].mark_thumb_dirty();
        self.dirty = true;
        if index == self.workspace.active_document {
            self.history.clear_redo();
            self.sync_layer_structure();
            self.seek_animation(start);
        }
        self.status = format!("{name} added: frames {}–{}", start + 1, end + 1);
        self.ai.message = format!("{} — regenerate for another result", self.status);
        true
    }

    pub(in crate::app) fn tick_animation_preview(&mut self) {
        let Some(count) = self.ai.anim_frames.as_ref().map(|frames| frames.len()) else {
            return;
        };
        if count == 0 {
            return;
        }
        let step = 1.0 / crate::comfyui::LTX_FPS as f64;
        self.ai.anim_play += self.ai.preview_input.dt.max(0.0) as f64;
        let duration = count as f64 * step;
        if self.ai.anim_play >= duration {
            self.ai.anim_play %= duration;
        }
        let index = (self.ai.anim_play / step).floor() as usize % count;
        if index != self.ai.anim_shown {
            self.ai.anim_shown = index;
            self.ai.preview_dirty = true;
        }
    }

    pub(in crate::app) fn video_input(&mut self) -> Result<image::RgbaImage, String> {
        self.video_frame_input(&self.ai.document_id.clone(), self.ai.video_layer)
    }

    pub(in crate::app) fn video_frame_input(
        &mut self,
        document_id: &str,
        layer: Option<usize>,
    ) -> Result<image::RgbaImage, String> {
        let index = self
            .workspace
            .documents
            .iter()
            .position(|d| d.id == document_id)
            .ok_or("Choose a source document")?;
        self.workspace.ensure_document_loaded(index)?;
        let p = &self.workspace.documents[index].project;
        crate::comfyui::scaled_size(p.width, p.height, 1.0)?;
        let pixels = if let Some(layer) = layer {
            let layer = p.layers.get(layer).ok_or("Choose a source layer")?;
            layer.pixels.region(0, 0, p.width, p.height)
        } else {
            crate::editing::composite_layers(&p.layers, p.width, p.height)
        };
        image::RgbaImage::from_raw(p.width, p.height, pixels).ok_or("Invalid video input".into())
    }

    pub(in crate::app) fn ai_mask(&self) -> Result<image::RgbaImage, String> {
        if self.ai.source == AiSource::Selection {
            return Err("Inpaint source must be a layer or the whole document".into());
        }
        let p = self.project();
        let selection = self
            .session
            .selection
            .as_ref()
            .ok_or("Select an area to use as the inpaint mask")?;
        if selection.width != p.width || selection.height != p.height {
            return Err("Selection does not match the current canvas".into());
        }
        if selection.is_empty() {
            return Err("Select an area to use as the inpaint mask".into());
        }
        let region = self.ai_source_region()?;
        let (width, height) = region.size();
        crate::comfyui::scaled_size(width, height, 1.0)?;
        crate::comfyui::scaled_size(width, height, self.ai.scale)?;
        let mask = image::RgbaImage::from_fn(width, height, |x, y| {
            let x = region.min_x as i64 + x as i64;
            let y = region.min_y as i64 + y as i64;
            let value = if x >= 0 && y >= 0 && selection.contains(x as u32, y as u32) {
                255
            } else {
                0
            };
            image::Rgba([value, value, value, 255])
        });
        let softness = self.ai.mask_softness;
        if !softness.is_finite() || !(0.0..=256.0).contains(&softness) {
            return Err("Mask softness must be between 0 and 256 px".into());
        }
        Ok(if softness > 0.0 {
            image::imageops::blur(&mask, softness)
        } else {
            mask
        })
    }

    /// Crop before worker-side scaling; retain document coordinates for Accept / Fit.
    pub(in crate::app) fn ai_source_region(&self) -> Result<crate::raster::RasterRect, String> {
        let p = self.project();
        if self.ai.source == AiSource::Selection {
            let selection = self
                .session
                .selection
                .as_ref()
                .ok_or("Select an area on the canvas first")?;
            if selection.width != p.width || selection.height != p.height {
                return Err("Selection does not match the current canvas".into());
            }
            return selection
                .bounds()
                .map(Into::into)
                .ok_or("Select an area on the canvas first".into());
        }
        if self.ai.source == AiSource::Layer {
            let pixels = p.active_pixels().ok_or("Choose a raster layer first")?;
            return self.ai.source_region(p.width, p.height, pixels.bounds());
        }
        self.ai.source_region(p.width, p.height, None)
    }

    pub(in crate::app) fn ai_input(
        &self,
    ) -> Result<(image::RgbaImage, crate::raster::RasterRect), String> {
        let p = self.project();
        let region = self.ai_source_region()?;
        let (width, height) = region.size();
        crate::comfyui::scaled_size(width, height, 1.0)?;
        crate::comfyui::scaled_size(width, height, self.ai.scale)?;
        let pixels = match self.ai.source {
            AiSource::Layer => p
                .active_pixels()
                .ok_or("Choose a raster layer first")?
                .region(region.min_x, region.min_y, width, height),
            AiSource::Document | AiSource::Selection => {
                crate::editing::composite_layers(&p.layers, p.width, p.height)
            }
        };
        let image = if self.ai.source == AiSource::Selection {
            let selection = self.session.selection.as_ref().unwrap();
            let mut cropped = image::RgbaImage::new(width, height);
            for y in region.min_y as u32..region.max_y as u32 {
                for x in region.min_x as u32..region.max_x as u32 {
                    if selection.contains(x, y) {
                        let index = ((y as usize * p.width as usize) + x as usize) * 4;
                        cropped.put_pixel(
                            x - region.min_x as u32,
                            y - region.min_y as u32,
                            image::Rgba(pixels[index..index + 4].try_into().unwrap()),
                        );
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
        if self.ai.frame_decode.take().is_some() && self.ai.job.is_none() {
            self.ai.message = "Cancelled".into();
            self.ai.fraction = None;
        }
        if let Some(job) = self.ai.job.take() {
            job.cancel();
            self.ai.result = None;
            self.ai.result_metadata = None;
            self.ai.fraction = None;
            self.ai.message = "Cancelled locally; remote stop requested in background".into();
        }
    }

    pub fn close_ai(&mut self) {
        self.cancel_ai();
        self.ai.open = false;
        self.ai.catalog_load = None;
        self.ai.result = None;
        self.ai.result_metadata = None;
        self.ai.input_region = None;
        self.ai.preview = None;
        self.ai.anim_frames = None;
        self.ai.anim_play = 0.0;
        self.ai.anim_shown = 0;
        self.ai.preview_dirty = false;
        self.ai.fraction = None;
    }

    pub fn accept_ai(&mut self) {
        if self.ai.job.is_some() || self.ai.result.is_none() {
            return;
        }
        if self.workspace.active_doc().id != self.ai.document_id {
            self.ai.message = "Destination document changed; result was not inserted".into();
            return;
        }
        let image = self.ai.result.as_ref().unwrap();
        let Some(region) = self.ai.input_region.filter(|r| !r.is_empty()) else {
            self.ai.message = "Missing source bounds; regenerate the image".into();
            return;
        };
        let size = if self.ai.fit_image {
            region.size()
        } else {
            image.dimensions()
        };
        let action = self.ai.mode.layer_prefix();
        let name = format!("{action} {}", self.workspace.active().layers.len() + 1);
        let project = self.workspace.active_mut();
        let onto_current = !self.ai.new_layer
            && !project.layers.is_empty()
            && project.width > 0
            && project.height > 0;
        let layer_index = project.active_layer;
        let before = onto_current.then(|| project.layers[layer_index].pixels.clone());
        let insertion =
            if !self.ai.mode.needs_input() && (project.width == 0 || project.height == 0) {
                project
                    .import_rgba(image.as_raw(), image.width(), image.height())
                    .map(|()| {
                        project.layers[0].name = name;
                    })
            } else if onto_current {
                project.composite_onto_active_layer(image, region, self.ai.fit_image)
            } else {
                project.add_raster_layer(image, name, region, self.ai.fit_image)
            };
        match insertion {
            Ok(()) => {
                if !onto_current {
                    let metadata = self.ai.result_metadata.clone();
                    let project = self.workspace.active_mut();
                    project.layers[project.active_layer].ai_metadata = metadata;
                }
                let needs_fit = self.image_size.is_none();
                if let Some(before) = before {
                    let after = self.project().layers[layer_index].pixels.clone();
                    self.history.push_diff(layer_index, &before, &after);
                } else {
                    self.history.clear();
                }
                self.sync_layer_structure();
                self.ai.layer_bounds = self
                    .project()
                    .active_pixels()
                    .and_then(|pixels| pixels.bounds());
                if needs_fit {
                    self.session.fit_image = true;
                }
                self.status = if onto_current {
                    format!("{action} applied to the current layer: {} × {} px", size.0, size.1)
                } else {
                    format!("{action} added: {} × {} px", size.0, size.1)
                };
                self.ai.preview = self.ai.result.take();
                self.ai.message = format!("{} — regenerate for another result", self.status);
            }
            Err(error) => self.ai.message = error,
        }
    }

    pub(in crate::app) fn poll_ai(&mut self, ui: &mut Ui) {
        let decoded = self
            .ai
            .frame_decode
            .as_ref()
            .and_then(|decode| decode.poll());
        if decoded.is_some() {
            self.ai.frame_decode = None;
        }
        if let Some(result) = decoded {
            match result {
                Ok(frames) => {
                    self.ai.anim_play = 0.0;
                    self.ai.anim_shown = 0;
                    self.ai.preview_dirty = true;
                    self.ai.message = format!(
                        "{} frames — accept to add them, or regenerate",
                        frames.len()
                    );
                    self.ai.anim_frames = Some(frames);
                }
                Err(error) => {
                    self.ai.message = error.clone();
                    self.status = error.clone();
                    ui.notify_error(&error);
                }
            }
        }
        let Some(job) = self.ai.job.as_ref() else {
            return;
        };
        if let Ok(mut progress) = job.progress.try_lock() {
            if !job.is_cancelling() {
                self.ai.message = progress.message.clone();
            }
            self.ai.fraction = progress.fraction;
            if let Some(preview) = progress.preview.take() {
                self.ai.preview = Some(preview);
                self.ai.preview_dirty = true;
            }
        }
        let Some(result) = job.poll() else {
            return;
        };
        self.ai.job = None;
        match result {
            crate::comfyui::JobResult::Done(image) => {
                if self.workspace.active_doc().id != self.ai.document_id {
                    self.ai.message =
                        "Destination document changed; result was not inserted".into();
                    return;
                }
                self.ai.message = format!(
                    "Result: {} × {} px — accept or regenerate",
                    image.width(),
                    image.height()
                );
                self.ai.preview = Some(image::imageops::thumbnail(&image, 1024, 1024));
                self.ai.preview_dirty = true;
                self.ai.fraction = None;
                self.ai.result = Some(image);
            }
            crate::comfyui::JobResult::Video(bytes) => {
                self.ai.fraction = None;
                if self.ai.mode == AiMode::Animation {
                    let (width, height) = self
                        .workspace
                        .documents
                        .iter()
                        .find(|d| d.id == self.ai.document_id)
                        .map(|d| (d.project.width, d.project.height))
                        .unwrap_or((0, 0));
                    self.ai.message = "Decoding frames…".into();
                    self.ai.frame_decode = Some(crate::video::FrameDecode::start(bytes, width, height));
                    return;
                }
                let stem = match self.ai.mode {
                    AiMode::MinimaxI2V => "Minimax H3 I2V",
                    AiMode::MinimaxR2V => "Minimax H3 R2V",
                    _ => "Ltx 2.3 I2V",
                };
                match self.workspace.add_video_named(bytes, stem) {
                    Ok(index) => {
                        self.dirty = true;
                        self.status =
                            format!("Video saved: videos/{}", self.workspace.videos[index].name);
                        self.ai.message = self.status.clone();
                    }
                    Err(error) => {
                        self.ai.message = error.clone();
                        self.status = error.clone();
                        ui.notify_error(&error);
                    }
                }
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
