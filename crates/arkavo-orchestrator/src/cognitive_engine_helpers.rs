//! Inherent methods on `CognitiveEngine` for narrow concerns.
//!
//! GitHub progress comments, event logging, budget checks, repository
//! parsing, and the two early-/late-stage outcome paths (plan-invalid
//! handling, pull-request creation). Kept in a sibling file so the core
//! module stays focused on the orchestration loop itself.

use crate::agent_assignment::AgentAssignment;
use crate::cognitive_engine_core::CognitiveEngine;
use crate::cognitive_engine_outcome::{ExecutionOutcome, record_plan_invalid};
use crate::cognitive_engine_types::{ExecutionPlan, ExecutionResult};
use crate::error::{Error, Result};
use crate::plan_validator;
use arkavo_events::{Event, EventPayload};
use serde::Serialize;
use tracing::{error, info, warn};

impl CognitiveEngine {
    pub async fn handle_plan_invalid(
        &self,
        assignment: &AgentAssignment,
        plan: &ExecutionPlan,
        validation: &plan_validator::ValidationReport,
        outcome: ExecutionOutcome,
    ) -> Result<ExecutionResult> {
        let summary = validation.summary();
        error!(%summary, "plan failed contract validation");
        self.log_event("plan_validation_failed", &validation.violations)
            .await;
        self.post_progress(
            assignment,
            &format!("❌ Plan contract validation failed: {summary}"),
        )
        .await?;
        record_plan_invalid(&self.attempt_history, plan, &summary);
        if let Some(store) = &self.plan_store {
            let _ = store.mark_failed(plan.id, &summary).await;
        }
        Ok(ExecutionResult {
            success: false,
            steps_completed: 0,
            total_tokens_used: outcome.total_tokens,
            verification_results: Vec::new(),
            final_comment: format!("Plan failed contract validation: {summary}"),
        })
    }

    pub async fn create_pull_request_for(
        &self,
        assignment: &AgentAssignment,
        plan: &ExecutionPlan,
        outcome: &ExecutionOutcome,
        final_comment: &str,
    ) -> Result<()> {
        info!("Execution successful, creating pull request");
        match self
            .pr_creator
            .create_pull_request(
                assignment,
                plan,
                outcome.steps_completed,
                outcome.total_tokens,
            )
            .await
        {
            Ok(pr_url) => {
                info!(pr_url, "Pull request created successfully");
                let pr_comment = format!("🎉 Pull request created: {pr_url}\n\n{final_comment}");
                self.post_progress(assignment, &pr_comment).await?;
            }
            Err(e) => {
                warn!(error = %e, "Failed to create pull request");
                let fallback =
                    format!("{final_comment}\n\n⚠️ Note: Could not create PR automatically: {e}");
                self.post_progress(assignment, &fallback).await?;
            }
        }
        Ok(())
    }

    pub async fn check_budget(
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

    pub async fn post_progress(&self, assignment: &AgentAssignment, message: &str) -> Result<()> {
        let (owner, repo) = Self::parse_repository(&assignment.repository)?;
        self.github_ops
            .post_comment(&owner, &repo, assignment.issue_number, message)
            .await?;
        Ok(())
    }

    pub async fn log_event<T: Serialize>(&self, event_type: &str, data: &T) {
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

    pub fn parse_repository(full_name: &str) -> Result<(String, String)> {
        let parts: Vec<&str> = full_name.split('/').collect();
        if parts.len() != 2 {
            return Err(Error::Other(anyhow::anyhow!(
                "Invalid repository format: {full_name}"
            )));
        }
        Ok((parts[0].to_string(), parts[1].to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[test]
    fn parse_repository_splits_owner_and_name() {
        let (owner, repo) = CognitiveEngine::parse_repository("arkavo-org/arkavo-edge").unwrap();
        assert_eq!(owner, "arkavo-org");
        assert_eq!(repo, "arkavo-edge");
    }

    #[spec("ORCH-011")]
    #[test]
    fn parse_repository_rejects_unqualified() {
        assert!(CognitiveEngine::parse_repository("invalid").is_err());
    }
}
