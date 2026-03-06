use crate::error::{Error, Result};
#[cfg(feature = "llama-cpp")]
use crate::judge;
use crate::learning::BurstFeedback;
use crate::{classifier, prompt_advisor, selector_quality, tool_extraction, validator};
use arkavo_llm::{Message, ProviderResponse, StreamResponse};
use arkavo_mcp_tools::ToolRegistry;
use futures::StreamExt;
use tokio_stream::Stream;

/// Default buffer size for streaming channels
const STREAM_BUFFER_SIZE: usize = 100;

impl super::Router {
    /// Route with automatic quality evaluation and model escalation
    #[deprecated(since = "0.44.0", note = "Use route() instead")]
    pub async fn route_with_quality_gate(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        max_retries: u8,
    ) -> Result<ProviderResponse> {
        let decision = self.classify(task_description).await?;
        self.route_with_quality_gate_from(
            decision,
            task_description,
            messages,
            tool_registry,
            max_retries,
        )
        .await
    }

    /// Route with quality gate using a pre-computed routing decision.
    pub async fn route_with_quality_gate_from(
        &self,
        decision: crate::RoutingDecision,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        max_retries: u8,
    ) -> Result<ProviderResponse> {
        let mut current_decision = decision;

        let input_tokens = tool_extraction::estimate_tokens(task_description);
        let is_simple = prompt_advisor::is_simple_query(&task_description.to_lowercase());

        for attempt in 0..max_retries {
            let inference_start = std::time::Instant::now();

            let tools_json = match tool_registry {
                Some(registry) => {
                    let detail_level = tool_extraction::detail_level_for_model(
                        &current_decision.recommended_model,
                    );
                    let keywords = tool_extraction::extract_keywords(task_description);

                    let tool_infos = tool_extraction::search_tools_hybrid(
                        registry,
                        &keywords,
                        detail_level,
                        Some(input_tokens),
                    )
                    .await;

                    let json = match current_decision.recommended_model {
                        crate::decision::ModelChoice::GeminiFlash
                        | crate::decision::ModelChoice::GeminiPro => {
                            arkavo_llm::McpConverter::to_gemini_format_minimal(&tool_infos)
                        }
                        _ => arkavo_llm::McpConverter::to_anthropic_format_minimal(&tool_infos),
                    };
                    Some(json)
                }
                None => None,
            };

            let (advised_messages, advice_labels) = if let Some(advice) = self
                .advisor
                .advise(current_decision.recommended_model.family(), is_simple)
            {
                tracing::debug!(
                    adjustments = ?advice.applied_labels,
                    "Prompt advisor: {} adjustments for {}",
                    advice.applied_labels.len(),
                    current_decision.recommended_model.family()
                );
                let mut msgs = vec![Message::system(advice.system_text)];
                msgs.extend(messages.clone());
                (msgs, Some(advice.applied_labels))
            } else {
                (messages.clone(), None)
            };

            let provider = self
                .instantiate_provider(&current_decision.recommended_model)
                .await?;

            let mut response = match provider
                .complete_with_tools(advised_messages, tools_json.clone(), None)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    self.record_model_cooldown(current_decision.recommended_model.name())
                        .await;

                    if attempt + 1 < max_retries {
                        let excluded = self.get_excluded_models().await;
                        let re_class = classifier::Classification::new(
                            current_decision.task_category,
                            current_decision.confidence,
                            "Re-routed after availability failure".to_string(),
                        );
                        current_decision = self
                            .selector
                            .select_adaptive(&self.model_learning, &re_class, 0.0, &excluded)
                            .await?;
                        tracing::info!(
                            model = %current_decision.recommended_model.name(),
                            "Re-routed after availability failure: {e}"
                        );
                        continue;
                    }
                    return Err(Error::ModelExecution(format!("Provider error: {e}")));
                }
            };

            self.advisor.observe(
                current_decision.recommended_model.family(),
                task_description,
                &response.content,
            );
            #[cfg(feature = "advisor-persistence")]
            self.persist_advisor_state();

            if response.tool_calls.is_empty() && !response.content.is_empty() {
                let extracted = tool_extraction::extract_tool_calls_from_text(&response.content);
                if !extracted.is_empty() {
                    tracing::debug!(
                        "Extracted {} tool calls from text response",
                        extracted.len()
                    );
                    response.tool_calls = extracted;
                }
            }

