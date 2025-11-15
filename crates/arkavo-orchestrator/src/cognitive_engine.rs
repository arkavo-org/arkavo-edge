use crate::agent_assignment::AgentAssignment;
use crate::error::{Error, Result};
use crate::github_operations::GitHubOperations;
use arkavo_budget::{BudgetTracker, TokenCost, cost::TokenUsage};
use arkavo_events::{Event, EventPayload, EventWriter};
use arkavo_llm::{Message as LlmMessage, Provider, Role};
use arkavo_mcp_tools::ToolRegistry;
use arkavo_protocol::mcp::McpClient;
use arkavo_router::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub issue_number: u64,
    pub repository: String,
    pub steps: Vec<PlanStep>,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub step_number: usize,
    pub description: String,
    pub commands: Vec<String>,
    pub verification: Vec<VerificationCheck>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationCheck {
    TestsPassing,
    LinterClean,
    BuildSuccessful,
    FileConstraint { max_lines: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub success: bool,
    pub steps_completed: usize,
    pub total_tokens_used: u32,
    pub verification_results: Vec<VerificationResult>,
    pub final_comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub check: VerificationCheck,
    pub passed: bool,
    pub details: String,
}

pub struct CognitiveEngine {
    mcp_client: Arc<McpClient>,
    budget_tracker: Arc<BudgetTracker>,
    event_writer: Arc<EventWriter>,
    github_ops: Arc<GitHubOperations>,
    router: Arc<Router>,
    tool_registry: Arc<ToolRegistry>,
    session_id: String,
    sequence: std::sync::atomic::AtomicU64,
}

impl CognitiveEngine {
    pub fn new(
        mcp_client: Arc<McpClient>,
        budget_tracker: Arc<BudgetTracker>,
        event_writer: Arc<EventWriter>,
        github_ops: Arc<GitHubOperations>,
        router: Arc<Router>,
        tool_registry: Arc<ToolRegistry>,
        session_id: String,
    ) -> Self {
        Self {
            mcp_client,
            budget_tracker,
            event_writer,
            github_ops,
            router,
            tool_registry,
            session_id,
            sequence: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub async fn execute(&self, assignment: AgentAssignment) -> Result<ExecutionResult> {
        info!(
            issue = assignment.issue_number,
            strategy = ?assignment.routing_decision.strategy,
            "Starting cognitive execution cycle"
        );

        let budget = assignment
            .routing_decision
            .analysis
            .complexity
            .token_budget();

        let mut total_tokens = 0u32;
        let mut steps_completed = 0usize;
        let mut verification_results = Vec::new();

        let plan = self.plan(&assignment).await?;
        total_tokens += plan.estimated_tokens;

        self.log_event("plan_generated", &plan).await;
        self.post_progress(&assignment, "📋 Execution plan generated")
            .await?;

        for (idx, step) in plan.steps.iter().enumerate() {
            if !self.check_budget(total_tokens, budget, &assignment).await? {
                break;
            }

            info!(step = idx + 1, "Executing plan step");
            self.post_progress(
                &assignment,
                &format!(
                    "⚙️ Step {}/{}: {}",
                    idx + 1,
                    plan.steps.len(),
                    step.description
                ),
            )
            .await?;

            match self.do_step(step).await {
                Ok(tokens) => {
                    total_tokens += tokens;
                    steps_completed += 1;

                    let check_results = self.check(step).await?;
                    verification_results.extend(check_results.clone());

                    if check_results.iter().any(|r| !r.passed) {
                        warn!(step = idx + 1, "Verification failed, attempting adjustment");
                        self.post_progress(
                            &assignment,
                            &format!("⚠️ Step {} verification failed, adjusting...", idx + 1),
                        )
                        .await?;

                        if let Some(adjusted_step) = self.adjust(step, &check_results).await? {
                            match self.do_step(&adjusted_step).await {
                                Ok(adj_tokens) => {
                                    total_tokens += adj_tokens;
                                    let recheck_results = self.check(&adjusted_step).await?;
                                    verification_results.extend(recheck_results.clone());

                                    if recheck_results.iter().any(|r| !r.passed) {
                                        error!(step = idx + 1, "Adjustment failed verification");
                                        break;
                                    }
                                }
                                Err(e) => {
                                    error!(step = idx + 1, error = %e, "Adjustment execution failed");
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(step = idx + 1, error = %e, "Step execution failed");
                    self.post_progress(&assignment, &format!("❌ Step {} failed: {}", idx + 1, e))
                        .await?;
                    break;
                }
            }
        }

        let success =
            steps_completed == plan.steps.len() && verification_results.iter().all(|r| r.passed);

        let final_comment = if success {
            format!(
                "✅ Issue completed successfully!\n\n\
                - Steps completed: {}/{}\n\
                - Tokens used: {}/{}\n\
                - All verifications passed",
                steps_completed,
                plan.steps.len(),
                total_tokens,
                budget
            )
        } else {
            format!(
                "⚠️ Issue execution incomplete\n\n\
                - Steps completed: {}/{}\n\
                - Tokens used: {}/{}\n\
                - Verification failures: {}",
                steps_completed,
                plan.steps.len(),
                total_tokens,
                budget,
                verification_results.iter().filter(|r| !r.passed).count()
            )
        };

        self.post_progress(&assignment, &final_comment).await?;

        if success {
            info!("Execution successful, creating pull request");
            match self.create_pull_request(&assignment, &plan, steps_completed, total_tokens).await {
                Ok(pr_url) => {
                    info!(pr_url, "Pull request created successfully");
                    let pr_comment = format!(
                        "🎉 Pull request created: {}\n\n{}",
                        pr_url, final_comment
                    );
                    self.post_progress(&assignment, &pr_comment).await?;
                }
                Err(e) => {
                    warn!(error = %e, "Failed to create pull request");
                    let fallback = format!(
                        "{}\n\n⚠️ Note: Could not create PR automatically: {}",
                        final_comment, e
                    );
                    self.post_progress(&assignment, &fallback).await?;
                }
            }
        }

        Ok(ExecutionResult {
            success,
            steps_completed,
            total_tokens_used: total_tokens,
            verification_results,
            final_comment,
        })
    }

    async fn plan(&self, assignment: &AgentAssignment) -> Result<ExecutionPlan> {
        debug!("Generating execution plan");

        let planning_prompt = format!(
            "Generate a detailed execution plan for this GitHub issue:\n\n\
            Title: {}\n\n\
            Description: {}\n\n\
            Type: {:?}\n\
            Complexity: {:?}\n\
            Technologies: {:?}\n\n\
            Return a structured plan with 3-5 concrete steps. For each step:\n\
            1. Brief description\n\
            2. Specific commands to execute (e.g., cargo test, cargo build, git commands)\n\
            3. Verification checks (tests, linter, build success)\n\n\
            Format each step as:\n\
            STEP N: [description]\n\
            COMMANDS: [comma-separated commands]\n\
            VERIFY: [comma-separated: tests, linter, build, or file_constraint_400]\n\
            CONFIDENCE: [0.0-1.0]",
            assignment.issue_title,
            assignment.issue_body,
            assignment.routing_decision.analysis.issue_type,
            assignment.routing_decision.analysis.complexity,
            assignment.routing_decision.analysis.technologies
        );

        let decision = self
            .router
            .route(&planning_prompt)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Routing failed: {e}")))?;

        info!(
            model = ?decision.recommended_model,
            estimated_cost = decision.estimated_cost_usd,
            "Planning with selected model"
        );

        let planning_provider: Arc<dyn Provider> = match decision.recommended_model {
            arkavo_router::ModelChoice::LocalGemma4B
            | arkavo_router::ModelChoice::LocalGemma12B => {
                return Err(Error::Other(anyhow::anyhow!(
                    "Local models not yet supported for planning. Set GEMINI_API_KEY for remote planning."
                )));
            }
            _ => {
                if let Some(gemini) = self.router.get_planning_provider() {
                    Arc::new(gemini)
                } else {
                    return Err(Error::Other(anyhow::anyhow!(
                        "Planning model not available. Set GEMINI_API_KEY for remote planning."
                    )));
                }
            }
        };

        let messages = vec![LlmMessage {
            role: Role::User,
            content: planning_prompt.clone(),
            images: None,
        }];

        let response = planning_provider
            .complete(messages)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Planning LLM call failed: {e}")))?;

        let steps = self.parse_plan_from_response(&response)?;

        let estimated_input_tokens = planning_prompt.len() as u32 / 4;
        let estimated_output_tokens = response.len() as u32 / 4;
        let total_tokens = estimated_input_tokens + estimated_output_tokens;

        let model_name = match decision.recommended_model {
            arkavo_router::ModelChoice::GeminiFlash => "gemini-1.5-flash",
            arkavo_router::ModelChoice::GeminiPro => "gemini-1.5-pro",
            _ => "unknown",
        };

        let usage = TokenUsage::new(estimated_input_tokens, estimated_output_tokens);
        let cost = TokenCost::from_dollars(decision.estimated_cost_usd);

        if let Err(e) = self
            .budget_tracker
            .record_spending(
                "github-orchestrator".to_string(),
                "gemini".to_string(),
                model_name.to_string(),
                usage,
                cost,
            )
            .await
        {
            warn!(error = %e, "Failed to record budget usage for planning");
        }

        Ok(ExecutionPlan {
            issue_number: assignment.issue_number,
            repository: assignment.repository.clone(),
            steps,
            estimated_tokens: total_tokens,
        })
    }

    fn parse_plan_from_response(&self, response: &str) -> Result<Vec<PlanStep>> {
        let mut steps = Vec::new();
        let lines: Vec<&str> = response.lines().collect();
        let mut current_step = None;

        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with("STEP ") {
                if let Some(step) = current_step.take() {
                    steps.push(step);
                }
                let desc = line
                    .split_once(": ")
                    .map(|(_, d)| d)
                    .unwrap_or(line)
                    .to_string();
                current_step = Some(PlanStep {
                    step_number: steps.len() + 1,
                    description: desc,
                    commands: Vec::new(),
                    verification: Vec::new(),
                    confidence: 0.8,
                });
            } else if line.starts_with("COMMANDS:")
                && let Some(step) = current_step.as_mut()
            {
                let cmds = line
                    .strip_prefix("COMMANDS:")
                    .unwrap_or("")
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                step.commands = cmds;
            } else if line.starts_with("VERIFY:")
                && let Some(step) = current_step.as_mut()
            {
                let checks_str = line.strip_prefix("VERIFY:").unwrap_or("");
                let checks: Vec<VerificationCheck> = checks_str
                    .split(',')
                    .filter_map(|s| {
                        let check = s.trim().to_lowercase();
                        if check.contains("test") {
                            Some(VerificationCheck::TestsPassing)
                        } else if check.contains("lint") {
                            Some(VerificationCheck::LinterClean)
                        } else if check.contains("build") {
                            Some(VerificationCheck::BuildSuccessful)
                        } else if check.contains("file_constraint") {
                            Some(VerificationCheck::FileConstraint { max_lines: 400 })
                        } else {
                            None
                        }
                    })
                    .collect();
                step.verification = checks;
            } else if line.starts_with("CONFIDENCE:")
                && let Some(step) = current_step.as_mut()
                && let Some(conf_str) = line.strip_prefix("CONFIDENCE:")
                && let Ok(conf) = conf_str.trim().parse::<f32>()
            {
                step.confidence = conf;
            }
        }

        if let Some(step) = current_step {
            steps.push(step);
        }

        if steps.is_empty() {
            warn!("No steps parsed from plan, using default");
            steps.push(PlanStep {
                step_number: 1,
                description: "Analyze and fix the issue".to_string(),
                commands: vec!["echo 'Analyzing issue'".to_string()],
                verification: vec![VerificationCheck::BuildSuccessful],
                confidence: 0.5,
            });
        }

        Ok(steps)
    }

    async fn do_step(&self, step: &PlanStep) -> Result<u32> {
        debug!(step = step.step_number, "Executing step");

        let mut tokens_used = 0u32;

        for command in &step.commands {
            debug!(command, "Executing command");

            let task_prompt = format!(
                "Execute this command as part of fixing a GitHub issue:\n\
                Step {}: {}\n\
                Command: {}\n\n\
                Use the available tools to complete this task. \
                You have access to filesystem, git, and github tools.",
                step.step_number, step.description, command
            );

            let messages = vec![LlmMessage {
                role: Role::User,
                content: task_prompt.clone(),
                images: None,
            }];

            let response = self
                .router
                .route_with_quality_gate(
                    command,
                    messages,
                    Some(&self.tool_registry),
                    3,
                )
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!("Command execution failed: {e}")))?;

            info!(
                step = step.step_number,
                tool_calls = response.tool_calls.len(),
                "Command executed with {} tool calls",
                response.tool_calls.len()
            );

            let estimated_tokens = (task_prompt.len() + response.content.len()) / 4;
            tokens_used += estimated_tokens as u32;

            if !response.tool_calls.is_empty() {
                debug!(
                    "Tool calls executed: {:?}",
                    response
                        .tool_calls
                        .iter()
                        .map(|tc| &tc.tool_name)
                        .collect::<Vec<_>>()
                );
            }
        }

        Ok(tokens_used)
    }

    async fn check(&self, step: &PlanStep) -> Result<Vec<VerificationResult>> {
        debug!(step = step.step_number, "Running verification checks");

        let mut results = Vec::new();

        for check in &step.verification {
            let result = match check {
                VerificationCheck::TestsPassing => self.verify_tests().await,
                VerificationCheck::LinterClean => self.verify_linter().await,
                VerificationCheck::BuildSuccessful => self.verify_build().await,
                VerificationCheck::FileConstraint { max_lines } => {
                    self.verify_file_constraints(*max_lines).await
                }
            };

            results.push(result);
        }

        Ok(results)
    }

    async fn adjust(
        &self,
        step: &PlanStep,
        failures: &[VerificationResult],
    ) -> Result<Option<PlanStep>> {
        debug!(step = step.step_number, "Generating adjustment plan");

        if failures.is_empty() {
            return Ok(None);
        }

        let failure_summary: Vec<String> = failures
            .iter()
            .filter(|r| !r.passed)
            .map(|r| format!("- {:?}: {}", r.check, r.details))
            .collect();

        if failure_summary.is_empty() {
            return Ok(None);
        }

        let adjustment_prompt = format!(
            "The following step failed verification:\n\n\
            Step {}: {}\n\
            Commands executed: {}\n\n\
            Verification failures:\n{}\n\n\
            Generate an adjusted plan to fix these failures. Provide:\n\
            1. Updated description\n\
            2. New commands to execute (comma-separated)\n\
            3. Same verification checks\n\n\
            Format:\n\
            STEP {}: [updated description]\n\
            COMMANDS: [comma-separated commands]\n\
            VERIFY: [same as before]\n\
            CONFIDENCE: [0.0-1.0]",
            step.step_number,
            step.description,
            step.commands.join(", "),
            failure_summary.join("\n"),
            step.step_number
        );

        let decision = self
            .router
            .route(&adjustment_prompt)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Routing failed: {e}")))?;

        info!(
            model = ?decision.recommended_model,
            "Using {:?} for adjustment generation",
            decision.recommended_model
        );

        let provider: Arc<dyn Provider> = if let Some(gemini) = self.router.get_planning_provider()
        {
            Arc::new(gemini)
        } else {
            return Err(Error::Other(anyhow::anyhow!(
                "Adjustment requires Gemini. Set GEMINI_API_KEY."
            )));
        };

        let messages = vec![LlmMessage {
            role: Role::User,
            content: adjustment_prompt.clone(),
            images: None,
        }];

        let response = provider
            .complete(messages)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Adjustment LLM call failed: {e}")))?;

        let estimated_input_tokens = adjustment_prompt.len() as u32 / 4;
        let estimated_output_tokens = response.len() as u32 / 4;

        let model_name = match decision.recommended_model {
            arkavo_router::ModelChoice::GeminiFlash => "gemini-1.5-flash",
            arkavo_router::ModelChoice::GeminiPro => "gemini-1.5-pro",
            _ => "unknown",
        };

        let usage = TokenUsage::new(estimated_input_tokens, estimated_output_tokens);
        let cost = TokenCost::from_dollars(decision.estimated_cost_usd);

        if let Err(e) = self
            .budget_tracker
            .record_spending(
                "github-orchestrator".to_string(),
                "gemini".to_string(),
                model_name.to_string(),
                usage,
                cost,
            )
            .await
        {
            warn!(error = %e, "Failed to record budget usage for adjustment");
        }

        let adjusted_steps = self.parse_plan_from_response(&response)?;

        if let Some(adjusted_step) = adjusted_steps.first() {
            info!(
                step = step.step_number,
                "Generated adjustment with {} commands",
                adjusted_step.commands.len()
            );
            Ok(Some(adjusted_step.clone()))
        } else {
            warn!(step = step.step_number, "Failed to parse adjustment");
            Ok(None)
        }
    }

    async fn check_budget(
        &self,
        used: u32,
        total: u32,
        assignment: &AgentAssignment,
    ) -> Result<bool> {
        let percentage = (used as f32 / total as f32) * 100.0;

        if percentage >= 50.0 {
            let message = format!("⚠️ Budget warning: {percentage:.0}% used ({used}/{total})");
            warn!("{}", message);
            self.post_progress(assignment, &message).await?;
        }

        if percentage >= 100.0 {
            error!("Budget exhausted");
            self.post_progress(assignment, "❌ Budget exhausted, halting execution")
                .await?;
            return Ok(false);
        }

        Ok(true)
    }

    async fn post_progress(&self, assignment: &AgentAssignment, message: &str) -> Result<()> {
        let (owner, repo) = Self::parse_repository(&assignment.repository)?;

        self.github_ops
            .post_comment(&owner, &repo, assignment.issue_number, message)
            .await
    }

    async fn log_event<T: Serialize>(&self, event_type: &str, data: &T) {
        let seq = self
            .sequence
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let payload = EventPayload::ReasoningStep {
            step_type: event_type.to_string(),
            description: serde_json::to_string(data).unwrap_or_default(),
            metadata: Some(serde_json::Value::Object(serde_json::Map::new())),
        };

        let event = Event::new(
            self.session_id.clone(),
            seq,
            "github-orchestrator".to_string(),
            payload,
        );

        if let Err(e) = self.event_writer.write(event).await {
            error!(error = %e, "Failed to log event");
        }
    }

    async fn verify_tests(&self) -> VerificationResult {
        debug!("Running tests");

        let output = tokio::process::Command::new("cargo")
            .arg("test")
            .arg("--all")
            .arg("--")
            .arg("--nocapture")
            .output()
            .await;

        match output {
            Ok(result) => {
                let passed = result.status.success();
                let stderr = String::from_utf8_lossy(&result.stderr);
                let stdout = String::from_utf8_lossy(&result.stdout);

                let details = if passed {
                    "All tests passed".to_string()
                } else {
                    format!(
                        "Tests failed:\n{}{}",
                        stdout.lines().take(10).collect::<Vec<_>>().join("\n"),
                        stderr.lines().take(10).collect::<Vec<_>>().join("\n")
                    )
                };

                VerificationResult {
                    check: VerificationCheck::TestsPassing,
                    passed,
                    details,
                }
            }
            Err(e) => VerificationResult {
                check: VerificationCheck::TestsPassing,
                passed: false,
                details: format!("Failed to run tests: {e}"),
            },
        }
    }

    async fn verify_linter(&self) -> VerificationResult {
        debug!("Running linter");

        let output = tokio::process::Command::new("cargo")
            .arg("clippy")
            .arg("--all-targets")
            .arg("--")
            .arg("-D")
            .arg("warnings")
            .output()
            .await;

        match output {
            Ok(result) => {
                let passed = result.status.success();
                let stderr = String::from_utf8_lossy(&result.stderr);
                let stdout = String::from_utf8_lossy(&result.stdout);

                let details = if passed {
                    "Linter checks passed".to_string()
                } else {
                    format!(
                        "Linter warnings/errors:\n{}{}",
                        stdout.lines().take(10).collect::<Vec<_>>().join("\n"),
                        stderr.lines().take(10).collect::<Vec<_>>().join("\n")
                    )
                };

                VerificationResult {
                    check: VerificationCheck::LinterClean,
                    passed,
                    details,
                }
            }
            Err(e) => VerificationResult {
                check: VerificationCheck::LinterClean,
                passed: false,
                details: format!("Failed to run linter: {e}"),
            },
        }
    }

    async fn verify_build(&self) -> VerificationResult {
        debug!("Running build");

        let output = tokio::process::Command::new("cargo")
            .arg("build")
            .arg("--all")
            .output()
            .await;

        match output {
            Ok(result) => {
                let passed = result.status.success();
                let stderr = String::from_utf8_lossy(&result.stderr);
                let stdout = String::from_utf8_lossy(&result.stdout);

                let details = if passed {
                    "Build successful".to_string()
                } else {
                    format!(
                        "Build failed:\n{}{}",
                        stdout.lines().take(10).collect::<Vec<_>>().join("\n"),
                        stderr.lines().take(10).collect::<Vec<_>>().join("\n")
                    )
                };

                VerificationResult {
                    check: VerificationCheck::BuildSuccessful,
                    passed,
                    details,
                }
            }
            Err(e) => VerificationResult {
                check: VerificationCheck::BuildSuccessful,
                passed: false,
                details: format!("Failed to run build: {e}"),
            },
        }
    }

    async fn verify_file_constraints(&self, max_lines: usize) -> VerificationResult {
        debug!(max_lines, "Checking file size constraints");

        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "find . -name '*.rs' -type f ! -path '*/target/*' ! -path '*/vendor/*' -exec wc -l {{}} \\; | awk '$1 > {max_lines} {{print}}'"
            ))
            .output()
            .await;

        match output {
            Ok(result) => {
                let stdout = String::from_utf8_lossy(&result.stdout);
                let violations: Vec<&str> = stdout.lines().collect();

                let passed = violations.is_empty();
                let details = if passed {
                    format!("All Rust files under {max_lines} lines")
                } else {
                    format!(
                        "{} files exceed {max_lines} lines:\n{}",
                        violations.len(),
                        violations
                            .iter()
                            .take(5)
                            .copied()
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                };

                VerificationResult {
                    check: VerificationCheck::FileConstraint { max_lines },
                    passed,
                    details,
                }
            }
            Err(e) => VerificationResult {
                check: VerificationCheck::FileConstraint { max_lines },
                passed: false,
                details: format!("Failed to check file constraints: {e}"),
            },
        }
    }

    fn parse_repository(full_name: &str) -> Result<(String, String)> {
        let parts: Vec<&str> = full_name.split('/').collect();
        if parts.len() != 2 {
            return Err(Error::Other(anyhow::anyhow!(
                "Invalid repository format: {full_name}"
            )));
        }
        Ok((parts[0].to_string(), parts[1].to_string()))
    }

    async fn create_pull_request(
        &self,
        assignment: &AgentAssignment,
        plan: &ExecutionPlan,
        steps_completed: usize,
        tokens_used: u32,
    ) -> Result<String> {
        info!("Creating pull request via GitHub operations");

        let issue_number = assignment.issue_number;
        let pr_title = format!("Fix issue #{}: {}", issue_number, assignment.issue_title);

        let pr_body = format!(
            "## Summary\n\nAutonomously resolved issue #{}\n\n\
            ## What Changed\n\n{}\n\n\
            ## Execution Details\n\n\
            - Steps completed: {}/{}\n\
            - Tokens used: {}\n\
            - All verifications passed ✓\n\n\
            ## Plan Executed\n\n{}\n\n\
            ---\n\n\
            🤖 Autonomous execution by Arkavo Edge\n\
            Resolves #{}\n",
            issue_number,
            plan.steps
                .iter()
                .enumerate()
                .map(|(i, step)| format!("{}. {}", i + 1, step.description))
                .collect::<Vec<_>>()
                .join("\n"),
            steps_completed,
            plan.steps.len(),
            tokens_used,
            plan.steps
                .iter()
                .enumerate()
                .map(|(i, step)| format!(
                    "### Step {}: {}\n\nCommands:\n{}\n",
                    i + 1,
                    step.description,
                    step.commands
                        .iter()
                        .map(|c| format!("- `{}`", c))
                        .collect::<Vec<_>>()
                        .join("\n")
                ))
                .collect::<Vec<_>>()
                .join("\n"),
            issue_number
        );

        // For MVP: Use gh CLI directly to create PR
        // TODO: Replace with MCP tool call in production
        let output = tokio::process::Command::new("gh")
            .args(&[
                "pr",
                "create",
                "--title",
                &pr_title,
                "--body",
                &pr_body,
                "--base",
                "main",
            ])
            .output()
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to execute gh command: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Other(anyhow::anyhow!(
                "gh pr create failed: {stderr}"
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let pr_url = stdout.lines().last().unwrap_or("PR created").trim();

        Ok(pr_url.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repository() {
        let (owner, repo) = CognitiveEngine::parse_repository("arkavo-org/arkavo-edge").unwrap();
        assert_eq!(owner, "arkavo-org");
        assert_eq!(repo, "arkavo-edge");
    }

    #[test]
    fn test_parse_repository_invalid() {
        let result = CognitiveEngine::parse_repository("invalid");
        assert!(result.is_err());
    }
}
