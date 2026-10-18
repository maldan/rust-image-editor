use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use super::{
    HostConfig,
    client::{Client, output_from_history},
};
use image::RgbaImage;

#[derive(Default)]
pub struct Progress {
    pub message: String,
    pub fraction: Option<f32>,
    pub preview: Option<RgbaImage>,
}

pub enum JobResult {
    Done(RgbaImage),
    Failed(String),
    Cancelled,
}

enum InputWorkflow {
    Edit(String),
    Lineart(i32),
    DrawCn { settings: super::DrawSettings, control: super::ControlNetSettings },
    Upscale { settings: super::DrawSettings, upscale: super::UpscaleSettings },
    Inpaint {
        settings: super::DrawSettings,
        denoise: f32,
        mask: RgbaImage,
    },
    Redraw {
        settings: super::DrawSettings,
        denoise: f32,
    },
}

enum Request {
    Input {
        pixels: RgbaImage,
        scale: f32,
        workflow: InputWorkflow,
    },
    Draw(super::DrawSettings),
}

pub struct Job {
    receiver: mpsc::Receiver<JobResult>,
    pub progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    remote: Option<Arc<RemoteCancellation>>,
}

struct RemoteCancellation {
    config: HostConfig,
    prompt_id: Arc<Mutex<Option<String>>>,
    stop_ws: Arc<AtomicBool>,
    requested: AtomicBool,
    finished: AtomicBool,
}

impl RemoteCancellation {
    fn stop(&self) -> Result<(),String> {
        // An upload/submission may still be in flight. If no prompt id is known
        // yet, the generation worker retries cleanup once it receives that id.
        let Some(id) = self.prompt_id.lock().unwrap().clone() else { return Ok(()); };
        if self.requested.swap(true,Ordering::Relaxed) { return Ok(()); }
        Client::new(self.config.clone())?.cancel(&id)
    }
}

impl Job {
    #[cfg(test)]
    pub(crate) fn pending() -> (Self,mpsc::Sender<JobResult>) {
        let (sender,receiver) = mpsc::channel();
        (Self {receiver,progress:Arc::new(Mutex::new(Progress::default())),
            cancel:Arc::new(AtomicBool::new(false)),remote:None},sender)
    }

