use crate::decision::ModelChoice;
use crate::error::{Error, Result};
use crate::model_discovery;
use arkavo_llm::Provider;

fn sampling_config_for(model: &ModelChoice) -> arkavo_llm::SamplingConfig {
    if let Some((temp, top_p, thinking)) = model.optimal_sampling() {
        arkavo_llm::SamplingConfig {
            temperature: temp,
            top_p,
            thinking_mode: Some(thinking),
            ..arkavo_llm::SamplingConfig::default()
        }
    } else {
        arkavo_llm::SamplingConfig::default()
    }
}

impl super::Router {
    /// Get a provider for the given model choice (local or cloud)
    pub async fn get_provider(&self, model: &ModelChoice) -> Result<Box<dyn Provider>> {
        self.instantiate_provider(model).await
    }

    /// Get a reference to the local provider for simple classification tasks
    pub fn get_local_provider(&self) -> std::sync::Arc<crate::TaskClassifier> {
        self.classifier.clone()
    }

    /// Get a Gemini provider for complex planning/thinking tasks
    pub fn get_planning_provider(&self) -> Option<arkavo_llm::GeminiProvider> {
        arkavo_llm::GeminiProvider::new().ok()
    }

    pub fn is_gemini_available(&self) -> bool {
        std::env::var("GEMINI_API_KEY").is_ok()
    }

    pub fn is_anthropic_available(&self) -> bool {
        std::env::var("ANTHROPIC_API_KEY").is_ok()
    }

    pub fn is_kimi_available(&self) -> bool {
        std::env::var("MOONSHOT_API_KEY").is_ok()
    }

    pub fn get_anthropic_provider(
        &self,
    ) -> Option<arkavo_llm::providers::anthropic::AnthropicProvider> {
        arkavo_llm::providers::anthropic::AnthropicProvider::from_env().ok()
    }

    pub(crate) fn get_local_fallback(
        &self,
        category: crate::classifier::TaskCategory,
    ) -> ModelChoice {
        use crate::classifier::TaskCategory;
        match category {
            TaskCategory::FrontendUI
            | TaskCategory::BackendAPI
            | TaskCategory::Refactoring
            | TaskCategory::CodeGeneration => ModelChoice::LocalMinistral3B,
            _ => ModelChoice::LocalQwen3,
        }
    }

    pub(crate) fn is_model_available(&self, model: &ModelChoice) -> bool {
        match model {
            ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus => self.is_anthropic_available(),
            ModelChoice::GeminiFlash | ModelChoice::GeminiPro => self.is_gemini_available(),
            ModelChoice::DeepSeekV32 | ModelChoice::DeepSeekV32Speciale => {
                std::env::var("DEEPSEEK_API_KEY").is_ok()
            }
            ModelChoice::KimiK2 => std::env::var("MOONSHOT_API_KEY").is_ok(),
            m if m.is_local() => match (m.repo_id(), m.gguf_filename()) {
                (Some(repo), Some(file)) => model_discovery::is_model_cached(repo, file),
                _ => false,
            },
            _ => false,
        }
    }

    /// Load a local model into the registry (if not already loaded) and create a provider.
    ///
    /// Vision is enabled only for models that declare support via
    /// [`ModelChoice::supports_vision`] and have an mmproj file alongside
    /// the model GGUF in the HuggingFace cache.
    #[cfg(feature = "llama-cpp")]
    pub(crate) async fn load_local_model(
        &self,
        model: &ModelChoice,
        repo: &str,
        filename: &str,
    ) -> Result<Box<dyn Provider>> {
        let registry_name = model.name();
        // Resolve model path unconditionally — hf_hub returns the cached path
        // instantly when the model is already downloaded (local stat, no network).
        let model_path = model_discovery::find_gguf_model(repo, filename)
            .await
            .map_err(Error::ModelExecution)?;

        if !self.model_registry.is_loaded(registry_name) {
            tracing::info!(
                model = registry_name,
                path = %model_path.display(),
                "Loading model into registry (first use)"
            );

            self.model_registry
                .load(registry_name, &model_path.to_string_lossy())
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to load {registry_name}: {e}"))
                })?;

