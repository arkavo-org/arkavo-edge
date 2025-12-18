use std::collections::HashMap;

/// Configuration for LLM providers.
///
/// This struct centralizes all LLM-related configuration, replacing the
/// previous pattern of setting environment variables at runtime.
#[derive(Debug, Clone, Default)]
pub struct LlmConfig {
    /// Provider name: "ollama", "kimi", "deepseek", "gemini", "anthropic", "qwen"
    pub provider: String,
    /// Base URL for the provider's API
    pub base_url: Option<String>,
    /// Model name to use
    pub model: Option<String>,
    /// API keys by provider/service name
    pub api_keys: HashMap<String, String>,
}

impl LlmConfig {
    /// Creates a new config with the specified provider.
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            ..Default::default()
        }
    }

    /// Creates config for Ollama provider.
    pub fn ollama() -> Self {
        Self::new("ollama")
    }

    /// Creates config for Ollama with custom settings.
    pub fn ollama_with(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: "ollama".to_string(),
            base_url: Some(base_url.into()),
            model: Some(model.into()),
            api_keys: HashMap::new(),
        }
    }

    /// Creates config for Kimi provider.
    pub fn kimi(api_key: impl Into<String>) -> Self {
        let mut api_keys = HashMap::new();
        api_keys.insert("MOONSHOT_API_KEY".to_string(), api_key.into());
        Self {
            provider: "kimi".to_string(),
            base_url: None,
            model: None,
            api_keys,
        }
    }

    /// Creates config for DeepSeek provider.
    pub fn deepseek(api_key: impl Into<String>) -> Self {
        let mut api_keys = HashMap::new();
        api_keys.insert("DEEPSEEK_API_KEY".to_string(), api_key.into());
        Self {
            provider: "deepseek".to_string(),
            base_url: None,
            model: None,
            api_keys,
        }
    }

    /// Creates config for Gemini provider.
    pub fn gemini(api_key: impl Into<String>) -> Self {
        let mut api_keys = HashMap::new();
        api_keys.insert("GEMINI_API_KEY".to_string(), api_key.into());
        Self {
            provider: "gemini".to_string(),
            base_url: None,
            model: None,
            api_keys,
        }
    }

    /// Creates config for Anthropic provider.
    pub fn anthropic(api_key: impl Into<String>) -> Self {
        let mut api_keys = HashMap::new();
        api_keys.insert("ANTHROPIC_API_KEY".to_string(), api_key.into());
        Self {
            provider: "anthropic".to_string(),
            base_url: None,
            model: None,
            api_keys,
        }
    }

    /// Creates config for Qwen provider.
    pub fn qwen(api_key: impl Into<String>) -> Self {
        let mut api_keys = HashMap::new();
        api_keys.insert("DASHSCOPE_API_KEY".to_string(), api_key.into());
        Self {
            provider: "qwen".to_string(),
            base_url: None,
            model: None,
            api_keys,
        }
    }

    /// Sets the base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// Sets the model name.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Adds an API key.
    pub fn with_api_key(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.api_keys.insert(key.into(), value.into());
        self
    }

    /// Loads configuration from environment variables (read-only, safe).
    ///
    /// This reads env vars once at startup, avoiding the need for unsafe set_var calls.
    pub fn from_env() -> Self {
        let provider = std::env::var("LLM_PROVIDER")
            .ok()
            .unwrap_or_else(|| "ollama".to_string());

        let base_url = match provider.as_str() {
            "ollama" => std::env::var("OLLAMA_BASE_URL").ok(),
            "kimi" => std::env::var("KIMI_API_BASE").ok(),
            "deepseek" => std::env::var("DEEPSEEK_BASE_URL").ok(),
            "qwen" => std::env::var("DASHSCOPE_BASE_URL").ok(),
            _ => None,
        };

        let model = match provider.as_str() {
            "ollama" => std::env::var("OLLAMA_MODEL").ok(),
            "kimi" => std::env::var("KIMI_MODEL").ok(),
            "deepseek" => std::env::var("DEEPSEEK_MODEL").ok(),
            "gemini" => std::env::var("GEMINI_MODEL").ok(),
            "anthropic" => std::env::var("ANTHROPIC_MODEL").ok(),
            "qwen" => std::env::var("QWEN_MODEL").ok(),
            _ => None,
        };

        let mut api_keys = HashMap::new();
        let env_keys = [
            "MOONSHOT_API_KEY",
            "DEEPSEEK_API_KEY",
            "GEMINI_API_KEY",
            "ANTHROPIC_API_KEY",
            "DASHSCOPE_API_KEY",
            "OPENAI_API_KEY",
        ];

        for key in env_keys {
            if let Ok(value) = std::env::var(key) {
                api_keys.insert(key.to_string(), value);
            }
        }

        Self {
            provider,
            base_url,
            model,
            api_keys,
        }
    }

    /// Gets an API key by name.
    pub fn get_api_key(&self, key: &str) -> Option<&String> {
        self.api_keys.get(key)
    }

    /// Gets the primary API key for this provider.
    pub fn primary_api_key(&self) -> Option<&String> {
        match self.provider.as_str() {
            "kimi" => self.api_keys.get("MOONSHOT_API_KEY"),
            "deepseek" => self.api_keys.get("DEEPSEEK_API_KEY"),
            "gemini" => self.api_keys.get("GEMINI_API_KEY"),
            "anthropic" => self.api_keys.get("ANTHROPIC_API_KEY"),
            "qwen" => self.api_keys.get("DASHSCOPE_API_KEY"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ollama_config() {
        let config = LlmConfig::ollama_with("http://localhost:11434", "qwen3:0.6b");
        assert_eq!(config.provider, "ollama");
        assert_eq!(config.base_url.as_deref(), Some("http://localhost:11434"));
        assert_eq!(config.model.as_deref(), Some("qwen3:0.6b"));
    }

    #[test]
    fn test_builder_pattern() {
        let config = LlmConfig::new("gemini")
            .with_model("gemini-pro")
            .with_api_key("GEMINI_API_KEY", "test-key");

        assert_eq!(config.provider, "gemini");
        assert_eq!(config.model.as_deref(), Some("gemini-pro"));
        assert_eq!(
            config.get_api_key("GEMINI_API_KEY"),
            Some(&"test-key".to_string())
        );
    }

    #[test]
    fn test_primary_api_key() {
        let config = LlmConfig::kimi("my-api-key");
        assert_eq!(config.primary_api_key(), Some(&"my-api-key".to_string()));
    }
}
