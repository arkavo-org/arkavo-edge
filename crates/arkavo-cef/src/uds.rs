use crate::error::{CefError, Result};
use crate::protocol::Protocol;
use bytes::BytesMut;
use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub struct UdsTransport {
    stream: UnixStream,
    read_buf: BytesMut,
    message_queue: VecDeque<ReceivedMessage>,
}

impl UdsTransport {
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

        Ok(Self {
            stream,
            read_buf: BytesMut::with_capacity(8192),
            message_queue: VecDeque::new(),
        })
    }

    pub async fn send_raw(&mut self, data: &[u8]) -> Result<()> {
        let framed = Protocol::frame_message(data);
        self.stream
            .write_all(&framed)
            .await
            .map_err(|e| CefError::UdsTransportError(e.to_string()))?;
        Ok(())
    }

    pub async fn recv_raw(&mut self) -> Result<Vec<u8>> {
        loop {
            if let Some(data) = Protocol::unframe_message(&mut self.read_buf)? {
                eprintln!(
                    "[UDS RUST] recv_raw returning {} bytes (msg_type={})",
                    data.len(),
                    if data.is_empty() { 0 } else { data[0] }
                );
                return Ok(data);
            }

            let n = self
                .stream
                .read_buf(&mut self.read_buf)
                .await
                .map_err(|e| CefError::UdsTransportError(e.to_string()))?;

            eprintln!("[UDS RUST] recv_raw read {n} bytes from socket");

            if n == 0 {
                return Err(CefError::UdsTransportError("Connection closed".to_string()));
            }
        }
    }

    // 1.98 files the same shape under a second name for functions in impl blocks.
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
    pub async fn try_recv_raw(&mut self) -> Result<Option<Vec<u8>>> {
        if let Some(data) = Protocol::unframe_message(&mut self.read_buf)? {
            eprintln!(
                "[UDS RUST] try_recv_raw returning buffered {} bytes (msg_type={})",
                data.len(),
                if data.is_empty() { 0 } else { data[0] }
            );
            return Ok(Some(data));
        }

        let mut temp_buf = vec![0u8; 4096];
        match self.stream.try_read(&mut temp_buf) {
            Ok(0) => {
                eprintln!("[UDS RUST] try_recv_raw: connection closed");
                Err(CefError::UdsTransportError("Connection closed".to_string()))
            }
            Ok(n) => {
                eprintln!("[UDS RUST] try_recv_raw read {n} bytes from socket");
                self.read_buf.extend_from_slice(&temp_buf[..n]);

                if let Some(data) = Protocol::unframe_message(&mut self.read_buf)? {
                    eprintln!(
                        "[UDS RUST] try_recv_raw returning {} bytes (msg_type={})",
                        data.len(),
                        if data.is_empty() { 0 } else { data[0] }
                    );
                    return Ok(Some(data));
                }
                Ok(None)
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => {
                eprintln!("[UDS RUST ERROR] try_recv_raw failed: {e}");
                Err(CefError::UdsTransportError(e.to_string()))
            }
        }
    }

    pub async fn send_command(
        &mut self,
        id: u32,
        op: u8,
        selector: &str,
        payload: &str,
        property: Option<&str>,
    ) -> Result<()> {
        let data = Protocol::serialize_command(id, op, selector, payload, property)?;
        self.send_raw(&data).await
    }

    pub async fn recv_feedback(&mut self) -> Result<crate::protocol::DOMFeedbackSimple> {
        loop {
            let data = self.recv_raw().await?;

            if data.is_empty() {
                continue;
            }

            match data[0] {
                0x01 => {
                    // Feedback message - return it
                    return Protocol::deserialize_feedback(&data);
                }
                0x02 => {
                    // Event message - queue it for later
                    let event = Protocol::deserialize_event(&data)?;
                    eprintln!(
                        "[UDS RUST] recv_feedback: queuing EVENT (type={}, value={})",
                        event.event_type, event.value
                    );
                    self.message_queue.push_back(ReceivedMessage::Event(event));
                }
                0x03 => {
                    // Error message - queue it for later
                    let error = Protocol::deserialize_error(&data)?;
                    eprintln!(
                        "[UDS RUST] recv_feedback: queuing ERROR (type={})",
                        error.error_type
                    );
                    self.message_queue.push_back(ReceivedMessage::Error(error));
                }
                _ => {
                    return Err(CefError::ProtocolError(format!(
                        "Unknown message type: {}",
                        data[0]
                    )));
                }
            }
        }
    }

    pub async fn recv_event(&mut self) -> Result<crate::protocol::DOMEvent> {
        let data = self.recv_raw().await?;
        Protocol::deserialize_event(&data)
    }

    pub async fn recv_error(&mut self) -> Result<crate::protocol::DOMError> {
        let data = self.recv_raw().await?;
        Protocol::deserialize_error(&data)
    }

    pub fn requeue_message(&mut self, msg: ReceivedMessage) {
        self.message_queue.push_front(msg);
    }

    pub async fn try_recv_message(&mut self) -> Result<Option<ReceivedMessage>> {
        // Check queue first
        if let Some(msg) = self.message_queue.pop_front() {
            eprintln!("[UDS RUST] try_recv_message: returning queued message");
            return Ok(Some(msg));
        }

        let data = match self.try_recv_raw().await? {
            Some(d) => d,
            None => return Ok(None),
        };

        if data.is_empty() {
            return Ok(None);
        }

        eprintln!(
            "[UDS RUST] try_recv_message processing msg_type={}",
            data[0]
        );

        match data[0] {
            0x01 => {
                let feedback = Protocol::deserialize_feedback(&data)?;
                eprintln!("[UDS RUST] Received FEEDBACK message (id={})", feedback.id);
                Ok(Some(ReceivedMessage::Feedback(feedback)))
            }
            0x02 => {
                let event = Protocol::deserialize_event(&data)?;
                eprintln!(
                    "[UDS RUST] Received EVENT message (type={}, value={})",
                    event.event_type, event.value
                );
                Ok(Some(ReceivedMessage::Event(event)))
            }
            0x03 => {
                let error = Protocol::deserialize_error(&data)?;
                eprintln!(
                    "[UDS RUST] Received ERROR message (type={})",
                    error.error_type
                );
                Ok(Some(ReceivedMessage::Error(error)))
            }
            _ => Err(CefError::ProtocolError(format!(
                "Unknown message type: {}",
                data[0]
            ))),
        }
    }
}

#[derive(Debug)]
pub enum ReceivedMessage {
    Feedback(crate::protocol::DOMFeedbackSimple),
    Event(crate::protocol::DOMEvent),
    Error(crate::protocol::DOMError),
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn test_uds_transport() {
        let socket_path = format!("/tmp/arkavo_test_{}.sock", std::process::id());

        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            stream.write_all(&buf[..n]).await.unwrap();
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut transport = UdsTransport::connect(&socket_path).await.unwrap();
        transport.send_raw(b"test").await.unwrap();
        let data = transport.recv_raw().await.unwrap();
        assert_eq!(data, b"test");

        server.await.unwrap();
    }
}
