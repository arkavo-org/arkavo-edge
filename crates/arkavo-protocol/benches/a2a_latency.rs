//! Benchmarks for A2A transport latency.
//!
//! Measures the full hot path: serialization, HTTP/WebSocket round-trip over localhost,
//! and concurrent request throughput.

#![allow(clippy::disallowed_methods, clippy::semicolon_if_nothing_returned)]

use arkavo_protocol::WebSocketTransport;
use arkavo_protocol::http::HttpTransport;
use arkavo_protocol::transport::{
    A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TlsConfig, TransportConfig,
};
use axum::{Json, Router, routing::post};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use jsonrpsee::core::async_trait;
use jsonrpsee::server::Server;
use jsonrpsee::types::ErrorObjectOwned;
use serde_json::json;
use std::net::SocketAddr;
use uuid::Uuid;

/// Small payload (~100B) — heartbeat/discovery
fn small_params() -> serde_json::Value {
    json!({"action": "ping"})
}

/// Medium payload (~1KB) — typical tool call with args
fn medium_params() -> serde_json::Value {
    json!({
        "method": "tools/call",
        "name": "file_read",
        "arguments": {
            "path": "/src/main.rs",
            "encoding": "utf-8",
            "range": {"start": 0, "end": 100},
            "context": "Reading source file for analysis and transformation pipeline"
        },
        "metadata": {
            "agent_id": "agent-001",
            "session_id": "sess-abc123",
            "timestamp": "2026-01-01T00:00:00Z",
            "priority": "normal",
            "tags": ["code", "rust", "analysis"],
            "capabilities": ["read", "write", "execute"]
        },
        "padding": "x".repeat(400)
    })
}

/// Large payload (~10KB) — code context
fn large_params() -> serde_json::Value {
    let code_block = "fn process_item(item: &Item) -> Result<Output> {\n    let validated = validate(item)?;\n    transform(validated)\n}\n";
    let repeated = code_block.repeat(50);
    json!({
        "method": "context/update",
        "context": {
            "files": [
                {"path": "src/main.rs", "content": &repeated},
                {"path": "src/lib.rs", "content": &repeated[..repeated.len() / 2]}
            ],
            "diagnostics": [
                {"severity": "warning", "message": "unused variable", "line": 42},
                {"severity": "error", "message": "type mismatch", "line": 87}
            ]
        }
    })
}

/// Build a success response matching what the server returns
fn success_response(id: Uuid) -> A2aResponse {
    A2aResponse::Success {
        jsonrpc: "2.0".to_string(),
        id,
        result: json!({"status": "ok"}),
    }
}

/// RPC handler: deserialize request, return success
async fn rpc_handler(Json(req): Json<A2aRequest>) -> Json<A2aResponse> {
    Json(success_response(req.id))
}

/// Start a minimal axum server on an ephemeral port, return the bound address.
fn start_server(rt: &tokio::runtime::Runtime) -> SocketAddr {
    let (tx, rx) = std::sync::mpsc::channel();

    rt.spawn(async move {
        let app = Router::new().route("/rpc", post(rpc_handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tx.send(addr).unwrap();
        axum::serve(listener, app).await.unwrap();
    });

    rx.recv().unwrap()
}

/// Create an HttpTransport configured for benchmarking (no TLS, no retries, short timeout).
fn bench_transport() -> HttpTransport {
    let config = TransportConfig {
        timeout_ms: 5000,
        max_retries: 0,
        retry_delay_ms: 0,
        tls_config: TlsConfig {
            verify_cert: false,
            require_tls: false,
            ..Default::default()
        },
    };
    HttpTransport::new(config).unwrap()
}

fn bench_serialization(c: &mut Criterion) {
    let small = A2aRequest::new("ping", small_params());
    let medium = A2aRequest::new("tools/call", medium_params());
    let large = A2aRequest::new("context/update", large_params());

    c.bench_function("a2a_serialize_request_small", |b| {
        b.iter(|| serde_json::to_vec(black_box(&small)).unwrap())
    });

    c.bench_function("a2a_serialize_request_medium", |b| {
        b.iter(|| serde_json::to_vec(black_box(&medium)).unwrap())
    });

    c.bench_function("a2a_serialize_request_large", |b| {
        b.iter(|| serde_json::to_vec(black_box(&large)).unwrap())
    });

    let resp = success_response(Uuid::new_v4());
    let resp_bytes = serde_json::to_vec(&resp).unwrap();

    c.bench_function("a2a_deserialize_response", |b| {
        b.iter(|| serde_json::from_slice::<A2aResponse>(black_box(&resp_bytes)).unwrap())
    });

    c.bench_function("a2a_roundtrip_serde", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&small)).unwrap();
            serde_json::from_slice::<A2aRequest>(black_box(&bytes)).unwrap()
        })
    });
}

fn bench_http_roundtrip(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let addr = start_server(&rt);
    // Brief wait for server readiness
    rt.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(10)).await });

    let transport = bench_transport();
    let endpoint = A2aEndpoint {
        url: format!("http://{addr}"),
        agent_id: "bench-agent".to_string(),
        public_key: None,
    };
    rt.block_on(transport.connect(&endpoint)).unwrap();

    c.bench_function("a2a_http_roundtrip_small", |b| {
        b.iter(|| {
            rt.block_on(async {
                let req = A2aRequest::new("ping", small_params());
                transport.send_request(black_box(req)).await.unwrap()
            })
        })
    });

    c.bench_function("a2a_http_roundtrip_medium", |b| {
        b.iter(|| {
            rt.block_on(async {
                let req = A2aRequest::new("tools/call", medium_params());
                transport.send_request(black_box(req)).await.unwrap()
            })
        })
    });

    c.bench_function("a2a_http_roundtrip_large", |b| {
        b.iter(|| {
            rt.block_on(async {
                let req = A2aRequest::new("context/update", large_params());
                transport.send_request(black_box(req)).await.unwrap()
            })
        })
    });
}