    #[cfg(test)]
    pub(crate) fn with_result(result: JobResult) -> Self {
        let (sender, receiver) = mpsc::channel();
        sender.send(result).ok();
        Self {
            receiver,
            progress: Arc::new(Mutex::new(Progress::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            remote: None,
        }
    }

    pub fn start(config: HostConfig, pixels: RgbaImage, scale: f32, prompt: String) -> Self {
        Self::start_request(
            config,
            Request::Input {
                pixels,
                scale,
                workflow: InputWorkflow::Edit(prompt),
            },
        )
    }

    pub fn start_lineart(config: HostConfig, pixels: RgbaImage, scale: f32, resolution: i32) -> Self {
        Self::start_request(
            config,
            Request::Input { pixels, scale, workflow: InputWorkflow::Lineart(resolution) },
        )
    }

    pub fn start_draw_cn(
        config: HostConfig,
        pixels: RgbaImage,
        scale: f32,
        settings: super::DrawSettings,
        control: super::ControlNetSettings,
    ) -> Self {
        Self::start_request(config, Request::Input {
            pixels, scale, workflow: InputWorkflow::DrawCn { settings, control },
        })
    }

    pub fn start_upscale(
        config: HostConfig, pixels: RgbaImage, scale: f32,
        settings: super::DrawSettings, upscale: super::UpscaleSettings,
    ) -> Self {
        Self::start_request(config, Request::Input {
            pixels, scale, workflow: InputWorkflow::Upscale { settings, upscale },
        })
    }

    pub fn start_draw(config: HostConfig, settings: super::DrawSettings) -> Self {
        Self::start_request(config, Request::Draw(settings))
    }

    pub fn start_redraw(
        config: HostConfig,
        pixels: RgbaImage,
        scale: f32,
        settings: super::DrawSettings,
        denoise: f32,
    ) -> Self {
        Self::start_request(
            config,
            Request::Input {
                pixels,
                scale,
                workflow: InputWorkflow::Redraw { settings, denoise },
            },
        )
    }

    pub fn start_inpaint(
        config: HostConfig,
        pixels: RgbaImage,
        mask: RgbaImage,
        scale: f32,
        settings: super::DrawSettings,
        denoise: f32,
    ) -> Self {
        Self::start_request(
            config,
            Request::Input {
                pixels,
                scale,
                workflow: InputWorkflow::Inpaint {
                    settings,
                    denoise,
                    mask,
                },
            },
        )
    }

    fn start_request(config: HostConfig, request: Request) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(Progress {
            message: "Preparing input…".into(),
            ..Default::default()
        }));
        let worker_cancel = cancel.clone();
        let worker_progress = progress.clone();
        let remote = Arc::new(RemoteCancellation {
            config:config.clone(),prompt_id:Arc::new(Mutex::new(None)),
            stop_ws:Arc::new(AtomicBool::new(false)),requested:AtomicBool::new(false),finished:AtomicBool::new(false),
        });
        let worker_remote = remote.clone();
        std::thread::spawn(move || {
            let result = run(
                config,
                request,
                &worker_cancel,
                worker_progress,
                worker_remote.prompt_id.clone(),
                worker_remote.stop_ws.clone(),
            );
            worker_remote.stop_ws.store(true, Ordering::Relaxed);
            let cancelled = worker_cancel.load(Ordering::Relaxed);
            let result = if cancelled || result.is_err() {
                let remote_error = worker_remote.stop().err();
                if let Some(error) = remote_error {
                    JobResult::Failed(format!(
                        "Local job stopped; remote cancellation could not be confirmed: {error}"
                    ))
                } else if cancelled {
                    JobResult::Cancelled
                } else {
                    JobResult::Failed(result.err().unwrap())
                }
            } else {
                match result {
                    Ok(image) => JobResult::Done(image),
                    Err(error) => JobResult::Failed(error),
                }
            };
            worker_remote.finished.store(true,Ordering::Relaxed);
            let _ = sender.send(result);
        });
        Self {
            receiver,
            progress,
            cancel,
            remote: Some(remote),
        }
    }

    pub fn cancel(&self) {
        if self.cancel.swap(true,Ordering::Relaxed) { return; }
        if let Some(remote) = &self.remote {
            remote.stop_ws.store(true,Ordering::Relaxed);
            if !remote.finished.load(Ordering::Relaxed) {
                let remote = remote.clone();
                // Independent of a stalled upload/history/download request.
                // Neither cancellation nor dropping a Job waits for the server.
                std::thread::spawn(move || {
                    if let Err(error) = remote.stop() {
                        log::warn!("Remote cancellation could not be confirmed: {error}");
                    }
                });
            }
        }
    }

