use arkavo_git::{GitManager, safety::RepoGuard};
use arkavo_protocol::{
    agent_registry::{AgentInfo, AgentRegistry},
    http::HttpTransport,
    server::{RlmBridge, estimate_tokens, model_context_size},
    transport::{A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TlsConfig, TransportConfig},
    types::{
        ChunkPreviewInfo, Message, MessagePart, MessageSendRequest, MessageSendResponse,
        TaskGetRequest, TaskGetResponse, TaskStatus,
    },
};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
struct TaskConfig {
    auto_approve: bool,
    push: bool,
    validate: bool,
    message: Option<String>,
    use_local_only: bool,
    target_agent_id: Option<String>,
    mesh_only: bool,
}

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
    let mut use_local_only = false;
    let mut target_agent_id: Option<String> = None;
    let mut mesh_only = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--yes" | "-y" => auto_approve = true,
            "--push" => push = true,
            "--no-validate" => validate = false,
            "--local-only" => use_local_only = true,
            "--mesh-only" => mesh_only = true,
            "--message" | "-m" => {
                if i + 1 < args.len() {
                    message = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    return Err("--message requires an argument".into());
                }
            }
            "--agent-id" => {
                if i + 1 < args.len() {
                    target_agent_id = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    return Err("--agent-id requires an argument".into());
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
        let config = TaskConfig {
            auto_approve,
            push,
            validate,
            message,
            use_local_only,
            target_agent_id,
            mesh_only,
        };
        return execute_ai_task(&task, &config);
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

/// Discover agents on the mesh network using mDNS
#[cfg(feature = "mdns")]
fn discover_mesh_agents() -> Result<Vec<AgentInfo>, Box<dyn std::error::Error>> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};
    use std::time::Duration;

    info!("Discovering mesh agents via mDNS...");

    let mdns = ServiceDaemon::new()?;
    let receiver = mdns.browse("_a2a._tcp.local.")?;

    let mut agents = Vec::new();
    let timeout = Duration::from_secs(3);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                if let ServiceEvent::ServiceResolved(info) = event {
                    let agent_id = info
                        .get_property_val_str("agent_id")
                        .unwrap_or("unknown")
                        .to_string();
                    let name = info.get_fullname().to_string();
                    let purpose = info
                        .get_property_val_str("purpose")
                        .unwrap_or("")
                        .to_string();

                    let capabilities_str = info
                        .get_property_val_str("capabilities")
                        .unwrap_or_default();
                    let mut capabilities: Vec<String> = if capabilities_str.is_empty() {
                        vec![]
                    } else {
                        capabilities_str.split(',').map(|s| s.to_string()).collect()
                    };

                    let mcp_tools_str = info.get_property_val_str("mcp_tools").unwrap_or_default();
                    if !mcp_tools_str.is_empty() {
                        let mcp_tools: Vec<String> =
                            mcp_tools_str.split(',').map(|s| s.to_string()).collect();
                        capabilities.extend(mcp_tools);
                    }

                    let address = info
                        .get_addresses()
                        .iter()
                        .next()
                        .map(|addr| format!("http://{}:{}", addr, info.get_port()));

                    let mut metadata = HashMap::new();
                    if let Some(model) = info.get_property_val_str("model") {
                        metadata.insert("model".to_string(), model.to_string());
                    }

                    agents.push(AgentInfo {
                        agent_id,
                        name: name.clone(),
                        purpose,
                        capabilities,
                        device_caps: None,
                        metadata,
                        last_seen: chrono::Utc::now(),
                        load: 0.0,
                        is_available: true,
                        address,
                    });

                    debug!("Discovered agent: {}", name);
                }
            }
            Err(_) => {
                // Timeout on recv - continue until overall timeout
            }
        }
    }

    info!("Discovered {} agents via mDNS", agents.len());
    Ok(agents)
}

