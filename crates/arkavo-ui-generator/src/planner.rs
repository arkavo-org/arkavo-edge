use anyhow::Result;
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
    pub async fn new() -> Result<Self> {
        Ok(Self {
            router: Arc::new(Router::new().await?),
        })
    }

    pub async fn plan(&self, user_prompt: &str) -> Result<BuildPlan> {
        let _planning_prompt = self.build_planning_prompt(user_prompt);

        let _decision = self.router.route(user_prompt).await?;

        self.fallback_plan(user_prompt)
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

    #[allow(dead_code)]
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

    fn fallback_plan(&self, user_prompt: &str) -> Result<BuildPlan> {
        let keywords = user_prompt.to_lowercase();

        let mut parts = vec![
            ComponentPart {
                id: "part-1".to_string(),
                name: "Page Header".to_string(),
                description: "Navigation and title section".to_string(),
                priority: 1,
            },
            ComponentPart {
                id: "part-2".to_string(),
                name: "Main Content Area".to_string(),
                description: format!("Primary content for: {user_prompt}"),
                priority: 2,
            },
        ];

        if keywords.contains("chart") || keywords.contains("graph") {
            parts.push(ComponentPart {
                id: "part-3".to_string(),
                name: "Data Visualization".to_string(),
                description: "Charts and graphs".to_string(),
                priority: 3,
            });
        }

        if keywords.contains("table") || keywords.contains("list") {
            parts.push(ComponentPart {
                id: "part-4".to_string(),
                name: "Data Table".to_string(),
                description: "Tabular data display".to_string(),
                priority: 4,
            });
        }

        if keywords.contains("form") || keywords.contains("input") {
            parts.push(ComponentPart {
                id: "part-5".to_string(),
                name: "Input Form".to_string(),
                description: "User input controls".to_string(),
                priority: 5,
            });
        }

        if parts.len() == 2 {
            parts.push(ComponentPart {
                id: "part-3".to_string(),
                name: "Interactive Controls".to_string(),
                description: "Buttons and actions".to_string(),
                priority: 3,
            });
        }

        Ok(BuildPlan { parts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plan() {
        let planner = UiPlanner {
            router: Arc::new(Router::new().unwrap()),
        };

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

    #[test]
    fn test_fallback_plan() {
        let planner = UiPlanner {
            router: Arc::new(Router::new().unwrap()),
        };

        let result = planner.fallback_plan("dashboard with charts");
        assert!(result.is_ok());

        let plan = result.unwrap();
        assert!(!plan.parts.is_empty());
        assert!(plan.parts.iter().any(|p| p.name.contains("Visualization")));
    }
}
