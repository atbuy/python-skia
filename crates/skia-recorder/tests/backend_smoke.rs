use std::process::Command;

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