    pub fn is_cancelling(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn poll(&self) -> Option<JobResult> {
        if self.is_cancelling() { return Some(JobResult::Cancelled); }
        match self.receiver.try_recv() {
            Ok(JobResult::Done(_)) if self.is_cancelling() => Some(JobResult::Cancelled),
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(JobResult::Failed(
                "ComfyUI worker stopped unexpectedly".into(),
            )),
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}

fn run(
    config: HostConfig,
    request: Request,
    cancel: &AtomicBool,
    progress: Arc<Mutex<Progress>>,
    prompt_id: Arc<Mutex<Option<String>>>,
    stop_ws: Arc<AtomicBool>,
) -> Result<RgbaImage, String> {
    let client = Client::new(config.clone())?;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let client_id = format!(
        "mega-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    check_cancel(cancel)?;
    // Fresh randomness for every Run/Regenerate; stay within exact JSON integer range.
    let seed = rand::random::<u64>() & ((1_u64 << 53) - 1);
    let (graph, save_node) = match request {
        Request::Draw(settings) => super::workflow::prepare_draw(&settings, seed)?,
        Request::Input {
            pixels,
            scale,
            workflow,
        } => {
            if let InputWorkflow::Lineart(resolution) = &workflow {
                super::workflow::validate_lineart_resolution(*resolution)?;
            }
            if let InputWorkflow::DrawCn { settings, control } = &workflow {
                control.validate(settings)?;
            }
            if let InputWorkflow::Inpaint {
                mask,
                settings,
                denoise,
            } = &workflow
            {
                settings.validate_redraw(*denoise)?;
                if mask.dimensions() != pixels.dimensions() {
                    return Err("Inpaint mask must match the input image size".into());
                }
            }
            let (w, h) = super::scaled_size(pixels.width(), pixels.height(), scale)?;
            if let InputWorkflow::Upscale { settings, upscale } = &workflow {
                upscale.validate(settings)?;
                super::scaled_size(w, h, upscale.scale_by)?;
            }
            check_cancel(cancel)?;
            let pixels = if pixels.dimensions() == (w, h) {
                pixels
            } else {
                image::imageops::resize(&pixels, w, h, image::imageops::FilterType::Lanczos3)
            };
            check_cancel(cancel)?;
            let png = crate::project::encode_png_rgba(pixels.as_raw(), w, h)?;
            progress.lock().unwrap().message = "Uploading input...".into();
            let filename = client.upload(png)?;
            check_cancel(cancel)?;
            match workflow {
                InputWorkflow::Inpaint {
                    settings,
                    denoise,
                    mask,
                } => {
                    let mask = if mask.dimensions() == (w, h) {
                        mask
                    } else {
                        image::imageops::resize(&mask, w, h, image::imageops::FilterType::Nearest)
                    };
                    check_cancel(cancel)?;
                    progress.lock().unwrap().message = "Uploading mask...".into();
                    let png = crate::project::encode_png_rgba(mask.as_raw(), w, h)?;
                    let mask_filename = client.upload_mask(png)?;
                    check_cancel(cancel)?;
                    super::workflow::prepare_inpaint(
                        &filename,
                        &mask_filename,
                        &settings,
                        denoise,
                        seed,
                    )?
                }
                InputWorkflow::Edit(prompt) => super::workflow::prepare(&filename, &prompt, seed)?,
                InputWorkflow::DrawCn { settings, control } => {
                    super::workflow::prepare_draw_cn(&filename, &settings, control, seed)?
                }
                InputWorkflow::Upscale { settings, upscale } => {
                    super::workflow::prepare_upscale(&filename, &settings, upscale, seed)?
                }
                InputWorkflow::Lineart(resolution) => {
                    super::workflow::prepare_lineart(&filename, resolution)?
                }
                InputWorkflow::Redraw { settings, denoise } => {
                    super::workflow::prepare_redraw(&filename, &settings, denoise, seed)?
                }
            }
        }
    };
    super::ws::listen(
        config,
        client_id.clone(),
        prompt_id.clone(),
        stop_ws,
        progress.clone(),
    );
    check_cancel(cancel)?;
    progress.lock().unwrap().message = "Submitting workflow…".into();
    let id = client.prompt(graph, &client_id)?;
    *prompt_id.lock().unwrap() = Some(id.clone());
    progress.lock().unwrap().message = "Generating…".into();
    let started = Instant::now();
    loop {
        check_cancel(cancel)?;
        if started.elapsed() > Duration::from_secs(3600) {
            return Err("ComfyUI generation timed out".into());
        }
        let history = client.history(&id)?;
        if let Some(output) = output_from_history(&history, &id, &save_node)? {
            check_cancel(cancel)?;
            progress.lock().unwrap().message = "Downloading result…".into();
            let bytes = client.download(&output)?;
            check_cancel(cancel)?;
            return super::decode_image(&bytes);
        }
        for _ in 0..10 {
            check_cancel(cancel)?;
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelling_a_pending_job_reports_cancelled_without_worker_response() {
        let (job,_sender) = Job::pending();
        assert!(job.poll().is_none());
        job.cancel();
        assert!(matches!(job.poll(),Some(JobResult::Cancelled)));
    }

    #[test]
    fn cancellation_before_prompt_id_is_known_keeps_late_cleanup_available() {
        let remote = RemoteCancellation {
            config:HostConfig {host:String::new(),ws:String::new(),login:String::new(),password:String::new()},
            prompt_id:Arc::new(Mutex::new(None)),stop_ws:Arc::new(AtomicBool::new(false)),
            requested:AtomicBool::new(false),finished:AtomicBool::new(false),
        };
        assert!(remote.stop().is_ok());
        assert!(!remote.requested.load(Ordering::Relaxed),"an in-flight submission must still be cleaned up once its id arrives");
    }

    #[test]
    fn cancelled_job_discards_late_success() {
        let (tx, rx) = mpsc::channel();
        let job = Job {
            receiver: rx,
            progress: Arc::new(Mutex::new(Progress::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            remote: None,
        };
        job.cancel();
        tx.send(JobResult::Done(RgbaImage::new(1, 1))).ok();
        assert!(matches!(job.poll(), Some(JobResult::Cancelled)));
    }

    #[test]
    #[ignore = "requires configured live ComfyUI; runs the embedded Qwen workflow"]
    fn live_edit_image() {
        let config = HostConfig::from_env().unwrap();
        let mut image = RgbaImage::from_pixel(256, 256, image::Rgba([70, 130, 200, 255]));
        for y in 64..192 {
            for x in 64..192 {
                image.put_pixel(x, y, image::Rgba([220, 150, 60, 255]));
            }
        }
        let job = Job::start(
            config,
            image,
            1.0,
            "Make the orange square green. Keep the blue background.".into(),
        );
        let started = Instant::now();
        let mut previews = 0;
        let mut saw_progress = false;
        loop {
            {
                let mut p = job.progress.lock().unwrap();
                if p.preview.take().is_some() {
                    previews += 1;
                }
                saw_progress |= p.fraction.is_some();
            }
            if let Some(result) = job.poll() {
                match result {
                    JobResult::Done(image) => {
                        eprintln!(
                            "ComfyUI success: {}x{}, previews={previews}, progress={saw_progress}",
                            image.width(),
                            image.height()
                        );
                        image.save("target/comfy-smoke.webp").unwrap();
                        assert!(image.width() > 0 && image.height() > 0);
                        return;
                    }
                    JobResult::Failed(e) => panic!("{e}"),
                    JobResult::Cancelled => panic!("Unexpected cancellation"),
                }
            }
            if started.elapsed() > Duration::from_secs(300) {
                job.cancel();
            }
            assert!(
                started.elapsed() < Duration::from_secs(350),
                "Live test timeout"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    #[ignore = "requires configured live ComfyUI; submits then cancels a Qwen job"]
    fn live_cancel() {
        let job = Job::start(
            HostConfig::from_env().unwrap(),
            RgbaImage::from_pixel(256, 256, image::Rgba([50, 100, 150, 255])),
            1.0,
            "Keep this image unchanged".into(),
        );
        let started = Instant::now();
        let mut requested = false;
        loop {
            if job.progress.lock().unwrap().message == "Generating…" && !requested {
                job.cancel();
                requested = true;
            }
            if let Some(result) = job.poll() {
                match result {
                    JobResult::Cancelled => {
                        assert!(requested);
                        return;
                    }
                    JobResult::Failed(e) => panic!("{e}"),
                    JobResult::Done(_) => panic!("Cancelled result must not be delivered"),
                }
            }
            if started.elapsed() > Duration::from_secs(30) {
                job.cancel();
            }
            assert!(
                started.elapsed() < Duration::from_secs(45),
                "Cancel timed out"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
