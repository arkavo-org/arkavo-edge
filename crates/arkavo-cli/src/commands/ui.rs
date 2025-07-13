use arkavo_agui::AgUiGateway;

pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if !args.is_empty() && matches!(args[0].as_str(), "help" | "-h" | "--help") {
        print_usage();
        return Ok(());
    }

    // Parse optional port argument
    let port = if !args.is_empty() {
        args[0].parse::<u16>().unwrap_or(7700)
    } else {
        7700
    };

    // Check if we're already in a runtime context
    let run_async = async {
        let gateway = AgUiGateway::new(port);
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
    println!("    arkavo ui [PORT]");
    println!();
    println!("OPTIONS:");
    println!("    PORT    Port to run the UI server on (default: 7700)");
    println!();
    println!("EXAMPLES:");
    println!("    arkavo ui          # Start UI on default port 7700");
    println!("    arkavo ui 8080     # Start UI on port 8080");
}
