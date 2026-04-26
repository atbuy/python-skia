use std::process::Command;

use crate::BackendName;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    Linux,
    Windows,
    Macos,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct RuntimeChecks {
    pub platform: Platform,
    pub ffmpeg_available: bool,
    pub ffmpeg_pipewire: bool,
    pub ffmpeg_x11grab: bool,
    pub ffmpeg_gdigrab: bool,
    pub ffmpeg_dshow: bool,
    pub ffmpeg_avfoundation: bool,
    pub gstreamer_available: bool,
    pub gstreamer_pipewiresrc: bool,
    pub gstreamer_videoconvert: bool,
    pub gstreamer_x264enc: bool,
    pub gstreamer_matroskamux: bool,
    pub gstreamer_splitmuxsink: bool,
    pub wayland_display: bool,
    pub x11_display: bool,
}

impl RuntimeChecks {
    pub fn detect() -> Self {
        let ffmpeg_devices = ffmpeg_devices();
        let gstreamer_available = command_available("gst-launch-1.0", "--version");
        Self {
            platform: detect_platform(),
            ffmpeg_available: command_available("ffmpeg", "-version"),
            ffmpeg_pipewire: ffmpeg_devices.contains(" pipewire"),
            ffmpeg_x11grab: ffmpeg_devices.contains(" x11grab"),
            ffmpeg_gdigrab: ffmpeg_devices.contains(" gdigrab"),
            ffmpeg_dshow: ffmpeg_devices.contains(" dshow"),
            ffmpeg_avfoundation: ffmpeg_devices.contains(" avfoundation"),
            gstreamer_available,
            gstreamer_pipewiresrc: gstreamer_available
                && gstreamer_element_available("pipewiresrc"),
            gstreamer_videoconvert: gstreamer_available
                && gstreamer_element_available("videoconvert"),
            gstreamer_x264enc: gstreamer_available && gstreamer_element_available("x264enc"),
            gstreamer_matroskamux: gstreamer_available
                && gstreamer_element_available("matroskamux"),
            gstreamer_splitmuxsink: gstreamer_available
                && gstreamer_element_available("splitmuxsink"),
            wayland_display: std::env::var_os("WAYLAND_DISPLAY").is_some(),
            x11_display: std::env::var_os("DISPLAY").is_some(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeCheckError {
    MissingFfmpeg,
    MissingFfmpegDevice(&'static str),
    MissingGstreamer,
    MissingGstreamerElement(&'static str),
    WaylandUnavailable,
    X11Unavailable,
    UnsupportedPlatform,
}

impl RuntimeCheckError {
    pub fn message(self) -> String {
        match self {
            Self::MissingFfmpeg => "ffmpeg is not installed or not available on PATH".to_string(),
            Self::MissingFfmpegDevice(device) => {
                format!("ffmpeg build does not support required input device: {device}")
            }
            Self::MissingGstreamer => {
                "gst-launch-1.0 is not installed or not available on PATH".to_string()
            }
            Self::MissingGstreamerElement(element) => {
                format!("gstreamer is missing required element: {element}")
            }
            Self::WaylandUnavailable => "Wayland backend requires WAYLAND_DISPLAY".to_string(),
            Self::X11Unavailable => "X11 backend requires DISPLAY".to_string(),
            Self::UnsupportedPlatform => {
                "selected backend is not supported on this platform".to_string()
            }
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
            if !checks.ffmpeg_pipewire {
                return Err(RuntimeCheckError::MissingFfmpegDevice("pipewire"));
            }
        }
        BackendName::LinuxWaylandGstreamer => {
            if checks.platform != Platform::Linux {
                return Err(RuntimeCheckError::UnsupportedPlatform);
            }
            if !checks.wayland_display {
                return Err(RuntimeCheckError::WaylandUnavailable);
            }
            if !checks.gstreamer_available {
                return Err(RuntimeCheckError::MissingGstreamer);
            }
            if !checks.gstreamer_pipewiresrc {
                return Err(RuntimeCheckError::MissingGstreamerElement("pipewiresrc"));
            }
            if !checks.gstreamer_videoconvert {
                return Err(RuntimeCheckError::MissingGstreamerElement("videoconvert"));
            }
            if !checks.gstreamer_x264enc {
                return Err(RuntimeCheckError::MissingGstreamerElement("x264enc"));
            }
            if !checks.gstreamer_matroskamux {
                return Err(RuntimeCheckError::MissingGstreamerElement("matroskamux"));
            }
            if !checks.gstreamer_splitmuxsink {
                return Err(RuntimeCheckError::MissingGstreamerElement("splitmuxsink"));
            }
        }
        BackendName::LinuxX11Ffmpeg => {
            if checks.platform != Platform::Linux {
                return Err(RuntimeCheckError::UnsupportedPlatform);
            }
            if !checks.x11_display {
                return Err(RuntimeCheckError::X11Unavailable);
            }
            if !checks.ffmpeg_x11grab {
                return Err(RuntimeCheckError::MissingFfmpegDevice("x11grab"));
            }
        }
        BackendName::WindowsFfmpeg => {
            if checks.platform != Platform::Windows {
                return Err(RuntimeCheckError::UnsupportedPlatform);
            }
            if !checks.ffmpeg_gdigrab {
                return Err(RuntimeCheckError::MissingFfmpegDevice("gdigrab"));
            }
            if !checks.ffmpeg_dshow {
                return Err(RuntimeCheckError::MissingFfmpegDevice("dshow"));
            }
        }
        BackendName::MacosFfmpeg => {
            if checks.platform != Platform::Macos {
                return Err(RuntimeCheckError::UnsupportedPlatform);
            }
            if !checks.ffmpeg_avfoundation {
                return Err(RuntimeCheckError::MissingFfmpegDevice("avfoundation"));
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

fn command_available(command: &str, version_flag: &str) -> bool {
    Command::new(command).arg(version_flag).output().is_ok()
}

fn ffmpeg_devices() -> String {
    match Command::new("ffmpeg")
        .args(["-hide_banner", "-devices"])
        .output()
    {
        Ok(output) => String::from_utf8_lossy(&output.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

fn gstreamer_element_available(element: &str) -> bool {
    Command::new("gst-inspect-1.0")
        .arg(element)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINUX_WAYLAND: RuntimeChecks = RuntimeChecks {
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
        gstreamer_matroskamux: true,
        gstreamer_splitmuxsink: true,
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
    fn rejects_missing_pipewire_device() {
        let checks = RuntimeChecks {
            ffmpeg_pipewire: false,
            ..LINUX_WAYLAND
        };

        assert_eq!(
            validate_backend(BackendName::LinuxWaylandFfmpeg, checks),
            Err(RuntimeCheckError::MissingFfmpegDevice("pipewire"))
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

    #[test]
    fn validates_linux_wayland_gstreamer_when_requirements_exist() {
        assert_eq!(
            validate_backend(BackendName::LinuxWaylandGstreamer, LINUX_WAYLAND),
            Ok(())
        );
    }

    #[test]
    fn rejects_missing_gstreamer_binary() {
        let checks = RuntimeChecks {
            gstreamer_available: false,
            ..LINUX_WAYLAND
        };

        assert_eq!(
            validate_backend(BackendName::LinuxWaylandGstreamer, checks),
            Err(RuntimeCheckError::MissingGstreamer)
        );
    }

    #[test]
    fn rejects_missing_gstreamer_pipewiresrc() {
        let checks = RuntimeChecks {
            gstreamer_pipewiresrc: false,
            ..LINUX_WAYLAND
        };

        assert_eq!(
            validate_backend(BackendName::LinuxWaylandGstreamer, checks),
            Err(RuntimeCheckError::MissingGstreamerElement("pipewiresrc"))
        );
    }

    #[test]
    fn rejects_missing_gstreamer_splitmuxsink() {
        let checks = RuntimeChecks {
            gstreamer_splitmuxsink: false,
            ..LINUX_WAYLAND
        };

        assert_eq!(
            validate_backend(BackendName::LinuxWaylandGstreamer, checks),
            Err(RuntimeCheckError::MissingGstreamerElement("splitmuxsink"))
        );
    }

    #[test]
    fn gstreamer_backend_still_requires_ffmpeg_for_export() {
        let checks = RuntimeChecks {
            ffmpeg_available: false,
            ..LINUX_WAYLAND
        };

        assert_eq!(
            validate_backend(BackendName::LinuxWaylandGstreamer, checks),
            Err(RuntimeCheckError::MissingFfmpeg)
        );
    }
}
