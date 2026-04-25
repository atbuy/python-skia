use std::fmt;

#[derive(Debug)]
pub enum PortalError {
    Session(String),
    NoStreams,
}

impl fmt::Display for PortalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(message) => write!(formatter, "{message}"),
            Self::NoStreams => write!(formatter, "portal did not return a PipeWire stream"),
        }
    }
}

impl std::error::Error for PortalError {}

#[cfg(target_os = "linux")]
pub fn acquire_wayland_pipewire_node() -> Result<String, PortalError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| PortalError::Session(error.to_string()))?;

    runtime.block_on(acquire_wayland_pipewire_node_async())
}

#[cfg(not(target_os = "linux"))]
pub fn acquire_wayland_pipewire_node() -> Result<String, PortalError> {
    Err(PortalError::Session(
        "Wayland portal capture is only available on Linux".to_string(),
    ))
}

#[cfg(target_os = "linux")]
async fn acquire_wayland_pipewire_node_async() -> Result<String, PortalError> {
    use ashpd::desktop::{
        PersistMode,
        screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
    };

    let proxy = Screencast::new()
        .await
        .map_err(|error| PortalError::Session(error.to_string()))?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|error| PortalError::Session(error.to_string()))?;

    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::Metadata)
                .set_sources(SourceType::Monitor | SourceType::Window)
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(|error| PortalError::Session(error.to_string()))?;

    let response = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(|error| PortalError::Session(error.to_string()))?
        .response()
        .map_err(|error| PortalError::Session(error.to_string()))?;

    response
        .streams()
        .first()
        .map(|stream| stream.pipe_wire_node_id().to_string())
        .ok_or(PortalError::NoStreams)
}
