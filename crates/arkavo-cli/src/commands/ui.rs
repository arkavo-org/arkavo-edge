#[allow(clippy::disallowed_methods)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if !args.is_empty() && matches!(args[0].as_str(), "help" | "-h" | "--help") {
        print_usage();
        return Ok(());
    }

    let mut port = 7700;
    let mut initial_prompt: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--prompt" | "-p" => {
                if i + 1 < args.len() {
                    initial_prompt = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --prompt requires a value");
                    return Err("Missing prompt value".into());
                }
            }
            "--port" => {
                if i + 1 < args.len() {
                    port = args[i + 1].parse::<u16>().unwrap_or(7700);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            arg => {
                if let Ok(p) = arg.parse::<u16>() {
                    port = p;
                }
                i += 1;
            }
        }
    }

    let run_async = async move {
        #[cfg(feature = "cef-ui")]
        {
            println!("Starting Arkavo UI with native CEF renderer...");
            use_cef_renderer(port, initial_prompt).await
        }

        #[cfg(all(feature = "web-ui", not(feature = "cef-ui")))]
        {
            println!("Starting Arkavo UI with web renderer...");
            use_web_gateway(port, initial_prompt).await
        }

        #[cfg(not(any(feature = "cef-ui", feature = "web-ui")))]
        {
            let _ = port;
            let _ = initial_prompt;
            Err(
                "No UI renderer available. Build with --features cef-ui or --features web-ui"
                    .into(),
            )
        }
    };

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(run_async),
        Err(_) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(run_async)
        }
    }
}

