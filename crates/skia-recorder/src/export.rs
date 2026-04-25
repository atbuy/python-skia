use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Segment;

#[derive(Debug)]
pub enum ExportError {
    EmptySegments,
    Io(std::io::Error),
    FfmpegFailed(String),
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySegments => write!(formatter, "no segments are available to export"),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::FfmpegFailed(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<std::io::Error> for ExportError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn export_clip(segments: &[Segment], output: &Path) -> Result<(), ExportError> {
    if segments.is_empty() {
        return Err(ExportError::EmptySegments);
    }

    let concat_path = concat_file_path(output);
    write_concat_file(segments, &concat_path)?;

    let ffmpeg_output = Command::new("ffmpeg")
        .args(ffmpeg_args(&concat_path, output))
        .output()?;

    let _ = fs::remove_file(&concat_path);

    if ffmpeg_output.status.success() {
        Ok(())
    } else {
        Err(ExportError::FfmpegFailed(
            String::from_utf8_lossy(&ffmpeg_output.stderr)
                .trim()
                .to_string(),
        ))
    }
}

pub fn write_concat_file(segments: &[Segment], path: &Path) -> Result<(), std::io::Error> {
    let mut body = String::from("ffconcat version 1.0\n");
    for segment in segments {
        body.push_str("file '");
        body.push_str(&escape_concat_path(&segment.path));
        body.push_str("'\n");
    }
    fs::write(path, body)
}

pub fn ffmpeg_args(concat_path: &Path, output: &Path) -> Vec<String> {
    vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-y".to_string(),
        "-f".to_string(),
        "concat".to_string(),
        "-safe".to_string(),
        "0".to_string(),
        "-i".to_string(),
        concat_path.display().to_string(),
        "-c".to_string(),
        "copy".to_string(),
        output.display().to_string(),
    ]
}

fn concat_file_path(output: &Path) -> PathBuf {
    let mut path = output.to_path_buf();
    path.set_extension("ffconcat");
    path
}

fn escape_concat_path(path: &Path) -> String {
    path.display().to_string().replace('\'', "'\\''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_args_use_concat_demuxer_and_stream_copy() {
        let args = ffmpeg_args(Path::new("/tmp/skia.ffconcat"), Path::new("/tmp/clip.mp4"));

        assert_eq!(
            args,
            vec![
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                "/tmp/skia.ffconcat",
                "-c",
                "copy",
                "/tmp/clip.mp4",
            ]
        );
    }

    #[test]
    fn write_concat_file_lists_segments() {
        let path = std::env::temp_dir().join(format!(
            "skia-test-{}-{}.ffconcat",
            std::process::id(),
            "segments"
        ));
        let segments = vec![
            Segment::new("/tmp/segment-0.mkv", 0, 2000),
            Segment::new("/tmp/segment-1.mkv", 2000, 4000),
        ];

        write_concat_file(&segments, &path).expect("write concat file");
        let content = fs::read_to_string(&path).expect("read concat file");
        let _ = fs::remove_file(path);

        assert_eq!(
            content,
            "ffconcat version 1.0\nfile '/tmp/segment-0.mkv'\nfile '/tmp/segment-1.mkv'\n"
        );
    }
}
