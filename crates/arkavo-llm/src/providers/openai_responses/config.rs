use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// Astra accepts up to this many output tokens per response.
const MAX_OUTPUT_TOKENS: usize = 128_000;

/// Astra supports deliberate reasoning at every effort level.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OpenAIReasoningEffort {
    Low,
    #[default]
    Medium,
    High,
    Xhigh,
    Max,
}

impl OpenAIReasoningEffort {
    /// Total HTTP budget for one Responses call at this effort.
    ///
    /// A single ceiling for every tier is wrong in both directions: it makes a
    /// low-effort agent loop wait a quarter of an hour on a wedged connection,
    /// and it cuts off the deliberate tiers that legitimately think for longer.
    pub fn request_timeout_secs(self) -> u64 {
        match self {
            Self::Low => 300,
            Self::Medium => 600,
            Self::High => 1800,
            Self::Xhigh => 2400,
            Self::Max => 3600,
        }
    }

    /// How long a stream may deliver nothing before it counts as stalled.
    ///
    /// Reasoning happens before the first output event, so the idle budget has
    /// to cover a whole deliberation at this effort — but it stays well under
    /// [`Self::request_timeout_secs`] so a dead connection fails on its own
    /// evidence instead of consuming the entire request budget.
    pub fn stream_idle_timeout_secs(self) -> u64 {
        match self {
            Self::Low => 120,
            Self::Medium => 180,
            Self::High => 300,
            Self::Xhigh => 420,
            Self::Max => 600,
        }
    }
}

#[derive(Clone)]
pub struct OpenAIResponsesConfig {
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub reasoning_effort: OpenAIReasoningEffort,
    pub max_output_tokens: usize,
    /// Override the effort's idle budget for streamed responses.
    /// `None` uses [`OpenAIReasoningEffort::stream_idle_timeout_secs`].
    pub stream_idle_timeout: Option<Duration>,
}

impl fmt::Debug for OpenAIResponsesConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAIResponsesConfig")
            .field("model", &self.model)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("stream_idle_timeout", &self.stream_idle_timeout)
            .finish_non_exhaustive()
    }
}

impl Default for OpenAIResponsesConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-6-astra".into(),
            reasoning_effort: OpenAIReasoningEffort::Medium,
            max_output_tokens: 16_384,
            stream_idle_timeout: None,
        }
    }
}

/// The one bound on a response's output-token cap.
///
/// Both the configured default and a per-call override run through here so a
/// caller-supplied cap cannot bypass the limit the configuration enforces.
pub(super) fn check_max_tokens(max_tokens: usize) -> Result<()> {
    if max_tokens == 0 || max_tokens > MAX_OUTPUT_TOKENS {
        return Err(Error::Config(
            "Astra output token limit must be 1..=128000".into(),
        ));
    }
    Ok(())
}

impl OpenAIResponsesConfig {
    /// Total HTTP budget, scaled by the configured reasoning effort.
    pub(super) fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.reasoning_effort.request_timeout_secs())
    }

    /// Idle budget for a streamed response.
    pub(super) fn stream_idle_timeout(&self) -> Duration {
        self.stream_idle_timeout.unwrap_or_else(|| {
            Duration::from_secs(self.reasoning_effort.stream_idle_timeout_secs())
        })
    }

    pub(crate) fn validate(&self) -> Result<url::Url> {
        let url = url::Url::parse(&self.base_url)
            .map_err(|_| Error::Config("Invalid OpenAI Responses base URL".into()))?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::Config("OpenAI Responses requires HTTPS (HTTP allowed on loopback only) and a credential-free URL".into()));
        }
        if self.model != "gpt-6-astra" {
            return Err(Error::Config(
                "OpenAI Responses requires gpt-6-astra".into(),
            ));
        }
        check_max_tokens(self.max_output_tokens)?;
        // An idle budget above the request budget could never fire, leaving a
        // stalled stream to consume the whole call.
        if self
            .stream_idle_timeout
            .is_some_and(|idle| idle.is_zero() || idle > self.request_timeout())
        {
            return Err(Error::Config(
                "Responses stream idle timeout must be within the request timeout".into(),
            ));
        }
        Ok(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stuck low-effort agent loop must fail long before a `max`-effort
    /// deliberation would, and no tier may wait longer than the hour ceiling.
    #[arkavo_test_macros::spec("ASTRA-001")]
    #[test]
    fn request_timeout_grows_with_effort_and_bounds_the_idle_budget() {
        let efforts = [
            OpenAIReasoningEffort::Low,
            OpenAIReasoningEffort::Medium,
            OpenAIReasoningEffort::High,
            OpenAIReasoningEffort::Xhigh,
            OpenAIReasoningEffort::Max,
        ];
        for pair in efforts.windows(2) {
            assert!(
                pair[0].request_timeout_secs() < pair[1].request_timeout_secs(),
                "{:?} must not wait as long as {:?}",
                pair[0],
                pair[1]
            );
            assert!(pair[0].stream_idle_timeout_secs() < pair[1].stream_idle_timeout_secs());
        }
        assert_eq!(OpenAIReasoningEffort::Low.request_timeout_secs(), 300);
        assert_eq!(OpenAIReasoningEffort::Max.request_timeout_secs(), 3600);
        for effort in efforts {
            assert!(effort.stream_idle_timeout_secs() < effort.request_timeout_secs());
        }
    }

    #[arkavo_test_macros::spec("ASTRA-001")]
    #[test]
    fn configured_timeouts_follow_the_effort_unless_overridden() {
        let config = OpenAIResponsesConfig {
            reasoning_effort: OpenAIReasoningEffort::High,
            ..Default::default()
        };
        assert_eq!(config.request_timeout(), Duration::from_mins(30));
        assert_eq!(config.stream_idle_timeout(), Duration::from_mins(5));
        let overridden = OpenAIResponsesConfig {
            stream_idle_timeout: Some(Duration::from_secs(5)),
            ..Default::default()
        };
        assert_eq!(overridden.stream_idle_timeout(), Duration::from_secs(5));
    }

    /// An idle budget above the request budget could never fire.
    #[arkavo_test_macros::spec("ASTRA-001")]
    #[test]
    fn unusable_idle_budgets_and_output_caps_are_rejected() {
        for idle in [Duration::ZERO, Duration::from_secs(4000)] {
            let config = OpenAIResponsesConfig {
                stream_idle_timeout: Some(idle),
                ..Default::default()
            };
            assert!(config.validate().is_err(), "{idle:?} must not validate");
        }
        assert!(check_max_tokens(0).is_err());
        assert!(check_max_tokens(MAX_OUTPUT_TOKENS + 1).is_err());
        assert!(check_max_tokens(MAX_OUTPUT_TOKENS).is_ok());
        let config = OpenAIResponsesConfig {
            max_output_tokens: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}
