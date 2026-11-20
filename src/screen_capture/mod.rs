mod overlay;
mod recorder;

use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
};

pub use recorder::Recording;

pub enum Event {
    Saving,
    Done(Result<Option<Recording>, String>),
}

pub struct Job {
    events: mpsc::Receiver<Event>,
    window: Arc<Mutex<usize>>,
}

impl Job {
    pub fn start(wake: impl Fn() + Send + 'static) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let window = Arc::new(Mutex::new(0));
        let frame_window = Arc::clone(&window);
        thread::Builder::new()
            .name("capture-frame".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    overlay::run(frame_window, || {
                        let _ = sender.send(Event::Saving);
                        wake();
                    })
                }))
                .unwrap_or_else(|_| Err("Screen recorder thread panicked".into()));
                let _ = sender.send(Event::Done(result));
                wake();
            })
            .map_err(|error| format!("Cannot open screen recorder: {error}"))?;
        Ok(Self {
            events: receiver,
            window,
        })
    }

    pub fn show(&self) {
        overlay::show(&self.window);
    }

    pub fn poll(&self) -> Option<Event> {
        match self.events.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Event::Done(Err("Screen recorder disconnected".into())))
            }
        }
    }
}
