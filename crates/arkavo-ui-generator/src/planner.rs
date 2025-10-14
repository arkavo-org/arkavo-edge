use anyhow::Result;
use arkavo_llm::{Message, Provider};
use arkavo_router::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
        let planning_prompt = self.build_planning_prompt(user_prompt);

        // Try Gemini first (if available), fall back to local model
        let response = if let Some(gemini_provider) = self.router.get_planning_provider() {
            gemini_provider
                .complete(vec![Message::user(planning_prompt.clone())])
                .await
                .map_err(|e| anyhow::anyhow!("Gemini planning failed: {e}"))?
        } else {
            // Use local model via Router
            let local_provider = self.router.get_local_provider();
            local_provider
                .complete(vec![Message::user(planning_prompt)])
                .await
                .map_err(|e| anyhow::anyhow!("Local model planning failed: {e}"))?
        };

        let plan = self.parse_plan(&response)?;

        Ok(plan)
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

    #[tokio::test]
    async fn test_parse_plan() {
        let router = Arc::new(Router::new().await.unwrap());
        let planner = UiPlanner::new(router);

        let json = r#"[
            {"id": "part-1", "name": "Header", "description": "Top bar", "priority": 1},
            {"id": "part-2", "name": "Content", "description": "Main area", "priority": 2}
        ]"#;

        let result = planner.parse_plan(json);
        assert!(result.is_ok());

        let plan = result.unwrap();
        assert_eq!(plan.parts.len(), 2);
        assert_eq!(plan.parts[0].name, "Header");
    }
}
