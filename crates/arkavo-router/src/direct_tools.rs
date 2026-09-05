//! Explicit and execution-model routing without adaptive quality retries.
use crate::error::{Error, Result};
use crate::learning::BurstFeedback;
use crate::usage::RoutedResponse;
use crate::{selector_quality, tool_extraction};
use arkavo_llm::{Message, ProviderResponse};
use arkavo_mcp_tools::ToolRegistry;

impl super::Router {
    /// Route for execution iterations — stripped profile for fast tool calls.
    ///
    /// Uses near-greedy temperature (0.1), thinking disabled, max 200 tokens,
    /// and skips Judge validation. For iterations where the model just needs
    /// to emit the next tool call, not reason about it.
    pub async fn route_with_tools_execution(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        model_hint: Option<&crate::ModelChoice>,
    ) -> Result<ProviderResponse> {
        // Execution mode: bypass classification and Thompson Sampling entirely.
        // The model is already known (from the hint or fastest local fallback)
        // and all tools should be passed with compact schemas since the model
        // already saw full schemas in round 0.
        self.route_with_tools_execution_attributed(
            task_description,
            messages,
            tool_registry,
            model_hint,
        )
        .await
        .map(|r| r.response)
    }

    pub async fn route_with_tools_execution_attributed(
        &self,
        _task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        model_hint: Option<&crate::ModelChoice>,
    ) -> Result<RoutedResponse> {
        // Execution mode: bypass classification and Thompson Sampling entirely.
        // The model is already known (from the hint or fastest local fallback)
        // and all tools should be passed with compact schemas since the model
        // already saw full schemas in round 0.
        let model = model_hint
            .cloned()
            .unwrap_or_else(|| self.default_chat_model());

        let tools_json = match tool_registry {
            Some(registry) => {
                // Pass ALL tools with NameAndDescription detail level.
                // Empty query returns everything — no keyword filtering needed
                // since the model already knows which tools to call.
                let tool_infos =
                    registry.search_tools("", arkavo_mcp_tools::DetailLevel::NameAndDescription);
                let json = match model {
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
                Some(json)
            }
            None => None,
        };

        let request_usage = crate::usage::estimate_request(&messages, tools_json.as_ref(), 4096);
        let estimated_cost = self.usage_cost(&model, &request_usage);
        // The execution path spends exactly like chat, so it faces the same
        // ledger and cloud policy — both before the provider is built, so a
        // refusal never opens a client. A caller-supplied hint is an explicit
        // model choice.
        let budget = self.call_budget();
        if let Some(budget) = budget {
            budget.check(estimated_cost).await?;
        }
        self.authorize_call(&model, estimated_cost, model_hint.is_some())
            .await?;
        let (provider, model) = self.get_provider_attributed(&model).await?;
        let _permit = self
            .inference_semaphore
            .acquire()
            .await
            .map_err(|_| Error::ModelExecution("Semaphore closed".to_string()))?;

        let result = provider
            .complete_with_tools(messages, tools_json, None)
            .await;
        let mut response = self
            .account_result(&model, &request_usage, result, budget)
            .await?;

        response.tool_calls = tool_extraction::filter_and_extract_tool_calls(response.tool_calls);

        if response.tool_calls.is_empty() && !response.content.is_empty() {
            let extracted = tool_extraction::extract_tool_calls_from_text(&response.content);
            if !extracted.is_empty() {
                response.tool_calls = extracted;
            }
        }

        let usage = self.attribute_response(model.clone(), &request_usage, &response);
        Ok(RoutedResponse {
            response,
            model,
            attempts: vec![usage],
        })
    }

    /// Route with a model override — bypass classification, Thompson Sampling,
    /// and quality gate retries. Use when AGENTS.md specifies `model:` and the
    /// caller wants the exact model with minimal overhead.
    pub async fn route_with_tools_override(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        model: &crate::ModelChoice,
    ) -> Result<ProviderResponse> {
        self.route_with_tools_override_attributed(task_description, messages, tool_registry, model)
            .await
            .map(|r| r.response)
    }

    pub async fn route_with_tools_override_attributed(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        model: &crate::ModelChoice,
    ) -> Result<RoutedResponse> {
        let inference_start = std::time::Instant::now();

        // Track whether tools were actually attached (non-empty) so the
        // quality scorer can penalize text-only responses correctly. A
        // registry that returns zero matches must NOT count as attached —
        // otherwise the model gets penalized for legitimately producing
        // text when no tools were callable.
        let (tools_json, tools_were_attached) = match tool_registry {
            Some(registry) => {
                let detail_level = tool_extraction::detail_level_for_model(model);
                let keywords = tool_extraction::extract_keywords(task_description);
                let input_tokens = tool_extraction::estimate_tokens(task_description);
                let tool_infos = tool_extraction::search_tools_hybrid(
                    registry,
                    &keywords,
                    detail_level,
                    Some(input_tokens),
                )
                .await;
                let attached = !tool_infos.is_empty();
                let json = match model {
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

        let model = model.clone();
        let request_usage = crate::usage::estimate_request(&messages, tools_json.as_ref(), 4096);
        let estimated_cost = self.usage_cost(&model, &request_usage);
        // An override names the model explicitly, which satisfies
        // `AskBeforeCloud` — but never `LocalOnly` or an exhausted cap.
        let budget = self.call_budget();
        if let Some(budget) = budget {
            budget.check(estimated_cost).await?;
        }
        self.authorize_call(&model, estimated_cost, true).await?;
        let use_spec = self.decide_spec_with_event(model.name());
        let provider = self
            .instantiate_provider_exact_with_spec(&model, use_spec)
            .await?;
        let _permit = self
            .inference_semaphore
            .acquire()
            .await
            .map_err(|_| Error::ModelExecution("Semaphore closed".to_string()))?;
        let result = provider
            .complete_with_tools(messages, tools_json, None)
            .await;
        let mut response = self
            .account_result(&model, &request_usage, result, budget)
            .await?;

        response.tool_calls = tool_extraction::filter_and_extract_tool_calls(response.tool_calls);

        if response.tool_calls.is_empty() && !response.content.is_empty() {
            let extracted = tool_extraction::extract_tool_calls_from_text(&response.content);
            if !extracted.is_empty() {
                response.tool_calls = extracted;
            }
        }

        let elapsed = inference_start.elapsed();
        let quality = selector_quality::compute_response_quality(
            &response.content,
            elapsed.as_millis() as u64,
            "general",
            response.tool_calls.len(),
            tools_were_attached,
        );
        tracing::info!(
            model = model.name(),
            quality = format!("{quality:.3}").as_str(),
            latency_ms = elapsed.as_millis() as u64,
            response_len = response.content.len(),
            tool_call_count = response.tool_calls.len(),
            "Model override: inference completed"
        );
        self.model_learning
            .immediate_update(
                model.name(),
                &BurstFeedback::success(
                    uuid::Uuid::new_v4(),
                    "general".to_string(),
                    elapsed.as_millis() as u64,
                )
                .with_quality(quality),
            )
            .await;

        if let Ok(mut guard) = self.last_routed_model.write() {
            *guard = Some(model.name().to_string());
        }

        let usage = self.attribute_response(model.clone(), &request_usage, &response);
        Ok(RoutedResponse {
            response,
            model,
            attempts: vec![usage],
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::{CountingProvider, cloud_router};
    use crate::{Error, ModelChoice};
    use arkavo_budget::CloudPolicy;
    use arkavo_llm::Message;
    use arkavo_test_macros::spec;

    fn prompt() -> Vec<Message> {
        vec![Message::user("summarize the diff")]
    }

    fn is_policy_error(error: &Error) -> bool {
        matches!(
            error,
            Error::ModerationBlocked { .. } | Error::CloudConfirmationRequired { .. }
        )
    }

    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn local_only_denies_the_override_path() {
        let provider = CountingProvider::new("ok");
        let error = cloud_router(CloudPolicy::LocalOnly, "openai", &provider)
            .await
            .route_with_tools_override_attributed(
                "summarize",
                prompt(),
                None,
                &ModelChoice::Gpt6Astra,
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

    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn local_only_denies_the_execution_path() {
        let provider = CountingProvider::new("ok");
        let error = cloud_router(CloudPolicy::LocalOnly, "openai", &provider)
            .await
            .route_with_tools_execution_attributed(
                "summarize",
                prompt(),
                None,
                Some(&ModelChoice::Gpt6Astra),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::ModerationBlocked { policy_id, .. } if policy_id == "cloud_spend"),
            "got {error:?}"
        );
        assert_eq!(provider.builds(), 0);
        assert_eq!(provider.calls(), 0);
    }

    /// Naming the model is the authorization under `AskBeforeCloud`. The
    /// contrast is what proves a gate is present at all: without one, the
    /// auto-selected arm would have spent too.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn explicit_override_proceeds_where_auto_selection_is_refused() {
        let provider = CountingProvider::new("ok");
        let router = cloud_router(CloudPolicy::AskBeforeCloud, "xai", &provider).await;
        assert_eq!(router.default_chat_model(), ModelChoice::Grok46);

        let refused = router
            .route_with_tools_execution_attributed("summarize", prompt(), None, None)
            .await
            .unwrap_err();
        assert!(
            matches!(&refused, Error::CloudConfirmationRequired { model, .. } if model == "grok-4.6"),
            "got {refused:?}"
        );
        assert_eq!(provider.calls(), 0, "an unconfirmed arm must not spend");

        let routed = router
            .route_with_tools_override_attributed("summarize", prompt(), None, &ModelChoice::Grok46)
            .await
            .unwrap();
        assert_eq!(routed.model, ModelChoice::Grok46);
        assert_eq!(provider.calls(), 1, "the named arm proceeds with no re-ask");
    }

    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn one_shot_confirmation_covers_one_call_then_clears() {
        let provider = CountingProvider::new("ok");
        let router = cloud_router(CloudPolicy::AskBeforeCloud, "xai", &provider).await;

        router.confirm_next_cloud_upgrade();
        router
            .route_with_tools_execution_attributed("summarize", prompt(), None, None)
            .await
            .unwrap();
        assert_eq!(provider.calls(), 1);

        let error = router
            .route_with_tools_execution_attributed("summarize", prompt(), None, None)
            .await
            .unwrap_err();
        assert!(
            matches!(error, Error::CloudConfirmationRequired { .. }),
            "the one-shot flag must not survive the call it authorized"
        );
        assert_eq!(provider.calls(), 1);
    }

    /// `arkavo agent` approves cloud once and then fans out into many routing
    /// calls it does not issue itself; a one-shot flag is spent by the first.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn session_confirmation_covers_every_later_call() {
        let provider = CountingProvider::new("ok");
        let router = cloud_router(CloudPolicy::AskBeforeCloud, "xai", &provider).await;

        router.confirm_cloud_for_session();
        for _ in 0..2 {
            router
                .route_with_tools_execution_attributed("summarize", prompt(), None, None)
                .await
                .unwrap();
        }
        assert_eq!(
            provider.calls(),
            2,
            "no re-ask on the second auto-selected call"
        );
        assert!(
            router.cloud_session_confirmed(),
            "session approval is not consumed"
        );
    }

    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn session_confirmation_still_obeys_local_only() {
        let provider = CountingProvider::new("ok");
        let router = cloud_router(CloudPolicy::LocalOnly, "xai", &provider).await;
        router.confirm_cloud_for_session();
        let error = router
            .route_with_tools_execution_attributed("summarize", prompt(), None, None)
            .await
            .unwrap_err();
        assert!(is_policy_error(&error), "got {error:?}");
        assert_eq!(provider.calls(), 0);
    }
}
