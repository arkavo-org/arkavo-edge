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
    auto_approve: bool,
    push: bool,
    validate: bool,
    message: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::env;

    println!("=== Task: Plan and Execute ===\n");
    println!("Task: {task}\n");

    // Lightweight API key detection (no model loading)
    let available_llms = detect_available_llms();

    // Display available models
    println!("Available LLMs:");
    for llm in &available_llms {
        println!("  ✓ {} ({}) - {}", llm.name, llm.provider, llm.model);
    }
    println!();

    // Select planning model (prefer capable cloud models)
    let planning_model = select_planning_model(&available_llms);
    println!("Planning Model: {}", planning_model);
    println!();

    // Step 1: Generate plan using capable model with MCP tools
    println!("=== Step 1: Planning ===\n");

    let planning_prompt = format!(
        "Task: {}

You are a planning agent with access to MCP tools. Your job is to:
1. Analyze the task requirements
2. Use @read_file, @git_status, @git_diff to understand current state
3. Create a detailed step-by-step plan
4. List exactly what files need to be changed and how

Use MCP tools to gather information. Output your plan in this format:

## Analysis
[What you discovered using tools]

## Plan
1. [Step 1]
2. [Step 2]
...

## Files to Modify
- file1.rs: [what to change]
- file2.rs: [what to change]

Be specific and thorough.",
        task
    );

    let plan_args = vec![
        "--prompt".to_string(),
        planning_prompt,
        "--model".to_string(),
        planning_model.clone(),
    ];

    println!("Generating plan with {}...\n", planning_model);
    crate::commands::chat::execute(&plan_args)?;

    println!("\n=== Step 2: Review Plan ===\n");

    if !auto_approve {
        print!("Execute this plan? [y/N]: ");
        use std::io::{self, Write};
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("Task cancelled");
            return Ok(());
        }
    }

    println!("\n=== Step 3: Execute Plan ===\n");

    let execution_prompt = format!(
        "Based on the plan above, execute the changes:

Task: {}

Use MCP tools to make the actual changes:
- @write_file to modify files
- @git_status to check changes
- @git_diff to verify modifications

Execute the plan step by step. Show what you're doing.",
        task
    );

    let exec_args = vec![
        "--prompt".to_string(),
        execution_prompt,
        "--model".to_string(),
        planning_model.clone(),
    ];

    crate::commands::chat::execute(&exec_args)?;

    println!("\n=== Step 4: Commit Changes ===\n");

    // Now commit the changes if there are any
    let git_manager = GitManager::new();
    let current_dir = env::current_dir()?;

    if let Ok(repo) = git_manager.open_repo(&current_dir) {
        let status = git_manager.status(&repo)?;
        let total_changes = status.modified.len() + status.added.len()
            + status.deleted.len() + status.untracked.len();

        if total_changes > 0 {
            println!("Found {total_changes} changed files");

            // Create commit with provided message or auto-generate
            let mut guard = RepoGuard::new(&repo)?;
            if validate {
                println!("Running validation...");
                guard = guard.with_fmt_check().with_clippy_check();
            }

            let result = guard.transaction(|repo| {
                if let Some(msg) = message.as_ref() {
                    git_manager.add_all(repo)?;
                    git_manager.commit_changes(repo, msg)
                } else {
                    git_manager.auto_commit(repo)
                }
            });

            match result {
                Ok(oid) => {
                    println!("✓ Committed: {oid}");

                    if push {
                        println!("Pushing to remote...");
                        git_manager.publish(&repo)?;
                        println!("✓ Pushed");
                    }
                }
                Err(e) => {
                    eprintln!("Failed to commit: {e}");
                    return Err(e.into());
                }
            }
        } else {
            println!("No changes to commit");
        }
    }

    println!("\n✓ Task completed");
    Ok(())
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

fn select_planning_model(llms: &[LlmInfo]) -> String {
    // For planning, always prefer the most capable model
    // Priority: Gemini > OpenAI > DeepSeek > Local

    if let Some(gemini) = llms.iter().find(|llm| llm.provider == "Google") {
        return gemini.model.clone();
    }

    if let Some(openai) = llms.iter().find(|llm| llm.provider == "OpenAI") {
        return openai.model.clone();
    }

    if let Some(deepseek) = llms.iter().find(|llm| llm.provider == "DeepSeek") {
        return deepseek.model.clone();
    }

    // Fallback to local (will warn user that planning may not work well)
    llms.iter()
        .find(|llm| llm.provider == "Local")
        .map(|llm| llm.model.clone())
        .unwrap_or_else(|| "gemma-3-270m-it".to_string())
}

fn select_model_for_task(task: &str, llms: &[LlmInfo]) -> String {
    let task_lower = task.to_lowercase();

    // Always prefer cloud models when available (they have tool use capability)
    // Cloud models are much better at using MCP tools

    // Check if task requires tools (filesystem, git, code operations)
    let needs_tools = task_lower.contains("list")
        || task_lower.contains("read")
        || task_lower.contains("write")
        || task_lower.contains("file")
        || task_lower.contains("git")
        || task_lower.contains("fix")
        || task_lower.contains("refactor")
        || task_lower.contains("design")
        || task_lower.contains("architect")
        || task_lower.contains("implement")
        || task_lower.contains("change")
        || task_lower.contains("update")
        || task.len() > 100;

    if needs_tools || llms.len() > 1 {
        // Prefer Gemini for tool-based tasks
        if let Some(gemini) = llms.iter().find(|llm| llm.provider == "Google") {
            return gemini.model.clone();
        }
        // Fallback to OpenAI
        if let Some(openai) = llms.iter().find(|llm| llm.provider == "OpenAI") {
            return openai.model.clone();
        }
        // Fallback to DeepSeek
        if let Some(deepseek) = llms.iter().find(|llm| llm.provider == "DeepSeek") {
            return deepseek.model.clone();
        }
    }

    // Only use local model if no cloud models available or for pure Q&A
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
