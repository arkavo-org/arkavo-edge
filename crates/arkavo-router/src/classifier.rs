use crate::decision::TokenEstimate;
use crate::{Error, Result};
use arkavo_llm::local::LocalProvider;
use arkavo_llm::{Message, Provider, Role};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TaskCategory {
    FrontendUI,
    BackendAPI,
    CodeSearch,
    SecurityScan,
    TestGeneration,
    Documentation,
    Refactoring,
    General,
}

impl TaskCategory {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "frontend_ui" | "frontend" | "ui" => Self::FrontendUI,
            "backend_api" | "backend" | "api" => Self::BackendAPI,
            "code_search" | "search" => Self::CodeSearch,
            "security_scan" | "security" => Self::SecurityScan,
            "test_generation" | "tests" | "testing" => Self::TestGeneration,
            "documentation" | "docs" => Self::Documentation,
            "refactoring" | "refactor" => Self::Refactoring,
            _ => Self::General,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::FrontendUI => "frontend_ui",
            Self::BackendAPI => "backend_api",
            Self::CodeSearch => "code_search",
            Self::SecurityScan => "security_scan",
            Self::TestGeneration => "test_generation",
            Self::Documentation => "documentation",
            Self::Refactoring => "refactoring",
            Self::General => "general",
        }
    }

    pub fn estimated_tokens(&self) -> TokenEstimate {
        match self {
            Self::FrontendUI => TokenEstimate {
                input: 500,
                output: 2000,
            },
            Self::BackendAPI => TokenEstimate {
                input: 400,
                output: 1500,
            },
            Self::CodeSearch => TokenEstimate {
                input: 200,
                output: 500,
            },
            Self::SecurityScan => TokenEstimate {
                input: 300,
                output: 800,
            },
            Self::TestGeneration => TokenEstimate {
                input: 500,
                output: 2500,
            },
            Self::Documentation => TokenEstimate {
                input: 300,
                output: 1000,
            },
            Self::Refactoring => TokenEstimate {
                input: 400,
                output: 1200,
            },
            Self::General => TokenEstimate {
                input: 300,
                output: 1000,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct Classification {
    pub category: TaskCategory,
    pub confidence: f32,
    pub reasoning: String,
}

pub struct TaskClassifier {
    provider: Arc<Mutex<LocalProvider>>,
}

impl TaskClassifier {
    pub async fn new() -> Result<Self> {
        let provider = LocalProvider::new(
            "gemma-3-270m-it".to_string(),
            Some("unsloth/gemma-3-270m-it-GGUF".to_string()),
        )
        .map_err(|e| Error::Provider(e))?;

        provider
            .initialize()
            .await
            .map_err(|e| Error::Provider(e))?;

        Ok(Self {
            provider: Arc::new(Mutex::new(provider)),
        })
    }

    pub async fn classify(&self, task_description: &str) -> Result<Classification> {
        if task_description.len() < 10 {
            return Ok(Classification {
                category: TaskCategory::General,
                confidence: 0.5,
                reasoning: "Task description too short for accurate classification".to_string(),
            });
        }

        let rule_based = self.try_rule_based_classification(task_description);
        if rule_based.confidence > 0.85 {
            return Ok(rule_based);
        }

        let llm_classification = self
            .classify_with_llm(task_description)
            .await
            .unwrap_or(rule_based);

        Ok(llm_classification)
    }

    fn try_rule_based_classification(&self, task: &str) -> Classification {
        let task_lower = task.to_lowercase();

        let (category, confidence, reasoning) = if task_lower.contains("react")
            || task_lower.contains("vue")
            || task_lower.contains("svelte")
            || task_lower.contains("tailwind")
            || task_lower.contains("component")
            || task_lower.contains("frontend")
            || task_lower.contains("ui")
        {
            (
                TaskCategory::FrontendUI,
                0.90,
                "Keywords match frontend development".to_string(),
            )
        } else if task_lower.contains("api")
            || task_lower.contains("endpoint")
            || task_lower.contains("backend")
            || task_lower.contains("database")
            || task_lower.contains("auth")
        {
            (
                TaskCategory::BackendAPI,
                0.85,
                "Keywords match backend development".to_string(),
            )
        } else if task_lower.contains("search")
            || task_lower.contains("find")
            || task_lower.contains("grep")
            || task_lower.contains("locate")
        {
            (
                TaskCategory::CodeSearch,
                0.80,
                "Keywords match code search".to_string(),
            )
        } else if task_lower.contains("security")
            || task_lower.contains("vulnerability")
            || task_lower.contains("audit")
            || task_lower.contains("scan")
        {
            (
                TaskCategory::SecurityScan,
                0.85,
                "Keywords match security analysis".to_string(),
            )
        } else if task_lower.contains("test")
            || task_lower.contains("jest")
            || task_lower.contains("pytest")
            || task_lower.contains("unit")
        {
            (
                TaskCategory::TestGeneration,
                0.80,
                "Keywords match test generation".to_string(),
            )
        } else if task_lower.contains("document")
            || task_lower.contains("readme")
            || task_lower.contains("comment")
            || task_lower.contains("docs")
        {
            (
                TaskCategory::Documentation,
                0.75,
                "Keywords match documentation".to_string(),
            )
        } else if task_lower.contains("refactor")
            || task_lower.contains("cleanup")
            || task_lower.contains("optimize")
        {
            (
                TaskCategory::Refactoring,
                0.75,
                "Keywords match refactoring".to_string(),
            )
        } else {
            (
                TaskCategory::General,
                0.50,
                "No strong keyword matches".to_string(),
            )
        };

        Classification {
            category,
            confidence,
            reasoning,
        }
    }

    async fn classify_with_llm(&self, task: &str) -> Result<Classification> {
        let prompt = self.build_classification_prompt(task);

        let messages = vec![Message {
            role: Role::User,
            content: prompt,
            images: None,
        }];

        let provider = self.provider.lock().await;
        let response = provider
            .complete(messages)
            .await
            .map_err(|e| Error::Classification(format!("LLM classification failed: {e}")))?;

        self.parse_classification_response(&response)
    }

    fn build_classification_prompt(&self, task: &str) -> String {
        format!(
            r#"Classify this coding task into ONE category:

Categories:
- frontend_ui: React/Vue/Svelte components, Tailwind CSS, web UI
- backend_api: REST APIs, authentication, databases, server logic
- code_search: Finding code, grep, repository search, AST analysis
- security_scan: Vulnerabilities, security audit, code scanning
- test_generation: Unit tests, integration tests, test suites
- documentation: README, API docs, comments, guides
- refactoring: Code cleanup, optimization, restructuring
- general: Other coding tasks

Task: {}

Reply with ONLY the category name and confidence (0-100):
Category: [category]
Confidence: [0-100]"#,
            task
        )
    }

    fn parse_classification_response(&self, response: &str) -> Result<Classification> {
        let lines: Vec<&str> = response.lines().collect();

        let mut category = TaskCategory::General;
        let mut confidence = 0.5;

        for line in lines {
            let line = line.trim();

            if line.starts_with("Category:") {
                let cat_str = line
                    .strip_prefix("Category:")
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
                category = TaskCategory::from_str(&cat_str);
            } else if line.starts_with("Confidence:") {
                if let Some(conf_str) = line.strip_prefix("Confidence:") {
                    if let Ok(conf) = conf_str.trim().parse::<f32>() {
                        confidence = (conf / 100.0).clamp(0.0, 1.0);
                    }
                }
            }
        }

        Ok(Classification {
            category,
            confidence,
            reasoning: format!("LLM classification: {}", category.as_str()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_category_from_str() {
        assert_eq!(
            TaskCategory::from_str("frontend_ui"),
            TaskCategory::FrontendUI
        );
        assert_eq!(TaskCategory::from_str("backend"), TaskCategory::BackendAPI);
        assert_eq!(TaskCategory::from_str("unknown"), TaskCategory::General);
    }

    #[test]
    fn test_token_estimation() {
        let estimate = TaskCategory::FrontendUI.estimated_tokens();
        assert!(estimate.output > estimate.input);
    }

    #[tokio::test]
    async fn test_rule_based_classification() {
        let classifier = TaskClassifier::new().await.unwrap();

        let classification =
            classifier.try_rule_based_classification("Create a React component with Tailwind CSS");

        assert_eq!(classification.category, TaskCategory::FrontendUI);
        assert!(classification.confidence > 0.8);
    }
}
