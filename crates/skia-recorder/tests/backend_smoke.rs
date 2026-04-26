use std::process::Command;

#[test]
#[ignore = "requires Linux Wayland session with portal-acquired PipeWire node; run manually on Wayland after `python -m skia.main --check`"]
fn wayland_gstreamer_capture_smoke() {
    let node = std::env::var("SKIA_PIPEWIRE_NODE")
        .expect("SKIA_PIPEWIRE_NODE must be set to a PipeWire node id from the Wayland portal");
    let cache = std::env::temp_dir().join(format!("skia-gst-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&cache).expect("create cache dir");

    let pattern = cache.join("segment-%06d.mkv");
    let location = format!("location={}", pattern.display());

    let mut child = Command::new("gst-launch-1.0")
        .args([
            "-e",
            "pipewiresrc",
            &format!("path={node}"),
            "!",
            "queue",
            "!",
            "videoconvert",
            "!",
            "queue",
            "!",
            "x264enc",
            "tune=zerolatency",
            "speed-preset=veryfast",
            "key-int-max=120",
            "!",
            "splitmuxsink",
            &location,
            "max-size-time=2000000000",
            "muxer-factory=matroskamux",
            "send-keyframe-requests=true",
        ])
        .spawn()
        .expect("spawn gst-launch-1.0");

    std::thread::sleep(std::time::Duration::from_secs(5));
    let _ = child.kill();
    let _ = child.wait();

    let mut found = false;
    if let Ok(read) = std::fs::read_dir(&cache) {
        for entry in read.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("segment-") && name.ends_with(".mkv") {
                    found = true;
                    break;
                }
            }
        }
    }

    assert!(found, "gstreamer wayland smoke produced no segment files");
    let _ = std::fs::remove_dir_all(cache);
}

#[test]
#[ignore = "requires Linux X11 session with DISPLAY; run manually on X11"]
fn x11_ffmpeg_capture_smoke() {
    let display = std::env::var("DISPLAY").expect("DISPLAY must be set");
    let output = std::env::temp_dir().join(format!("skia-x11-smoke-{}.mkv", std::process::id()));
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "x11grab",
            "-video_size",
            "640x480",
            "-framerate",
            "10",
            "-t",
            "1",
            "-i",
            &display,
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            output.to_str().expect("output path"),
        ])
        .status()
        .expect("run ffmpeg x11 smoke");

    assert!(status.success(), "x11 ffmpeg smoke failed");
    assert!(output.exists(), "x11 smoke output missing");
    let _ = std::fs::remove_file(output);
}

#[test]
#[ignore = "requires Windows desktop session with FFmpeg gdigrab; run manually on Windows"]
fn windows_gdigrab_capture_smoke() {
    let output =
        std::env::temp_dir().join(format!("skia-windows-smoke-{}.mkv", std::process::id()));
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "gdigrab",
            "-framerate",
            "10",
            "-t",
            "1",
            "-i",
            "desktop",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            output.to_str().expect("output path"),
        ])
        .status()
        .expect("run ffmpeg windows smoke");

    assert!(status.success(), "windows ffmpeg smoke failed");
    assert!(output.exists(), "windows smoke output missing");
    let _ = std::fs::remove_file(output);
}

#[test]
#[ignore = "requires macOS Screen Recording permission and avfoundation device ids; run manually on macOS"]
fn macos_avfoundation_capture_smoke() {
    let output = std::env::temp_dir().join(format!("skia-macos-smoke-{}.mkv", std::process::id()));
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "avfoundation",
            "-framerate",
            "10",
            "-t",
            "1",
            "-i",
            "1",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            output.to_str().expect("output path"),
        ])
        .status()
        .expect("run ffmpeg macos smoke");

    assert!(status.success(), "macos ffmpeg smoke failed");
    assert!(output.exists(), "macos smoke output missing");
    let _ = std::fs::remove_file(output);
}
