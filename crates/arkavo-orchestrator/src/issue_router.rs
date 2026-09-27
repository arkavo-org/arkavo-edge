use crate::issue_analyzer::{Complexity, IssueAnalysis, IssueType};
use crate::types::IssueEvent;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStrategy {
    AutoExecute,
    PlanFirst,
    OrchestratorConsultation,
    HumanApprovalRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub strategy: ExecutionStrategy,
    pub analysis: IssueAnalysis,
    pub rationale: String,
    pub should_notify_human: bool,
    pub priority: Priority,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

pub struct IssueRouter;

impl IssueRouter {
    pub fn route(event: &IssueEvent) -> RoutingDecision {
        let analysis = crate::issue_analyzer::IssueAnalyzer::analyze(event);

        let (strategy, rationale) = Self::determine_strategy(&analysis, event);
        let should_notify_human = Self::should_notify_human(&analysis, &strategy);
        let priority = Self::determine_priority(&analysis, event);

        RoutingDecision {
            strategy,
            analysis,
            rationale,
            should_notify_human,
            priority,
        }
    }

    fn determine_strategy(
        analysis: &IssueAnalysis,
        event: &IssueEvent,
    ) -> (ExecutionStrategy, String) {
        match analysis.complexity {
            Complexity::Trivial => {
                if Self::is_safe_auto_execute(&analysis.issue_type) {
                    (
                        ExecutionStrategy::AutoExecute,
                        "Trivial complexity with safe issue type - auto-executing".to_string(),
                    )
                } else {
                    (
                        ExecutionStrategy::PlanFirst,
                        "Trivial but requires verification before execution".to_string(),
                    )
                }
            }
            Complexity::Simple => {
                if Self::requires_testing(&analysis.issue_type) {
                    (
                        ExecutionStrategy::PlanFirst,
                        "Simple issue but requires test verification before execution".to_string(),
                    )
                } else if analysis.issue_type == IssueType::Bug {
                    (
                        ExecutionStrategy::PlanFirst,
                        "Bug fix requires analysis and testing plan".to_string(),
                    )
                } else {
                    (
                        ExecutionStrategy::AutoExecute,
                        "Simple issue with low risk - auto-executing with verification".to_string(),
                    )
                }
            }
            Complexity::Moderate => {
                if Self::is_breaking_change(event) {
                    (
                        ExecutionStrategy::HumanApprovalRequired,
                        "Moderate complexity with potential breaking changes".to_string(),
                    )
                } else if analysis.technologies.len() > 3 {
                    (
                        ExecutionStrategy::OrchestratorConsultation,
                        format!(
                            "Moderate complexity spanning {} technologies - requires orchestrator coordination",
                            analysis.technologies.len()
                        ),
                    )
                } else {
                    (
                        ExecutionStrategy::PlanFirst,
                        "Moderate complexity requires detailed implementation plan".to_string(),
                    )
                }
            }
            Complexity::Complex => {
                if Self::is_architectural_change(event) {
                    (
                        ExecutionStrategy::HumanApprovalRequired,
                        "Complex architectural change requires human review".to_string(),
                    )
                } else if analysis.required_capabilities.len() > 5 {
                    (
                        ExecutionStrategy::OrchestratorConsultation,
                        format!(
                            "Complex issue requiring {} capabilities - multi-agent coordination needed",
                            analysis.required_capabilities.len()
                        ),
                    )
                } else {
                    (
                        ExecutionStrategy::OrchestratorConsultation,
                        "Complex issue requires orchestrator planning and agent coordination"
                            .to_string(),
                    )
                }
            }
        }
    }

    fn is_safe_auto_execute(issue_type: &IssueType) -> bool {
        matches!(issue_type, IssueType::Documentation | IssueType::Question)
    }

    fn requires_testing(issue_type: &IssueType) -> bool {
        matches!(
            issue_type,
            IssueType::Bug | IssueType::Feature | IssueType::Security | IssueType::Performance
        )
    }

    fn is_breaking_change(event: &IssueEvent) -> bool {
        let title_lower = event.issue.title.to_lowercase();
        let body_lower = event.issue.body.as_deref().unwrap_or("").to_lowercase();

        let breaking_keywords = ["breaking", "breaking change", "bc:", "[breaking]"];

        breaking_keywords
            .iter()
            .any(|kw| title_lower.contains(kw) || body_lower.contains(kw))
            || event.issue.labels.iter().any(|l| {
                l.name.to_lowercase().contains("breaking") || l.name.to_lowercase() == "major"
            })
    }

    fn is_architectural_change(event: &IssueEvent) -> bool {
        let title_lower = event.issue.title.to_lowercase();
        let body_lower = event.issue.body.as_deref().unwrap_or("").to_lowercase();

        let arch_keywords = [
            "architecture",
            "redesign",
            "refactor",
            "migration",
            "overhaul",
            "rewrite",
        ];

        arch_keywords
            .iter()
            .any(|kw| title_lower.contains(kw) || body_lower.contains(kw))
            || event
                .issue
                .labels
                .iter()
                .any(|l| l.name.to_lowercase().contains("architecture"))
    }

    fn should_notify_human(analysis: &IssueAnalysis, strategy: &ExecutionStrategy) -> bool {
        matches!(strategy, ExecutionStrategy::HumanApprovalRequired)
            || matches!(analysis.issue_type, IssueType::Security)
            || matches!(analysis.complexity, Complexity::Complex)
    }

    fn determine_priority(analysis: &IssueAnalysis, event: &IssueEvent) -> Priority {
        if event
            .issue
            .labels
            .iter()
            .any(|l| l.name.to_lowercase().contains("critical") || l.name == "P0")
        {
            return Priority::Critical;
        }

        if matches!(analysis.issue_type, IssueType::Security) {
            return Priority::Critical;
        }

        if event.issue.labels.iter().any(|l| {
            l.name.to_lowercase().contains("urgent")
                || l.name.to_lowercase().contains("high")
                || l.name == "P1"
        }) {
            return Priority::High;
        }

        if matches!(analysis.issue_type, IssueType::Bug) {
            return Priority::High;
        }

        if matches!(analysis.complexity, Complexity::Complex) {
            return Priority::Medium;
        }

        Priority::Low
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Issue, IssueAction, Label, Repository, User};
    use arkavo_test_macros::spec;

    fn create_test_event(title: &str, body: &str, labels: Vec<&str>) -> IssueEvent {
        IssueEvent {
            action: IssueAction::Assigned,
            issue: Issue {
                id: 1,
                number: 1,
                title: title.to_string(),
                body: Some(body.to_string()),
                state: "open".to_string(),
                user: User {
                    id: 1,
                    login: "test".to_string(),
                    user_type: "User".to_string(),
                    avatar_url: "".to_string(),
                    html_url: "".to_string(),
                },
                labels: labels
                    .iter()
                    .enumerate()
                    .map(|(i, name)| Label {
                        id: i as u64,
                        name: name.to_string(),
                        color: "000000".to_string(),
                        description: None,
                    })
                    .collect(),
                assignees: vec![],
                created_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
                closed_at: None,
                html_url: "https://github.com/test/test/issues/1".to_string(),
                milestone: None,
            },
            repository: Repository {
                id: 1,
                name: "test".to_string(),
                full_name: "test/test".to_string(),
                owner: User {
                    id: 1,
                    login: "test".to_string(),
                    user_type: "User".to_string(),
                    avatar_url: "".to_string(),
                    html_url: "".to_string(),
                },
                html_url: "https://github.com/test/test".to_string(),
                description: None,
                default_branch: "main".to_string(),
                language: Some("Rust".to_string()),
            },
            sender: User {
                id: 1,
                login: "test".to_string(),
                user_type: "User".to_string(),
                avatar_url: "".to_string(),
                html_url: "".to_string(),
            },
            assignee: None,
        }
    }

    #[spec("ORCH-003")]
    #[test]
    fn test_trivial_documentation_auto_execute() {
        let event = create_test_event(
            "Fix typo in README",
            "Small typo",
            vec!["documentation", "good first issue"],
        );
        let decision = IssueRouter::route(&event);

        assert_eq!(decision.strategy, ExecutionStrategy::AutoExecute);
        assert_eq!(decision.priority, Priority::Low);
    }

    #[spec("ORCH-003")]
    #[test]
    fn test_simple_bug_requires_plan() {
        let event = create_test_event("Fix memory leak", "Small memory leak found", vec!["bug"]);
        let decision = IssueRouter::route(&event);

        assert_eq!(decision.strategy, ExecutionStrategy::PlanFirst);
        assert_eq!(decision.priority, Priority::High);
    }

    #[spec("ORCH-003")]
    #[test]
    fn test_breaking_change_requires_approval() {
        let event = create_test_event(
            "Breaking: Redesign API",
            "This is a breaking change",
            vec!["breaking", "enhancement"],
        );
        let decision = IssueRouter::route(&event);

        assert_eq!(decision.strategy, ExecutionStrategy::HumanApprovalRequired);
        assert!(decision.should_notify_human);
    }

    #[spec("ORCH-003")]
    #[test]
    fn test_security_issue_is_critical() {
        let event = create_test_event(
            "Security vulnerability in auth",
            "Found security issue",
            vec!["security"],
        );
        let decision = IssueRouter::route(&event);

        assert_eq!(decision.priority, Priority::Critical);
        assert!(decision.should_notify_human);
    }

    #[spec("ORCH-003")]
    #[test]
    fn test_complex_multi_tech_orchestration() {
        let event = create_test_event(
            "Implement distributed agent coordination",
            "Requires Rust, Python, Docker, Kubernetes integration",
            vec!["enhancement", "complex"],
        );
        let decision = IssueRouter::route(&event);

        assert_eq!(
            decision.strategy,
            ExecutionStrategy::OrchestratorConsultation
        );
    }
}
