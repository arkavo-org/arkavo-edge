//! NET-007, NET-014, NET-018: an agent-path request never reaches a blocked
//! host, whichever of the three routes it takes there.
//!
//! Every "must not reach" case is proved on a real listener: the request is
//! aimed at a socket this test owns, and the socket's accept queue is empty
//! afterwards. An error alone would not show the connection was never made.
#![allow(clippy::disallowed_methods)] // tokio::test expands to Runtime::block_on

use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use arkavo_test_macros::spec;
use arkavo_validation::{EgressClient, EgressError, EgressPolicy};

fn silent_listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

/// The client has finished (it returned an error), so any connection it made
/// has completed its handshake and is waiting in the accept queue.
fn assert_untouched(listener: &TcpListener) {
    match listener.accept() {
        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
        Ok((_, peer)) => panic!("egress reached the listener from {peer}"),
        Err(e) => panic!("listener failed: {e}"),
    }
}

/// Answer one request with `response`, on a thread so the client under test
/// can run on the test's runtime.
fn serve_once(response: String) -> u16 {
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
        stream.write_all(response.as_bytes()).unwrap();
    });
    port
}

fn redirect_to(location: &str) -> String {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";

/// The timeout turns a regression into a failure: a request that reached a
/// silent listener would otherwise wait forever for a response it never gets.
fn client(allowlist: &str) -> EgressClient {
    EgressClient::builder()
        .policy(Arc::new(EgressPolicy::with_allowlist(allowlist).unwrap()))
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

fn error_chain(err: &reqwest::Error) -> String {
    let mut chain = err.to_string();
    let mut source = std::error::Error::source(err);
    while let Some(cause) = source {
        chain.push_str(" <- ");
        chain.push_str(&cause.to_string());
        source = cause.source();
    }
    chain
}

#[spec("NET-007")]
#[tokio::test]
async fn an_ip_literal_loopback_url_is_refused_before_connecting() {
    let (listener, port) = silent_listener();
    let client = client("");

    for target in [
        format!("http://127.0.0.1:{port}/"),
        format!("http://2130706433:{port}/"),
        format!("http://[::ffff:127.0.0.1]:{port}/"),
    ] {
        assert!(
            matches!(client.get(&target), Err(EgressError::BlockedIp(_))),
            "{target} must be refused"
        );
    }
    assert_untouched(&listener);
}

#[spec("NET-014")]
#[tokio::test]
async fn a_name_resolving_to_loopback_is_refused_at_resolution() {
    let (listener, port) = silent_listener();

    let err = client("")
        .get(&format!("http://localhost:{port}/"))
        .unwrap()
        .send()
        .await
        .unwrap_err();

    let chain = error_chain(&err);
    assert!(chain.contains("SSRF attempt blocked"), "{chain}");
    assert_untouched(&listener);
}

#[spec("NET-007", "NET-018")]
#[tokio::test]
async fn a_redirect_to_a_loopback_literal_is_not_followed() {
    let (target, target_port) = silent_listener();
    let redirector = serve_once(redirect_to(&format!("http://127.0.0.1:{target_port}/")));

    let err = client(&format!("http://127.0.0.1:{redirector}"))
        .get(&format!("http://127.0.0.1:{redirector}/"))
        .unwrap()
        .send()
        .await
        .unwrap_err();

    assert!(err.is_redirect(), "{}", error_chain(&err));
    assert!(error_chain(&err).contains("SSRF attempt blocked"));
    assert_untouched(&target);
}

#[spec("NET-014", "NET-018")]
#[tokio::test]
async fn a_redirect_to_a_loopback_name_is_not_followed() {
    let (target, target_port) = silent_listener();
    let redirector = serve_once(redirect_to(&format!("http://localhost:{target_port}/")));

    let err = client(&format!("http://127.0.0.1:{redirector}"))
        .get(&format!("http://127.0.0.1:{redirector}/"))
        .unwrap()
        .send()
        .await
        .unwrap_err();

    assert!(
        error_chain(&err).contains("SSRF attempt blocked"),
        "{}",
        error_chain(&err)
    );
    assert_untouched(&target);
}

#[spec("NET-018")]
#[tokio::test]
async fn an_allowlisted_origin_is_reached() {
    let port = serve_once(OK.to_string());

    let body = client(&format!("http://127.0.0.1:{port}"))
        .get(&format!("http://127.0.0.1:{port}/"))
        .unwrap()
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert_eq!(body, "ok");
}

/// The resolver never sees the port, so this is the test that proves the
/// URL-level port check closes the gap an exempted name would open.
#[spec("NET-018")]
#[tokio::test]
async fn an_allowlisted_name_is_reached_on_its_port_and_no_other() {
    let allowed = serve_once(OK.to_string());
    let (other, other_port) = silent_listener();
    let client = client(&format!("http://localhost:{allowed}"));

    let body = client
        .get(&format!("http://localhost:{allowed}/"))
        .unwrap()
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(body, "ok");

    assert_eq!(
        client.get(&format!("http://localhost:{other_port}/")).err(),
        Some(EgressError::BlockedDomain(format!(
            "localhost:{other_port}"
        )))
    );
    assert_untouched(&other);
}

/// `hops` allowlisted redirectors in a row, the last pointing at `port`.
/// Returns the first redirector's port and an allowlist admitting every hop
/// and `port`, so the hop limit is the only thing that can stop the chain.
fn redirect_chain(hops: usize, port: u16) -> (u16, String) {
    let mut allow = vec![format!("http://127.0.0.1:{port}")];
    let mut next = port;
    for _ in 0..hops {
        next = serve_once(redirect_to(&format!("http://127.0.0.1:{next}/")));
        allow.push(format!("http://127.0.0.1:{next}"));
    }
    (next, allow.join(","))
}

#[spec("NET-014")]
#[tokio::test]
async fn ten_redirects_are_followed_and_an_eleventh_is_not() {
    let (first, allow) = redirect_chain(10, serve_once(OK.to_string()));
    let body = client(&allow)
        .get(&format!("http://127.0.0.1:{first}/"))
        .unwrap()
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(body, "ok");

    let (target, target_port) = silent_listener();
    let (first, allow) = redirect_chain(11, target_port);
    let err = client(&allow)
        .get(&format!("http://127.0.0.1:{first}/"))
        .unwrap()
        .send()
        .await
        .unwrap_err();
    assert!(err.is_redirect(), "{}", error_chain(&err));
    assert_untouched(&target);
}
