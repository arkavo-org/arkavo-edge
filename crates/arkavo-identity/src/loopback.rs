//! OAuth redirect receiver on numeric `127.0.0.1` (never `localhost`).

use crate::error::IdentityError;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const LOOPBACK_PORTS: [u16; 8] = [52171, 52172, 52173, 52174, 52175, 52176, 52177, 52178];
pub const CALLBACK_DEADLINE: Duration = Duration::from_secs(300);

const CLOSE_WINDOW: &str = "You can close this window.";
const MAX_HEADERS: usize = 8192;

pub struct BoundCallback {
    pub listener: TcpListener,
    pub port: u16,
    pub redirect_uri: String, // exact "http://127.0.0.1:{port}/cb"
}

#[derive(Debug)]
pub enum Callback {
    Code { code: String, state: String },
    Error { error: String, state: String },
}

pub fn bind() -> Result<BoundCallback, IdentityError> {
    for port in LOOPBACK_PORTS {
        if let Ok(bound) = bind_port(port) {
            return Ok(bound);
        }
    }
    Err(IdentityError::Transport(
        "all loopback ports 52171-52178 are busy".into(),
    ))
}

pub async fn wait_for_callback(
    bound: BoundCallback,
    expected_state: &str,
    deadline: Duration,
) -> Result<Callback, IdentityError> {
    match tokio::time::timeout(deadline, accept_until_match(bound, expected_state)).await {
        Ok(result) => result,
        Err(_) => Err(IdentityError::TimedOut),
    }
}

fn bind_port(port: u16) -> Result<BoundCallback, IdentityError> {
    let std_listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| IdentityError::Transport(e.to_string()))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| IdentityError::Transport(e.to_string()))?;
    let port = std_listener
        .local_addr()
        .map_err(|e| IdentityError::Transport(e.to_string()))?
        .port();
    let listener =
        TcpListener::from_std(std_listener).map_err(|e| IdentityError::Transport(e.to_string()))?;
    Ok(BoundCallback {
        listener,
        port,
        redirect_uri: format!("http://127.0.0.1:{port}/cb"),
    })
}

#[cfg(test)]
fn bind_ephemeral_for_test() -> BoundCallback {
    bind_port(0).expect("ephemeral 127.0.0.1:0 bind")
}

async fn accept_until_match(
    bound: BoundCallback,
    expected_state: &str,
) -> Result<Callback, IdentityError> {
    loop {
        let (mut stream, _) = bound
            .listener
            .accept()
            .await
            .map_err(|e| IdentityError::Transport(e.to_string()))?;
        match read_and_parse(&mut stream, expected_state).await {
            Some(cb) => {
                let _ = write_http(&mut stream, 200, CLOSE_WINDOW).await;
                return Ok(cb);
            }
            None => {
                let _ = write_http(&mut stream, 400, "").await;
            }
        }
    }
}

async fn read_and_parse(stream: &mut TcpStream, expected_state: &str) -> Option<Callback> {
    parse_callback(&read_headers(stream).await?, expected_state)
}

async fn read_headers(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; MAX_HEADERS];
    let mut n = 0;
    while n < buf.len() {
        let got = stream.read(&mut buf[n..]).await.ok()?;
        if got == 0 {
            break;
        }
        n += got;
        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    buf.truncate(n);
    Some(buf)
}

async fn write_http(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.shutdown().await
}

