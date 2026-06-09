use anyhow::Result;
use arkavo_llm::{LlmClient, Message};
use arkavo_router::{ModelChoice, Router, RoutingDecision};

pub struct LlmIntegration {
    router: Router,
}

impl LlmIntegration {
    pub async fn new() -> Result<Self> {
        let gemini_available = std::env::var("GEMINI_API_KEY").is_ok();

        let router = if gemini_available {
            Router::new().await?
        } else {
            tracing::info!("No GEMINI_API_KEY found - using local models only");
            Router::new_offline().await?
        };

        Ok(Self { router })
    }

    pub async fn new_offline() -> Result<Self> {
        Ok(Self {
            router: Router::new_offline().await?,
        })
    }

    pub async fn route_prompt(&self, prompt: &str) -> Result<RoutingDecision> {
        self.router
            .classify(prompt)
            .await
            .map_err(|e| anyhow::anyhow!("Router error: {}", e))
    }

    pub async fn create_client_from_routing(
        &self,
        decision: &RoutingDecision,
    ) -> Result<LlmClient> {
        match decision.recommended_model {
            ModelChoice::GeminiFlash
            | ModelChoice::Gemini35Flash
            | ModelChoice::Gemini35FlashMinimal
            | ModelChoice::Gemini35FlashMedium
            | ModelChoice::Gemini35FlashHigh
            | ModelChoice::GeminiPro => {
                #[cfg(feature = "gemini")]
                {
                    use arkavo_llm::GeminiProvider;
                    let provider = Box::new(GeminiProvider::new()?);
                    Ok(LlmClient::new(provider))
                }
                #[cfg(not(feature = "gemini"))]
                {
                    anyhow::bail!("Gemini feature not enabled")
                }
            }
            ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus | ModelChoice::ClaudeFable5 => {
                use arkavo_llm::providers::anthropic::AnthropicProvider;
                if let Ok(provider) =
                    AnthropicProvider::from_env_with_model(decision.recommended_model.name())
                {
                    Ok(LlmClient::new(Box::new(provider)))
                } else {
                    anyhow::bail!("ANTHROPIC_API_KEY not set")
                }
            }
            ModelChoice::DeepSeekV32 | ModelChoice::DeepSeekV32Speciale => {
                #[cfg(feature = "deepseek")]
                {
                    use arkavo_llm::DeepSeekProvider;
                    if let Ok(provider) = DeepSeekProvider::from_env() {
                        Ok(LlmClient::new(Box::new(provider)))
                    } else {
                        anyhow::bail!("DEEPSEEK_API_KEY not set")
                    }
                }
                #[cfg(not(feature = "deepseek"))]
                {
                    anyhow::bail!("DeepSeek feature not enabled")
                }
            }
            ModelChoice::KimiK2 => {
                #[cfg(feature = "kimi")]
                {
                    use arkavo_llm::KimiProvider;
                    if let Ok(provider) = KimiProvider::from_env() {
                        Ok(LlmClient::new(Box::new(provider)))
                    } else {
                        anyhow::bail!("MOONSHOT_API_KEY not set")
                    }
                }
                #[cfg(not(feature = "kimi"))]
                {
                    anyhow::bail!("Kimi feature not enabled")
                }
            }
            ModelChoice::LocalQwen3
            | ModelChoice::LocalMinistral3B
            | ModelChoice::LocalMinistral8B
            | ModelChoice::LocalQwen35_9B
            | ModelChoice::LocalQwen35_27B
            | ModelChoice::LocalQwen36A3B
            | ModelChoice::LocalGlm47Flash
            | ModelChoice::LocalGemma4E2B
            | ModelChoice::LocalGemma4E4B
            | ModelChoice::LocalGemma4_26B
            | ModelChoice::LocalGemma4_31B
            | ModelChoice::LocalGemma4_12B
            | ModelChoice::LocalGemma270M
            | ModelChoice::LocalGemma4B
            | ModelChoice::LocalGemma12B
            | ModelChoice::LocalDeepSeekCoder => {
                tracing::info!("Checking for Ollama...");
                if let Ok(client) = LlmClient::from_env()
                    && client.complete(vec![Message::user("ping")]).await.is_ok()
                {
                    tracing::info!("Using Ollama for local model");
                    return Ok(client);
                }

                #[cfg(feature = "llama-cpp")]
                {
                    tracing::info!("Ollama not available, using embedded llama.cpp...");

                    let model_name = decision.recommended_model.name();
                    self.initialize_llama_cpp_client(model_name).await
                }
                #[cfg(not(feature = "llama-cpp"))]
                {
                    anyhow::bail!(
                        "No local LLM available. Please install Ollama or enable llama-cpp feature."
                    )
                }
            }
        }
    }

