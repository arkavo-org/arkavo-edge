/// Information about an available LLM
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmInfo {
    pub name: String,
    pub provider: String,
    pub model: String,
    pub available: bool,
}

impl super::Router {
    pub fn get_available_llms(&self) -> Vec<LlmInfo> {
        let mut llms = Vec::new();
        for (api_key, name, prov, model_key, default) in [
            (
                "ANTHROPIC_API_KEY",
                "Claude",
                "Anthropic",
                "ANTHROPIC_MODEL",
                "claude-sonnet-4-5-20250929",
            ),
            (
                "GEMINI_API_KEY",
                "Gemini",
                "Google",
                "GEMINI_MODEL",
                "gemini-3.5-flash",
            ),
            (
                "MOONSHOT_API_KEY",
                "Kimi",
                "Moonshot",
                "KIMI_MODEL",
                "kimi-k2.5",
            ),
        ] {
            let enabled = match api_key {
                "ANTHROPIC_API_KEY" => self.is_anthropic_available(),
                "GEMINI_API_KEY" => self.is_gemini_available(),
                "MOONSHOT_API_KEY" => self.is_kimi_available(),
                _ => false,
            };
            if enabled {
                let model = std::env::var(model_key).unwrap_or_else(|_| default.to_string());
                llms.push(LlmInfo {
                    name: name.to_string(),
                    provider: prov.to_string(),
                    model,
                    available: true,
                });
            }
        }
        if self.is_openai_available() {
            llms.push(LlmInfo {
                name: "GPT-6 Astra".into(),
                provider: "OpenAI".into(),
                model: "gpt-6-astra".into(),
                available: true,
            });
        }
        llms.push(LlmInfo {
            name: "Local".to_string(),
            provider: "Local".to_string(),
            model: "qwen3.5-0.8b / ministral-3b".to_string(),
            available: crate::ModelChoice::ALL_LOCAL
                .iter()
                .any(|m| self.is_model_available(m)),
        });
        llms
    }
}