fn parse_callback(buf: &[u8], expected_state: &str) -> Option<Callback> {
    let text = std::str::from_utf8(buf).ok()?;
    let line = text.split("\r\n").next()?;
    let mut parts = line.split(' ');
    let method = parts.next()?;
    let target = parts.next()?;
    let version = parts.next()?;
    if parts.next().is_some() || method != "GET" || version != "HTTP/1.1" {
        return None;
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/cb" {
        return None;
    }
    let mut code = None;
    let mut error = None;
    let mut state = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let k = percent_decode(k)?;
        let v = percent_decode(v)?;
        match k.as_str() {
            "code" => code = Some(v),
            "error" => error = Some(v),
            "state" => state = Some(v),
            _ => {}
        }
    }
    let state = state?;
    if state != expected_state {
        return None;
    }
    if let Some(code) = code {
        Some(Callback::Code { code, state })
    } else {
        Some(Callback::Error {
            error: error?,
            state,
        })
    }
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let hi = from_hex(bytes[i + 1])?;
                let lo = from_hex(bytes[i + 2])?;
                out.push((hi << 4) | lo);
                i += 3;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // #[tokio::test] expands to Runtime::block_on
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn matching_code_unblocks_and_mismatch_stays_open() {
        let bound = bind_ephemeral_for_test(); // test-only bind on 127.0.0.1:0, record port
        let port = bound.port;
        let expected = "good-state";
        let waiter = tokio::spawn(async move {
            wait_for_callback(bound, expected, Duration::from_secs(5)).await
        });
        // mismatch
        let _ = send_get(port, "/cb?code=nope&state=wrong").await;
        // match
        let _ = send_get(port, "/cb?code=abc&state=good-state").await;
        let cb = waiter.await.unwrap().unwrap();
        match cb {
            Callback::Code { code, state } => {
                assert_eq!(code, "abc");
                assert_eq!(state, "good-state");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn error_callback_with_matching_state_is_accepted() {
        let bound = bind_ephemeral_for_test();
        let port = bound.port;
        let waiter =
            tokio::spawn(
                async move { wait_for_callback(bound, "s", Duration::from_secs(5)).await },
            );
        let _ = send_get(port, "/cb?error=access_denied&state=s").await;
        match waiter.await.unwrap().unwrap() {
            Callback::Error { error, .. } => assert_eq!(error, "access_denied"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn deadline_without_callback_is_timed_out() {
        let bound = bind_ephemeral_for_test();
        let err = wait_for_callback(bound, "s", Duration::from_millis(80))
            .await
            .unwrap_err();
        assert!(matches!(err, IdentityError::TimedOut));
    }

    #[tokio::test]
    async fn percent_decodes_query_values() {
        let bound = bind_ephemeral_for_test();
        let port = bound.port;
        let waiter =
            tokio::spawn(
                async move { wait_for_callback(bound, "s t", Duration::from_secs(5)).await },
            );
        let _ = send_get(port, "/cb?code=a%2Fb&state=s%20t").await;
        match waiter.await.unwrap().unwrap() {
            Callback::Code { code, state } => {
                assert_eq!(code, "a/b");
                assert_eq!(state, "s t");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn matching_request_writes_close_window_body() {
        let bound = bind_ephemeral_for_test();
        let port = bound.port;
        let waiter =
            tokio::spawn(
                async move { wait_for_callback(bound, "s", Duration::from_secs(5)).await },
            );
        let body = send_get(port, "/cb?code=x&state=s").await.unwrap();
        waiter.await.unwrap().unwrap();
        assert!(body.starts_with("HTTP/1.1 200"));
        assert!(body.contains(CLOSE_WINDOW));
    }

    #[tokio::test]
    async fn mismatch_is_http_400_then_match_unblocks() {
        let bound = bind_ephemeral_for_test();
        let port = bound.port;
        let waiter =
            tokio::spawn(
                async move { wait_for_callback(bound, "good", Duration::from_secs(5)).await },
            );
        let mismatch = send_get(port, "/cb?code=x&state=bad").await.unwrap();
        assert!(mismatch.starts_with("HTTP/1.1 400"));
        let _ = send_get(port, "/cb?code=x&state=good").await;
        waiter.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn non_cb_path_stays_open() {
        let bound = bind_ephemeral_for_test();
        let port = bound.port;
        let waiter =
            tokio::spawn(
                async move { wait_for_callback(bound, "s", Duration::from_secs(5)).await },
            );
        let rejected = send_get(port, "/other?code=x&state=s").await.unwrap();
        assert!(rejected.starts_with("HTTP/1.1 400"));
        let _ = send_get(port, "/cb?code=ok&state=s").await;
        match waiter.await.unwrap().unwrap() {
            Callback::Code { code, .. } => assert_eq!(code, "ok"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn registered_loopback_uris_and_deadline() {
        assert_eq!(
            LOOPBACK_PORTS,
            [52171, 52172, 52173, 52174, 52175, 52176, 52177, 52178]
        );
        assert_eq!(CALLBACK_DEADLINE, Duration::from_secs(300));
        assert_eq!(
            format!("http://127.0.0.1:{}/cb", LOOPBACK_PORTS[0]),
            "http://127.0.0.1:52171/cb"
        );
    }

    async fn send_get(port: u16, path: &str) -> std::io::Result<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
        let req =
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).await?;
        let mut buf = String::new();
        stream.read_to_string(&mut buf).await?;
        Ok(buf)
    }
}
