use std::fmt;
use std::path::{Path, PathBuf};

use crate::{BackendName, Segment};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegSegmentConfig {
    pub backend: BackendName,
    pub fps: u32,
    pub segment_seconds: u64,
    pub video_input: String,
    pub audio_input: Option<String>,
    pub segment_pattern: PathBuf,
    pub segment_list: PathBuf,
}

#[derive(Debug)]
pub enum BackendCommandError {
    MissingVideoInput,
    SegmentList(csv::Error),
    InvalidSegmentTime(String),
}

impl fmt::Display for BackendCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVideoInput => write!(formatter, "video input is required"),
            Self::SegmentList(error) => write!(formatter, "{error}"),
            Self::InvalidSegmentTime(value) => {
                write!(formatter, "invalid segment timestamp: {value}")
            }
        }
    }
}

impl std::error::Error for BackendCommandError {}

pub fn ffmpeg_segment_args(
    config: &FfmpegSegmentConfig,
) -> Result<Vec<String>, BackendCommandError> {
    if config.video_input.is_empty() {
        return Err(BackendCommandError::MissingVideoInput);
    }

    let mut args = common_prefix();

    match config.backend {
        BackendName::LinuxWaylandFfmpeg => {
            args.extend([
                "-f".to_string(),
                "pipewire".to_string(),
                "-framerate".to_string(),
                config.fps.to_string(),
                "-i".to_string(),
                config.video_input.clone(),
            ]);
            add_audio_input(&mut args, "pulse", config.audio_input.as_deref());
        }
        BackendName::LinuxX11Ffmpeg => {
            args.extend([
                "-f".to_string(),
                "x11grab".to_string(),
                "-framerate".to_string(),
                config.fps.to_string(),
                "-i".to_string(),
                config.video_input.clone(),
            ]);
            add_audio_input(&mut args, "pulse", config.audio_input.as_deref());
        }
        BackendName::WindowsFfmpeg => {
            args.extend([
                "-f".to_string(),
                "gdigrab".to_string(),
                "-framerate".to_string(),
                config.fps.to_string(),
                "-i".to_string(),
                config.video_input.clone(),
            ]);
            add_audio_input(&mut args, "dshow", config.audio_input.as_deref());
        }
        BackendName::MacosFfmpeg => {
            let input = match config.audio_input.as_deref() {
                Some(audio) => format!("{}:{audio}", config.video_input),
                None => config.video_input.clone(),
            };
            args.extend([
                "-f".to_string(),
                "avfoundation".to_string(),
                "-framerate".to_string(),
                config.fps.to_string(),
                "-i".to_string(),
                input,
            ]);
        }
    }

    args.extend(segment_output_args(config));
    Ok(args)
}

fn common_prefix() -> Vec<String> {
    vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-y".to_string(),
    ]
}

fn add_audio_input(args: &mut Vec<String>, format: &str, input: Option<&str>) {
    if let Some(input) = input {
        args.extend([
            "-f".to_string(),
            format.to_string(),
            "-i".to_string(),
            input.to_string(),
        ]);
    }
}

fn segment_output_args(config: &FfmpegSegmentConfig) -> Vec<String> {
    let mut args = vec!["-map".to_string(), "0:v:0".to_string()];

    if config.audio_input.is_some() {
        args.extend(["-map".to_string(), "1:a:0".to_string()]);
    }

    args.extend([
        "-c:v".to_string(),
        "libx264".to_string(),
        "-preset".to_string(),
        "veryfast".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
    ]);

    if config.audio_input.is_some() {
        args.extend(["-c:a".to_string(), "aac".to_string()]);
    }

    args.extend([
        "-f".to_string(),
        "segment".to_string(),
        "-segment_time".to_string(),
        config.segment_seconds.to_string(),
        "-reset_timestamps".to_string(),
        "1".to_string(),
        "-segment_format".to_string(),
        "matroska".to_string(),
        "-segment_list".to_string(),
        config.segment_list.display().to_string(),
        "-segment_list_type".to_string(),
        "csv".to_string(),
        config.segment_pattern.display().to_string(),
    ]);

    args
}

