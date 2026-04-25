use std::process::Command;

use crate::BackendName;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
    Macos,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeChecks {
    pub platform: Platform,
    pub ffmpeg_available: bool,
    pub wayland_display: bool,
    pub x11_display: bool,
}

impl RuntimeChecks {
    pub fn detect() -> Self {
        Self {
            platform: detect_platform(),
            ffmpeg_available: command_available("ffmpeg"),
            wayland_display: std::env::var_os("WAYLAND_DISPLAY").is_some(),
            x11_display: std::env::var_os("DISPLAY").is_some(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeCheckError {
    MissingFfmpeg,
    WaylandUnavailable,
    X11Unavailable,
    UnsupportedPlatform,
}

impl RuntimeCheckError {
    pub fn message(self) -> &'static str {
        match self {
            Self::MissingFfmpeg => "ffmpeg is not installed or not available on PATH",
            Self::WaylandUnavailable => "Wayland backend requires WAYLAND_DISPLAY",
            Self::X11Unavailable => "X11 backend requires DISPLAY",
            Self::UnsupportedPlatform => "selected backend is not supported on this platform",
        }
    }
}

pub fn validate_backend(
    backend: BackendName,
    checks: RuntimeChecks,
) -> Result<(), RuntimeCheckError> {
    if !checks.ffmpeg_available {
        return Err(RuntimeCheckError::MissingFfmpeg);
    }

    match backend {
        BackendName::LinuxWaylandFfmpeg => {
            if checks.platform != Platform::Linux {
                return Err(RuntimeCheckError::UnsupportedPlatform);
            }
            if !checks.wayland_display {
                return Err(RuntimeCheckError::WaylandUnavailable);
            }
        }
        BackendName::LinuxX11Ffmpeg => {
            if checks.platform != Platform::Linux {
                return Err(RuntimeCheckError::UnsupportedPlatform);
            }
            if !checks.x11_display {
                return Err(RuntimeCheckError::X11Unavailable);
            }
        }
        BackendName::WindowsFfmpeg => {
            if checks.platform != Platform::Windows {
                return Err(RuntimeCheckError::UnsupportedPlatform);
            }
        }
        BackendName::MacosFfmpeg => {
            if checks.platform != Platform::Macos {
                return Err(RuntimeCheckError::UnsupportedPlatform);
            }
        }
    }

    Ok(())
}

fn detect_platform() -> Platform {
    if cfg!(target_os = "linux") {
        Platform::Linux
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::Macos
    } else {
        Platform::Other
    }
}

fn command_available(command: &str) -> bool {
    Command::new(command).arg("-version").output().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINUX_WAYLAND: RuntimeChecks = RuntimeChecks {
        platform: Platform::Linux,
        ffmpeg_available: true,
        wayland_display: true,
        x11_display: false,
    };

    #[test]
    fn validates_linux_wayland_when_requirements_exist() {
        assert_eq!(
            validate_backend(BackendName::LinuxWaylandFfmpeg, LINUX_WAYLAND),
            Ok(())
        );
    }

    #[test]
    fn rejects_missing_ffmpeg() {
        let checks = RuntimeChecks {
            ffmpeg_available: false,
            ..LINUX_WAYLAND
        };

        assert_eq!(
            validate_backend(BackendName::LinuxWaylandFfmpeg, checks),
            Err(RuntimeCheckError::MissingFfmpeg)
        );
    }

    #[test]
    fn rejects_missing_wayland_display() {
        let checks = RuntimeChecks {
            wayland_display: false,
            ..LINUX_WAYLAND
        };

        assert_eq!(
            validate_backend(BackendName::LinuxWaylandFfmpeg, checks),
            Err(RuntimeCheckError::WaylandUnavailable)
        );
    }

    #[test]
    fn rejects_wrong_platform() {
        let checks = RuntimeChecks {
            platform: Platform::Windows,
            ..LINUX_WAYLAND
        };

        assert_eq!(
            validate_backend(BackendName::LinuxWaylandFfmpeg, checks),
            Err(RuntimeCheckError::UnsupportedPlatform)
        );
    }
}
