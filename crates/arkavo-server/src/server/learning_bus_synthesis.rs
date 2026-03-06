//! Synthesis and tool pattern methods for LearningBus

use arkavo_router::learning::{Episode, Lesson, ToolCallFormat};

use super::episode_buffer::ToolObservation;
use super::learning_bus::LearningBus;

impl LearningBus {
    /// Set the router for LLM-based synthesis
    pub async fn set_router(&self, router: std::sync::Arc<arkavo_router::Router>) {
        *self.router.write().await = Some(router);
    }

    /// Synthesize an episode from observations using LLM
    pub async fn synthesize_episode(
        &self,
        observations: &[ToolObservation],
        category: &str,
    ) -> Result<Episode, String> {
        let router_guard = self.router.read().await;
        let router = router_guard.as_ref().ok_or("Router not configured")?;
        super::synthesis::synthesize_episode(
            router,
            &self.agent_id,
            &self.swarm_id,
            observations,
            category,
        )
        .await
    }

    /// Synthesize a lesson from episodes using LLM
    pub async fn synthesize_lesson(
        &self,
        episodes: &[Episode],
        category: &str,
    ) -> Result<Option<Lesson>, String> {
        let router_guard = self.router.read().await;
        let router = router_guard.as_ref().ok_or("Router not configured")?;
        super::synthesis::synthesize_lesson(
            router,
            &self.agent_id,
            &self.swarm_id,
            episodes,
            category,
            self.config.min_lesson_confidence,
        )
        .await
    }

    /// Get the tool pattern observer reference
    pub fn tool_pattern_observer(
        &self,
    ) -> &std::sync::Arc<tokio::sync::RwLock<super::tool_pattern_observer::ToolPatternObserver>>
    {
        &self.tool_pattern_observer
    }

    /// Record a successful tool call pattern
    pub async fn record_tool_pattern_success(
        &self,
        tool_name: &str,
        format: ToolCallFormat,
        raw_invocation: &str,
        args: &serde_json::Value,
    ) {
        let mut observer = self.tool_pattern_observer.write().await;
        observer.record_success(tool_name, format, raw_invocation, args);
    }

    /// Add a tool pattern lesson to the policy cache
    pub async fn add_tool_pattern_to_cache(&self, lesson: Lesson) {
        let mut cache = self.policy_cache.write().await;
        cache.add_lesson(lesson);
    }

    /// Get few-shot examples for prompt injection from policy cache
    pub async fn get_few_shot_examples(
        &self,
        tool_names: &[String],
        _format: ToolCallFormat,
    ) -> String {
        let cache = self.policy_cache.read().await;
        cache.get_few_shot_examples(tool_names)
    }

    /// Get behavior guidance for prompt injection from cached lessons
    pub async fn get_behavior_guidance(&self, category: Option<&str>) -> String {
        let cache = self.policy_cache.read().await;
        cache.get_behavior_guidance(category)
    }

    /// Check if there are patterns ready for lesson synthesis
    pub async fn has_ready_patterns(&self) -> bool {
        let observer = self.tool_pattern_observer.read().await;
        observer.has_ready_patterns()
    }

    /// Get the number of cached tool format lessons
    pub async fn cached_tool_pattern_count(&self) -> usize {
        self.policy_cache.read().await.tool_format_lesson_count()
    }

    /// Update the model name for pattern attribution
    pub async fn set_model_name(&self, model_name: String) {
        let mut observer = self.tool_pattern_observer.write().await;
        observer.set_model_name(model_name);
    }

    /// Record a quality score for an (agent, category) pair
    pub async fn record_quality(&self, agent_id: &str, category: &str, score: f64) {
        let mut cache = self.policy_cache.write().await;
        cache.record_quality(agent_id, category, score);
    }

    /// Get all quality trends from the policy cache
    pub async fn get_quality_trends(&self) -> Vec<super::policy_cache::QualityTrend> {
        let cache = self.policy_cache.read().await;
        cache.get_all_trends()
    }

