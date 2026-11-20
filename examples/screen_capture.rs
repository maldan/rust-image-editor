#[cfg(windows)]
#[path = "../src/screen_capture/recorder.rs"]
mod recorder;

#[cfg(windows)]
fn main() -> Result<(), String> {
    use std::{
        io::{self, BufRead, Write},
        path::{Path, PathBuf},
        sync::mpsc,
        thread,
        time::Duration,
    };

    let mut args = std::env::args().skip(1);
    let usage = "Usage: screen_capture OUTPUT.mp4 X Y WIDTH HEIGHT [SECONDS]";
    let path = PathBuf::from(args.next().ok_or(usage)?);
    let mut area = [0; 4];
    for part in &mut area {
        *part = args.next().ok_or(usage)?.parse().map_err(|_| usage)?;
    }
    let limit = args
        .next()
        .map(|value| {
            value
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or(usage)
        })
        .transpose()?
        .map(Duration::from_secs);
    if args.next().is_some()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
    {
        return Err(usage.into());
    }
    if path.exists() {
        return Err(format!("Output already exists: {}", path.display()));
    }
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetProcessDPIAware() };
    let region = recorder::CaptureRegion::default();
    region.update(area);
    let recorder = recorder::Recorder::start(region)?;
    let (sender, controls) = mpsc::channel();
    thread::spawn(move || {
        for line in io::stdin().lock().lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    println!("Recording. Enter p to pause/resume, q to finish.");
    while !recorder.finishing() && !limit.is_some_and(|limit| recorder.duration() >= limit) {
        match controls.recv_timeout(Duration::from_millis(20)) {
            Ok(command) if command.trim().eq_ignore_ascii_case("p") => {
                println!(
                    "{}",
                    if recorder.toggle_pause() {
                        "Paused."
                    } else {
                        "Recording resumed."
                    }
                );
            }
            Ok(command) if command.trim().eq_ignore_ascii_case("q") => break,
            Err(mpsc::RecvTimeoutError::Disconnected) if limit.is_none() => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => thread::sleep(Duration::from_millis(20)),
            _ => {}
        }
    }
    println!("Finishing MP4...");
    let recording = recorder.finish()?;
    let directory = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut output =
        tempfile::NamedTempFile::new_in(directory).map_err(|error| error.to_string())?;
    output
        .write_all(&recording.bytes)
        .map_err(|error| error.to_string())?;
    output
        .persist_noclobber(&path)
        .map_err(|error| error.to_string())?;
    println!("Saved {}", path.display());
    if let Some(warning) = recording.warning {
        eprintln!("Capture ended early: {warning}");
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This prototype requires Windows.");
}
