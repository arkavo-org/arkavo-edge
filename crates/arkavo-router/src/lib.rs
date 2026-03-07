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
pub mod prompt_advisor;
pub mod provider;
pub mod provider_info;
pub(crate) mod quality_gate;
pub mod response;
pub mod rlm;
pub(crate) mod routing_deprecated;
pub mod selector;
pub mod selector_quality;
pub mod stream;
#[cfg(feature = "tdf-encrypt")]
pub mod tdf_audit;
#[allow(clippy::redundant_pub_crate)]
pub(crate) mod tool_extraction;
pub mod tool_request_parser;
pub mod tools;
pub mod validator;

pub use architect::{
    ArchitectExecutor, ArchitectPlan, ArchitectPlanner, ArchitectResult, ComplexityScore,
    ComplexityScorer, Subtask, SubtaskResult,
};
pub use classifier::{TaskCategory, TaskClassifier, classify_task_keywords};
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
pub use preflight::{
    AgentConfig, BudgetYamlConfig, KasYamlConfig, ModerationResult, PolicyId, PreflightFeature,
    PreflightModerator, build_moderator_from_config, load_agent_config,
};
pub use prompt_advisor::{AdvisorIssue, DynamicSnapshot, PromptAdvice, PromptAdvisor};
pub use provider_info::LlmInfo;
pub use rlm::{
    RlmConfig, RlmContextManager, RlmDecompositionResult, RlmProbeResult, RlmSearchResult,
    RlmStats, SharedRlmManager, create_rlm_manager, create_rlm_manager_with_config,
};
pub use selector::{ModelSelector, ProviderAvailability};
pub use stream::{RouteMetadata, RouteResponse, RouteStream, StreamChunk};
pub use validator::{ResponseValidator, ValidationError};

#[cfg(feature = "tdf-encrypt")]
pub use tdf_audit::{MessageEncryptor, TdfAuditConfig};

// Re-export response processing types
pub use response::{sanitize_response, strip_think_blocks, strip_tool_blocks};

pub use learning::{
    AgentContribution, AgentUtility, AgentUtilityStats, BetaPrior, BurstFeedback, FinalTaskReport,
    LearningConfig, LearningModule, QualityMetrics,
};

use arkavo_llm::Message;
#[cfg(feature = "llama-cpp")]
use arkavo_llm::ModelRegistry;
use arkavo_mcp_tools::ToolRegistry;
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};

/// Base cooldown after first availability failure (5 minutes).
/// Doubles on each consecutive failure: 5 → 10 → 20 → 40 → 60 (capped).
/// Resets to base after the first successful response from that model.
const MODEL_COOLDOWN_BASE_SECS: u64 = 300;

/// Maximum cooldown duration regardless of consecutive failures (1 hour).
const MODEL_COOLDOWN_MAX_SECS: u64 = 3600;

/// Short cooldown for quality failures (timeout, no tool calls).
/// Shorter than availability cooldown to allow faster model rotation.
const MODEL_QUALITY_COOLDOWN_BASE_SECS: u64 = 30;

/// Intelligent router for cost-optimized model selection
pub struct Router {
    classifier: Arc<TaskClassifier>,
    selector: Arc<ModelSelector>,
    model_learning: Arc<LearningModule>,
    /// Cached local models — loaded once, reused across requests.
    #[cfg(feature = "llama-cpp")]
    model_registry: Arc<ModelRegistry>,
    /// Temporarily excluded models: name → (when, consecutive_failures).
    /// Cooldown duration doubles each consecutive failure, resets on success.
    model_cooldowns: Arc<RwLock<std::collections::HashMap<String, (std::time::Instant, u32)>>>,
    metrics: Arc<RwLock<RoutingMetrics>>,
    connectivity: Arc<ConnectivityChecker>,
    offline_mode: bool,
    preflight: Option<Arc<preflight::PreflightModerator>>,
    advisor: Arc<PromptAdvisor>,
    #[cfg(feature = "critic")]
    critic: Option<Arc<arkavo_critic::CriticPipeline>>,
    #[cfg(feature = "advisor-persistence")]
    advisor_store: Option<Arc<arkavo_memory::AdvisorStateStore>>,
    #[cfg(feature = "advisor-persistence")]
    advisor_persist_count: std::sync::atomic::AtomicU64,
    #[cfg(feature = "advisor-persistence")]
    advisor_last_persist: std::sync::Mutex<std::time::Instant>,
    #[cfg(feature = "tdf-encrypt")]
    tdf_encryptor: Option<Arc<tdf_audit::MessageEncryptor>>,
    #[cfg(feature = "tdf-encrypt")]
    tdf_audit_store: Option<Arc<arkavo_memory::TdfAuditStore>>,
    /// Serializes concurrent LLM inference calls for task/orchestrator work.
    /// Local models (llama-cpp) share a single KV cache and cannot handle
    /// concurrent context allocation. This semaphore queues requests so the
    /// second caller waits instead of failing with an OOM/slot error.
    inference_semaphore: Arc<Semaphore>,
    /// Separate semaphore for chat inference so chat never blocks game ticks.
    /// Uses the fastest local model (0.8B/3B) which has its own context pool.
    chat_semaphore: Arc<Semaphore>,
    /// Tracks which model was last selected by route_with_tools().
    /// The conductor reads this after tool execution to attribute
    /// reward-based corrective feedback to the right Thompson Sampling prior.
    last_routed_model: Arc<std::sync::RwLock<Option<String>>>,
    /// Last routing decision trace for downstream attribution
    last_decision_trace: Arc<std::sync::RwLock<Option<learning::DecisionTrace>>>,
    /// Recent decision traces for UI dashboard (ring buffer, max 50)
    recent_traces: Arc<std::sync::RwLock<std::collections::VecDeque<learning::DecisionTrace>>>,
}

