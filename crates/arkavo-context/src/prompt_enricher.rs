use crate::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeContext {
    pub relevant_files: Vec<FileContext>,
    pub dependencies: Vec<String>,
    pub test_files: Vec<String>,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContext {
    pub path: String,
    pub content: String,
    pub relevance_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProblemStatement {
    pub title: String,
    pub description: String,
    pub hints: Vec<String>,
    pub repo_name: String,
    pub base_commit: String,
}

#[derive(Debug, Clone)]
pub enum PromptTemplate {
    CodeFix,
    FeatureAddition,
    Refactoring,
    BugAnalysis,
    Generic,
}

pub struct PromptEnricher {
    max_context_tokens: u32,
    include_dependencies: bool,
    include_tests: bool,
}

impl Default for PromptEnricher {
    fn default() -> Self {
        Self {
            max_context_tokens: 8000,
            include_dependencies: true,
            include_tests: true,
        }
    }
}

impl PromptEnricher {
    pub fn new(max_context_tokens: u32) -> Self {
        Self {
            max_context_tokens,
            include_dependencies: true,
            include_tests: true,
        }
    }

    pub fn with_dependencies(mut self, include: bool) -> Self {
        self.include_dependencies = include;
        self
    }

    pub fn with_tests(mut self, include: bool) -> Self {
        self.include_tests = include;
        self
    }

    pub fn enrich(
        &self,
        problem: &ProblemStatement,
        context: &CodeContext,
        template: PromptTemplate,
    ) -> Result<String> {
        let mut prompt = String::new();

        prompt.push_str(&self.format_header(problem));
        prompt.push_str(&self.format_problem(problem));

        if self.include_dependencies && !context.dependencies.is_empty() {
            prompt.push_str(&self.format_dependencies(&context.dependencies));
        }

        prompt.push_str(&self.format_files(&context.relevant_files));

        if self.include_tests && !context.test_files.is_empty() {
            prompt.push_str(&self.format_tests(&context.test_files));
        }

        prompt.push_str(&self.format_instructions(&template));
        prompt.push_str(&self.format_output_format());

        Ok(prompt)
    }

    fn format_header(&self, problem: &ProblemStatement) -> String {
        format!(
            "# Code Solution Task\n\n\
            Repository: {}\n\
            Base Commit: {}\n\n",
            problem.repo_name, problem.base_commit
        )
    }

    fn format_problem(&self, problem: &ProblemStatement) -> String {
        let mut section = format!(
            "## Problem Statement\n\n\
            **{}**\n\n\
            {}\n\n",
            problem.title, problem.description
        );

        if !problem.hints.is_empty() {
            section.push_str("### Hints\n\n");
            for hint in &problem.hints {
                section.push_str(&format!("- {}\n", hint));
            }
            section.push('\n');
        }

        section
    }

    fn format_dependencies(&self, dependencies: &[String]) -> String {
        let mut section = String::from("## Dependencies\n\n");
        section.push_str("The following dependencies are relevant to this issue:\n\n");

        for dep in dependencies {
            section.push_str(&format!("- {}\n", dep));
        }
        section.push('\n');

        section
    }

    fn format_files(&self, files: &[FileContext]) -> String {
        let mut section = String::from("## Relevant Code Files\n\n");

        if files.is_empty() {
            section.push_str("No relevant files identified. You may need to create new files.\n\n");
            return section;
        }

        section.push_str(&format!(
            "The following {} file(s) are most relevant to this issue:\n\n",
            files.len()
        ));

        for file in files {
            section.push_str(&format!(
                "### File: {}\n\
                Relevance: {:.2}\n\n\
                ```\n{}\n```\n\n",
                file.path, file.relevance_score, file.content
            ));
        }

        section
    }

    fn format_tests(&self, test_files: &[String]) -> String {
        let mut section = String::from("## Test Files\n\n");
        section.push_str("The following test files should pass after your changes:\n\n");

        for test_file in test_files {
            section.push_str(&format!("- {}\n", test_file));
        }
        section.push('\n');

        section
    }

    fn format_instructions(&self, template: &PromptTemplate) -> String {
        let instructions = match template {
            PromptTemplate::CodeFix => {
                "## Instructions\n\n\
                1. Analyze the problem statement and relevant code files\n\
                2. Identify the root cause of the issue\n\
                3. Implement a minimal, focused fix\n\
                4. Ensure your changes don't break existing functionality\n\
                5. Make sure all tests pass\n\n"
            }
            PromptTemplate::FeatureAddition => {
                "## Instructions\n\n\
                1. Understand the requested feature from the problem statement\n\
                2. Review the existing code structure and patterns\n\
                3. Implement the feature following existing conventions\n\
                4. Add appropriate error handling\n\
                5. Ensure backward compatibility\n\n"
            }
            PromptTemplate::Refactoring => {
                "## Instructions\n\n\
                1. Analyze the current code structure\n\
                2. Identify areas for improvement\n\
                3. Refactor while preserving functionality\n\
                4. Maintain or improve test coverage\n\
                5. Ensure no behavioral changes\n\n"
            }
            PromptTemplate::BugAnalysis => {
                "## Instructions\n\n\
                1. Reproduce the bug based on the description\n\
                2. Trace the code execution to find the root cause\n\
                3. Develop a fix that addresses the root cause\n\
                4. Add test cases to prevent regression\n\
                5. Verify the fix with existing tests\n\n"
            }
            PromptTemplate::Generic => {
                "## Instructions\n\n\
                1. Carefully read and understand the problem statement\n\
                2. Review all provided code context\n\
                3. Implement a solution that addresses the requirements\n\
                4. Follow existing code patterns and conventions\n\
                5. Ensure all tests pass\n\n"
            }
        };

        instructions.to_string()
    }

    fn format_output_format(&self) -> String {
        "## Output Format\n\n\
        Provide your solution as a unified git diff that can be applied with `git apply`. \
        \n\n\
        **CRITICAL REQUIREMENTS**:\n\
        - Use EXACT file paths from the \"Relevant Code Files\" section above\n\
        - Do NOT create new files or reference files not shown in the context\n\
        - Include at least 3 lines of context before and after each change\n\
        - Ensure line numbers match the actual file content provided\n\
        - Use @@ (two at-signs) for hunk headers, not @@@\n\n\
        Format requirements:\n\
        - File headers: diff --git a/path/to/file b/path/to/file\n\
        - Hunk headers: @@ -oldstart,oldcount +newstart,newcount @@\n\
        - Context lines: start with space\n\
        - Removed lines: start with -\n\
        - Added lines: start with +\n\n\
        Example format:\n\
        ```diff\n\
        diff --git a/src/example.py b/src/example.py\n\
        index abc123..def456 100644\n\
        --- a/src/example.py\n\
        +++ b/src/example.py\n\
        @@ -10,7 +10,7 @@\n\
         context line\n\
         context line\n\
         context line\n\
        -old line\n\
        +new line\n\
         context line\n\
         context line\n\
         context line\n\
        ```\n\n\
        **IMPORTANT**: \n\
        - Output ONLY the diff, nothing else\n\
        - Do not include explanations before or after the diff\n\
        - Do not wrap the diff in markdown code blocks\n\
        - Start directly with 'diff --git'\n"
            .to_string()
    }

    pub fn estimate_tokens(text: &str) -> u32 {
        let words = text.split_whitespace().count();
        ((words as f64) / 0.75) as u32
    }

    pub fn truncate_context(
        &self,
        mut context: CodeContext,
        max_tokens: Option<u32>,
    ) -> CodeContext {
        let limit = max_tokens.unwrap_or(self.max_context_tokens);

        let mut current_tokens = 0u32;
        let mut truncated_files = Vec::new();

        for file in context.relevant_files.drain(..) {
            let file_tokens = Self::estimate_tokens(&file.content);

            if current_tokens + file_tokens <= limit {
                current_tokens += file_tokens;
                truncated_files.push(file);
            } else {
                let remaining = limit.saturating_sub(current_tokens);
                if remaining > 100 {
                    let chars_to_take = (remaining as f64 * 0.75 * 4.0) as usize;
                    let truncated_content: String =
                        file.content.chars().take(chars_to_take).collect();

                    truncated_files.push(FileContext {
                        path: file.path,
                        content: format!("{}...\n[truncated]", truncated_content),
                        relevance_score: file.relevance_score,
                    });

                    current_tokens = limit;
                }
                break;
            }
        }

        context.relevant_files = truncated_files;
        context.total_tokens = current_tokens;
        context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_problem() -> ProblemStatement {
        ProblemStatement {
            title: "Fix null pointer exception".to_string(),
            description: "The function crashes when input is None".to_string(),
            hints: vec!["Check for None values".to_string()],
            repo_name: "test/repo".to_string(),
            base_commit: "abc123".to_string(),
        }
    }

    fn create_test_context() -> CodeContext {
        CodeContext {
            relevant_files: vec![FileContext {
                path: "src/main.py".to_string(),
                content: "def process(data):\n    return data.value".to_string(),
                relevance_score: 0.95,
            }],
            dependencies: vec!["requests==2.28.0".to_string()],
            test_files: vec!["tests/test_main.py".to_string()],
            total_tokens: 100,
        }
    }

    #[test]
    fn test_enrich_basic() {
        let enricher = PromptEnricher::default();
        let problem = create_test_problem();
        let context = create_test_context();

        let result = enricher.enrich(&problem, &context, PromptTemplate::CodeFix);
        assert!(result.is_ok());

        let prompt = result.unwrap();
        assert!(prompt.contains("test/repo"));
        assert!(prompt.contains("Fix null pointer exception"));
        assert!(prompt.contains("src/main.py"));
    }

    #[test]
    fn test_truncate_context() {
        let enricher = PromptEnricher::new(50);

        let large_content = "word ".repeat(1000);
        let context = CodeContext {
            relevant_files: vec![FileContext {
                path: "large.py".to_string(),
                content: large_content,
                relevance_score: 0.9,
            }],
            dependencies: vec![],
            test_files: vec![],
            total_tokens: 1000,
        };

        let truncated = enricher.truncate_context(context, Some(50));
        assert!(truncated.total_tokens <= 50);
    }

    #[test]
    fn test_without_dependencies() {
        let enricher = PromptEnricher::default().with_dependencies(false);
        let problem = create_test_problem();
        let context = create_test_context();

        let prompt = enricher
            .enrich(&problem, &context, PromptTemplate::Generic)
            .unwrap();
        assert!(!prompt.contains("Dependencies"));
    }

    #[test]
    fn test_token_estimation() {
        let text = "This is a test with ten words in total";
        let tokens = PromptEnricher::estimate_tokens(text);
        assert!(tokens > 0);
        assert!(tokens < 50);
    }
}
