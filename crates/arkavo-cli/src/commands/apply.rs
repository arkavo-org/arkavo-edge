use arkavo_git::{GitManager, safety::RepoGuard};
use std::env;

pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let git_manager = GitManager::new();
    let current_dir = env::current_dir()?;

    // Parse arguments
    let mut commit = true;
    let mut push = false;
    let mut message: Option<String> = None;
    let mut validate = true;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-commit" => commit = false,
            "--push" => push = true,
            "--no-validate" => validate = false,
            "--message" | "-m" => {
                if i + 1 < args.len() {
                    message = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    return Err("--message requires an argument".into());
                }
            }
            _ => {
                return Err(format!("Unknown argument: {}", args[i]).into());
            }
        }
        i += 1;
    }

    // Check if we're in a git repository
    let repo = if let Ok(repo) = git_manager.open_repo(&current_dir) {
        repo
    } else {
        eprintln!("Warning: Not in a git repository. Skipping git operations.");
        println!("Apply command completed (no git operations performed)");
        return Ok(());
    };

    // Get repository status
    let status = git_manager.status(&repo)?;
    let total_changes =
        status.modified.len() + status.added.len() + status.deleted.len() + status.untracked.len();

    if total_changes == 0 {
        println!("No changes to apply");
        return Ok(());
    }

    println!("Found {total_changes} changed files:");
    if !status.modified.is_empty() {
        println!("  Modified: {}", status.modified.len());
    }
    if !status.added.is_empty() {
        println!("  Added: {}", status.added.len());
    }
    if !status.deleted.is_empty() {
        println!("  Deleted: {}", status.deleted.len());
    }
    if !status.untracked.is_empty() {
        println!("  Untracked: {}", status.untracked.len());
    }

    if commit {
        // Create RepoGuard for safe operations
        let mut guard = RepoGuard::new(&repo)?;

        if validate {
            println!("\nRunning pre-commit validation...");
            guard = guard.with_fmt_check().with_clippy_check();
        }

        // Perform the commit
        let result = guard.transaction(|repo| {
            if let Some(msg) = message.as_ref() {
                // Use provided message
                git_manager.add_all(repo)?;
                git_manager.commit_changes(repo, msg)
            } else {
                // Use auto-generated message
                println!("Generating commit message...");
                git_manager.auto_commit(repo)
            }
        });

        match result {
            Ok(oid) => {
                println!("\nSuccessfully created commit: {oid}");

                // Get the commit message for display
                if let Ok(commit) = repo.find_commit(oid)
                    && let Some(msg) = commit.message()
                {
                    println!("Message: {}", msg.lines().next().unwrap_or(""));
                }

                if push {
                    println!("\nPushing to remote...");
                    match git_manager.publish(&repo) {
                        Ok(()) => println!("Successfully pushed to remote"),
                        Err(e) => {
                            eprintln!("Failed to push: {e}");
                            eprintln!("You can manually push with: git push");
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("\nFailed to create commit: {e}");
                if validate && e.to_string().contains("PreCommitFailed") {
                    eprintln!("\nPre-commit validation failed. You can:");
                    eprintln!("  - Fix the issues and try again");
                    eprintln!("  - Use --no-validate to skip validation");
                }
                return Err(e.into());
            }
        }
    } else {
        println!("\nSkipping commit (--no-commit specified)");
        println!("Changes remain in working directory");
    }

    Ok(())
}
