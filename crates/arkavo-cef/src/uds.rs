use crate::error::{CefError, Result};
use crate::protocol::Protocol;
use bytes::BytesMut;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub struct UdsTransport {
    stream: UnixStream,
    read_buf: BytesMut,
    write_buf: BytesMut,
}

impl UdsTransport {
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|e| CefError::UdsConnectionFailed(e.to_string()))?;

        Ok(Self {
            stream,
            read_buf: BytesMut::with_capacity(8192),
            write_buf: BytesMut::with_capacity(8192),
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
                return Ok(data);
            }

            let n = self
                .stream
                .read_buf(&mut self.read_buf)
                .await
                .map_err(|e| CefError::UdsTransportError(e.to_string()))?;

            if n == 0 {
                return Err(CefError::UdsTransportError("Connection closed".to_string()));
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
        let data = self.recv_raw().await?;
        Protocol::deserialize_feedback(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn test_uds_transport() {
        let temp_dir = tempfile::tempdir().unwrap();
        let socket_path = temp_dir.path().join("test.sock");

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