fn bench_http_concurrent(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let addr = start_server(&rt);
    rt.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(10)).await });

    let transport = std::sync::Arc::new(bench_transport());
    let endpoint = A2aEndpoint {
        url: format!("http://{addr}"),
        agent_id: "bench-agent".to_string(),
        public_key: None,
    };
    rt.block_on(transport.connect(&endpoint)).unwrap();

    c.bench_function("a2a_http_concurrent_10", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut handles = Vec::with_capacity(10);
                for _ in 0..10 {
                    let t = transport.clone();
                    handles.push(tokio::spawn(async move {
                        let req = A2aRequest::new("ping", small_params());
                        t.send_request(req).await.unwrap()
                    }));
                }
                for h in handles {
                    h.await.unwrap();
                }
            })
        })
    });
}

// --- WebSocket benchmark server using jsonrpsee ---

/// JSON-RPC API that echoes back a fixed success response for any registered method.
#[jsonrpsee::proc_macros::rpc(server)]
trait BenchRpc {
    #[method(name = "ping")]
    async fn ping(&self) -> Result<serde_json::Value, ErrorObjectOwned>;

    #[method(name = "tools/call")]
    async fn tools_call(&self) -> Result<serde_json::Value, ErrorObjectOwned>;

    #[method(name = "context/update")]
    async fn context_update(&self) -> Result<serde_json::Value, ErrorObjectOwned>;
}

struct BenchRpcImpl;

#[async_trait]
impl BenchRpcServer for BenchRpcImpl {
    async fn ping(&self) -> Result<serde_json::Value, ErrorObjectOwned> {
        Ok(json!({"status": "ok"}))
    }

    async fn tools_call(&self) -> Result<serde_json::Value, ErrorObjectOwned> {
        Ok(json!({"status": "ok"}))
    }

    async fn context_update(&self) -> Result<serde_json::Value, ErrorObjectOwned> {
        Ok(json!({"status": "ok"}))
    }
}

/// Start a jsonrpsee server (HTTP + WS) on an ephemeral port, return the port.
fn start_ws_server(rt: &tokio::runtime::Runtime) -> u16 {
    let (tx, rx) = std::sync::mpsc::channel();

    rt.spawn(async move {
        let server = Server::builder().build("127.0.0.1:0").await.unwrap();
        let port = server.local_addr().unwrap().port();
        let _handle = server.start(BenchRpcImpl.into_rpc());
        tx.send(port).unwrap();
        // Keep server alive
        futures::future::pending::<()>().await;
    });

    rx.recv().unwrap()
}

/// Create a WebSocketTransport configured for benchmarking.
fn bench_ws_transport() -> WebSocketTransport {
    let config = TransportConfig {
        timeout_ms: 5000,
        max_retries: 0,
        retry_delay_ms: 0,
        tls_config: TlsConfig {
            verify_cert: false,
            require_tls: false,
            ..Default::default()
        },
    };
    WebSocketTransport::new(config)
}

fn bench_ws_roundtrip(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let port = start_ws_server(&rt);
    rt.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(10)).await });

    let transport = bench_ws_transport();
    let endpoint = A2aEndpoint {
        url: format!("ws://127.0.0.1:{port}"),
        agent_id: "bench-agent".to_string(),
        public_key: None,
    };
    rt.block_on(transport.connect(&endpoint)).unwrap();

    c.bench_function("a2a_ws_roundtrip_small", |b| {
        b.iter(|| {
            rt.block_on(async {
                let req = A2aRequest::new("ping", small_params());
                transport.send_request(black_box(req)).await.unwrap()
            })
        })
    });

    c.bench_function("a2a_ws_roundtrip_medium", |b| {
        b.iter(|| {
            rt.block_on(async {
                let req = A2aRequest::new("tools/call", medium_params());
                transport.send_request(black_box(req)).await.unwrap()
            })
        })
    });

    c.bench_function("a2a_ws_roundtrip_large", |b| {
        b.iter(|| {
            rt.block_on(async {
                let req = A2aRequest::new("context/update", large_params());
                transport.send_request(black_box(req)).await.unwrap()
            })
        })
    });
}

fn bench_ws_concurrent(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let port = start_ws_server(&rt);
    rt.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(10)).await });

    let transport = std::sync::Arc::new(bench_ws_transport());
    let endpoint = A2aEndpoint {
        url: format!("ws://127.0.0.1:{port}"),
        agent_id: "bench-agent".to_string(),
        public_key: None,
    };
    rt.block_on(transport.connect(&endpoint)).unwrap();

    c.bench_function("a2a_ws_concurrent_10", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut handles = Vec::with_capacity(10);
                for _ in 0..10 {
                    let t = transport.clone();
                    handles.push(tokio::spawn(async move {
                        let req = A2aRequest::new("ping", small_params());
                        t.send_request(req).await.unwrap()
                    }));
                }
                for h in handles {
                    h.await.unwrap();
                }
            })
        })
    });
}

criterion_group!(serialization, bench_serialization,);

criterion_group!(http_roundtrip, bench_http_roundtrip,);

criterion_group!(http_concurrent, bench_http_concurrent,);

criterion_group!(ws_roundtrip, bench_ws_roundtrip,);

criterion_group!(ws_concurrent, bench_ws_concurrent,);

criterion_main!(
    serialization,
    http_roundtrip,
    http_concurrent,
    ws_roundtrip,
    ws_concurrent
);
