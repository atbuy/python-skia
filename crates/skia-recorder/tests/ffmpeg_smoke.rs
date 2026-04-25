use std::fs;
use std::process::Command;

use skia_recorder::{SegmentRing, export_clip, parse_ffmpeg_segment_list};

#[test]
#[ignore = "requires local ffmpeg; run with `cargo test -p skia-recorder --test ffmpeg_smoke -- --ignored`"]
fn ffmpeg_segment_and_export_smoke() {
    let root = std::env::temp_dir().join(format!("skia-ffmpeg-smoke-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create smoke dir");

    let segment_list = root.join("segments.csv");
    let segment_pattern = root.join("segment-%06d.mkv");
    let output = root.join("clip.mp4");

    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30:duration=4",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:duration=4",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-f",
            "segment",
            "-segment_time",
            "2",
            "-reset_timestamps",
            "1",
            "-segment_format",
            "matroska",
            "-segment_list",
            segment_list.to_str().expect("segment list path"),
            "-segment_list_type",
            "csv",
            segment_pattern.to_str().expect("segment pattern path"),
        ])
        .status()
        .expect("run ffmpeg segment smoke");
    assert!(status.success(), "ffmpeg segment generation failed");

    let content = fs::read_to_string(&segment_list).expect("read segment list");
    let segments = parse_ffmpeg_segment_list(&content, &root).expect("parse segment list");
    assert!(!segments.is_empty(), "ffmpeg wrote no segments");

    let mut ring = SegmentRing::new(30, 6);
    ring.replace(segments);

    let selected = ring.select_last(30);
    export_clip(&selected, &output).expect("export clip");
    assert!(output.exists(), "exported clip missing");

    let _ = fs::remove_dir_all(root);
}
