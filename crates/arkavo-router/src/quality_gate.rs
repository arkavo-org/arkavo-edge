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
            None,
            None,
        )
        .await
        .map(|r| r.response)
    }

    /// Route on behalf of one chat session, so a cloud approval its user gave
    /// authorizes this call and no other session's.
    pub async fn route_with_tools_for_session(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        session: &str,
    ) -> Result<ProviderResponse> {
        self.route_with_tools_internal(
            task_description,
            messages,
            tool_registry,
            None,
            None,
            Some(session),
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
            Some(budget),
            None,
        )
        .await
    }

    pub async fn route_with_tools_attributed(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<RoutedResponse> {
        self.route_with_tools_internal(task_description, messages, tool_registry, None, None, None)
            .await
    }

    // Internal plumbing, not API surface: every public entry point above hands
    // this one call its own shape (hint, ledger, session).
    async fn route_with_tools_internal(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        model_hint: Option<&crate::ModelChoice>,
        budget: Option<CallBudget<'_>>,
        session: Option<&str>,
    ) -> Result<RoutedResponse> {
        const MAX_RETRIES: u8 = 3;
        let budget = budget.or_else(|| self.call_budget());
        let mut current_decision = self.classify_for_session(task_description, session).await?;

        // Which arm the caller's hint actually put in play, if any. Not
        // "the hint happens to equal what classification chose": the hint is
        // declined on cooldown or when its weights are absent, and
        // classification can land on the same arm by itself. Treating that
        // coincidence as the caller's choice let an unapplied hint stand in
        // for both consent to spend and consent to download.
        let mut applied_hint: Option<crate::ModelChoice> = None;

        if let Some(hint) = model_hint {
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
                applied_hint = Some(hint.clone());
            } else {
                tracing::debug!(
                    hint = hint.name(),
                    "Model hint not available, using default"
                );
            }
        }

        let mut feedback_messages: Vec<Message> = Vec::new();
        let mut attempts = Vec::new();
        // Cloud arm already authorized for this dispatch, so a retry against the
        // same model is not re-asked while a switch to a different cloud arm
        // still is.
        let mut authorized_cloud: Option<crate::ModelChoice> = None;

        let input_tokens = tool_extraction::estimate_tokens(task_description);
        let is_simple = prompt_advisor::is_simple_query(&task_description.to_lowercase());

        for attempt in 0..MAX_RETRIES {
            let inference_start = std::time::Instant::now();

            // Track whether tools were actually attached (non-empty) so the
            // quality scorer below can penalize text-only responses correctly.
            // A registry that returns zero matches must NOT count as attached.
            let (tools_json, tools_were_attached) = match tool_registry {
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
            let max_tokens = 4096usize;
            // Reserve for the request in hand, not for a maximum-length answer:
            // the loop settles every attempt against measured usage below, so
            // the preflight only has to be a realistic bound.
            let estimated_usage = crate::usage::reserve_request(
                &advised_messages,
                tools_json.as_ref(),
                max_tokens as u32,
            );
            let estimated_cost = self.usage_cost(&actual_model, &estimated_usage);
            // Cloud-spend policy gates the tool-loop exactly as it gates chat,
            // and before the provider is built so a denial never opens a client.
            // "Explicit" means the caller named this model (an applied hint) or
            // it was already authorized for this dispatch.
            let caller_authorized = authorized_cloud.as_ref() == Some(&actual_model)
                || applied_hint.as_ref() == Some(&actual_model);
            // Unconditional, with no exemption for a hinted arm: applying a
            // hint already required `is_model_available`, which asks the
            // selector the same cache question, so an applied local hint
            // passes here anyway and a cloud arm is never local. An arm
            // Thompson Sampling picked is not a request to download it.
            self.require_provisioned(&actual_model)?;
            if let Some(budget) = budget {
                budget.check(estimated_cost).await?;
            }
            self.authorize_call(&actual_model, estimated_cost, caller_authorized, session)
                .await?;
            if actual_model.is_cloud() {
                authorized_cloud = Some(actual_model.clone());
            }
            let provider = self
                .instantiate_provider_exact_with_spec(
                    &actual_model,
                    current_decision.use_spec_decoding,
                )
                .await?;

            let _permit = self
                .inference_semaphore
                .acquire()
                .await
                .map_err(|_| Error::ModelExecution("Semaphore closed".to_string()))?;
            tracing::debug!("Inference semaphore acquired");

            // Feasibility plane (pre-dispatch): assess whether the local model
            // can run this prompt now, surfacing a reshape/unavailable signal
            // before we spend an inference on a doomed call. Local-only; never
            // spends.
            self.check_local_feasibility(&current_decision.recommended_model, input_tokens as u32);

            let request_usage =
                crate::usage::estimate_request(&advised_messages, tools_json.as_ref(), 0);
            let mut response = match provider
                .complete_with_tools(advised_messages, tools_json, Some(max_tokens))
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
            if response.tool_calls.is_empty() && attempt + 1 < MAX_RETRIES {
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
                    // cap — never on the quality signal alone. A standing
                    // approval from this caller (recorded after a
                    // CloudUpgradeOffered) satisfies AskBeforeCloud; otherwise the
                    // decision tells us whether to offer (ask) or refuse.
                    let allow_cloud = if let UpgradeOffer::Offer(reason) = offer {
                        let caps = self.cloud_spend_caps().await;
                        let projected = self.projected_cloud_cost(&current_decision);
                        // Only a *user* approval authorizes the upgrade. A
                        // caller naming a model authorizes that model, not a
                        // later switch to a different, possibly dearer arm, so
                        // `authorized_cloud` deliberately does not count here.
                        let confirmed = self.cloud_approved(session);
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

    /// Regression: the tool loop resolved its arm from classification and went
    /// straight to provider construction, so a device with no weights on disk
    /// and no cloud keys reached `load_local_model` and started a
    /// multi-gigabyte fetch inside the caller's turn. The guard sits at the
    /// dispatch site, not in `classify`: the hint is applied *after*
    /// classification, so a check inside `classify` would test an arm the loop
    /// is not going to run — and classification is also used for previews and
    /// metrics that never dispatch.
    #[spec("ROUTER-003")]
    #[tokio::test]
    async fn the_tool_loop_refuses_an_unprovisioned_automatic_arm() {
        use crate::test_support::CountingProvider;
        use crate::{Error, ModelSelector, ProviderAvailability, Router};

        let provider = CountingProvider::new("answer");
        let router = Router::new_offline()
            .await
            .unwrap()
            .with_selector(ModelSelector::with_availability(
                ProviderAvailability::default(),
                false,
            ))
            .await
            .with_provider_factory(provider.factory());

        let error = router
            .route_with_tools(
                "summarize the diff",
                vec![arkavo_llm::Message::user("hello")],
                None,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::ModelNotAvailable { .. }),
            "got {error:?}"
        );
        assert_eq!(provider.builds(), 0, "a refusal must not open a client");
        assert_eq!(provider.calls(), 0);
    }

    /// Regression: the guard exempted "the hint equals the arm we are about to
    /// run", which is value equality against whatever classification produced —
    /// not evidence that the hint was applied. On a bare device the hint block
    /// correctly declines an uncached `qwen3.5-0.8b`, classification lands on
    /// the same arm by itself, and the coincidence used to skip the guard and
    /// download it.
    #[spec("ROUTER-003")]
    #[tokio::test]
    async fn a_declined_hint_does_not_authorize_a_download() {
        use crate::test_support::CountingProvider;
        use crate::{Error, ModelChoice, ModelSelector, ProviderAvailability, Router};

        let provider = CountingProvider::new("answer");
        let router = Router::new_offline()
            .await
            .unwrap()
            .with_selector(ModelSelector::with_availability(
                ProviderAvailability::default(),
                false,
            ))
            .await
            .with_provider_factory(provider.factory());

        let error = router
            .route_with_tools_hinted(
                "summarize the diff",
                vec![arkavo_llm::Message::user("hello")],
                None,
                Some(&ModelChoice::LocalQwen3),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::ModelNotAvailable { .. }),
            "got {error:?}"
        );
        assert_eq!(provider.builds(), 0, "a refusal must not open a client");
        assert_eq!(provider.calls(), 0);
    }

    /// The positive control: the same hint on a provisioned device is applied
    /// and served.
    #[spec("ROUTER-003")]
    #[tokio::test]
    async fn an_applied_hint_is_served_on_a_provisioned_device() {
        use crate::test_support::CountingProvider;
        use crate::{ModelChoice, ModelSelector, ProviderAvailability, Router};

        let provider = CountingProvider::new("answer");
        let router = Router::new_offline()
            .await
            .unwrap()
            .with_selector(ModelSelector::with_availability(
                ProviderAvailability::default(),
                true,
            ))
            .await
            .with_provider_factory(provider.factory());

        let response = router
            .route_with_tools_hinted(
                "summarize the diff",
                vec![arkavo_llm::Message::user("hello")],
                None,
                Some(&ModelChoice::LocalQwen3),
            )
            .await
            .expect("an applied hint on a provisioned device is served");
        assert_eq!(response.content, "answer");
        assert_eq!(provider.built_models(), vec![ModelChoice::LocalQwen3]);
    }

    /// The same coincidence stood in for consent to *spend*: a cooled-down
    /// hint is not applied, but classification picked the same cloud arm, and
    /// value equality reported that as the caller having named it — silently
    /// satisfying `AskBeforeCloud`.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn a_declined_cloud_hint_is_not_the_users_consent_to_spend() {
        use crate::test_support::{CountingProvider, cloud_router};
        use crate::{Error, ModelChoice};

        let provider = CountingProvider::new("answer");
        let router = cloud_router(
            arkavo_budget::CloudPolicy::AskBeforeCloud,
            "openai",
            &provider,
        )
        .await;
        // Three sustained quality failures put the hint past
        // HINT_OVERRIDE_THRESHOLD, so it is declined. Unlike a cooldown this
        // does not exclude the arm from the feasible set, so classification
        // still reaches Astra on its own — which is the coincidence under test.
        for _ in 0..3 {
            router
                .record_reward_failure(ModelChoice::Gpt6Astra.name())
                .await;
        }

        let error = router
            .route_with_tools_hinted(
                "design the API surface",
                vec![arkavo_llm::Message::user("hello")],
                None,
                Some(&ModelChoice::Gpt6Astra),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::CloudConfirmationRequired { .. }),
            "a hint that was never applied is nobody's consent: got {error:?}"
        );
        assert_eq!(provider.builds(), 0, "a refusal must not open a client");
    }

    /// The mirror: with the weights on disk the same call is served, so the
    /// guard refuses a missing weight and nothing else.
    #[spec("ROUTER-003")]
    #[tokio::test]
    async fn the_tool_loop_serves_a_provisioned_automatic_arm() {
        use crate::test_support::CountingProvider;
        use crate::{ModelSelector, ProviderAvailability, Router};

        let provider = CountingProvider::new("answer");
        let router = Router::new_offline()
            .await
            .unwrap()
            .with_selector(ModelSelector::with_availability(
                ProviderAvailability::default(),
                true,
            ))
            .await
            .with_provider_factory(provider.factory());

        let response = router
            .route_with_tools(
                "summarize the diff",
                vec![arkavo_llm::Message::user("hello")],
                None,
            )
            .await
            .expect("a provisioned device serves the tool loop");
        assert_eq!(response.content, "answer");
        assert_eq!(provider.builds(), 1);
    }

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
        router.approve_cloud_for_host();

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
        router.approve_cloud_for_host();

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

    /// A one-cent session cap is genuinely exhausted for any paid arm, and the
    /// ledger must say so before a client is opened. The proportional reserve
    /// only re-prices the call; `CallBudget::check` still rounds it up to a whole
    /// cent, so re-pricing must not soften this refusal into a bypass.
    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn a_tiny_cap_still_refuses_the_loop_before_a_provider_exists() {
        use crate::test_support::{CountingProvider, cloud_router};
        use crate::{Error, ModelChoice};
        use arkavo_budget::{BudgetConfig, BudgetTracker, TokenCost};
        use std::sync::Arc;

        let mut config = BudgetConfig::default();
        config.limits.session_limit = Some(TokenCost::from_cents(1));
        let tracker = Arc::new(BudgetTracker::new(config).await.unwrap());
        let provider = CountingProvider::new("ready");
        let router = cloud_router(
            arkavo_budget::CloudPolicy::CloudWithinCap,
            "openai",
            &provider,
        )
        .await
        .with_budget_tracker(tracker.clone());

        let error = router
            .route_with_tools_hinted(
                "reply to the operator",
                vec![arkavo_llm::Message::user(
                    "Reply with the single word ready.",
                )],
                None,
                Some(&ModelChoice::Gpt6Astra),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::BudgetExceeded(_)),
            "an exhausted cap must refuse the loop: got {error:?}"
        );
        assert_eq!(provider.builds(), 0, "a refusal must not open a client");
        assert_eq!(provider.calls(), 0);
        assert!(tracker.get_spending_history(10).await.is_empty());
    }

    /// Regression: the hinted tool loop reserved a full 4096-token response for
    /// every call, so on Astra's $50/MTok output rate the preflight bound was
    /// at least $0.2048 before the prompt was even counted — and `CallBudget`
    /// rounds that up to 21 cents. A 20-cent session cap could therefore never
    /// fund a single one-line request: it reported `BudgetExceeded` and
    /// abandoned the task without ever building a provider, while the answer it
    /// refused to make would have cost a fraction of a cent.
    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn a_modest_cap_funds_a_small_astra_tool_loop_call() {
        use crate::ModelChoice;
        use crate::test_support::{CountingProvider, cloud_router};
        use arkavo_budget::{BudgetConfig, BudgetTracker, TokenCost};
        use std::sync::Arc;

        let mut config = BudgetConfig::default();
        config.limits.session_limit = Some(TokenCost::from_cents(20));
        let tracker = Arc::new(BudgetTracker::new(config).await.unwrap());
        let provider = CountingProvider::new("ready");
        let router = cloud_router(
            arkavo_budget::CloudPolicy::CloudWithinCap,
            "openai",
            &provider,
        )
        .await
        .with_budget_tracker(tracker.clone());

        let response = router
            .route_with_tools_hinted(
                "reply to the operator",
                vec![arkavo_llm::Message::user(
                    "Reply with the single word ready.",
                )],
                None,
                Some(&ModelChoice::Gpt6Astra),
            )
            .await
            .expect("a 20-cent cap must fund one short cloud call");
        assert_eq!(response.content, "ready");
        assert_eq!(provider.built_models(), vec![ModelChoice::Gpt6Astra]);
        assert_eq!(provider.calls(), 1, "one dispatch answered the request");
        // Settlement is what actually charges the ledger, so the call the
        // reserve admitted is still accounted against the cap.
        assert_eq!(tracker.get_spending_history(10).await.len(), 1);
    }
}
