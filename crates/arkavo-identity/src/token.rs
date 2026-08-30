//! Authorization-code exchange and refresh-token rotation for `arkavo-edge`.

use crate::error::IdentityError;
use crate::store::StoredTokens;

const CLIENT_ID: &str = "arkavo-edge";

pub async fn exchange_code(
    http: &reqwest::Client,
    token_endpoint: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<StoredTokens, IdentityError> {
    request_tokens(
        http,
        token_endpoint,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", CLIENT_ID),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ],
    )
    .await
}

pub async fn refresh(
    http: &reqwest::Client,
    token_endpoint: &str,
    refresh_token: &str,
) -> Result<StoredTokens, IdentityError> {
    request_tokens(
        http,
        token_endpoint,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ],
    )
    .await
}

async fn request_tokens(
    http: &reqwest::Client,
    token_endpoint: &str,
    form: &[(&str, &str)],
) -> Result<StoredTokens, IdentityError> {
    let response = http
        .post(token_endpoint)
        .form(form)
        .send()
        .await
        .map_err(|e| IdentityError::Transport(e.to_string()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| IdentityError::Transport(e.to_string()))?;
    if !status.is_success() {
        return Err(IdentityError::Token(text));
    }
    parse_stored_tokens(&text)
}

fn parse_stored_tokens(text: &str) -> Result<StoredTokens, IdentityError> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| IdentityError::Token(format!("invalid token JSON: {e}")))?;
    let access_token = value
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IdentityError::Token("missing access_token".into()))?
        .to_owned();
    let refresh_token = value
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let expires_in = value
        .get("expires_in")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok()))
        })
        .unwrap_or(0);
    Ok(StoredTokens {
        access_token,
        refresh_token,
        expires_at: unix_now().saturating_add(expires_in),
    })
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // #[tokio::test] expands to Runtime::block_on
mod tests {
    use super::*;
    use crate::error::IdentityError;
    use std::collections::HashMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap()
    }

    async fn serve_token(
        status: u16,
        json_body: impl Into<String>,
    ) -> (
        std::net::SocketAddr,
        tokio::sync::oneshot::Receiver<HashMap<String, String>>,
        tokio::task::JoinHandle<()>,
    ) {
        let json_body = json_body.into();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let mut tx = Some(tx);
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let json_body = json_body.clone();
                let tx = tx.take();
                tokio::spawn(async move {
                    let form = read_form(&mut stream).await;
                    if let Some(tx) = tx {
                        let _ = tx.send(form);
                    }
                    write_json_response(&mut stream, status, &json_body).await;
                });
            }
        });
        (addr, rx, handle)
    }

    async fn read_form(stream: &mut tokio::net::TcpStream) -> HashMap<String, String> {
        let body = read_http_body(stream).await.unwrap_or_default();
        url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect()
    }

    async fn read_http_body(stream: &mut tokio::net::TcpStream) -> Option<String> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        loop {
            let n = stream.read(&mut tmp).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = std::str::from_utf8(&buf[..header_end]).ok()?;
                let content_length = headers.lines().find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                });
                let body_start = header_end + 4;
                let want = content_length.unwrap_or(0);
                while buf.len() < body_start + want {
                    let n = stream.read(&mut tmp).await.ok()?;
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                let end = (body_start + want).min(buf.len());
                return std::str::from_utf8(&buf[body_start..end])
                    .ok()
                    .map(str::to_owned);
            }
            if buf.len() > 64 * 1024 {
                return None;
            }
        }
        None
    }

    async fn write_json_response(stream: &mut tokio::net::TcpStream, status: u16, body: &str) {
        let reason = if (200..300).contains(&status) {
            "OK"
        } else {
            "Error"
        };
        let header = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes()).await;
        let _ = stream.write_all(body.as_bytes()).await;
        let _ = stream.shutdown().await;
    }

    #[tokio::test]
    async fn exchange_sends_verifier_and_exact_redirect_uri() {
        let (addr, form_rx, handle) = serve_token(
            200,
            r#"{"access_token":"atk","refresh_token":"rtk","expires_in":3600,"token_type":"Bearer"}"#,
        )
        .await;
        let http = http_client();
        let redirect_uri = "http://127.0.0.1:52171/cb";
        let tokens = exchange_code(
            &http,
            &format!("http://{addr}/oauth/token"),
            "abc",
            redirect_uri,
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        )
        .await
        .expect("token exchange");
        let form = form_rx.await.expect("form body");
        assert_eq!(
            form.get("grant_type").map(String::as_str),
            Some("authorization_code")
        );
        assert_eq!(form.get("code").map(String::as_str), Some("abc"));
        assert_eq!(
            form.get("client_id").map(String::as_str),
            Some("arkavo-edge")
        );
        assert_eq!(
            form.get("redirect_uri").map(String::as_str),
            Some(redirect_uri)
        );
        assert_eq!(
            form.get("code_verifier").map(String::as_str),
            Some("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk")
        );
        assert!(
            !form.contains_key("client_secret"),
            "public client must not send a secret"
        );
        assert_eq!(tokens.access_token, "atk");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rtk"));
        let expected = unix_now() + 3600;
        assert!(
            (tokens.expires_at - expected).abs() <= 5,
            "expires_at={} expected around {expected}",
            tokens.expires_at
        );
        handle.abort();
    }

    #[tokio::test]
    async fn refresh_returns_rotated_refresh_token() {
        let (addr, form_rx, handle) = serve_token(
            200,
            r#"{"access_token":"atk2","refresh_token":"r2","expires_in":3600}"#,
        )
        .await;
        let http = http_client();
        let tokens = refresh(&http, &format!("http://{addr}/oauth/token"), "r1")
            .await
            .expect("refresh");
        let form = form_rx.await.expect("form body");
        assert_eq!(
            form.get("grant_type").map(String::as_str),
            Some("refresh_token")
        );
        assert_eq!(form.get("refresh_token").map(String::as_str), Some("r1"));
        assert_eq!(
            form.get("client_id").map(String::as_str),
            Some("arkavo-edge")
        );
        assert!(
            !form.contains_key("client_secret"),
            "public client must not send a secret"
        );
        assert_eq!(tokens.access_token, "atk2");
        assert_eq!(tokens.refresh_token.as_deref(), Some("r2"));
        handle.abort();
    }

    #[tokio::test]
    async fn non_2xx_is_token_error_containing_body() {
        let (addr, _form_rx, handle) = serve_token(400, r#"{"error":"invalid_grant"}"#).await;
        let http = http_client();
        let err = exchange_code(
            &http,
            &format!("http://{addr}/oauth/token"),
            "abc",
            "http://127.0.0.1:52171/cb",
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        )
        .await
        .expect_err("non-2xx must fail");
        match err {
            IdentityError::Token(body) => {
                assert!(
                    body.contains(r#"{"error":"invalid_grant"}"#) || body.contains("invalid_grant"),
                    "Token must contain the response body, got {body}"
                );
            }
            other => panic!("expected IdentityError::Token, got {other:?}"),
        }
        handle.abort();
    }

    #[tokio::test]
    async fn missing_access_token_is_token_error() {
        let (addr, _form_rx, handle) =
            serve_token(200, r#"{"refresh_token":"r","expires_in":3600}"#).await;
        let http = http_client();
        let err = exchange_code(
            &http,
            &format!("http://{addr}/oauth/token"),
            "abc",
            "http://127.0.0.1:52171/cb",
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        )
        .await
        .expect_err("200 without access_token must fail");
        assert!(
            matches!(err, IdentityError::Token(_)),
            "expected IdentityError::Token, got {err:?}"
        );
        handle.abort();
    }
}
