#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unused_async)]

pub mod architect;
pub mod classifier;
pub mod connectivity;
pub mod decision;
pub mod deliberation;
pub mod error;
pub mod health;
pub mod judge;
pub mod learning;
pub mod metrics;
pub mod model_discovery;
pub mod orchestrator;
pub mod prediction;
pub mod preflight;
pub mod rlm;
pub mod selector;
pub mod stream;
pub mod tool_request_parser;
pub mod tools;
pub mod validator;

pub use architect::{
    ArchitectExecutor, ArchitectPlan, ArchitectPlanner, ArchitectResult, ComplexityScore,
    ComplexityScorer, Subtask, SubtaskResult,
};
pub use classifier::{TaskCategory, TaskClassifier};
pub use connectivity::ConnectivityChecker;
pub use decision::{ModelChoice, PlannerTier, RoutingDecision};
pub use deliberation::{DeliberationConfig, DeliberationResult, Deliberator};
pub use error::{Error, Result};
pub use judge::{IssueType, JudgmentResult, ResponseJudge};
pub use metrics::RoutingMetrics;
pub use orchestrator::{
    ArchitectRoutingResult, CostOrchestrator, CostRecommendation, OrchestratorMetrics,
    ScalingDecision,
};
pub use prediction::{BudgetRunway, WorkflowCostPrediction, WorkflowCostPredictor};
pub use preflight::{ModerationResult, PolicyId, PreflightFeature, PreflightModerator};
pub use rlm::{
    RlmConfig, RlmContextManager, RlmDecompositionResult, RlmProbeResult, RlmSearchResult,
    RlmStats, SharedRlmManager, create_rlm_manager, create_rlm_manager_with_config,
};
pub use selector::{ModelSelector, ProviderAvailability};
pub use stream::{RouteMetadata, RouteResponse, RouteStream, StreamChunk};
pub use validator::{ResponseValidator, ValidationError};

pub use learning::{
    AgentContribution, AgentUtility, AgentUtilityStats, BetaPrior, BurstFeedback, FinalTaskReport,
    LearningConfig, LearningModule, QualityMetrics,
};

use arkavo_llm::{Message, Provider, ProviderResponse, StreamResponse, ToolParser};
use arkavo_mcp_tools::ToolRegistry;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_stream::Stream;

/// Default buffer size for streaming channels
/// Balance between memory usage and backpressure handling
const STREAM_BUFFER_SIZE: usize = 100;

/// Intelligent router for cost-optimized model selection
pub struct Router {
    classifier: Arc<TaskClassifier>,
    selector: Arc<ModelSelector>,
    metrics: Arc<RwLock<RoutingMetrics>>,
    connectivity: Arc<ConnectivityChecker>,
    offline_mode: bool,
    preflight: Option<Arc<preflight::PreflightModerator>>,
    #[cfg(feature = "critic")]
    critic: Option<Arc<arkavo_critic::CriticPipeline>>,
}

