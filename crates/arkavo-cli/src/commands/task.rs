use arkavo_git::{GitManager, safety::RepoGuard};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
enum ModelCapability {
    Small,  // <2B params - info gathering, simple tasks
    Medium, // 2-7B params - planning, tool use, reasoning
    Large,  // >7B params - complex planning, multi-step
}

#[derive(Debug, Clone)]
struct LocalModelInfo {
    name: String,
    path: PathBuf,
    size_gb: f64,
    capability: ModelCapability,
}

fn infer_capability(name: &str, size_gb: f64) -> ModelCapability {
    let name_lower = name.to_lowercase();

    // Parse parameter count from name or size
    if name_lower.contains("270m") || name_lower.contains("1b") || size_gb < 2.0 {
        ModelCapability::Small
    } else if name_lower.contains("2b")
        || name_lower.contains("4b")
        || (2.0..8.0).contains(&size_gb)
    {
        ModelCapability::Medium
    } else {
        // 7b, 12b, or size >= 8GB
        ModelCapability::Large
    }
}

fn discover_local_models() -> Vec<LocalModelInfo> {
    let mut models = Vec::new();

    // Get HuggingFace cache directory
    let Some(hf_cache_dir) = dirs::home_dir().map(|d| d.join(".cache/huggingface/hub")) else {
        return models;
    };

    if !hf_cache_dir.exists() {
        return models;
    }

    // Scan for GGUF models in the cache
    let Ok(entries) = std::fs::read_dir(&hf_cache_dir) else {
        return models;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        // Check if it's a model directory
        if !dir_name.starts_with("models--") {
            continue;
        }

        // Look for GGUF files in snapshots
        let snapshots_dir = path.join("snapshots");
        if !snapshots_dir.exists() {
            continue;
        }

        let Ok(snapshot_entries) = std::fs::read_dir(&snapshots_dir) else {
            continue;
        };

        for snapshot in snapshot_entries.flatten() {
            let snapshot_path = snapshot.path();
            if !snapshot_path.is_dir() {
                continue;
            }

            // Check for .gguf files
            let Ok(files) = std::fs::read_dir(&snapshot_path) else {
                continue;
            };

            for file in files.flatten() {
                let file_name_os = file.file_name();
                let Some(file_name) = file_name_os.to_str() else {
                    continue;
                };

                if !file_name.ends_with(".gguf") {
                    continue;
                }

                let file_path = file.path();
                let size_bytes = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
                let size_gb = size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);

                let capability = infer_capability(file_name, size_gb);

                models.push(LocalModelInfo {
                    name: file_name.to_string(),
                    path: file_path,
                    size_gb,
                    capability,
                });
            }
        }
    }

    models
}

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
                    return Err(format!("Unknown argument: {arg}").into());
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

    // Discover available models
    let cloud_llms = detect_available_llms();
    let local_models = discover_local_models();

    // Select models for task
    let (local_model_path, cloud_model) = select_models_for_task(&cloud_llms, &local_models);

    // Display discovered models
    println!("=== Available Models ===\n");

    if !local_models.is_empty() {
        println!("Local Models:");
        for model in &local_models {
            let cap_str = match model.capability {
                ModelCapability::Small => "Small",
                ModelCapability::Medium => "Medium",
                ModelCapability::Large => "Large",
            };
            println!("  ✓ {} ({}) - {:.1} GB", model.name, cap_str, model.size_gb);
        }
        println!();
    }

    if !cloud_llms.is_empty() {
        println!("Cloud Models:");
        for llm in &cloud_llms {
            println!("  ✓ {} ({})", llm.name, llm.model);
        }
        println!();
    }

    // Determine final models to use
    let planning_model = cloud_model.as_ref().or(local_model_path.as_ref()).cloned();
    let local_model = local_model_path.as_ref().or(cloud_model.as_ref()).cloned();

    if planning_model.is_none() {
        eprintln!("Error: No models available");
        eprintln!("Please either:");
        eprintln!("  - Set GEMINI_API_KEY, OPENAI_API_KEY, or DEEPSEEK_API_KEY");
        eprintln!(
            "  - Download a local model with: huggingface-cli download unsloth/gemma-3-4b-it-GGUF"
        );
        return Err("No models available".into());
    }

    let planning_model = planning_model.unwrap();
    let local_model = local_model.unwrap();

    println!("=== Selected for Task ===");
    if cloud_model.is_some() {
        println!("Planning: {planning_model} (cloud)");
    } else {
        let model_name = local_models
            .iter()
            .find(|m| m.path.to_string_lossy() == local_model)
            .map(|m| m.name.as_str())
            .unwrap_or("local");
        println!("Planning: {model_name} (local)");
    }

    let model_name = local_models
        .iter()
        .find(|m| m.path.to_string_lossy() == local_model)
        .map(|m| m.name.as_str())
        .unwrap_or(&local_model);
    let cap = local_models
        .iter()
        .find(|m| m.path.to_string_lossy() == local_model)
        .map(|m| match m.capability {
            ModelCapability::Small => "Small",
            ModelCapability::Medium => "Medium",
            ModelCapability::Large => "Large",
        })
        .unwrap_or("Unknown");
    println!("Info Gathering: {model_name} ({cap})");
    println!("Verification: {model_name} ({cap})");
    println!();

    // Step 1: Multi-agent planning (local gathers info, cloud refines)
    println!("=== Step 1: Collaborative Planning ===\n");

    // Round 1: Local model gathers information
    println!("[Local Agent] Gathering information with {model_name}...\n");

    let gather_prompt = format!(
        "Task: {task}

Use MCP tools to gather information. Be concise.
- @filesystem {{\"action\": \"list_directory\", \"dir_path\": \".\"}}
- @git_status

Report your findings in 3-4 sentences. Then ask the planning agent 2-3 specific questions about what needs to be done."
    );

    let gather_args = vec![
        "--prompt".to_string(),
        gather_prompt,
        "--model".to_string(),
        local_model.clone(),
    ];
    crate::commands::chat::execute(&gather_args)?;

    // Round 2: Cloud model creates initial plan
    println!("\n[Cloud Agent] Creating plan with {planning_model}...\n");

    let plan_prompt = format!(
        "Task: {task}

Based on the local agent's findings above, create a detailed plan.

**Available MCP Tools:**
- @filesystem {{\"action\": \"read_file\", \"file_path\": \"path\"}}
- @filesystem {{\"action\": \"list_directory\", \"dir_path\": \"path\"}}
- @git_status
- @git_diff

Use tools to verify your assumptions. Output your plan:

## Analysis
[Key findings]

## Plan
1. [Step]
2. [Step]

## Files to Modify
- file: [changes]

Ask the local agent to verify anything uncertain."
    );

    let plan_args = vec![
        "--prompt".to_string(),
        plan_prompt,
        "--model".to_string(),
        planning_model.clone(),
    ];
    crate::commands::chat::execute(&plan_args)?;

    // Round 3: Local model verifies
    println!("\n[Local Agent] Verifying plan...\n");

    let verify_prompt = "Based on the cloud agent's plan above, verify the details using MCP tools.
Read the mentioned files and confirm the plan makes sense. Report any issues."
        .to_string();

    let verify_args = vec![
        "--prompt".to_string(),
        verify_prompt,
        "--model".to_string(),
        local_model.clone(),
    ];
    crate::commands::chat::execute(&verify_args)?;

    println!("\n=== Step 2: Review Plan ===\n");
    println!("Plan created through collaboration between local and cloud agents.");

    use std::io::{self, Write};
    if !auto_approve {
        print!("\nExecute this plan? [y/N]: ");
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
        "Based on the plan from our conversation, execute the changes:

Task: {task}

Use MCP tools to make the actual changes:
- @filesystem {{\"action\": \"write_file\", \"file_path\": \"path\", \"content\": \"...\"}} - Write file
- @filesystem {{\"action\": \"edit_file\", \"file_path\": \"path\", \"line_number\": N, \"new_content\": \"...\", \"mode\": \"replace\"}} - Edit specific line
- @git_status - Check changes
- @git_diff - Verify modifications

Execute the plan step by step. Show what you're doing."
    );

    let exec_args = vec![
        "--prompt".to_string(),
        execution_prompt,
        "--model".to_string(),
        planning_model,
    ];

    crate::commands::chat::execute(&exec_args)?;

    println!("\n=== Step 4: Commit Changes ===\n");

    // Now commit the changes if there are any
    let git_manager = GitManager::new();
    let current_dir = env::current_dir()?;

    if let Ok(repo) = git_manager.open_repo(&current_dir) {
        let status = git_manager.status(&repo)?;
        let total_changes = status.modified.len()
            + status.added.len()
            + status.deleted.len()
            + status.untracked.len();

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

fn select_models_for_task(
    cloud_llms: &[LlmInfo],
    local_models: &[LocalModelInfo],
) -> (Option<String>, Option<String>) {
    // Returns (local_model_path, cloud_model_name)

    // Select cloud model: Gemini > OpenAI > DeepSeek
    let cloud = cloud_llms
        .iter()
        .find(|llm| llm.provider == "Google")
        .or_else(|| cloud_llms.iter().find(|llm| llm.provider == "OpenAI"))
        .or_else(|| cloud_llms.iter().find(|llm| llm.provider == "DeepSeek"))
        .map(|llm| llm.model.clone());

    // Select local model: prefer Medium > Large > Small
    // Medium (4B) is ideal: fast + capable for tool use
    // Large (12B) works but slower
    // Small (270M) only as fallback
    let local = local_models
        .iter()
        .filter(|m| matches!(m.capability, ModelCapability::Medium))
        .max_by_key(|m| (m.size_gb * 1000.0) as u64)
        .or_else(|| {
            local_models
                .iter()
                .filter(|m| matches!(m.capability, ModelCapability::Large))
                .min_by_key(|m| (m.size_gb * 1000.0) as u64)
        })
        .or_else(|| local_models.first())
        .map(|m| m.path.to_string_lossy().to_string());

    (local, cloud)
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
