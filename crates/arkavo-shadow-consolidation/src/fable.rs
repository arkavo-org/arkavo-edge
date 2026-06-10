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

/// Client configuration.
#[derive(Debug, Clone)]
pub struct FableConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_tokens: u32,
    pub effort: String,
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

    /// Run one consolidation completion.
    pub async fn complete(&self, system: &str, user: &str) -> anyhow::Result<FableCompletion> {
        let body = build_request_body(&self.cfg, system, user);
        let url = format!("{}/v1/messages", self.cfg.base_url);

        let started = Instant::now();
        let resp = self
            .http
            .post(&url)
            .header("x-api-key", &self.cfg.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("sending Fable request")?;

        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().await.unwrap_or_default();
            bail!("Fable API error {status}: {detail}");
        }

        let parsed: ApiResponse = resp.json().await.context("decoding Fable response")?;
        let latency_ms = started.elapsed().as_millis() as u64;

        Ok(FableCompletion {
            text: extract_text(&parsed.content),
            usage: parsed.usage,
            latency_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> FableConfig {
        FableConfig {
            api_key: "k".into(),
            base_url: "https://api.anthropic.com".into(),
            model: "claude-fable-5".into(),
            max_tokens: 8192,
            effort: "high".into(),
        }
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
