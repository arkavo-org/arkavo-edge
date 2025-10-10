use anyhow::Result;
use arkavo_llm::Message;
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

        let classifier = self.router.get_local_provider();

        let response = classifier
            .complete(vec![Message::user(planning_prompt)])
            .await
            .map_err(|e| anyhow::anyhow!("LLM planning failed: {e}"))?;

        self.parse_plan(&response)
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
        let json_str = if let Some(start) = trimmed.find('[') {
            if let Some(end) = trimmed.rfind(']') {
                &trimmed[start..=end]
            } else {
                trimmed
            }
        } else {
            trimmed
        };

        let parts: Vec<ComponentPart> = serde_json::from_str(json_str)?;

        if parts.is_empty() || parts.len() > 10 {
            anyhow::bail!("Invalid number of parts: {}", parts.len());
        }

        Ok(BuildPlan { parts })
    }
}

#[cfg(test)]
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