#[cfg(feature = "cef-ui")]
#[allow(clippy::future_not_send)]
async fn use_cef_renderer(
    _port: u16,
    initial_prompt: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::first_run;
    use arkavo_agui::UiRenderer;
    use arkavo_agui::renderer::async_cef_renderer::AsyncCefRendererImpl;
    use arkavo_memory::PlanStateStore;

    println!("Creating Async CEF renderer (non-blocking)...");
    let mut cef_renderer = AsyncCefRendererImpl::new().await?;

    println!("CEF window opened with prompt bar!");

    // Wait for page to fully load - reduced from 2000ms
    println!("[DEBUG] Waiting for page to load...");
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    println!("[DEBUG] Page should be loaded now");

    // Check if this is first run and show welcome screen
    if first_run::is_first_run() {
        let caps = first_run::detect_capabilities();
        let model_size_gb = caps.recommended_model.size_bytes() as f64 / 1_000_000_000.0;

        let welcome_html = format!(
            r#"<div style="padding:40px;font-family:system-ui,-apple-system,sans-serif;background:linear-gradient(135deg,#667eea 0%,#764ba2 100%);min-height:calc(100vh - 80px);color:white;">
                <h1 style="margin:0 0 16px 0;font-size:32px;">Welcome Friend</h1>
                <p style="margin:0 0 32px 0;opacity:0.9;">Arkavo Edge needs a local AI model to run.</p>
                <div style="background:rgba(255,255,255,0.15);border-radius:12px;padding:24px;max-width:400px;">
                    <h3 style="margin:0 0 16px 0;font-size:18px;">Recommended Model</h3>
                    <div style="display:flex;justify-content:space-between;margin-bottom:8px;">
                        <span>{}</span>
                        <span>{:.1} GB</span>
                    </div>
                    <div style="font-size:14px;opacity:0.8;margin-bottom:16px;">
                        System: {} | Available: {:.1} GB
                    </div>
                    <button id="download-model-btn" onclick="window.arkavoSubmit && window.arkavoSubmit('download-model')" style="
                        width:100%;padding:12px 24px;background:white;color:#667eea;
                        border:none;border-radius:8px;font-size:16px;font-weight:600;
                        cursor:pointer;transition:transform 0.2s;
                    ">Download Model</button>
                    <p style="font-size:12px;margin-top:12px;opacity:0.7;text-align:center;">
                        Or enter a prompt below to start with a cloud model
                    </p>
                </div>
            </div>"#,
            caps.recommended_model.display_name(),
            model_size_gb,
            caps.device_profile,
            caps.available_disk_gb
        );
        cef_renderer.render(&welcome_html, "", "").await?;
    }

    // Check for interrupted plans that can be resumed
    let db_path = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("arkavo")
        .join("plan_state.db");

    if let Ok(store) = PlanStateStore::new(&db_path).await {
        // Mark any "executing" plans as interrupted (app was closed)
        let interrupted = store.mark_executing_as_interrupted().await.unwrap_or(0);
        if interrupted > 0 {
            eprintln!("[STARTUP] Marked {interrupted} executing plans as interrupted");
        }

        // Check for resumable plans
        if let Ok(resumable) = store.get_resumable_plans().await
            && !resumable.is_empty()
        {
            let plan = &resumable[0];
            eprintln!(
                "[STARTUP] Found resumable plan: {} ({}/{} subtasks)",
                plan.original_prompt, plan.current_subtask, plan.total_subtasks
            );

            // Show resume prompt in UI
            let resume_html = format!(
                r#"<div style="padding:40px;font-family:system-ui,-apple-system,sans-serif;">
                        <div style="background:#fff3cd;padding:20px;border-radius:8px;border-left:4px solid #ffc107;margin-bottom:20px;">
                            <strong style="color:#856404;">Interrupted Plan Found</strong>
                            <div style="margin-top:8px;color:#333;">"{}"</div>
                            <div style="font-size:13px;color:#666;margin-top:4px;">
                                Progress: {}/{} subtasks completed
                            </div>
                            <div style="margin-top:12px;font-size:13px;color:#666;">
                                Enter a new prompt to start fresh, or type <strong>resume</strong> to continue.
                            </div>
                        </div>
                    </div>"#,
                plan.original_prompt
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;"),
                plan.current_subtask,
                plan.total_subtasks
            );
            cef_renderer.render(&resume_html, "", "").await?;
        }
    }

    if let Some(prompt) = initial_prompt {
        println!("Processing initial prompt: {prompt}");
        handle_prompt_async(&mut cef_renderer, &prompt).await?;
    }

    println!("CEF renderer is running. Enter prompts in the UI or press Ctrl+C to exit.");

    // Event loop - poll for prompt submissions and errors
    let mut loop_count = 0;
    let mut last_health_check = std::time::Instant::now();
    loop {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        loop_count += 1;

        if loop_count % 50 == 0 {
            eprintln!("[HEARTBEAT] Event loop iteration {loop_count}");
        }

        // Check for stuck commands every 30 seconds
        if last_health_check.elapsed().as_secs() >= 30 {
            use arkavo_observability::health_reporter::HealthRegistry;

            let registry = HealthRegistry::global();
            let reports = registry.check_all().await;

            eprintln!("[HEARTBEAT] Health check: {} components", reports.len());
            for report in &reports {
                if report.component == "cef" {
                    eprintln!(
                        "[HEARTBEAT]   CEF: {:?} - {}",
                        report.status, report.message
                    );

                    // Check if degraded (stuck commands)
                    if format!("{:?}", report.status) == "Degraded" {
                        eprintln!("[WARNING] CEF has stuck commands!");
                        // Show warning in UI
                        let warning_html = format!(
                            r#"<div style="position:fixed;top:10px;right:10px;background:#ff9;border:2px solid #f80;padding:12px;border-radius:6px;z-index:9999;">
                            <strong>⚠️ Warning:</strong> {}</div>"#,
                            report
                                .message
                                .replace('&', "&amp;")
                                .replace('<', "&lt;")
                                .replace('>', "&gt;")
                        );
                        let _ = cef_renderer.update_element("body", &warning_html).await;
                    }
                }
            }

            last_health_check = std::time::Instant::now();
        }

        if !cef_renderer.is_running() {
            eprintln!("[DEBUG] CEF renderer is no longer running, breaking event loop");
            break;
        }

        // Poll for events from CEF (non-blocking)
        if let Some(event) = cef_renderer.try_recv_event() {
            eprintln!(
                "[DEBUG] Event received (async): type={}, value={}",
                event.event_type, event.value
            );

            match event.event_type.as_str() {
                "js_error" => {
                    eprintln!("[CEF JS ERROR] {}", event.value);
                    eprintln!("[CEF JS ERROR] Data: {}", event.data);

                    // Log error with health registry
                    use arkavo_observability::health_reporter::HealthRegistry;
                    let registry = HealthRegistry::global();
                    let _reports = registry.check_all().await;
                    eprintln!("[ERROR TELEMETRY] CEF JavaScript exception captured:");
                    eprintln!("  Selector: {}", event.selector);
                    eprintln!("  Target: {}", event.target_id);
                    eprintln!("  Error: {}", event.value);
                    eprintln!("  Context: {}", event.data);

                    // Show error in UI
                    let error_display = format!(
                        r#"<div style="position:fixed;top:10px;right:10px;background:#fee;border:2px solid #f44;padding:12px;border-radius:6px;z-index:9999;max-width:400px;">
                        <strong style="color:#c33;">⚠️ JavaScript Error</strong><br>
                        <span style="font-size:12px;color:#666;">{}</span>
                        </div>"#,
                        event
                            .value
                            .replace('&', "&amp;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;")
                    );
                    let _ = cef_renderer.update_element("body", &error_display).await;
                }
                "submit" if event.value.trim() == "download-model" => {
                    eprintln!("[DEBUG] Processing download-model request");
                    // Show downloading status
                    let downloading_html = r#"<div style="padding:40px;font-family:system-ui,-apple-system,sans-serif;background:linear-gradient(135deg,#667eea 0%,#764ba2 100%);min-height:calc(100vh - 80px);color:white;">
                        <h1 style="margin:0 0 16px 0;font-size:32px;">Downloading Model...</h1>
                        <p style="margin:0 0 32px 0;opacity:0.9;">This may take a few minutes.</p>
                        <div style="background:rgba(255,255,255,0.15);border-radius:12px;padding:24px;max-width:400px;">
                            <div style="display:flex;gap:8px;align-items:center;">
                                <div style="width:8px;height:8px;background:white;border-radius:50%;animation:pulse 1.5s ease-in-out infinite;"></div>
                                <span>Downloading from HuggingFace...</span>
                            </div>
                        </div>
                    </div>"#;
                    cef_renderer.render(downloading_html, "", "").await?;

                    // Download the model
                    let caps = first_run::detect_capabilities();
                    match first_run::download_model(&caps.recommended_model).await {
                        Ok(_) => {
                            // Show success
                            let success_html = r#"<div style="padding:40px;font-family:system-ui,-apple-system,sans-serif;background:linear-gradient(135deg,#667eea 0%,#764ba2 100%);min-height:calc(100vh - 80px);color:white;">
                                <h1 style="margin:0 0 16px 0;font-size:32px;">Download Complete!</h1>
                                <p style="margin:0 0 32px 0;opacity:0.9;">Arkavo Edge is ready.</p>
                                <div style="background:rgba(255,255,255,0.15);border-radius:12px;padding:24px;max-width:400px;">
                                    <p style="margin:0;font-size:16px;">Enter a prompt below to get started.</p>
                                </div>
                            </div>"#;
                            cef_renderer.render(success_html, "", "").await?;
                        }
                        Err(e) => {
                            let error_html = format!(
                                r#"<div style="padding:40px;font-family:system-ui,-apple-system,sans-serif;">
                                    <div style="background:#fee;padding:20px;border-radius:8px;border-left:4px solid #f44;">
                                        <strong style="color:#c33;">Download Failed</strong><br>
                                        <span style="color:#666;">{}</span>
                                    </div>
                                </div>"#,
                                e.replace('&', "&amp;")
                                    .replace('<', "&lt;")
                                    .replace('>', "&gt;")
                            );
                            cef_renderer.render(&error_html, "", "").await?;
                        }
                    }
                }
                "submit" if !event.value.trim().is_empty() => {
                    eprintln!("[DEBUG] Processing submit event (async)");
                    if let Err(e) = handle_prompt_async(&mut cef_renderer, &event.value).await {
                        eprintln!("[ERROR] Prompt processing failed: {e}");
                        return Err(e);
                    }
                    eprintln!("[DEBUG] Prompt processing completed");
                }
                _ => {
                    eprintln!("[DEBUG] Unhandled event type: {}", event.event_type);
                }
            }
        }
    }

    // Renderer stopped - clean shutdown
    println!("CEF window closed, shutting down...");
    eprintln!("[DEBUG] Calling renderer.shutdown()");
    match Box::new(cef_renderer).shutdown().await {
        Ok(_) => {
            eprintln!("[DEBUG] Shutdown successful");
            println!("✓ Application closed gracefully");
            Ok(())
        }
        Err(e) => {
            eprintln!("[DEBUG] Shutdown error: {e}");
            if e.to_string().contains("Connection closed") {
                println!("✓ Application closed");
                Ok(())
            } else {
                eprintln!("⚠ Shutdown error (non-fatal): {e}");
                println!("✓ Application closed");
                Ok(())
            }
        }
    }
}

