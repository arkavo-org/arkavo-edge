use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

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

#[derive(Clone)]
pub struct OpenAIResponsesConfig {
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub reasoning_effort: OpenAIReasoningEffort,
    pub max_output_tokens: usize,
}

impl fmt::Debug for OpenAIResponsesConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAIResponsesConfig")
            .field("model", &self.model)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("max_output_tokens", &self.max_output_tokens)
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
        }
    }
}

impl OpenAIResponsesConfig {
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
        if self.model != "gpt-6-astra"
            || self.max_output_tokens == 0
            || self.max_output_tokens > 128_000
        {
            return Err(Error::Config(
                "OpenAI Responses requires gpt-6-astra and 1..=128000 output tokens".into(),
            ));
        }
        Ok(url)
    }
}
