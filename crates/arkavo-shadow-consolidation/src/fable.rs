//! Minimal Fable client over raw rustls HTTP.
//!
//! This is deliberately not the runtime LLM provider: the audit-plane job
//! talks to Fable directly so it fully controls the request shape (adaptive
//! thinking, no sampling params) and reads the exact `usage` block it needs to
//! build a real cost ledger. No arkavo runtime crate is involved.

use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::{Value, json};

/// Fable 5 list price, USD per million tokens (input).
pub const FABLE_INPUT_USD_PER_MTOK: f64 = 10.0;
/// Fable 5 list price, USD per million tokens (output, thinking billed here).
pub const FABLE_OUTPUT_USD_PER_MTOK: f64 = 50.0;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// Generous ceiling — adaptive thinking at high effort can take a while, and
/// this is an overnight job, not an interactive request.
const REQUEST_TIMEOUT_SECS: u64 = 600;

/// Token usage from a Fable completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

impl Usage {
    /// Cost of this completion at Fable list price (thinking is billed as
    /// output tokens by the API, so it is already included here).
    #[must_use]
    pub fn cost_usd(&self) -> f64 {
        let output_cost = (self.output_tokens as f64 / 1_000_000.0) * FABLE_OUTPUT_USD_PER_MTOK;
        (self.input_tokens as f64 / 1_000_000.0).mul_add(FABLE_INPUT_USD_PER_MTOK, output_cost)
    }
}

/// A completed Fable call.
#[derive(Debug, Clone)]
pub struct FableCompletion {
    /// Concatenated text blocks (thinking blocks are excluded).
    pub text: String,
    pub usage: Usage,
    pub latency_ms: u64,
}

/// Attempts per call on transient failures; an unattended overnight run must
/// outlive a 3 a.m. overload spike, not die on it.
const DEFAULT_RETRY_ATTEMPTS: u32 = 3;
/// First backoff; doubles per retry.
const DEFAULT_RETRY_BACKOFF_MS: u64 = 2_000;

/// Client configuration.
#[derive(Debug, Clone)]
pub struct FableConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_tokens: u32,
    pub effort: String,
    pub retry_attempts: u32,
    pub retry_backoff_ms: u64,
}

impl FableConfig {
    /// Build configuration from the environment. Requires `ANTHROPIC_API_KEY`;
    /// honors `ANTHROPIC_BASE_URL` for proxies / gateways.
    pub fn from_env(model: String, max_tokens: u32, effort: String) -> anyhow::Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY must be set to call Fable (use --dry-run to skip)")?;
        anyhow::ensure!(!api_key.trim().is_empty(), "ANTHROPIC_API_KEY is empty");
        let base_url =
            std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Ok(Self {
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            max_tokens,
            effort,
            retry_attempts: DEFAULT_RETRY_ATTEMPTS,
            retry_backoff_ms: DEFAULT_RETRY_BACKOFF_MS,
        })
    }
}

/// Build the `/v1/messages` request body for a Fable consolidation call.
///
/// Fable 5 requires adaptive thinking and rejects sampling parameters, so the
/// body carries `thinking: {type: adaptive}` and no `temperature` / `top_p`.
#[must_use]
pub fn build_request_body(cfg: &FableConfig, system: &str, user: &str) -> Value {
    json!({
        "model": cfg.model,
        "max_tokens": cfg.max_tokens,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
        "thinking": { "type": "adaptive" },
        "output_config": { "effort": cfg.effort }
    })
}

/// Extract and concatenate `text` blocks from an Anthropic `content` array.
fn extract_text(content: &[Value]) -> String {
    content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    #[serde(default)]
    content: Vec<Value>,
    usage: Usage,
}

/// Fable client.
pub struct FableClient {
    http: reqwest::Client,
    cfg: FableConfig,
}

impl FableClient {
    pub fn new(cfg: FableConfig) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .context("building HTTP client")?;
        Ok(Self { http, cfg })
    }

    /// The model id this client calls.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.cfg.model
    }

    /// Run one consolidation completion, retrying transient failures with
    /// exponential backoff. A decode failure after a successful status is
    /// never retried: the call was already billed, and re-sending would bill
    /// it again.
    pub async fn complete(&self, system: &str, user: &str) -> anyhow::Result<FableCompletion> {
        let body = build_request_body(&self.cfg, system, user);
        let url = format!("{}/v1/messages", self.cfg.base_url);

        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let started = Instant::now();
            let result = self
                .http
                .post(&url)
                .header("x-api-key", &self.cfg.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await;

            let err = match result {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let parsed: ApiResponse =
                            resp.json().await.context("decoding Fable response")?;
                        let latency_ms = started.elapsed().as_millis() as u64;
                        return Ok(FableCompletion {
                            text: extract_text(&parsed.content),
                            usage: parsed.usage,
                            latency_ms,
                        });
                    }
                    let detail = resp.text().await.unwrap_or_default();
                    if !retryable(status) {
                        bail!("Fable API error {status}: {detail}");
                    }
                    anyhow::anyhow!("Fable API error {status}: {detail}")
                }
                Err(e) => anyhow::Error::new(e).context("sending Fable request"),
            };

            if attempt >= self.cfg.retry_attempts.max(1) {
                return Err(err.context(format!("giving up after {attempt} attempts")));
            }
            let backoff = Duration::from_millis(self.cfg.retry_backoff_ms << (attempt - 1));
            tokio::time::sleep(backoff).await;
        }
    }
}