#[cfg(feature = "cef-ui")]
#[allow(clippy::future_not_send)]
async fn handle_prompt_async(
    renderer: &mut dyn arkavo_agui::UiRenderer,
    prompt: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_llm::Message;
    use arkavo_router::{ComplexityScorer, Router};
    use std::sync::Arc;
    use tokio_stream::StreamExt;

    let enhanced_prompt = prompt.to_string();

    // Analyze task complexity to determine if architect mode should be used
    let scorer = ComplexityScorer::new();
    let complexity = scorer.analyze(&enhanced_prompt);

    if complexity.architect_recommended {
        return handle_architect_prompt(renderer, prompt, complexity).await;
    }

    // Check for special commands before processing with LLM
    let lower_prompt = prompt.trim().to_lowercase();
    if matches!(
        lower_prompt.as_str(),
        "list tools" | "show tools" | "what tools" | "available tools"
    ) {
        // Initialize MCP to get tool list
        #[cfg(all(unix, feature = "mcp-tools"))]
        {
            use crate::mcp_integration::McpConnection;
            #[cfg(all(target_os = "macos", feature = "mcp-macos"))]
            let mcp_result = McpConnection::new_in_process_async()
                .await
                .or_else(|_| McpConnection::new_external(std::env::var("MCP_URL").ok()));

            #[cfg(not(all(target_os = "macos", feature = "mcp-macos")))]
            let mcp_result = McpConnection::new_cross_platform()
                .or_else(|_| McpConnection::new_external(std::env::var("MCP_URL").ok()));

            if let Ok(mcp) = mcp_result {
                match mcp.list_tools() {
                    Ok(tools) if !tools.is_empty() => {
                        use std::fmt::Write as _;
                        let tools_list = tools.iter().fold(String::new(), |mut acc, t| {
                            let _ = write!(
                                acc,
                                r#"<div style="border-left:3px solid #667eea;padding-left:12px;">
                                        <strong style="color:#333;">{}</strong><br>
                                        <span style="color:#666;font-size:14px;">{}</span>
                                    </div>"#,
                                t.name
                                    .replace('&', "&amp;")
                                    .replace('<', "&lt;")
                                    .replace('>', "&gt;"),
                                t.description
                                    .replace('&', "&amp;")
                                    .replace('<', "&lt;")
                                    .replace('>', "&gt;")
                            );
                            acc
                        });
                        let tools_html = format!(
                            r#"<div style="padding: 40px; font-family: system-ui, -apple-system, sans-serif;">
                                <div style="background: #f5f5f5; padding: 12px 16px; border-radius: 8px; margin-bottom: 20px; color: #333;">
                                    <strong style="color: #667eea;">You:</strong> <span style="color: #333;">{prompt}</span>
                                </div>
                                <div style="background: white; padding: 20px; border-radius: 8px; box-shadow: 0 2px 8px rgba(0,0,0,0.1);">
                                    <h3 style="margin-top:0;color:#667eea;">Available Tools ({}) total</h3>
                                    <div style="display:grid;gap:12px;">
                                        {tools_list}
                                    </div>
                                </div>
                            </div>"#,
                            tools.len(),
                        );
                        renderer.render(&tools_html, "", "").await?;
                        return Ok(());
                    }
                    Ok(_) => {
                        let no_tools_html = format!(
                            r#"<div style="padding: 40px; font-family: system-ui, -apple-system, sans-serif;">
                                <div style="background: #f5f5f5; padding: 12px 16px; border-radius: 8px; margin-bottom: 20px; color: #333;">
                                    <strong style="color: #667eea;">You:</strong> <span style="color: #333;">{prompt}</span>
                                </div>
                                <div style="background: #fef3cd; padding: 20px; border-radius: 8px; border-left: 4px solid #f0ad4e;">
                                    <strong style="color:#8a6d3b;">⚠ No tools available</strong><br>
                                    <span style="color:#856404;">MCP connection established but no tools are registered.</span>
                                </div>
                            </div>"#
                        );
                        renderer.render(&no_tools_html, "", "").await?;
                        return Ok(());
                    }
                    Err(e) => {
                        let error_html = format!(
                            r#"<div style="padding: 40px; font-family: system-ui, -apple-system, sans-serif;">
                                <div style="background: #f5f5f5; padding: 12px 16px; border-radius: 8px; margin-bottom: 20px; color: #333;">
                                    <strong style="color: #667eea;">You:</strong> <span style="color: #333;">{prompt}</span>
                                </div>
                                <div style="background: #fee; padding: 20px; border-radius: 8px; border-left: 4px solid #f44;">
                                    <strong style="color:#c33;">⚠ Error listing tools</strong><br>
                                    <span style="color:#666;font-size:14px;">{}</span>
                                </div>
                            </div>"#,
                            e.to_string()
                                .replace('&', "&amp;")
                                .replace('<', "&lt;")
                                .replace('>', "&gt;")
                        );
                        renderer.render(&error_html, "", "").await?;
                        return Ok(());
                    }
                }
            }
        }

        #[cfg(not(all(unix, feature = "mcp-tools")))]
        {
            let no_mcp_html = format!(
                r#"<div style="padding: 40px; font-family: system-ui, -apple-system, sans-serif;">
                    <div style="background: #f5f5f5; padding: 12px 16px; border-radius: 8px; margin-bottom: 20px; color: #333;">
                        <strong style="color: #667eea;">You:</strong> <span style="color: #333;">{prompt}</span>
                    </div>
                    <div style="background: #fef3cd; padding: 20px; border-radius: 8px; border-left: 4px solid #f0ad4e;">
                        <strong style="color:#8a6d3b;">⚠ MCP tools not available</strong><br>
                        <span style="color:#856404;">Build with --features mcp-tools on Unix platforms to enable tool support.</span>
                    </div>
                </div>"#
            );
            renderer.render(&no_mcp_html, "", "").await?;
            return Ok(());
        }
    }

    // Initialize MCP connection for tool calling
    #[cfg(all(unix, feature = "mcp-tools"))]
    let mcp_client = {
        use crate::mcp_integration::McpConnection;
        // Use async version for macOS (avoids nested runtime panic)
        // Use cross-platform version for other Unix systems
        #[cfg(all(target_os = "macos", feature = "mcp-macos"))]
        let mcp_result = McpConnection::new_in_process_async()
            .await
            .or_else(|_| McpConnection::new_external(std::env::var("MCP_URL").ok()));

        #[cfg(not(all(target_os = "macos", feature = "mcp-macos")))]
        let mcp_result = McpConnection::new_cross_platform()
            .or_else(|_| McpConnection::new_external(std::env::var("MCP_URL").ok()));

        let client = mcp_result
            .ok()
            .map(|mcp| Arc::new(mcp) as Arc<dyn arkavo_mcp_tools::McpClient>);

        // Debug: Show MCP tools being passed
        if std::env::var("ARKAVO_DEBUG_CHAT").is_ok() {
            eprintln!("\n=== MCP TOOLS DEBUG (UI) ===");
            if let Some(ref mcp) = client {
                match mcp.list_tools() {
                    Ok(tools) => {
                        eprintln!("Tools registered: {}", tools.len());
                        for tool in &tools {
                            eprintln!("  - {} : {}", tool.name, tool.description);
                        }
                    }
                    Err(e) => eprintln!("Error listing tools: {e}"),
                }
            } else {
                eprintln!("No MCP client initialized");
            }
            eprintln!("============================\n");
        }

        client
    };

    // Check for API key availability to determine if we should use cloud models
    let gemini_available = std::env::var("GEMINI_API_KEY").is_ok();

    let router = if gemini_available {
        Router::new().await?
    } else {
        Router::new_offline().await?
    };

    let messages = vec![Message::user(&enhanced_prompt)];

    // Use router with tools for native tool calling
    #[cfg(all(unix, feature = "mcp-tools"))]
    let use_tools = true;
    #[cfg(not(all(unix, feature = "mcp-tools")))]
    let use_tools = false;

    // Show "thinking" indicator in UI
    let thinking_html = format!(
        r#"<div style="padding: 40px; font-family: system-ui, -apple-system, sans-serif;">
            <div style="background: #f5f5f5; padding: 12px 16px; border-radius: 8px; margin-bottom: 20px; color: #333;">
                <strong style="color: #667eea;">You:</strong> <span style="color: #333;">{prompt}</span>
            </div>
            <div style="background: white; padding: 20px; border-radius: 8px; box-shadow: 0 2px 8px rgba(0,0,0,0.1);">
                <div style="display:flex;gap:8px;align-items:center;color:#667eea;">
                    <div style="width:8px;height:8px;background:#667eea;border-radius:50%;animation:pulse 1.5s ease-in-out infinite;"></div>
                    <span>Thinking...</span>
                </div>
            </div>
        </div>"#
    );
    renderer.render(&thinking_html, "", "").await?;

    // Route the request to determine which model to use
    let routing_decision = router.classify(&enhanced_prompt).await?;

    let response_text = if use_tools {
        // Use router with native tool calling
        #[cfg(all(unix, feature = "mcp-tools"))]
        {
            use crate::tool_integration::complete_with_tools;
            complete_with_tools(&enhanced_prompt, messages, mcp_client).await?
        }
        #[cfg(not(all(unix, feature = "mcp-tools")))]
        String::new()
    } else {
        // Fallback to direct streaming
        let client = create_client_from_routing(&routing_decision).await?;

        let stream_result = client.stream(messages).await;
        let mut stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                let error_msg = e.to_string();
                if error_msg.contains("Connection refused") || error_msg.contains("connect") {
                    let error_html = format!(
                        r#"<div style="padding: 40px; font-family: system-ui, -apple-system, sans-serif;">
                            <div style="background: #f5f5f5; padding: 12px 16px; border-radius: 8px; margin-bottom: 20px;">
                                <strong style="color: #667eea;">You:</strong> {prompt}
                            </div>
                            <div style="background: #fee; padding: 20px; border-radius: 8px; border-left: 4px solid #f44;">
                                <strong style="color: #c33;">Connection Error</strong><br>
                                <span>Cannot connect to Ollama. Please start Ollama:</span><br>
                                <code style="background: #f5f5f5; padding: 4px 8px; border-radius: 4px; display: inline-block; margin-top: 8px;">brew services start ollama</code>
                            </div>
                        </div>"#
                    );
                    renderer.render(&error_html, "", "").await?;
                }
                return Err(e.into());
            }
        };

        let mut response_text = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    response_text.push_str(&chunk.content);
                }
                Err(e) => {
                    eprintln!("[ERROR] Stream error: {e}");
                    return Err(e.into());
                }
            }
        }
        response_text
    };

    // Detect if response is HTML (contains <html> or multiple HTML tags)
    let is_html = response_text.contains("<html")
        || (response_text.contains("<div") && response_text.contains("<style"));

    let html = if is_html {
        // LLM generated UI - render it directly
        response_text.clone()
    } else {
        // Chat response - format as conversation
        let escaped_response = response_text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\n', "<br>");

        format!(
            r#"<div style="padding: 40px; font-family: system-ui, -apple-system, sans-serif;">
                <div style="background: #f5f5f5; padding: 12px 16px; border-radius: 8px; margin-bottom: 20px; color: #333;">
                    <strong style="color: #667eea;">You:</strong> <span style="color: #333;">{prompt}</span>
                </div>
                <div style="background: white; padding: 20px; border-radius: 8px; box-shadow: 0 2px 8px rgba(0,0,0,0.1); line-height: 1.6; color: #333;">
                    {escaped_response}
                </div>
                <div style="margin-top: 16px; padding: 8px 12px; background: #f0f0f0; border-radius: 4px; font-size: 12px; color: #666;">
                    Model: {} | Cost: ${:.4}
                </div>
            </div>"#,
            routing_decision.recommended_model.name(),
            routing_decision.estimated_cost_usd
        )
    };

    eprintln!(
        "[DEBUG] Rendering response ({} bytes, is_html={})",
        html.len(),
        is_html
    );
    eprintln!("[DEBUG] First 200 chars: {}", &html[..html.len().min(200)]);

    renderer.render(&html, "", "").await.map_err(|e| {
        eprintln!("[ERROR] renderer.render() failed (async): {e}");
        e.into()
    })
}

