use anyhow::{Result, anyhow};
use chrono::Utc;
use tracing::{error, info, warn};

use super::github_api::{fetch_new_issues, fetch_repository};
use super::init::create_polling_orchestrator;
use super::process::process_github_issue;
use super::types::PollState;

pub(super) async fn poll_github(
    repo: String,
    interval: u64,
    once: bool,
    token: Option<String>,
    labels: Option<String>,
) -> Result<()> {
    let token = token.ok_or_else(|| {
        anyhow!(
            "GitHub token required for '{repo}'. Set GITHUB_TOKEN environment variable or use --token"
        )
    })?;

    info!("Starting GitHub polling for repository: {}", repo);
    if once {
        info!("Running in one-shot mode");
    } else {
        info!("Polling every {} seconds", interval);
    }

    let mut state = PollState::load(&repo)?;
    let orchestrator = create_polling_orchestrator(&token).await?;
    let repo_info = fetch_repository(&repo, &token).await?;

    loop {
        info!("Polling {} for new issues", repo);

        match fetch_new_issues(&repo, &token, &state.last_poll, labels.as_deref()).await {
            Ok(issues) => {
                info!("Found {} issues to process", issues.len());

                for issue in issues {
                    if state.contains(issue.number) {
                        continue;
                    }

                    info!("Processing issue #{}: {}", issue.number, issue.title);

                    if let Err(e) =
                        process_github_issue(&orchestrator, &repo, &repo_info, &issue).await
                    {
                        error!("Failed to process issue #{}: {}", issue.number, e);
                    } else {
                        state.insert_processed(issue.number);
                    }
                }

                state.last_poll = Utc::now();
                if let Err(e) = state.save(&repo) {
                    warn!("Failed to save poll state: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to fetch issues: {}", e);
            }
        }

        if once {
            info!("One-shot mode complete");
            break;
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(interval)).await;
    }

    Ok(())
}

pub(super) async fn process_issue(
    repo: String,
    issue_number: u64,
    token: Option<String>,
) -> Result<()> {
    let token = token.ok_or_else(|| {
        anyhow!(
            "GitHub token required for '{repo}'. Set GITHUB_TOKEN environment variable or use --token"
        )
    })?;

    info!("Processing issue #{} from {}", issue_number, repo);

    let orchestrator = create_polling_orchestrator(&token).await?;
    let repo_info = fetch_repository(&repo, &token).await?;
    let issue = super::github_api::fetch_issue(&repo, &token, issue_number).await?;

    info!("Issue #{}: {}", issue.number, issue.title);
    process_github_issue(&orchestrator, &repo, &repo_info, &issue).await?;

    info!("Issue processing complete");
    Ok(())
}