/// Transient statuses worth another attempt on an unattended run: rate
/// limiting (429) and server-side errors including Anthropic overload (529).
fn retryable(status: reqwest::StatusCode) -> bool {
    status.as_u16() == 429 || status.is_server_error()
}

#[cfg(test)]
mod tests {
    // #[tokio::test] expands to Runtime::block_on; harmless in tests.
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn cfg() -> FableConfig {
        FableConfig {
            api_key: "k".into(),
            base_url: "https://api.anthropic.com".into(),
            model: "claude-fable-5".into(),
            max_tokens: 8192,
            effort: "high".into(),
            retry_attempts: 3,
            retry_backoff_ms: 1,
        }
    }

    const SUCCESS_BODY: &str = r#"{"content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":10,"output_tokens":5}}"#;

    /// Serve the given (status, body) responses, one connection each, then
    /// stop listening. Plain HTTP/1.1 over a loopback socket — enough to
    /// exercise the client's status handling without any TLS or HTTP dep.
    async fn serve(responses: Vec<(u16, &'static str)>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut sock, _) = listener.accept().await.unwrap();
                read_request(&mut sock).await;
                let resp = format!(
                    "HTTP/1.1 {status} STATUS\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
            }
        });
        addr
    }

    /// Read headers plus the content-length body so the client never sees a
    /// reset mid-request.
    async fn read_request(sock: &mut tokio::net::TcpStream) {
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 {
                return;
            }
            data.extend_from_slice(&buf[..n]);
            if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&data[..pos]).to_lowercase();
                let len = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if data.len() >= pos + 4 + len {
                    return;
                }
            }
        }
    }

    fn local_cfg(addr: SocketAddr) -> FableConfig {
        FableConfig {
            base_url: format!("http://{addr}"),
            ..cfg()
        }
    }

    #[tokio::test]
    async fn retries_transient_overload_then_succeeds() {
        let addr = serve(vec![
            (529, "overloaded"),
            (429, "rate limited"),
            (200, SUCCESS_BODY),
        ])
        .await;
        let client = FableClient::new(local_cfg(addr)).unwrap();
        let completion = client.complete("sys", "usr").await.unwrap();
        assert_eq!(completion.text, "ok");
        assert_eq!(completion.usage.input_tokens, 10);
        assert_eq!(completion.usage.output_tokens, 5);
    }

    #[tokio::test]
    async fn non_retryable_status_fails_without_retry() {
        // One served response only: a retry would hit a closed listener and
        // surface a connect error instead of the 400 detail asserted here.
        let addr = serve(vec![(400, "bad request")]).await;
        let client = FableClient::new(local_cfg(addr)).unwrap();
        let err = client.complete("sys", "usr").await.unwrap_err();
        assert!(
            format!("{err:#}").contains("400"),
            "unexpected error: {err:#}"
        );
    }

    #[tokio::test]
    async fn exhausts_retries_and_reports_last_error() {
        let addr = serve(vec![(500, "boom"), (500, "boom"), (500, "boom")]).await;
        let client = FableClient::new(local_cfg(addr)).unwrap();
        let err = client.complete("sys", "usr").await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("500"), "unexpected error: {msg}");
        assert!(msg.contains("attempts"), "unexpected error: {msg}");
    }

    #[test]
    fn request_body_uses_adaptive_thinking_and_no_sampling() {
        let body = build_request_body(&cfg(), "sys", "usr");
        assert_eq!(body["model"], "claude-fable-5");
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(body["system"], "sys");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "usr");
        // Fable rejects sampling params — they must be absent.
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
    }

    #[test]
    fn cost_matches_fable_list_price() {
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
        };
        // $10 input + $50 output per MTok.
        assert!((usage.cost_usd() - 60.0).abs() < 1e-9);

        let small = Usage {
            input_tokens: 2000,
            output_tokens: 800,
        };
        let input_cost = 2000.0 / 1e6 * 10.0;
        let output_cost = 800.0 / 1e6 * 50.0;
        let expected = input_cost + output_cost;
        assert!((small.cost_usd() - expected).abs() < 1e-12);
    }

    #[test]
    fn extracts_only_text_blocks() {
        let content = vec![
            json!({"type": "thinking", "thinking": "reasoning"}),
            json!({"type": "text", "text": "hello"}),
            json!({"type": "text", "text": "world"}),
        ];
        assert_eq!(extract_text(&content), "hello\nworld");
    }

    #[test]
    fn from_env_requires_key() {
        // SAFETY: single-threaded test; restore afterwards.
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
        }
        let err = FableConfig::from_env("claude-fable-5".into(), 8192, "high".into()).unwrap_err();
        assert!(err.to_string().contains("ANTHROPIC_API_KEY"));
    }
}
