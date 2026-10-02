#![allow(clippy::disallowed_methods)]

use arkavo_protocol::error::A2aError;
use arkavo_protocol::http::HttpTransport;
use arkavo_protocol::transport::{
    A2aEndpoint, A2aRequest, A2aTransport, TlsConfig, TransportConfig,
};
use arkavo_protocol::websocket::WebSocketTransport;
use jsonrpsee::server::{ServerBuilder, ServerHandle};
use jsonrpsee::{core::async_trait, proc_macros::rpc};
use std::fmt::Write as _;
use std::fs;
use std::net::SocketAddr;
use tokio::time::{Duration, sleep};

fn test_certs_dir() -> tempfile::TempDir {
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};

    // Each test owns its files until it finishes. A shared directory let one
    // generator truncate a key while another test was parsing it.
    let dir = tempfile::tempdir().expect("certificate fixture directory");
    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Test CA");
    let ca_key = KeyPair::generate().expect("CA key");
    let ca = ca_params.self_signed(&ca_key).expect("CA certificate");
    fs::write(dir.path().join("ca.crt"), ca.pem()).unwrap();
    fs::write(dir.path().join("ca.key"), ca_key.serialize_pem()).unwrap();

    for name in ["server", "client", "invalid_client"] {
        let names = if name == "server" {
            vec!["localhost".into(), "127.0.0.1".into()]
        } else {
            Vec::new()
        };
        let mut params = CertificateParams::new(names).expect("certificate parameters");
        params.distinguished_name.push(DnType::CommonName, name);
        let key = KeyPair::generate().expect("fixture key");
        let cert = if name == "invalid_client" {
            params.self_signed(&key)
        } else {
            params.signed_by(&key, &ca, &ca_key)
        }
        .expect("fixture certificate");
        fs::write(dir.path().join(format!("{name}.crt")), cert.pem()).unwrap();
        fs::write(dir.path().join(format!("{name}.key")), key.serialize_pem()).unwrap();
    }
    dir
}

#[rpc(server)]
trait TestApi {
    #[method(name = "test_method")]
    async fn test_method(
        &self,
        value: String,
    ) -> Result<String, jsonrpsee::types::ErrorObjectOwned>;
}

struct TestServer;

#[async_trait]
impl TestApiServer for TestServer {
    async fn test_method(
        &self,
        value: String,
    ) -> Result<String, jsonrpsee::types::ErrorObjectOwned> {
        Ok(format!("Echo: {value}"))
    }
}

async fn start_test_server() -> (ServerHandle, SocketAddr) {
    let server = ServerBuilder::default().build("127.0.0.1:0").await.unwrap();

    let addr = server.local_addr().unwrap();
    let handle = server.start(TestServer.into_rpc());

    (handle, addr)
}

