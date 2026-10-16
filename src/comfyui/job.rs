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

pub struct Job {
    receiver: mpsc::Receiver<JobResult>,
    pub progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    #[cfg(test)]
    pub(crate) fn with_result(result: JobResult) -> Self {
        let (sender, receiver) = mpsc::channel();
        sender.send(result).ok();
        Self { receiver, progress: Arc::new(Mutex::new(Progress::default())), cancel: Arc::new(AtomicBool::new(false)) }
    }

    pub fn start(config: HostConfig, pixels: RgbaImage, scale: f32, prompt: String) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(Progress {
            message: "Preparing input…".into(),
            ..Default::default()
        }));
        let worker_cancel = cancel.clone();
        let worker_progress = progress.clone();
        std::thread::spawn(move || {
            let prompt_id = Arc::new(Mutex::new(None));
            let stop_ws = Arc::new(AtomicBool::new(false));
            let result = run(
                config.clone(),
                pixels,
                scale,
                prompt,
                &worker_cancel,
                worker_progress,
                prompt_id.clone(),
                stop_ws.clone(),
            );
            stop_ws.store(true, Ordering::Relaxed);
            let cancelled = worker_cancel.load(Ordering::Relaxed);
            let result = if cancelled || result.is_err() {
                let id = prompt_id.lock().unwrap().clone();
                let remote_error =
                    id.and_then(|id| Client::new(config).and_then(|c| c.cancel(&id)).err());
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
            let _ = sender.send(result);
        });
        Self {
            receiver,
            progress,
            cancel,
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelling(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn poll(&self) -> Option<JobResult> {
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
    pixels: RgbaImage,
    scale: f32,
    prompt: String,
    cancel: &AtomicBool,
    progress: Arc<Mutex<Progress>>,
    prompt_id: Arc<Mutex<Option<String>>>,
    stop_ws: Arc<AtomicBool>,
) -> Result<RgbaImage, String> {
    let client = Client::new(config.clone())?;
    let (w, h) = super::scaled_size(pixels.width(), pixels.height(), scale)?;
    check_cancel(cancel)?;
    let pixels = if pixels.dimensions() == (w, h) {
        pixels
    } else {
        image::imageops::resize(&pixels, w, h, image::imageops::FilterType::Lanczos3)
    };
    check_cancel(cancel)?;
    let png = crate::project::encode_png_rgba(pixels.as_raw(), w, h)?;
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
    progress.lock().unwrap().message = "Uploading input…".into();
    let filename = client.upload(png, &format!("{client_id}.png"))?;
    check_cancel(cancel)?;
    // Fresh randomness for every Run/Regenerate; stay within exact JSON integer range.
    let seed = rand::random::<u64>() & ((1_u64 << 53) - 1);
    let (graph, save_node) = super::workflow::prepare(&filename, &prompt, seed)?;
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
    fn cancelled_job_discards_late_success() {
        let (tx, rx) = mpsc::channel();
        let job = Job {
            receiver: rx,
            progress: Arc::new(Mutex::new(Progress::default())),
            cancel: Arc::new(AtomicBool::new(false)),
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
