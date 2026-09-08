use anyhow::Result;
use arkavo_llm::Message;
use arkavo_router::{ModelChoice, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Output allowance for one planning call — a JSON array of at most ten parts.
const MAX_PLAN_TOKENS: u32 = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildPlan {
    pub parts: Vec<ComponentPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentPart {
    pub id: String,
    pub name: String,
    pub description: String,
    pub priority: usize,
}

pub struct UiPlanner {
    router: Arc<Router>,
}

impl UiPlanner {
    pub fn new(router: Arc<Router>) -> Self {
        Self { router }
    }

    pub async fn plan(&self, user_prompt: &str) -> Result<BuildPlan> {
        let plan = self.try_llm_plan(user_prompt).await?;
        println!(
            "UiPlanner: Using LLM-generated plan with {} parts",
            plan.parts.len()
        );
        for part in &plan.parts {
            println!("  - {} ({}): {}", part.name, part.id, part.description);
        }
        Ok(plan)
    }

    async fn try_llm_plan(&self, user_prompt: &str) -> Result<BuildPlan> {
        let messages = vec![Message::user(self.build_planning_prompt(user_prompt))];
        let model = self.planning_model()?;

        // Planning spends like any other call, so it faces the router's cloud
        // policy and spend caps before a client exists — a refusal must not
        // open a connection, and must not be silently downgraded to a
        // different arm than the one the user was asked about.
        let usage = arkavo_router::usage::estimate_request(&messages, None, MAX_PLAN_TOKENS);
        let cost = self.router.usage_cost(&model, &usage);
        self.router
            .authorize_call(&model, cost, false, None)
            .await?;

        let (provider, _) = self.router.get_provider_attributed(&model).await?;
        let response = provider
            .complete(messages)
            .await
            .map_err(|e| anyhow::anyhow!("UI planning failed on {}: {e}", model.name()))?;

        self.parse_plan(&response)
    }

    /// The arm that plans: this device's local model when its weights are on
    /// disk, and the configured cloud augmentation only when they are not.
    /// Cloud augments local inference here as everywhere else; it does not
    /// replace it.
    fn planning_model(&self) -> Result<ModelChoice> {
        let local = self.router.default_chat_model();
        if self.router.require_provisioned(&local).is_ok() {
            return Ok(local);
        }
        self.router.cloud_augmentation_model().ok_or_else(|| {
            anyhow::anyhow!(
                "No planning model available: {} is not provisioned and no cloud provider is configured",
                local.name()
            )
        })
    }

    fn build_planning_prompt(&self, user_prompt: &str) -> String {
        format!(
            r#"You are a UI architect. Break down this UI request into 5-10 discrete, buildable parts.

User Request: {user_prompt}

Respond with ONLY a JSON array in this exact format:
[
  {{"id": "part-1", "name": "Header Section", "description": "Top navigation and branding", "priority": 1}},
  {{"id": "part-2", "name": "Main Content", "description": "Primary content area", "priority": 2}}
]

Rules:
- Each part must be independently buildable
- Order by logical rendering priority (1 = first)
- Keep parts focused (one clear purpose each)
- Total 5-10 parts maximum
- Description should be clear and specific

Return ONLY the JSON array, nothing else."#
        )
    }

    fn parse_plan(&self, response: &str) -> Result<BuildPlan> {
        let trimmed = response.trim();

        // Try to extract JSON from markdown code fences first
        let json_str = if let Some(json_start) = trimmed.find("```json") {
            // Look for the end of the code fence
            let after_fence = &trimmed[json_start + 7..]; // Skip past ```json
            if let Some(fence_end) = after_fence.find("```") {
                after_fence[..fence_end].trim()
            } else {
                // No closing fence, try to find JSON array
                after_fence.trim()
            }
        } else if let Some(start) = trimmed.find('[') {
            // Find the matching closing bracket for the first array
            let after_start = &trimmed[start..];
            let mut depth = 0;
            let mut end_pos = 0;

            for (i, ch) in after_start.chars().enumerate() {
                match ch {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            end_pos = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if end_pos > 0 {
                &after_start[..end_pos]
            } else {
                trimmed
            }
        } else {
            trimmed
        };

        let parts: Vec<ComponentPart> = serde_json::from_str(json_str).map_err(|e| {
            anyhow::anyhow!("Failed to parse JSON plan: {}. JSON was: {}", e, json_str)
        })?;

        if parts.is_empty() || parts.len() > 10 {
            anyhow::bail!("Invalid number of parts: {}", parts.len());
        }

        Ok(BuildPlan { parts })
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_router::{
        ConnectivityChecker, ModelSelector, ProviderAvailability, ProviderFactory,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts provider construction, so "the plan was refused before a client
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

    #[test]
    fn test_parse_plan() {
        // Test the JSON parsing logic without requiring a Router
        let json = r#"[
            {"id": "part-1", "name": "Header", "description": "Top bar", "priority": 1},
            {"id": "part-2", "name": "Content", "description": "Main area", "priority": 2}
        ]"#;

        // Parse directly using serde_json since we're just testing parsing
        let parts: Result<Vec<ComponentPart>, _> = serde_json::from_str(json);
        assert!(parts.is_ok());

        let parts = parts.unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "Header");
        assert_eq!(parts[0].description, "Top bar");
        assert_eq!(parts[0].priority, 1);
    }

    /// Regression: the planner reached for a raw Gemini client through
    /// `Router::get_planning_provider`, which never consulted the cloud
    /// policy — so a `LocalOnly` install still sent the prompt to Gemini.
    /// Planning now goes through the same gate as every other call, and a
    /// refusal opens no client.
    #[tokio::test]
    async fn local_only_refuses_the_planning_call_without_building_a_client() {
        let factory = Arc::new(CountingFactory::default());
        let mut router = Router::new_offline().await.unwrap();
        router.set_offline_mode(false);
        let router = router
            .with_cloud_policy(arkavo_budget::CloudPolicy::LocalOnly)
            .with_connectivity(ConnectivityChecker::assume(true))
            // No local weights on disk, one configured cloud provider: the
            // deployment where the ungated path used to reach the network.
            .with_selector(ModelSelector::with_availability(
                ProviderAvailability {
                    gemini: true,
                    ..ProviderAvailability::default()
                },
                false,
            ))
            .await
            .with_provider_factory(factory.clone());

        let error = UiPlanner::new(Arc::new(router))
            .plan("a dashboard with charts")
            .await
            .expect_err("a LocalOnly install must refuse a cloud planning call");
        assert!(
            error.to_string().contains("Cloud inference denied"),
            "got {error}"
        );
        assert_eq!(
            factory.builds.load(Ordering::SeqCst),
            0,
            "a refused plan must not open a client"
        );
    }
}
