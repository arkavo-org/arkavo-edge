use crate::{Error, Result};
use arkavo_context::{CodeContext, FileContext, ProblemStatement, PromptEnricher, PromptTemplate};
use arkavo_llm::Message;
use arkavo_mcp_tools::ToolRegistry;
use arkavo_router::{IssueType, JudgmentResult, Router};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverConfig {
    pub max_context_tokens: u32,
    pub max_quality_retries: u8,
    pub include_dependencies: bool,
    pub include_tests: bool,
    pub search_depth: usize,
    pub min_patch_tokens: usize,
    pub enable_escalation: bool,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: 131_072, // 128K default (supports most cloud models)
            max_quality_retries: 3,
            include_dependencies: true,
            include_tests: true,
            search_depth: 5,
            min_patch_tokens: 100,
            enable_escalation: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverMetrics {
    pub total_time_ms: u64,
    pub context_build_time_ms: u64,
    pub solution_gen_time_ms: u64,
    pub quality_retries: u8,
    pub total_tokens: u32,
    pub files_analyzed: usize,
    pub escalated_to_cloud: bool,
    pub local_attempt_tokens: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct SolverResult {
    pub solution: String,
    pub judgment: JudgmentResult,
    pub metrics: SolverMetrics,
}

pub struct CodeSolver {
    router: Arc<Router>,
    enricher: PromptEnricher,
    config: SolverConfig,
}

impl CodeSolver {
    pub async fn new(router: Arc<Router>, config: Option<SolverConfig>) -> Result<Self> {
        let config = config.unwrap_or_default();
        let enricher = PromptEnricher::new(config.max_context_tokens)
            .with_dependencies(config.include_dependencies)
            .with_tests(config.include_tests);

        Ok(Self {
            router,
            enricher,
            config,
        })
    }

    pub async fn solve(
        &self,
        repo_path: &Path,
        problem: &ProblemStatement,
        template: PromptTemplate,
    ) -> Result<SolverResult> {
        let start_time = Instant::now();
        let mut metrics = SolverMetrics {
            total_time_ms: 0,
            context_build_time_ms: 0,
            solution_gen_time_ms: 0,
            quality_retries: 0,
            total_tokens: 0,
            files_analyzed: 0,
            escalated_to_cloud: false,
            local_attempt_tokens: None,
        };

        info!(
            "Starting CodeSolver for problem: {} in repo: {}",
            problem.title, problem.repo_name
        );

        let context_start = Instant::now();
        let context = self.build_context(repo_path, problem).await?;
        metrics.context_build_time_ms = context_start.elapsed().as_millis() as u64;
        metrics.files_analyzed = context.relevant_files.len();
        metrics.total_tokens = context.total_tokens;

        debug!(
            "Context built: {} files, {} tokens in {}ms",
            metrics.files_analyzed, metrics.total_tokens, metrics.context_build_time_ms
        );

        let enriched_prompt = self.enricher.enrich(problem, &context, template)?;

        debug!("Enriched prompt length: {} chars", enriched_prompt.len());

        let tool_registry = ToolRegistry::default();

        let messages = vec![Message::user(enriched_prompt.clone())];

        let solution_start = Instant::now();

        // Always try local model first (cost optimization goal)
        info!("Attempting code generation with local model");
        let response = self
            .router
            .route_with_quality_gate(
                "code_generation: Generate a unified git diff patch to solve the problem",
                messages.clone(),
                Some(&tool_registry),
                self.config.max_quality_retries,
            )
            .await
            .map_err(|e| Error::External(format!("Router error: {e}")))?;

        let response_tokens = response.content.split_whitespace().count();
        metrics.local_attempt_tokens = Some(response_tokens);

        // Validate patch syntax (not just quality gate)
        let patch_valid = Self::validate_patch_syntax(&response.content);

        // Escalate if: (1) patch invalid OR (2) too short
        let should_escalate = self.config.enable_escalation
            && (!patch_valid || response_tokens < self.config.min_patch_tokens)
            && self.router.check_connectivity().await;

        let final_response = if should_escalate {
            if !patch_valid {
                warn!("Local model generated invalid patch syntax, escalating to cloud");
            } else {
                warn!(
                    "Local model generated only {} tokens (min: {}), escalating to cloud",
                    response_tokens, self.config.min_patch_tokens
                );
            }
            metrics.escalated_to_cloud = true;

            info!("Escalating to Gemini for complete patch generation");
            self.router
                .route_with_quality_gate(
                    "code_generation: Generate a complete unified git diff patch with full context",
                    messages,
                    Some(&tool_registry),
                    self.config.max_quality_retries,
                )
                .await
                .map_err(|e| Error::External(format!("Cloud escalation error: {e}")))?
        } else {
            if response_tokens < self.config.min_patch_tokens {
                warn!(
                    "Local model generated only {} tokens but escalation disabled",
                    response_tokens
                );
            }
            response
        };

        metrics.solution_gen_time_ms = solution_start.elapsed().as_millis() as u64;

        let solution = final_response.content;

        let judgment = JudgmentResult {
            passed: true,
            reason: Some("Quality gate passed".to_string()),
            issue_type: IssueType::None,
        };

        metrics.total_time_ms = start_time.elapsed().as_millis() as u64;

        info!(
            "CodeSolver completed in {}ms (context: {}ms, solution: {}ms)",
            metrics.total_time_ms, metrics.context_build_time_ms, metrics.solution_gen_time_ms
        );

        Ok(SolverResult {
            solution,
            judgment,
            metrics,
        })
    }

    /// Validate git patch syntax
    /// Checks for: diff headers, file markers, hunk headers, line prefixes
    fn validate_patch_syntax(content: &str) -> bool {
        let lines: Vec<&str> = content.lines().collect();

        // Must have at least one diff header
        let has_diff = lines.iter().any(|l| l.starts_with("diff --git"));
        if !has_diff {
            return false;
        }

        // Must have file markers (--- and +++)
        let has_from = lines.iter().any(|l| l.starts_with("---"));
        let has_to = lines.iter().any(|l| l.starts_with("+++"));
        if !has_from || !has_to {
            return false;
        }

        // Must have at least one hunk header
        let has_hunk = lines.iter().any(|l| l.starts_with("@@"));
        if !has_hunk {
            return false;
        }

        // Check for malformed hunks (common local model error)
        for line in &lines {
            // Valid patch lines start with: ' ', '+', '-', '@', 'd', 'i', '---', '+++'
            if !line.is_empty() {
                let first_char = line.chars().next().unwrap();
                let is_valid = matches!(first_char, ' ' | '+' | '-' | '@' | 'd' | 'i' | '\\')
                    || line.starts_with("---")
                    || line.starts_with("+++")
                    || line.starts_with("new file")
                    || line.starts_with("index ")
                    || line.starts_with("Binary files");

                if !is_valid {
                    debug!("Invalid patch line detected: {}", line);
                    return false;
                }
            }
        }

        true
    }

    async fn build_context(
        &self,
        repo_path: &Path,
        problem: &ProblemStatement,
    ) -> Result<CodeContext> {
        let relevant_files = self.find_relevant_files(repo_path, problem).await?;

        let dependencies = self.extract_file_dependencies(repo_path);

        let test_files = self.find_test_files(repo_path);

        let total_tokens: u32 = relevant_files
            .iter()
            .map(|f| PromptEnricher::estimate_tokens(&f.content))
            .sum();

        let mut context = CodeContext {
            relevant_files,
            dependencies,
            test_files,
            total_tokens,
        };

        if context.total_tokens > self.config.max_context_tokens {
            warn!(
                "Context exceeds token limit ({} > {}), truncating",
                context.total_tokens, self.config.max_context_tokens
            );
            context = self
                .enricher
                .truncate_context(context, Some(self.config.max_context_tokens));
        }

        Ok(context)
    }

    async fn find_relevant_files(
        &self,
        repo_path: &Path,
        problem: &ProblemStatement,
    ) -> Result<Vec<FileContext>> {
        let search_terms = self.extract_search_terms(problem);

        debug!("Searching for terms: {:?}", search_terms);

        let mut relevant_files = Vec::new();

        for term in search_terms.iter().take(self.config.search_depth) {
            if let Ok(matches) = self.search_files(repo_path, term).await {
                relevant_files.extend(matches);
            }
        }

        relevant_files.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        relevant_files.truncate(10);

        if relevant_files.is_empty() {
            warn!("No relevant files found, using fallback strategy");
            relevant_files = self.fallback_file_selection(repo_path).await?;
        }

        Ok(relevant_files)
    }

    fn extract_search_terms(&self, problem: &ProblemStatement) -> Vec<String> {
        let mut terms = Vec::new();

        let words: Vec<&str> = problem
            .title
            .split_whitespace()
            .chain(problem.description.split_whitespace())
            .collect();

        for word in words {
            let clean_word = word
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
                .to_string();

            if clean_word.len() > 3 && !is_common_word(&clean_word) {
                terms.push(clean_word);
            }
        }

        terms.dedup();
        terms
    }

    async fn search_files(&self, repo_path: &Path, term: &str) -> Result<Vec<FileContext>> {
        let mut matches = Vec::new();
        self.search_files_recursive(repo_path, term, &mut matches, 0)?;
        Ok(matches)
    }

    fn search_files_recursive(
        &self,
        dir: &Path,
        term: &str,
        matches: &mut Vec<FileContext>,
        depth: usize,
    ) -> Result<()> {
        if depth > 10 {
            return Ok(());
        }

        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                if let Some(name) = path.file_name().and_then(|n| n.to_str())
                    && (name.starts_with('.')
                        || name == "node_modules"
                        || name == "target"
                        || name == "__pycache__")
                {
                    continue;
                }

                if path.is_dir() {
                    self.search_files_recursive(&path, term, matches, depth + 1)?;
                } else if path.is_file()
                    && let Some(ext) = path.extension()
                    && matches!(ext.to_str(), Some("py" | "rs" | "js" | "ts" | "go"))
                    && let Ok(content) = std::fs::read_to_string(&path)
                    && content.contains(term)
                {
                    let relevance = self.calculate_relevance(&content, term);

                    matches.push(FileContext {
                        path: path.display().to_string(),
                        content,
                        relevance_score: relevance,
                    });
                }
            }
        }

        Ok(())
    }

    fn calculate_relevance(&self, content: &str, term: &str) -> f32 {
        let occurrences = content.matches(term).count();
        let lines = content.lines().count();

        if lines == 0 {
            return 0.0;
        }

        let density = (occurrences as f32) / (lines as f32);

        density.min(1.0)
    }

    async fn fallback_file_selection(&self, repo_path: &Path) -> Result<Vec<FileContext>> {
        let mut files = Vec::new();

        if let Ok(entries) = std::fs::read_dir(repo_path) {
            for entry in entries.flatten().take(3) {
                let path = entry.path();

                if path.is_file()
                    && let Some(ext) = path.extension()
                    && matches!(ext.to_str(), Some("py" | "rs" | "js" | "ts"))
                    && let Ok(content) = std::fs::read_to_string(&path)
                {
                    files.push(FileContext {
                        path: path.display().to_string(),
                        content,
                        relevance_score: 0.5,
                    });
                }
            }
        }

        Ok(files)
    }

    fn extract_file_dependencies(&self, _repo_path: &Path) -> Vec<String> {
        Vec::new()
    }

    fn find_test_files(&self, repo_path: &Path) -> Vec<String> {
        let mut test_files = Vec::new();

        if let Ok(entries) = std::fs::read_dir(repo_path) {
            for entry in entries.flatten().take(5) {
                let path = entry.path();
                let path_str = path.display().to_string();

                if path.is_file() && (path_str.contains("test") || path_str.contains("spec")) {
                    test_files.push(path_str);
                }
            }
        }

        test_files
    }
}