#[cfg(not(feature = "mdns"))]
fn discover_mesh_agents() -> Result<Vec<AgentInfo>, Box<dyn std::error::Error>> {
    warn!("mDNS feature not compiled in - cannot discover agents");
    Ok(Vec::new())
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

/// Try to execute task using mesh network agents
fn try_mesh_execution(task: &str, config: &TaskConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Discover agents on the network
    let discovered_agents = discover_mesh_agents()?;

    if discovered_agents.is_empty() {
        return Err("No mesh agents discovered".into());
    }

    println!("=== Discovered Mesh Agents ===\n");
    for agent in &discovered_agents {
        println!("  ✓ {} ({})", agent.name, agent.agent_id);
        if !agent.purpose.is_empty() {
            println!("    Purpose: {}", agent.purpose);
        }
        if !agent.capabilities.is_empty() {
            println!("    Capabilities: {}", agent.capabilities.join(", "));
        }
        if let Some(addr) = &agent.address {
            println!("    Address: {addr}");
        }
    }
    println!();

    // Select agent
    let selected_agent = if let Some(target_id) = &config.target_agent_id {
        discovered_agents
            .iter()
            .find(|a| &a.agent_id == target_id)
            .ok_or_else(|| format!("Agent {target_id} not found"))?
    } else {
        // Use agent registry for load balancing
        let registry = Arc::new(AgentRegistry::new());

        // Register all discovered agents
        #[allow(clippy::disallowed_methods)]
        for agent in &discovered_agents {
            let _ = tokio::runtime::Runtime::new()?.block_on(registry.register_agent(
                agent.agent_id.clone(),
                agent.name.clone(),
                agent.purpose.clone(),
                agent.capabilities.clone(),
                agent.device_caps.clone(),
                agent.metadata.clone(),
                agent.address.clone(),
            ));
        }

        // Find best agent for code generation
        #[allow(clippy::disallowed_methods)]
        let best_agent_id = tokio::runtime::Runtime::new()?
            .block_on(registry.find_best_agent("code_generation"))
            .or_else(|| {
                // Fallback to any available agent
                discovered_agents.first().map(|a| a.agent_id.clone())
            })
            .ok_or("No suitable agent found")?;

        discovered_agents
            .iter()
            .find(|a| a.agent_id == best_agent_id)
            .ok_or("Selected agent not found")?
    };

    println!("=== Selected Agent ===");
    println!(
        "  Agent: {} ({})",
        selected_agent.name, selected_agent.agent_id
    );
    println!("  Load: {:.0}%", selected_agent.load * 100.0);
    println!();

    // Submit task via A2A protocol
    let address = selected_agent
        .address
        .as_ref()
        .ok_or("Agent has no address")?;

    println!("=== Connecting to Agent ===\n");
    println!("  Address: {address}");

    // Execute A2A communication in async context
    #[allow(clippy::disallowed_methods)]
    tokio::runtime::Runtime::new()?.block_on(async {
        // Create transport configuration
        let transport_config = TransportConfig {
            timeout_ms: 60000,
            max_retries: 2,
            tls_config: TlsConfig {
                require_tls: false,
                ..Default::default()
            },
            ..Default::default()
        };

        // Create HTTP transport
        let transport = Arc::new(HttpTransport::new(transport_config)?);

        // Create endpoint
        let endpoint = A2aEndpoint {
            url: address.clone(),
            agent_id: selected_agent.agent_id.clone(),
            public_key: None,
        };

        // Connect to agent
        transport.connect(&endpoint).await.map_err(|e| {
            format!("Failed to connect to agent: {e}")
        })?;

        println!("  ✓ Connected\n");

        // Build and submit task
        println!("=== Submitting Task ===\n");
        println!("  Task: {task}");

        // Pre-flight check on outgoing A2A request (load from user config)
        let moderator = arkavo_router::preflight::load_policies_from_config()
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to load policies: {e}, using empty moderator");
                arkavo_router::preflight::PreflightModerator::new()
            });
        if let arkavo_router::ModerationResult::Block {
            policy_id, reason, ..
        } = moderator.check(task)
        {
            transport.close().await.ok();
            return Err(format!("Task blocked by policy {policy_id}: {reason}").into());
        }

        // Check if RLM mode should activate for large tasks
        let task_prompt = format!(
            "Task: {task}\n\nPlease analyze the repository, plan the changes, and execute the task. Use MCP tools to read files, make changes, and verify results."
        );
        let input_tokens = estimate_tokens(&task_prompt);
        // Assume mesh agents have smaller context windows (8K default for local models)
        let context_size = model_context_size(None, false);
        let rlm_bridge = RlmBridge::with_default_manager();

        let message = if rlm_bridge.should_activate(input_tokens, context_size) {
            info!(
                "RLM mode activated for mesh task: {} tokens > {}% of {} context",
                input_tokens, 70, context_size
            );

            // Decompose context and send manifest
            match rlm_bridge.manager().decompose(&task_prompt).await {
                Ok(result) => {
                    info!(
                        "Task context decomposed: {} chunks, {} tokens, manifest={}",
                        result.chunk_count, result.total_tokens, result.manifest_id
                    );

                    // Convert chunk previews
                    let chunk_previews: Vec<ChunkPreviewInfo> = result
                        .chunk_previews
                        .iter()
                        .map(|p| ChunkPreviewInfo {
                            index: p.index,
                            tokens: p.tokens,
                            preview: p.preview.clone(),
                            hints: p.hints.clone(),
                        })
                        .collect();

                    Message {
                        parts: vec![MessagePart::ContextManifest {
                            manifest_id: result.manifest_id,
                            summary: result.summary,
                            chunk_count: result.chunk_count,
                            total_tokens: result.total_tokens,
                            chunk_previews,
                        }],
                        metadata: Some(serde_json::json!({
                            "task_type": "code_task",
                            "source": "arkavo_mesh_cli",
                            "auto_execute": config.auto_approve,
                            "rlm_mode": true,
                        })),
                    }
                }
                Err(e) => {
                    warn!("RLM decomposition failed, falling back to text: {e}");
                    Message {
                        parts: vec![MessagePart::Text {
                            content: task_prompt,
                        }],
                        metadata: Some(serde_json::json!({
                            "task_type": "code_task",
                            "source": "arkavo_mesh_cli",
                            "auto_execute": config.auto_approve,
                        })),
                    }
                }
            }
        } else {
            debug!(
                "RLM mode not needed: {} tokens within {} context limit",
                input_tokens, context_size
            );
            Message {
                parts: vec![MessagePart::Text {
                    content: task_prompt,
                }],
                metadata: Some(serde_json::json!({
                    "task_type": "code_task",
                    "source": "arkavo_mesh_cli",
                    "auto_execute": config.auto_approve,
                })),
            }
        };

        let send_request = MessageSendRequest {
            message,
            task_id: None,
        };

        // Wrap in object with "request" key as expected by RPC method signature
        let rpc_request = A2aRequest::new(
            "message/send",
            serde_json::json!([send_request]),
        );

        let response = transport.send_request(rpc_request).await.map_err(|e| {
            format!("Failed to send task: {e}")
        })?;

        let task_id = match response {
            A2aResponse::Success { result, .. } => {
                let send_response: MessageSendResponse = serde_json::from_value(result)
                    .map_err(|e| format!("Failed to parse response: {e}"))?;
                println!("  ✓ Task submitted!");
                println!("  Task ID: {}", send_response.task_id);
                println!("  Status: {:?}\n", send_response.status);
                send_response.task_id
            }
            A2aResponse::Error { error, .. } => {
                transport.close().await.ok();
                return Err(format!("RPC error: {} - {}", error.code, error.message).into());
            }
        };

        // Poll for task completion
        println!("=== Monitoring Task Progress ===\n");

        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(300); // 5 minute timeout

        loop {
            if start.elapsed() > timeout {
                transport.close().await.ok();
                return Err("Task execution timed out after 5 minutes".into());
            }

            let get_request = TaskGetRequest {
                task_id: task_id.clone(),
            };

            let rpc_request = A2aRequest::new(
                "tasks/get",
                serde_json::json!([get_request]),
            );

            let response = transport.send_request(rpc_request).await.map_err(|e| {
                format!("Failed to get task status: {e}")
            })?;

            match response {
                A2aResponse::Success { result, .. } => {
                    let task_response: TaskGetResponse = serde_json::from_value(result)
                        .map_err(|e| format!("Failed to parse task response: {e}"))?;

                    match task_response.status {
                        TaskStatus::Completed => {
                            println!("\n  ✓ Task completed successfully!\n");

                            if let Some(result_msg) = task_response.result {
                                println!("=== Result ===\n");
                                for part in result_msg.parts {
                                    if let MessagePart::Text { content } = part {
                                        println!("{content}");
                                    }
                                }
                                println!();
                            }

                            transport.close().await.ok();
                            return Ok(());
                        }
                        TaskStatus::Failed => {
                            let error_msg = if let Some(error) = &task_response.error {
                                format!("{}: {}", error.code, error.message)
                            } else {
                                "Unknown error".to_string()
                            };

                            transport.close().await.ok();
                            return Err(format!("Task failed: {error_msg}").into());
                        }
                        TaskStatus::Canceled => {
                            transport.close().await.ok();
                            return Err("Task was canceled".into());
                        }
                        TaskStatus::Submitted | TaskStatus::Working | TaskStatus::InputRequired => {
                            // Show progress
                            if let Some(progress) = &task_response.progress {
                                if let Some(msg) = &progress.message {
                                    println!("  → {msg}");
                                }
                                if let Some(pct) = progress.percentage {
                                    println!("  {pct}% complete");
                                }
                            } else {
                                println!("  Status: {:?}", task_response.status);
                            }

                            // Wait before next poll
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                        _ => {
                            println!("  Status: {:?}", task_response.status);
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                    }
                }
                A2aResponse::Error { error, .. } => {
                    transport.close().await.ok();
                    return Err(format!("RPC error: {} - {}", error.code, error.message).into());
                }
            }
        }
    })
}

fn execute_ai_task(task: &str, config: &TaskConfig) -> Result<(), Box<dyn std::error::Error>> {
    use std::env;

    println!("=== Task: Plan and Execute ===\n");
    println!("Task: {task}\n");

    // Try to use mesh agents unless --local-only specified
    if !config.use_local_only {
        match try_mesh_execution(task, config) {
            Ok(()) => {
                println!("\n✓ Task completed via mesh network");
                return Ok(());
            }
            Err(e) => {
                if config.mesh_only {
                    eprintln!("Error: Mesh execution failed and --mesh-only specified");
                    return Err(e);
                }
                warn!(
                    "Mesh execution failed: {}, falling back to local execution",
                    e
                );
                println!("\n⚠ Mesh agents unavailable, falling back to local execution\n");
            }
        }
    }

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
            "  - Download a local model with: huggingface-cli download Qwen/Qwen3-0.6B-GGUF Qwen3-0.6B-Q8_0.gguf"
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
    if !config.auto_approve {
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
            if config.validate {
                println!("Running validation...");
                guard = guard.with_fmt_check().with_clippy_check();
            }

            let result = guard.transaction(|repo| {
                if let Some(msg) = config.message.as_ref() {
                    git_manager.add_all(repo)?;
                    git_manager.commit_changes(repo, msg)
                } else {
                    git_manager.auto_commit(repo)
                }
            });

            match result {
                Ok(oid) => {
                    println!("✓ Committed: {oid}");

                    if config.push {
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
        let model = env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-3-pro-preview".to_string());
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
        name: "Local Qwen3".to_string(),
        provider: "Local".to_string(),
        model: "qwen3-0.6b".to_string(),
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
    println!(
        "    arkavo task [TASK]              Execute AI task (tries mesh, falls back to local)"
    );
    println!("    arkavo task [OPTIONS]           Commit existing changes");
    println!();
    println!("EXAMPLES:");
    println!("    arkavo task 'fix all warnings'");
    println!("    arkavo task 'add tests' --mesh-only     # Require mesh agents");
    println!("    arkavo task 'refactor' --local-only     # Force local execution");
    println!("    arkavo task --yes                        # Auto-approve commit");
    println!();
    println!("OPTIONS:");
    println!("    -y, --yes          Auto-approve without prompting");
    println!("    --push             Push to remote after committing");
    println!("    --no-validate      Skip pre-commit validation");
    println!("    -m, --message <M>  Use custom commit message");
    println!("    --local-only       Skip mesh discovery, use local execution only");
    println!("    --mesh-only        Fail if no mesh agents available (no fallback)");
    println!("    --agent-id <ID>    Target specific agent by ID");
    println!("    -h, --help         Show this help");
}