/// Handle complex prompts using the Architect system (task decomposition + multi-model routing)
#[cfg(feature = "cef-ui")]
async fn handle_architect_prompt(
    renderer: &mut dyn arkavo_agui::UiRenderer,
    prompt: &str,
    complexity: arkavo_router::ComplexityScore,
) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_llm::Message;
    use arkavo_memory::{PersistedPlan, PlanStateStore, PlanStatus};
    use arkavo_router::{ArchitectExecutor, ArchitectPlanner, Router};
    use std::sync::Arc;

    eprintln!(
        "[ARCHITECT] Complex task detected: {} estimated subtasks, {:.1}% potential savings",
        complexity.estimated_subtasks, complexity.estimated_savings_percent
    );

    // Initialize plan state store for persistence
    let plan_store = {
        let db_path = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("arkavo")
            .join("plan_state.db");
        PlanStateStore::new(&db_path).await.ok()
    };

    // Show planning indicator in UI with two-pane layout
    let planning_html = format!(
        r#"<div style="display:flex;height:100vh;font-family:system-ui,-apple-system,sans-serif;">
            <div style="flex:1;padding:40px;overflow-y:auto;">
                <div style="background:#f5f5f5;padding:12px 16px;border-radius:8px;margin-bottom:20px;color:#333;">
                    <strong style="color:#667eea;">You:</strong> <span style="color:#333;">{}</span>
                </div>
                <div style="background:white;padding:20px;border-radius:8px;box-shadow:0 2px 8px rgba(0,0,0,0.1);">
                    <div style="display:flex;gap:8px;align-items:center;color:#667eea;">
                        <div style="width:8px;height:8px;background:#667eea;border-radius:50%;animation:pulse 1.5s ease-in-out infinite;"></div>
                        <span>Planning task decomposition...</span>
                    </div>
                    <div style="margin-top:12px;font-size:14px;color:#666;">
                        Detected complex task with {} estimated subtasks
                    </div>
                </div>
            </div>
            <div id="side-panel" style="width:320px;background:#f8f9fa;border-left:1px solid #e0e0e0;padding:20px;overflow-y:auto;">
                <h3 style="margin:0 0 16px 0;color:#333;font-size:14px;text-transform:uppercase;letter-spacing:0.5px;">Task Progress</h3>
                <div id="plan-status" style="background:white;border-radius:8px;padding:16px;margin-bottom:16px;">
                    <div style="color:#667eea;font-weight:600;margin-bottom:8px;">Planning Phase</div>
                    <div style="font-size:13px;color:#666;">Analyzing task complexity...</div>
                </div>
                <div id="subtask-list"></div>
            </div>
        </div>"#,
        prompt
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;"),
        complexity.estimated_subtasks
    );
    renderer.render(&planning_html, "", "").await?;

    // Create planner and generate task graph
    let planner = ArchitectPlanner::new();
    let plan = match planner.create_plan(prompt, complexity.clone()).await {
        Ok(p) => p,
        Err(e) => {
            let error_html = format!(
                r#"<div style="padding:40px;font-family:system-ui,-apple-system,sans-serif;">
                    <div style="background:#f5f5f5;padding:12px 16px;border-radius:8px;margin-bottom:20px;color:#333;">
                        <strong style="color:#667eea;">You:</strong> <span style="color:#333;">{}</span>
                    </div>
                    <div style="background:#fee;padding:20px;border-radius:8px;border-left:4px solid #f44;">
                        <strong style="color:#c33;">⚠ Planning Failed</strong><br>
                        <span style="color:#666;font-size:14px;">{}</span>
                    </div>
                </div>"#,
                prompt
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;"),
                e.to_string()
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            );
            renderer.render(&error_html, "", "").await?;
            return Err(e.into());
        }
    };

    eprintln!(
        "[ARCHITECT] Plan created: {} subtasks, est. ${:.4} (saves ${:.4})",
        plan.subtasks.len(),
        plan.architect_estimate_usd,
        plan.opus_only_estimate_usd - plan.architect_estimate_usd
    );

    // Persist the plan for resume capability
    let plan_id = plan.id;
    if let Some(ref store) = plan_store {
        let persisted = PersistedPlan {
            id: plan_id,
            original_prompt: prompt.to_string(),
            plan_json: serde_json::to_string(&plan).unwrap_or_default(),
            status: PlanStatus::Executing,
            current_subtask: 0,
            total_subtasks: plan.subtasks.len(),
            completed_results_json: None,
            error_message: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        if let Err(e) = store.save_plan(&persisted).await {
            eprintln!("[ARCHITECT] Failed to persist plan: {e}");
        }
    }

    // Build subtask list HTML
    use std::fmt::Write;
    let subtask_items = plan.subtasks.iter().fold(String::new(), |mut acc, st| {
        let _ = write!(
            acc,
            r#"<div id="subtask-{}" style="background:white;border-radius:6px;padding:12px;margin-bottom:8px;border-left:3px solid #ccc;">
                    <div style="font-size:13px;font-weight:500;color:#333;">{}. {}</div>
                    <div style="font-size:11px;color:#888;margin-top:4px;">{:?} • {}</div>
                </div>"#,
            st.index,
            st.index + 1,
            st.description
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;"),
            st.category,
            st.assigned_model.name()
        );
        acc
    });

    // Show plan in UI
    let plan_html = format!(
        r#"<div style="display:flex;height:100vh;font-family:system-ui,-apple-system,sans-serif;">
            <div style="flex:1;padding:40px;overflow-y:auto;">
                <div style="background:#f5f5f5;padding:12px 16px;border-radius:8px;margin-bottom:20px;color:#333;">
                    <strong style="color:#667eea;">You:</strong> <span style="color:#333;">{}</span>
                </div>
                <div id="execution-output" style="background:white;padding:20px;border-radius:8px;box-shadow:0 2px 8px rgba(0,0,0,0.1);">
                    <div style="display:flex;gap:8px;align-items:center;color:#667eea;margin-bottom:16px;">
                        <div style="width:8px;height:8px;background:#667eea;border-radius:50%;animation:pulse 1.5s ease-in-out infinite;"></div>
                        <span>Executing {} subtasks...</span>
                    </div>
                </div>
            </div>
            <div id="side-panel" style="width:320px;background:#f8f9fa;border-left:1px solid #e0e0e0;padding:20px;overflow-y:auto;">
                <h3 style="margin:0 0 16px 0;color:#333;font-size:14px;text-transform:uppercase;letter-spacing:0.5px;">Task Progress</h3>
                <div id="plan-status" style="background:#e8f5e9;border-radius:8px;padding:16px;margin-bottom:16px;">
                    <div style="color:#2e7d32;font-weight:600;margin-bottom:8px;">Execution Phase</div>
                    <div style="font-size:13px;color:#666;">Running {} subtasks</div>
                    <div style="font-size:12px;color:#888;margin-top:4px;">Est. savings: {:.1}%</div>
                </div>
                <div id="subtask-list">{}</div>
            </div>
        </div>"#,
        prompt
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;"),
        plan.subtasks.len(),
        plan.subtasks.len(),
        plan.estimated_savings_percent(),
        subtask_items
    );
    renderer.render(&plan_html, "", "").await?;

    // Execute the plan
    let router = Router::new().await?;
    let executor = ArchitectExecutor::new(Arc::new(router));
    let messages = vec![Message::user(prompt)];

    let result = match executor.execute(&plan, messages, None).await {
        Ok(r) => r,
        Err(e) => {
            // Mark plan as failed in persistent store
            if let Some(ref store) = plan_store {
                let _ = store.mark_failed(plan_id, &e.to_string()).await;
            }

            let error_html = format!(
                r#"<div style="padding:40px;font-family:system-ui,-apple-system,sans-serif;">
                    <div style="background:#f5f5f5;padding:12px 16px;border-radius:8px;margin-bottom:20px;color:#333;">
                        <strong style="color:#667eea;">You:</strong> <span style="color:#333;">{}</span>
                    </div>
                    <div style="background:#fee;padding:20px;border-radius:8px;border-left:4px solid #f44;">
                        <strong style="color:#c33;">⚠ Execution Failed</strong><br>
                        <span style="color:#666;font-size:14px;">{}</span>
                    </div>
                </div>"#,
                prompt
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;"),
                e.to_string()
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            );
            renderer.render(&error_html, "", "").await?;
            return Err(e.into());
        }
    };

    // Mark plan as completed in persistent store
    if let Some(ref store) = plan_store {
        let results_json = serde_json::to_string(&result.subtask_results).ok();
        let _ = store
            .update_progress(
                plan_id,
                result.subtask_results.len(),
                results_json.as_deref(),
            )
            .await;
        let _ = store.mark_completed(plan_id).await;
    }

    eprintln!(
        "[ARCHITECT] Execution complete: ${:.4} actual cost, saved ${:.4} ({:.1}%)",
        result.actual_cost_usd,
        result.actual_savings_usd,
        result.savings_percent()
    );

    // Build completed subtask list
    let completed_items = result.subtask_results.iter().fold(String::new(), |mut acc, sr| {
        let status_color = if sr.success { "#4caf50" } else { "#f44336" };
        let status_icon = if sr.success { "✓" } else { "✗" };
        let _ = write!(
            acc,
            r#"<div style="background:white;border-radius:6px;padding:12px;margin-bottom:8px;border-left:3px solid {};">
                    <div style="display:flex;justify-content:space-between;align-items:center;">
                        <span style="font-size:13px;font-weight:500;color:#333;">{}. {}</span>
                        <span style="color:{};">{}</span>
                    </div>
                    <div style="font-size:11px;color:#888;margin-top:4px;">{} • ${:.4}</div>
                </div>"#,
            status_color,
            sr.index + 1,
            plan.subtasks
                .get(sr.index)
                .map(|s| s.description.as_str())
                .unwrap_or("Unknown"),
            status_color,
            status_icon,
            sr.model_used.name(),
            sr.actual_cost_usd
        );
        acc
    });

    // Render final result with completion status
    let escaped_response = result
        .final_response
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', "<br>");

    let final_html = format!(
        r#"<div style="display:flex;height:100vh;font-family:system-ui,-apple-system,sans-serif;">
            <div style="flex:1;padding:40px;overflow-y:auto;">
                <div style="background:#f5f5f5;padding:12px 16px;border-radius:8px;margin-bottom:20px;color:#333;">
                    <strong style="color:#667eea;">You:</strong> <span style="color:#333;">{}</span>
                </div>
                <div style="background:white;padding:20px;border-radius:8px;box-shadow:0 2px 8px rgba(0,0,0,0.1);line-height:1.6;color:#333;">
                    {}
                </div>
                <div style="margin-top:16px;padding:8px 12px;background:#e8f5e9;border-radius:4px;font-size:12px;color:#2e7d32;">
                    Architect Mode • {} subtasks • ${:.4} (saved ${:.4}, {:.1}%)
                </div>
            </div>
            <div id="side-panel" style="width:320px;background:#f8f9fa;border-left:1px solid #e0e0e0;padding:20px;overflow-y:auto;">
                <h3 style="margin:0 0 16px 0;color:#333;font-size:14px;text-transform:uppercase;letter-spacing:0.5px;">Completed</h3>
                <div id="plan-status" style="background:#e8f5e9;border-radius:8px;padding:16px;margin-bottom:16px;">
                    <div style="color:#2e7d32;font-weight:600;margin-bottom:8px;">✓ Task Complete</div>
                    <div style="font-size:13px;color:#666;">{}/{} subtasks succeeded</div>
                    <div style="font-size:12px;color:#2e7d32;margin-top:4px;">Saved {:.1}% vs single model</div>
                </div>
                <div id="subtask-list">{}</div>
            </div>
        </div>"#,
        prompt
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;"),
        escaped_response,
        result.subtask_results.len(),
        result.actual_cost_usd,
        result.actual_savings_usd,
        result.savings_percent(),
        result.subtask_results.iter().filter(|r| r.success).count(),
        result.subtask_results.len(),
        result.savings_percent(),
        completed_items
    );

    renderer.render(&final_html, "", "").await.map_err(|e| {
        eprintln!("[ERROR] renderer.render() failed (architect): {e}");
        e.into()
    })
}

