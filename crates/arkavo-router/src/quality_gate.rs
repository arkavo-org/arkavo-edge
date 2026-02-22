use crate::error::{Error, Result};
#[cfg(feature = "llama-cpp")]
use crate::judge;
use crate::learning::BurstFeedback;
use crate::{classifier, prompt_advisor, selector_quality, tool_extraction, validator};
use arkavo_llm::{Message, ProviderResponse, Role};
use arkavo_mcp_tools::ToolRegistry;

impl super::Router {
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
        let mut feedback_messages: Vec<Message> = Vec::new();

        let input_tokens = tool_extraction::estimate_tokens(task_description);
        let is_simple = prompt_advisor::is_simple_query(&task_description.to_lowercase());

        for attempt in 0..MAX_RETRIES {
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

                    Some(arkavo_llm::McpConverter::to_anthropic_format_minimal(
                        &tool_infos,
                    ))
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
                msgs.extend(feedback_messages.clone());
                (msgs, Some(advice.applied_labels))
            } else {
                let mut msgs = messages.clone();
                msgs.extend(feedback_messages.clone());
                (msgs, None)
            };

            // TDF audit: encrypt cloud-bound messages for local audit trail
            #[cfg(feature = "tdf-encrypt")]
            if current_decision.recommended_model.is_cloud()
                && let Some(ref encryptor) = self.tdf_encryptor
            {
                let manifests = encryptor.encrypt_messages(&advised_messages).await;
                if !manifests.is_empty() {
                    let total_bytes: usize =
                        manifests.iter().map(|(_, m)| m.payload.value.len()).sum();
                    tracing::info!(
                        "TDF audit: encrypted {} cloud-bound messages ({total_bytes} bytes ciphertext)",
                        manifests.len(),
                    );

                    if let Some(ref store) = self.tdf_audit_store {
                        let model_name = current_decision.recommended_model.name().to_string();
                        let agent_id = encryptor.agent_id().to_string();
                        let records: Vec<arkavo_memory::AuditRecord> = manifests
                            .iter()
                            .map(|(idx, m)| arkavo_memory::AuditRecord {
                                session_id: String::new(),
                                message_index: *idx,
                                agent_id: agent_id.clone(),
                                model: model_name.clone(),
                                algorithm: m.encryption_information.method.algorithm.clone(),
                                ciphertext_bytes: m.payload.value.len(),
                                policy_attributes: m
                                    .encryption_information
                                    .key_access
                                    .iter()
                                    .map(|ka| ka.url.clone())
                                    .collect(),
                                created_at: chrono::Utc::now(),
                            })
                            .collect();
                        let store = store.clone();
                        tokio::spawn(async move {
                            if let Err(e) = store.save_batch(&records).await {
                                tracing::warn!("TDF audit persist failed: {e}");
                            }
                        });
                    }
                }
            }

            let provider = self
                .instantiate_provider(&current_decision.recommended_model)
                .await?;

            // Acquire inference semaphore — serializes concurrent LLM calls
            // so the second request waits instead of failing with a KV cache OOM.
            let _permit = self
                .inference_semaphore
                .acquire()
                .await
                .map_err(|_| Error::ModelExecution("Inference semaphore closed".to_string()))?;
            tracing::debug!("Inference semaphore acquired");

            let mut response = match provider
                .complete_with_tools(advised_messages, tools_json, None)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    self.record_model_cooldown(current_decision.recommended_model.name())
                        .await;

                    if attempt + 1 < MAX_RETRIES {
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

            response.tool_calls =
                tool_extraction::filter_and_extract_tool_calls(response.tool_calls);

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

            if !response.tool_calls.is_empty() {
                for tc in &response.tool_calls {
                    tracing::debug!("[Judge] Tool call: {} args={}", tc.tool_name, tc.arguments);
                }
            }

            if let Some(registry) = tool_registry {
                let tool_infos = registry.list_tools();

                let validator = validator::ResponseValidator::new(&tool_infos);
                if let Err(validation_error) = validator.quick_validate(&response) {
                    tracing::warn!(
                        "Fast validation failed on attempt {}/{}: {}",
                        attempt + 1,
                        MAX_RETRIES,
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

                    if attempt + 1 < MAX_RETRIES {
                        // RL FEEDBACK: Inject validation error back into conversation
                        // so the model can learn the correct format and retry
                        let feedback_msg = Message {
                            role: Role::User,
                            content: format!(
                                "ERROR: {validation_error}\n\nYou MUST include ALL required parameters inside the fence. Example:\n```tool_name\nparam1: value1\nparam2: value2\n```\nTry again with the correct parameters.",
                            ),
                            images: None,
                        };
                        feedback_messages.push(feedback_msg);
                        tracing::info!(
                            "RL feedback: injecting validation error for retry (attempt {})",
                            attempt + 1
                        );
                        continue;
                    }
                    tracing::warn!(
                        "Validation failed after {} attempts, returning response",
                        MAX_RETRIES
                    );
                    return Ok(response);
                }

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

                                if judgment.issue_type == IssueType::MissingToolUse
                                    && !judgment.suggested_keywords.is_empty()
                                {
                                    tracing::warn!(
                                        "Judge suggested tools {:?} but available tools may differ — continuing with response",
                                        judgment.suggested_keywords
                                    );
                                }

                                if attempt + 1 < MAX_RETRIES {
                                    // RL FEEDBACK: Inject judge rejection reason back into
                                    // conversation so the model can learn from the feedback
                                    let reason = judgment
                                        .reason
                                        .as_deref()
                                        .unwrap_or("Quality check failed");
                                    let feedback_msg = Message {
                                        role: Role::User,
                                        content: format!(
                                            "ERROR: Your response was rejected: {reason}\n\nPlease fix the issue and try again. Use the correct tool call format.",
                                        ),
                                        images: None,
                                    };
                                    feedback_messages.push(feedback_msg);
                                    tracing::info!(
                                        "RL feedback: injecting judge rejection for retry (attempt {})",
                                        attempt + 1
                                    );
                                    continue;
                                }
                                tracing::warn!(
                                    "Judge rejected after {} attempts, returning response",
                                    MAX_RETRIES
                                );
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

            // Record which model was selected so the conductor can attribute
            // reward-based corrective feedback to the right Thompson Sampling prior.
            if let Ok(mut guard) = self.last_routed_model.write() {
                *guard = Some(current_decision.recommended_model.name().to_string());
            }

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
}
