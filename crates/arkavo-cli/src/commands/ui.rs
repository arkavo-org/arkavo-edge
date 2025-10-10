use arkavo_agui::AgUiGateway;

#[allow(clippy::disallowed_methods)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if !args.is_empty() && matches!(args[0].as_str(), "help" | "-h" | "--help") {
        print_usage();
        return Ok(());
    }

    let mut port = 7700;
    let mut initial_prompt: Option<String> = None;
    let mut blank_mode = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--blank" => {
                blank_mode = true;
                i += 1;
            }
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
        let mut gateway = AgUiGateway::new(port);

        if blank_mode {
            gateway.set_blank_mode(true);
            println!("Starting in blank canvas mode");
        }

        if let Some(prompt) = initial_prompt {
            println!("Starting UI with initial prompt: {}", prompt);
            println!("UI will generate incrementally - you can interrupt and modify at any time");
            gateway.set_initial_prompt(prompt);
        }

        gateway.start().await
    };

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(run_async),
        Err(_) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(run_async)
        }
    }
}

fn print_usage() {
    println!("Arkavo UI - Web interface for agent orchestration");
    println!();
    println!("USAGE:");
    println!("    arkavo ui [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    --blank                    Start with blank canvas (no dashboard)");
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
    println!("    arkavo ui --blank --prompt \"Build a bank account page\"");
    println!("    arkavo ui --prompt \"Create a dashboard with charts\"");
    println!("    GEMINI_API_KEY=xxx arkavo ui --blank --prompt \"bank + portfolio\"");
}
