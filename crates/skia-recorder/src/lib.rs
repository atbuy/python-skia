use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};
use thiserror::Error;

mod backend;
mod export;
mod runtime;
mod segment;

pub use backend::{BackendCommandError, FfmpegSegmentConfig, ffmpeg_segment_args};
pub use export::{ExportError, export_clip, ffmpeg_args, write_concat_file};
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
            | Self::Stop { id } => id,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StartConfig {
    pub clip_seconds: u64,
    pub segment_seconds: u64,
    pub backend: BackendSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendSelection {
    Auto,
    LinuxWaylandFfmpeg,
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
}

#[derive(Debug, Default)]
struct DaemonState {
    recording: bool,
    backend: Option<BackendName>,
    segments: Option<SegmentRing>,
}

impl RecorderDaemon {
    pub fn new() -> Self {
        Self {
            state: DaemonState::default(),
            runtime: RuntimeChecks::detect(),
        }
    }

    pub fn with_runtime(runtime: RuntimeChecks) -> Self {
        Self {
            state: DaemonState::default(),
            runtime,
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
            Command::Status { id } => vec![self.status(id)],
            Command::Stop { id } => self.stop(id),
        }
    }

    fn start(&mut self, id: String, config: StartConfig) -> Vec<Event> {
        if self.state.recording {
            return vec![Event::Error {
                id: Some(id),
                code: ErrorCode::AlreadyRecording,
                message: "recorder is already running".to_string(),
            }];
        }

        let backend = select_backend(config.backend, self.runtime);
        if let Err(error) = validate_backend(backend, self.runtime) {
            return vec![Event::Error {
                id: Some(id),
                code: runtime_error_code(error),
                message: error.message().to_string(),
            }];
        }

        self.state.recording = true;
        self.state.backend = Some(backend);
        self.state.segments = Some(SegmentRing::new(config.clip_seconds, 6));

        vec![Event::RecordingStarted { id, backend }]
    }

    fn save_last(&self, id: String, seconds: u64, output: String) -> Vec<Event> {
        if !self.state.recording {
            return vec![Event::Error {
                id: Some(id),
                code: ErrorCode::NotRecording,
                message: "recorder is not running".to_string(),
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
                message: "no recorded segments are available yet".to_string(),
            }];
        }

        tracing::info!(
            seconds,
            output,
            segment_count = segments.len(),
            "exporting clip"
        );

        match export_clip(&segments, std::path::Path::new(&output)) {
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

    fn status(&self, id: String) -> Event {
        Event::Status {
            id,
            state: if self.state.recording {
                RecorderState::Recording
            } else {
                RecorderState::Idle
            },
            backend: self.state.backend,
        }
    }

    fn stop(&mut self, id: String) -> Vec<Event> {
        self.state.recording = false;
        self.state.backend = None;
        self.state.segments = None;
        vec![Event::Stopped { id }]
    }
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
        RuntimeCheckError::MissingFfmpeg => ErrorCode::MissingDependency,
        RuntimeCheckError::WaylandUnavailable
        | RuntimeCheckError::X11Unavailable
        | RuntimeCheckError::UnsupportedPlatform => ErrorCode::UnsupportedSession,
    }
}

fn select_backend(selection: BackendSelection, runtime: RuntimeChecks) -> BackendName {
    match selection {
        BackendSelection::Auto => auto_backend(runtime),
        BackendSelection::LinuxWaylandFfmpeg => BackendName::LinuxWaylandFfmpeg,
        BackendSelection::LinuxX11Ffmpeg => BackendName::LinuxX11Ffmpeg,
        BackendSelection::WindowsFfmpeg => BackendName::WindowsFfmpeg,
        BackendSelection::MacosFfmpeg => BackendName::MacosFfmpeg,
    }
}

fn auto_backend(runtime: RuntimeChecks) -> BackendName {
    match runtime.platform {
        Platform::Linux if runtime.wayland_display => BackendName::LinuxWaylandFfmpeg,
        Platform::Linux => BackendName::LinuxX11Ffmpeg,
        Platform::Windows => BackendName::WindowsFfmpeg,
        Platform::Macos => BackendName::MacosFfmpeg,
        Platform::Other => BackendName::LinuxWaylandFfmpeg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_RUNTIME: RuntimeChecks = RuntimeChecks {
        platform: Platform::Linux,
        ffmpeg_available: true,
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
                message: "no recorded segments are available yet".to_string(),
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
}
