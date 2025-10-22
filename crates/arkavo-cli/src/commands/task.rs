use arkavo_git::{GitManager, safety::RepoGuard};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Parse arguments
    let mut auto_approve = false;
    let mut push = false;
    let mut message: Option<String> = None;
    let mut validate = true;
    let mut task_description: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--yes" | "-y" => auto_approve = true,
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
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            arg => {
                // First non-flag argument is the task description
                if !arg.starts_with('-') {
                    task_description = Some(arg.to_string());
                    // Collect remaining args as part of task description
                    if i + 1 < args.len() {
                        let remaining: Vec<String> = args[i + 1..].to_vec();
                        task_description = Some(format!("{} {}", arg, remaining.join(" ")));
                        break;
                    }
                } else {
                    return Err(format!("Unknown argument: {}", arg).into());
                }
            }
        }
        i += 1;
    }

    // If task description provided, run AI agent mode
    if let Some(task) = task_description {
        return execute_ai_task(&task, auto_approve, push, validate, message);
    }

    let git_manager = GitManager::new();
    let current_dir = env::current_dir()?;

    // Check if we're in a git repository
    let repo = if let Ok(repo) = git_manager.open_repo(&current_dir) {
        repo
    } else {
        eprintln!("Error: Not in a git repository");
        return Err("Task command requires a git repository".into());
    };

    // Step 1: Generate plan
    println!("=== Generating Change Plan ===\n");
    println!("Repository: {}", current_dir.display());
    println!();

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
        for file in &status.modified {
            println!("    - {file}");
        }
    }
    if !status.added.is_empty() {
        println!("  Added: {}", status.added.len());
        for file in &status.added {
            println!("    - {file}");
        }
    }
    if !status.deleted.is_empty() {
        println!("  Deleted: {}", status.deleted.len());
        for file in &status.deleted {
            println!("    - {file}");
        }
    }
    if !status.untracked.is_empty() {
        println!("  Untracked: {}", status.untracked.len());
        for file in &status.untracked {
            println!("    - {file}");
        }
    }

    println!();

    // Show plan summary
    show_plan_summary(&current_dir)?;

    // Step 2: Ask for approval (unless auto-approve)
    if !auto_approve {
        println!();
        print!("Apply this plan and commit changes? [y/N]: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("Task cancelled");
            return Ok(());
        }
    }

    // Step 3: Apply changes
    println!("\n=== Applying Changes ===\n");

    // Create RepoGuard for safe operations
    let mut guard = RepoGuard::new(&repo)?;

    if validate {
        println!("Running pre-commit validation...");
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
            println!("\n✓ Successfully created commit: {oid}");

            // Get the commit message for display
            if let Ok(commit) = repo.find_commit(oid)
                && let Some(msg) = commit.message()
            {
                println!("Message: {}", msg.lines().next().unwrap_or(""));
            }

            if push {
                println!("\nPushing to remote...");
                match git_manager.publish(&repo) {
                    Ok(()) => println!("✓ Successfully pushed to remote"),
                    Err(e) => {
                        eprintln!("Failed to push: {e}");
                        eprintln!("You can manually push with: git push");
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("\nFailed to apply changes: {e}");
            if validate && e.to_string().contains("PreCommitFailed") {
                eprintln!("\nPre-commit validation failed. You can:");
                eprintln!("  - Fix the issues and run 'arkavo task' again");
                eprintln!("  - Use --no-validate to skip validation");
            }
            return Err(e.into());
        }
    }

    Ok(())
}

fn show_plan_summary(current_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Plan Summary ===\n");

    match fs::read_dir(current_dir) {
        Ok(entries) => {
            let mut source_files = Vec::new();

            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();

                if !path.is_dir()
                    && (name.ends_with(".rs")
                        || name.ends_with(".toml")
                        || name.ends_with(".md")
                        || name.ends_with(".json")
                        || name.ends_with(".yaml")
                        || name.ends_with(".yml"))
                {
                    source_files.push(name);
                }
            }

            source_files.sort();

            if source_files.is_empty() {
                println!("No source files found in current directory");
            } else {
                println!("Potential files affected:");
                for file in source_files {
                    println!("  • {file}");
                }
            }
        }
        Err(e) => {
            eprintln!("Warning: Could not read directory: {e}");
        }
    }

    Ok(())
}