#[cfg(feature = "cef-ui")]
async fn create_client_from_routing(
    decision: &arkavo_router::RoutingDecision,
) -> Result<arkavo_llm::LlmClient, Box<dyn std::error::Error>> {
    use arkavo_llm::{LlmClient, Message};
    use arkavo_router::ModelChoice;

    match decision.recommended_model {
        ModelChoice::GeminiFlash
        | ModelChoice::Gemini35Flash
        | ModelChoice::Gemini35FlashMinimal
        | ModelChoice::Gemini35FlashMedium
        | ModelChoice::Gemini35FlashHigh
        | ModelChoice::GeminiPro => {
            #[cfg(feature = "gemini")]
            {
                use arkavo_llm::GeminiProvider;
                let provider = Box::new(GeminiProvider::new()?);
                Ok(LlmClient::new(provider))
            }
            #[cfg(not(feature = "gemini"))]
            {
                Err("Gemini feature not enabled".into())
            }
        }
        ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus | ModelChoice::ClaudeFable5 => {
            #[cfg(feature = "llm-remote")]
            {
                use arkavo_llm::providers::AnthropicProvider;
                let provider = Box::new(AnthropicProvider::from_env_with_model(
                    decision.recommended_model.name(),
                )?);
                Ok(LlmClient::new(provider))
            }
            #[cfg(not(feature = "llm-remote"))]
            {
                Err("Anthropic support requires llm-remote feature".into())
            }
        }
        ModelChoice::LocalQwen3
        | ModelChoice::LocalMinistral3B
        | ModelChoice::LocalMinistral8B
        | ModelChoice::LocalQwen35_9B
        | ModelChoice::LocalQwen35_27B
        | ModelChoice::LocalQwen36A3B
        | ModelChoice::LocalGlm47Flash
        | ModelChoice::LocalGemma4E2B
        | ModelChoice::LocalGemma4E4B
        | ModelChoice::LocalGemma4_26B
        | ModelChoice::LocalGemma4_31B
        | ModelChoice::LocalGemma4_12B
        | ModelChoice::LocalGemma270M
        | ModelChoice::LocalGemma4B
        | ModelChoice::LocalGemma12B
        | ModelChoice::LocalDeepSeekCoder => {
            // Try Ollama first (from_env defaults to ollama)
            println!("Checking for Ollama...");
            if let Ok(client) = LlmClient::from_env() {
                // Test if ollama is actually running
                if client.complete(vec![Message::user("ping")]).await.is_ok() {
                    println!("Using Ollama for local model");
                    return Ok(client);
                }
            }

            // Ollama is required for local models in UI mode
            // The chat command uses the router which handles llama.cpp internally
            Err(
                "No local LLM available. Please install and start Ollama (https://ollama.ai)."
                    .into(),
            )
        }
        ModelChoice::DeepSeekV32 | ModelChoice::DeepSeekV32Speciale => {
            #[cfg(feature = "deepseek")]
            {
                use arkavo_llm::DeepSeekProvider;
                let provider = Box::new(DeepSeekProvider::from_env()?);
                Ok(LlmClient::new(provider))
            }
            #[cfg(not(feature = "deepseek"))]
            {
                Err("DeepSeek support requires deepseek feature".into())
            }
        }
        ModelChoice::KimiK2 => {
            #[cfg(feature = "kimi")]
            {
                use arkavo_llm::KimiProvider;
                let provider = Box::new(KimiProvider::from_env()?);
                Ok(LlmClient::new(provider))
            }
            #[cfg(not(feature = "kimi"))]
            {
                Err("Kimi support requires kimi feature".into())
            }
        }
    }
}

