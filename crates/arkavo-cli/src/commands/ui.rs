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
    use arkavo_agui::{create_renderer, default_renderer_type};

    let renderer_type = default_renderer_type().ok_or("CEF renderer not available")?;

    println!("Creating CEF renderer...");
    let mut renderer = create_renderer(renderer_type).await?;

    println!("CEF window opened successfully!");

    let html = r#"
        <div id="app">
            <h1>Arkavo UI Generator</h1>
            <p>Native CEF rendering with Rust-driven DOM</p>
            <p style="color: #888;">Waiting for UI generation...</p>
        </div>
    "#;

    let css = r#"
        body {
            margin: 0;
            padding: 40px;
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            min-height: 100vh;
        }
        #app {
            max-width: 800px;
            margin: 0 auto;
        }
        h1 {
            font-size: 2.5em;
            margin-bottom: 20px;
        }
    "#;

    renderer.render(html, css, "").await?;

    if let Some(prompt) = initial_prompt {
        println!("Processing initial prompt: {}", prompt);
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        let generated_html = format!(
            r#"<div id="app">
                <h1>Generated from prompt</h1>
                <p>"{}"</p>
                <button>Action Button</button>
            </div>"#,
            prompt
        );
        renderer.render(&generated_html, css, "").await?;
    }

    println!("CEF renderer is running. Close the window or press Ctrl+C to exit.");

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        if !renderer.is_running() {
            break;
        }
    }

    // Renderer stopped - clean shutdown
    match Box::new(renderer).shutdown().await {
        Ok(_) => {
            println!("Application closed gracefully");
            Ok(())
        }
        Err(e) => {
            // Ignore connection errors during shutdown - they're expected
            if e.to_string().contains("Connection closed") {
                println!("Application closed");
                Ok(())
            } else {
                Err(e.into())
            }
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
