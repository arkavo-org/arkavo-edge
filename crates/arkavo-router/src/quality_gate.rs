use crate::error::{Error, Result};
#[cfg(feature = "llama-cpp")]
use crate::judge;
use crate::learning::BurstFeedback;
use crate::usage::{CallBudget, RoutedResponse};
use crate::{classifier, prompt_advisor, selector_quality, tool_extraction, validator};
use arkavo_llm::{Message, ProviderResponse};
use arkavo_mcp_tools::ToolRegistry;

impl super::Router {
    /// Route with tools and Judge loop validation (local models only).
    pub async fn route_with_tools(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<ProviderResponse> {
        self.route_with_tools_hinted(task_description, messages, tool_registry, None)
            .await
    }

    /// Route with a model hint from AGENTS.md configuration.
    ///
    /// If the hinted model is available, it biases the initial Thompson Sampling
    /// selection. Escalation and quality gates still apply if inference fails.
    pub async fn route_with_tools_hinted(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        model_hint: Option<&crate::ModelChoice>,
    ) -> Result<ProviderResponse> {
        self.route_with_tools_internal(
            task_description,
            messages,
            tool_registry,
            model_hint,
            false,
            None,
        )
        .await
        .map(|r| r.response)
    }

    /// Attribute every completed attempt, including responses rejected by a retry gate.
    /// Spending is recorded inside the loop, so a later error cannot erase earlier usage.
    pub async fn route_with_tools_budgeted(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        budget: CallBudget<'_>,
    ) -> Result<RoutedResponse> {
        self.route_with_tools_internal(
            task_description,
            messages,
            tool_registry,
            None,
            false,
            Some(budget),
        )
        .await
    }

    pub async fn route_with_tools_attributed(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<RoutedResponse> {
        self.route_with_tools_internal(task_description, messages, tool_registry, None, false, None)
            .await
    }

    async fn route_with_tools_internal(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        model_hint: Option<&crate::ModelChoice>,
        execution_mode: bool,
        budget: Option<CallBudget<'_>>,
    ) -> Result<RoutedResponse> {
        const MAX_RETRIES: u8 = 3;
        let budget = budget.or_else(|| self.call_budget());
        let mut current_decision = self.classify(task_description).await?;

        // Execution iterations: when a model hint is provided (from AGENTS.md),
        // use it with execution-mode sampling (temp 0.1, thinking off, max 200 tokens).
        // Without a hint, fall back to the fastest local model for mechanical tool calls.
        let fast_model = self.selector.fastest_local_model();
        let effective_hint =
            if execution_mode && model_hint.is_none() && self.is_model_available(&fast_model) {
                tracing::debug!(
                    fast_model = fast_model.name(),
                    "Execution mode: using fastest local model (no hint)"
                );
                current_decision.recommended_model = fast_model;
                None
            } else {
                model_hint
            };

        if let Some(hint) = effective_hint {
            let consecutive = self.get_cooldown_consecutive(hint.name()).await;
            let reward_failures = self.get_reward_failure_count(hint.name()).await;
            // Hinted models get 3 chances to learn from feedback before
            // Thompson Sampling takes over. Tracks both availability failures
            // (timeouts, crashes) and quality failures (sustained negative rewards).
            const HINT_OVERRIDE_THRESHOLD: u32 = 3;
            if consecutive >= HINT_OVERRIDE_THRESHOLD || reward_failures >= HINT_OVERRIDE_THRESHOLD
            {
                tracing::info!(
                    hint = hint.name(),
                    consecutive,
                    reward_failures,
                    selected = current_decision.recommended_model.name(),
                    "Model hint overridden (cooldown={consecutive}, reward_fail={reward_failures}), Thompson Sampling selecting"
                );
            } else if self.is_model_available(hint) {
                tracing::info!(
                    hint = hint.name(),
                    original = current_decision.recommended_model.name(),
                    "Applying model hint from AGENTS.md"
                );
                current_decision.recommended_model = hint.clone();
            } else {
                tracing::debug!(
                    hint = hint.name(),
                    "Model hint not available, using default"
                );
            }
        }

        let mut feedback_messages: Vec<Message> = Vec::new();
        let mut attempts = Vec::new();
        // Cloud arm already authorized for this dispatch. The user's one-shot
        // confirmation is consumed once — by the collapse upgrade or by the
        // first attempt — so retries against that same model must not re-ask,
        // while a switch to a different cloud arm still does.
        let mut authorized_cloud: Option<crate::ModelChoice> = None;
        // Set only when a *user* approval (one-shot or session) paid for this
        // dispatch. A caller naming a model authorizes that model, not a later
        // upgrade to a different, possibly dearer arm — so it must not count.
        let mut user_paid_for_cloud = false;

        let input_tokens = tool_extraction::estimate_tokens(task_description);
        let is_simple = prompt_advisor::is_simple_query(&task_description.to_lowercase());

        for attempt in 0..MAX_RETRIES {
            let inference_start = std::time::Instant::now();

            // Track whether tools were actually attached (non-empty) so the
            // quality scorer below can penalize text-only responses correctly.
            // A registry that returns zero matches must NOT count as attached.
            let (tools_json, tools_were_attached) = match tool_registry {
                Some(registry) => {
                    // Execution mode uses NameAndDescription to keep Jinja template
                    // expansion compact — the model already saw full schemas in round 0.
                    let detail_level = if execution_mode {
                        arkavo_mcp_tools::DetailLevel::NameAndDescription
                    } else {
                        tool_extraction::detail_level_for_model(&current_decision.recommended_model)
                    };
                    let keywords = tool_extraction::extract_keywords(task_description);

                    let tool_infos = tool_extraction::search_tools_hybrid(
                        registry,
                        &keywords,
                        detail_level,
                        Some(input_tokens),
                    )
                    .await;

                    let attached = !tool_infos.is_empty();
                    let json = match current_decision.recommended_model {
                        crate::ModelChoice::GeminiFlash
                        | crate::ModelChoice::Gemini35Flash
                        | crate::ModelChoice::Gemini35FlashMinimal
                        | crate::ModelChoice::Gemini35FlashMedium
                        | crate::ModelChoice::Gemini35FlashHigh
                        | crate::ModelChoice::GeminiPro => {
                            arkavo_llm::McpConverter::to_gemini_format_minimal(&tool_infos)
                        }
                        _ => arkavo_llm::McpConverter::to_anthropic_format_minimal(&tool_infos),
                    };
                    (Some(json), attached)
                }
                None => (None, false),
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
                let mut msgs = messages.clone();
                // Merge advisor system text into existing system message to
                // avoid duplicate system messages (Qwen Jinja enforces single
                // system message at position 0).
                if let Some(first) = msgs.first_mut() {
                    if first.role == arkavo_llm::Role::System {
                        first.content = format!("{}\n\n{}", advice.system_text, first.content);
                    } else {
                        msgs.insert(0, Message::system(advice.system_text));
                    }
                } else {
                    msgs.push(Message::system(advice.system_text));
                }
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

            let actual_model = current_decision.recommended_model.clone();
            let estimated_usage = crate::usage::estimate_request(
                &advised_messages,
                tools_json.as_ref(),
                if execution_mode { 200 } else { 4096 },
            );
            let estimated_cost = self.usage_cost(&actual_model, &estimated_usage);
            // Cloud-spend policy gates the tool-loop exactly as it gates chat,
            // and before the provider is built so a denial never opens a client.
            // "Explicit" means the caller named this model (an applied hint) or
            // it was already authorized for this dispatch.
            let caller_authorized = authorized_cloud.as_ref() == Some(&actual_model)
                || effective_hint.is_some_and(|hint| *hint == actual_model);
            if let Some(budget) = budget {
                budget.check(estimated_cost).await?;
            }
            let approval_pending = self.cloud_confirmation_pending();
            self.authorize_call(&actual_model, estimated_cost, caller_authorized)
                .await?;
            if actual_model.is_cloud() {
                authorized_cloud = Some(actual_model.clone());
                // A cloud arm the caller did not name can only have cleared the
                // gate on the user's approval, which `authorize_call` has now
                // spent.
                user_paid_for_cloud |= !caller_authorized && approval_pending;
            }
            let provider = if execution_mode {
                self.instantiate_provider_execution(&actual_model).await?
            } else {
                self.instantiate_provider_exact_with_spec(
                    &actual_model,
                    current_decision.use_spec_decoding,
                )
                .await?
            };

            // Execution iterations use the chat semaphore — they're fast, sub-second
            // inferences that shouldn't queue behind heavy planning work.
            let semaphore = if execution_mode {
                &self.chat_semaphore
            } else {
                &self.inference_semaphore
            };
            let _permit = semaphore
                .acquire()
                .await
                .map_err(|_| Error::ModelExecution("Semaphore closed".to_string()))?;
            tracing::debug!(
                execution_mode,
                semaphore = if execution_mode { "chat" } else { "inference" },
                "Semaphore acquired"
            );

            // Feasibility plane (pre-dispatch): assess whether the local model
            // can run this prompt now, surfacing a reshape/unavailable signal
            // before we spend an inference on a doomed call. Local-only; never
            // spends.
            self.check_local_feasibility(&current_decision.recommended_model, input_tokens as u32);

            let max_tokens = if execution_mode { Some(200usize) } else { None };
            let request_usage =
                crate::usage::estimate_request(&advised_messages, tools_json.as_ref(), 0);
            let mut response = match provider
                .complete_with_tools(advised_messages, tools_json, max_tokens)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    if let Some(timing) = e.inference_timing() {
                        let failed_response = ProviderResponse {
                            inference_timing: Some(timing.clone()),
                            ..Default::default()
                        };
                        let attributed = self.attribute_response(
                            actual_model.clone(),
                            &request_usage,
                            &failed_response,
                        );
                        if let Some(budget) = budget {
                            budget.record(&attributed).await?;
                        }
                        attempts.push(attributed);
                    }
                    self.record_model_cooldown(current_decision.recommended_model.name())
                        .await;

                    if attempt + 1 < MAX_RETRIES {
                        // Feasibility plane: a provider error (timeout / OOM /
                        // crash) is an *availability* failure, not a quality
                        // one. Per the plane separation it may not silently
                        // cross into paid cloud — `reroute_exclusions` drops the
                        // cloud arms unless the cloud policy authorizes silent
                        // spend, so the retry stays local under the default
                        // `AskBeforeCloud` posture.
                        let excluded = self.reroute_exclusions().await;
                        let re_class = classifier::Classification::new(
                            current_decision.task_category,
                            current_decision.confidence,
                            "Re-routed after availability failure".to_string(),
                        );
                        match self
                            .selector
                            .select_adaptive(&self.model_learning, &re_class, 0.0, &excluded)
                            .await
                        {
                            Ok(next) => {
                                current_decision = next;
                                tracing::info!(
                                    model = %current_decision.recommended_model.name(),
                                    cloud_policy = ?self.cloud_policy(),
                                    stayed_local = current_decision.recommended_model.is_local(),
                                    "Re-routed after availability failure: {e}"
                                );
                                continue;
                            }
                            Err(reroute_err) => {
                                // No local model could be selected and the cloud
                                // policy bars a silent paid fallback. Surface the
                                // quality→spend boundary so callers can tell
                                // "local unavailable, cloud blocked by policy"
                                // from a generic provider failure, then propagate
                                // the more specific re-route error.
                                self.emit_event(crate::RouterEvent::CloudEscalationBlocked {
                                    reason: format!("availability:{e}"),
                                    policy: format!("{:?}", self.cloud_policy()),
                                });
                                return Err(reroute_err);
                            }
                        }
                    }
                    return Err(Error::ModelExecution(format!("Provider error: {e}")));
                }
            };

            let attributed =
                self.attribute_response(actual_model.clone(), &request_usage, &response);
            if let Some(budget) = budget {
                budget.record(&attributed).await?;
            }
            attempts.push(attributed);

            // Record per-attempt inference latency so retries are individually visible
            let attempt_ms = inference_start.elapsed().as_millis() as u64;
            if attempt > 0 {
                tracing::info!(
                    attempt = attempt + 1,
                    attempt_ms,
                    execution_mode,
                    model = %current_decision.recommended_model.name(),
                    "Quality gate retry inference completed"
                );
            }

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
                        // RL FEEDBACK: Inject specific validation error with actionable fix.
                        // Insert assistant→user pair to maintain role alternation
                        // required by Jinja chat templates (Qwen3.5, Ministral).
                        let available_tool_names: Vec<&str> =
                            tool_infos.iter().map(|t| t.name.as_str()).collect();
                        let fix = validation_error.fix_suggestion(&available_tool_names);
                        append_rejected_response(&mut feedback_messages, &response);
                        feedback_messages.push(Message::user(format!(
                            "ERROR: {validation_error}\n\nFix: {fix}",
                        )));
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
                    return Ok(RoutedResponse {
                        response,
                        model: actual_model,
                        attempts,
                    });
                }

                // Skip Judge validation in execution mode — fast syntax check is sufficient
                // for mechanical tool calls (send_task, list_agents, get_task_status).
                #[cfg(feature = "llama-cpp")]
                if !execution_mode {
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
                                    return Err(Error::MissingToolUse {
                                        keywords: judgment.suggested_keywords.clone(),
                                    });
                                }

                                if attempt + 1 < MAX_RETRIES {
                                    // RL FEEDBACK: Inject judge rejection reason back into
                                    // conversation. Insert assistant→user pair for Jinja
                                    // role alternation compliance.
                                    let reason = judgment
                                        .reason
                                        .as_deref()
                                        .unwrap_or("Quality check failed");
                                    append_rejected_response(&mut feedback_messages, &response);
                                    feedback_messages.push(Message::user(format!(
                                        "ERROR: Your response was rejected: {reason}\n\nPlease fix the issue and try again. Use the correct tool call format.",
                                    )));
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
                                return Ok(RoutedResponse {
                                    response,
                                    model: actual_model,
                                    attempts,
                                });
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Judge validation skipped (model unavailable): {}", e);
                        }
                    }
                }
            }

            // Collapse plane (adequacy v1): catch visible breakdowns the
            // validator and Judge miss — empty output and repetition loops on a
            // final-answer turn. A collapse may trigger a retry/offer, but per
            // the plane separation it never silently spends: cloud becomes a
            // retry candidate only when the policy authorizes silent spend.
            if !execution_mode && response.tool_calls.is_empty() && attempt + 1 < MAX_RETRIES {
                use crate::planes::{self, CollapseSignal, CollapseVerdict, UpgradeOffer};
                let collapse = planes::detect_collapse(&planes::AnswerObservation {
                    text: &response.content,
                    hit_output_cap: response.finish_reason.as_deref() == Some("length"),
                    tool_call_required: false,
                    precomputed: None,
                    avg_logprob: response
                        .inference_timing
                        .as_ref()
                        .and_then(|t| t.avg_logprob),
                });
                // Retry/offer on breakdowns where a fresh attempt or a stronger
                // model helps: empty/repetition (local re-roll) and low token
                // confidence (the adequacy signal — a stronger model may be
                // surer). A truncated-but-coherent long answer is not re-rolled.
                if let CollapseVerdict::Collapsed(
                    signal @ (CollapseSignal::EmptyOutput
                    | CollapseSignal::RepetitionLoop
                    | CollapseSignal::LowConfidence),
                ) = collapse
                {
                    let offer = planes::upgrade_offer(
                        self.cloud_policy(),
                        &collapse,
                        planes::UpgradeContext::default(),
                    );
                    // Spend plane: a collapse only *requests* cloud. Authorize
                    // it through the budget plane — policy AND the live remaining
                    // cap — never on the quality signal alone. A one-shot user
                    // confirmation (set via confirm_next_cloud_upgrade after a
                    // CloudUpgradeOffered) satisfies AskBeforeCloud; otherwise the
                    // decision tells us whether to offer (ask) or refuse.
                    let allow_cloud = if let UpgradeOffer::Offer(reason) = offer {
                        let caps = self.cloud_spend_caps().await;
                        let projected = self.projected_cloud_cost(&current_decision);
                        // A user approval spent by this dispatch's own first
                        // attempt still counts: the one-shot flag is gone, but
                        // asking the same user twice for one request is the bug
                        // the session-sticky flag exists to avoid. An explicit
                        // caller model is deliberately not enough — it approves
                        // that arm, not an upgrade to a different one.
                        let confirmed = user_paid_for_cloud || self.cloud_confirmed();
                        match planes::authorize_upgrade(
                            self.cloud_policy(),
                            reason,
                            projected,
                            caps,
                            confirmed,
                        ) {
                            arkavo_budget::CloudSpendDecision::Authorized { .. } => true,
                            arkavo_budget::CloudSpendDecision::NeedsUserConfirmation {
                                projected_cost,
                            } => {
                                // Policy permits cloud but needs the user's OK:
                                // surface the offer and stay local this turn.
                                self.emit_event(crate::RouterEvent::CloudUpgradeOffered {
                                    reason: format!("{reason:?}"),
                                    projected_cost_cents: projected_cost.as_cents(),
                                });
                                false
                            }
                            arkavo_budget::CloudSpendDecision::Denied(_) => {
                                self.emit_event(crate::RouterEvent::CloudEscalationBlocked {
                                    reason: format!("collapse:{signal:?}"),
                                    policy: format!("{:?}", self.cloud_policy()),
                                });
                                false
                            }
                        }
                    } else {
                        false
                    };
                    let mut excluded = if allow_cloud {
                        self.get_excluded_models().await
                    } else {
                        self.reroute_exclusions().await
                    };
                    // Exclude the model that just collapsed so re-selection
                    // actually rotates the arm instead of reproducing the same
                    // collapse and burning a retry.
                    let collapsed_name = current_decision.recommended_model.name().to_string();
                    if !excluded.iter().any(|e| e == &collapsed_name) {
                        excluded.push(collapsed_name);
                    }
                    let re_class = classifier::Classification::new(
                        current_decision.task_category,
                        current_decision.confidence,
                        format!("Re-routed after local collapse ({signal:?})"),
                    );
                    if let Ok(next) = self
                        .selector
                        .select_adaptive(&self.model_learning, &re_class, 0.0, &excluded)
                        .await
                    {
                        // Steer Thompson Sampling away from the collapsing
                        // model before rotating off it, mirroring the
                        // Judge-rejection path — otherwise the retry doesn't
                        // learn from the collapse.
                        self.model_learning
                            .immediate_update(
                                current_decision.recommended_model.name(),
                                &BurstFeedback::failure(
                                    uuid::Uuid::new_v4(),
                                    current_decision.task_category.as_str().to_string(),
                                    inference_start.elapsed().as_millis() as u64,
                                ),
                            )
                            .await;
                        tracing::info!(
                            signal = ?signal,
                            from = %current_decision.recommended_model.name(),
                            to = %next.recommended_model.name(),
                            cloud_allowed = allow_cloud,
                            "Re-routed after local collapse"
                        );
                        current_decision = next;
                        if allow_cloud && current_decision.recommended_model.is_cloud() {
                            authorized_cloud = Some(current_decision.recommended_model.clone());
                        }
                        continue;
                    }
                }
            }

            if let Some(ref labels) = advice_labels {
                self.advisor.record_feedback(labels, true);
            }

            self.clear_model_cooldown(current_decision.recommended_model.name())
                .await;

            let elapsed = inference_start.elapsed();
            let latency_ms = elapsed.as_millis() as u64;
            self.metrics.write().await.record_router_latency(latency_ms);
            arkavo_observability::subsystem_timing::global_timing()
                .router_decisions
                .record(latency_ms);
            arkavo_observability::subsystem_timing::global_timing()
                .inference
                .record(latency_ms);

            let quality = selector_quality::compute_response_quality(
                &response.content,
                elapsed.as_millis() as u64,
                current_decision.task_category.as_str(),
                response.tool_calls.len(),
                tools_were_attached,
            );
            tracing::info!(
                model = %current_decision.recommended_model.name(),
                category = current_decision.task_category.as_str(),
                quality = format!("{quality:.3}").as_str(),
                latency_ms = elapsed.as_millis() as u64,
                response_len = response.content.len(),
                tool_call_count = response.tool_calls.len(),
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

            // Feasibility plane (post-dispatch): fold this call's real decode
            // throughput into the per-config baseline so "slow" is learned per
            // model+context, and surface a degraded-throughput signal when this
            // sample is slow for that configuration.
            if let Some(timing) = response.inference_timing.as_ref() {
                self.record_local_throughput(
                    &current_decision.recommended_model,
                    timing,
                    input_tokens as u32,
                );
            }

            // Record which model was selected so the conductor can attribute
            // reward-based corrective feedback to the right Thompson Sampling prior.
            if let Ok(mut guard) = self.last_routed_model.write() {
                *guard = Some(current_decision.recommended_model.name().to_string());
            }

            // Store the decision trace for downstream attribution
            if let Ok(mut guard) = self.last_decision_trace.write() {
                *guard = Some(current_decision.trace.clone());
            }

            // Append to recent traces ring buffer for UI dashboard
            if let Ok(mut guard) = self.recent_traces.write() {
                guard.push_back(current_decision.trace.clone());
                while guard.len() > 50 {
                    guard.pop_front();
                }
            }

            response.quality_gate_retries = attempt;
            return Ok(RoutedResponse {
                response,
                model: actual_model,
                attempts,
            });
        }

        Err(Error::MaxRetriesExceeded {
            attempts: MAX_RETRIES,
        })
    }
}