impl Router {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            classifier: Arc::new(TaskClassifier::new().await?),
            selector: Arc::new(ModelSelector::new()),
            metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
            connectivity: Arc::new(ConnectivityChecker::new()),
            offline_mode: false,
            preflight: None,
            #[cfg(feature = "critic")]
            critic: None,
        })
    }

    pub async fn new_offline() -> Result<Self> {
        Ok(Self {
            classifier: Arc::new(TaskClassifier::new().await?),
            selector: Arc::new(ModelSelector::new()),
            metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
            connectivity: Arc::new(ConnectivityChecker::new()),
            offline_mode: true,
            preflight: None,
            #[cfg(feature = "critic")]
            critic: None,
        })
    }

    /// Add pre-flight moderation to the router
    ///
    /// Pre-flight moderation evaluates TØR-G circuits against requests
    /// BEFORE LLM inference, blocking policy-violating requests early.
    #[must_use]
    pub fn with_preflight(mut self, moderator: preflight::PreflightModerator) -> Self {
        self.preflight = Some(Arc::new(moderator));
        self
    }

    /// Add post-LLM critic validation to the router
    ///
    /// The CriticPipeline validates LLM responses AFTER inference,
    /// checking for policy violations, schema errors, and semantic issues.
    #[cfg(feature = "critic")]
    #[must_use]
    pub fn with_critic(mut self, pipeline: arkavo_critic::CriticPipeline) -> Self {
        self.critic = Some(Arc::new(pipeline));
        self
    }

    pub fn set_offline_mode(&mut self, offline: bool) {
        self.offline_mode = offline;
    }

    pub async fn check_connectivity(&self) -> bool {
        self.connectivity.is_online().await
    }

    /// Get routing decision without executing (for callers that just need model selection)
    pub async fn classify(&self, task_description: &str) -> Result<RoutingDecision> {
        let classification = self.classifier.classify(task_description).await?;

        tracing::info!(
            category = ?classification.category,
            confidence = classification.confidence,
            "Task classified"
        );

        let mut decision = self.selector.select(&classification, task_description)?;

        tracing::info!(
            model = %decision.recommended_model.name(),
            reasoning = %decision.reasoning,
            "Model selected"
        );

        if (self.offline_mode || !self.connectivity.is_online().await)
            && decision.recommended_model.is_cloud()
        {
            let local_model = self.get_local_fallback(classification.category);

            decision.reasoning = format!(
                "Offline mode: Using local {}. Original: {}",
                local_model.name(),
                decision.reasoning
            );

            decision.recommended_model = local_model;
            decision.estimated_cost_usd = 0.0;
            decision.should_compress = false;
        }

        self.metrics
            .write()
            .await
            .record_routing(&classification, &decision);

        Ok(decision)
    }

    /// Unified routing function - routes task and returns a stream
    ///
    /// This is the main entry point for all routing operations. It:
    /// - Auto-detects task complexity and uses architect mode when beneficial
    /// - Handles quality validation and model escalation internally
    /// - Always returns a stream that can be iterated or awaited
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Streaming (real-time UI)
    /// let stream = router.route(task, messages, Some(&tools)).await?;
    /// while let Some(chunk) = stream.next().await {
    ///     print!("{}", chunk?.content);
    /// }
    ///
    /// // Await full response
    /// let response = router.route(task, messages, None).await?.complete().await?;
    /// ```
    #[allow(deprecated)] // Uses deprecated route_architect and route_with_quality_gate internally
    pub async fn route(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<RouteStream> {
        use crate::stream::{RouteResponse, RouteStream};

        // Pre-flight moderation check (before any LLM inference)
        if let Some(preflight) = &self.preflight {
            match preflight.check(task_description) {
                preflight::ModerationResult::Allow => {}
                preflight::ModerationResult::Block {
                    policy_id, reason, ..
                } => {
                    return Err(Error::ModerationBlocked { policy_id, reason });
                }
            }
        }

        // Check complexity for architect mode
        let scorer = ComplexityScorer::new();
        let complexity = scorer.analyze(task_description);

        if complexity.architect_recommended {
            // Use architect mode for complex tasks
            tracing::info!(
                "Architect mode activated: {} estimated subtasks",
                complexity.estimated_subtasks
            );

            let result = self
                .route_architect(task_description, messages, tool_registry)
                .await?;

            let response = RouteResponse {
                content: result.final_response,
                tool_calls: Vec::new(),
                model: ModelChoice::ClaudeOpus,
                cost_usd: result.actual_cost_usd,
                used_architect_mode: true,
                architect_savings: Some(result.actual_savings_usd),
            };

            return Ok(RouteStream::from_response(response));
        }

        // Simple task - use quality-gated routing
        let provider_response = self
            .route_with_quality_gate(task_description, messages, tool_registry, 3)
            .await?;

        let decision = self.classify(task_description).await?;

        let response = RouteResponse {
            content: provider_response.content,
            tool_calls: provider_response.tool_calls,
            model: decision.recommended_model,
            cost_usd: decision.estimated_cost_usd,
            used_architect_mode: false,
            architect_savings: None,
        };

        Ok(RouteStream::from_response(response))
    }

    fn get_local_fallback(&self, category: TaskCategory) -> ModelChoice {
        match category {
            TaskCategory::FrontendUI | TaskCategory::BackendAPI | TaskCategory::Refactoring => {
                ModelChoice::LocalMinistral3B
            }
            TaskCategory::CodeGeneration => ModelChoice::LocalMinistral3B,
            _ => ModelChoice::LocalQwen3,
        }
    }

    pub async fn get_metrics(&self) -> RoutingMetrics {
        self.metrics.read().await.clone()
    }

    /// Get a reference to the local provider for simple classification tasks
    pub fn get_local_provider(&self) -> Arc<TaskClassifier> {
        self.classifier.clone()
    }

    /// Get a Gemini provider for complex planning/thinking tasks
    /// Returns None if Gemini is not available (no API key)
    pub fn get_planning_provider(&self) -> Option<arkavo_llm::GeminiProvider> {
        arkavo_llm::GeminiProvider::new().ok()
    }

    /// Get a provider for the given model choice (local or cloud)
    ///
    /// This creates the appropriate provider for the specified model,
    /// supporting both local LlamaCpp models and cloud API providers.
    pub async fn get_provider(&self, model: &ModelChoice) -> Result<Box<dyn Provider>> {
        self.instantiate_provider(model).await
    }

    /// Check if Gemini API is available
    pub fn is_gemini_available(&self) -> bool {
        std::env::var("GEMINI_API_KEY").is_ok()
    }

    /// Check if Anthropic API is available
    pub fn is_anthropic_available(&self) -> bool {
        std::env::var("ANTHROPIC_API_KEY").is_ok()
    }

    /// Get Anthropic provider if configured
    pub fn get_anthropic_provider(
        &self,
    ) -> Option<arkavo_llm::providers::anthropic::AnthropicProvider> {
        arkavo_llm::providers::anthropic::AnthropicProvider::from_env().ok()
    }

    /// Get the list of available LLMs for status reporting
    pub fn get_available_llms(&self) -> Vec<LlmInfo> {
        let mut llms = Vec::new();

        // Check for Anthropic Claude
        if self.is_anthropic_available() {
            let model = std::env::var("ANTHROPIC_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-5-20250929".to_string());
            llms.push(LlmInfo {
                name: "Claude".to_string(),
                provider: "Anthropic".to_string(),
                model,
                available: true,
            });
        }

        // Check for Gemini
        if self.is_gemini_available() {
            let model = std::env::var("GEMINI_MODEL")
                .unwrap_or_else(|_| "gemini-3-pro-preview".to_string());
            llms.push(LlmInfo {
                name: "Gemini".to_string(),
                provider: "Google".to_string(),
                model,
                available: true,
            });
        }

        // Always include local model (prefer Qwen3/Ministral)
        llms.push(LlmInfo {
            name: "Local".to_string(),
            provider: "Local".to_string(),
            model: "qwen3-0.6b / ministral-3b".to_string(),
            available: true,
        });

        llms
    }

    /// Route with tools and Judge loop validation (local models only)
    ///
    /// Includes quality gate with:
    /// - ResponseValidator for fast validation (hallucinated tools, missing params)
    /// - ResponseJudge for LLM-based quality evaluation
    /// - Automatic model escalation within LOCAL models only (up to 3 retries)
    pub async fn route_with_tools(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<ProviderResponse> {
        const MAX_RETRIES: u8 = 3;
        let mut current_decision = self.classify(task_description).await?;

        // Estimate input tokens for context-aware tool discovery
        let input_tokens = Self::estimate_tokens(task_description);

        for attempt in 0..MAX_RETRIES {
            let tools_json = match tool_registry {
                Some(registry) => {
                    let detail_level =
                        Self::detail_level_for_model(&current_decision.recommended_model);
                    let keywords = Self::extract_keywords(task_description);

                    // Use hybrid search: semantic + token-based + context-size-aware
                    let tool_infos = Self::search_tools_hybrid(
                        registry,
                        &keywords,
                        detail_level,
                        Some(input_tokens),
                    )
                    .await;

                    Some(arkavo_llm::McpConverter::to_anthropic_format_minimal(
                        &tool_infos,
                    ))
                }
                None => None,
            };

            let provider = self
                .instantiate_provider(&current_decision.recommended_model)
                .await?;

            let mut response = provider
                .complete_with_tools(messages.clone(), tools_json, None)
                .await
                .map_err(|e| Error::ModelExecution(format!("Provider error: {e}")))?;

            // Post-process tool calls to handle language identifier fences (e.g., ```python)
            // The provider may return these as tool calls, but they contain nested tool calls
            response.tool_calls = Self::filter_and_extract_tool_calls(response.tool_calls);

            // Extract tool calls from text content if structured tool_calls is empty
            if response.tool_calls.is_empty() && !response.content.is_empty() {
                let extracted = Self::extract_tool_calls_from_text(&response.content);
                if !extracted.is_empty() {
                    tracing::debug!(
                        "Extracted {} tool calls from text response",
                        extracted.len()
                    );
                    response.tool_calls = extracted;
                }
            }

            // Debug: log tool calls
            if !response.tool_calls.is_empty() {
                for tc in &response.tool_calls {
                    tracing::debug!("[Judge] Tool call: {} args={}", tc.tool_name, tc.arguments);
                }
            }

            // Quality gate validation
            if let Some(registry) = tool_registry {
                let tool_infos = registry.list_tools();

                // Fast validation: check for hallucinated tools, missing params
                let validator = validator::ResponseValidator::new(&tool_infos);
                if let Err(validation_error) = validator.quick_validate(&response) {
                    tracing::warn!(
                        "Fast validation failed on attempt {}/{}: {}",
                        attempt + 1,
                        MAX_RETRIES,
                        validation_error
                    );

                    if attempt + 1 < MAX_RETRIES
                        && let Some(upgraded) =
                            Self::upgrade_model_local_only(&current_decision.recommended_model)
                    {
                        current_decision.recommended_model = upgraded;
                        tracing::info!(
                            "Upgrading to {:?} due to validation failure (local only)",
                            current_decision.recommended_model
                        );
                        continue;
                    }
                    // No more local upgrades available or max retries reached
                    // Return the response anyway with a warning
                    tracing::warn!(
                        "Validation failed but no local upgrade available, returning response"
                    );
                    return Ok(response);
                }

                // LLM-based Judge evaluation (only with llama-cpp feature)
                #[cfg(feature = "llama-cpp")]
                {
                    use crate::judge::IssueType;

                    match judge::ResponseJudge::new_local().await {
                        Ok(judge) => {
                            let judgment = judge
                                .evaluate(task_description, &response, &tool_infos, None)
                                .await?;

                            if !judgment.passed {
                                tracing::warn!(
                                    "Judge rejected response on attempt {}/{}: {:?} - {}",
                                    attempt + 1,
                                    MAX_RETRIES,
                                    judgment.issue_type,
                                    judgment.reason.as_deref().unwrap_or("No reason provided")
                                );

                                // Special handling for MissingToolUse
                                if judgment.issue_type == IssueType::MissingToolUse
                                    && !judgment.suggested_keywords.is_empty()
                                {
                                    tracing::info!(
                                        "Judge detected missing tool usage, searching for: {:?}",
                                        judgment.suggested_keywords
                                    );
                                    return Err(Error::ModelExecution(format!(
                                        "MISSING_TOOL_USE:{:?}",
                                        judgment.suggested_keywords
                                    )));
                                }

                                if attempt + 1 < MAX_RETRIES
                                    && let Some(upgraded) = Self::upgrade_model_local_only(
                                        &current_decision.recommended_model,
                                    )
                                {
                                    current_decision.recommended_model = upgraded;
                                    tracing::info!(
                                        "Upgrading to {:?} after judge rejection (local only)",
                                        current_decision.recommended_model
                                    );
                                    continue;
                                }
                                // No more local upgrades, return response with warning
                                tracing::warn!(
                                    "Judge rejected but no local upgrade available, returning response"
                                );
                                return Ok(response);
                            }
                        }
                        Err(e) => {
                            // Judge unavailable, skip LLM-based validation
                            tracing::debug!("Judge validation skipped (model unavailable): {}", e);
                        }
                    }
                }
            }

            return Ok(response);
        }

        Err(Error::ModelExecution(
            "Max retries exceeded without successful response".to_string(),
        ))
    }

    /// Upgrade model within local tier only - never escalate to cloud
    /// Only returns an upgrade if the target model is actually cached.
    fn upgrade_model_local_only(current: &ModelChoice) -> Option<ModelChoice> {
        let candidate = match current {
            // Qwen3/Ministral upgrade path
            ModelChoice::LocalQwen3 => Some(ModelChoice::LocalMinistral3B),
            ModelChoice::LocalMinistral3B => Some(ModelChoice::LocalMinistral8B),
            ModelChoice::LocalMinistral8B => None, // Already at max local

            // Gemma upgrade path
            ModelChoice::LocalGemma270M => Some(ModelChoice::LocalGemma4B),
            ModelChoice::LocalGemma4B => Some(ModelChoice::LocalGemma12B),
            ModelChoice::LocalGemma12B => None, // Already at max local

            // DeepSeek local stays local
            ModelChoice::LocalDeepSeekCoder => None, // Don't escalate to cloud

            // Cloud models - no local upgrade available
            _ => None,
        };

        // Verify the candidate model is actually cached before returning it
        candidate.filter(Self::is_model_cached_static)
    }

    /// Check if a local model is cached (static version for use in upgrade_model_local_only)
    fn is_model_cached_static(model: &ModelChoice) -> bool {
        match model {
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
            // Cloud models aren't cached locally
            _ => false,
        }
    }

    /// Route with automatic quality evaluation and model escalation
    #[deprecated(since = "0.44.0", note = "Use route() instead")]
    pub async fn route_with_quality_gate(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        max_retries: u8,
    ) -> Result<ProviderResponse> {
        let mut current_decision = self.classify(task_description).await?;

        // Estimate input tokens for context-aware tool discovery
        let input_tokens = Self::estimate_tokens(task_description);

        for attempt in 0..max_retries {
            let tools_json = match tool_registry {
                Some(registry) => {
                    let detail_level =
                        Self::detail_level_for_model(&current_decision.recommended_model);
                    let keywords = Self::extract_keywords(task_description);

                    // Use hybrid search: semantic + token-based + context-size-aware
                    let tool_infos = Self::search_tools_hybrid(
                        registry,
                        &keywords,
                        detail_level,
                        Some(input_tokens),
                    )
                    .await;

                    // Use the correct format based on the model provider
                    let json = match current_decision.recommended_model {
                        decision::ModelChoice::GeminiFlash | decision::ModelChoice::GeminiPro => {
                            arkavo_llm::McpConverter::to_gemini_format_minimal(&tool_infos)
                        }
                        _ => arkavo_llm::McpConverter::to_anthropic_format_minimal(&tool_infos),
                    };
                    Some(json)
                }
                None => None,
            };

            let provider = self
                .instantiate_provider(&current_decision.recommended_model)
                .await?;

            let mut response = provider
                .complete_with_tools(messages.clone(), tools_json.clone(), None)
                .await
                .map_err(|e| Error::ModelExecution(format!("Provider error: {e}")))?;

            // For local models, extract tool calls from text content if structured tool_calls is empty
            if response.tool_calls.is_empty() && !response.content.is_empty() {
                let extracted = Self::extract_tool_calls_from_text(&response.content);
                if !extracted.is_empty() {
                    tracing::debug!(
                        "Extracted {} tool calls from text response",
                        extracted.len()
                    );
                    response.tool_calls = extracted;
                }
            }

            // Debug: log tool calls
            if !response.tool_calls.is_empty() {
                for tc in &response.tool_calls {
                    eprintln!("[LLM] Tool call: {} args={}", tc.tool_name, tc.arguments);
                }
            }

            if let Some(registry) = tool_registry {
                let tool_infos = registry.list_tools();

                let validator = validator::ResponseValidator::new(&tool_infos);
                if let Err(validation_error) = validator.quick_validate(&response) {
                    tracing::warn!(
                        "Fast validation failed on attempt {}/{}: {}",
                        attempt + 1,
                        max_retries,
                        validation_error
                    );

                    if attempt + 1 < max_retries {
                        current_decision.recommended_model =
                            self.upgrade_model(&current_decision.recommended_model);
                        tracing::info!(
                            "Upgrading to {:?} due to validation failure",
                            current_decision.recommended_model
                        );
                        continue;
                    }
                    return Err(Error::ModelExecution(format!(
                        "Max retries exceeded. Last error: {validation_error}"
                    )));
                }

                #[cfg(feature = "llama-cpp")]
                {
                    // Use local model for cost-free judgment (prefers Qwen3/Ministral)
                    match judge::ResponseJudge::new_local().await {
                        Ok(judge) => {
                            let judgment = judge
                                .evaluate(task_description, &response, &tool_infos, None)
                                .await?;

                            if !judgment.passed {
                                tracing::warn!(
                                    "Judge rejected response on attempt {}/{}: {:?} - {}",
                                    attempt + 1,
                                    max_retries,
                                    judgment.issue_type,
                                    judgment.reason.as_deref().unwrap_or("No reason provided")
                                );

                                // Special handling for MissingToolUse - search for tools instead of upgrading model
                                if judgment.issue_type == IssueType::MissingToolUse
                                    && !judgment.suggested_keywords.is_empty()
                                {
                                    tracing::info!(
                                        "Judge detected missing tool usage, searching for: {:?}",
                                        judgment.suggested_keywords
                                    );

                                    // Return error with special marker to trigger tool search
                                    return Err(Error::ModelExecution(format!(
                                        "MISSING_TOOL_USE:{:?}",
                                        judgment.suggested_keywords
                                    )));
                                }

                                if attempt + 1 < max_retries {
                                    current_decision.recommended_model =
                                        self.upgrade_model(&current_decision.recommended_model);
                                    tracing::info!(
                                        "Upgrading to {:?} after judge rejection",
                                        current_decision.recommended_model
                                    );
                                    continue;
                                }
                                return Err(Error::ModelExecution(format!(
                                    "Max retries exceeded. Judge rejected: {:?}",
                                    judgment.issue_type
                                )));
                            }
                        }
                        Err(e) => {
                            // Judge unavailable, skip LLM-based validation
                            tracing::debug!("Judge validation skipped (model unavailable): {}", e);
                        }
                    }
                }
            }

            return Ok(response);
        }

        Err(Error::ModelExecution(
            "Max retries exceeded without successful response".to_string(),
        ))
    }

    /// Route with streaming and async quality validation
    /// Streams responses immediately for real-time UX, validates in background
    ///
    /// Note: Streaming mode cannot retry on failure - use route_with_quality_gate() for retry capability.
    /// The _max_retries parameter is kept for API compatibility with non-streaming version.
    #[deprecated(since = "0.44.0", note = "Use route() instead for streaming")]
    pub async fn route_with_quality_gate_stream(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        _max_retries: u8, // API compatibility - streaming cannot retry once started
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let decision = self.classify(task_description).await?;
        let provider = self
            .instantiate_provider(&decision.recommended_model)
            .await?;

        let stream = provider
            .stream(messages.clone())
            .await
            .map_err(|e| Error::ModelExecution(format!("Provider streaming error: {e}")))?;

        let (tx, rx) = tokio::sync::mpsc::channel(STREAM_BUFFER_SIZE);
        let model = decision.recommended_model.clone();
        let tool_infos = tool_registry.map(|r| r.list_tools());
        #[cfg(feature = "llama-cpp")]
        let task_desc = task_description.to_string();

        tokio::spawn(async move {
            let mut stream = stream;
            let mut accumulated = String::new();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        accumulated.push_str(&chunk.content);
                        if tx.send(Ok(chunk)).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(Error::ModelExecution(format!("Stream error: {e}"))))
                            .await;
                        break;
                    }
                }
            }

            // Async quality validation - runs after streaming completes
            // Future enhancement: Track validation failures for metrics/alerting
            if let Some(tools) = tool_infos {
                let response = ProviderResponse {
                    content: accumulated.clone(),
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    finish_reason: Some("stop".to_string()),
                };

                // Fast validation: Check for hallucinated tools and parameter issues
                let validator = validator::ResponseValidator::new(&tools);
                if let Err(e) = validator.quick_validate(&response) {
                    tracing::warn!(
                        target: "arkavo_router::quality_gate",
                        model = ?model,
                        error = %e,
                        "Async validation failed - consider metrics/alerting"
                    );
                }

                // Deep validation with LLM judge (when available)
                #[cfg(feature = "llama-cpp")]
                {
                    if let Ok(judge) = judge::ResponseJudge::new_local().await
                        && let Ok(judgment) =
                            judge.evaluate(&task_desc, &response, &tools, None).await
                        && !judgment.passed
                    {
                        tracing::warn!(
                            target: "arkavo_router::quality_gate",
                            model = ?model,
                            issue = ?judgment.issue_type,
                            reason = %judgment.reason.as_deref().unwrap_or("No reason"),
                            "Async judge rejected response - consider metrics/alerting"
                        );
                    }
                }
            }
        });

        Ok(Box::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    /// Route complex tasks through architect mode
    ///
    /// Auto-detects complex multi-step tasks and:
    /// 1. Uses Opus to create a decomposed plan
    /// 2. Auto-classifies each subtask
    /// 3. Routes subtasks to appropriate models (cheaper for frontend, premium for backend)
    /// 4. Aggregates results and tracks cost savings
    #[deprecated(
        since = "0.44.0",
        note = "Use route() instead - it auto-detects architect mode"
    )]
    #[allow(deprecated)] // Uses route_with_quality_gate internally
    pub async fn route_architect(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<architect::ArchitectResult> {
        use architect::{ArchitectExecutor, ArchitectPlanner, ComplexityScorer};

        // Step 1: Analyze complexity
        let scorer = ComplexityScorer::new();
        let complexity = scorer.analyze(task_description);

        // Step 2: Check if architect mode should activate
        if !complexity.architect_recommended {
            tracing::debug!(
                "Architect mode not recommended: {} subtasks, {} tokens",
                complexity.estimated_subtasks,
                complexity.estimated_output_tokens
            );

            // Fall back to normal quality-gated routing
            let response = self
                .route_with_quality_gate(task_description, messages, tool_registry, 3)
                .await?;

            return Ok(architect::ArchitectResult::single_task(
                response.content,
                0.0, // Would need actual cost from provider
            ));
        }

        tracing::info!(
            "Architect mode activated: {} estimated subtasks, {:.1}% estimated savings",
            complexity.estimated_subtasks,
            complexity.estimated_savings_percent
        );

        // Step 3: Generate plan using Opus
        let planner = ArchitectPlanner::new();
        let plan = planner.create_plan(task_description, complexity).await?;

        tracing::info!(
            "Created plan with {} subtasks, estimated ${:.4} (saves ${:.4})",
            plan.subtasks.len(),
            plan.architect_estimate_usd,
            plan.opus_only_estimate_usd - plan.architect_estimate_usd
        );

        // Step 4: Execute the plan
        let executor = ArchitectExecutor::new(Arc::new(self.clone_for_executor().await?));
        let result = executor.execute(&plan, messages, tool_registry).await?;

        tracing::info!(
            "Architect mode complete: actual cost ${:.4}, savings ${:.4} ({:.1}%)",
            result.actual_cost_usd,
            result.actual_savings_usd,
            result.savings_percent()
        );

        Ok(result)
    }

    /// Create a clone of Router for use in executor
    async fn clone_for_executor(&self) -> Result<Self> {
        Ok(Self {
            classifier: self.classifier.clone(),
            selector: self.selector.clone(),
            metrics: self.metrics.clone(),
            connectivity: self.connectivity.clone(),
            offline_mode: self.offline_mode,
            preflight: self.preflight.clone(),
            #[cfg(feature = "critic")]
            critic: self.critic.clone(),
        })
    }

    /// Upgrade to a more capable model, but only if it's available
    ///
    /// Returns the current model if the upgrade target is not installed/cached.
    /// This prevents upgrade loops where we try to use unavailable models.
    fn upgrade_model(&self, current: &ModelChoice) -> ModelChoice {
        let candidate = match current {
            // Qwen3/Ministral upgrade path - stay local, don't escalate to cloud
            ModelChoice::LocalQwen3 => ModelChoice::LocalMinistral3B,
            ModelChoice::LocalMinistral3B => ModelChoice::LocalMinistral8B,
            ModelChoice::LocalMinistral8B => ModelChoice::LocalMinistral8B, // Cap at local
            // Legacy Gemma upgrade path - stay local
            ModelChoice::LocalGemma270M => ModelChoice::LocalGemma4B,
            ModelChoice::LocalGemma4B => ModelChoice::LocalGemma12B,
            ModelChoice::LocalGemma12B => ModelChoice::LocalGemma12B, // Cap at local
            // Other upgrade paths
            ModelChoice::LocalDeepSeekCoder => ModelChoice::DeepSeekV32,
            ModelChoice::DeepSeekV32 => ModelChoice::ClaudeSonnet,
            ModelChoice::DeepSeekV32Speciale => ModelChoice::ClaudeOpus,
            ModelChoice::GeminiFlash => ModelChoice::ClaudeSonnet,
            ModelChoice::ClaudeSonnet => ModelChoice::GeminiPro,
            ModelChoice::GeminiPro => ModelChoice::ClaudeOpus,
            ModelChoice::ClaudeOpus => ModelChoice::ClaudeOpus,
        };

        // Check if the candidate model is available before upgrading
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

    /// Check if a model is available (installed/cached or has API key)
    fn is_model_available(&self, model: &ModelChoice) -> bool {
        match model {
            // Cloud models - check API keys
            ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus => self.is_anthropic_available(),
            ModelChoice::GeminiFlash | ModelChoice::GeminiPro => self.is_gemini_available(),
            ModelChoice::DeepSeekV32 | ModelChoice::DeepSeekV32Speciale => {
                std::env::var("DEEPSEEK_API_KEY").is_ok()
            }
            // Local models - check if cached
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
        }
    }

    async fn instantiate_provider(&self, model: &ModelChoice) -> Result<Box<dyn Provider>> {
        tracing::debug!(model = %model.name(), "Instantiating provider");
        match model {
            ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus => {
                use arkavo_llm::providers::anthropic::AnthropicProvider;
                if let Ok(provider) = AnthropicProvider::from_env() {
                    Ok(Box::new(provider))
                } else {
                    // Fallback to Gemini if available
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
                // Try to create Gemini provider, fallback to local if API key not available
                if let Ok(provider) = arkavo_llm::GeminiProvider::new() {
                    Ok(Box::new(provider))
                } else {
                    // Fallback to local model when Gemini API key is not available
                    #[cfg(feature = "llama-cpp")]
                    {
                        // Use model discovery to find any compatible model (prefers Qwen3/Ministral)
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

                        let provider = arkavo_llm::LlamaCppProvider::new(
                            model_name,
                            model_path.to_string_lossy().to_string(),
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
                let model_path = model_discovery::find_gguf_model(
                    "Qwen/Qwen3-0.6B-GGUF",
                    "Qwen3-0.6B-Q8_0.gguf",
                )
                .await
                .map_err(Error::ModelExecution)?;

                tracing::info!(path = %model_path.display(), "Loading Qwen3 model");

                let provider = arkavo_llm::LlamaCppProvider::new(
                    "qwen3-0.6b".to_string(),
                    model_path.to_string_lossy().to_string(),
                )
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to create LlamaCpp provider: {e}"))
                })?;

                tracing::info!("Qwen3 provider ready");
                Ok(Box::new(provider))
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalMinistral3B => {
                let model_path = model_discovery::find_gguf_model(
                    "mistralai/Ministral-3-3B-Instruct-2512-GGUF",
                    "Ministral-3-3B-Instruct-2512-Q5_K_M.gguf",
                )
                .await
                .map_err(Error::ModelExecution)?;

                tracing::info!(path = %model_path.display(), "Loading Ministral-3B model");

                let provider = arkavo_llm::LlamaCppProvider::new(
                    "ministral-3b".to_string(),
                    model_path.to_string_lossy().to_string(),
                )
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to create LlamaCpp provider: {e}"))
                })?;

                tracing::info!("Ministral-3B provider ready");
                Ok(Box::new(provider))
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalMinistral8B => {
                let model_path = model_discovery::find_gguf_model(
                    "mistralai/Ministral-3-8B-Instruct-2512-GGUF",
                    "Ministral-3-8B-Instruct-2512-Q5_K_M.gguf",
                )
                .await
                .map_err(Error::ModelExecution)?;

                let provider = arkavo_llm::LlamaCppProvider::new(
                    "ministral-8b".to_string(),
                    model_path.to_string_lossy().to_string(),
                )
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to create LlamaCpp provider: {e}"))
                })?;
                Ok(Box::new(provider))
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalGemma270M => {
                let model_path = model_discovery::find_gguf_model(
                    "unsloth/gemma-3-270m-it-GGUF",
                    "gemma-3-270m-it-Q4_0.gguf",
                )
                .await
                .map_err(Error::ModelExecution)?;

                let provider = arkavo_llm::LlamaCppProvider::new(
                    "gemma-3-270m-it".to_string(),
                    model_path.to_string_lossy().to_string(),
                )
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to create LlamaCpp provider: {e}"))
                })?;
                Ok(Box::new(provider))
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalGemma4B => {
                let model_path = model_discovery::find_gguf_model(
                    "unsloth/gemma-3-4b-it-GGUF",
                    "gemma-3-4b-it-Q4_0.gguf",
                )
                .await
                .map_err(Error::ModelExecution)?;

                let provider = arkavo_llm::LlamaCppProvider::new(
                    "gemma-3-4b-it".to_string(),
                    model_path.to_string_lossy().to_string(),
                )
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to create LlamaCpp provider: {e}"))
                })?;
                Ok(Box::new(provider))
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalGemma12B => {
                let model_path = model_discovery::find_gguf_model(
                    "unsloth/gemma-3-12b-it-GGUF",
                    "gemma-3-12b-it-Q4_0.gguf",
                )
                .await
                .map_err(Error::ModelExecution)?;

                let provider = arkavo_llm::LlamaCppProvider::new(
                    "gemma-3-12b-it".to_string(),
                    model_path.to_string_lossy().to_string(),
                )
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to create LlamaCpp provider: {e}"))
                })?;
                Ok(Box::new(provider))
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalDeepSeekCoder => {
                let model_path = model_discovery::find_gguf_model(
                    "bartowski/DeepSeek-Coder-V2-Lite-Instruct-GGUF",
                    "DeepSeek-Coder-V2-Lite-Instruct-Q4_K_M.gguf",
                )
                .await
                .map_err(Error::ModelExecution)?;

                let provider = arkavo_llm::LlamaCppProvider::new(
                    "deepseek-coder-v2-lite-instruct".to_string(),
                    model_path.to_string_lossy().to_string(),
                )
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to create DeepSeek-Coder provider: {e}"))
                })?;
                Ok(Box::new(provider))
            }
            #[allow(unreachable_patterns)]
            _ => Err(Error::ModelExecution(format!(
                "Model {model:?} not available (feature not enabled)"
            ))),
        }
    }

    /// Extract keywords from task description for tool search
    fn extract_keywords(task: &str) -> String {
        let words: Vec<&str> = task
            .split_whitespace()
            .filter(|w| w.len() > 2) // Allow 3-char words like "all", "get", "set"
            .filter(|w| {
                ![
                    "this", "that", "with", "have", "from", "what", "where", "the", "and", "for",
                ]
                .contains(w)
            })
            .collect();
        words.join(" ")
    }

    /// Estimate token count from text (approximately 4 chars per token)
    fn estimate_tokens(text: &str) -> usize {
        text.len() / 4
    }

    /// Determine detail level based on model context size
    fn detail_level_for_model(model: &decision::ModelChoice) -> arkavo_mcp_tools::DetailLevel {
        use decision::ModelChoice;
        match model {
            // Small models - minimal context
            ModelChoice::LocalQwen3 | ModelChoice::LocalGemma270M => {
                arkavo_mcp_tools::DetailLevel::NameOnly
            }
            // Medium models - some description
            ModelChoice::LocalMinistral3B
            | ModelChoice::LocalGemma4B
            | ModelChoice::LocalGemma12B
            | ModelChoice::LocalDeepSeekCoder => arkavo_mcp_tools::DetailLevel::NameAndDescription,
            // Large models and cloud - full schema
            ModelChoice::LocalMinistral8B
            | ModelChoice::GeminiFlash
            | ModelChoice::GeminiPro
            | ModelChoice::ClaudeSonnet
            | ModelChoice::ClaudeOpus
            | ModelChoice::DeepSeekV32
            | ModelChoice::DeepSeekV32Speciale => arkavo_mcp_tools::DetailLevel::FullSchema,
        }
    }

    /// Search tools using hybrid approach: semantic (if available) + token-based
    ///
    /// When input_tokens exceeds RLM threshold (70% of model context), automatically
    /// includes "context" in the search to surface RLM context tools (context_search,
    /// context_probe, context_decompose) for large context handling.
    async fn search_tools_hybrid(
        registry: &arkavo_mcp_tools::ToolRegistry,
        query: &str,
        detail: arkavo_mcp_tools::DetailLevel,
        input_tokens: Option<usize>,
    ) -> Vec<arkavo_mcp_tools::MinimalToolInfo> {
        // Check if we should surface RLM context tools based on input size
        let augmented_query = if let Some(tokens) = input_tokens {
            // RLM activates at 70% of typical model context (8K = 5600 tokens)
            const RLM_THRESHOLD: usize = 5600;
            if tokens > RLM_THRESHOLD {
                tracing::debug!(
                    "Large context detected ({} tokens > {}), surfacing context tools",
                    tokens,
                    RLM_THRESHOLD
                );
                format!("{query} context")
            } else {
                query.to_string()
            }
        } else {
            query.to_string()
        };

        // See #383: Add semantic search with local model when llama-cpp feature is enabled
        registry.search_tools(&augmented_query, detail)
    }

    /// Filter and extract tool calls from provider-returned tool_calls
    ///
    /// When a local LLM returns a fence like ```python\ncontext_search(...)```,
    /// the provider parses this as tool_name="python" with the code as arguments.
    /// This function detects language identifiers and extracts the nested tool calls.
    fn filter_and_extract_tool_calls(
        tool_calls: Vec<arkavo_llm::ParsedToolCall>,
    ) -> Vec<arkavo_llm::ParsedToolCall> {
        const LANG_IDENTIFIERS: &[&str] = &[
            "python",
            "py",
            "javascript",
            "js",
            "typescript",
            "ts",
            "rust",
            "go",
            "java",
            "ruby",
            "bash",
            "sh",
            "shell",
            "zsh",
            "powershell",
            "ps1",
            "sql",
            "json",
            "yaml",
            "toml",
        ];

        tool_calls
            .into_iter()
            .flat_map(|call| {
                let tool_lower = call.tool_name.to_lowercase();

                if LANG_IDENTIFIERS.contains(&tool_lower.as_str()) {
                    // This is a language fence, try to extract nested tool calls
                    let content = serde_json::to_string(&call.arguments)
                        .unwrap_or_default()
                        .trim_matches('"')
                        .replace("\\n", "\n")
                        .replace("\\\"", "\"");

                    tracing::debug!(
                        "Found language fence '{}', extracting nested calls from: {}",
                        call.tool_name,
                        &content[..std::cmp::min(100, content.len())]
                    );

                    // Try Python function-call syntax first (most common for code blocks)
                    if let Some(extracted) = Self::extract_python_function_calls(&content) {
                        return extracted;
                    }

                    // Try other formats
                    if let Some(extracted) = Self::extract_json_tool_calls(&content) {
                        return extracted;
                    }

                    // No valid tool calls found in language fence
                    vec![]
                } else {
                    // Keep non-language tool calls as-is
                    vec![call]
                }
            })
            .collect()
    }

    /// Extract tool calls from text content using multiple parsing strategies
    ///
    /// Local LLMs output tool calls as text in various formats. This function
    /// tries multiple parsers to be accommodating of different output styles:
    /// 1. Fence format: ```tool_name\nkey: value\n```
    /// 2. JSON format: {"tool": "name", "parameters": {...}}
    /// 3. XML format: <tool_call><name>...</name><arguments>...</arguments></tool_call>
    /// 4. Python function-call syntax: tool_name(param="value")
    fn extract_tool_calls_from_text(content: &str) -> Vec<arkavo_llm::ParsedToolCall> {
        // Known programming language identifiers that aren't tool names
        const LANG_IDENTIFIERS: &[&str] = &[
            "python",
            "py",
            "javascript",
            "js",
            "typescript",
            "ts",
            "rust",
            "go",
            "java",
            "ruby",
            "bash",
            "sh",
            "shell",
            "zsh",
            "powershell",
            "ps1",
            "sql",
            "json",
            "yaml",
            "toml",
        ];

        // Try fence format first (most common for local models)
        if let Ok(calls) = ToolParser::parse_fence(content) {
            // Handle special cases where fence type is a language, not a tool
            let filtered: Vec<_> = calls
                .into_iter()
                .flat_map(|call| {
                    let tool_lower = call.tool_name.to_lowercase();

                    if tool_lower == "json" {
                        // The content is JSON, try to extract tool call from it
                        let json_str = serde_json::to_string(&call.arguments).unwrap_or_default();
                        if let Some(extracted) = Self::extract_json_tool_calls(&json_str) {
                            return extracted;
                        }
                        vec![call]
                    } else if LANG_IDENTIFIERS.contains(&tool_lower.as_str()) {
                        // Fence is a language identifier (e.g., ```python)
                        // Try to extract tool calls from the content
                        // First, try Python function-call syntax
                        let fence_content = serde_json::to_string(&call.arguments)
                            .unwrap_or_default()
                            .trim_matches('"')
                            .to_string();
                        if let Some(extracted) = Self::extract_python_function_calls(&fence_content)
                        {
                            return extracted;
                        }
                        // Try to find tool name as first word on a line
                        if let Some(extracted) = Self::extract_tool_from_first_word(&fence_content)
                        {
                            return extracted;
                        }
                        vec![] // No tool found in language fence, filter it out
                    } else {
                        vec![call]
                    }
                })
                .collect();

            if !filtered.is_empty() {
                return filtered;
            }
        }

        // Try to find JSON tool calls embedded in text
        if let Some(calls) = Self::extract_json_tool_calls(content) {
            return calls;
        }

        // Try XML format
        if let Ok(calls) = ToolParser::parse_xml(content) {
            return calls;
        }

        // Try Python function-call syntax: tool_name(param="value")
        if let Some(calls) = Self::extract_python_function_calls(content) {
            return calls;
        }

        Vec::new()
    }

    /// Extract Python-style function calls from text
    ///
    /// Handles formats like:
    /// - tool_name(param="value")
    /// - result = tool-name(param="value", param2=123)
    /// - context_probe(indices=[0, 1])
    /// - `context_search(keywords=["security", "auth"])`
    /// - Code inside markdown blocks: ```python\ncontext_search(...)\n```
    fn extract_python_function_calls(content: &str) -> Option<Vec<arkavo_llm::ParsedToolCall>> {
        use regex::Regex;

        // First, extract content from markdown code blocks to avoid matching "python" as tool
        let content_to_parse = Self::extract_code_block_content(content);

        // Match: optional_var = tool-name(args) - handle nested brackets in args
        let re = match Regex::new(
            r"(?:\w+\s*=\s*)?([a-zA-Z][a-zA-Z0-9_-]*)\(([^)]*(?:\[[^\]]*\][^)]*)*)\)",
        ) {
            Ok(r) => r,
            Err(_) => return None,
        };

        let mut calls = Vec::new();

        for cap in re.captures_iter(&content_to_parse) {
            let (tool_name, args_str) = match (cap.get(1), cap.get(2)) {
                (Some(t), Some(a)) => (t.as_str().to_string(), a.as_str()),
                _ => continue,
            };

            // Skip common Python built-ins and markdown code fence language hints
            if [
                "print",
                "len",
                "str",
                "int",
                "float",
                "list",
                "dict",
                "range",
                "type",
                "python",
                "bash",
                "shell",
                "rust",
                "javascript",
                "js",
                "typescript",
                "ts",
                "json",
                "yaml",
            ]
            .contains(&tool_name.as_str())
            {
                continue;
            }

            let args = Self::parse_python_kwargs(args_str);

            calls.push(arkavo_llm::ParsedToolCall {
                tool_name,
                arguments: serde_json::Value::Object(args),
                call_id: None,
            });
        }

        if calls.is_empty() { None } else { Some(calls) }
    }

    /// Extract content from markdown code blocks
    ///
    /// Handles ```language\ncode\n``` blocks, returning the code content
    /// with the language hint stripped out. Also returns non-block content.
    fn extract_code_block_content(content: &str) -> String {
        use regex::Regex;

        // Match markdown code blocks: ```lang\ncode\n```
        let code_block_re = match Regex::new(r"```(?:\w+)?\s*\n([\s\S]*?)```") {
            Ok(r) => r,
            Err(_) => return content.to_string(),
        };

        let mut extracted = String::new();

        // Extract content from inside code blocks
        for cap in code_block_re.captures_iter(content) {
            if let Some(code) = cap.get(1) {
                extracted.push_str(code.as_str());
                extracted.push('\n');
            }
        }

        // Also include content outside code blocks (for inline function calls)
        // Remove code blocks first, then add remaining content
        let without_blocks = code_block_re.replace_all(content, "");
        extracted.push_str(&without_blocks);

        extracted
    }

    /// Parse Python keyword arguments including lists
    ///
    /// Handles: `param="value"`, `param2=123`, `indices=[0, 1]`, `keywords=["a", "b"]`
    fn parse_python_kwargs(args_str: &str) -> serde_json::Map<String, serde_json::Value> {
        use regex::Regex;

        let mut args = serde_json::Map::new();

        // Match keyword=value pairs, handling lists with brackets
        // Pattern: word = (string | number | list | identifier)
        let kwarg_re = match Regex::new(
            r#"(\w+)\s*=\s*(?:"([^"]*)"|'([^']*)'|(\d+(?:\.\d+)?)|(\[[^\]]*\])|(\w+))"#,
        ) {
            Ok(r) => r,
            Err(_) => return args,
        };

        for kwarg in kwarg_re.captures_iter(args_str) {
            let key = match kwarg.get(1) {
                Some(k) => k.as_str().to_string(),
                None => continue,
            };

            let value = if let Some(s) = kwarg.get(2) {
                // Double-quoted string
                serde_json::Value::String(s.as_str().to_string())
            } else if let Some(s) = kwarg.get(3) {
                // Single-quoted string
                serde_json::Value::String(s.as_str().to_string())
            } else if let Some(n) = kwarg.get(4) {
                // Number
                if let Ok(num) = n.as_str().parse::<i64>() {
                    serde_json::json!(num)
                } else if let Ok(num) = n.as_str().parse::<f64>() {
                    serde_json::json!(num)
                } else {
                    serde_json::Value::String(n.as_str().to_string())
                }
            } else if let Some(list_str) = kwarg.get(5) {
                // List: [0, 1] or ["a", "b"]
                Self::parse_python_list(list_str.as_str())
            } else if let Some(id) = kwarg.get(6) {
                // Bare identifier (True, False, None, or variable name)
                match id.as_str() {
                    "True" => serde_json::Value::Bool(true),
                    "False" => serde_json::Value::Bool(false),
                    "None" => serde_json::Value::Null,
                    other => serde_json::Value::String(other.to_string()),
                }
            } else {
                continue;
            };

            args.insert(key, value);
        }

        args
    }

    /// Parse a Python list literal like `[0, 1]` or `["a", "b"]`
    fn parse_python_list(list_str: &str) -> serde_json::Value {
        // Remove brackets and split by comma
        let inner = list_str
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim();

        if inner.is_empty() {
            return serde_json::json!([]);
        }

        let mut items = Vec::new();

        // Simple split by comma - handles basic cases
        for item in inner.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }

            // Try to parse as number first
            if let Ok(n) = item.parse::<i64>() {
                items.push(serde_json::json!(n));
            } else if let Ok(n) = item.parse::<f64>() {
                items.push(serde_json::json!(n));
            } else if item.starts_with('"') && item.ends_with('"') {
                // Double-quoted string
                let s = item.trim_matches('"');
                items.push(serde_json::Value::String(s.to_string()));
            } else if item.starts_with('\'') && item.ends_with('\'') {
                // Single-quoted string
                let s = item.trim_matches('\'');
                items.push(serde_json::Value::String(s.to_string()));
            } else {
                // Bare identifier or other
                items.push(serde_json::Value::String(item.to_string()));
            }
        }

        serde_json::Value::Array(items)
    }

    /// Extract tool name from first word on each line
    ///
    /// Handles cases like:
    /// ```python
    /// detect-gamemode()
    /// get-position()
    /// ```
    /// Where the fence is a language but the first word is the tool name
    fn extract_tool_from_first_word(content: &str) -> Option<Vec<arkavo_llm::ParsedToolCall>> {
        let mut calls = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }

            // Extract first word (tool name) - may end with () or have args
            // Match: tool-name or tool-name() or tool_name(args)
            let first_word = trimmed
                .split(|c: char| c.is_whitespace() || c == '(' || c == '=')
                .next()
                .unwrap_or("")
                .trim();

            // Must look like a tool name (contains letters, may have hyphens/underscores)
            if first_word.is_empty()
                || !first_word.chars().next().unwrap_or(' ').is_alphabetic()
                || first_word
                    .chars()
                    .all(|c| c.is_lowercase() && c.is_ascii_alphabetic())
                    && first_word.len() < 3
            {
                continue;
            }

            // Skip common language keywords
            if [
                "import", "from", "def", "class", "if", "else", "for", "while", "return", "let",
                "const", "var", "function", "async", "await", "try", "catch", "finally",
            ]
            .contains(&first_word)
            {
                continue;
            }

            // Check if this looks like a tool call (has parentheses or follows = assignment)
            if trimmed.contains('(') || trimmed.contains('=') {
                calls.push(arkavo_llm::ParsedToolCall {
                    tool_name: first_word.to_string(),
                    arguments: serde_json::Value::Object(serde_json::Map::new()),
                    call_id: None,
                });
            }
        }

        if calls.is_empty() { None } else { Some(calls) }
    }

    /// Extract JSON-formatted tool calls from text
    ///
    /// Handles various JSON formats LLMs might output:
    /// - {"tool": "name", "parameters": {...}}
    /// - {"tool_name": "name", "args": {...}}
    /// - {"name": "tool", "arguments": {...}}
    fn extract_json_tool_calls(content: &str) -> Option<Vec<arkavo_llm::ParsedToolCall>> {
        let mut calls = Vec::new();

        // Find JSON objects in the content (between { and })
        let mut depth = 0;
        let mut start = None;

        for (i, c) in content.char_indices() {
            match c {
                '{' => {
                    if depth == 0 {
                        start = Some(i);
                    }
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(s) = start {
                            let json_str = &content[s..=i];
                            if let Some(call) = Self::parse_json_tool_call(json_str) {
                                calls.push(call);
                            }
                        }
                        start = None;
                    }
                }
                _ => {}
            }
        }

        if calls.is_empty() { None } else { Some(calls) }
    }

    /// Parse a single JSON object as a tool call
    fn parse_json_tool_call(json_str: &str) -> Option<arkavo_llm::ParsedToolCall> {
        let json: serde_json::Value = serde_json::from_str(json_str).ok()?;
        let obj = json.as_object()?;

        // Try different field names for tool name
        let tool_name = obj
            .get("tool")
            .or_else(|| obj.get("tool_name"))
            .or_else(|| obj.get("name"))
            .and_then(|v| v.as_str())?
            .to_string();

        // Skip if it doesn't look like a tool call (must have a colon or be a known tool pattern)
        if tool_name.is_empty() {
            return None;
        }

        // Try different field names for arguments
        let arguments = obj
            .get("parameters")
            .or_else(|| obj.get("args"))
            .or_else(|| obj.get("arguments"))
            .or_else(|| obj.get("input"))
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));

        Some(arkavo_llm::ParsedToolCall {
            tool_name,
            arguments,
            call_id: None,
        })
    }
}

