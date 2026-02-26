use crate::decision::ModelChoice;
use crate::error::{Error, Result};
use crate::model_discovery;
use arkavo_llm::Provider;

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

    pub(crate) fn upgrade_model(&self, current: &ModelChoice) -> ModelChoice {
        let candidate = match current {
            ModelChoice::LocalQwen3 => ModelChoice::LocalMinistral3B,
            ModelChoice::LocalMinistral3B => ModelChoice::LocalMinistral8B,
            ModelChoice::LocalMinistral8B => ModelChoice::LocalQwen35_27B,
            ModelChoice::LocalQwen35_27B => ModelChoice::LocalGlm47Flash,
            ModelChoice::LocalGlm47Flash => ModelChoice::LocalGlm47Flash,
            ModelChoice::LocalGemma270M => ModelChoice::LocalGemma4B,
            ModelChoice::LocalGemma4B => ModelChoice::LocalGemma12B,
            ModelChoice::LocalGemma12B => ModelChoice::LocalGemma12B,
            ModelChoice::LocalDeepSeekCoder => ModelChoice::DeepSeekV32,
            ModelChoice::DeepSeekV32 => ModelChoice::ClaudeSonnet,
            ModelChoice::DeepSeekV32Speciale => ModelChoice::ClaudeOpus,
            ModelChoice::GeminiFlash => ModelChoice::ClaudeSonnet,
            ModelChoice::ClaudeSonnet => ModelChoice::GeminiPro,
            ModelChoice::GeminiPro => ModelChoice::ClaudeOpus,
            ModelChoice::ClaudeOpus => ModelChoice::ClaudeOpus,
            ModelChoice::KimiK2 => ModelChoice::ClaudeSonnet,
        };

        if self.is_model_available(&candidate) {
            candidate
        } else {
            tracing::debug!(
                "Upgrade target {:?} not available, staying with {:?}",
                candidate,
                current
            );
            current.clone()
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
            ModelChoice::LocalQwen3 => {
                model_discovery::is_model_cached("Qwen/Qwen3-0.6B-GGUF", "Qwen3-0.6B-Q8_0.gguf")
            }
            ModelChoice::LocalMinistral3B => model_discovery::is_model_cached(
                "mistralai/Ministral-3-3B-Instruct-2512-GGUF",
                "Ministral-3-3B-Instruct-2512-Q5_K_M.gguf",
            ),
            ModelChoice::LocalMinistral8B => model_discovery::is_model_cached(
                "mistralai/Ministral-3-8B-Instruct-2512-GGUF",
                "Ministral-3-8B-Instruct-2512-Q5_K_M.gguf",
            ),
            ModelChoice::LocalGemma270M => model_discovery::is_model_cached(
                "unsloth/gemma-3-270m-it-GGUF",
                "gemma-3-270m-it-Q4_0.gguf",
            ),
            ModelChoice::LocalGemma4B => model_discovery::is_model_cached(
                "unsloth/gemma-3-4b-it-GGUF",
                "gemma-3-4b-it-Q4_0.gguf",
            ),
            ModelChoice::LocalGemma12B => model_discovery::is_model_cached(
                "unsloth/gemma-3-12b-it-GGUF",
                "gemma-3-12b-it-Q4_0.gguf",
            ),
            ModelChoice::LocalDeepSeekCoder => model_discovery::is_model_cached(
                "bartowski/DeepSeek-Coder-V2-Lite-Instruct-GGUF",
                "DeepSeek-Coder-V2-Lite-Instruct-Q4_K_M.gguf",
            ),
            ModelChoice::LocalQwen35_27B => model_discovery::is_model_cached(
                "unsloth/Qwen3.5-27B-GGUF",
                "Qwen3.5-27B-UD-Q6_K_XL.gguf",
            ),
            ModelChoice::LocalGlm47Flash => model_discovery::is_model_cached(
                "unsloth/GLM-4.7-Flash-GGUF",
                "GLM-4.7-Flash-Q4_K_M.gguf",
            ),
        }
    }

    /// Load a local model into the registry (if not already loaded) and create a provider.
    ///
    /// Automatically discovers and enables vision support when an mmproj file
    /// is found alongside the model GGUF in the HuggingFace cache.
    #[cfg(feature = "llama-cpp")]
    pub(crate) async fn load_local_model(
        &self,
        registry_name: &str,
        repo: &str,
        filename: &str,
    ) -> Result<Box<dyn Provider>> {
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
            arkavo_llm::SamplingConfig::default(),
        )
        .map_err(|e| {
            Error::ModelExecution(format!(
                "Failed to create provider for {registry_name}: {e}"
            ))
        })?;

        // Enable vision if mmproj file is found alongside the model
        let provider =
            if let Some(mmproj_path) = model_discovery::find_mmproj_for_model(&model_path) {
                provider
                    .enable_vision(&mmproj_path.to_string_lossy())
                    .map_err(|e| Error::ModelExecution(format!("Failed to enable vision: {e}")))?
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
                        let model_path = model_discovery::find_any_gguf()
                            .await
                            .ok_or_else(|| Error::ModelExecution(
                                "No local GGUF models found. Download with: hf download Qwen/Qwen3-0.6B-GGUF Qwen3-0.6B-Q8_0.gguf".to_string()
                            ))?;

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
            ModelChoice::LocalQwen3 => {
                self.load_local_model("qwen3-0.6b", "Qwen/Qwen3-0.6B-GGUF", "Qwen3-0.6B-Q8_0.gguf")
                    .await
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalMinistral3B => {
                self.load_local_model(
                    "ministral-3b",
                    "mistralai/Ministral-3-3B-Instruct-2512-GGUF",
                    "Ministral-3-3B-Instruct-2512-Q5_K_M.gguf",
                )
                .await
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalMinistral8B => {
                self.load_local_model(
                    "ministral-8b",
                    "mistralai/Ministral-3-8B-Instruct-2512-GGUF",
                    "Ministral-3-8B-Instruct-2512-Q5_K_M.gguf",
                )
                .await
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalGemma270M => {
                self.load_local_model(
                    "gemma-3-270m-it",
                    "unsloth/gemma-3-270m-it-GGUF",
                    "gemma-3-270m-it-Q4_0.gguf",
                )
                .await
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalGemma4B => {
                self.load_local_model(
                    "gemma-3-4b-it",
                    "unsloth/gemma-3-4b-it-GGUF",
                    "gemma-3-4b-it-Q4_0.gguf",
                )
                .await
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalGemma12B => {
                self.load_local_model(
                    "gemma-3-12b-it",
                    "unsloth/gemma-3-12b-it-GGUF",
                    "gemma-3-12b-it-Q4_0.gguf",
                )
                .await
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalDeepSeekCoder => {
                self.load_local_model(
                    "deepseek-coder-v2-lite-instruct",
                    "bartowski/DeepSeek-Coder-V2-Lite-Instruct-GGUF",
                    "DeepSeek-Coder-V2-Lite-Instruct-Q4_K_M.gguf",
                )
                .await
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalQwen35_27B => {
                self.load_local_model(
                    "qwen3.5-27b",
                    "unsloth/Qwen3.5-27B-GGUF",
                    "Qwen3.5-27B-UD-Q6_K_XL.gguf",
                )
                .await
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalGlm47Flash => {
                self.load_local_model(
                    "glm-4.7-flash",
                    "unsloth/GLM-4.7-Flash-GGUF",
                    "GLM-4.7-Flash-Q4_K_M.gguf",
                )
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
            #[allow(unreachable_patterns)]
            _ => Err(Error::ModelExecution(format!(
                "Model {model:?} not available (feature not enabled)"
            ))),
        }
    }
}