    pub async fn create_client_from_model(&self, model_name: &str) -> Result<LlmClient> {
        if model_name.contains("gemini") {
            #[cfg(feature = "gemini")]
            {
                use arkavo_llm::GeminiProvider;
                let provider = Box::new(GeminiProvider::new()?);
                Ok(LlmClient::new(provider))
            }
            #[cfg(not(feature = "gemini"))]
            {
                anyhow::bail!("Gemini feature not enabled")
            }
        } else {
            if let Ok(client) = LlmClient::from_env()
                && client.complete(vec![Message::user("ping")]).await.is_ok()
            {
                return Ok(client);
            }

            #[cfg(feature = "llama-cpp")]
            {
                self.initialize_llama_cpp_client(model_name).await
            }
            #[cfg(not(feature = "llama-cpp"))]
            {
                anyhow::bail!(
                    "No local LLM available. Please install Ollama or enable llama-cpp feature."
                )
            }
        }
    }

    #[cfg(feature = "llama-cpp")]
    async fn initialize_llama_cpp_client(&self, model_name: &str) -> Result<LlmClient> {
        use arkavo_llm::LlamaCppProvider;

        let model_path = Self::resolve_model_path(model_name)?;

        tracing::info!("Loading llama.cpp model from: {}", model_path.display());

        let provider = LlamaCppProvider::new(
            model_name.to_string(),
            model_path.to_string_lossy().to_string(),
        )?;

        Ok(LlmClient::new(Box::new(provider)))
    }

    #[cfg(feature = "llama-cpp")]
    fn resolve_model_path(model_name: &str) -> Result<std::path::PathBuf> {
        use std::path::PathBuf;

        let home_dir = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map_err(|_| anyhow::anyhow!("Could not find home directory"))?;

        let hf_cache = PathBuf::from(&home_dir)
            .join(".cache")
            .join("huggingface")
            .join("hub");

        let model_repo = match model_name {
            "gemma-3-270m" => "models--unsloth--Qwen3-VL-4B-Instruct-unsloth-bnb-4bit",
            "gemma-4b" | "gemma-2-4b" => "models--unsloth--gemma-2-2b-it-bnb-4bit",
            "gemma-12b" => "models--lmstudio-community--gemma-2-9b-it-GGUF",
            _ => "models--unsloth--Qwen3-VL-4B-Instruct-unsloth-bnb-4bit",
        };

        let repo_path = hf_cache.join(model_repo);

        if !repo_path.exists() {
            anyhow::bail!(
                "Model repository not found at {}\nPlease run 'arkavo chat --prompt hi' first to download a model.",
                repo_path.display()
            );
        }

        let snapshots_dir = repo_path.join("snapshots");
        if !snapshots_dir.exists() {
            anyhow::bail!("Snapshots directory not found in model repository");
        }

        let mut snapshot_dirs: Vec<_> = std::fs::read_dir(&snapshots_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();

        if snapshot_dirs.is_empty() {
            anyhow::bail!("No snapshot found in model repository");
        }

        snapshot_dirs.sort_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });

        let latest_snapshot = snapshot_dirs.last().unwrap().path();

        let gguf_files: Vec<_> = std::fs::read_dir(&latest_snapshot)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "gguf")
                    .unwrap_or(false)
            })
            .collect();

        if gguf_files.is_empty() {
            anyhow::bail!("No GGUF file found in snapshot");
        }

        Ok(gguf_files[0].path())
    }
}
