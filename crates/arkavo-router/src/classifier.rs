use crate::decision::TokenEstimate;
use crate::{Error, Result};
use arkavo_llm::Message;
use serde::{Deserialize, Serialize};

/// Match a keyword as a whole word (not as a substring).
/// "ui" matches " ui " or "ui " at start, but NOT "build" or "blueprint".
fn contains_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|w| w == word)
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llm::{Provider, Role};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::sync::Arc;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use tokio::sync::Mutex;

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llm::LlamaCppProvider;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TaskCategory {
    FrontendUI,
    BackendAPI,
    CodeSearch,
    SecurityScan,
    TestGeneration,
    CodeReview,
    Documentation,
    Refactoring,
    CodeGeneration,
    VisionAnalysis,
    General,
}

impl TaskCategory {
    pub fn from_string(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "frontend_ui" | "frontend" | "ui" => Self::FrontendUI,
            "backend_api" | "backend" | "api" => Self::BackendAPI,
            "code_search" | "search" => Self::CodeSearch,
            "security_scan" | "security" => Self::SecurityScan,
            "test_generation" | "tests" | "testing" => Self::TestGeneration,
            "code_review" | "review" => Self::CodeReview,
            "documentation" | "docs" => Self::Documentation,
            "refactoring" | "refactor" => Self::Refactoring,
            "code_generation" | "codegen" | "patch" | "diff" | "generate" => Self::CodeGeneration,
            "vision_analysis" | "vision" | "screenshot" | "image" => Self::VisionAnalysis,
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
            Self::CodeReview => "code_review",
            Self::Documentation => "documentation",
            Self::Refactoring => "refactoring",
            Self::CodeGeneration => "code_generation",
            Self::VisionAnalysis => "vision_analysis",
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
            Self::CodeReview => TokenEstimate {
                input: 400,
                output: 1500,
            },
            Self::Documentation => TokenEstimate {
                input: 300,
                output: 1000,
            },
            Self::Refactoring => TokenEstimate {
                input: 400,
                output: 1200,
            },
            Self::CodeGeneration => TokenEstimate {
                input: 800,
                output: 3000,
            },
            Self::VisionAnalysis => TokenEstimate {
                input: 2000,
                output: 3000,
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
    /// Whether this task appears to be multi-step (triggers architect mode consideration)
    pub is_multi_step: bool,
    /// Estimated number of subtasks if this is a complex task
    pub estimated_subtasks: u8,
}

impl Classification {
    /// Create a new classification with default complexity fields
    pub fn new(category: TaskCategory, confidence: f32, reasoning: String) -> Self {
        Self {
            category,
            confidence,
            reasoning,
            is_multi_step: false,
            estimated_subtasks: 1,
        }
    }

    /// Create a classification with complexity analysis
    pub fn with_complexity(
        category: TaskCategory,
        confidence: f32,
        reasoning: String,
        task: &str,
    ) -> Self {
        let (is_multi_step, estimated_subtasks) = Self::detect_complexity(task);
        Self {
            category,
            confidence,
            reasoning,
            is_multi_step,
            estimated_subtasks,
        }
    }

    /// Detect complexity indicators in a task description
    fn detect_complexity(task: &str) -> (bool, u8) {
        let task_lower = task.to_lowercase();

        // Multi-step indicators
        let multi_step_patterns = [
            "and then",
            "after that",
            "first",
            "second",
            "third",
            "finally",
            "next",
            ". then",
            "step 1",
            "step 2",
        ];

        let mut step_count = 0u8;
        for pattern in multi_step_patterns {
            if task_lower.contains(pattern) {
                step_count = step_count.saturating_add(1);
            }
        }

        // Complexity keywords
        let complexity_keywords = [
            "refactor",
            "migration",
            "redesign",
            "multi-step",
            "comprehensive",
            "complete",
            "full implementation",
            "end-to-end",
            "entire",
        ];

        let has_complexity_keyword = complexity_keywords.iter().any(|k| task_lower.contains(k));

        // Count action verbs
        let action_verbs = [
            "create",
            "build",
            "implement",
            "add",
            "update",
            "fix",
            "write",
            "generate",
            "test",
            "refactor",
            "migrate",
            "deploy",
        ];
        let verb_count = action_verbs
            .iter()
            .filter(|v| task_lower.contains(*v))
            .count();

        // Sentence count
        let sentence_count = task.matches(". ").count() + task.matches("? ").count() + 1;

        // Determine if multi-step
        let is_multi_step = step_count >= 2
            || has_complexity_keyword
            || (verb_count >= 3 && sentence_count >= 3)
            || task.len() > 300;

        // Estimate subtasks
        let estimated_subtasks = if is_multi_step {
            let base = step_count.max(1);
            let verb_bonus = (verb_count / 2).min(3) as u8;
            (base + verb_bonus).clamp(2, 10)
        } else {
            1
        };

        (is_multi_step, estimated_subtasks)
    }
}

/// Rule-based task classification from description keywords (no LLM required)
pub fn classify_task_keywords(description: &str) -> TaskCategory {
    let lower = description.to_lowercase();

    if lower.contains("react")
        || lower.contains("vue")
        || lower.contains("svelte")
        || lower.contains("tailwind")
        || lower.contains("component")
        || lower.contains("frontend")
        || contains_word(&lower, "ui")
    {
        TaskCategory::FrontendUI
    } else if contains_word(&lower, "api")
        || lower.contains("endpoint")
        || lower.contains("backend")
        || lower.contains("database")
        || contains_word(&lower, "auth")
    {
        TaskCategory::BackendAPI
    } else if lower.contains("search")
        || lower.contains("find")
        || lower.contains("grep")
        || lower.contains("locate")
    {
        TaskCategory::CodeSearch
    } else if lower.contains("security")
        || lower.contains("vulnerability")
        || lower.contains("audit")
        || lower.contains("scan")
    {
        TaskCategory::SecurityScan
    } else if lower.contains("test")
        || lower.contains("jest")
        || lower.contains("pytest")
        || lower.contains("unit")
    {
        TaskCategory::TestGeneration
    } else if lower.contains("review")
        || lower.contains("code quality")
        || lower.contains("anti-pattern")
        || lower.contains("complexity")
        || lower.contains("error handling")
    {
        TaskCategory::CodeReview
    } else if lower.contains("document")
        || lower.contains("readme")
        || lower.contains("comment")
        || lower.contains("docs")
    {
        TaskCategory::Documentation
    } else if lower.contains("refactor") || lower.contains("cleanup") || lower.contains("optimize")
    {
        TaskCategory::Refactoring
    } else if lower.contains("screenshot")
        || lower.contains("image")
        || lower.contains("vision")
        || lower.contains("analyze ui")
        || lower.contains("ui from")
    {
        TaskCategory::VisionAnalysis
    } else {
        TaskCategory::General
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub struct TaskClassifier {
    provider: Arc<Mutex<LlamaCppProvider>>,
}

#[cfg(all(
    not(all(feature = "llama-cpp", not(target_env = "musl"))),
    feature = "llama-cpp"
))]
pub struct TaskClassifier;

#[cfg(not(any(
    all(feature = "llama-cpp", not(target_env = "musl")),
    feature = "llama-cpp"
)))]
pub struct TaskClassifier {
    _phantom: std::marker::PhantomData<()>,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl TaskClassifier {
    pub async fn new() -> Result<Self> {
        // Use model discovery to find any available model (prefers Qwen3/Ministral)
        let model_path = crate::model_discovery::find_any_gguf()
            .await
            .ok_or_else(|| {
                Error::Classification(format!(
                    "No local GGUF models found. Download with: {}",
                    crate::decision::ModelChoice::LocalQwen3
                        .download_hint()
                        .unwrap_or_default()
                ))
            })?;

        let model_name = model_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("local-classifier")
            .to_string();

        let provider = LlamaCppProvider::new(model_name, model_path.to_string_lossy().to_string())
            .map_err(Error::Provider)?;

        Ok(Self {
            provider: Arc::new(Mutex::new(provider)),
        })
    }

    /// Create a classifier that uses only rule-based classification (no local model required)
    pub async fn new_fallback() -> Result<Self> {
        // Try to create with a model, but if none available, return error
        // The caller should use classify_rule_based() directly for fallback mode
        Self::new().await
    }

    /// Rule-based classification without LLM (for fallback/first-run mode)
    pub fn classify_rule_based(&self, task_description: &str) -> Classification {
        self.try_rule_based_classification(task_description)
    }

    pub async fn classify(&self, task_description: &str) -> Result<Classification> {
        if task_description.len() < 10 {
            return Ok(Classification::new(
                TaskCategory::General,
                0.5,
                "Task description too short for accurate classification".to_string(),
            ));
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

    /// Complete an LLM prompt using the local provider
    pub async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        self.complete_with_options(messages, None).await
    }

    /// Complete an LLM prompt with optional max_tokens limit
    pub async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        max_tokens: Option<usize>,
    ) -> Result<String> {
        let provider = self.provider.lock().await;
        provider
            .complete_with_options(messages, max_tokens)
            .await
            .map_err(|e| Error::Classification(format!("LLM completion failed: {e}")))
    }

    fn try_rule_based_classification(&self, task: &str) -> Classification {
        let task_lower = task.to_lowercase();

        let (category, confidence, reasoning) = if task_lower.contains("react")
            || task_lower.contains("vue")
            || task_lower.contains("svelte")
            || task_lower.contains("tailwind")
            || task_lower.contains("component")
            || task_lower.contains("frontend")
            || contains_word(&task_lower, "ui")
        {
            (
                TaskCategory::FrontendUI,
                0.90,
                "Keywords match frontend development".to_string(),
            )
        } else if contains_word(&task_lower, "api")
            || task_lower.contains("endpoint")
            || task_lower.contains("backend")
            || task_lower.contains("database")
            || contains_word(&task_lower, "auth")
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
        } else if task_lower.contains("review")
            || task_lower.contains("code quality")
            || task_lower.contains("anti-pattern")
            || task_lower.contains("complexity")
            || task_lower.contains("error handling")
        {
            (
                TaskCategory::CodeReview,
                0.80,
                "Keywords match code review".to_string(),
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
        } else if task_lower.contains("screenshot")
            || task_lower.contains("image")
            || task_lower.contains("vision")
            || task_lower.contains("analyze ui")
            || task_lower.contains("ui from")
        {
            (
                TaskCategory::VisionAnalysis,
                0.90,
                "Keywords match vision/screenshot analysis".to_string(),
            )
        } else {
            (
                TaskCategory::General,
                0.50,
                "No strong keyword matches".to_string(),
            )
        };

        Classification::with_complexity(category, confidence, reasoning, task)
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
- code_review: Code review, anti-patterns, error handling, code quality
- documentation: README, API docs, comments, guides
- refactoring: Code cleanup, optimization, restructuring
- general: Other coding tasks

Task: {task}

Reply with ONLY the category name and confidence (0-100):
Category: [category]
Confidence: [0-100]"#
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
                category = TaskCategory::from_string(&cat_str);
            } else if line.starts_with("Confidence:")
                && let Some(conf_str) = line.strip_prefix("Confidence:")
                && let Ok(conf) = conf_str.trim().parse::<f32>()
            {
                confidence = (conf / 100.0).clamp(0.0, 1.0);
            }
        }