#[cfg(feature = "web-ui")]
#[allow(dead_code)]
async fn use_web_gateway(
    port: u16,
    initial_prompt: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating web UI gateway...");
    let mut gateway = arkavo_agui::AgUiGateway::new(port);

    if let Some(prompt) = initial_prompt {
        println!("Starting UI with initial prompt: {prompt}");
        println!("UI will generate incrementally - you can interrupt and modify at any time");
        gateway.set_initial_prompt(prompt);
    }

    println!("Web UI is available at http://localhost:{port}");
    println!("Press Ctrl+C to exit");

    gateway.start().await
}

fn print_usage() {
    println!("Arkavo UI - Web interface for AI-driven UI generation");
    println!();
    println!("USAGE:");
    println!("    arkavo ui [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    --port <PORT>              Port to run the UI server on (default: 7700)");
    println!("    --prompt, -p <PROMPT>      Initial prompt for UI generation");
    println!("    <PORT>                     Port number (shorthand)");
    println!();
    println!("ENVIRONMENT:");
    println!("    GEMINI_API_KEY             Gemini API key for UI generation");
    println!();
    println!("EXAMPLES:");
    println!("    arkavo ui");
    println!("    arkavo ui 8080");
    println!("    arkavo ui --prompt \"Build a bank account page\"");
    println!("    arkavo ui --prompt \"Create a dashboard with charts\"");
    println!("    GEMINI_API_KEY=xxx arkavo ui --prompt \"bank + portfolio\"");
}