            tracing::info!(model = registry_name, "Model loaded and cached in registry");
        } else {
            tracing::debug!(model = registry_name, "Using cached model from registry");
        }

        let provider = arkavo_llm::LlamaCppProvider::new_with_registry(
            self.model_registry.clone(),
            registry_name.to_string(),
            sampling_config_for(model),
        )
        .map_err(|e| {
            Error::ModelExecution(format!(
                "Failed to create provider for {registry_name}: {e}"
            ))
        })?;

        // Enable vision only for models that declare support (e.g., 27B, not 0.8B).
        // The MtmdContext is cached in the registry so the CLIP model is loaded once,
        // not on every inference call. Vision is unavailable on musl targets.
        #[cfg(not(target_env = "musl"))]
        let provider = if model.supports_vision() {
            if let Some(cached_ctx) = self.model_registry.get_vision_ctx(registry_name) {
                tracing::debug!(model = registry_name, "Using cached vision context");
                provider.enable_vision_cached(cached_ctx)
            } else if let Some(mmproj_path) = model_discovery::find_mmproj_for_model(&model_path) {
                let p = provider
                    .enable_vision(&mmproj_path.to_string_lossy())
                    .map_err(|e| Error::ModelExecution(format!("Failed to enable vision: {e}")))?;
                // Cache the vision context for future calls
                if let Some(ctx) = p.vision_ctx() {
                    self.model_registry.store_vision_ctx(registry_name, ctx);
                }
                p
            } else {
                provider
            }
        } else {
            provider
        };

        Ok(Box::new(provider))
    }

    pub(crate) async fn instantiate_provider(
        &self,
        model: &ModelChoice,
    ) -> Result<Box<dyn Provider>> {
        tracing::debug!(model = %model.name(), "Instantiating provider");
        match model {
            ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus => {
                use arkavo_llm::providers::anthropic::AnthropicProvider;
                if let Ok(provider) = AnthropicProvider::from_env() {
                    Ok(Box::new(provider))
                } else {
                    #[cfg(feature = "gemini")]
                    if let Ok(provider) = arkavo_llm::GeminiProvider::new() {
                        return Ok(Box::new(provider));
                    }
                    Err(Error::ModelExecution(
                        "ANTHROPIC_API_KEY not set and no fallback available".to_string(),
                    ))
                }
            }
            #[cfg(feature = "gemini")]
            ModelChoice::GeminiFlash | ModelChoice::GeminiPro => {
                if let Ok(provider) = arkavo_llm::GeminiProvider::new() {
                    Ok(Box::new(provider))
                } else {
                    #[cfg(feature = "llama-cpp")]
                    {
                        let hint = ModelChoice::LocalQwen3.download_hint().unwrap_or_default();
                        let model_path =
                            model_discovery::find_any_gguf().await.ok_or_else(|| {
                                Error::ModelExecution(format!(
                                    "No local GGUF models found. Download with: {hint}"
                                ))
                            })?;

                        let model_name = model_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("local-model")
                            .to_string();

                        if !self.model_registry.is_loaded(&model_name) {
                            self.model_registry
                                .load(&model_name, &model_path.to_string_lossy())
                                .map_err(|e| {
                                    Error::ModelExecution(format!(
                                        "Failed to load fallback model: {e}"
                                    ))
                                })?;
                        }

                        let provider = arkavo_llm::LlamaCppProvider::new_with_registry(
                            self.model_registry.clone(),
                            model_name,
                            arkavo_llm::SamplingConfig::default(),
                        )
                        .map_err(|e| {
                            Error::ModelExecution(format!(
                                "Failed to create fallback local provider: {e}"
                            ))
                        })?;
                        Ok(Box::new(provider))
                    }
                    #[cfg(not(feature = "llama-cpp"))]
                    {
                        Err(Error::ModelExecution(
                            "Gemini API key not set and no local model fallback available. Set GEMINI_API_KEY or rebuild with llama-cpp feature.".to_string()
                        ))
                    }
                }
            }
            #[cfg(feature = "llama-cpp")]
            m if m.is_local() => {
                let repo = m
                    .repo_id()
                    .ok_or_else(|| Error::ModelExecution(format!("No repo_id for {m:?}")))?;
                let file = m
                    .gguf_filename()
                    .ok_or_else(|| Error::ModelExecution(format!("No gguf_filename for {m:?}")))?;
                self.load_local_model(m, repo, file).await
            }
            #[cfg(feature = "kimi")]
            ModelChoice::KimiK2 => {
                use arkavo_llm::providers::anthropic::{AnthropicConfig, AnthropicProvider};

                let api_key = std::env::var("MOONSHOT_API_KEY")
                    .map_err(|_| Error::ModelExecution("MOONSHOT_API_KEY not set".to_string()))?;

                let config = AnthropicConfig {
                    api_key,
                    base_url: "https://api.moonshot.ai/anthropic".to_string(),
                    model: "kimi-k2.5".to_string(),
                    api_version: "2023-06-01".to_string(),
                };

                let provider = AnthropicProvider::new(config).map_err(|e| {
                    Error::ModelExecution(format!("Failed to create Kimi provider: {e}"))
                })?;
                Ok(Box::new(provider))
            }
            #[allow(unreachable_patterns)]
            _ => Err(Error::ModelExecution(format!(
                "Model {model:?} not available (feature not enabled)"
            ))),
        }
    }
}
