use std::fmt;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GstreamerSegmentConfig {
    pub node_id: String,
    /// File descriptor returned by xdg-desktop-portal `OpenPipeWireRemote`.
    /// `pipewiresrc` needs this to talk to the portal's private PipeWire
    /// instance; without it the node id is only valid on the user's default
    /// PipeWire socket, which the portal does not publish to.
    pub pipe_wire_fd: Option<i32>,
    pub fps: u32,
    pub segment_seconds: u64,
    pub segment_pattern: PathBuf,
}

#[derive(Debug)]
pub enum BackendCommandError {
    MissingVideoInput,
    Spawn(std::io::Error),
    SegmentList(csv::Error),
    InvalidSegmentTime(String),
    ScanFailed(std::io::Error),
}

impl fmt::Display for BackendCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVideoInput => write!(formatter, "video input is required"),
            Self::Spawn(error) => write!(formatter, "failed to start backend process: {error}"),
            Self::SegmentList(error) => write!(formatter, "{error}"),
            Self::InvalidSegmentTime(value) => {
                write!(formatter, "invalid segment timestamp: {value}")
            }
            Self::ScanFailed(error) => {
                write!(formatter, "failed to scan segment directory: {error}")
            }
        }
    }
}

impl std::error::Error for BackendCommandError {}

#[derive(Debug)]
pub struct RecorderProcess {
    child: Child,
    stderr_tail: Arc<Mutex<Vec<String>>>,
}

#[derive(Debug, Clone, Copy)]
enum LogSource {
    Ffmpeg,
    Gstreamer,
}

impl RecorderProcess {
    pub fn start_ffmpeg(config: &FfmpegSegmentConfig) -> Result<Self, BackendCommandError> {
        let args = ffmpeg_segment_args(config)?;
        Self::spawn("ffmpeg", &args, LogSource::Ffmpeg)
    }

    pub fn start_gstreamer(config: &GstreamerSegmentConfig) -> Result<Self, BackendCommandError> {
        let args = gstreamer_segment_args(config)?;
        let mut command = Command::new("gst-launch-1.0");
        command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        #[cfg(unix)]
        if let Some(fd) = config.pipe_wire_fd {
            use std::os::unix::process::CommandExt;
            // The portal returns the PipeWire remote fd with FD_CLOEXEC set
            // (default for dbus-passed fds). Clear it so the spawned
            // gst-launch process inherits the fd across exec.
            unsafe {
                command.pre_exec(move || {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }

        Self::spawn_command(command, LogSource::Gstreamer)
    }

    fn spawn(
        program: &str,
        args: &[String],
        source: LogSource,
    ) -> Result<Self, BackendCommandError> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        Self::spawn_command(command, source)
    }

    fn spawn_command(mut command: Command, source: LogSource) -> Result<Self, BackendCommandError> {
        let mut child = command.spawn().map_err(BackendCommandError::Spawn)?;

        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        if let Some(stderr) = child.stderr.take() {
            let stderr_tail = Arc::clone(&stderr_tail);
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if let Ok(mut tail) = stderr_tail.lock() {
                        tail.push(line.clone());
                        if tail.len() > 8 {
                            tail.remove(0);
                        }
                    }
                    match source {
                        LogSource::Ffmpeg => {
                            tracing::error!(target: "skia_recorder::ffmpeg", "{line}");
                        }
                        LogSource::Gstreamer => {
                            tracing::error!(target: "skia_recorder::gstreamer", "{line}");
                        }
                    }
                }
            });
        }