#[tokio::test]
async fn test_http_mtls_with_valid_client_cert() {
    let fixture = test_certs_dir();
    let certs_dir = fixture.path();

    // Configure transport with mTLS
    let config = TransportConfig {
        tls_config: TlsConfig {
            verify_cert: false, // Self-signed cert
            require_tls: false, // Allow http for testing
            client_cert_path: Some(certs_dir.join("client.crt").to_string_lossy().to_string()),
            client_key_path: Some(certs_dir.join("client.key").to_string_lossy().to_string()),
            ca_cert_path: Some(certs_dir.join("ca.crt").to_string_lossy().to_string()),
        },
        ..Default::default()
    };

    let transport = HttpTransport::new(config).unwrap();

    // Start test server
    let (_handle, addr) = start_test_server().await;
    sleep(Duration::from_millis(100)).await;

    let endpoint = A2aEndpoint {
        url: format!("http://{addr}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    // Connect should succeed
    transport.connect(&endpoint).await.unwrap();
    assert!(transport.is_connected());

    // Send a test request
    let request = A2aRequest::new("test_method", serde_json::json!(["test_value"]));
    let response = transport.send_request(request).await.unwrap();

    match response {
        arkavo_protocol::transport::A2aResponse::Success { result, .. } => {
            assert_eq!(result.as_str().unwrap(), "Echo: test_value");
        }
        _ => panic!("Expected success response"),
    }
}

#[tokio::test]
async fn test_websocket_mtls_configuration() {
    let fixture = test_certs_dir();
    let certs_dir = fixture.path();

    // Configure transport with mTLS
    let config = TransportConfig {
        tls_config: TlsConfig {
            verify_cert: false, // Self-signed cert
            require_tls: false, // Allow ws for testing
            client_cert_path: Some(certs_dir.join("client.crt").to_string_lossy().to_string()),
            client_key_path: Some(certs_dir.join("client.key").to_string_lossy().to_string()),
            ca_cert_path: Some(certs_dir.join("ca.crt").to_string_lossy().to_string()),
        },
        ..Default::default()
    };

    // Should create transport successfully with mTLS config
    let transport = WebSocketTransport::new(config);
    assert!(!transport.is_connected());
}

#[tokio::test]
async fn test_http_without_client_cert() {
    // Configure transport without client certificates
    let config = TransportConfig {
        tls_config: TlsConfig {
            verify_cert: false,
            require_tls: false,
            client_cert_path: None,
            client_key_path: None,
            ca_cert_path: None,
        },
        ..Default::default()
    };

    let transport = HttpTransport::new(config).unwrap();

    // Start test server
    let (_handle, addr) = start_test_server().await;
    sleep(Duration::from_millis(100)).await;

    let endpoint = A2aEndpoint {
        url: format!("http://{addr}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    // Connect should succeed without mTLS
    transport.connect(&endpoint).await.unwrap();
    assert!(transport.is_connected());
}

#[tokio::test]
async fn test_tls_requirement_enforcement() {
    // Configure transport to require TLS
    let config = TransportConfig {
        tls_config: TlsConfig {
            verify_cert: true,
            require_tls: true,
            client_cert_path: None,
            client_key_path: None,
            ca_cert_path: None,
        },
        ..Default::default()
    };

    let http_transport = HttpTransport::new(config.clone()).unwrap();
    let ws_transport = WebSocketTransport::new(config);

    // HTTP endpoint without TLS should fail
    let http_endpoint = A2aEndpoint {
        url: "http://localhost:8080".to_string(),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    let result = http_transport.connect(&http_endpoint).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("TLS is required"));

    // WebSocket endpoint without TLS should fail
    let ws_endpoint = A2aEndpoint {
        url: "ws://localhost:8080".to_string(),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };

    let result = ws_transport.connect(&ws_endpoint).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("TLS is required"));
}

#[tokio::test]
async fn test_invalid_cert_paths() {
    let config = TransportConfig {
        tls_config: TlsConfig {
            verify_cert: false,
            require_tls: false,
            client_cert_path: Some("/nonexistent/client.crt".to_string()),
            client_key_path: Some("/nonexistent/client.key".to_string()),
            ca_cert_path: None,
        },
        ..Default::default()
    };

    // HTTP transport should fail to create with invalid paths
    let result = HttpTransport::new(config);
    assert!(result.is_err());
    if let Err(err) = result {
        match err {
            A2aError::Tls(msg) => {
                assert!(msg.contains("Failed to read client"));
            }
            _ => panic!("Expected TLS error"),
        }
    }
}

#[tokio::test]
async fn test_ca_certificate_loading() {
    let fixture = test_certs_dir();
    let certs_dir = fixture.path();

    // Configure with CA certificate
    let config = TransportConfig {
        tls_config: TlsConfig {
            verify_cert: true,
            require_tls: false,
            client_cert_path: None,
            client_key_path: None,
            ca_cert_path: Some(certs_dir.join("ca.crt").to_string_lossy().to_string()),
        },
        ..Default::default()
    };

    // Should successfully create transport with CA cert
    let transport = HttpTransport::new(config).unwrap();
    assert!(!transport.is_connected());
}

fn is_certificate_verification_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("certificate")
        || e.contains("unknownissuer")
        || e.contains("unknown issuer")
        || e.contains("notvalidforname")
        || e.contains("invalid peer")
        || e.contains("cert")
}

async fn spawn_self_signed_tls_listener() -> (u16, tokio::task::JoinHandle<()>) {
    use rustls::ServerConfig;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use std::sync::Arc;
    use tokio_rustls::TlsAcceptor;

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let fixture = test_certs_dir();
    let certs_dir = fixture.path();
    let cert_file = fs::File::open(certs_dir.join("server.crt")).expect("server.crt");
    let key_file = fs::File::open(certs_dir.join("server.key")).expect("server.key");
    let certs: Vec<CertificateDer> = CertificateDer::pem_reader_iter(cert_file)
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("parse server cert");
    let key = PrivateKeyDer::from_pem_reader(key_file).expect("parse server key");
    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("tls server config");
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local addr").port();
    let handle = tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = acceptor.accept(stream).await;
        }
    });
    (port, handle)
}

