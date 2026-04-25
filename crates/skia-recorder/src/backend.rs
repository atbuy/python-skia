use std::fmt;
use std::path::PathBuf;

use crate::BackendName;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegSegmentConfig {
    pub backend: BackendName,
    pub fps: u32,
    pub segment_seconds: u64,
    pub video_input: String,
    pub audio_input: Option<String>,
    pub segment_pattern: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendCommandError {
    MissingVideoInput,
}

impl fmt::Display for BackendCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVideoInput => write!(formatter, "video input is required"),
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
        config.segment_pattern.display().to_string(),
    ]);

    args
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
}