/// Information about an available LLM
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmInfo {
    pub name: String,
    pub provider: String,
    pub model: String,
    pub available: bool,
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_router_creation() {
        let result = Router::new().await;
        if result.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_python_list_numbers() {
        let result = Router::parse_python_list("[0, 1, 2]");
        assert_eq!(result, serde_json::json!([0, 1, 2]));
    }

    #[test]
    fn test_parse_python_list_strings() {
        let result = Router::parse_python_list(r#"["security", "password"]"#);
        assert_eq!(result, serde_json::json!(["security", "password"]));
    }

    #[test]
    fn test_parse_python_kwargs_with_list() {
        let result = Router::parse_python_kwargs(r#"indices=[0, 1], name="test""#);
        assert_eq!(result.get("indices"), Some(&serde_json::json!([0, 1])));
        assert_eq!(result.get("name"), Some(&serde_json::json!("test")));
    }

    #[test]
    fn test_extract_python_function_calls_with_list() {
        let content = r#"context_probe(indices=[0, 1])"#;
        let calls = Router::extract_python_function_calls(content).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "context_probe");
        assert_eq!(
            calls[0].arguments.get("indices"),
            Some(&serde_json::json!([0, 1]))
        );
    }

    #[test]
    fn test_extract_python_function_calls_context_search() {
        let content = r#"context_search(keywords=["security", "auth"])"#;
        let calls = Router::extract_python_function_calls(content).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "context_search");
        assert_eq!(
            calls[0].arguments.get("keywords"),
            Some(&serde_json::json!(["security", "auth"]))
        );
    }

    #[test]
    fn test_extract_tool_calls_from_llm_output() {
        // Simulates actual LLM output with Python code block
        let content = r#"
Here's how to search:

```python
results = context_search(keywords=["security", "password"])
chunks = context_probe(indices=[0, 1, 2])
```
"#;
        let calls = Router::extract_tool_calls_from_text(content);
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().any(|c| c.tool_name == "context_search"));
        assert!(calls.iter().any(|c| c.tool_name == "context_probe"));

        // Ensure "python" is not extracted as a tool
        assert!(!calls.iter().any(|c| c.tool_name == "python"));
    }

    #[test]
    fn test_extract_code_block_content() {
        let content = r#"
Let me explain:

```python
result = context_search(keywords=["auth"])
```

You can also use:

```bash
echo "hello"
```
"#;
        let extracted = Router::extract_code_block_content(content);

        // Should contain the code from inside the blocks
        assert!(extracted.contains("result = context_search"));
        // Should NOT contain the language hints as standalone text
        // (they're stripped from the fence markers)
    }
}