    /// Get behavior lesson count
    pub async fn behavior_lesson_count(&self) -> usize {
        let cache = self.policy_cache.read().await;
        cache.behavior_lesson_count()
    }

    /// Add a lesson to the policy cache
    pub async fn add_lesson_to_cache(&self, lesson: Lesson) {
        let mut cache = self.policy_cache.write().await;
        cache.add_lesson(lesson);
    }

    /// Get count of cached lessons
    pub async fn cached_lesson_count(&self) -> usize {
        self.policy_cache.read().await.len()
    }

    /// Load AGENTS.md content as configuration lessons into the policy cache.
    ///
    /// Called once at startup. Parses constraints (MUST/NEVER/ALWAYS) and
    /// suggestions (bullet points) into canonical lessons.
    pub async fn load_agents_md_lessons(&self, content: &str) {
        let signals = arkavo_router::learning::human_teaching::parse_agents_md(
            content,
            std::path::PathBuf::from("AGENTS.md"),
        );
        if signals.is_empty() {
            return;
        }
        let lessons = arkavo_router::learning::human_teaching::signals_to_lessons(
            &signals,
            &self.agent_id,
            &self.swarm_id,
        );
        let count = lessons.len();
        let mut cache = self.policy_cache.write().await;
        for lesson in lessons {
            cache.add_lesson(lesson);
        }
        tracing::info!(
            "{count} lessons loaded from AGENTS.md ({} constraints, {} suggestions)",
            signals.iter().filter(|s| !s.overridable).count(),
            signals.iter().filter(|s| s.overridable).count(),
        );
    }

    /// Inject a human instruction as a lesson directly into the policy cache.
    ///
    /// Bypasses the normal synthesis pipeline — human lessons are immediately
    /// canonical and immune to decay.
    pub async fn inject_human_instruction(
        &self,
        instruction: &str,
        scope: arkavo_router::learning::LessonScope,
        category: &str,
    ) {
        let lesson = arkavo_router::learning::human_teaching::create_instruction_lesson(
            instruction,
            scope,
            &self.agent_id,
            &self.swarm_id,
            category,
        );
        tracing::info!(
            lesson_id = %lesson.id,
            scope = %lesson.scope,
            "Human instruction injected into policy cache"
        );
        let mut cache = self.policy_cache.write().await;
        cache.add_lesson(lesson);
    }

    /// Record a human correction as an anti-pattern.
    ///
    /// Creates an anti-pattern entry linked to the decision trace so the
    /// routing layer can penalize the model that produced the corrected action.
    pub async fn record_human_correction(
        &self,
        text: &str,
        trace_id: Option<uuid::Uuid>,
        model_name: Option<&str>,
    ) {
        let model = model_name.unwrap_or("unknown");
        let signature = format!("human_correction:{}", &text[..text.len().min(50)]);

        let mut cache = self.policy_cache.write().await;
        cache
            .anti_patterns
            .record_failure(model, "human_correction", &signature, trace_id);
        tracing::info!(
            model = model,
            trace_id = ?trace_id,
            "Human correction recorded as anti-pattern"
        );
    }

    /// Record human reinforcement by boosting quality for the model.
    pub async fn record_human_reinforcement(&self, trace_id: Option<uuid::Uuid>) {
        if trace_id.is_none() {
            return;
        }
        let router_guard = self.router.read().await;
        if let Some(router) = router_guard.as_ref()
            && let Some(model_name) = router.last_routed_model()
        {
            let feedback = arkavo_router::BurstFeedback::success(
                uuid::Uuid::new_v4(),
                "human_reinforcement".to_string(),
                0,
            )
            .with_quality(1.0);
            router
                .model_learning()
                .immediate_update(&model_name, &feedback)
                .await;
            tracing::info!(
                model = %model_name,
                "Human reinforcement: boosted model quality"
            );
        }
    }
}
