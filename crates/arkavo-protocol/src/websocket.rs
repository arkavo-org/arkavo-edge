use crate::error::{A2aError, Result};
use crate::transport::{A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TransportConfig};
use async_trait::async_trait;
use dashmap::DashMap;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use std::fs;
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{RwLock, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsSink = SplitSink<WsStream, Message>;
type WsReader = SplitStream<WsStream>;

struct WebSocketConnection {
    writer: Arc<RwLock<WsSink>>,
    reader_task: JoinHandle<()>,
}

pub struct WebSocketTransport {
    config: TransportConfig,
    connection: Arc<RwLock<Option<WebSocketConnection>>>,
    endpoint: Arc<RwLock<Option<A2aEndpoint>>>,
    pending_requests: Arc<DashMap<Uuid, oneshot::Sender<A2aResponse>>>,
}

impl WebSocketTransport {
    pub fn new(config: TransportConfig) -> Self {
        Self {
            config,
            connection: Arc::new(RwLock::new(None)),
            endpoint: Arc::new(RwLock::new(None)),
            pending_requests: Arc::new(DashMap::new()),
        }
    }

    async fn build_tls_connector(&self) -> Result<Connector> {
        // Start with root certificates
        let mut root_store = RootCertStore::empty();

        // Add system root certificates
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        // Add custom CA certificate if provided
        if let Some(ca_path) = &self.config.tls_config.ca_cert_path {
            debug!("Loading CA certificate from: {}", ca_path);
            let ca_pem = fs::read(ca_path)
                .map_err(|e| A2aError::Tls(format!("Failed to read CA certificate: {e}")))?;
            let mut ca_reader = BufReader::new(&ca_pem[..]);
            let ca_certs = rustls_pemfile::certs(&mut ca_reader)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| A2aError::Tls(format!("Failed to parse CA certificate: {e}")))?;

            for cert in ca_certs {
                root_store
                    .add(cert)
                    .map_err(|e| A2aError::Tls(format!("Failed to add CA certificate: {e}")))?;
            }
            info!("Custom CA certificate added");
        }

        // Build the client config based on whether mTLS is needed
        let mut config = if let (Some(cert_path), Some(key_path)) = (
            &self.config.tls_config.client_cert_path,
            &self.config.tls_config.client_key_path,
        ) {
            debug!("Loading client certificate from: {}", cert_path);
            debug!("Loading client key from: {}", key_path);

            let cert_pem = fs::read(cert_path)
                .map_err(|e| A2aError::Tls(format!("Failed to read client certificate: {e}")))?;
            let key_pem = fs::read(key_path)
                .map_err(|e| A2aError::Tls(format!("Failed to read client key: {e}")))?;

            let mut cert_reader = BufReader::new(&cert_pem[..]);
            let certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut cert_reader)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| A2aError::Tls(format!("Failed to parse client certificate: {e}")))?;

            let mut key_reader = BufReader::new(&key_pem[..]);
            let key = rustls_pemfile::private_key(&mut key_reader)
                .map_err(|e| A2aError::Tls(format!("Failed to parse client key: {e}")))?
                .ok_or_else(|| A2aError::Tls("No private key found in file".to_string()))?;

            info!("Client certificate configured for mTLS");

            ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_client_auth_cert(certs, key)
                .map_err(|e| A2aError::Tls(format!("Failed to configure client auth: {e}")))?
        } else {
            ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth()
        };

        // Configure certificate verification
        if !self.config.tls_config.verify_cert {
            warn!("Certificate verification disabled - accepting any certificate");
            config
                .dangerous()
                .set_certificate_verifier(Arc::new(AcceptAnyServerCert));
        }

        Ok(Connector::Rustls(Arc::new(config)))
    }

    async fn connect_websocket(&self, url: &str) -> Result<WsStream> {
        let connector = if url.starts_with("wss://") {
            self.build_tls_connector().await?
        } else {
            Connector::Plain
        };

        let timeout_duration = Duration::from_millis(self.config.timeout_ms);

        let (ws_stream, _) = timeout(
            timeout_duration,
            connect_async_tls_with_config(url, None, false, Some(connector)),
        )
        .await
        .map_err(|_| A2aError::Timeout(self.config.timeout_ms))?
        .map_err(|e| A2aError::ConnectionFailed(format!("WebSocket connection failed: {e}")))?;

        Ok(ws_stream)
    }

    fn start_reader_task(
        mut reader: WsReader,
        pending_requests: Arc<DashMap<Uuid, oneshot::Sender<A2aResponse>>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(msg_result) = reader.next().await {
                match msg_result {
                    Ok(Message::Text(text)) => {
                        if let Ok(response) = serde_json::from_str::<A2aResponse>(&text) {
                            let id = match &response {
                                A2aResponse::Success { id, .. } => *id,
                                A2aResponse::Error { id, .. } => *id,
                            };

                            if let Some((_, sender)) = pending_requests.remove(&id) {
                                let _ = sender.send(response);
                            } else {
                                warn!("Received response for unknown request ID: {}", id);
                            }
                        } else {
                            warn!("Failed to parse WebSocket message as JSON-RPC response");
                        }
                    }
                    Ok(Message::Close(_)) => {
                        info!("WebSocket connection closed by server");
                        break;
                    }
                    Err(e) => {
                        error!("WebSocket read error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }

            // Clean up any pending requests
            pending_requests.clear();
        })
    }
}

#[async_trait]
impl A2aTransport for WebSocketTransport {
    async fn connect(&self, endpoint: &A2aEndpoint) -> anyhow::Result<()> {
        info!("Connecting to WebSocket endpoint: {}", endpoint.url);

        if !endpoint.url.starts_with("ws://") && !endpoint.url.starts_with("wss://") {
            return Err(A2aError::InvalidEndpoint(format!(
                "WebSocket URL must start with ws:// or wss://, got: {}",
                endpoint.url
            ))
            .into());
        }

        if self.config.tls_config.require_tls && endpoint.url.starts_with("ws://") {
            return Err(A2aError::Tls(
                "TLS is required but URL uses ws:// instead of wss://".to_string(),
            )
            .into());
        }

        let ws_stream = self.connect_websocket(&endpoint.url).await?;
        let (writer, reader) = ws_stream.split();

        let reader_task = Self::start_reader_task(reader, self.pending_requests.clone());

        let connection = WebSocketConnection {
            writer: Arc::new(RwLock::new(writer)),
            reader_task,
        };

        {
            let mut conn_guard = self.connection.write().await;
            *conn_guard = Some(connection);
        }

        {
            let mut endpoint_guard = self.endpoint.write().await;
            *endpoint_guard = Some(endpoint.clone());
        }

        info!("Successfully connected to {}", endpoint.url);
        Ok(())
    }

    async fn send_request(&self, request: A2aRequest) -> anyhow::Result<A2aResponse> {
        debug!(
            "Sending request: method={}, id={}",
            request.method, request.id
        );

        let connection = {
            let guard = self.connection.read().await;
            guard.as_ref().ok_or(A2aError::NotConnected)?.writer.clone()
        };

        // Create oneshot channel for response
        let (tx, rx) = oneshot::channel();
        self.pending_requests.insert(request.id, tx);

        // Build JSON-RPC request
        let json_rpc = serde_json::json!({
            "jsonrpc": request.jsonrpc,
            "method": request.method,
            "params": request.params,
            "id": request.id.to_string()
        });

        let message = Message::Text(json_rpc.to_string());

        // Send the request
        {
            let mut writer = connection.write().await;
            writer
                .send(message)
                .await
                .map_err(|e| A2aError::WebSocket(format!("Failed to send request: {e}")))?;
        }

        // Wait for response with timeout
        let timeout_duration = Duration::from_millis(self.config.timeout_ms);
        let response = timeout(timeout_duration, rx)
            .await
            .map_err(|_| {
                self.pending_requests.remove(&request.id);
                A2aError::Timeout(self.config.timeout_ms)
            })?
            .map_err(|_| A2aError::WebSocket("Response channel closed".to_string()))?;

        debug!("Received response for request id={}", request.id);
        Ok(response)
    }

    async fn close(&self) -> anyhow::Result<()> {
        info!("Closing WebSocket connection");

        if let Some(connection) = self.connection.write().await.take() {
            // Send close frame
            if let Ok(mut writer) = connection.writer.try_write() {
                let _ = writer.send(Message::Close(None)).await;
            }

            // Abort the reader task
            connection.reader_task.abort();
        }

        {
            let mut endpoint_guard = self.endpoint.write().await;
            *endpoint_guard = None;
        }

        // Clear pending requests
        self.pending_requests.clear();

        Ok(())
    }

    fn is_connected(&self) -> bool {
        let guard = futures::executor::block_on(self.connection.read());
        guard.is_some()
    }
}

// Custom certificate verifier that accepts any certificate (for development only)
#[derive(Debug)]
struct AcceptAnyServerCert;

impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_websocket_transport_creation() {
        let config = TransportConfig::default();
        let transport = WebSocketTransport::new(config);
        assert!(!transport.is_connected());
    }

    #[tokio::test]
    async fn test_invalid_endpoint() {
        let config = TransportConfig::default();
        let transport = WebSocketTransport::new(config);

        let endpoint = A2aEndpoint {
            url: "http://localhost:8080".to_string(),
            agent_id: "test-agent".to_string(),
            public_key: None,
        };

        let result = transport.connect(&endpoint).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("WebSocket URL must start with")
        );
    }

    #[tokio::test]
    async fn test_tls_requirement() {
        let config = TransportConfig {
            tls_config: crate::transport::TlsConfig {
                require_tls: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let transport = WebSocketTransport::new(config);

        let endpoint = A2aEndpoint {
            url: "ws://localhost:8080".to_string(),
            agent_id: "test-agent".to_string(),
            public_key: None,
        };

        let result = transport.connect(&endpoint).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("TLS is required"));
    }

    #[tokio::test]
    async fn test_not_connected_error() {
        let config = TransportConfig::default();
        let transport = WebSocketTransport::new(config);

        let request = A2aRequest::new("test_method", serde_json::json!({}));
        let result = transport.send_request(request).await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transport not connected")
        );
    }
}