        Ok(Self { child, stderr_tail })
    }

    pub fn has_exited(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_some()
    }

    pub fn stderr_summary(&self) -> String {
        self.stderr_tail
            .lock()
            .map(|tail| tail.join("\n"))
            .unwrap_or_default()
    }

    pub fn stop(&mut self) {
        if self.has_exited() {
            return;
        }

        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for RecorderProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn ffmpeg_segment_args(
    config: &FfmpegSegmentConfig,
) -> Result<Vec<String>, BackendCommandError> {
    if config.video_input.is_empty() {
        return Err(BackendCommandError::MissingVideoInput);
    }

    let mut args = common_prefix();

    match config.backend {
        BackendName::LinuxWaylandGstreamer => {
            unreachable!("ffmpeg_segment_args called with gstreamer backend");
        }
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

pub fn gstreamer_segment_args(
    config: &GstreamerSegmentConfig,
) -> Result<Vec<String>, BackendCommandError> {
    if config.node_id.is_empty() {
        return Err(BackendCommandError::MissingVideoInput);
    }

    let segment_ns = config.segment_seconds.saturating_mul(1_000_000_000);
    let key_int_max = (config.fps as u64).saturating_mul(config.segment_seconds.max(1));

    let mut args = vec!["-e".to_string(), "pipewiresrc".to_string()];
    if let Some(fd) = config.pipe_wire_fd {
        args.push(format!("fd={fd}"));
    }
    args.extend([
        format!("path={}", config.node_id),
        "!".to_string(),
        "queue".to_string(),
        "!".to_string(),
        "videoconvert".to_string(),
        "!".to_string(),
        "queue".to_string(),
        "!".to_string(),
        "x264enc".to_string(),
        "tune=zerolatency".to_string(),
        "speed-preset=veryfast".to_string(),
        format!("key-int-max={}", key_int_max),
        "!".to_string(),
        "splitmuxsink".to_string(),
        format!("location={}", config.segment_pattern.display()),
        format!("max-size-time={}", segment_ns),
        "muxer-factory=matroskamux".to_string(),
        "send-keyframe-requests=true".to_string(),
    ]);
    Ok(args)
}

pub fn scan_gstreamer_segments(
    cache_dir: &Path,
    segment_seconds: u64,
) -> Result<Vec<Segment>, BackendCommandError> {
    let read = match std::fs::read_dir(cache_dir) {
        Ok(read) => read,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(BackendCommandError::ScanFailed(error)),
    };

    let mut entries: Vec<(u64, PathBuf)> = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(index) = parse_segment_index(name) else {
            continue;
        };
        entries.push((index, path));
    }

    entries.sort_by_key(|(index, _)| *index);
    // The highest-indexed file is the one splitmuxsink is currently writing.
    // Exclude it: it may be missing the matroska tail and is unsafe to concat.
    entries.pop();

    if segment_seconds == 0 {
        return Ok(Vec::new());
    }

    let segment_ms = segment_seconds * 1000;
    Ok(entries
        .into_iter()
        .map(|(index, path)| {
            let start = index * segment_ms;
            let end = start + segment_ms;
            Segment::new(path, start, end)
        })
        .collect())
}

fn parse_segment_index(name: &str) -> Option<u64> {
    let stem = name.strip_suffix(".mkv")?;
    let digits = stem.strip_prefix("segment-")?;
    digits.parse().ok()
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

    fn gst_config() -> GstreamerSegmentConfig {
        GstreamerSegmentConfig {
            node_id: "42".to_string(),
            pipe_wire_fd: None,
            fps: 60,
            segment_seconds: 2,
            segment_pattern: "/tmp/skia/segment-%06d.mkv".into(),
        }
    }

    #[test]
    fn builds_gstreamer_pipewire_segment_args() {
        let args = gstreamer_segment_args(&gst_config()).expect("args");

        assert_eq!(args.first().map(String::as_str), Some("-e"));
        assert!(args.iter().any(|arg| arg == "pipewiresrc"));
        assert!(args.iter().any(|arg| arg == "path=42"));
        assert!(args.iter().any(|arg| arg == "videoconvert"));
        assert!(args.iter().any(|arg| arg == "x264enc"));
        assert!(args.iter().any(|arg| arg == "splitmuxsink"));
        assert!(args.iter().any(|arg| arg == "muxer-factory=matroskamux"));
        assert!(
            args.iter()
                .any(|arg| arg == "location=/tmp/skia/segment-%06d.mkv")
        );
        assert!(args.iter().any(|arg| arg == "max-size-time=2000000000"));
        assert!(args.iter().any(|arg| arg == "key-int-max=120"));
        assert!(args.iter().any(|arg| arg == "send-keyframe-requests=true"));
    }

    #[test]
    fn includes_pipe_wire_fd_when_provided() {
        let mut config = gst_config();
        config.pipe_wire_fd = Some(7);

        let args = gstreamer_segment_args(&config).expect("args");

        assert!(args.iter().any(|arg| arg == "fd=7"));
        let fd_index = args.iter().position(|arg| arg == "fd=7").expect("fd arg");
        let path_index = args
            .iter()
            .position(|arg| arg == "path=42")
            .expect("path arg");
        assert!(fd_index < path_index, "fd must precede path on pipewiresrc");
    }

    #[test]
    fn omits_fd_when_not_provided() {
        let args = gstreamer_segment_args(&gst_config()).expect("args");
        assert!(!args.iter().any(|arg| arg.starts_with("fd=")));
    }

    #[test]
    fn rejects_gstreamer_without_node_id() {
        let mut config = gst_config();
        config.node_id = String::new();

        assert!(matches!(
            gstreamer_segment_args(&config),
            Err(BackendCommandError::MissingVideoInput)
        ));
    }

    #[test]
    fn scan_gstreamer_segments_drops_in_progress_tail() {
        let cache_dir = std::env::temp_dir().join(format!("skia-scan-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache_dir);
        std::fs::create_dir_all(&cache_dir).expect("create cache dir");
        for index in 0..3 {
            std::fs::write(
                cache_dir.join(format!("segment-{index:06}.mkv")),
                b"placeholder",
            )
            .expect("write segment");
        }
        std::fs::write(cache_dir.join("notes.txt"), b"ignored").expect("write extra");

        let segments = scan_gstreamer_segments(&cache_dir, 2).expect("scan");

        assert_eq!(
            segments,
            vec![
                Segment::new(cache_dir.join("segment-000000.mkv"), 0, 2000),
                Segment::new(cache_dir.join("segment-000001.mkv"), 2000, 4000),
            ]
        );

        let _ = std::fs::remove_dir_all(cache_dir);
    }

    #[test]
    fn scan_gstreamer_segments_returns_empty_when_only_tail_exists() {
        let cache_dir =
            std::env::temp_dir().join(format!("skia-scan-test-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache_dir);
        std::fs::create_dir_all(&cache_dir).expect("create cache dir");
        std::fs::write(cache_dir.join("segment-000000.mkv"), b"x").expect("write");

        let segments = scan_gstreamer_segments(&cache_dir, 2).expect("scan");
        assert!(segments.is_empty());

        let _ = std::fs::remove_dir_all(cache_dir);
    }

    #[test]
    fn scan_gstreamer_segments_returns_empty_when_directory_missing() {
        let cache_dir =
            std::env::temp_dir().join(format!("skia-scan-test-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache_dir);

        let segments = scan_gstreamer_segments(&cache_dir, 2).expect("scan");
        assert!(segments.is_empty());
    }
}
