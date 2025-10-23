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

    // Prevent unused variable/assignment warnings when CEF UI is not enabled
    #[cfg(not(feature = "cef-ui"))]
    {
        let _ = port;
        let _ = &initial_prompt;
    }

    let run_async = async move {
        // Determine which renderer to use
        #[cfg(feature = "cef-ui")]
        {
            println!("Starting Arkavo UI with native CEF renderer...");
            use_cef_renderer(port, initial_prompt).await
        }

        #[cfg(not(feature = "cef-ui"))]
        {
            Err("No UI renderer available. Build with --features cef-ui".into())
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
    use arkavo_agui::renderer::cef_renderer::CefRendererImpl;

    println!("Creating CEF renderer...");
    let mut cef_renderer = CefRendererImpl::new().await?;

    println!("CEF window opened with prompt bar!");

    // Wait for page to fully load before processing initial prompt
    println!("[DEBUG] Waiting for page to load...");
    tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
    println!("[DEBUG] Page should be loaded now");

    if let Some(prompt) = initial_prompt {
        println!("Processing initial prompt: {prompt}");
        handle_prompt(&mut cef_renderer, &prompt).await?;
    }

    println!("CEF renderer is running. Enter prompts in the UI or press Ctrl+C to exit.");

    // Event loop - poll for prompt submissions
    loop {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        if !cef_renderer.is_running() {
            break;
        }

        // Poll for events from the prompt bar
        if let Ok(Some(event)) = cef_renderer.try_recv_event().await
            && event.event_type == "submit"
            && !event.value.trim().is_empty()
        {
            let value = &event.value;
            println!("\n[Prompt received]: {value}");
            handle_prompt(&mut cef_renderer, &event.value).await?;
        }
    }

    // Renderer stopped - clean shutdown
    match Box::new(cef_renderer).shutdown().await {
        Ok(_) => {
            println!("Application closed gracefully");
            Ok(())
        }
        Err(e) => {
            if e.to_string().contains("Connection closed") {
                println!("Application closed");
                Ok(())
            } else {
                Err(e.into())
            }
        }
    }
}

#[cfg(feature = "cef-ui")]
async fn handle_prompt(
    renderer: &mut dyn arkavo_agui::UiRenderer,
    prompt: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Generating UI from prompt: \"{prompt}\"");

    // Generate UI based on prompt
    let generated_html = format!(
        r#"
        <div style="padding: 40px;">
            <h1 style="color: #667eea; margin-bottom: 20px;">Generated UI</h1>
            <div style="background: white; padding: 20px; border-radius: 8px; box-shadow: 0 2px 8px rgba(0,0,0,0.1);">
                <h2 style="color: #333; margin-top: 0;">Your Request</h2>
                <p style="color: #666; font-size: 16px; line-height: 1.6;">{prompt}</p>
                <button style="background: #667eea; color: white; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; font-size: 16px; margin-top: 20px;">
                    Action Button
                </button>
            </div>
        </div>
        "#
    );

    let css = "";

    let html_len = generated_html.len();
    println!("[DEBUG] About to call renderer.render() with {html_len} bytes of HTML");
    match renderer.render(&generated_html, css, "").await {
        Ok(_) => {
            println!("[DEBUG] renderer.render() returned Ok");
            println!("UI updated successfully!");
            Ok(())
        }
        Err(e) => {
            eprintln!("[ERROR] renderer.render() failed: {e}");
            Err(e.into())
        }
    }
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