impl Router {
    pub async fn new() -> Result<Self> {
        let selector = Arc::new(ModelSelector::new());
        let model_learning = Arc::new(LearningModule::new());
        selector_quality::seed_model_learning(&selector, &model_learning).await;

        Ok(Self {
            classifier: Arc::new(TaskClassifier::new().await?),
            selector,
            model_learning,
            #[cfg(feature = "llama-cpp")]
            model_registry: Arc::new(ModelRegistry::new()),
            model_cooldowns: Arc::new(RwLock::new(std::collections::HashMap::new())),
            metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
            connectivity: Arc::new(ConnectivityChecker::new()),
            offline_mode: false,
            preflight: None,
            advisor: Arc::new(PromptAdvisor::new()),
            #[cfg(feature = "critic")]
            critic: None,
            #[cfg(feature = "advisor-persistence")]
            advisor_store: None,
            #[cfg(feature = "advisor-persistence")]
            advisor_persist_count: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "advisor-persistence")]
            advisor_last_persist: std::sync::Mutex::new(std::time::Instant::now()),
            #[cfg(feature = "tdf-encrypt")]
            tdf_encryptor: None,
            #[cfg(feature = "tdf-encrypt")]
            tdf_audit_store: None,
            inference_semaphore: Arc::new(Semaphore::new(1)),
            chat_semaphore: Arc::new(Semaphore::new(1)),
            last_routed_model: Arc::new(std::sync::RwLock::new(None)),
            last_decision_trace: Arc::new(std::sync::RwLock::new(None)),
            recent_traces: Arc::new(std::sync::RwLock::new(std::collections::VecDeque::new())),
        })
    }

    pub async fn new_offline() -> Result<Self> {
        let selector = Arc::new(ModelSelector::new());
        let model_learning = Arc::new(LearningModule::new());
        selector_quality::seed_model_learning(&selector, &model_learning).await;

        Ok(Self {
            classifier: Arc::new(TaskClassifier::new().await?),
            selector,
            model_learning,
            #[cfg(feature = "llama-cpp")]
            model_registry: Arc::new(ModelRegistry::new()),
            model_cooldowns: Arc::new(RwLock::new(std::collections::HashMap::new())),
            metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
            connectivity: Arc::new(ConnectivityChecker::new()),
            offline_mode: true,
            preflight: None,
            advisor: Arc::new(PromptAdvisor::new()),
            #[cfg(feature = "critic")]
            critic: None,
            #[cfg(feature = "advisor-persistence")]
            advisor_store: None,
            #[cfg(feature = "advisor-persistence")]
            advisor_persist_count: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "advisor-persistence")]
            advisor_last_persist: std::sync::Mutex::new(std::time::Instant::now()),
            #[cfg(feature = "tdf-encrypt")]
            tdf_encryptor: None,
            #[cfg(feature = "tdf-encrypt")]
            tdf_audit_store: None,
            inference_semaphore: Arc::new(Semaphore::new(1)),
            chat_semaphore: Arc::new(Semaphore::new(1)),
            last_routed_model: Arc::new(std::sync::RwLock::new(None)),
            last_decision_trace: Arc::new(std::sync::RwLock::new(None)),
            recent_traces: Arc::new(std::sync::RwLock::new(std::collections::VecDeque::new())),
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

    /// Run preflight moderation check without full classification.
    ///
    /// Returns `None` if no preflight moderator is configured (allows all).
    /// Returns `Some(result)` with the moderation outcome otherwise.
    pub fn check_preflight(&self, input: &str) -> Option<preflight::ModerationResult> {
        self.preflight.as_ref().map(|pf| pf.check(input))
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

    /// Attach an advisor state store for persisting learned adjustments.
    ///
    /// Loads all previously persisted adjustments and imports them into
    /// the in-memory advisor. The store is then retained for runtime saves.
    #[cfg(feature = "advisor-persistence")]
    #[must_use]
    pub async fn with_advisor_store(mut self, store: arkavo_memory::AdvisorStateStore) -> Self {
        if let Ok(persisted) = store.load_all().await {
            let snapshots: Vec<prompt_advisor::DynamicSnapshot> = persisted
                .into_iter()
                .filter_map(|p| {
                    let issue = match p.issue.as_str() {
                        "UnwantedCodeFence" => AdvisorIssue::UnwantedCodeFence,
                        "OutputLoop" => AdvisorIssue::OutputLoop,
                        "WrongExpert" => AdvisorIssue::WrongExpert,
                        "Timeout" => AdvisorIssue::Timeout,
                        "ToolError" => AdvisorIssue::ToolError,
                        "NoToolCalls" => AdvisorIssue::NoToolCalls,
                        _ => return None,
                    };
                    Some(prompt_advisor::DynamicSnapshot {
                        label: p.label,
                        model_family: p.model_family,
                        issue,
                        text: p.text,
                        success_rate: p.success_rate,
                        applications: p.applications,
                        feedback_count: p.feedback_count,
                    })
                })
                .collect();

            if !snapshots.is_empty() {
                tracing::info!("Loaded {} persisted advisor adjustments", snapshots.len());
                self.advisor.import_dynamic(snapshots);
            }
        }

        self.advisor_store = Some(Arc::new(store));
        self
    }

    /// Attach a TDF encryptor for cloud-bound prompt audit.
    #[cfg(feature = "tdf-encrypt")]
    #[must_use]
    pub fn with_tdf_encryptor(mut self, encryptor: tdf_audit::MessageEncryptor) -> Self {
        self.tdf_encryptor = Some(Arc::new(encryptor));
        self
    }

    /// Attach a TDF audit store for persisting encryption manifests.
    #[cfg(feature = "tdf-encrypt")]
    #[must_use]
    pub fn with_tdf_audit_store(mut self, store: arkavo_memory::TdfAuditStore) -> Self {
        self.tdf_audit_store = Some(Arc::new(store));
        self
    }

    /// Get a reference to the prompt advisor
    pub fn advisor(&self) -> &PromptAdvisor {
        &self.advisor
    }

    /// Get a reference to the model learning module (Thompson Sampling state)
    pub fn model_learning(&self) -> &LearningModule {
        &self.model_learning
    }

    /// Get the model name last selected by `route_with_tools()`.
    ///
    /// Returns `None` if no routing has occurred yet. Used by the conductor
    /// to attribute reward-based corrective feedback to the right model.
    pub fn last_routed_model(&self) -> Option<String> {
        self.last_routed_model.read().ok().and_then(|g| g.clone())
    }

    /// Get the trace from the last routing decision.
    /// Used by the conductor to attribute trace IDs to tool call events.
    pub fn last_decision_trace(&self) -> Option<learning::DecisionTrace> {
        self.last_decision_trace.read().ok().and_then(|g| g.clone())
    }

    /// Get recent decision traces for the UI dashboard.
    pub fn recent_decision_traces(&self, limit: usize) -> Vec<learning::DecisionTrace> {
        self.recent_traces
            .read()
            .ok()
            .map(|g| g.iter().rev().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    /// Extract model family from a model name (e.g., "glm-4.7-flash" → "glm").
    ///
    /// Simple heuristic: take the first segment before any dash-digit pattern.
    pub fn detect_model_family(model_name: &str) -> String {
        let lower = model_name.to_lowercase();
        // Common model family prefixes
        for family in &[
            "glm",
            "qwen",
            "gemma",
            "ministral",
            "mistral",
            "deepseek",
            "llama",
            "phi",
        ] {
            if lower.starts_with(family) {
                return (*family).to_string();
            }
        }
        // Fallback: take first segment before '-'
        lower.split('-').next().unwrap_or(&lower).to_string()
    }

    /// Log the full Thompson Sampling state for all tracked models.
    ///
    /// Shows Beta prior (α, β), expected value, and observation count
    /// per (model, category) pair. Useful for diagnostics after a mesh run.
    pub async fn dump_learning_state(&self) {
        let models = self.selector.feasible_models();
        tracing::info!("=== Thompson Sampling State ===");
        for model in &models {
            let stats = self.model_learning.get_category_stats(model.name()).await;
            if stats.is_empty() {
                tracing::info!(model = model.name(), "No learning data");
                continue;
            }
            for (category, alpha, beta, ev, obs) in &stats {
                tracing::info!(
                    model = model.name(),
                    category = category.as_str(),
                    alpha = format!("{alpha:.2}").as_str(),
                    beta = format!("{beta:.2}").as_str(),
                    expected = format!("{ev:.3}").as_str(),
                    observations = obs,
                    "Prior state"
                );
            }
        }
        tracing::info!("=== End Thompson Sampling State ===");
    }

    /// Record that a model is temporarily unavailable (availability failure, not quality).
    ///
    /// This does NOT update the model's Beta prior — availability failures are
    /// operational events, not learning events. Cooldown duration doubles on
    /// each consecutive failure (5 → 10 → 20 → 40 → 60 min cap), then resets
    /// after the first successful response from that model.
    pub async fn record_model_cooldown(&self, model_name: &str) {
        let mut cooldowns = self.model_cooldowns.write().await;
        let consecutive = cooldowns
            .get(model_name)
            .map(|(_, count)| count + 1)
            .unwrap_or(1);
        cooldowns.insert(
            model_name.to_string(),
            (std::time::Instant::now(), consecutive),
        );
        let duration = Self::cooldown_duration_secs(consecutive);
        tracing::info!(
            model = model_name,
            consecutive,
            "Model cooled down for {}s (availability failure)",
            duration
        );
    }

    /// Record a quality-based cooldown (timeout, no tool calls).
    ///
    /// Uses a shorter base (30s) than availability cooldown (5min) to allow
    /// faster model rotation while still breaking retry loops. Progression:
    /// 30s → 60s → 120s → 240s → ... → 3600s cap.
    pub async fn record_quality_cooldown(&self, model_name: &str) {
        let mut cooldowns = self.model_cooldowns.write().await;
        let consecutive = cooldowns
            .get(model_name)
            .map(|(_, count)| count + 1)
            .unwrap_or(1);
        cooldowns.insert(
            model_name.to_string(),
            (std::time::Instant::now(), consecutive),
        );
        let duration = Self::quality_cooldown_duration_secs(consecutive);
        tracing::info!(
            model = model_name,
            consecutive,
            "Model cooled down for {duration}s (quality failure)"
        );
    }

    /// Quality cooldown duration: shorter base (30s) with exponential backoff.
    fn quality_cooldown_duration_secs(consecutive: u32) -> u64 {
        let shift = consecutive.saturating_sub(1).min(10);
        let multiplier = 1u64 << shift;
        MODEL_QUALITY_COOLDOWN_BASE_SECS
            .saturating_mul(multiplier)
            .min(MODEL_COOLDOWN_MAX_SECS)
    }

    /// Clear cooldown for a model after a successful response.
    ///
    /// Resets the exponential backoff so the next failure starts at the base duration.
    async fn clear_model_cooldown(&self, model_name: &str) {
        self.model_cooldowns.write().await.remove(model_name);
    }

    /// Cooldown duration with exponential backoff: base × 2^(n-1), capped.
    fn cooldown_duration_secs(consecutive: u32) -> u64 {
        let shift = consecutive.saturating_sub(1).min(10); // prevent overflow
        let multiplier = 1u64 << shift;
        MODEL_COOLDOWN_BASE_SECS
            .saturating_mul(multiplier)
            .min(MODEL_COOLDOWN_MAX_SECS)
    }

    /// Get model names currently on cooldown (expired entries are pruned)
    pub async fn get_excluded_models(&self) -> Vec<String> {
        let mut cooldowns = self.model_cooldowns.write().await;
        cooldowns.retain(|_, (since, consecutive)| {
            let duration =
                std::time::Duration::from_secs(Self::cooldown_duration_secs(*consecutive));
            since.elapsed() < duration
        });
        let excluded: Vec<String> = cooldowns.keys().cloned().collect();
        if !excluded.is_empty() {
            tracing::info!(
                excluded = %excluded.join(", "),
                "Models on cooldown (excluded from selection)"
            );
        }
        excluded
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

        let excluded = self.get_excluded_models().await;
        let mut decision = self
            .selector
            .select_adaptive(&self.model_learning, &classification, 0.0, &excluded)
            .await?;

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

    /// Route using the fastest available local model, skipping classification.
    ///
    /// Designed for internal ML Brain tasks (episode synthesis, lesson extraction)
    /// that need speed over quality. Uses qwen3.5-0.8b or ministral-3b directly.
    pub async fn route_fast(
        &self,
        task_description: &str,
        messages: Vec<Message>,
    ) -> Result<RouteStream> {
        use crate::stream::{RouteResponse, RouteStream};

        let model = self.selector.fastest_local_model();
        tracing::debug!(model = %model.name(), "Fast-path routing (internal task)");

        let provider = self.instantiate_provider(&model).await?;

        let _permit = self
            .inference_semaphore
            .acquire()
            .await
            .map_err(|_| Error::ModelExecution("Inference semaphore closed".to_string()))?;

        let content = provider
            .complete(messages)
            .await
            .map_err(|e| Error::ModelExecution(format!("{task_description}: {e}")))?;

        let response = RouteResponse {
            content,
            tool_calls: Vec::new(),
            model,
            cost_usd: 0.0,
            used_architect_mode: false,
            architect_savings: None,
        };

        Ok(RouteStream::from_response(response))
    }

    /// Route a chat message using the fastest local model on a separate semaphore.
    ///
    /// Chat inference never blocks task/orchestrator work. Uses the smallest
    /// available model (0.8B/3B) which avoids <think> tag issues from larger
    /// reasoning models. Strips think blocks from the response.
    pub async fn route_chat(&self, messages: Vec<Message>) -> Result<arkavo_llm::ProviderResponse> {
        let model = self.selector.fastest_local_model();
        tracing::debug!(model = %model.name(), "Chat-path routing (separate semaphore)");

        let provider = self.instantiate_provider(&model).await?;

        let _permit = self
            .chat_semaphore
            .acquire()
            .await
            .map_err(|_| Error::ModelExecution("Chat semaphore closed".to_string()))?;

        let content = provider
            .complete(messages)
            .await
            .map_err(|e| Error::ModelExecution(format!("chat: {e}")))?;

        // Strip <think> blocks that small models may still emit
        let content = crate::response::strip_think_blocks(&content);

        Ok(arkavo_llm::ProviderResponse {
            content,
            reasoning_content: None,
            tool_calls: Vec::new(),
            finish_reason: Some("stop".to_string()),
        })
    }

    /// Get the fastest available local model choice (for callers that need to know)
    pub fn fastest_local_model(&self) -> ModelChoice {
        self.selector.fastest_local_model()
    }

    /// Persist validated dynamic adjustments in the background.
    ///
    /// Debounced: only flushes every 10 responses or 60 seconds, whichever
    /// comes first, to avoid chatty SQLite writes on every LLM call.
    #[cfg(feature = "advisor-persistence")]
    fn persist_advisor_state(&self) {
        use std::sync::atomic::Ordering;

        const FLUSH_EVERY_N: u64 = 10;
        const FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

        let count = self.advisor_persist_count.fetch_add(1, Ordering::Relaxed) + 1;
        let timed_out = self
            .advisor_last_persist
            .lock()
            .map(|last| last.elapsed() >= FLUSH_TIMEOUT)
            .unwrap_or(false);

        if count < FLUSH_EVERY_N && !timed_out {
            return;
        }

        self.advisor_persist_count.store(0, Ordering::Relaxed);
        if let Ok(mut last) = self.advisor_last_persist.lock() {
            *last = std::time::Instant::now();
        }

        if let Some(store) = &self.advisor_store {
            let snapshots = self.advisor.export_dynamic();
            let store = store.clone();
            tokio::spawn(async move {
                let to_save: Vec<arkavo_memory::PersistedAdjustment> = snapshots
                    .iter()
                    .filter(|s| s.feedback_count >= 3 && s.success_rate >= 0.5)
                    .map(|s| arkavo_memory::PersistedAdjustment {
                        label: s.label.clone(),
                        model_family: s.model_family.clone(),
                        issue: format!("{:?}", s.issue),
                        text: s.text.clone(),
                        success_rate: s.success_rate,
                        applications: s.applications,
                        feedback_count: s.feedback_count,
                        updated_at: chrono::Utc::now(),
                    })
                    .collect();
                if !to_save.is_empty() {
                    let _ = store.save_batch(&to_save).await;
                }
            });
        }
    }

    pub async fn get_metrics(&self) -> RoutingMetrics {
        self.metrics.read().await.clone()
    }
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
}
