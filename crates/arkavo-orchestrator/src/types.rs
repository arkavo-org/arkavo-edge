use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum GitHubEvent {
    #[serde(rename = "issues")]
    Issues(Box<IssueEvent>),
    #[serde(rename = "pull_request")]
    PullRequest(Box<PullRequestEvent>),
    #[serde(rename = "ping")]
    Ping(PingEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueEvent {
    pub action: IssueAction,
    pub issue: Issue,
    pub repository: Repository,
    pub sender: User,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<User>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IssueAction {
    Opened,
    Edited,
    Deleted,
    Pinned,
    Unpinned,
    Closed,
    Reopened,
    Assigned,
    Unassigned,
    Labeled,
    Unlabeled,
    Locked,
    Unlocked,
    Transferred,
    Milestoned,
    Demilestoned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: u64,
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub user: User,
    pub labels: Vec<Label>,
    pub assignees: Vec<User>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    pub html_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone: Option<Milestone>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestEvent {
    pub action: PullRequestAction,
    pub number: u64,
    pub pull_request: PullRequest,
    pub repository: Repository,
    pub sender: User,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestAction {
    Assigned,
    Unassigned,
    ReviewRequested,
    ReviewRequestRemoved,
    Labeled,
    Unlabeled,
    Opened,
    Edited,
    Closed,
    Reopened,
    Synchronize,
    ConvertedToDraft,
    Locked,
    Unlocked,
    Enqueued,
    Dequeued,
    Milestoned,
    Demilestoned,
    ReadyForReview,
    AutoMergeEnabled,
    AutoMergeDisabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    pub id: u64,
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub draft: bool,
    pub merged: bool,
    pub user: User,
    pub labels: Vec<Label>,
    pub assignees: Vec<User>,
    pub requested_reviewers: Vec<User>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    pub html_url: String,
    pub head: GitRef,
    pub base: GitRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRef {
    pub label: String,
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
    pub user: User,
    pub repo: Repository,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub owner: User,
    pub html_url: String,
    pub description: Option<String>,
    pub default_branch: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub login: String,
    #[serde(rename = "type")]
    pub user_type: String,
    pub avatar_url: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub id: u64,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub id: u64,
    pub number: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingEvent {
    pub zen: String,
    pub hook_id: u64,
    pub hook: WebhookConfig,
    pub repository: Option<Repository>,
    pub sender: User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    #[serde(rename = "type")]
    pub hook_type: String,
    pub id: u64,
    pub name: String,
    pub active: bool,
    pub events: Vec<String>,
    pub config: WebhookConfigDetails,
    pub updated_at: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfigDetails {
    pub content_type: Option<String>,
    pub insecure_ssl: Option<String>,
    pub url: Option<String>,
}

impl GitHubEvent {
    pub fn event_type(&self) -> &str {
        match self {
            GitHubEvent::Issues(_) => "issues",
            GitHubEvent::PullRequest(_) => "pull_request",
            GitHubEvent::Ping(_) => "ping",
        }
    }
}

impl IssueEvent {
    pub fn is_assigned_to(&self, login: &str) -> bool {
        self.issue
            .assignees
            .iter()
            .any(|assignee| assignee.login == login)
    }

    pub fn has_label(&self, label_name: &str) -> bool {
        self.issue
            .labels
            .iter()
            .any(|label| label.name == label_name)
    }
}

impl PullRequestEvent {
    pub fn is_assigned_to(&self, login: &str) -> bool {
        self.pull_request
            .assignees
            .iter()
            .any(|assignee| assignee.login == login)
    }

    pub fn has_label(&self, label_name: &str) -> bool {
        self.pull_request
            .labels
            .iter()
            .any(|label| label.name == label_name)
    }
}
