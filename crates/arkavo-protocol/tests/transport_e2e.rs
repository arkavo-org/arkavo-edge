#![allow(clippy::disallowed_methods)]

use arkavo_protocol::{
    A2aEndpoint, A2aRequest, A2aTransport, HttpTransport, TransportConfig, WebSocketTransport,
};
use jsonrpsee::server::Server;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::{core::async_trait, proc_macros::rpc};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

#[rpc(server)]
trait TestApi {
    #[method(name = "echo")]
    async fn echo(&self, message: String) -> Result<String, ErrorObjectOwned>;

    #[method(name = "delay")]
    async fn delay(&self, millis: u64) -> Result<String, ErrorObjectOwned>;

    #[method(name = "error")]
    async fn error(&self) -> Result<String, ErrorObjectOwned>;
}

struct TestApiImpl;

#[async_trait]
impl TestApiServer for TestApiImpl {
    async fn echo(&self, message: String) -> Result<String, ErrorObjectOwned> {
        Ok(format!("Echo: {message}"))
    }

    async fn delay(&self, millis: u64) -> Result<String, ErrorObjectOwned> {
        sleep(Duration::from_millis(millis)).await;
        Ok(format!("Delayed {millis} ms"))
    }

    async fn error(&self) -> Result<String, ErrorObjectOwned> {
        Err(ErrorObjectOwned::owned(
            -32000,
            "Test error",
            Some("This is a test error"),
        ))
    }
}

async fn start_test_server() -> anyhow::Result<(jsonrpsee::server::ServerHandle, u16)> {
    let server = Server::builder().build("127.0.0.1:0").await?;
    let addr = server.local_addr()?;
    let port = addr.port();

    let api = TestApiImpl;
    let handle = server.start(api.into_rpc());

    Ok((handle, port))
}

fn test_config() -> TransportConfig {
    let mut config = TransportConfig::default();
    config.tls_config.require_tls = false;
    config
}

async fn test_transport_echo(transport: Arc<dyn A2aTransport>, endpoint: &A2aEndpoint) {
    // Connect
    transport.connect(endpoint).await.unwrap();
    assert!(transport.is_connected());

    // Test echo
    let request = A2aRequest::new("echo", serde_json::json!(["Hello, A2A!"]));
    let response = transport.send_request(request).await.unwrap();

    match response {
        arkavo_protocol::A2aResponse::Success { result, .. } => {
            let message: String = serde_json::from_value(result).unwrap();
            assert_eq!(message, "Echo: Hello, A2A!");
        }
        _ => panic!("Expected success response"),
    }

    // Close
    transport.close().await.unwrap();
    assert!(!transport.is_connected());
}

async fn test_transport_error(transport: Arc<dyn A2aTransport>, endpoint: &A2aEndpoint) {
    transport.connect(endpoint).await.unwrap();

    let request = A2aRequest::new("error", serde_json::json!([]));
    let response = transport.send_request(request).await.unwrap();

    match response {
        arkavo_protocol::A2aResponse::Error { error, .. } => {
            assert_eq!(error.code, -32000);
            assert_eq!(error.message, "Test error");
        }
        _ => panic!("Expected error response"),
    }

    transport.close().await.unwrap();
}

async fn test_transport_timeout(transport: Arc<dyn A2aTransport>, endpoint: &A2aEndpoint) {
    transport.connect(endpoint).await.unwrap();

    // This should timeout (delay is 5000ms, timeout is 1000ms)
    let request = A2aRequest::new("delay", serde_json::json!([5000u64]));
    let result = transport.send_request(request).await;

    // Should fail due to timeout
    assert!(result.is_err());

    transport.close().await.unwrap();
}