pub fn parse_ffmpeg_segment_list(
    content: &str,
    base_dir: &Path,
) -> Result<Vec<Segment>, BackendCommandError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(content.as_bytes());
    let mut segments = Vec::new();

    for record in reader.records() {
        let record = record.map_err(BackendCommandError::SegmentList)?;
        if record.len() < 3 {
            continue;
        }

        let path = base_dir.join(&record[0]);
        let start_ms = seconds_to_ms(&record[1])?;
        let end_ms = seconds_to_ms(&record[2])?;

        if end_ms > start_ms {
            segments.push(Segment::new(path, start_ms, end_ms));
        }
    }

    Ok(segments)
}

fn seconds_to_ms(value: &str) -> Result<u64, BackendCommandError> {
    let seconds = value
        .parse::<f64>()
        .map_err(|_| BackendCommandError::InvalidSegmentTime(value.to_string()))?;

    Ok((seconds * 1000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config(backend: BackendName) -> FfmpegSegmentConfig {
        FfmpegSegmentConfig {
            backend,
            fps: 60,
            segment_seconds: 2,
            video_input: "input".to_string(),
            audio_input: Some("default".to_string()),
            segment_pattern: "/tmp/skia/segment-%06d.mkv".into(),
            segment_list: "/tmp/skia/segments.csv".into(),
        }
    }

    #[test]
    fn builds_wayland_pipewire_segment_args() {
        let mut config = base_config(BackendName::LinuxWaylandFfmpeg);
        config.video_input = "42".to_string();

        let args = ffmpeg_segment_args(&config).expect("args");

        assert!(args.windows(2).any(|window| window == ["-f", "pipewire"]));
        assert!(args.windows(2).any(|window| window == ["-i", "42"]));
        assert!(args.windows(2).any(|window| window == ["-f", "segment"]));
        assert!(
            args.windows(2)
                .any(|window| window == ["-segment_list", "/tmp/skia/segments.csv"])
        );
        assert!(args.ends_with(&["/tmp/skia/segment-%06d.mkv".to_string()]));
    }

    #[test]
    fn builds_x11_segment_args() {
        let mut config = base_config(BackendName::LinuxX11Ffmpeg);
        config.video_input = ":0.0".to_string();

        let args = ffmpeg_segment_args(&config).expect("args");

        assert!(args.windows(2).any(|window| window == ["-f", "x11grab"]));
        assert!(args.windows(2).any(|window| window == ["-i", ":0.0"]));
    }

    #[test]
    fn builds_windows_segment_args() {
        let mut config = base_config(BackendName::WindowsFfmpeg);
        config.video_input = "desktop".to_string();

        let args = ffmpeg_segment_args(&config).expect("args");

        assert!(args.windows(2).any(|window| window == ["-f", "gdigrab"]));
        assert!(args.windows(2).any(|window| window == ["-f", "dshow"]));
    }

    #[test]
    fn builds_macos_segment_args() {
        let mut config = base_config(BackendName::MacosFfmpeg);
        config.video_input = "1".to_string();
        config.audio_input = Some("0".to_string());

        let args = ffmpeg_segment_args(&config).expect("args");

        assert!(
            args.windows(2)
                .any(|window| window == ["-f", "avfoundation"])
        );
        assert!(args.windows(2).any(|window| window == ["-i", "1:0"]));
    }

    #[test]
    fn parses_ffmpeg_segment_csv_list() {
        let content = "\
segment-000000.mkv,0.000000,2.000000\n\
segment-000001.mkv,2.000000,4.000000\n";

        let segments = parse_ffmpeg_segment_list(content, Path::new("/tmp/skia")).expect("parse");

        assert_eq!(
            segments,
            vec![
                Segment::new("/tmp/skia/segment-000000.mkv", 0, 2000),
                Segment::new("/tmp/skia/segment-000001.mkv", 2000, 4000),
            ]
        );
    }
}