// A rejected tool call still needs an output paired to its native ID before
// Responses can continue the conversation. Nothing in this retry was executed.
fn append_rejected_response(messages: &mut Vec<Message>, response: &ProviderResponse) {
    if response.provider_state.is_empty() {
        messages.push(Message::assistant(response.content.clone()));
        return;
    }
    messages.push(response.as_assistant_message());
    for (id, name) in response.provider_state.native_calls() {
        messages.push(Message::tool_result(
            "Tool call rejected by response validation; it was not executed.",
            id,
            // A provider that recorded a call but omitted its name still needs
            // an answer, so it is attributed to a generic tool, not dropped.
            if name.is_empty() { "tool" } else { name },
        ));
    }
}

#[cfg(test)]
mod tests {
    use crate::selector_quality::compute_response_quality;
    use crate::tool_extraction;
    use arkavo_mcp_tools::{DetailLevel, ToolRegistry};
    use arkavo_test_macros::spec;

    /// Regression for the bug surfaced by gitar-bot on PR #598: an empty
    /// `ToolRegistry` (or a registry whose keyword search yields zero hits)
    /// produced `Some(json!([]))` for `tools_json`, and the prior derivation
    /// `tools_were_attached = tools_json.is_some()` collapsed that to `true`.
    /// That triggered the `-0.7` text-without-tool-call penalty in
    /// `compute_response_quality` and poisoned Thompson Sampling for models
    /// that had behaved correctly (no tools were ever actually offered).
    ///
    /// The fix derives `tools_were_attached` from `!tool_infos.is_empty()`,
    /// so the empty-tools path scores the same as the no-registry path.
    #[spec("ROUTER-002")]
    #[tokio::test]
    async fn empty_tool_search_does_not_count_as_attached() {
        let registry = ToolRegistry::empty();
        let tool_infos = tool_extraction::search_tools_hybrid(
            &registry,
            "nonexistent_keyword_xyz",
            DetailLevel::NameAndDescription,
            Some(100),
        )
        .await;
        assert!(
            tool_infos.is_empty(),
            "empty registry must return zero search hits"
        );

        // The Anthropic JSON wrapper still produces Some(json!([])), but the
        // fix derives attachment from tool_infos, not the JSON wrapper.
        let json_wrapper = arkavo_llm::McpConverter::to_anthropic_format_minimal(&tool_infos);
        assert!(
            json_wrapper
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(false),
            "empty tools should serialize to an empty JSON array"
        );

        let tools_were_attached = !tool_infos.is_empty();
        assert!(
            !tools_were_attached,
            "zero tool infos must NOT be treated as 'tools attached'"
        );

        let prose = "I will analyze the situation and consider my options before acting.";
        let quality_no_penalty =
            compute_response_quality(prose, 500, "general", 0, tools_were_attached);
        let quality_with_penalty = compute_response_quality(prose, 500, "general", 0, true);
        assert!(
            quality_no_penalty > quality_with_penalty,
            "empty-tools path must avoid the -0.7 tool-required penalty \
             (no_penalty={quality_no_penalty}, with_penalty={quality_with_penalty})"
        );
    }
    #[spec("ASTRA-002")]
    #[test]
    fn validation_retry_preserves_reasoning_and_resolves_tool_ids() {
        let response = arkavo_llm::ProviderResponse {
            provider_state: arkavo_llm::ProviderState::openai_responses(vec![
                serde_json::json!({"type":"reasoning","id":"reasoning-1","encrypted_content":"opaque"}),
                serde_json::json!({"type":"function_call","call_id":"call-1","name":"read","arguments":"{}"}),
            ]),
            tool_calls: vec![arkavo_llm::tool_parser::ParsedToolCall {
                tool_name: "read".into(),
                arguments: serde_json::json!({}),
                call_id: Some("call-1".into()),
            }],
            ..Default::default()
        };
        let mut messages = Vec::new();
        super::append_rejected_response(&mut messages, &response);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].provider_state, response.provider_state);
        assert_eq!(messages[1].tool_call_id.as_deref(), Some("call-1"));
        assert!(messages[1].content.contains("not executed"));
    }

    /// The tool loop dispatches paid cloud calls exactly like chat, so it must
    /// consult the same cloud-spend policy — `LocalOnly` used to be silently
    /// ignored here, letting an OPENAI_API_KEY-only agent reach Astra.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn local_only_denies_the_tool_loop_path() {
        use crate::Error;
        use crate::test_support::{CountingProvider, cloud_router};

        let provider = CountingProvider::new("ok");
        let router = cloud_router(arkavo_budget::CloudPolicy::LocalOnly, "openai", &provider).await;
        let error = router
            .route_with_tools_attributed(
                "summarize the diff",
                vec![arkavo_llm::Message::user("summarize the diff")],
                None,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::ModerationBlocked { policy_id, .. } if policy_id == "cloud_spend"),
            "got {error:?}"
        );
        assert_eq!(provider.builds(), 0, "a denied call must not open a client");
        assert_eq!(provider.calls(), 0);
    }

    /// Amendment (b): an auto-selected cloud arm has no caller authorization,
    /// so `AskBeforeCloud` must ask before the loop spends anything.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn auto_selected_cloud_asks_before_the_loop_spends() {
        use crate::Error;
        use crate::test_support::{CountingProvider, cloud_router};

        let provider = CountingProvider::new("ok");
        let router = cloud_router(
            arkavo_budget::CloudPolicy::AskBeforeCloud,
            "openai",
            &provider,
        )
        .await;
        let error = router
            .route_with_tools_attributed(
                "summarize the diff",
                vec![arkavo_llm::Message::user("summarize the diff")],
                None,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::CloudConfirmationRequired { model, .. } if model == "gpt-6-astra"),
            "got {error:?}"
        );
        assert_eq!(provider.calls(), 0);
    }

    /// Amendment (c): once confirmed the loop proceeds — and a retry *inside*
    /// that loop must not re-ask, because the one-shot flag is already spent.
    /// The empty registry rejects the answer's tool call, so all three attempts
    /// run against the same authorized arm.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn confirmation_covers_every_retry_of_the_same_arm() {
        use crate::test_support::{CountingProvider, cloud_router};

        let provider = CountingProvider::calling_tool("no_such_tool");
        let router = cloud_router(
            arkavo_budget::CloudPolicy::AskBeforeCloud,
            "openai",
            &provider,
        )
        .await;
        router.confirm_next_cloud_upgrade();

        let registry = ToolRegistry::empty();
        let routed = router
            .route_with_tools_attributed(
                "summarize the diff",
                vec![arkavo_llm::Message::user("summarize the diff")],
                Some(&registry),
            )
            .await
            .expect("the confirmed arm must not be re-asked mid-loop");
        assert_eq!(routed.model, crate::ModelChoice::Gpt6Astra);
        assert_eq!(
            provider.calls(),
            3,
            "every validation retry reuses the authorization granted once"
        );
    }

    /// The collapse plane may upgrade a breakdown to cloud once the spend plane
    /// authorizes it. That authorization consumes the user's one-shot flag, so
    /// the retry it schedules must inherit it — otherwise the loop either
    /// re-asks (and fails the whole request) or silently re-offers the upgrade
    /// it was just granted.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn a_confirmed_collapse_upgrade_is_not_re_asked() {
        use crate::selector::ModelSelector;
        use crate::test_support::{CountingProvider, only};
        use crate::{ConnectivityChecker, Router, RouterEvent};

        // Exactly two feasible arms: the cheapest cached local model and the one
        // configured cloud provider. The memory budget drops every other local.
        let selector = ModelSelector::with_availability(only("openai"), true);
        selector.set_memory_budget(600_000_000);
        assert_eq!(
            selector.feasible_models(),
            vec![
                crate::ModelChoice::LocalQwen3,
                crate::ModelChoice::Gpt6Astra
            ]
        );

        let provider = CountingProvider::blank_then("a complete answer for the request");
        let mut router = Router::new_offline().await.unwrap();
        router.set_offline_mode(false);
        let router = router
            .with_cloud_policy(arkavo_budget::CloudPolicy::AskBeforeCloud)
            .with_connectivity(ConnectivityChecker::assume(true))
            .with_selector(selector)
            .await
            .with_provider_factory(provider.factory());
        let _ = router.drain_events();
        router.confirm_next_cloud_upgrade();

        let routed = router
            .route_with_tools_attributed(
                "summarize the diff",
                vec![arkavo_llm::Message::user("summarize the diff")],
                None,
            )
            .await
            .expect("a confirmed dispatch must not be re-asked mid-loop");
        assert!(!routed.response.content.is_empty());
        assert_eq!(provider.calls(), 2, "the collapse must have been retried");
        assert!(
            !router
                .drain_events()
                .iter()
                .any(|event| matches!(event, RouterEvent::CloudUpgradeOffered { .. })),
            "an already-confirmed upgrade must not be offered again"
        );
    }
}