#[tokio::test]
async fn test_http_transport_echo() {
    let (_server, port) = start_test_server().await.unwrap();
    let endpoint = A2aEndpoint {
        url: format!("http://127.0.0.1:{port}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    let transport = Arc::new(HttpTransport::new(test_config()).unwrap());
    test_transport_echo(transport, &endpoint).await;
}

#[tokio::test]
async fn test_websocket_transport_echo() {
    let (_server, port) = start_test_server().await.unwrap();
    let endpoint = A2aEndpoint {
        url: format!("ws://127.0.0.1:{port}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    let transport = Arc::new(WebSocketTransport::new(test_config()));
    test_transport_echo(transport, &endpoint).await;
}

#[tokio::test]
async fn test_http_transport_error() {
    let (_server, port) = start_test_server().await.unwrap();
    let endpoint = A2aEndpoint {
        url: format!("http://127.0.0.1:{port}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    let transport = Arc::new(HttpTransport::new(test_config()).unwrap());
    test_transport_error(transport, &endpoint).await;
}

#[tokio::test]
async fn test_websocket_transport_error() {
    let (_server, port) = start_test_server().await.unwrap();
    let endpoint = A2aEndpoint {
        url: format!("ws://127.0.0.1:{port}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    let transport = Arc::new(WebSocketTransport::new(test_config()));
    test_transport_error(transport, &endpoint).await;
}

#[tokio::test]
async fn test_http_transport_timeout() {
    let (_server, port) = start_test_server().await.unwrap();
    let endpoint = A2aEndpoint {
        url: format!("http://127.0.0.1:{port}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    let mut config = test_config();
    config.timeout_ms = 1000; // 1 second timeout
    let transport = Arc::new(HttpTransport::new(config).unwrap());
    test_transport_timeout(transport, &endpoint).await;
}

#[tokio::test]
async fn test_websocket_transport_timeout() {
    let (_server, port) = start_test_server().await.unwrap();
    let endpoint = A2aEndpoint {
        url: format!("ws://127.0.0.1:{port}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    let mut config = test_config();
    config.timeout_ms = 1000; // 1 second timeout
    let transport = Arc::new(WebSocketTransport::new(config));
    test_transport_timeout(transport, &endpoint).await;
}

#[tokio::test]
async fn test_http_retry_mechanism() {
    // Test will use a server that returns errors initially
    let server = Server::builder().build("127.0.0.1:0").await.unwrap();
    let port = server.local_addr().unwrap().port();

    // Counter for tracking attempts
    use std::sync::atomic::{AtomicU32, Ordering};
    static ATTEMPT_COUNT: AtomicU32 = AtomicU32::new(0);

    #[derive(Clone)]
    struct RetryApi;

    #[async_trait]
    impl TestApiServer for RetryApi {
        async fn echo(&self, message: String) -> Result<String, ErrorObjectOwned> {
            let attempt = ATTEMPT_COUNT.fetch_add(1, Ordering::SeqCst);
            if attempt < 2 {
                // Fail first two attempts
                Err(ErrorObjectOwned::owned(
                    -32603,
                    "Temporary failure",
                    None::<String>,
                ))
            } else {
                // Succeed on third attempt
                Ok(format!("Echo after {} attempts: {}", attempt + 1, message))
            }
        }

        async fn delay(&self, millis: u64) -> Result<String, ErrorObjectOwned> {
            sleep(Duration::from_millis(millis)).await;
            Ok(format!("Delayed {millis} ms"))
        }

        async fn error(&self) -> Result<String, ErrorObjectOwned> {
            Err(ErrorObjectOwned::owned(
                -32000,
                "Test error",
                Some("This is a test error"),
            ))
        }
    }

    let _server = server.start(RetryApi.into_rpc());

    let endpoint = A2aEndpoint {
        url: format!("http://127.0.0.1:{port}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    let mut config = test_config();
    config.max_retries = 3;
    config.retry_delay_ms = 100;

    let transport = Arc::new(HttpTransport::new(config).unwrap());
    transport.connect(&endpoint).await.unwrap();

    // Reset counter
    ATTEMPT_COUNT.store(0, Ordering::SeqCst);

    // This should succeed after retries
    let request = A2aRequest::new("echo", serde_json::json!(["Retry test"]));
    let response = transport.send_request(request).await.unwrap();

    match response {
        arkavo_protocol::A2aResponse::Success { result, .. } => {
            let message: String = serde_json::from_value(result).unwrap();
            assert!(message.contains("Echo after 3 attempts"));
        }
        _ => panic!("Expected success response after retries"),
    }

    transport.close().await.unwrap();
}

#[tokio::test]
async fn test_transport_parity() {
    // Test that both transports behave identically
    let (_server, port) = start_test_server().await.unwrap();

    let http_endpoint = A2aEndpoint {
        url: format!("http://127.0.0.1:{port}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    let ws_endpoint = A2aEndpoint {
        url: format!("ws://127.0.0.1:{port}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    let http_transport = Arc::new(HttpTransport::new(test_config()).unwrap());
    let ws_transport = Arc::new(WebSocketTransport::new(test_config()));

    // Connect both
    http_transport.connect(&http_endpoint).await.unwrap();
    ws_transport.connect(&ws_endpoint).await.unwrap();

    // Send identical requests
    let request1 = A2aRequest::new("echo", serde_json::json!(["Parity test"]));
    let request2 = A2aRequest::new("echo", serde_json::json!(["Parity test"]));

    let http_response = http_transport.send_request(request1).await.unwrap();
    let ws_response = ws_transport.send_request(request2).await.unwrap();

    // Verify responses are identical (except for request IDs)
    match (http_response, ws_response) {
        (
            arkavo_protocol::A2aResponse::Success {
                result: http_result,
                ..
            },
            arkavo_protocol::A2aResponse::Success {
                result: ws_result, ..
            },
        ) => {
            assert_eq!(http_result, ws_result);
        }
        _ => panic!("Expected matching success responses"),
    }

    http_transport.close().await.unwrap();
    ws_transport.close().await.unwrap();
}