fn is_common_word(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "the" | "and" | "for" | "with" | "this" | "that" | "from" | "have" | "been"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_problem() -> ProblemStatement {
        ProblemStatement {
            title: "Fix authentication bug".to_string(),
            description: "User authentication fails when password is empty".to_string(),
            hints: vec!["Check password validation".to_string()],
            repo_name: "test/repo".to_string(),
            base_commit: "abc123".to_string(),
        }
    }

    #[tokio::test]
    async fn test_extract_search_terms() {
        let router = Arc::new(Router::new().await.unwrap());
        let solver = CodeSolver {
            router,
            enricher: PromptEnricher::default(),
            config: SolverConfig::default(),
        };

        let problem = create_test_problem();
        let terms = solver.extract_search_terms(&problem);

        assert!(!terms.is_empty());
        assert!(terms.contains(&"authentication".to_string()));
        assert!(terms.contains(&"password".to_string()));
    }

    #[tokio::test]
    async fn test_calculate_relevance() {
        let router = Arc::new(Router::new().await.unwrap());
        let solver = CodeSolver {
            router,
            enricher: PromptEnricher::default(),
            config: SolverConfig::default(),
        };

        let content = "auth\nauth\nauth\nother\n";
        let relevance = solver.calculate_relevance(content, "auth");

        assert!(relevance > 0.0);
        assert!(relevance <= 1.0);
    }
}