/// `verify_cert: false` must not disable verification: a self-signed server
/// is still rejected (CWE-295 / CodeQL rust/disabled-certificate-check).
#[tokio::test]
async fn test_http_verify_cert_false_still_rejects_self_signed() {
    let (port, server) = spawn_self_signed_tls_listener().await;
    let config = TransportConfig {
        timeout_ms: 3000,
        max_retries: 0,
        retry_delay_ms: 0,
        tls_config: TlsConfig {
            verify_cert: false,
            require_tls: true,
            ..Default::default()
        },
    };
    let transport = HttpTransport::new(config).unwrap();
    let endpoint = A2aEndpoint {
        url: format!("https://127.0.0.1:{port}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };
    transport.connect(&endpoint).await.unwrap();
    let err = transport
        .send_request(A2aRequest::new("test_method", serde_json::json!([])))
        .await
        .expect_err("self-signed HTTPS must fail certificate verification");
    server.abort();
    let mut msg = String::new();
    for cause in err.chain() {
        write!(msg, "{cause} {cause:?} ").unwrap();
    }
    assert!(
        is_certificate_verification_error(&msg),
        "expected certificate verification failure, got: {msg}"
    );
}

#[tokio::test]
async fn test_websocket_verify_cert_false_still_rejects_self_signed() {
    let (port, server) = spawn_self_signed_tls_listener().await;
    let config = TransportConfig {
        timeout_ms: 3000,
        max_retries: 0,
        retry_delay_ms: 0,
        tls_config: TlsConfig {
            verify_cert: false,
            require_tls: true,
            ..Default::default()
        },
    };
    let transport = WebSocketTransport::new(config);
    let endpoint = A2aEndpoint {
        url: format!("wss://127.0.0.1:{port}"),
        agent_id: "test-agent".to_string(),
        public_key: None,
    };
    let err = transport
        .connect(&endpoint)
        .await
        .expect_err("self-signed WSS must fail certificate verification");
    server.abort();
    let msg = err.to_string();
    assert!(
        is_certificate_verification_error(&msg),
        "expected certificate verification failure, got: {msg}"
    );
}

#[test]
fn test_certificates_exist() {
    let fixture = test_certs_dir();
    let certs_dir = fixture.path();

    // Verify all test certificates were generated
    assert!(certs_dir.join("ca.crt").exists(), "CA certificate missing");
    assert!(certs_dir.join("ca.key").exists(), "CA key missing");
    assert!(
        certs_dir.join("server.crt").exists(),
        "Server certificate missing"
    );
    assert!(certs_dir.join("server.key").exists(), "Server key missing");
    assert!(
        certs_dir.join("client.crt").exists(),
        "Client certificate missing"
    );
    assert!(certs_dir.join("client.key").exists(), "Client key missing");
    assert!(
        certs_dir.join("invalid_client.crt").exists(),
        "Invalid client certificate missing"
    );
    assert!(
        certs_dir.join("invalid_client.key").exists(),
        "Invalid client key missing"
    );
}

#[test]
fn certificate_fixtures_are_private_and_complete_under_concurrent_use() {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::sync::{Arc, Barrier};

    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..8 {
            let barrier = Arc::clone(&barrier);
            workers.push(scope.spawn(move || {
                barrier.wait();
                let fixture = test_certs_dir();
                let path: &Path = fixture.as_ref();
                for name in ["ca", "server", "client", "invalid_client"] {
                    let cert = CertificateDer::from_pem_file(path.join(format!("{name}.crt")))
                        .expect("fixture certificate must be complete");
                    let key = PrivateKeyDer::from_pem_file(path.join(format!("{name}.key")))
                        .expect("fixture key must be complete");
                    rustls::ServerConfig::builder_with_provider(Arc::new(
                        rustls::crypto::aws_lc_rs::default_provider(),
                    ))
                    .with_safe_default_protocol_versions()
                    .unwrap()
                    .with_no_client_auth()
                    .with_single_cert(vec![cert], key)
                    .expect("fixture certificate must match its key");
                }
                fixture
            }));
        }
        let fixtures: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        let paths: BTreeSet<_> = fixtures
            .iter()
            .map(|fixture| {
                let path: &Path = fixture.as_ref();
                path.to_path_buf()
            })
            .collect();
        assert_eq!(
            paths.len(),
            fixtures.len(),
            "fixtures must not share mutable files"
        );
    });
}
