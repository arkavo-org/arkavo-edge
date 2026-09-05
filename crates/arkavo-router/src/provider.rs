use crate::decision::ModelChoice;
use crate::error::{Error, Result};
use crate::model_discovery;
use arkavo_llm::Provider;
use std::path::Path;

/// Substitutes live provider construction.
///
/// Install one with [`crate::Router::with_provider_factory`] to drive routing,
/// cloud policy and spend accounting deterministically: the router resolves
/// every dispatch through the factory instead of reading credentials, the model
/// cache or the network. Returning an error models an unavailable arm.
pub trait ProviderFactory: Send + Sync {
    fn build(&self, model: &ModelChoice) -> Result<Box<dyn Provider>>;
}

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
    #[cfg(any(
        feature = "llm-remote",
        feature = "llama-cpp",
        feature = "gemini",
        feature = "deepseek"
    ))]
    fn protect_provider(&self, provider: Box<dyn Provider>) -> Box<dyn Provider> {
        #[cfg(feature = "sentinel")]
        {
            crate::response_policy::protect(provider)
        }
        #[cfg(not(feature = "sentinel"))]
        {
            provider
        }
    }

    /// Substituted provider for `model`, when a factory is installed.
    fn substituted_provider(&self, model: &ModelChoice) -> Option<Result<Box<dyn Provider>>> {
        self.provider_factory
            .as_ref()
            .map(|factory| factory.build(model))
    }

    /// Get a provider for the given model choice (local or cloud)
    pub async fn get_provider(&self, model: &ModelChoice) -> Result<Box<dyn Provider>> {
        self.instantiate_provider(model).await
    }

    /// Construct the requested arm without hidden fallback so billing keeps its identity.
    pub async fn get_provider_attributed(
        &self,
        model: &ModelChoice,
    ) -> Result<(Box<dyn Provider>, ModelChoice)> {
        Ok((
            self.instantiate_provider_exact_with_spec(model, true)
                .await?,
            model.clone(),
        ))
    }

    pub(crate) async fn instantiate_provider_exact_with_spec(
        &self,
        model: &ModelChoice,
        use_spec_decoding: bool,
    ) -> Result<Box<dyn Provider>> {
        self.instantiate_provider_inner(model, use_spec_decoding, false)
            .await
    }

    /// Get a reference to the local provider for simple classification tasks
    pub fn get_local_provider(&self) -> std::sync::Arc<crate::TaskClassifier> {
        self.classifier.clone()
    }

    /// Get a Gemini provider for complex planning/thinking tasks
    #[cfg(feature = "gemini")]
    pub fn get_planning_provider(&self) -> Option<Box<dyn Provider>> {
        arkavo_llm::GeminiProvider::new()
            .ok()
            .map(|provider| self.protect_provider(Box::new(provider)))
    }

    #[cfg(not(feature = "gemini"))]
    pub fn get_planning_provider(&self) -> Option<Box<dyn Provider>> {
        None
    }

    pub fn is_gemini_available(&self) -> bool {
        cfg!(feature = "gemini") && std::env::var("GEMINI_API_KEY").is_ok()
    }

    pub fn is_anthropic_available(&self) -> bool {
        cfg!(feature = "llm-remote") && std::env::var("ANTHROPIC_API_KEY").is_ok()
    }

    pub fn is_kimi_available(&self) -> bool {
        cfg!(feature = "kimi") && std::env::var("MOONSHOT_API_KEY").is_ok()
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

    #[cfg(feature = "llm-remote")]
    pub fn get_anthropic_provider(&self) -> Option<Box<dyn Provider>> {
        arkavo_llm::providers::anthropic::AnthropicProvider::from_env()
            .ok()
            .map(|provider| self.protect_provider(Box::new(provider)))
    }

    #[cfg(not(feature = "llm-remote"))]
    pub fn get_anthropic_provider(&self) -> Option<Box<dyn Provider>> {
        None
    }

    pub fn is_openai_available(&self) -> bool {
        cfg!(feature = "openai")
            && std::env::var("OPENAI_API_KEY").is_ok_and(|key| !key.trim().is_empty())
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
                cfg!(feature = "deepseek") && std::env::var("DEEPSEEK_API_KEY").is_ok()
            }
            ModelChoice::KimiK2 => self.is_kimi_available(),
            ModelChoice::Gpt6Astra => self.is_openai_available(),
            ModelChoice::Glm52 => cfg!(feature = "glm") && std::env::var("GLM_API_KEY").is_ok(),
            ModelChoice::Grok46 | ModelChoice::Grok46Xhigh => {
                cfg!(feature = "xai") && std::env::var("XAI_API_KEY").is_ok()
            }
            m if m.is_local() && cfg!(feature = "llama-cpp") => {
                match (m.repo_id(), m.gguf_filename()) {
                    (Some(repo), Some(file)) => model_discovery::is_model_cached(repo, file),
                    _ => false,
                }
            }
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

            self.ensure_loaded(registry_name, &model_path).await?;

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

        Ok(self.protect_provider(Box::new(provider)))
    }

    /// Load an on-disk GGUF (or `.gguf.tdf`) by path. Not a named catalog model.
    #[cfg(feature = "llama-cpp")]
    pub(crate) async fn instantiate_gguf_path(
        &self,
        path: &Path,
        use_spec_decoding: bool,
    ) -> Result<Box<dyn Provider>> {
        if let Some(substituted) = self.substituted_provider(&ModelChoice::LocalQwen3) {
            return substituted;
        }
        let resolved = model_discovery::resolve_gguf_path(path);
        if !resolved.exists() {
            return Err(Error::ModelExecution(format!(
                "GGUF not found: {}",
                path.display()
            )));
        }
        let canonical = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());
        let registry_name = format!("gguf:{}", canonical.display());

        if !self.model_registry.is_loaded(&registry_name) {
            tracing::info!(
                model = %registry_name,
                path = %canonical.display(),
                "Loading GGUF path into registry"
            );
            self.ensure_loaded(&registry_name, &canonical).await?;

            #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
            if let Ok(ctx) = self.model_registry.acquire_fresh_context(&registry_name) {
                let _ = self
                    .model_registry
                    .release_context(&registry_name, ctx, true);
                tracing::info!(model = %registry_name, "Context pool pre-warmed");
            }
            tracing::info!(model = %registry_name, "GGUF loaded and cached in registry");
        } else {
            tracing::debug!(model = %registry_name, "Using cached GGUF from registry");
        }

        let sampling = arkavo_llm::SamplingConfig {
            use_spec_decoding,
            thinking_mode: Some(arkavo_llm::ThinkingMode::Off),
            ..arkavo_llm::SamplingConfig::default()
        };
        let provider = arkavo_llm::LlamaCppProvider::new_with_registry(
            self.model_registry.clone(),
            registry_name.clone(),
            sampling,
        )
        .map_err(|e| {
            Error::ModelExecution(format!(
                "Failed to create provider for {registry_name}: {e}"
            ))
        })?;
        Ok(self.protect_provider(Box::new(provider)))
    }

    #[cfg(not(feature = "llama-cpp"))]
    pub(crate) async fn instantiate_gguf_path(
        &self,
        path: &Path,
        _use_spec_decoding: bool,
    ) -> Result<Box<dyn Provider>> {
        Err(Error::ModelExecution(format!(
            "llama-cpp required to load GGUF {}",
            path.display()
        )))
    }

    /// Create a provider with execution-mode overrides: temp 0.1, thinking off.
    /// Used for tool-loop iterations that emit mechanical tool calls.
    #[cfg(feature = "llama-cpp")]
    pub(crate) async fn instantiate_provider_execution(
        &self,
        model: &ModelChoice,
    ) -> Result<Box<dyn Provider>> {
        if let Some(substituted) = self.substituted_provider(model) {
            return substituted;
        }
        if !model.is_local() {
            return self.instantiate_provider_exact_with_spec(model, true).await;
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
        self.ensure_loaded(registry_name, &model_path).await?;
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
        Ok(self.protect_provider(Box::new(provider)))
    }

    #[cfg(not(feature = "llama-cpp"))]
    pub(crate) async fn instantiate_provider_execution(
        &self,
        model: &ModelChoice,
    ) -> Result<Box<dyn Provider>> {
        self.instantiate_provider_exact_with_spec(model, true).await
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
        self.instantiate_provider_inner(model, use_spec_decoding, true)
            .await
    }

    async fn instantiate_provider_inner(
        &self,
        model: &ModelChoice,
        use_spec_decoding: bool,
        allow_fallback: bool,
    ) -> Result<Box<dyn Provider>> {
        if let Some(substituted) = self.substituted_provider(model) {
            return substituted;
        }
        let _ = (use_spec_decoding, allow_fallback);
        tracing::debug!(model = %model.name(), use_spec_decoding, "Instantiating provider");
        match model {
            #[cfg(feature = "llm-remote")]
            ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus | ModelChoice::ClaudeFable5 => {
                use arkavo_llm::providers::anthropic::AnthropicProvider;
                // Pass the routed model id so distinct arms reach distinct
                // API models instead of collapsing to the env/default model.
                if let Ok(provider) = AnthropicProvider::from_env_with_model(model.name()) {
                    Ok(self.protect_provider(Box::new(provider)))
                } else {
                    if !allow_fallback {
                        return Err(Error::ModelExecution(
                            "Requested Anthropic provider unavailable".into(),
                        ));
                    }
                    #[cfg(feature = "gemini")]
                    if let Ok(provider) = arkavo_llm::GeminiProvider::new() {
                        return Ok(self.protect_provider(Box::new(provider)));
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
                    arkavo_llm::GeminiProvider::for_model_with_thinking(api_model, thinking);
                let built = if allow_fallback {
                    built.or_else(|_| arkavo_llm::GeminiProvider::new())
                } else {
                    built
                };
                if let Ok(provider) = built {
                    Ok(self.protect_provider(Box::new(provider)))
                } else {
                    if !allow_fallback {
                        return Err(Error::ModelExecution(
                            "Requested Gemini provider unavailable".into(),
                        ));
                    }
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

                        self.ensure_loaded(&model_name, &model_path).await?;

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
                        Ok(self.protect_provider(Box::new(provider)))
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
            #[cfg(feature = "deepseek")]
            ModelChoice::DeepSeekV32 | ModelChoice::DeepSeekV32Speciale => {
                let provider = if matches!(model, ModelChoice::DeepSeekV32Speciale) {
                    arkavo_llm::DeepSeekProvider::v32_speciale()
                } else {
                    arkavo_llm::DeepSeekProvider::from_env()
                }
                .map_err(|e| Error::ModelExecution(e.to_string()))?;
                Ok(self.protect_provider(Box::new(provider)))
            }
            #[cfg(feature = "openai")]
            ModelChoice::Gpt6Astra => {
                use arkavo_llm::providers::{OpenAIResponsesConfig, OpenAIResponsesProvider};
                let config = OpenAIResponsesConfig {
                    api_key: Some(
                        std::env::var("OPENAI_API_KEY")
                            .map_err(|_| Error::ModelExecution("OPENAI_API_KEY not set".into()))?,
                    ),
                    model: model.name().into(),
                    ..OpenAIResponsesConfig::default()
                };
                let provider = OpenAIResponsesProvider::new(config)
                    .map_err(|e| Error::ModelExecution(e.to_string()))?;
                Ok(self.protect_provider(Box::new(provider)))
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
                Ok(self.protect_provider(Box::new(provider)))
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
                Ok(self.protect_provider(Box::new(provider)))
            }
            #[cfg(feature = "xai")]
            ModelChoice::Grok46 | ModelChoice::Grok46Xhigh => {
                // Grok 4.6 uses the xAI Responses API (not Chat Completions)
                // for reasoning_effort control. The API model is always
                // `grok-4.6`; `Grok46Xhigh` forces `reasoning.effort = xhigh`.
                use arkavo_llm::providers::xai_responses::{
                    ReasoningEffort, ResponsesConfig, ResponsesProvider,
                };

                let api_key = std::env::var("XAI_API_KEY")
                    .map_err(|_| Error::ModelExecution("XAI_API_KEY not set".to_string()))?;
                let base_url = std::env::var("XAI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.x.ai/v1".to_string());
                let api_model = model.grok_api_model().unwrap_or("grok-4.6").to_string();

                let effort = if matches!(model, ModelChoice::Grok46Xhigh) {
                    ReasoningEffort::Xhigh
                } else {
                    ReasoningEffort::Low
                };
                let config = ResponsesConfig::for_routed_arm(api_key, base_url, api_model, effort);

                let provider = ResponsesProvider::new(config).map_err(|e| {
                    Error::ModelExecution(format!("Failed to create xAI Responses provider: {e}"))
                })?;
                Ok(self.protect_provider(Box::new(provider)))
            }
            #[allow(unreachable_patterns)]
            _ => Err(Error::ModelExecution(format!(
                "Model {model:?} not available (feature not enabled)"
            ))),
        }
    }
}
