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
    router: Option<Arc<Router>>,
}

impl UiPlanner {
    pub async fn new() -> Result<Self> {
        let router = match Router::new().await {
            Ok(r) => Some(Arc::new(r)),
            Err(e) => {
                eprintln!("UiPlanner: Router initialization failed (using fallback only): {e}");
                None
            }
        };

        Ok(Self { router })
    }

    pub async fn plan(&self, user_prompt: &str) -> Result<BuildPlan> {
        let _planning_prompt = self.build_planning_prompt(user_prompt);

        if let Some(router) = &self.router {
            let _decision = router.route(user_prompt).await.ok();
        }

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
        let mut parts = Vec::new();
        let mut part_id = 1;

        // Always start with header
        parts.push(ComponentPart {
            id: format!("part-{part_id}"),
            name: "Page Header".to_string(),
            description: "Top navigation bar with logo and menu".to_string(),
            priority: part_id,
        });
        part_id += 1;

        // Domain-specific components
        if keywords.contains("bank") || keywords.contains("account") {
            parts.push(ComponentPart {
                id: format!("part-{part_id}"),
                name: "Account Summary Card".to_string(),
                description: "Display account balance, account number, and quick stats".to_string(),
                priority: part_id,
            });
            part_id += 1;

            parts.push(ComponentPart {
                id: format!("part-{part_id}"),
                name: "Recent Transactions".to_string(),
                description: "Table showing recent account activity with dates and amounts".to_string(),
                priority: part_id,
            });
            part_id += 1;
        }

        if keywords.contains("portfolio") || keywords.contains("stock") {
            parts.push(ComponentPart {
                id: format!("part-{part_id}"),
                name: "Portfolio Overview".to_string(),
                description: "Summary of total portfolio value, gains/losses, and allocation".to_string(),
                priority: part_id,
            });
            part_id += 1;

            parts.push(ComponentPart {
                id: format!("part-{part_id}"),
                name: "Holdings Table".to_string(),
                description: "List of stocks/assets with current prices, quantities, and values".to_string(),
                priority: part_id,
            });
            part_id += 1;

            parts.push(ComponentPart {
                id: format!("part-{part_id}"),
                name: "Performance Chart".to_string(),
                description: "Line chart showing portfolio value over time".to_string(),
                priority: part_id,
            });
            part_id += 1;
        }

        // Generic patterns
        if keywords.contains("chart") || keywords.contains("graph") {
            if !parts.iter().any(|p| p.name.contains("Chart")) {
                parts.push(ComponentPart {
                    id: format!("part-{part_id}"),
                    name: "Data Visualization".to_string(),
                    description: "Interactive charts and graphs".to_string(),
                    priority: part_id,
                });
                part_id += 1;
            }
        }

        if keywords.contains("table") || keywords.contains("list") {
            if !parts.iter().any(|p| p.name.contains("Table") || p.name.contains("Transactions")) {
                parts.push(ComponentPart {
                    id: format!("part-{part_id}"),
                    name: "Data Table".to_string(),
                    description: "Sortable and filterable data table".to_string(),
                    priority: part_id,
                });
                part_id += 1;
            }
        }

        if keywords.contains("form") || keywords.contains("input") {
            parts.push(ComponentPart {
                id: format!("part-{part_id}"),
                name: "Input Form".to_string(),
                description: "User input controls and validation".to_string(),
                priority: part_id,
            });
            part_id += 1;
        }

        if keywords.contains("dashboard") {
            parts.push(ComponentPart {
                id: format!("part-{part_id}"),
                name: "Dashboard Widgets".to_string(),
                description: "Grid of summary cards and metrics".to_string(),
                priority: part_id,
            });
            part_id += 1;
        }

        // Add action buttons if we have a reasonable number of parts
        if parts.len() >= 3 {
            parts.push(ComponentPart {
                id: format!("part-{part_id}"),
                name: "Action Buttons".to_string(),
                description: "Primary actions like Transfer, Buy/Sell, Export".to_string(),
                priority: part_id,
            });
            part_id += 1;
        }

        // Footer for completeness
        parts.push(ComponentPart {
            id: format!("part-{part_id}"),
            name: "Page Footer".to_string(),
            description: "Footer with links, disclaimers, and copyright".to_string(),
            priority: part_id,
        });

        println!(
            "UiPlanner: Generated {} parts for prompt: '{}'",
            parts.len(),
            user_prompt
        );
        for part in &parts {
            println!("  - {} ({}): {}", part.name, part.id, part.description);
        }

        Ok(BuildPlan { parts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plan() {
        let planner = UiPlanner { router: None };

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
        let planner = UiPlanner { router: None };

        let result = planner.fallback_plan("dashboard with charts");
        assert!(result.is_ok());

        let plan = result.unwrap();
        assert!(!plan.parts.is_empty());
        assert!(plan.parts.iter().any(|p| p.name.contains("Visualization")));
    }
}
