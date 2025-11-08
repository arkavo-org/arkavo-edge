use anyhow::Result;
use arkavo_orchestrator::{
    IssueEvent, Orchestrator,
    types::{Issue, IssueAction, Label, Repository, User},
};
use std::sync::Arc;

use super::github_api::parse_repo;
use super::types::{GitHubIssue, GitHubRepository};

pub(super) async fn process_github_issue(
    orchestrator: &Arc<Orchestrator>,
    repo: &str,
    repo_info: &GitHubRepository,
    issue: &GitHubIssue,
) -> Result<()> {
    let (_owner, _repo_name) = parse_repo(repo)?;

    let issue_event = IssueEvent {
        action: IssueAction::Opened,
        issue: Issue {
            id: issue.id,
            number: issue.number,
            title: issue.title.clone(),
            body: issue.body.clone(),
            state: issue.state.clone(),
            html_url: issue.html_url.clone(),
            user: User {
                id: issue.user.id,
                login: issue.user.login.clone(),
                user_type: issue.user.user_type.clone(),
                avatar_url: issue.user.avatar_url.clone(),
                html_url: issue.user.html_url.clone(),
            },
            labels: issue
                .labels
                .iter()
                .map(|l| Label {
                    id: l.id,
                    name: l.name.clone(),
                    color: l.color.clone(),
                    description: l.description.clone(),
                })
                .collect(),
            assignees: issue
                .assignees
                .iter()
                .map(|a| User {
                    id: a.id,
                    login: a.login.clone(),
                    user_type: a.user_type.clone(),
                    avatar_url: a.avatar_url.clone(),
                    html_url: a.html_url.clone(),
                })
                .collect(),
            created_at: issue.created_at.clone(),
            updated_at: issue.updated_at.clone(),
            closed_at: issue.closed_at.clone(),
            milestone: None,
        },
        repository: Repository {
            id: repo_info.id,
            full_name: repo_info.full_name.clone(),
            name: repo_info.name.clone(),
            owner: User {
                id: repo_info.owner.id,
                login: repo_info.owner.login.clone(),
                user_type: repo_info.owner.user_type.clone(),
                avatar_url: repo_info.owner.avatar_url.clone(),
                html_url: repo_info.owner.html_url.clone(),
            },
            html_url: repo_info.html_url.clone(),
            description: repo_info.description.clone(),
            default_branch: repo_info.default_branch.clone(),
            language: repo_info.language.clone(),
        },
        sender: User {
            id: issue.user.id,
            login: issue.user.login.clone(),
            user_type: issue.user.user_type.clone(),
            avatar_url: issue.user.avatar_url.clone(),
            html_url: issue.user.html_url.clone(),
        },
        assignee: None,
    };

    orchestrator.handle_issue_event(issue_event).await?;
    Ok(())
}
