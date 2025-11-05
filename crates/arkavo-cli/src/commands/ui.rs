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
async fn use_cef_renderer(
    _port: u16,
    initial_prompt: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_agui::UiRenderer;
    use arkavo_agui::renderer::async_cef_renderer::AsyncCefRendererImpl;

    println!("Creating Async CEF renderer (non-blocking)...");
    let mut cef_renderer = AsyncCefRendererImpl::new().await?;

    println!("CEF window opened with prompt bar!");

    // Wait for page to fully load before processing initial prompt
    println!("[DEBUG] Waiting for page to load...");
    tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
    println!("[DEBUG] Page should be loaded now");

    if let Some(prompt) = initial_prompt {
        println!("Processing initial prompt: {prompt}");
        handle_prompt_async(&mut cef_renderer, &prompt).await?;
    }

    println!("CEF renderer is running. Enter prompts in the UI or press Ctrl+C to exit.");

    // Event loop - poll for prompt submissions and errors
    let mut loop_count = 0;
    let mut last_health_check = std::time::Instant::now();
    let mut processing_prompt = false;
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
                        "[HEARTBEAT]   CEF: {} - {}",
                        format!("{:?}", report.status),
                        report.message
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
                    let reports = registry.check_all().await;
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
                        event.value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
                    );
                    let _ = cef_renderer.update_element("body", &error_display).await;
                }
                "submit" if !event.value.trim().is_empty() => {
                    if processing_prompt {
                        eprintln!("[DEBUG] Ignoring prompt - already processing one");
                    } else {
                        eprintln!("[DEBUG] Processing submit event (async)");
                        processing_prompt = true;
                        match handle_prompt_async(&mut cef_renderer, &event.value).await {
                            Ok(_) => {
                                eprintln!("[DEBUG] Prompt processing completed");
                                processing_prompt = false;
                            }
                            Err(e) => {
                                eprintln!("[ERROR] Prompt processing failed: {}", e);
                                processing_prompt = false;
                                return Err(e);
                            }
                        }
                    }
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
async fn handle_prompt_async(
    renderer: &mut dyn arkavo_agui::UiRenderer,
    prompt: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_llm::Message;
    use arkavo_router::Router;
    use tokio_stream::StreamExt;

    let enhanced_prompt = prompt.to_string();

    // Check for API key availability to determine if we should use cloud models
    let gemini_available = std::env::var("GEMINI_API_KEY").is_ok();

    let router = if gemini_available {
        Router::new().await?
    } else {
        Router::new_offline().await?
    };

    let routing_decision = router.route(&enhanced_prompt).await?;

    let client = create_client_from_routing(&routing_decision).await?;

    let messages = vec![Message::user(&enhanced_prompt)];

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

#[cfg(feature = "cef-ui")]
async fn create_client_from_routing(
    decision: &arkavo_router::RoutingDecision,
) -> Result<arkavo_llm::LlmClient, Box<dyn std::error::Error>> {
    use arkavo_llm::{LlmClient, Message};
    use arkavo_router::ModelChoice;

    match decision.recommended_model {
        ModelChoice::GeminiFlash | ModelChoice::GeminiPro => {
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
        ModelChoice::LocalGemma270M | ModelChoice::LocalGemma4B | ModelChoice::LocalGemma12B => {
            // Try Ollama first (from_env defaults to ollama)
            println!("Checking for Ollama...");
            if let Ok(client) = LlmClient::from_env() {
                // Test if ollama is actually running
                if client.complete(vec![Message::user("ping")]).await.is_ok() {
                    println!("Using Ollama for local model");
                    return Ok(client);
                }
            }

            // Fall back to llama.cpp - use same logic as chat command
            #[cfg(feature = "llama-cpp")]
            {
                println!("Ollama not available, using embedded llama.cpp...");

                let model_name = decision.recommended_model.name();

                // Use chat command's initialization logic by calling with default params
                super::chat::initialize_llm_for_ui(model_name)
                    .await
                    .map_err(|e| {
                        eprintln!("\nError loading local model: {e}");
                        eprintln!(
                            "Please run 'arkavo chat --prompt hi' first to download a model.\n"
                        );
                        e
                    })
            }
            #[cfg(not(feature = "llama-cpp"))]
            {
                Err(
                    "No local LLM available. Please install Ollama or enable llama-cpp feature."
                        .into(),
                )
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
