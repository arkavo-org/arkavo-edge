use crate::{UiGenerationRequest, UiPreferences};
use anyhow::Result;
use arkavo_router::RoutingDecision;

pub struct PromptBuilder {
    system_prompt: String,
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self {
            system_prompt: Self::default_system_prompt(),
        }
    }

    fn default_system_prompt() -> String {
        r#"You are a UI code generator for the Arkavo Edge agent orchestration platform.

Generate clean, production-ready HTML, CSS, and JavaScript based on user requests.

Rules:
1. Use vanilla JavaScript (no frameworks unless specified)
2. Follow modern web standards
3. Include accessibility features
4. Optimize for dark theme by default
5. Make components responsive
6. Include error handling
7. NO placeholder comments or TODOs
8. Generate complete, working code

Output format:
```html
<!-- HTML here -->
```

```css
/* CSS here */
```

```javascript
// JavaScript here
```
"#
        .to_string()
    }

    pub fn build(
        &self,
        request: &UiGenerationRequest,
        decision: &RoutingDecision,
    ) -> Result<String> {
        let context_info = self.build_context_section(&request.context);
        let preference_info = self.build_preference_section(&request.preferences);
        let model_guidance = self.build_model_guidance(decision);

        Ok(format!(
            "{}\n\n{}\n\n{}\n\n{}\n\nUser Request: {}",
            self.system_prompt, context_info, preference_info, model_guidance, request.user_intent
        ))
    }

    fn build_context_section(&self, context: &crate::UiContext) -> String {
        let mut sections = Vec::new();

        if !context.available_agents.is_empty() {
            sections.push(format!(
                "Available Agents: {}",
                context.available_agents.join(", ")
            ));
        }

        if !context.active_telemetry.is_empty() {
            sections.push(format!(
                "Active Telemetry Streams: {} metrics",
                context.active_telemetry.len()
            ));
        }

        if let Some(page) = &context.current_page {
            sections.push(format!("Current Page: {page}"));
        }

        format!("Context:\n{}", sections.join("\n"))
    }

    fn build_preference_section(&self, prefs: &UiPreferences) -> String {
        format!(
            "Preferences:\n- Theme: {}\n- Component Library: {}\n- Max Complexity: {}",
            prefs.theme, prefs.component_library, prefs.max_complexity
        )
    }

    fn build_model_guidance(&self, decision: &RoutingDecision) -> String {
        let complexity = if decision.should_compress {
            "Keep code simple and concise."
        } else {
            "You can use more advanced patterns."
        };

        format!(
            "Model Guidance:\n- Selected: {}\n- {}",
            decision.recommended_model.name(),
            complexity
        )
    }
}

impl Default for PromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::{UiContext, UiGenerationRequest};
    use arkavo_router::{ModelChoice, RoutingDecision, TaskCategory};

    #[test]
    fn test_prompt_building() {
        let builder = PromptBuilder::new();
        let request = UiGenerationRequest {
            user_intent: "Show CPU metrics chart".to_string(),
            context: UiContext {
                available_agents: vec!["agent1".to_string()],
                active_telemetry: vec![],
                current_page: None,
            },
            preferences: UiPreferences::default(),
        };

        let decision = RoutingDecision::new(
            ModelChoice::LocalQwen3,
            TaskCategory::FrontendUI,
            0.9,
            "test".to_string(),
        );

        let prompt = builder.build(&request, &decision);
        assert!(prompt.is_ok());
        let prompt_text = prompt.unwrap();
        assert!(prompt_text.contains("Show CPU metrics chart"));
        assert!(prompt_text.contains("agent1"));
    }
}
