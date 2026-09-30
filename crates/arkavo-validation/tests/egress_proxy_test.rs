//! NET-007: the egress client ignores proxy environment variables.
//!
//! A proxy resolves the destination itself, out of the resolver's sight, so a
//! client that honoured `HTTP_PROXY` would have the policy judge the proxy and
//! never the target. Environment variables are process-wide and reqwest reads
//! them when the client is built, so the request is made by a re-executed copy
//! of this test binary whose environment this test controls.
#![allow(clippy::disallowed_methods)] // tokio::test expands to Runtime::block_on

use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use arkavo_test_macros::spec;
use arkavo_validation::{EgressClient, EgressPolicy};

const TARGET_PORT_ENV: &str = "ARKAVO_TEST_PROXY_HELPER_TARGET_PORT";
const HELPER: &str = "helper_requests_the_target_with_proxy_variables_set";

/// Runs only in the re-executed child, where the parent has set the target's
/// port; in an ordinary run there is nothing to do and it passes.
#[tokio::test]
async fn helper_requests_the_target_with_proxy_variables_set() {
    let Ok(port) = std::env::var(TARGET_PORT_ENV) else {
        return;
    };
    let origin = format!("http://127.0.0.1:{port}");
    let client = EgressClient::builder()
        .policy(Arc::new(EgressPolicy::with_allowlist(&origin).unwrap()))
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let body = client
        .get(&format!("{origin}/"))
        .unwrap()
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert_eq!(body, "ok");
}

fn serve_once_ok() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut seen = Vec::new();
        let mut buf = [0u8; 1024];
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            seen.extend_from_slice(&buf[..n]);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .unwrap();
    });
    port
}

#[spec("NET-007")]
#[test]
fn proxy_environment_variables_are_ignored() {
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let proxy_url = format!("http://127.0.0.1:{}", proxy.local_addr().unwrap().port());
    let target_port = serve_once_ok();

    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", HELPER, "--test-threads=1"])
        .env(TARGET_PORT_ENV, target_port.to_string())
        .env_remove("NO_PROXY")
        .env_remove("no_proxy");
    for name in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        child.env(name, &proxy_url);
    }
    let output = child.output().unwrap();

    assert!(
        output.status.success(),
        "the helper did not reach the target directly\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("test result: ok. 1 passed"),
        "the helper did not run: {}",
        String::from_utf8_lossy(&output.stdout),
    );
    match proxy.accept() {
        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
        Ok((_, peer)) => panic!("the request went through the proxy, connecting from {peer}"),
        Err(e) => panic!("proxy listener failed: {e}"),
    }
}