            if response.tool_calls.len() > 1 {
                let original_count = response.tool_calls.len();
                let mut seen = std::collections::HashSet::new();
                response.tool_calls.retain(|tc| {
                    let key = format!("{}:{}", tc.tool_name, tc.arguments);
                    seen.insert(key)
                });
                if response.tool_calls.len() < original_count {
                    tracing::debug!(
                        "Deduplicated tool calls: {} -> {}",
                        original_count,
                        response.tool_calls.len()
                    );
                }
            }

            if !response.tool_calls.is_empty() {
                let display_count = response.tool_calls.len().min(5);
                for tc in response.tool_calls.iter().take(display_count) {
                    tracing::debug!("[LLM] Tool call: {} args={}", tc.tool_name, tc.arguments);
                }
                if response.tool_calls.len() > 5 {
                    tracing::debug!(
                        "[LLM] ... and {} more tool calls",
                        response.tool_calls.len() - 5
                    );
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

                    if let Some(ref labels) = advice_labels {
                        self.advisor.record_feedback(labels, false);
                    }

                    let elapsed = inference_start.elapsed();
                    tracing::info!(
                        model = %current_decision.recommended_model.name(),
                        category = current_decision.task_category.as_str(),
                        latency_ms = elapsed.as_millis() as u64,
                        "Quality feedback recorded (negative — validation failure)"
                    );
                    self.model_learning
                        .immediate_update(
                            current_decision.recommended_model.name(),
                            &BurstFeedback::failure(
                                uuid::Uuid::new_v4(),
                                current_decision.task_category.as_str().to_string(),
                                elapsed.as_millis() as u64,
                            ),
                        )
                        .await;

                    if attempt + 1 < max_retries {
                        current_decision.recommended_model =
                            self.upgrade_model(&current_decision.recommended_model);
                        tracing::info!(
                            "Upgrading to {:?} due to validation failure",
                            current_decision.recommended_model
                        );
                        continue;
                    }
                    tracing::warn!(
                        "Validation failed after max retries, stripping invalid tool calls and continuing"
                    );
                    response.tool_calls.clear();
                    return Ok(response);
                }

                #[cfg(feature = "llama-cpp")]
                {
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

                                if let Some(ref labels) = advice_labels {
                                    self.advisor.record_feedback(labels, false);
                                }

                                let elapsed = inference_start.elapsed();
                                tracing::info!(
                                    model = %current_decision.recommended_model.name(),
                                    category = current_decision.task_category.as_str(),
                                    latency_ms = elapsed.as_millis() as u64,
                                    "Quality feedback recorded (negative — judge rejection)"
                                );
                                self.model_learning
                                    .immediate_update(
                                        current_decision.recommended_model.name(),
                                        &BurstFeedback::failure(
                                            uuid::Uuid::new_v4(),
                                            current_decision.task_category.as_str().to_string(),
                                            elapsed.as_millis() as u64,
                                        ),
                                    )
                                    .await;

                                if judgment.issue_type == crate::judge::IssueType::MissingToolUse
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

                                if attempt + 1 < max_retries {
                                    current_decision.recommended_model =
                                        self.upgrade_model(&current_decision.recommended_model);
                                    tracing::info!(
                                        "Upgrading to {:?} after judge rejection",
                                        current_decision.recommended_model
                                    );
                                    continue;
                                }
                                tracing::warn!(
                                    "Judge rejected after max retries, continuing with response: {}",
                                    judgment.reason.as_deref().unwrap_or("unspecified reason")
                                );
                                response.tool_calls.clear();
                                return Ok(response);
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Judge validation skipped (model unavailable): {}", e);
                        }
                    }
                }
            }

            if let Some(ref labels) = advice_labels {
                self.advisor.record_feedback(labels, true);
            }

            self.clear_model_cooldown(current_decision.recommended_model.name())
                .await;

            let elapsed = inference_start.elapsed();
            let quality = selector_quality::compute_response_quality(
                &response.content,
                elapsed.as_millis() as u64,
                current_decision.task_category.as_str(),
            );
            tracing::info!(
                model = %current_decision.recommended_model.name(),
                category = current_decision.task_category.as_str(),
                quality = format!("{quality:.3}").as_str(),
                latency_ms = elapsed.as_millis() as u64,
                response_len = response.content.len(),
                "Quality feedback recorded (positive)"
            );
            self.model_learning
                .immediate_update(
                    current_decision.recommended_model.name(),
                    &BurstFeedback::success(
                        uuid::Uuid::new_v4(),
                        current_decision.task_category.as_str().to_string(),
                        elapsed.as_millis() as u64,
                    )
                    .with_quality(quality)
                    .with_usage(current_decision.estimated_cost_usd, 0),
                )
                .await;

            return Ok(response);
        }

        tracing::warn!("Route loop completed without returning, using empty response");
        Ok(ProviderResponse {
            content: String::new(),
            reasoning_content: None,
            tool_calls: Vec::new(),
            finish_reason: None,
        })
    }

    /// Route with streaming and async quality validation
    #[deprecated(since = "0.44.0", note = "Use route() instead for streaming")]
    pub async fn route_with_quality_gate_stream(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        _max_retries: u8,
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

            if let Some(tools) = tool_infos {
                let response = ProviderResponse {
                    content: accumulated.clone(),
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    finish_reason: Some("stop".to_string()),
                };

                let validator = validator::ResponseValidator::new(&tools);
                if let Err(e) = validator.quick_validate(&response) {
                    tracing::warn!(
                        target: "arkavo_router::quality_gate",
                        model = ?model,
                        error = %e,
                        "Async validation failed - consider metrics/alerting"
                    );
                }

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
    #[deprecated(
        since = "0.44.0",
        note = "Use route() instead - it auto-detects architect mode"
    )]
    #[allow(deprecated)]
    pub async fn route_architect(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<crate::architect::ArchitectResult> {
        use crate::architect::{ArchitectExecutor, ArchitectPlanner, ComplexityScorer};

        let scorer = ComplexityScorer::new();
        let complexity = scorer.analyze(task_description);

        if !complexity.architect_recommended {
            tracing::debug!(
                "Architect mode not recommended: {} subtasks, {} tokens",
                complexity.estimated_subtasks,
                complexity.estimated_output_tokens
            );

            let response = self
                .route_with_quality_gate(task_description, messages, tool_registry, 3)
                .await?;

            return Ok(crate::architect::ArchitectResult::single_task(
                response.content,
                0.0,
            ));
        }

        tracing::info!(
            "Architect mode activated: {} estimated subtasks, {:.1}% estimated savings",
            complexity.estimated_subtasks,
            complexity.estimated_savings_percent
        );

        let planner = ArchitectPlanner::new();
        let plan = planner.create_plan(task_description, complexity).await?;

        tracing::info!(
            "Created plan with {} subtasks, estimated ${:.4} (saves ${:.4})",
            plan.subtasks.len(),
            plan.architect_estimate_usd,
            plan.opus_only_estimate_usd - plan.architect_estimate_usd
        );

        let executor =
            ArchitectExecutor::new(std::sync::Arc::new(self.clone_for_executor().await?));
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
    pub(crate) async fn clone_for_executor(&self) -> Result<Self> {
        Ok(Self {
            classifier: self.classifier.clone(),
            selector: self.selector.clone(),
            model_learning: self.model_learning.clone(),
            #[cfg(feature = "llama-cpp")]
            model_registry: self.model_registry.clone(),
            model_cooldowns: self.model_cooldowns.clone(),
            metrics: self.metrics.clone(),
            connectivity: self.connectivity.clone(),
            offline_mode: self.offline_mode,
            preflight: self.preflight.clone(),
            advisor: self.advisor.clone(),
            #[cfg(feature = "critic")]
            critic: self.critic.clone(),
            #[cfg(feature = "advisor-persistence")]
            advisor_store: self.advisor_store.clone(),
            #[cfg(feature = "advisor-persistence")]
            advisor_persist_count: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "advisor-persistence")]
            advisor_last_persist: std::sync::Mutex::new(std::time::Instant::now()),
            #[cfg(feature = "tdf-encrypt")]
            tdf_encryptor: self.tdf_encryptor.clone(),
            #[cfg(feature = "tdf-encrypt")]
            tdf_audit_store: self.tdf_audit_store.clone(),
            inference_semaphore: self.inference_semaphore.clone(),
            chat_semaphore: self.chat_semaphore.clone(),
            last_routed_model: self.last_routed_model.clone(),
        })
    }
}
