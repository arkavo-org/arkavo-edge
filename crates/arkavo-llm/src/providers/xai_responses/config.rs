use serde::{Deserialize, Serialize};

/// Reasoning effort for Grok models.
///
/// xAI's API default is `high`. Arkavo defaults to [`ReasoningEffort::Low`] for
/// agent latency; set medium/high/`xhigh` when the task needs deeper
/// chain-of-thought. `"xhigh"` is supported on `grok-4.6` and later; older
/// models treat it as `"high"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    #[default]
    Low,
    Medium,
    High,
    /// Maximum reasoning depth on Grok 4.6+. Correspondingly higher latency.
    Xhigh,
}

impl ReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
        }
    }

    /// Parse an effort name. Unknown values fall back to [`Self::Low`].
    pub fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" | "x-high" | "extra-high" => Self::Xhigh,
            _ => Self::Low,
        }
    }

    /// HTTP timeout for a Responses call at this effort.
    ///
    /// xAI's own examples use a 3600s ceiling for reasoning models; `xhigh`
    /// is the only tier that needs the full hour. Lower tiers stay tighter
    /// so a stuck agent loop fails faster.
    pub fn request_timeout_secs(self) -> u64 {
        match self {
            Self::Low => 300,
            Self::Medium => 600,
            Self::High => 1800,
            Self::Xhigh => 3600,
        }
    }
}

/// Configuration for the xAI Responses API.
///
/// ## Multi-turn (v1)
///
/// The agent loop re-sends the full transcript each turn (`previous_response_id`
/// is not wired through `Provider::complete_with_tools`). `store` therefore
/// defaults to `false` so ephemeral agent prompts are not retained server-side.
/// Set `store: true` (or `XAI_STORE=1`) only when you intend to use
/// [`super::ResponsesProvider::continue_with_tool_outputs`] with response ids.
///
/// `prompt_cache_key` (optional / `XAI_PROMPT_CACHE_KEY`) improves multi-turn
/// cache hit rates without requiring server-side conversation storage.
#[derive(Clone, Debug)]
pub struct ResponsesConfig {
    pub api_key: String,
    /// Base URL including `/v1`, e.g. `https://api.x.ai/v1`.
    pub base_url: String,
    pub model: String,
    pub reasoning_effort: ReasoningEffort,
    /// Persist server-side state (enables `previous_response_id` chaining).
    /// Default `false` for privacy in agent loops.
    pub store: bool,
    /// Optional service tier (`"priority"` for lower TTFT under load).
    pub service_tier: Option<String>,
    /// Optional stable key for prompt-cache hits across full-transcript turns.
    pub prompt_cache_key: Option<String>,
}

impl Default for ResponsesConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.x.ai/v1".to_string(),
            model: "grok-4.6".to_string(),
            reasoning_effort: ReasoningEffort::Low,
            store: false,
            service_tier: None,
            prompt_cache_key: None,
        }
    }
}

impl ResponsesConfig {
    /// Build from `XAI_API_KEY` / optional `XAI_BASE_URL`, `XAI_STORE`,
    /// `XAI_PROMPT_CACHE_KEY`, and `XAI_REASONING_EFFORT`.
    pub fn from_env() -> Result<Self, crate::Error> {
        let api_key = std::env::var("XAI_API_KEY")
            .map_err(|_| crate::Error::Config("XAI_API_KEY not set".to_string()))?;
        let base_url =
            std::env::var("XAI_BASE_URL").unwrap_or_else(|_| "https://api.x.ai/v1".to_string());
        Ok(Self {
            api_key,
            base_url,
            store: env_truthy("XAI_STORE"),
            prompt_cache_key: std::env::var("XAI_PROMPT_CACHE_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            reasoning_effort: env_reasoning_effort(),
            ..Default::default()
        })
    }

    /// Agent-oriented construction: env key + base URL, ephemeral `store=false`
    /// unless `XAI_STORE` opts in.
    pub fn for_agent(api_key: String, base_url: String, model: String) -> Self {
        Self {
            api_key,
            base_url,
            model,
            store: env_truthy("XAI_STORE"),
            prompt_cache_key: std::env::var("XAI_PROMPT_CACHE_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            reasoning_effort: env_reasoning_effort(),
            ..Default::default()
        }
    }

    /// Override reasoning effort after [`Self::for_agent`] / [`Self::from_env`].
    /// Used by the `Grok46Xhigh` routing arm so the env default cannot
    /// silently downgrade a max-effort selection.
    pub fn with_reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.reasoning_effort = effort;
        self
    }
}

fn env_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
    )
}

fn env_reasoning_effort() -> ReasoningEffort {
    ReasoningEffort::from_name(&std::env::var("XAI_REASONING_EFFORT").unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_effort_serializes_lowercase() {
        assert_eq!(ReasoningEffort::Low.as_str(), "low");
        assert_eq!(ReasoningEffort::High.as_str(), "high");
        assert_eq!(ReasoningEffort::Xhigh.as_str(), "xhigh");
    }

    #[test]
    fn reasoning_effort_from_name_recognizes_xhigh_aliases() {
        assert_eq!(ReasoningEffort::from_name("xhigh"), ReasoningEffort::Xhigh);
        assert_eq!(ReasoningEffort::from_name("XHIGH"), ReasoningEffort::Xhigh);
        assert_eq!(ReasoningEffort::from_name("x-high"), ReasoningEffort::Xhigh);
        assert_eq!(
            ReasoningEffort::from_name("extra-high"),
            ReasoningEffort::Xhigh
        );
        assert_eq!(
            ReasoningEffort::from_name("medium"),
            ReasoningEffort::Medium
        );
        assert_eq!(ReasoningEffort::from_name("high"), ReasoningEffort::High);
        assert_eq!(ReasoningEffort::from_name(""), ReasoningEffort::Low);
        assert_eq!(ReasoningEffort::from_name("nope"), ReasoningEffort::Low);
    }

    #[test]
    fn xhigh_uses_hour_timeout() {
        assert_eq!(ReasoningEffort::Xhigh.request_timeout_secs(), 3600);
        assert!(
            ReasoningEffort::Xhigh.request_timeout_secs()
                > ReasoningEffort::High.request_timeout_secs()
        );
    }

    #[test]
    fn default_store_is_false_for_ephemeral_agents() {
        let cfg = ResponsesConfig::default();
        assert!(!cfg.store);
        assert_eq!(cfg.reasoning_effort, ReasoningEffort::Low);
        assert_eq!(cfg.model, "grok-4.6");
        assert!(cfg.prompt_cache_key.is_none());
    }

    #[test]
    fn with_reasoning_effort_overrides_default() {
        let cfg = ResponsesConfig::default().with_reasoning_effort(ReasoningEffort::Xhigh);
        assert_eq!(cfg.reasoning_effort, ReasoningEffort::Xhigh);
        assert_eq!(cfg.model, "grok-4.6");
    }
}