        Ok(Classification::new(
            category,
            confidence,
            format!("LLM classification: {}", category.as_str()),
        ))
    }
}

#[cfg(all(
    not(all(feature = "llama-cpp", not(target_env = "musl"))),
    feature = "llama-cpp"
))]
impl TaskClassifier {
    pub async fn new() -> Result<Self> {
        Ok(Self)
    }

    pub async fn classify(&self, task_description: &str) -> Result<Classification> {
        if task_description.len() < 10 {
            return Ok(Classification::new(
                TaskCategory::General,
                0.5,
                "Task description too short for accurate classification".to_string(),
            ));
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

    pub async fn complete(&self, _messages: Vec<Message>) -> Result<String> {
        Err(Error::Classification(
            "LocalProvider (llama-cpp on MUSL) is not supported in this build".to_string(),
        ))
    }

    pub async fn complete_with_options(
        &self,
        _messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String> {
        Err(Error::Classification(
            "LocalProvider (llama-cpp on MUSL) is not supported in this build".to_string(),
        ))
    }

    fn try_rule_based_classification(&self, task: &str) -> Classification {
        let task_lower = task.to_lowercase();

        let (category, confidence, reasoning) = if task_lower.contains("react")
            || task_lower.contains("vue")
            || task_lower.contains("svelte")
            || task_lower.contains("tailwind")
            || task_lower.contains("component")
            || task_lower.contains("frontend")
            || contains_word(&task_lower, "ui")
        {
            (
                TaskCategory::FrontendUI,
                0.90,
                "Keywords match frontend development".to_string(),
            )
        } else if contains_word(&task_lower, "api")
            || task_lower.contains("endpoint")
            || task_lower.contains("backend")
            || task_lower.contains("database")
            || contains_word(&task_lower, "auth")
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
        } else if task_lower.contains("review")
            || task_lower.contains("code quality")
            || task_lower.contains("anti-pattern")
            || task_lower.contains("complexity")
            || task_lower.contains("error handling")
        {
            (
                TaskCategory::CodeReview,
                0.80,
                "Keywords match code review".to_string(),
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
        } else if task_lower.contains("screenshot")
            || task_lower.contains("image")
            || task_lower.contains("vision")
            || task_lower.contains("analyze ui")
            || task_lower.contains("ui from")
        {
            (
                TaskCategory::VisionAnalysis,
                0.90,
                "Keywords match vision/screenshot analysis".to_string(),
            )
        } else {
            (
                TaskCategory::General,
                0.50,
                "No strong keyword matches".to_string(),
            )
        };

        Classification::with_complexity(category, confidence, reasoning, task)
    }

    async fn classify_with_llm(&self, _task: &str) -> Result<Classification> {
        // LLM classification not available on musl target
        Err(Error::Classification(
            "LLM classification not available on musl target".to_string(),
        ))
    }
}

#[cfg(not(any(
    all(feature = "llama-cpp", not(target_env = "musl")),
    feature = "llama-cpp"
)))]
impl TaskClassifier {
    pub async fn new() -> Result<Self> {
        Err(Error::Classification(
            "TaskClassifier requires llama-cpp or llama-cpp feature".to_string(),
        ))
    }

