use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

mod backend;
mod export;
mod portal;
mod runtime;
mod segment;

pub use backend::{
    BackendCommandError, FfmpegSegmentConfig, GstreamerSegmentConfig, GstreamerVideoEncoder,
    RecorderProcess, ffmpeg_segment_args, gstreamer_segment_args, parse_ffmpeg_segment_list,
    scan_gstreamer_segments,
};
pub use export::{ExportError, export_clip, ffmpeg_args, write_concat_file};
pub use portal::{PortalError, PortalSession};
pub use runtime::{Platform, RuntimeCheckError, RuntimeChecks, validate_backend};
pub use segment::{Segment, SegmentRing};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    Start {
        id: String,
        config: StartConfig,
    },
    SaveLast {
        id: String,
        seconds: u64,
        output: String,
    },
    Status {
        id: String,
    },
    Check {
        id: String,
    },
    Stop {
        id: String,
    },
}

impl Command {
    fn id(&self) -> &str {
        match self {
            Self::Start { id, .. }
            | Self::SaveLast { id, .. }
            | Self::Status { id }
            | Self::Check { id }
            | Self::Stop { id } => id,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StartConfig {
    pub clip_seconds: u64,
    pub segment_seconds: u64,
    pub backend: BackendSelection,
    #[serde(default)]
    pub cache_dir: Option<String>,
    #[serde(default)]
    pub fps: Option<u32>,
    #[serde(default)]
    pub video_input: Option<String>,
    #[serde(default)]
    pub audio_input: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendSelection {
    Auto,
    LinuxWaylandFfmpeg,
    LinuxWaylandGstreamer,
    LinuxX11Ffmpeg,
    WindowsFfmpeg,
    MacosFfmpeg,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Ready {
        version: &'static str,
    },
    RecordingStarted {
        id: String,
        backend: BackendName,
    },
    ClipSaved {
        id: String,
        path: String,
        duration_seconds: u64,
    },
    Status {
        id: String,
        state: RecorderState,
        backend: Option<BackendName>,
    },
    Check {
        id: String,
        runtime: RuntimeChecks,
    },
    Stopped {
        id: String,
    },
    Error {
        id: Option<String>,
        code: ErrorCode,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecorderState {
    Idle,
    Recording,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendName {
    LinuxWaylandFfmpeg,
    LinuxWaylandGstreamer,
    LinuxX11Ffmpeg,
    WindowsFfmpeg,
    MacosFfmpeg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidCommand,
    AlreadyRecording,
    NotRecording,
    MissingDependency,
    UnsupportedSession,
    CacheUnavailable,
    BackendStartFailed,
    RecorderExited,
    SegmentRefreshFailed,
    NoSegments,
    ExportUnavailable,
    ExportFailed,
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("failed to read command line")]
    Read(#[from] std::io::Error),
    #[error("failed to parse command JSON: {0}")]
    Parse(#[from] serde_json::Error),
}

pub struct RecorderDaemon {
    state: DaemonState,
    runtime: RuntimeChecks,
    processes_enabled: bool,
}

#[derive(Debug, Default)]
struct DaemonState {
    recording: bool,
    backend: Option<BackendName>,
    segments: Option<SegmentRing>,
    segment_source: Option<SegmentSource>,
    process: Option<RecorderProcess>,
    portal_session: Option<PortalSession>,
}

#[derive(Debug, Clone)]
enum SegmentSource {
    FfmpegCsv {
        list: PathBuf,
        cache_dir: PathBuf,
    },
    GstreamerScan {
        cache_dir: PathBuf,
        segment_seconds: u64,
    },
}

impl RecorderDaemon {
    pub fn new() -> Self {
        Self {
            state: DaemonState::default(),
            runtime: RuntimeChecks::detect(),
            processes_enabled: true,
        }
    }

    pub fn with_runtime(runtime: RuntimeChecks) -> Self {
        Self {
            state: DaemonState::default(),
            runtime,
            processes_enabled: false,
        }
    }

    pub fn ready_event(&self) -> Event {
        Event::Ready { version: VERSION }
    }

    pub fn handle_command(&mut self, command: Command) -> Vec<Event> {
        match command {
            Command::Start { id, config } => self.start(id, config),
            Command::SaveLast {
                id,
                seconds,
                output,
            } => self.save_last(id, seconds, output),
            Command::Status { id } => self.status(id),
            Command::Check { id } => vec![self.check(id)],
            Command::Stop { id } => self.stop(id),
        }
    }

    fn start(&mut self, id: String, config: StartConfig) -> Vec<Event> {
        if self.state.recording {
            return error_event(
                id,
                ErrorCode::AlreadyRecording,
                "recorder is already running",
            );
        }

        let backend = select_backend(config.backend, self.runtime);
        if let Err(error) = validate_backend(backend, self.runtime) {
            return error_event(id, runtime_error_code(error), error.message());
        }

        let cache_dir = config
            .cache_dir
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(default_cache_dir);
        if let Err(error) = std::fs::create_dir_all(&cache_dir) {
            return error_event(
                id,
                ErrorCode::CacheUnavailable,
                format!("failed to create cache directory: {error}"),
            );
        }
        if let Err(error) = clean_segment_files(&cache_dir) {
            return error_event(
                id,
                ErrorCode::CacheUnavailable,
                format!("failed to clear stale segments: {error}"),
            );
        }

        let segment_pattern = cache_dir.join("segment-%06d.mkv");

        let portal_session = match self.acquire_portal_session(&id, backend, &config) {
            Ok(session) => session,
            Err(events) => return events,
        };
        let portal_node = portal_session.as_ref().map(|session| session.node_id());
        #[cfg(unix)]
        let portal_fd = portal_session
            .as_ref()
            .map(|session| session.pipe_wire_fd());
        #[cfg(not(unix))]
        let portal_fd: Option<i32> = None;

        let (segment_source, process) = match backend {
            BackendName::LinuxWaylandGstreamer => {
                let source = SegmentSource::GstreamerScan {
                    cache_dir: cache_dir.clone(),
                    segment_seconds: config.segment_seconds,
                };
                let process = match self.spawn_gstreamer(
                    &id,
                    &config,
                    &segment_pattern,
                    portal_node,
                    portal_fd,
                ) {
                    Ok(process) => process,
                    Err(events) => return events,
                };
                (source, process)
            }
            _ => {
                let segment_list = cache_dir.join("segments.csv");
                let source = SegmentSource::FfmpegCsv {
                    list: segment_list.clone(),
                    cache_dir: cache_dir.clone(),
                };
                let process = match self.spawn_ffmpeg(
                    &id,
                    backend,
                    &config,
                    &segment_pattern,
                    &segment_list,
                    portal_node,
                ) {
                    Ok(process) => process,
                    Err(events) => return events,
                };
                (source, process)
            }
        };

        self.state.recording = true;
        self.state.backend = Some(backend);
        self.state.segments = Some(SegmentRing::new(config.clip_seconds, 6));
        self.state.segment_source = Some(segment_source);
        self.state.process = process;
        self.state.portal_session = portal_session;

        vec![Event::RecordingStarted { id, backend }]
    }

    fn acquire_portal_session(
        &self,
        id: &str,
        backend: BackendName,
        config: &StartConfig,
    ) -> Result<Option<PortalSession>, Vec<Event>> {
        if !self.processes_enabled {
            return Ok(None);
        }
        if config.video_input.is_some() {
            return Ok(None);
        }
        let needs_portal = matches!(
            backend,
            BackendName::LinuxWaylandFfmpeg | BackendName::LinuxWaylandGstreamer
        );
        if !needs_portal {
            return Ok(None);
        }

        match PortalSession::acquire() {
            Ok(session) => Ok(Some(session)),
            Err(error) => {
                tracing::error!(error = %error, "failed to acquire Wayland portal stream");
                Err(error_event(
                    id.to_string(),
                    ErrorCode::BackendStartFailed,
                    format!("failed to acquire Wayland PipeWire stream node: {error}"),
                ))
            }
        }
    }

    fn spawn_ffmpeg(
        &self,
        id: &str,
        backend: BackendName,
        config: &StartConfig,
        segment_pattern: &Path,
        segment_list: &Path,
        portal_node: Option<&str>,
    ) -> Result<Option<RecorderProcess>, Vec<Event>> {
        if !self.processes_enabled {
            return Ok(None);
        }

        let ffmpeg_config =
            ffmpeg_config(backend, config, segment_pattern, segment_list, portal_node).map_err(
                |message| error_event(id.to_string(), ErrorCode::BackendStartFailed, message),
            )?;

        let mut process = RecorderProcess::start_ffmpeg(&ffmpeg_config).map_err(|error| {
            error_event(
                id.to_string(),
                ErrorCode::BackendStartFailed,
                error.to_string(),
            )
        })?;

        thread::sleep(Duration::from_millis(250));
        if process.has_exited() {
            let stderr = process.stderr_summary();
            let message = if stderr.is_empty() {
                "ffmpeg backend exited immediately".to_string()
            } else {
                format!("ffmpeg backend exited immediately: {stderr}")
            };
            return Err(error_event(
                id.to_string(),
                ErrorCode::BackendStartFailed,
                message,
            ));
        }

        Ok(Some(process))
    }

    fn spawn_gstreamer(
        &self,
        id: &str,
        config: &StartConfig,
        segment_pattern: &Path,
        portal_node: Option<&str>,
        portal_fd: Option<i32>,
    ) -> Result<Option<RecorderProcess>, Vec<Event>> {
        if !self.processes_enabled {
            return Ok(None);
        }

        let gst_config = gstreamer_config(
            config,
            segment_pattern,
            portal_node,
            portal_fd,
            self.runtime,
        )
        .map_err(|message| error_event(id.to_string(), ErrorCode::BackendStartFailed, message))?;

        let mut process = RecorderProcess::start_gstreamer(&gst_config).map_err(|error| {
            error_event(
                id.to_string(),
                ErrorCode::BackendStartFailed,
                error.to_string(),
            )
        })?;

        thread::sleep(Duration::from_millis(250));
        if process.has_exited() {
            let stderr = process.stderr_summary();
            let message = if stderr.is_empty() {
                "gstreamer pipeline exited immediately".to_string()
            } else {
                format!("gstreamer pipeline exited immediately: {stderr}")
            };
            return Err(error_event(
                id.to_string(),
                ErrorCode::BackendStartFailed,
                message,
            ));
        }

        Ok(Some(process))
    }

    fn save_last(&mut self, id: String, seconds: u64, output: String) -> Vec<Event> {
        if !self.state.recording {
            return vec![Event::Error {
                id: Some(id),
                code: ErrorCode::NotRecording,
                message: "recorder is not running".to_string(),
            }];
        }

        if self.recorder_process_exited() {
            let message = self.recorder_exited_message();
            self.clear_recording_state();
            return vec![Event::Error {
                id: Some(id),
                code: ErrorCode::RecorderExited,
                message,
            }];
        }

        if let Err(error) = self.refresh_segments() {
            return vec![Event::Error {
                id: Some(id),
                code: ErrorCode::SegmentRefreshFailed,
                message: error,
            }];
        }

        let segments = self
            .state
            .segments
            .as_ref()
            .map(|ring| ring.select_last(seconds))
            .unwrap_or_default();

        if segments.is_empty() {
            return vec![Event::Error {
                id: Some(id),
                code: ErrorCode::NoSegments,
                message: "no completed recorded segments are available yet; wait at least one segment duration and verify the backend is producing media".to_string(),
            }];
        }

        tracing::info!(
            seconds,
            output,
            segment_count = segments.len(),
            "exporting clip"
        );

        match export_clip(&segments, Path::new(&output)) {
            Ok(()) => vec![Event::ClipSaved {
                id,
                path: output,
                duration_seconds: seconds,
            }],
            Err(error) => vec![Event::Error {
                id: Some(id),
                code: ErrorCode::ExportFailed,
                message: error.to_string(),
            }],
        }
    }

    fn status(&mut self, id: String) -> Vec<Event> {
        if self.recorder_process_exited() {
            let message = self.recorder_exited_message();
            self.clear_recording_state();
            return vec![Event::Error {
                id: Some(id),
                code: ErrorCode::RecorderExited,
                message,
            }];
        }

        vec![Event::Status {
            id,
            state: if self.state.recording {
                RecorderState::Recording
            } else {
                RecorderState::Idle
            },
            backend: self.state.backend,
        }]
    }

    fn stop(&mut self, id: String) -> Vec<Event> {
        if let Some(process) = self.state.process.as_mut() {
            process.stop();
        }

        self.clear_recording_state();
        vec![Event::Stopped { id }]
    }

    fn check(&self, id: String) -> Event {
        Event::Check {
            id,
            runtime: RuntimeChecks::detect(),
        }
    }

    fn clear_recording_state(&mut self) {
        self.state.recording = false;
        self.state.backend = None;
        self.state.segments = None;
        self.state.segment_source = None;
        self.state.process = None;
        // Drop the portal session AFTER the recorder process has been torn
        // down so the PipeWire stream stays alive while gst/ffmpeg is still
        // shutting down.
        self.state.portal_session = None;
    }

    fn recorder_process_exited(&mut self) -> bool {
        self.state
            .process
            .as_mut()
            .is_some_and(RecorderProcess::has_exited)
    }

    fn recorder_exited_message(&self) -> String {
        let stderr = self
            .state
            .process
            .as_ref()
            .map(RecorderProcess::stderr_summary)
            .unwrap_or_default();

        if stderr.is_empty() {
            "recorder backend process exited unexpectedly".to_string()
        } else {
            format!("recorder backend process exited unexpectedly: {stderr}")
        }
    }

    fn refresh_segments(&mut self) -> Result<(), String> {
        let Some(source) = self.state.segment_source.as_ref() else {
            return Ok(());
        };

        let segments = match source {
            SegmentSource::FfmpegCsv { list, cache_dir } => {
                if !list.exists() {
                    return Ok(());
                }
                let content = std::fs::read_to_string(list)
                    .map_err(|error| format!("failed to read segment list: {error}"))?;
                parse_ffmpeg_segment_list(&content, cache_dir)
                    .map_err(|error| format!("failed to parse segment list: {error}"))?
            }
            SegmentSource::GstreamerScan {
                cache_dir,
                segment_seconds,
            } => scan_gstreamer_segments(cache_dir, *segment_seconds)
                .map_err(|error| format!("failed to scan segments: {error}"))?,
        };

        if let Some(ring) = self.state.segments.as_mut() {
            for segment in ring.replace(segments) {
                if let Err(error) = std::fs::remove_file(&segment.path) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!(
                            path = %segment.path.display(),
                            error = %error,
                            "failed to remove pruned segment"
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

fn error_event(id: String, code: ErrorCode, message: impl Into<String>) -> Vec<Event> {
    vec![Event::Error {
        id: Some(id),
        code,
        message: message.into(),
    }]
}

fn clean_segment_files(cache_dir: &Path) -> std::io::Result<()> {
    let read = match std::fs::read_dir(cache_dir) {
        Ok(read) => read,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    for entry in read.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("segment-") && name.ends_with(".mkv") {
            if let Err(error) = std::fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(error);
                }
            }
        }
    }
    Ok(())
}

pub fn parse_command(line: &str) -> Result<Command, serde_json::Error> {
    serde_json::from_str(line)
}

pub fn write_event(mut writer: impl Write, event: &Event) -> std::io::Result<()> {
    serde_json::to_writer(&mut writer, event)?;
    writer.write_all(b"\n")
}

pub fn run_jsonl(
    input: impl BufRead,
    mut output: impl Write,
    mut daemon: RecorderDaemon,
) -> Result<(), ProtocolError> {
    write_event(&mut output, &daemon.ready_event())?;

    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let command = match parse_command(&line) {
            Ok(command) => command,
            Err(error) => {
                let event = Event::Error {
                    id: None,
                    code: ErrorCode::InvalidCommand,
                    message: error.to_string(),
                };
                write_event(&mut output, &event)?;
                continue;
            }
        };

        tracing::debug!(id = command.id(), "handling command");

        for event in daemon.handle_command(command) {
            write_event(&mut output, &event)?;
        }
    }

    Ok(())
}

fn runtime_error_code(error: RuntimeCheckError) -> ErrorCode {
    match error {
        RuntimeCheckError::MissingFfmpeg
        | RuntimeCheckError::MissingFfmpegDevice(_)
        | RuntimeCheckError::MissingGstreamer
        | RuntimeCheckError::MissingGstreamerElement(_) => ErrorCode::MissingDependency,
        RuntimeCheckError::WaylandUnavailable
        | RuntimeCheckError::X11Unavailable
        | RuntimeCheckError::UnsupportedPlatform => ErrorCode::UnsupportedSession,
    }
}

fn select_backend(selection: BackendSelection, runtime: RuntimeChecks) -> BackendName {
    match selection {
        BackendSelection::Auto => auto_backend(runtime),
        BackendSelection::LinuxWaylandFfmpeg => BackendName::LinuxWaylandFfmpeg,
        BackendSelection::LinuxWaylandGstreamer => BackendName::LinuxWaylandGstreamer,
        BackendSelection::LinuxX11Ffmpeg => BackendName::LinuxX11Ffmpeg,
        BackendSelection::WindowsFfmpeg => BackendName::WindowsFfmpeg,
        BackendSelection::MacosFfmpeg => BackendName::MacosFfmpeg,
    }
}

fn auto_backend(runtime: RuntimeChecks) -> BackendName {
    match runtime.platform {
        Platform::Linux if runtime.wayland_display => {
            if runtime.ffmpeg_pipewire {
                BackendName::LinuxWaylandFfmpeg
            } else {
                BackendName::LinuxWaylandGstreamer
            }
        }
        Platform::Linux => BackendName::LinuxX11Ffmpeg,
        Platform::Windows => BackendName::WindowsFfmpeg,
        Platform::Macos => BackendName::MacosFfmpeg,
        Platform::Other => BackendName::LinuxWaylandFfmpeg,
    }
}

fn default_cache_dir() -> PathBuf {
    std::env::temp_dir().join("skia-recorder")
}

fn ffmpeg_config(
    backend: BackendName,
    config: &StartConfig,
    segment_pattern: &Path,
    segment_list: &Path,
    portal_node: Option<&str>,
) -> Result<FfmpegSegmentConfig, String> {
    let video_input = config
        .video_input
        .clone()
        .or_else(|| portal_node.map(str::to_string))
        .or_else(|| default_video_input(backend))
        .ok_or_else(|| "failed to resolve video input for backend".to_string())?;

    Ok(FfmpegSegmentConfig {
        backend,
        fps: config.fps.unwrap_or(60),
        segment_seconds: config.segment_seconds,
        video_input,
        audio_input: config.audio_input.clone(),
        segment_pattern: segment_pattern.to_path_buf(),
        segment_list: segment_list.to_path_buf(),
    })
}

fn default_video_input(backend: BackendName) -> Option<String> {
    match backend {
        BackendName::LinuxWaylandFfmpeg | BackendName::LinuxWaylandGstreamer => None,
        BackendName::LinuxX11Ffmpeg => {
            Some(std::env::var("DISPLAY").unwrap_or_else(|_| ":0.0".to_string()))
        }
        BackendName::WindowsFfmpeg => Some("desktop".to_string()),
        BackendName::MacosFfmpeg => Some("1".to_string()),
    }
}

fn gstreamer_config(
    config: &StartConfig,
    segment_pattern: &Path,
    portal_node: Option<&str>,
    portal_fd: Option<i32>,
    runtime: RuntimeChecks,
) -> Result<GstreamerSegmentConfig, String> {
    let node_id = config
        .video_input
        .clone()
        .or_else(|| portal_node.map(str::to_string))
        .ok_or_else(|| "failed to resolve PipeWire node id for gstreamer backend".to_string())?;

    let video_encoder = if runtime.gstreamer_nvh264enc {
        GstreamerVideoEncoder::Nvh264
    } else if runtime.gstreamer_vah264enc {
        GstreamerVideoEncoder::Vah264
    } else if runtime.gstreamer_vaapih264enc {
        GstreamerVideoEncoder::Vaapih264
    } else {
        GstreamerVideoEncoder::X264
    };

    Ok(GstreamerSegmentConfig {
        node_id,
        pipe_wire_fd: portal_fd,
        audio_input: config.audio_input.clone(),
        fps: config.fps.unwrap_or(60),
        segment_seconds: config.segment_seconds,
        segment_pattern: segment_pattern.to_path_buf(),
        video_encoder,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_RUNTIME: RuntimeChecks = RuntimeChecks {
        platform: Platform::Linux,
        ffmpeg_available: true,
        ffmpeg_pipewire: true,
        ffmpeg_x11grab: true,
        ffmpeg_gdigrab: true,
        ffmpeg_dshow: true,
        ffmpeg_avfoundation: true,
        gstreamer_available: true,
        gstreamer_pipewiresrc: true,
        gstreamer_videoconvert: true,
        gstreamer_x264enc: true,
        gstreamer_nvh264enc: false,
        gstreamer_vah264enc: false,
        gstreamer_vaapih264enc: false,
        gstreamer_matroskamux: true,
        gstreamer_splitmuxsink: true,
        wayland_display: true,
        x11_display: true,
    };

    #[test]
    fn parses_start_command() {
        let command = parse_command(
            r#"{"id":"1","cmd":"start","config":{"clip_seconds":30,"segment_seconds":2,"backend":"auto"}}"#,
        )
        .expect("valid command");

        assert_eq!(
            command,
            Command::Start {
                id: "1".to_string(),
                config: StartConfig {
                    clip_seconds: 30,
                    segment_seconds: 2,
                    backend: BackendSelection::Auto,
                    cache_dir: None,
                    fps: None,
                    video_input: None,
                    audio_input: None,
                },
            }
        );
    }

    #[test]
    fn serializes_ready_event() {
        let mut output = Vec::new();
        write_event(&mut output, &Event::Ready { version: "0.1.0" }).expect("write event");

        assert_eq!(
            String::from_utf8(output).expect("utf8"),
            r#"{"event":"ready","version":"0.1.0"}"#.to_string() + "\n"
        );
    }

    #[test]
    fn status_starts_idle() {
        let mut daemon = RecorderDaemon::new();
        let events = daemon.handle_command(Command::Status {
            id: "status-1".to_string(),
        });

        assert_eq!(
            events,
            vec![Event::Status {
                id: "status-1".to_string(),
                state: RecorderState::Idle,
                backend: None,
            }]
        );
    }

    #[test]
    fn start_changes_status_to_recording() {
        let mut daemon = RecorderDaemon::with_runtime(TEST_RUNTIME);

        daemon.handle_command(Command::Start {
            id: "start-1".to_string(),
            config: StartConfig {
                clip_seconds: 30,
                segment_seconds: 2,
                backend: BackendSelection::LinuxWaylandFfmpeg,
                cache_dir: None,
                fps: None,
                video_input: None,
                audio_input: None,
            },
        });

        let events = daemon.handle_command(Command::Status {
            id: "status-1".to_string(),
        });

        assert_eq!(
            events,
            vec![Event::Status {
                id: "status-1".to_string(),
                state: RecorderState::Recording,
                backend: Some(BackendName::LinuxWaylandFfmpeg),
            }]
        );
    }

    #[test]
    fn save_last_before_segments_returns_structured_error() {
        let mut daemon = RecorderDaemon::with_runtime(TEST_RUNTIME);
        daemon.handle_command(Command::Start {
            id: "start-1".to_string(),
            config: StartConfig {
                clip_seconds: 30,
                segment_seconds: 2,
                backend: BackendSelection::LinuxWaylandFfmpeg,
                cache_dir: None,
                fps: None,
                video_input: None,
                audio_input: None,
            },
        });

        let events = daemon.handle_command(Command::SaveLast {
            id: "save-1".to_string(),
            seconds: 30,
            output: "/tmp/clip.mp4".to_string(),
        });

        assert_eq!(
            events,
            vec![Event::Error {
                id: Some("save-1".to_string()),
                code: ErrorCode::NoSegments,
                message: "no completed recorded segments are available yet; wait at least one segment duration and verify the backend is producing media".to_string(),
            }]
        );
    }

    #[test]
    fn start_returns_error_when_ffmpeg_is_missing() {
        let mut daemon = RecorderDaemon::with_runtime(RuntimeChecks {
            ffmpeg_available: false,
            ..TEST_RUNTIME
        });

        let events = daemon.handle_command(Command::Start {
            id: "start-1".to_string(),
            config: StartConfig {
                clip_seconds: 30,
                segment_seconds: 2,
                backend: BackendSelection::LinuxWaylandFfmpeg,
                cache_dir: None,
                fps: None,
                video_input: None,
                audio_input: None,
            },
        });

        assert_eq!(
            events,
            vec![Event::Error {
                id: Some("start-1".to_string()),
                code: ErrorCode::MissingDependency,
                message: "ffmpeg is not installed or not available on PATH".to_string(),
            }]
        );
    }

    #[test]
    fn refresh_segments_reads_ffmpeg_segment_list() {
        let cache_dir =
            std::env::temp_dir().join(format!("skia-refresh-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache_dir);
        std::fs::create_dir_all(&cache_dir).expect("create cache dir");
        std::fs::write(
            cache_dir.join("segments.csv"),
            "segment-000000.mkv,0.000000,2.000000\n",
        )
        .expect("write segment list");

        let mut daemon = RecorderDaemon::with_runtime(TEST_RUNTIME);
        daemon.handle_command(Command::Start {
            id: "start-1".to_string(),
            config: StartConfig {
                clip_seconds: 30,
                segment_seconds: 2,
                backend: BackendSelection::LinuxWaylandFfmpeg,
                cache_dir: Some(cache_dir.display().to_string()),
                fps: None,
                video_input: None,
                audio_input: None,
            },
        });

        daemon.refresh_segments().expect("refresh segments");
        let selected = daemon
            .state
            .segments
            .as_ref()
            .expect("ring")
            .select_last(30);

        assert_eq!(
            selected,
            vec![Segment::new(cache_dir.join("segment-000000.mkv"), 0, 2000)]
        );

        let _ = std::fs::remove_dir_all(cache_dir);
    }

    #[test]
    fn auto_backend_prefers_wayland_on_linux_when_available() {
        assert_eq!(
            select_backend(BackendSelection::Auto, TEST_RUNTIME),
            BackendName::LinuxWaylandFfmpeg
        );
    }

    #[test]
    fn auto_backend_falls_back_to_x11_on_linux_without_wayland() {
        let runtime = RuntimeChecks {
            wayland_display: false,
            x11_display: true,
            ..TEST_RUNTIME
        };

        assert_eq!(
            select_backend(BackendSelection::Auto, runtime),
            BackendName::LinuxX11Ffmpeg
        );
    }

    #[test]
    fn auto_backend_picks_gstreamer_on_wayland_without_ffmpeg_pipewire() {
        let runtime = RuntimeChecks {
            ffmpeg_pipewire: false,
            ..TEST_RUNTIME
        };

        assert_eq!(
            select_backend(BackendSelection::Auto, runtime),
            BackendName::LinuxWaylandGstreamer
        );
    }

    #[test]
    fn parses_gstreamer_backend_selection() {
        let command = parse_command(
            r#"{"id":"1","cmd":"start","config":{"clip_seconds":30,"segment_seconds":2,"backend":"linux-wayland-gstreamer"}}"#,
        )
        .expect("valid command");

        assert!(matches!(
            command,
            Command::Start {
                config: StartConfig {
                    backend: BackendSelection::LinuxWaylandGstreamer,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn gstreamer_start_records_scan_source() {
        let cache_dir =
            std::env::temp_dir().join(format!("skia-gst-source-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache_dir);

        let mut daemon = RecorderDaemon::with_runtime(TEST_RUNTIME);
        let events = daemon.handle_command(Command::Start {
            id: "start-1".to_string(),
            config: StartConfig {
                clip_seconds: 30,
                segment_seconds: 2,
                backend: BackendSelection::LinuxWaylandGstreamer,
                cache_dir: Some(cache_dir.display().to_string()),
                fps: Some(30),
                video_input: Some("99".to_string()),
                audio_input: None,
            },
        });

        assert_eq!(
            events,
            vec![Event::RecordingStarted {
                id: "start-1".to_string(),
                backend: BackendName::LinuxWaylandGstreamer,
            }]
        );

        match daemon.state.segment_source.as_ref() {
            Some(SegmentSource::GstreamerScan {
                cache_dir: dir,
                segment_seconds,
            }) => {
                assert_eq!(dir, &cache_dir);
                assert_eq!(*segment_seconds, 2);
            }
            other => panic!("expected GstreamerScan source, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(cache_dir);
    }

    #[test]
    fn gstreamer_refresh_reads_directory_segments() {
        let cache_dir =
            std::env::temp_dir().join(format!("skia-gst-refresh-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache_dir);

        let mut daemon = RecorderDaemon::with_runtime(TEST_RUNTIME);
        daemon.handle_command(Command::Start {
            id: "start-1".to_string(),
            config: StartConfig {
                clip_seconds: 30,
                segment_seconds: 2,
                backend: BackendSelection::LinuxWaylandGstreamer,
                cache_dir: Some(cache_dir.display().to_string()),
                fps: None,
                video_input: Some("99".to_string()),
                audio_input: None,
            },
        });

        for index in 0..3 {
            std::fs::write(
                cache_dir.join(format!("segment-{index:06}.mkv")),
                b"placeholder",
            )
            .expect("write segment");
        }

        daemon.refresh_segments().expect("refresh segments");
        let selected = daemon
            .state
            .segments
            .as_ref()
            .expect("ring")
            .select_last(30);

        assert_eq!(
            selected,
            vec![
                Segment::new(cache_dir.join("segment-000000.mkv"), 0, 2000),
                Segment::new(cache_dir.join("segment-000001.mkv"), 2000, 4000),
            ]
        );

        let _ = std::fs::remove_dir_all(cache_dir);
    }

    #[test]
    fn gstreamer_start_returns_error_when_runtime_missing_element() {
        let runtime = RuntimeChecks {
            gstreamer_pipewiresrc: false,
            ..TEST_RUNTIME
        };
        let mut daemon = RecorderDaemon::with_runtime(runtime);

        let events = daemon.handle_command(Command::Start {
            id: "start-1".to_string(),
            config: StartConfig {
                clip_seconds: 30,
                segment_seconds: 2,
                backend: BackendSelection::LinuxWaylandGstreamer,
                cache_dir: None,
                fps: None,
                video_input: None,
                audio_input: None,
            },
        });

        assert_eq!(
            events,
            vec![Event::Error {
                id: Some("start-1".to_string()),
                code: ErrorCode::MissingDependency,
                message: "gstreamer is missing required element: pipewiresrc".to_string(),
            }]
        );
    }

    #[test]
    fn clean_segment_files_removes_stale_segments_only() {
        let cache_dir =
            std::env::temp_dir().join(format!("skia-clean-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache_dir);
        std::fs::create_dir_all(&cache_dir).expect("create cache dir");
        std::fs::write(cache_dir.join("segment-000000.mkv"), b"x").expect("write seg0");
        std::fs::write(cache_dir.join("segment-000005.mkv"), b"x").expect("write seg5");
        std::fs::write(cache_dir.join("segments.csv"), b"keep").expect("write csv");
        std::fs::write(cache_dir.join("notes.txt"), b"keep").expect("write notes");

        clean_segment_files(&cache_dir).expect("clean");

        assert!(!cache_dir.join("segment-000000.mkv").exists());
        assert!(!cache_dir.join("segment-000005.mkv").exists());
        assert!(cache_dir.join("segments.csv").exists());
        assert!(cache_dir.join("notes.txt").exists());

        let _ = std::fs::remove_dir_all(cache_dir);
    }

    #[test]
    fn jsonl_loop_emits_ready_and_status() {
        let input = br#"{"id":"1","cmd":"status"}
"#;
        let mut output = Vec::new();

        run_jsonl(&input[..], &mut output, RecorderDaemon::new()).expect("run protocol");

        let output = String::from_utf8(output).expect("utf8");
        let lines: Vec<&str> = output.lines().collect();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], r#"{"event":"ready","version":"0.1.0"}"#);
        assert_eq!(
            lines[1],
            r#"{"event":"status","id":"1","state":"idle","backend":null}"#
        );
    }

    #[test]
    fn check_returns_runtime_details() {
        let mut daemon = RecorderDaemon::with_runtime(TEST_RUNTIME);
        let events = daemon.handle_command(Command::Check {
            id: "check-1".to_string(),
        });

        assert_eq!(
            events,
            vec![Event::Check {
                id: "check-1".to_string(),
                runtime: RuntimeChecks::detect(),
            }]
        );
    }
}
