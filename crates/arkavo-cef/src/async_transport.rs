use crate::error::{CefError, Result};
use crate::protocol::{DOMEvent, DOMFeedbackSimple, Protocol};
use bytes::BytesMut;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, mpsc, oneshot};

pub struct AsyncTransport {
    write_half: Arc<Mutex<tokio::io::WriteHalf<UnixStream>>>,
    pending_feedbacks: Arc<Mutex<HashMap<u32, oneshot::Sender<DOMFeedbackSimple>>>>,
    event_rx: mpsc::UnboundedReceiver<DOMEvent>,
}

impl AsyncTransport {
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy();

        let valid_prefixes = ["/tmp/arkavo_", "/private/tmp/arkavo_", "/var/folders/"];
        if !valid_prefixes
            .iter()
            .any(|prefix| path_str.starts_with(prefix))
        {
            return Err(CefError::InvalidSocketPath(
                "Socket path must start with /tmp/arkavo_, /private/tmp/arkavo_, or /var/folders/"
                    .to_string(),
            ));
        }

        if path_str.len() > 100 {
            return Err(CefError::InvalidSocketPath(
                "Socket path too long".to_string(),
            ));
        }

        let timeout_duration = Duration::from_secs(10);

        let stream = tokio::time::timeout(timeout_duration, UnixStream::connect(path))
            .await
            .map_err(|_| CefError::ConnectionTimeout)?
            .map_err(|e| CefError::UdsConnectionFailed(e.to_string()))?;

        let (read_half, write_half) = tokio::io::split(stream);

        let pending_feedbacks: Arc<Mutex<HashMap<u32, oneshot::Sender<DOMFeedbackSimple>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        // Spawn background task to continuously read from socket
        let pending_clone = pending_feedbacks.clone();
        eprintln!("[AsyncTransport] Spawning background reader task");
        tokio::spawn(async move {
            eprintln!("[AsyncTransport] Background reader task started");
            Self::read_loop(read_half, pending_clone, event_tx).await;
            eprintln!("[AsyncTransport] Background reader task ended");
        });

        Ok(Self {
            write_half: Arc::new(Mutex::new(write_half)),
            pending_feedbacks,
            event_rx,
        })
    }

    async fn read_loop(
        mut read_half: tokio::io::ReadHalf<UnixStream>,
        pending: Arc<Mutex<HashMap<u32, oneshot::Sender<DOMFeedbackSimple>>>>,
        event_tx: mpsc::UnboundedSender<DOMEvent>,
    ) {
        let mut read_buf = BytesMut::with_capacity(8192);

        loop {
            match read_half.read_buf(&mut read_buf).await {
                Ok(0) => {
                    eprintln!("[AsyncTransport] Connection closed");
                    break;
                }
                Ok(n) => {
                    eprintln!("[AsyncTransport] Read {n} bytes from socket");

                    // Process all complete messages in buffer
                    while let Ok(Some(data)) = Protocol::unframe_message(&mut read_buf) {
                        if let Err(e) = Self::route_message(&data, &pending, &event_tx).await {
                            eprintln!("[AsyncTransport] Failed to route message: {e}");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[AsyncTransport] Read error: {e}");
                    break;
                }
            }
        }
    }

    async fn route_message(
        data: &[u8],
        pending: &Arc<Mutex<HashMap<u32, oneshot::Sender<DOMFeedbackSimple>>>>,
        event_tx: &mpsc::UnboundedSender<DOMEvent>,
    ) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        eprintln!("[AsyncTransport] Routing message type={}", data[0]);

        match data[0] {
            0x01 => {
                // Feedback message
                let feedback = Protocol::deserialize_feedback(data)?;
                eprintln!(
                    "[AsyncTransport] Routing feedback for command id={}, status={}",
                    feedback.id, feedback.status
                );

                let mut pending_map = pending.lock().await;
                eprintln!("[AsyncTransport] Pending commands: {:?}", pending_map.keys().collect::<Vec<_>>());
                if let Some(tx) = pending_map.remove(&feedback.id) {
                    eprintln!("[AsyncTransport] Sending feedback to command {}", feedback.id);
                    match tx.send(feedback) {
                        Ok(_) => eprintln!("[AsyncTransport] Feedback sent successfully"),
                        Err(_) => eprintln!("[AsyncTransport] ERROR: Feedback receiver dropped!"),
                    }
                } else {
                    eprintln!(
                        "[AsyncTransport] Warning: No pending command for feedback id={}",
                        feedback.id
                    );
                }
            }
            0x02 => {
                // Event message
                let event = Protocol::deserialize_event(data)?;
                eprintln!(
                    "[AsyncTransport] Routing event: type={}, value={}",
                    event.event_type, event.value
                );
                let _ = event_tx.send(event);
            }
            0x03 => {
                // Error message - for now just log it
                let error = Protocol::deserialize_error(data)?;
                eprintln!("[AsyncTransport] Error message: {error:?}");
            }
            _ => {
                eprintln!("[AsyncTransport] Unknown message type: {}", data[0]);
            }
        }

        Ok(())
    }

    pub async fn send_command(
        &mut self,
        id: u32,
        op: u8,
        selector: &str,
        payload: &str,
        property: Option<&str>,
    ) -> Result<oneshot::Receiver<DOMFeedbackSimple>> {
        let data = Protocol::serialize_command(id, op, selector, payload, property)?;
        let framed = Protocol::frame_message(&data);

        // Register for feedback BEFORE sending command
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending_feedbacks.lock().await;
            pending.insert(id, tx);
        }

        // Send command
        {
            let mut write = self.write_half.lock().await;
            eprintln!("[AsyncTransport] Sending command id={}, framed_size={} bytes", id, framed.len());
            write
                .write_all(&framed)
                .await
                .map_err(|e| CefError::UdsTransportError(e.to_string()))?;
            eprintln!("[AsyncTransport] Command id={} sent successfully", id);
        }

        Ok(rx)
    }

    pub fn try_recv_event(&mut self) -> Option<DOMEvent> {
        self.event_rx.try_recv().ok()
    }
}
