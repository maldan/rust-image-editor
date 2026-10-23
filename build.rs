use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = PathBuf::from(
        env::var_os("FFMPEG_DIR")
            .expect("Run scripts/setup-video.ps1 or set FFMPEG_DIR to an FFmpeg shared SDK"),
    );
    let profile = PathBuf::from(env::var_os("OUT_DIR").unwrap())
        .ancestors()
        .nth(3)
        .unwrap()
        .to_path_buf();
    // Stage DLLs next to both the application and Cargo's test executables.
    for entry in fs::read_dir(root.join("bin")).expect("FFmpeg SDK bin directory is missing") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("dll") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy();
        // avcodec also imports swresample in the pinned SDK. No filter/device DLLs are needed.
        if ![
            "avcodec-",
            "avformat-",
            "avutil-",
            "swscale-",
            "swresample-",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        for destination in [&profile, &profile.join("deps")] {
            fs::create_dir_all(destination).unwrap();
            let target = destination.join(path.file_name().unwrap());
            if !target.exists() || fs::read(&target).unwrap() != fs::read(&path).unwrap() {
                fs::copy(&path, target).expect("Cannot stage FFmpeg DLL next to the executable");
            }
        }
    }
    let license = root.join("LICENSE.txt");
    if license.is_file() {
        println!("cargo:rerun-if-changed={}", license.display());
        fs::copy(license, profile.join("FFMPEG-LICENSE.txt")).expect("Cannot stage FFmpeg license");
    }
}
