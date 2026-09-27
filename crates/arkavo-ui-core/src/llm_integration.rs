use anyhow::Result;
use arkavo_llm::{LlmClient, Message};
use arkavo_router::{ModelChoice, Router, RoutingDecision};

/// Output allowance used to price a call before it is authorized.
const MAX_OUTPUT_TOKENS: u32 = 4096;

pub struct LlmIntegration {
    router: Router,
}

impl LlmIntegration {
    /// Build on a caller-supplied router, so a caller that already configured
    /// one — with its cloud policy, spend caps and provider set — is not
    /// silently served by a second router built from the environment.
    pub fn with_router(router: Router) -> Self {
        Self { router }
    }

    /// A gated client for `model`: the cloud policy and spend caps answer
    /// before the client exists, so a refusal never opens a connection.
    ///
    /// `explicit` says the caller named this arm itself (a `--model` choice),
    /// which is its own consent; an automatically routed arm is not.
    async fn gated_client(&self, model: &ModelChoice, explicit: bool) -> Result<LlmClient> {
        let usage = arkavo_router::usage::estimate_request(&[], None, MAX_OUTPUT_TOKENS);
        let cost = self.router.usage_cost(model, &usage);
        self.router
            .authorize_call(model, cost, explicit, None)
            .await?;
        if !explicit {
            self.router.require_provisioned(model)?;
        }
        Ok(LlmClient::new(
            self.router.get_provider_attributed(model).await?.0,
        ))
    }

    pub async fn new() -> Result<Self> {
        let cloud_available = arkavo_router::ProviderAvailability::from_env().has_cloud();

        let router = if cloud_available {
            Router::new().await?
        } else {
            tracing::info!("No cloud provider available - using local models only");
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
            ref model if model.is_cloud() => self.gated_client(model, false).await,
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
            ref model => self.gated_client(model, false).await,
        }
    }

    pub async fn create_client_from_model(&self, model_name: &str) -> Result<LlmClient> {
        if let Some(model) = ModelChoice::from_name(model_name)
            && model.is_cloud()
        {
            // Naming an arm is the caller's own consent to spend on it, but the
            // policy and the caps still decide.
            return self.gated_client(&model, true).await;
        }
        if model_name.contains("gemini") {
            // A gemini-shaped name that is not a routable arm has no pricing
            // and no policy identity, so it cannot be gated — and an ungated
            // cloud client is exactly what this path used to hand out.
            anyhow::bail!("Unknown cloud model: {model_name}")
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

#[cfg(test)]
// `#[tokio::test]` expands to `Runtime::block_on`, which this crate disallows
// outside tests.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_router::classifier::TaskCategory;
    use arkavo_router::{
        ConnectivityChecker, ModelSelector, ProviderAvailability, ProviderFactory,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts provider construction, so "the call was refused before a client
    /// existed" is an assertion rather than an inference.
    #[derive(Default)]
    struct CountingFactory {
        builds: AtomicUsize,
    }

    impl ProviderFactory for CountingFactory {
        fn build(
            &self,
            _model: &ModelChoice,
        ) -> arkavo_router::Result<Box<dyn arkavo_llm::Provider>> {
            self.builds.fetch_add(1, Ordering::SeqCst);
            Err(arkavo_router::Error::ModelExecution(
                "no client may be built in this test".to_string(),
            ))
        }
    }

    /// No local weights on disk, one configured cloud provider, cloud spend
    /// refused: the deployment where the ungated path used to reach the network.
    async fn local_only(factory: Arc<CountingFactory>) -> LlmIntegration {
        let mut router = Router::new_offline().await.unwrap();
        router.set_offline_mode(false);
        LlmIntegration::with_router(
            router
                .with_cloud_policy(arkavo_budget::CloudPolicy::LocalOnly)
                .with_connectivity(ConnectivityChecker::assume(true))
                .with_selector(ModelSelector::with_availability(
                    ProviderAvailability {
                        gemini: true,
                        ..ProviderAvailability::default()
                    },
                    false,
                ))
                .await
                .with_provider_factory(factory),
        )
    }

    /// Regression: this handed out a raw `GeminiProvider` built straight from
    /// the environment, so a `LocalOnly` install still sent the prompt to
    /// Gemini and the ledger never saw the call. Naming the arm is consent to
    /// spend on it, but the policy still decides.
    #[tokio::test]
    async fn a_named_cloud_model_faces_the_cloud_policy() {
        let factory = Arc::new(CountingFactory::default());
        let error = local_only(factory.clone())
            .await
            .create_client_from_model("gemini-3.5-flash")
            .await
            .err()
            .expect("a LocalOnly install must refuse a cloud client");
        assert!(
            error.to_string().contains("Cloud inference denied"),
            "got {error}"
        );
        assert_eq!(
            factory.builds.load(Ordering::SeqCst),
            0,
            "a refused call must not open a client"
        );
    }

    /// The routed path used `Router::get_provider`, which gated nothing either.
    #[tokio::test]
    async fn a_routed_cloud_arm_faces_the_cloud_policy() {
        let factory = Arc::new(CountingFactory::default());
        let decision = RoutingDecision::new(
            ModelChoice::Gemini35Flash,
            TaskCategory::General,
            0.9,
            "test".to_string(),
        );
        let error = local_only(factory.clone())
            .await
            .create_client_from_routing(&decision)
            .await
            .err()
            .expect("a LocalOnly install must refuse a cloud client");
        assert!(
            error.to_string().contains("Cloud inference denied"),
            "got {error}"
        );
        assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    }
}
