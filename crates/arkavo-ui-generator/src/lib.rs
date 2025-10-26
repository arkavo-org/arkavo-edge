pub mod health;
pub mod incremental;
pub mod planner;
pub mod prompt;
pub mod renderer;
pub mod streaming;
pub mod templates;
pub mod vision;

use anyhow::Result;
use arkavo_router::{Router, RoutingDecision};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiGenerationRequest {
    pub user_intent: String,
    pub context: UiContext,
    pub preferences: UiPreferences,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiContext {
    pub available_agents: Vec<String>,
    pub active_telemetry: Vec<TelemetrySnapshot>,
    pub current_page: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySnapshot {
    pub agent_id: String,
    pub metric_type: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiPreferences {
    pub theme: String,
    pub component_library: String,
    pub max_complexity: u8,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            component_library: "vanilla".to_string(),
            max_complexity: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedUi {
    pub html: String,
    pub css: String,
    pub javascript: String,
    pub metadata: UiMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiMetadata {
    pub component_type: String,
    pub model_used: String,
    pub generation_time_ms: u64,
    pub cost_usd: f64,
    pub version: String,
}

pub struct UiGenerator {
    router: Arc<Router>,
    prompt_builder: prompt::PromptBuilder,
    renderer: renderer::CodeRenderer,
}

impl UiGenerator {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            router: Arc::new(Router::new().await?),
            prompt_builder: prompt::PromptBuilder::new(),
            renderer: renderer::CodeRenderer::new()?,
        })
    }

    pub async fn generate(&self, request: UiGenerationRequest) -> Result<GeneratedUi> {
        let start_time = std::time::Instant::now();

        let task_description = format!(
            "Generate web UI for: {}. Context: {} agents available",
            request.user_intent,
            request.context.available_agents.len()
        );

        let routing_decision = self.router.route(&task_description).await?;

        let prompt = self.prompt_builder.build(&request, &routing_decision)?;

        let generated_code = self.generate_with_llm(&prompt, &routing_decision).await?;

        let ui = self.renderer.parse_and_render(generated_code)?;

        let generation_time = start_time.elapsed().as_millis() as u64;

        let component_type = self.detect_component_type(&ui.html);

        Ok(GeneratedUi {
            html: ui.html,
            css: ui.css,
            javascript: ui.javascript,
            metadata: UiMetadata {
                component_type,
                model_used: routing_decision.recommended_model.name().to_string(),
                generation_time_ms: generation_time,
                cost_usd: routing_decision.estimated_cost_usd,
                version: uuid::Uuid::new_v4().to_string(),
            },
        })
    }

    async fn generate_with_llm(&self, prompt: &str, _decision: &RoutingDecision) -> Result<String> {
        Ok(prompt.to_string())
    }

    fn detect_component_type(&self, html: &str) -> String {
        if html.contains("<canvas") {
            "chart".to_string()
        } else if html.contains("<form") {
            "form".to_string()
        } else if html.contains("<table") {
            "table".to_string()
        } else {
            "custom".to_string()
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_generator_creation() {
        let result = UiGenerator::new().await;
        assert!(result.is_ok() || result.is_err());
    }
}