fn execute_ai_task(
    task: &str,
    _auto_approve: bool,
    _push: bool,
    _validate: bool,
    _message: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::env;

    println!("=== AI Task Execution ===\n");
    println!("Task: {task}\n");

    // Lightweight API key detection (no model loading)
    let available_llms = detect_available_llms();

    // Display available models
    println!("Available LLMs:");
    for llm in &available_llms {
        println!("  ✓ {} ({}) - {}", llm.name, llm.provider, llm.model);
    }
    println!();

    // Simple heuristic model selection (no classifier needed)
    let selected_model = select_model_for_task(task, &available_llms);

    println!("Selected Model: {}", selected_model);
    println!("Executing task...\n");

    // Use chat command with selected model
    let chat_args = vec![
        "--prompt".to_string(),
        task.to_string(),
        "--model".to_string(),
        selected_model,
    ];
    crate::commands::chat::execute(&chat_args)
}

#[derive(Debug, Clone)]
struct LlmInfo {
    name: String,
    provider: String,
    model: String,
}

fn detect_available_llms() -> Vec<LlmInfo> {
    use std::env;
    let mut llms = Vec::new();

    // Check for Gemini
    if env::var("GEMINI_API_KEY").is_ok() {
        let model = env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-2.5-pro".to_string());
        llms.push(LlmInfo {
            name: "Gemini".to_string(),
            provider: "Google".to_string(),
            model,
        });
    }

    // Check for OpenAI
    if env::var("OPENAI_API_KEY").is_ok() {
        llms.push(LlmInfo {
            name: "OpenAI".to_string(),
            provider: "OpenAI".to_string(),
            model: "gpt-4".to_string(),
        });
    }

    // Check for DeepSeek
    if env::var("DEEPSEEK_API_KEY").is_ok() {
        llms.push(LlmInfo {
            name: "DeepSeek".to_string(),
            provider: "DeepSeek".to_string(),
            model: "deepseek-chat".to_string(),
        });
    }

    // Always include local model
    llms.push(LlmInfo {
        name: "Local Gemma".to_string(),
        provider: "Local".to_string(),
        model: "gemma-3-270m-it".to_string(),
    });

    llms
}

fn select_model_for_task(task: &str, llms: &[LlmInfo]) -> String {
    let task_lower = task.to_lowercase();

    // Prefer cloud models for complex tasks
    let is_complex = task_lower.contains("refactor")
        || task_lower.contains("design")
        || task_lower.contains("architect")
        || task_lower.contains("implement")
        || task_lower.contains("fix all")
        || task.len() > 100;

    if is_complex {
        // Prefer Gemini for complex tasks
        if let Some(gemini) = llms.iter().find(|llm| llm.provider == "Google") {
            return gemini.model.clone();
        }
        // Fallback to OpenAI
        if let Some(openai) = llms.iter().find(|llm| llm.provider == "OpenAI") {
            return openai.model.clone();
        }
    }

    // For simple tasks or if no cloud models, use local
    llms.iter()
        .find(|llm| llm.provider == "Local")
        .map(|llm| llm.model.clone())
        .unwrap_or_else(|| "gemma-3-270m-it".to_string())
}

fn print_usage() {
    println!("Plan and apply code changes");
    println!();
    println!("USAGE:");
    println!("    arkavo task [TASK]              Execute AI task");
    println!("    arkavo task [OPTIONS]           Commit existing changes");
    println!();
    println!("EXAMPLES:");
    println!("    arkavo task 'fix all warnings'");
    println!("    arkavo task --yes               # Auto-approve commit");
    println!();
    println!("OPTIONS:");
    println!("    -y, --yes          Auto-approve without prompting");
    println!("    --push             Push to remote after committing");
    println!("    --no-validate      Skip pre-commit validation");
    println!("    -m, --message <M>  Use custom commit message");
    println!("    -h, --help         Show this help");
}
