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
mod imp {
    use std::os::fd::{AsRawFd, OwnedFd, RawFd};
    use std::sync::mpsc;
    use std::thread::{self, JoinHandle};

    use tokio::sync::oneshot;

    use super::PortalError;

    /// Holds an xdg-desktop-portal screencast session alive on a dedicated
    /// thread for the duration of recording.
    ///
    /// The portal closes the PipeWire stream when its `Session` proxy is
    /// dropped, so consumers must keep this struct alive while the captured
    /// node id is in use. The owned PipeWire remote fd must also stay open;
    /// `pipewiresrc` (and equivalent ffmpeg device support) connects through
    /// it because the portal exposes the screencast node on a private
    /// PipeWire instance, not the user's default socket.
    pub struct PortalSession {
        node_id: String,
        pipe_wire_fd: OwnedFd,
        shutdown: Option<oneshot::Sender<()>>,
        thread: Option<JoinHandle<()>>,
    }

    struct AcquiredStream {
        node_id: String,
        fd: OwnedFd,
    }

    impl PortalSession {
        pub fn acquire() -> Result<Self, PortalError> {
            let (stream_tx, stream_rx) = mpsc::channel::<Result<AcquiredStream, PortalError>>();
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

            let thread = thread::Builder::new()
                .name("skia-portal".to_string())
                .spawn(move || {
                    let runtime = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            let _ = stream_tx.send(Err(PortalError::Session(error.to_string())));
                            return;
                        }
                    };
                    runtime.block_on(portal_session_main(stream_tx, shutdown_rx));
                })
                .map_err(|error| PortalError::Session(error.to_string()))?;

            let stream = match stream_rx.recv() {
                Ok(result) => result?,
                Err(_) => {
                    let _ = thread.join();
                    return Err(PortalError::Session(
                        "portal worker thread exited before sending stream details".to_string(),
                    ));
                }
            };

            Ok(Self {
                node_id: stream.node_id,
                pipe_wire_fd: stream.fd,
                shutdown: Some(shutdown_tx),
                thread: Some(thread),
            })
        }

        pub fn node_id(&self) -> &str {
            &self.node_id
        }

        pub fn pipe_wire_fd(&self) -> RawFd {
            self.pipe_wire_fd.as_raw_fd()
        }
    }

    impl Drop for PortalSession {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            // pipe_wire_fd's OwnedFd closes itself here.
        }
    }

    impl std::fmt::Debug for PortalSession {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("PortalSession")
                .field("node_id", &self.node_id)
                .field("pipe_wire_fd", &self.pipe_wire_fd.as_raw_fd())
                .finish()
        }
    }

    async fn portal_session_main(
        stream_tx: mpsc::Sender<Result<AcquiredStream, PortalError>>,
        shutdown_rx: oneshot::Receiver<()>,
    ) {
        use ashpd::desktop::{
            PersistMode,
            screencast::{
                CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
            },
        };

        let proxy = match Screencast::new().await {
            Ok(proxy) => proxy,
            Err(error) => {
                let _ = stream_tx.send(Err(PortalError::Session(error.to_string())));
                return;
            }
        };

        let session = match proxy.create_session(Default::default()).await {
            Ok(session) => session,
            Err(error) => {
                let _ = stream_tx.send(Err(PortalError::Session(error.to_string())));
                return;
            }
        };

        let cursor_mode = match proxy.available_cursor_modes().await {
            Ok(modes) if modes.contains(CursorMode::Metadata) => CursorMode::Metadata,
            Ok(modes) if modes.contains(CursorMode::Embedded) => CursorMode::Embedded,
            Ok(modes) if modes.contains(CursorMode::Hidden) => CursorMode::Hidden,
            Ok(_) => CursorMode::Hidden,
            Err(error) => {
                let _ = stream_tx.send(Err(PortalError::Session(error.to_string())));
                return;
            }
        };

        if let Err(error) = proxy
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_cursor_mode(cursor_mode)
                    .set_sources(SourceType::Monitor | SourceType::Window)
                    .set_multiple(false)
                    .set_persist_mode(PersistMode::DoNot),
            )
            .await
        {
            let _ = stream_tx.send(Err(PortalError::Session(error.to_string())));
            return;
        }

        let response = match proxy.start(&session, None, Default::default()).await {
            Ok(response) => response,
            Err(error) => {
                let _ = stream_tx.send(Err(PortalError::Session(error.to_string())));
                return;
            }
        };

        let response = match response.response() {
            Ok(response) => response,
            Err(error) => {
                let _ = stream_tx.send(Err(PortalError::Session(error.to_string())));
                return;
            }
        };

        let node_id = match response
            .streams()
            .first()
            .map(|stream| stream.pipe_wire_node_id().to_string())
        {
            Some(node_id) => node_id,
            None => {
                let _ = stream_tx.send(Err(PortalError::NoStreams));
                return;
            }
        };

        let fd = match proxy
            .open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default())
            .await
        {
            Ok(fd) => fd,
            Err(error) => {
                let _ = stream_tx.send(Err(PortalError::Session(error.to_string())));
                return;
            }
        };

        if stream_tx.send(Ok(AcquiredStream { node_id, fd })).is_err() {
            return;
        }

        // Keep the proxy and session in scope until the consumer signals
        // shutdown; dropping them earlier tears down the PipeWire stream.
        let _ = shutdown_rx.await;
        drop(session);
        drop(proxy);
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::PortalError;

    #[cfg(unix)]
    use std::os::fd::RawFd;

    #[derive(Debug)]
    pub struct PortalSession;

    impl PortalSession {
        pub fn acquire() -> Result<Self, PortalError> {
            Err(PortalError::Session(
                "Wayland portal capture is only available on Linux".to_string(),
            ))
        }

        pub fn node_id(&self) -> &str {
            ""
        }

        #[cfg(unix)]
        pub fn pipe_wire_fd(&self) -> RawFd {
            -1
        }
    }
}

pub use imp::PortalSession;
