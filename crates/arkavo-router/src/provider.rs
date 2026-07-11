use crate::decision::ModelChoice;
use crate::error::{Error, Result};
use crate::model_discovery;
use arkavo_llm::Provider;

#[cfg(feature = "llama-cpp")]
fn sampling_config_for(
    model: &ModelChoice,
    store: &crate::optimal_config::OptimalConfigStore,
    use_spec_decoding: bool,
) -> arkavo_llm::SamplingConfig {
    if let Some(oc) = store.get(model) {
        arkavo_llm::SamplingConfig {
            temperature: oc.temperature,
            top_p: oc.top_p,
            thinking_mode: Some(oc.thinking_mode),
            use_spec_decoding,
            ..arkavo_llm::SamplingConfig::default()
        }
    } else if let Some((temp, top_p, thinking)) = model.optimal_sampling() {
        arkavo_llm::SamplingConfig {
            temperature: temp,
            top_p,
            thinking_mode: Some(thinking),
            use_spec_decoding,
            ..arkavo_llm::SamplingConfig::default()
        }
    } else {
        arkavo_llm::SamplingConfig {
            use_spec_decoding,
            ..arkavo_llm::SamplingConfig::default()
        }
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
    #[cfg(feature = "gemini")]
    pub fn get_planning_provider(&self) -> Option<arkavo_llm::GeminiProvider> {
        arkavo_llm::GeminiProvider::new().ok()
    }

    #[cfg(not(feature = "gemini"))]
    pub fn get_planning_provider(&self) -> Option<()> {
        None
    }

    pub fn is_gemini_available(&self) -> bool {
        cfg!(feature = "gemini") && std::env::var("GEMINI_API_KEY").is_ok()
    }

    pub fn is_anthropic_available(&self) -> bool {
        std::env::var("ANTHROPIC_API_KEY").is_ok()
    }

    pub fn is_kimi_available(&self) -> bool {
        std::env::var("MOONSHOT_API_KEY").is_ok()
    }

    pub fn is_glm_available(&self) -> bool {
        // Only "available" when the arm can actually be built: instantiation is
        // `#[cfg(feature = "glm")]`, so gate availability the same way. Without
        // this, a no-`glm` build with GLM_API_KEY set marks Glm52 feasible and
        // then dead-ends on the catch-all.
        cfg!(feature = "glm") && std::env::var("GLM_API_KEY").is_ok()
    }

    pub fn is_xai_available(&self) -> bool {
        cfg!(feature = "xai") && std::env::var("XAI_API_KEY").is_ok()
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
            ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus | ModelChoice::ClaudeFable5 => {
                self.is_anthropic_available()
            }
            ModelChoice::GeminiFlash
            | ModelChoice::Gemini35Flash
            | ModelChoice::Gemini35FlashMinimal
            | ModelChoice::Gemini35FlashMedium
            | ModelChoice::Gemini35FlashHigh
            | ModelChoice::GeminiPro => self.is_gemini_available(),
            ModelChoice::DeepSeekV32 | ModelChoice::DeepSeekV32Speciale => {
                std::env::var("DEEPSEEK_API_KEY").is_ok()
            }
            ModelChoice::KimiK2 => std::env::var("MOONSHOT_API_KEY").is_ok(),
            ModelChoice::Glm52 => cfg!(feature = "glm") && std::env::var("GLM_API_KEY").is_ok(),
            ModelChoice::Grok45 => cfg!(feature = "xai") && std::env::var("XAI_API_KEY").is_ok(),
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
        use_spec_decoding: bool,
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

            // Pre-warm the context pool so the first inference avoids allocation latency
            #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
            if let Ok(ctx) = self.model_registry.acquire_fresh_context(registry_name) {
                let _ = self
                    .model_registry
                    .release_context(registry_name, ctx, true);
                tracing::info!(model = registry_name, "Context pool pre-warmed");
            }
            tracing::info!(model = registry_name, "Model loaded and cached in registry");
        } else {
            tracing::debug!(model = registry_name, "Using cached model from registry");
        }

        let provider = arkavo_llm::LlamaCppProvider::new_with_registry(
            self.model_registry.clone(),
            registry_name.to_string(),
            sampling_config_for(model, &self.optimal_configs, use_spec_decoding),
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

    /// Create a provider with execution-mode overrides: temp 0.1, thinking off.
    /// Used for tool-loop iterations that emit mechanical tool calls.
    #[cfg(feature = "llama-cpp")]
    pub(crate) async fn instantiate_provider_execution(
        &self,
        model: &ModelChoice,
    ) -> Result<Box<dyn Provider>> {
        if !model.is_local() {
            return self.instantiate_provider(model).await;
        }
        let repo = model
            .repo_id()
            .ok_or_else(|| Error::ModelExecution(format!("No repo_id for {model:?}")))?;
        let file = model
            .gguf_filename()
            .ok_or_else(|| Error::ModelExecution(format!("No gguf_filename for {model:?}")))?;
        let registry_name = model.name();
        let model_path = model_discovery::find_gguf_model(repo, file)
            .await
            .map_err(Error::ModelExecution)?;
        if !self.model_registry.is_loaded(registry_name) {
            self.model_registry
                .load(registry_name, &model_path.to_string_lossy())
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to load {registry_name}: {e}"))
                })?;
        }
        let execution_config = arkavo_llm::SamplingConfig {
            temperature: 0.1,
            top_p: 0.9,
            thinking_mode: Some(arkavo_llm::ThinkingMode::Off),
            ..arkavo_llm::SamplingConfig::default()
        };
        let provider = arkavo_llm::LlamaCppProvider::new_with_registry(
            self.model_registry.clone(),
            registry_name.to_string(),
            execution_config,
        )
        .map_err(|e| {
            Error::ModelExecution(format!(
                "Failed to create execution provider for {registry_name}: {e}"
            ))
        })?;
        Ok(Box::new(provider))
    }

    #[cfg(not(feature = "llama-cpp"))]
    pub(crate) async fn instantiate_provider_execution(
        &self,
        model: &ModelChoice,
    ) -> Result<Box<dyn Provider>> {
        self.instantiate_provider(model).await
    }

    pub(crate) async fn instantiate_provider(
        &self,
        model: &ModelChoice,
    ) -> Result<Box<dyn Provider>> {
        // Default: spec decoding enabled unless per-model stats say otherwise.
        // Call sites that already hold a RoutingDecision should use
        // `instantiate_provider_with_spec` to forward the flag.
        self.instantiate_provider_with_spec(model, true).await
    }

    /// Instantiate a provider with an explicit spec-decoding flag.
    ///
    /// Pass `use_spec_decoding` from `RoutingDecision.use_spec_decoding` so the
    /// router's per-model rolling stats reach the llama.cpp `SamplingConfig`.
    /// Cloud providers ignore the flag; it only affects local llama.cpp paths.
    pub(crate) async fn instantiate_provider_with_spec(
        &self,
        model: &ModelChoice,
        use_spec_decoding: bool,
    ) -> Result<Box<dyn Provider>> {
        tracing::debug!(model = %model.name(), use_spec_decoding, "Instantiating provider");
        match model {
            ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus | ModelChoice::ClaudeFable5 => {
                use arkavo_llm::providers::anthropic::AnthropicProvider;
                // Pass the routed model id so distinct arms reach distinct
                // API models instead of collapsing to the env/default model.
                if let Ok(provider) = AnthropicProvider::from_env_with_model(model.name()) {
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
            ModelChoice::GeminiFlash
            | ModelChoice::Gemini35Flash
            | ModelChoice::Gemini35FlashMinimal
            | ModelChoice::Gemini35FlashMedium
            | ModelChoice::Gemini35FlashHigh
            | ModelChoice::GeminiPro => {
                // Each ModelChoice arm carries its own API model id +
                // thinking budget. Construct the provider explicitly so
                // distinct Thompson Sampling arms actually send different
                // `thinkingConfig` payloads to the Gemini API.
                let api_model = model.gemini_api_model().unwrap_or("gemini-3.5-flash");
                let thinking = model.gemini_thinking_budget();
                let built =
                    arkavo_llm::GeminiProvider::for_model_with_thinking(api_model, thinking)
                        .or_else(|_| arkavo_llm::GeminiProvider::new());
                if let Ok(provider) = built {
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
                self.load_local_model(m, repo, file, use_spec_decoding)
                    .await
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
            #[cfg(feature = "glm")]
            ModelChoice::Glm52 => {
                // GLM-5.2 speaks the OpenAI chat-completions wire format, so it
                // routes through the generic OpenAI-compatible provider rather
                // than a bespoke crate. Base URL defaults to Z.ai's v4 endpoint
                // and is overridable for the mainland (open.bigmodel.cn) host.
                use arkavo_llm::providers::openai::{OpenAIConfig, OpenAIProvider};

                let api_key = std::env::var("GLM_API_KEY")
                    .map_err(|_| Error::ModelExecution("GLM_API_KEY not set".to_string()))?;
                let base_url = std::env::var("GLM_BASE_URL")
                    .unwrap_or_else(|_| "https://api.z.ai/api/paas/v4".to_string());

                let config = OpenAIConfig {
                    api_key,
                    base_url,
                    model: model.name().to_string(),
                    organization_id: None,
                    api_version: None,
                    is_azure: false,
                };

                let provider = OpenAIProvider::new(config).map_err(|e| {
                    Error::ModelExecution(format!("Failed to create GLM provider: {e}"))
                })?;
                Ok(Box::new(provider))
            }
            #[cfg(feature = "xai")]
            ModelChoice::Grok45 => {
                // Grok 4.5 uses the xAI Responses API (not Chat Completions)
                // for reasoning_effort control. v1 multi-turn is full-transcript;
                // store stays off unless XAI_STORE opts in.
                use arkavo_llm::providers::xai_responses::{ResponsesConfig, ResponsesProvider};

                let api_key = std::env::var("XAI_API_KEY")
                    .map_err(|_| Error::ModelExecution("XAI_API_KEY not set".to_string()))?;
                let base_url = std::env::var("XAI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.x.ai/v1".to_string());

                let config =
                    ResponsesConfig::for_agent(api_key, base_url, model.name().to_string());

                let provider = ResponsesProvider::new(config).map_err(|e| {
                    Error::ModelExecution(format!("Failed to create xAI Responses provider: {e}"))
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