    pub async fn classify(&self, _task_description: &str) -> Result<Classification> {
        Err(Error::Classification(
            "TaskClassifier requires llama-cpp or llama-cpp feature".to_string(),
        ))
    }

    pub async fn complete(&self, _messages: Vec<Message>) -> Result<String> {
        Err(Error::Classification(
            "TaskClassifier requires llama-cpp or llama-cpp feature".to_string(),
        ))
    }

    pub async fn complete_with_options(
        &self,
        _messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String> {
        Err(Error::Classification(
            "TaskClassifier requires llama-cpp or llama-cpp feature".to_string(),
        ))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_task_category_from_str() {
        assert_eq!(
            TaskCategory::from_string("frontend_ui"),
            TaskCategory::FrontendUI
        );
        assert_eq!(
            TaskCategory::from_string("backend"),
            TaskCategory::BackendAPI
        );
        assert_eq!(TaskCategory::from_string("unknown"), TaskCategory::General);
    }

    #[test]
    fn test_token_estimation() {
        let estimate = TaskCategory::FrontendUI.estimated_tokens();
        assert!(estimate.output > estimate.input);
    }

    #[test]
    fn test_classify_task_keywords() {
        assert_eq!(
            classify_task_keywords("Build a React component with Tailwind CSS"),
            TaskCategory::FrontendUI
        );
        assert_eq!(
            classify_task_keywords("Create a REST API endpoint for authentication"),
            TaskCategory::BackendAPI
        );
        assert_eq!(
            classify_task_keywords("Scan for security vulnerabilities"),
            TaskCategory::SecurityScan
        );
        assert_eq!(
            classify_task_keywords("Generate unit tests for the parser"),
            TaskCategory::TestGeneration
        );
        assert_eq!(
            classify_task_keywords("Review this error handling pattern"),
            TaskCategory::CodeReview
        );
        assert_eq!(
            classify_task_keywords("Write documentation for the module"),
            TaskCategory::Documentation
        );
        assert_eq!(
            classify_task_keywords("Refactor the data pipeline"),
            TaskCategory::Refactoring
        );
        assert_eq!(classify_task_keywords("Hello world"), TaskCategory::General);
        // "ui" as substring in "build"/"blueprint" must NOT match frontend_ui
        assert_eq!(
            classify_task_keywords("Build a bed blueprint"),
            TaskCategory::General
        );
        // "ui" as whole word should match
        assert_eq!(
            classify_task_keywords("Create a new UI layout"),
            TaskCategory::FrontendUI
        );
    }

    #[tokio::test]
    #[cfg(any(
        all(feature = "llama-cpp", not(target_env = "musl")),
        feature = "llama-cpp"
    ))]
    async fn test_rule_based_classification() {
        let classifier = TaskClassifier::new().await;
        if classifier.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        let classifier = classifier.unwrap();

        let classification =
            classifier.try_rule_based_classification("Create a React component with Tailwind CSS");

        assert_eq!(classification.category, TaskCategory::FrontendUI);
        assert!(classification.confidence > 0.8);
    }
}
